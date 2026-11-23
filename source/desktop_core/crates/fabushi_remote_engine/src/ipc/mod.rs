use crate::error::{EngineResult, RemoteEngineError};
pub use futures::future::BoxFuture;
use gpui_telegram_proto::remote_control_contracts::{
    LocalIpcEnvelope, LocalIpcRequest, LocalIpcResponse,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;

pub const IPC_SOCKET_ENV: &str = "FABUSHI_IPC_SOCKET";
const MAX_IPC_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Resolves the platform-native local IPC endpoint path (overridable via `FABUSHI_IPC_SOCKET`).
pub fn default_ipc_endpoint() -> PathBuf {
    if let Some(override_path) = std::env::var_os(IPC_SOCKET_ENV) {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/Fabushi/run/ipc.sock");
        }
        PathBuf::from("/tmp/fabushi-ipc.sock")
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            return PathBuf::from(runtime).join("fabushi/ipc.sock");
        }
        let uid = unsafe { libc::geteuid() };
        PathBuf::from(format!("/tmp/fabushi-{uid}/ipc.sock"))
    }
    #[cfg(target_os = "windows")]
    {
        PathBuf::from(r"\\.\pipe\fabushi-ipc")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        PathBuf::from("/tmp/fabushi-ipc.sock")
    }
}

pub trait LocalIpcHandler: Send + Sync + 'static {
    fn handle(&self, req: LocalIpcRequest) -> BoxFuture<'_, LocalIpcResponse>;
}

/// Writes a 4-byte Big-Endian length-prefixed JSON payload.
pub async fn write_length_prefixed_json<W, T>(writer: &mut W, value: &T) -> EngineResult<()>
where
    W: AsyncWrite + Unpin,
    T: serde::Serialize,
{
    let payload = serde_json::to_vec(value)
        .map_err(|e| RemoteEngineError::Ipc(format!("failed to serialize IPC frame: {e}")))?;
    if payload.len() > MAX_IPC_FRAME_BYTES {
        return Err(RemoteEngineError::Ipc(format!(
            "IPC frame size {} exceeds maximum {}",
            payload.len(),
            MAX_IPC_FRAME_BYTES
        )));
    }
    let len_prefix = (payload.len() as u32).to_be_bytes();
    writer.write_all(&len_prefix).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads a 4-byte Big-Endian length-prefixed JSON payload (`Ok(None)` on clean EOF at frame boundary).
pub async fn read_length_prefixed_json<R, T>(reader: &mut R) -> EngineResult<Option<T>>
where
    R: AsyncRead + Unpin,
    T: serde::de::DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err.into()),
    }
    let frame_len = u32::from_be_bytes(len_buf) as usize;
    if frame_len == 0 || frame_len > MAX_IPC_FRAME_BYTES {
        return Err(RemoteEngineError::Ipc(format!(
            "invalid IPC frame length: {frame_len}"
        )));
    }
    let mut payload = vec![0u8; frame_len];
    reader.read_exact(&mut payload).await?;
    let parsed = serde_json::from_slice::<T>(&payload)
        .map_err(|e| RemoteEngineError::Ipc(format!("invalid JSON-RPC IPC payload: {e}")))?;
    Ok(Some(parsed))
}

#[cfg(unix)]
pub fn verify_unix_peer_uid(fd: std::os::unix::io::RawFd, expected_uid: u32) -> EngineResult<u32> {
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        let mut euid: libc::uid_t = 0;
        let mut egid: libc::gid_t = 0;
        let rc = unsafe { libc::getpeereid(fd, &mut euid, &mut egid) };
        if rc != 0 {
            return Err(RemoteEngineError::PermissionDenied(format!(
                "getpeereid failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        if euid != expected_uid {
            return Err(RemoteEngineError::PermissionDenied(format!(
                "peer UID {euid} does not match expected UID {expected_uid}"
            )));
        }
        Ok(euid)
    }

    #[cfg(target_os = "linux")]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast::<libc::c_void>(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(RemoteEngineError::PermissionDenied(format!(
                "SO_PEERCRED failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        if cred.uid != expected_uid {
            return Err(RemoteEngineError::PermissionDenied(format!(
                "peer UID {} does not match expected UID {expected_uid}",
                cred.uid
            )));
        }
        Ok(cred.uid)
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "linux"
    )))]
    {
        let _ = (fd, expected_uid);
        Err(RemoteEngineError::PermissionDenied(
            "peer UID verification unsupported on this Unix target".to_string(),
        ))
    }
}

/// Cross-platform Local IPC server (Unix Domain Socket on POSIX with `0600` perms + peer UID check;
/// Named Pipe on Windows).
pub struct LocalIpcServer {
    endpoint: PathBuf,
    shutdown_tx: watch::Sender<bool>,
    stopped_rx: watch::Receiver<bool>,
    cleanup_unix_socket: bool,
}

impl LocalIpcServer {
    pub async fn bind_default(handler: Arc<dyn LocalIpcHandler>) -> EngineResult<Self> {
        Self::bind(default_ipc_endpoint(), handler).await
    }

    pub async fn bind(endpoint: PathBuf, handler: Arc<dyn LocalIpcHandler>) -> EngineResult<Self> {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let (stopped_tx, stopped_rx) = watch::channel(false);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            use std::os::unix::io::AsRawFd;
            use tokio::net::{UnixListener, UnixStream};

            if let Some(parent) = endpoint.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                    let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
                }
            }

            if endpoint.exists() {
                match UnixStream::connect(&endpoint).await {
                    Ok(_) => {
                        return Err(RemoteEngineError::Ipc(format!(
                            "IPC endpoint '{}' is already active",
                            endpoint.display()
                        )));
                    }
                    Err(_) => {
                        let _ = std::fs::remove_file(&endpoint);
                    }
                }
            }

            let listener = UnixListener::bind(&endpoint)?;
            std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))?;
            let expected_uid = unsafe { libc::geteuid() };

            let socket_path = endpoint.clone();

            tokio::spawn(async move {
                loop {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                break;
                            }
                        }
                        accept_res = listener.accept() => {
                            match accept_res {
                                Ok((mut stream, _addr)) => {
                                    let fd = stream.as_raw_fd();
                                    if let Err(err) = verify_unix_peer_uid(fd, expected_uid) {
                                        tracing::warn!(error = %err, "rejected unauthorized local IPC peer");
                                        continue;
                                    }
                                    let h = Arc::clone(&handler);
                                    tokio::spawn(async move {
                                        loop {
                                            let req: LocalIpcRequest = match read_length_prefixed_json(&mut stream).await {
                                                Ok(Some(r)) => r,
                                                Ok(None) => break,
                                                Err(err) => {
                                                    let resp = LocalIpcResponse::err("", "invalid_frame", err.to_string());
                                                    let _ = write_length_prefixed_json(&mut stream, &resp).await;
                                                    break;
                                                }
                                            };
                                            let resp = h.handle(req).await;
                                            if write_length_prefixed_json(&mut stream, &resp).await.is_err() {
                                                break;
                                            }
                                        }
                                    });
                                }
                                Err(err) => {
                                    tracing::warn!(error = %err, "IPC accept error");
                                }
                            }
                        }
                    }
                }
                let _ = std::fs::remove_file(&socket_path);
                let _ = stopped_tx.send(true);
            });

            return Ok(Self {
                endpoint,
                shutdown_tx,
                stopped_rx,
                cleanup_unix_socket: true,
            });
        }

        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ServerOptions;
            let pipe_name = endpoint.to_string_lossy().to_string();
            let first_server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&pipe_name)?;

            tokio::spawn(async move {
                let mut current = first_server;
                loop {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                break;
                            }
                        }
                        res = current.connect() => {
                            match res {
                                Ok(()) => {
                                    let connected_pipe = current;
                                    match ServerOptions::new().create(&pipe_name) {
                                        Ok(next) => current = next,
                                        Err(err) => {
                                            tracing::warn!(error = %err, "failed to create next Windows named pipe instance");
                                            let h = Arc::clone(&handler);
                                            handle_pipe_stream(connected_pipe, h).await;
                                            break;
                                        }
                                    }
                                    let h = Arc::clone(&handler);
                                    tokio::spawn(async move {
                                        handle_pipe_stream(connected_pipe, h).await;
                                    });
                                }
                                Err(err) => {
                                    tracing::warn!(error = %err, "Windows named pipe connect error");
                                    break;
                                }
                            }
                        }
                    }
                }
                let _ = stopped_tx.send(true);
            });

            return Ok(Self {
                endpoint,
                shutdown_tx,
                stopped_rx,
                cleanup_unix_socket: false,
            });
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = (handler, shutdown_tx, stopped_rx, shutdown_rx, stopped_tx);
            Err(RemoteEngineError::Unavailable(
                "local IPC unsupported on this platform".into(),
            ))
        }
    }

    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    pub async fn wait_stopped(&self) {
        let mut rx = self.stopped_rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }
}

impl Drop for LocalIpcServer {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if self.cleanup_unix_socket {
            let _ = std::fs::remove_file(&self.endpoint);
        }
    }
}

#[cfg(windows)]
async fn handle_pipe_stream<S>(mut stream: S, handler: Arc<dyn LocalIpcHandler>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let req: LocalIpcRequest = match read_length_prefixed_json(&mut stream).await {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(err) => {
                let resp = LocalIpcResponse::err("", "invalid_frame", err.to_string());
                let _ = write_length_prefixed_json(&mut stream, &resp).await;
                break;
            }
        };
        let resp = handler.handle(req).await;
        if write_length_prefixed_json(&mut stream, &resp).await.is_err() {
            break;
        }
    }
}

/// Client for talking to the running local Fabushi Desktop or Daemon over UDS / Named Pipe.
#[derive(Debug, Clone)]
pub struct LocalIpcClient {
    endpoint: PathBuf,
}

impl LocalIpcClient {
    pub fn new(endpoint: PathBuf) -> Self {
        Self { endpoint }
    }

    pub fn default_endpoint() -> Self {
        Self::new(default_ipc_endpoint())
    }

    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }

    pub async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> EngineResult<LocalIpcResponse> {
        let req = LocalIpcRequest::new(uuid::Uuid::new_v4().to_string(), method, params);

        #[cfg(unix)]
        {
            use tokio::net::UnixStream;
            let mut stream = UnixStream::connect(&self.endpoint).await.map_err(|e| {
                RemoteEngineError::Ipc(format!(
                    "failed to connect to IPC socket '{}': {e}",
                    self.endpoint.display()
                ))
            })?;
            write_length_prefixed_json(&mut stream, &req).await?;
            let resp: LocalIpcResponse = read_length_prefixed_json(&mut stream)
                .await?
                .ok_or_else(|| RemoteEngineError::Ipc("IPC server closed connection".into()))?;
            return Ok(resp);
        }

        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ClientOptions;
            let pipe_name = self.endpoint.to_string_lossy().to_string();
            let mut client = ClientOptions::new().open(&pipe_name).map_err(|e| {
                RemoteEngineError::Ipc(format!("failed to open Windows named pipe '{pipe_name}': {e}"))
            })?;
            write_length_prefixed_json(&mut client, &req).await?;
            let resp: LocalIpcResponse = read_length_prefixed_json(&mut client)
                .await?
                .ok_or_else(|| RemoteEngineError::Ipc("IPC pipe server closed connection".into()))?;
            return Ok(resp);
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = req;
            Err(RemoteEngineError::Unavailable(
                "local IPC unsupported on this platform".into(),
            ))
        }
    }

    pub async fn call_typed<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> EngineResult<LocalIpcEnvelope<T>> {
        let resp = self.call(method, params).await?;
        let data = match resp.data {
            Some(val) => Some(
                serde_json::from_value::<T>(val)
                    .map_err(|e| RemoteEngineError::Ipc(e.to_string()))?,
            ),
            None => None,
        };
        Ok(LocalIpcEnvelope {
            success: resp.success,
            data,
            error: resp.error,
            timestamp: resp.timestamp,
        })
    }

    pub async fn ping_status(&self) -> bool {
        self.call("status", serde_json::json!({}))
            .await
            .map(|r| r.success)
            .unwrap_or(false)
    }
}
