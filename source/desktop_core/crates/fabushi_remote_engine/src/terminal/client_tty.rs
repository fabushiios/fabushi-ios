use crate::error::{EngineResult, RemoteEngineError};
use crate::ipc::LocalIpcClient;
use crate::terminal::is_inside_mux;
use gpui_telegram_proto::remote_control_contracts::TerminalSessionDto;
use std::io::Write;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct TtyAttachOptions {
    pub session_name: Option<String>,
    pub shell: Option<String>,
    pub create_if_missing: bool,
    pub ignore_size: bool,
}

/// Stateful `Ctrl+] d` (`0x1d` then `'d'`) detach sequence detector.
#[derive(Debug, Default)]
pub struct DetachEscapeParser {
    armed: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EscapeAction {
    Forward(Vec<u8>),
    Detach,
}

impl DetachEscapeParser {
    pub fn process(&mut self, input: &[u8]) -> EscapeAction {
        let mut forward = Vec::with_capacity(input.len());
        for &b in input {
            if self.armed {
                self.armed = false;
                if b == b'd' || b == b'D' {
                    return EscapeAction::Detach;
                }
                forward.push(0x1d);
                forward.push(b);
            } else if b == 0x1d {
                self.armed = true;
            } else {
                forward.push(b);
            }
        }
        EscapeAction::Forward(forward)
    }
}

// iOS target_os adaptation: local raw-TTY termios/TIOCGWINSZ/SIGWINCH are desktop CLI APIs;
// iOS attaches remote PTY sessions through SwiftUI terminal UI over RemoteControlHub.
#[cfg(all(unix, not(target_os = "ios")))]
struct RawTermiosGuard {
    fd: std::os::unix::io::RawFd,
    original: libc::termios,
}

#[cfg(all(unix, not(target_os = "ios")))]
impl RawTermiosGuard {
    fn enter(fd: std::os::unix::io::RawFd) -> Option<Self> {
        unsafe {
            if libc::isatty(fd) != 1 {
                return None;
            }
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut orig) != 0 {
                return None;
            }
            let mut raw = orig;
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(fd, libc::TCSANOW, &raw) != 0 {
                return None;
            }
            Some(Self { fd, original: orig })
        }
    }
}

#[cfg(all(unix, not(target_os = "ios")))]
impl Drop for RawTermiosGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

pub fn query_terminal_size() -> (u16, u16) {
    #[cfg(all(unix, not(target_os = "ios")))]
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ as _, &mut ws) == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            return (ws.ws_col, ws.ws_row);
        }
    }
    (120, 36)
}

/// Interactive raw-mode local TTY client (`fabushi lterm attach/new`), rejecting nested attach/new
/// when `FABUSHI_IN_MUX=1`, supporting `Ctrl+] d` detach and `SIGWINCH` forwarding.
pub async fn run_local_tty_attach(
    client: &LocalIpcClient,
    opts: TtyAttachOptions,
) -> EngineResult<i32> {
    if is_inside_mux() {
        return Err(RemoteEngineError::NestedMuxBlocked(
            "refusing recursive lterm attach/new inside an active Fabushi PTY session".to_string(),
        ));
    }

    let (cols, rows) = query_terminal_size();
    let target_name = match opts.session_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => {
            let has_resp = client
                .call("lterm.has", serde_json::json!({ "name": n }))
                .await?;
            let exists = has_resp
                .data
                .as_ref()
                .and_then(|d| d.get("exists"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !exists {
                if opts.create_if_missing {
                    let created = client
                        .call(
                            "lterm.new",
                            serde_json::json!({
                                "name": n,
                                "shell": opts.shell,
                                "cols": cols,
                                "rows": rows,
                            }),
                        )
                        .await?;
                    if !created.success {
                        return Err(RemoteEngineError::Ipc(
                            created
                                .error
                                .map(|e| e.message)
                                .unwrap_or_else(|| "failed to create terminal session".into()),
                        ));
                    }
                } else {
                    return Err(RemoteEngineError::NotFound(format!(
                        "terminal session '{n}' does not exist"
                    )));
                }
            }
            n.to_string()
        }
        None => {
            let ls_resp = client.call("lterm.ls", serde_json::json!({})).await?;
            let existing: Vec<TerminalSessionDto> = ls_resp
                .data
                .as_ref()
                .and_then(|d| d.get("sessions"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            if let Some(first) = existing.first() {
                first.name.clone()
            } else if opts.create_if_missing {
                let created = client
                    .call(
                        "lterm.new",
                        serde_json::json!({
                            "shell": opts.shell,
                            "cols": cols,
                            "rows": rows,
                        }),
                    )
                    .await?;
                let dto: TerminalSessionDto = serde_json::from_value(
                    created.data.unwrap_or_default(),
                )
                .map_err(|e| RemoteEngineError::Ipc(e.to_string()))?;
                dto.name
            } else {
                return Err(RemoteEngineError::NotFound(
                    "no active terminal sessions to attach".into(),
                ));
            }
        }
    };

    let client_id = format!("tty-{}", uuid::Uuid::new_v4());
    let attach_resp = client
        .call(
            "lterm.attach",
            serde_json::json!({
                "name": target_name,
                "client_id": client_id,
                "is_remote": false,
                "ignore_size": opts.ignore_size,
            }),
        )
        .await?;
    if !attach_resp.success {
        return Err(RemoteEngineError::Ipc(
            attach_resp
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "attach failed".into()),
        ));
    }

    let mut offset = attach_resp
        .data
        .as_ref()
        .and_then(|d| d.get("offset"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if let Some(snap) = attach_resp
        .data
        .as_ref()
        .and_then(|d| d.get("scrollback"))
        .and_then(|v| v.as_str())
    {
        if !snap.is_empty() {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(snap.as_bytes());
            let _ = stdout.flush();
        }
    }

    let _ = client
        .call(
            "lterm.resize",
            serde_json::json!({
                "name": target_name,
                "client_id": client_id,
                "cols": cols,
                "rows": rows,
                "is_remote": false,
                "ignore_size": opts.ignore_size,
            }),
        )
        .await;

    #[cfg(all(unix, not(target_os = "ios")))]
    let _raw_guard = RawTermiosGuard::enter(libc::STDIN_FILENO);

    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    std::thread::Builder::new()
        .name("fabushi-tty-stdin".into())
        .spawn(move || {
            use std::io::Read;
            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 1024];
            while let Ok(n) = stdin.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if stdin_tx.blocking_send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        })
        .ok();

    let mut parser = DetachEscapeParser::default();
    let mut poll_tick = tokio::time::interval(Duration::from_millis(25));

    #[cfg(all(unix, not(target_os = "ios")))]
    let mut winch_stream =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change()).ok();

    loop {
        #[cfg(all(unix, not(target_os = "ios")))]
        let winch_fut = async {
            if let Some(ref mut s) = winch_stream {
                s.recv().await
            } else {
                std::future::pending::<Option<()>>().await
            }
        };
        #[cfg(not(all(unix, not(target_os = "ios"))))]
        let winch_fut = std::future::pending::<Option<()>>();

        tokio::select! {
            _ = poll_tick.tick() => {
                if let Ok(resp) = client.call("lterm.read", serde_json::json!({
                    "name": target_name,
                    "since_offset": offset,
                })).await {
                    if let Some(data) = resp.data {
                        if let Some(next_off) = data.get("next_offset").and_then(|v| v.as_u64()) {
                            offset = next_off;
                        }
                        if let Some(b64) = data.get("data_b64").and_then(|v| v.as_str()) {
                            use base64::Engine as _;
                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                                if !bytes.is_empty() {
                                    let mut stdout = std::io::stdout();
                                    let _ = stdout.write_all(&bytes);
                                    let _ = stdout.flush();
                                }
                            }
                        }
                        if data.get("alive").and_then(|v| v.as_bool()) == Some(false) {
                            break;
                        }
                    }
                }
            }
            maybe_in = stdin_rx.recv() => {
                let Some(bytes) = maybe_in else {
                    break;
                };
                match parser.process(&bytes) {
                    EscapeAction::Detach => {
                        break;
                    }
                    EscapeAction::Forward(fwd) if !fwd.is_empty() => {
                        use base64::Engine as _;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&fwd);
                        let _ = client.call("lterm.write", serde_json::json!({
                            "name": target_name,
                            "data_b64": b64,
                        })).await;
                    }
                    _ => {}
                }
            }
            _ = winch_fut => {
                let (c, r) = query_terminal_size();
                let _ = client.call("lterm.resize", serde_json::json!({
                    "name": target_name,
                    "client_id": client_id,
                    "cols": c,
                    "rows": r,
                    "is_remote": false,
                    "ignore_size": opts.ignore_size,
                })).await;
            }
        }
    }

    let _ = client
        .call(
            "lterm.detach",
            serde_json::json!({
                "name": target_name,
                "client_id": client_id,
            }),
        )
        .await;
    Ok(0)
}
