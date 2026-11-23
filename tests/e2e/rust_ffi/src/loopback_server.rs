//! Ephemeral real-socket HTTP/1.1 + RFC 6455 WebSocket loopback protocol server
//! bound to `127.0.0.1:0` with multi-packet & RFC 9112 chunked HTTP decoding,
//! HTTP 206/416 range streaming, deterministic fault injection, stateful shared
//! multi-device backend tables, strict RFC 6455 WebSocket framing, and clean
//! wake-on-drop connection teardown.

use crate::contracts::sha1_digest;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WS_MAGIC_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;
const HTTP_READ_DEADLINE: Duration = Duration::from_secs(5);
const WS_POLL_TIMEOUT: Duration = Duration::from_millis(50);
const WS_FRAME_BODY_DEADLINE: Duration = Duration::from_secs(3);
const MAX_WS_FRAME_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;
const MAX_WS_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_UPLOAD_TOTAL_SIZE: u64 = 4_294_967_296;
const MAX_UPLOAD_PART_SIZE: u64 = 16_777_216;

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct InjectedFault {
    path_prefix: String,
    status: u16,
    body: String,
    retry_after_ms: Option<u64>,
    remaining_hits: Option<usize>,
}

#[derive(Debug, Clone)]
struct UserRecord {
    user_id: u64,
    username: String,
    auth_version: i64,
    password: Option<String>,
    auth_salt_b64: String,
    auth_verifier_b64: Option<String>,
    kdf_params: Value,
    vault_envelope: Option<Value>,
    two_factor_password: Option<String>,
    two_factor_hint: Option<String>,
    two_factor_recovery_email: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionRecord {
    session_id: String,
    user_id: u64,
    token: String,
    device_name: String,
    created_at_ms: i64,
    expires_at_ms: i64,
    revoked: bool,
}

#[derive(Debug, Clone)]
struct TwoFactorChallenge {
    user_id: u64,
    device_name: String,
    expires_at_ms: i64,
}

#[derive(Debug, Clone)]
struct QrPairRecord {
    pair_token: String,
    device_name: String,
    status: String,
    accepted_by_user_id: Option<u64>,
    requires_2fa: bool,
    issued_session: Option<Value>,
    expires_at_ms: i64,
}

#[derive(Debug, Clone)]
struct ChatRecord {
    chat_id: i64,
    title: String,
    chat_type: u64,
    member_count: u64,
}

#[derive(Debug, Clone)]
struct ChatInviteRecord {
    invite_id: String,
    chat_id: i64,
    code: String,
    max_uses: Option<u64>,
    uses: u64,
    revoked: bool,
}

#[derive(Debug, Clone)]
struct StoredMessage {
    message_id: i64,
    chat_id: i64,
    pts: u64,
    text: String,
    content: Value,
    client_nonce: Option<u64>,
    timestamp_ms: i64,
}

#[derive(Debug, Clone)]
struct DeviceSyncRecord {
    request_id: String,
    device_name: String,
    ephemeral_x25519_pub_b64: String,
    verification_code: String,
    status: String,
    wrapped_key_b64: Option<String>,
    expires_at_ms: i64,
}

#[derive(Debug, Clone)]
struct UploadSession {
    upload_id: String,
    total_size: u64,
    part_size: u64,
    expected_sha256: Option<String>,
    parts: BTreeMap<usize, Vec<u8>>,
}

#[derive(Default)]
struct ServerState {
    offline: bool,
    recorded: Vec<RecordedRequest>,
    faults: Vec<InjectedFault>,
    delays: HashMap<String, u64>,
    drop_next_ws: bool,
    ws_push_queue: VecDeque<String>,
    ws_received_frames: Vec<String>,
    media_blobs: HashMap<String, Vec<u8>>,
    journal_floors: HashMap<i64, u64>,
    oauth_confirmed: HashMap<String, Value>,
    users_by_id: HashMap<u64, UserRecord>,
    username_to_id: HashMap<String, u64>,
    sessions_by_token: HashMap<String, SessionRecord>,
    session_id_to_token: HashMap<String, String>,
    revoked_tokens: HashSet<String>,
    two_factor_challenges: HashMap<String, TwoFactorChallenge>,
    qr_pairs: HashMap<String, QrPairRecord>,
    chats: HashMap<i64, ChatRecord>,
    chat_invites: HashMap<String, ChatInviteRecord>,
    chat_messages: HashMap<i64, Vec<StoredMessage>>,
    chat_pts: HashMap<i64, u64>,
    chat_nonce_map: HashMap<(i64, u64), StoredMessage>,
    vault_items: HashMap<(u64, String, String), Value>,
    device_syncs: HashMap<String, DeviceSyncRecord>,
    uploads: HashMap<String, UploadSession>,
    next_user_id: u64,
    next_chat_id: i64,
    next_msg_id: i64,
    next_seq: u64,
}

impl ServerState {
    fn new() -> Self {
        Self {
            next_user_id: 1000,
            next_chat_id: 10,
            next_msg_id: 1,
            next_seq: 1,
            ..Self::default()
        }
    }

    fn alloc_seq(&mut self) -> u64 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    fn mint_session(&mut self, user_id: u64, device_name: &str, ts: i64) -> SessionRecord {
        let seq = self.alloc_seq();
        let session_id = format!("sess_{user_id}_{seq}");
        let token = format!("tok_{user_id}_{seq}_{ts}");
        let rec = SessionRecord {
            session_id: session_id.clone(),
            user_id,
            token: token.clone(),
            device_name: device_name.to_owned(),
            created_at_ms: ts,
            expires_at_ms: ts + 86_400_000,
            revoked: false,
        };
        self.session_id_to_token.insert(session_id, token.clone());
        self.sessions_by_token.insert(token, rec.clone());
        rec
    }

    fn default_user_id(&self) -> u64 {
        self.users_by_id
            .keys()
            .copied()
            .min()
            .unwrap_or(1000)
    }

    fn ensure_user_by_id(&mut self, user_id: u64) -> &mut UserRecord {
        self.users_by_id.entry(user_id).or_insert_with(|| UserRecord {
            user_id,
            username: format!("user_{user_id}"),
            auth_version: 2,
            password: None,
            auth_salt_b64: B64.encode([0x42u8; 16]),
            auth_verifier_b64: None,
            kdf_params: json!({ "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }),
            vault_envelope: None,
            two_factor_password: None,
            two_factor_hint: None,
            two_factor_recovery_email: None,
        })
    }
}

pub struct LoopbackServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    state: Arc<Mutex<ServerState>>,
    conn_workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
    worker: Option<JoinHandle<()>>,
}

impl LoopbackServer {
    pub fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|err| format!("failed to bind loopback 127.0.0.1:0: {err}"))?;
        let addr = listener
            .local_addr()
            .map_err(|err| format!("failed to read loopback local_addr: {err}"))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(ServerState::new()));
        let conn_workers = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));

        let shutdown_clone = Arc::clone(&shutdown);
        let state_clone = Arc::clone(&state);
        let workers_clone = Arc::clone(&conn_workers);

        let worker = thread::Builder::new()
            .name(format!("fabushi-e2e-loopback-{}", addr.port()))
            .spawn(move || {
                for incoming in listener.incoming() {
                    if shutdown_clone.load(Ordering::SeqCst) {
                        break;
                    }
                    match incoming {
                        Ok(stream) => {
                            if shutdown_clone.load(Ordering::SeqCst) {
                                let _ = stream.shutdown(Shutdown::Both);
                                break;
                            }
                            let st = Arc::clone(&state_clone);
                            let sd = Arc::clone(&shutdown_clone);
                            let handle = thread::spawn(move || {
                                handle_connection(stream, st, sd);
                            });
                            if let Ok(mut workers) = workers_clone.lock() {
                                workers.retain(|h| !h.is_finished());
                                workers.push(handle);
                            }
                        }
                        Err(_) => {
                            if shutdown_clone.load(Ordering::SeqCst) {
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(|err| format!("failed to spawn loopback worker thread: {err}"))?;

        Ok(Self {
            addr,
            shutdown,
            state,
            conn_workers,
            worker: Some(worker),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}/api/v1/ws", self.addr)
    }

    pub fn set_offline(&self, offline: bool) {
        if let Ok(mut guard) = self.state.lock() {
            guard.offline = offline;
        }
    }

    pub fn inject_http_status(
        &self,
        path_prefix: &str,
        status: u16,
        body_json: Value,
        retry_after_ms: Option<u64>,
        remaining_hits: Option<usize>,
    ) {
        if let Ok(mut guard) = self.state.lock() {
            guard.faults.push(InjectedFault {
                path_prefix: path_prefix.to_owned(),
                status,
                body: body_json.to_string(),
                retry_after_ms,
                remaining_hits,
            });
        }
    }

    pub fn inject_delay_ms(&self, path_prefix: &str, delay_ms: u64) {
        if let Ok(mut guard) = self.state.lock() {
            guard.delays.insert(path_prefix.to_owned(), delay_ms);
        }
    }

    pub fn clear_faults(&self) {
        if let Ok(mut guard) = self.state.lock() {
            guard.faults.clear();
            guard.delays.clear();
            guard.drop_next_ws = false;
            guard.offline = false;
        }
    }

    pub fn drop_next_ws_connection(&self) {
        if let Ok(mut guard) = self.state.lock() {
            guard.drop_next_ws = true;
        }
    }

    pub fn queue_ws_push_frame(&self, text_json: &str) {
        if let Ok(mut guard) = self.state.lock() {
            guard.ws_push_queue.push_back(text_json.to_owned());
        }
    }

    pub fn ws_received_frames(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|g| g.ws_received_frames.clone())
            .unwrap_or_default()
    }

    pub fn seed_media_blob(&self, file_id: &str, bytes: Vec<u8>) {
        if let Ok(mut guard) = self.state.lock() {
            guard.media_blobs.insert(file_id.to_owned(), bytes);
        }
    }

    pub fn set_journal_floor(&self, chat_id: i64, floor_pts: u64) {
        if let Ok(mut guard) = self.state.lock() {
            guard.journal_floors.insert(chat_id, floor_pts);
        }
    }

    pub fn confirm_alipay_attempt(&self, attempt_id: &str, session: Value) {
        if let Ok(mut guard) = self.state.lock() {
            guard.oauth_confirmed.insert(attempt_id.to_owned(), session);
        }
    }

    pub fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.state
            .lock()
            .map(|g| g.recorded.clone())
            .unwrap_or_default()
    }

    pub fn request_count_for_path(&self, path_prefix: &str) -> usize {
        self.state
            .lock()
            .map(|g| {
                g.recorded
                    .iter()
                    .filter(|r| r.path.starts_with(path_prefix))
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn clear_recorded_requests(&self) {
        if let Ok(mut guard) = self.state.lock() {
            guard.recorded.clear();
        }
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Wake blocking `listener.incoming()` accept call cleanly.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        let workers = self
            .conn_workers
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        for handle in workers {
            let _ = handle.join();
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn status_reason(code: u16) -> &'static str {
    match code {
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        206 => "Partial Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra_headers: &[(&str, String)],
    body: &[u8],
) {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        status,
        status_reason(status),
        content_type,
        body.len()
    );
    for (k, v) in extra_headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

enum HttpReadError {
    Closed,
    BadRequest(&'static str),
    PayloadTooLarge,
}

fn remaining_timeout(deadline: Instant) -> Option<Duration> {
    let now = Instant::now();
    if now >= deadline {
        None
    } else {
        Some((deadline - now).max(Duration::from_millis(1)))
    }
}

fn read_more_with_deadline(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    max_read: usize,
    deadline: Instant,
    eof_err: &'static str,
    timeout_err: &'static str,
) -> Result<usize, HttpReadError> {
    let mut tmp = [0u8; 8192];
    let to_read = max_read.clamp(1, tmp.len());
    loop {
        let Some(rem) = remaining_timeout(deadline) else {
            return Err(HttpReadError::BadRequest(timeout_err));
        };
        let _ = stream.set_read_timeout(Some(rem));
        match stream.read(&mut tmp[..to_read]) {
            Ok(0) => return Err(HttpReadError::BadRequest(eof_err)),
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                return Ok(n);
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if Instant::now() >= deadline {
                    return Err(HttpReadError::BadRequest(timeout_err));
                }
            }
            Err(_) => return Err(HttpReadError::BadRequest(eof_err)),
        }
    }
}

fn read_exact_len_with_deadline(
    stream: &mut TcpStream,
    body: &mut Vec<u8>,
    target_len: usize,
    deadline: Instant,
) -> Result<(), HttpReadError> {
    while body.len() < target_len {
        let want = (target_len - body.len()).min(8192);
        let _ = read_more_with_deadline(
            stream,
            body,
            want,
            deadline,
            "truncated_body_eof",
            "truncated_body_timeout",
        )?;
    }
    Ok(())
}

fn ensure_raw_bytes_with_deadline(
    stream: &mut TcpStream,
    raw: &mut Vec<u8>,
    cursor: usize,
    needed: usize,
    deadline: Instant,
) -> Result<(), HttpReadError> {
    while raw.len().saturating_sub(cursor) < needed {
        let deficit = needed - raw.len().saturating_sub(cursor);
        let _ = read_more_with_deadline(
            stream,
            raw,
            deficit.min(8192),
            deadline,
            "truncated_chunk_payload",
            "truncated_chunk_timeout",
        )?;
    }
    Ok(())
}

fn read_crlf_line_with_deadline(
    stream: &mut TcpStream,
    raw: &mut Vec<u8>,
    cursor: &mut usize,
    deadline: Instant,
) -> Result<String, HttpReadError> {
    const MAX_LINE_LEN: usize = 1024;
    loop {
        if let Some(pos) = raw[*cursor..].windows(2).position(|w| w == b"\r\n") {
            let line = String::from_utf8_lossy(&raw[*cursor..*cursor + pos]).into_owned();
            *cursor += pos + 2;
            return Ok(line);
        }
        if raw.len().saturating_sub(*cursor) > MAX_LINE_LEN {
            return Err(HttpReadError::BadRequest("chunk_line_too_long"));
        }
        let _ = read_more_with_deadline(
            stream,
            raw,
            4096,
            deadline,
            "truncated_chunk_header",
            "truncated_chunk_header_timeout",
        )?;
    }
}

fn read_chunked_body_with_deadline(
    stream: &mut TcpStream,
    initial: &[u8],
    deadline: Instant,
) -> Result<Vec<u8>, HttpReadError> {
    let mut raw = initial.to_vec();
    let mut cursor = 0usize;
    let mut decoded = Vec::new();

    loop {
        let size_line = read_crlf_line_with_deadline(stream, &mut raw, &mut cursor, deadline)?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        if size_hex.is_empty() {
            return Err(HttpReadError::BadRequest("empty_chunk_size"));
        }
        let chunk_size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| HttpReadError::BadRequest("invalid_chunk_size"))?;

        if chunk_size == 0 {
            // Consume optional trailers until empty CRLF line.
            loop {
                let trailer =
                    read_crlf_line_with_deadline(stream, &mut raw, &mut cursor, deadline)?;
                if trailer.is_empty() {
                    return Ok(decoded);
                }
            }
        }

        if decoded.len().saturating_add(chunk_size) > MAX_HTTP_BODY_BYTES {
            return Err(HttpReadError::PayloadTooLarge);
        }

        ensure_raw_bytes_with_deadline(stream, &mut raw, cursor, chunk_size + 2, deadline)?;
        if &raw[cursor + chunk_size..cursor + chunk_size + 2] != b"\r\n" {
            return Err(HttpReadError::BadRequest("missing_chunk_crlf"));
        }
        decoded.extend_from_slice(&raw[cursor..cursor + chunk_size]);
        cursor += chunk_size + 2;

        if cursor >= 65_536 {
            raw.drain(..cursor);
            cursor = 0;
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> Result<RecordedRequest, HttpReadError> {
    let deadline = Instant::now() + HTTP_READ_DEADLINE;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end;

    loop {
        let Some(rem) = remaining_timeout(deadline) else {
            return Err(HttpReadError::BadRequest("header_timeout"));
        };
        let _ = stream.set_read_timeout(Some(rem));
        match stream.read(&mut tmp) {
            Ok(0) => {
                if buf.is_empty() {
                    return Err(HttpReadError::Closed);
                }
                return Err(HttpReadError::BadRequest("eof_in_headers"));
            }
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = pos + 4;
                    break;
                }
                if buf.len() > MAX_HTTP_HEADER_BYTES {
                    return Err(HttpReadError::BadRequest("headers_too_large"));
                }
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if Instant::now() >= deadline {
                    return Err(HttpReadError::BadRequest("header_timeout"));
                }
            }
            Err(_) => {
                if buf.is_empty() {
                    return Err(HttpReadError::Closed);
                }
                return Err(HttpReadError::BadRequest("io_error_in_headers"));
            }
        }
    }

    let head_str = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = head_str.lines();
    let req_line = lines
        .next()
        .ok_or(HttpReadError::BadRequest("missing_request_line"))?;
    let mut parts = req_line.split_whitespace();
    let method = parts
        .next()
        .ok_or(HttpReadError::BadRequest("missing_method"))?
        .to_owned();
    let raw_target = parts
        .next()
        .ok_or(HttpReadError::BadRequest("missing_target"))?;
    let (path, query) = match raw_target.split_once('?') {
        Some((p, q)) => (p.to_owned(), q.to_owned()),
        None => (raw_target.to_owned(), String::new()),
    };

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    if headers
        .get("expect")
        .is_some_and(|v| v.to_ascii_lowercase().contains("100-continue"))
    {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        let _ = stream.flush();
    }

    let is_chunked = headers
        .get("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));

    let body = if is_chunked {
        read_chunked_body_with_deadline(stream, &buf[header_end..], deadline)?
    } else if let Some(cl_raw) = headers.get("content-length") {
        let content_length: usize = cl_raw
            .trim()
            .parse()
            .map_err(|_| HttpReadError::BadRequest("invalid_content_length"))?;
        if content_length > MAX_HTTP_BODY_BYTES {
            return Err(HttpReadError::PayloadTooLarge);
        }
        let mut body = buf[header_end..].to_vec();
        if body.len() < content_length {
            read_exact_len_with_deadline(stream, &mut body, content_length, deadline)?;
        }
        if body.len() > content_length {
            body.truncate(content_length);
        }
        body
    } else {
        Vec::new()
    };

    Ok(RecordedRequest {
        method,
        path,
        query,
        headers,
        body,
    })
}

/// Computes RFC 6455 `Sec-WebSocket-Accept` header value using FIPS 180-4 SHA-1.
pub fn compute_websocket_accept(sec_websocket_key: &str) -> String {
    let mut material = Vec::with_capacity(sec_websocket_key.len() + WS_MAGIC_GUID.len());
    material.extend_from_slice(sec_websocket_key.trim().as_bytes());
    material.extend_from_slice(WS_MAGIC_GUID.as_bytes());
    let digest = sha1_digest(&material);
    B64.encode(digest)
}

fn write_ws_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | (opcode & 0x0F));
    let len = payload.len();
    if len <= 125 {
        header.push(len as u8);
    } else if len <= 65_535 {
        header.push(126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    stream.flush()
}

fn write_ws_close(stream: &mut TcpStream, code: u16, reason: &str) -> std::io::Result<()> {
    let mut payload = Vec::with_capacity(2 + reason.len().min(123));
    payload.extend_from_slice(&code.to_be_bytes());
    let mut end = reason.len().min(123);
    while end > 0 && !reason.is_char_boundary(end) {
        end -= 1;
    }
    payload.extend_from_slice(&reason.as_bytes()[..end]);
    write_ws_frame(stream, 0x8, &payload)
}

struct WsFrame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

enum WsReadError {
    Protocol(u16, &'static str),
    Io,
}

struct WsConnReader {
    buf: Vec<u8>,
}

impl WsConnReader {
    fn ensure_bytes(&mut self, stream: &mut TcpStream, needed: usize) -> Result<(), WsReadError> {
        let deadline = Instant::now() + WS_FRAME_BODY_DEADLINE;
        let mut tmp = [0u8; 4096];
        while self.buf.len() < needed {
            let Some(rem) = remaining_timeout(deadline) else {
                return Err(WsReadError::Io);
            };
            let _ = stream.set_read_timeout(Some(rem));
            match stream.read(&mut tmp) {
                Ok(0) => {
                    return Err(WsReadError::Io);
                }
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if Instant::now() >= deadline {
                        return Err(WsReadError::Io);
                    }
                }
                Err(_) => return Err(WsReadError::Io),
            }
        }
        Ok(())
    }

    fn read_frame(&mut self, stream: &mut TcpStream) -> Result<Option<WsFrame>, WsReadError> {
        if self.buf.is_empty() {
            let _ = stream.set_read_timeout(Some(WS_POLL_TIMEOUT));
            let mut tmp = [0u8; 4096];
            match stream.read(&mut tmp) {
                Ok(0) => {
                    return Err(WsReadError::Io);
                }
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => return Ok(None),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return Ok(None);
                }
                Err(_) => return Err(WsReadError::Io),
            }
        }

        self.ensure_bytes(stream, 2)?;
        let fin = (self.buf[0] & 0x80) != 0;
        if (self.buf[0] & 0x70) != 0 {
            return Err(WsReadError::Protocol(1002, "non_zero_rsv"));
        }
        let opcode = self.buf[0] & 0x0F;
        let masked = (self.buf[1] & 0x80) != 0;
        if !masked {
            return Err(WsReadError::Protocol(1002, "unmasked_client_frame"));
        }

        let len7 = self.buf[1] & 0x7F;
        let (offset, payload_len_u64) = if len7 <= 125 {
            (2usize, u64::from(len7))
        } else if len7 == 126 {
            self.ensure_bytes(stream, 4)?;
            (4usize, u64::from(u16::from_be_bytes([self.buf[2], self.buf[3]])))
        } else {
            self.ensure_bytes(stream, 10)?;
            let v = u64::from_be_bytes([
                self.buf[2],
                self.buf[3],
                self.buf[4],
                self.buf[5],
                self.buf[6],
                self.buf[7],
                self.buf[8],
                self.buf[9],
            ]);
            if (v & (1u64 << 63)) != 0 {
                return Err(WsReadError::Protocol(1002, "invalid_64bit_length_msb"));
            }
            (10usize, v)
        };

        if (opcode & 0x08) != 0 && (!fin || payload_len_u64 > 125) {
            return Err(WsReadError::Protocol(1002, "invalid_control_frame"));
        }

        if payload_len_u64 > MAX_WS_FRAME_PAYLOAD_BYTES as u64 {
            return Err(WsReadError::Protocol(1009, "frame_too_large"));
        }

        let payload_len = payload_len_u64 as usize;
        let total = offset + 4 + payload_len;
        self.ensure_bytes(stream, total)?;

        let mask_key = [
            self.buf[offset],
            self.buf[offset + 1],
            self.buf[offset + 2],
            self.buf[offset + 3],
        ];
        let mut payload = self.buf[offset + 4..total].to_vec();
        self.buf.drain(..total);

        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask_key[i % 4];
        }

        Ok(Some(WsFrame {
            fin,
            opcode,
            payload,
        }))
    }
}

fn dispatch_ws_message(
    stream: &mut TcpStream,
    op: u8,
    payload: Vec<u8>,
    state: &Arc<Mutex<ServerState>>,
) -> bool {
    if op == 0x1 {
        let text = String::from_utf8_lossy(&payload).into_owned();
        if let Ok(mut g) = state.lock() {
            g.ws_received_frames.push(text.clone());
        }
        let echo = json!({
            "type": "ws_ack",
            "echo": text,
            "timestamp_ms": now_ms()
        })
        .to_string();
        write_ws_frame(stream, 0x1, echo.as_bytes()).is_ok()
    } else if op == 0x2 {
        write_ws_frame(stream, 0x2, &payload).is_ok()
    } else {
        true
    }
}

fn handle_websocket(
    mut stream: TcpStream,
    ws_key: &str,
    state: Arc<Mutex<ServerState>>,
    shutdown: Arc<AtomicBool>,
) {
    let should_drop = state
        .lock()
        .map(|mut g| {
            let d = g.drop_next_ws;
            g.drop_next_ws = false;
            d
        })
        .unwrap_or(false);
    if should_drop {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }

    let accept = compute_websocket_accept(ws_key);
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if stream.write_all(handshake.as_bytes()).is_err() {
        return;
    }
    let _ = stream.flush();

    let mut reader = WsConnReader { buf: Vec::new() };
    let mut frag_opcode: Option<u8> = None;
    let mut frag_buf: Vec<u8> = Vec::new();

    while !shutdown.load(Ordering::SeqCst) {
        let queued: Vec<String> = state
            .lock()
            .map(|mut g| {
                if g.offline {
                    Vec::new()
                } else {
                    g.ws_push_queue.drain(..).collect()
                }
            })
            .unwrap_or_default();

        if state.lock().map(|g| g.offline).unwrap_or(false) {
            let _ = write_ws_close(&mut stream, 1001, "offline");
            break;
        }

        for msg in queued {
            if write_ws_frame(&mut stream, 0x1, msg.as_bytes()).is_err() {
                return;
            }
        }

        match reader.read_frame(&mut stream) {
            Ok(Some(frame)) => match frame.opcode {
                0x9 => {
                    if write_ws_frame(&mut stream, 0xA, &frame.payload).is_err() {
                        break;
                    }
                }
                0xA => {}
                0x8 => {
                    let _ = write_ws_close(&mut stream, 1000, "");
                    break;
                }
                0x1 | 0x2 => {
                    if frag_opcode.is_some() {
                        let _ = write_ws_close(&mut stream, 1002, "interleaved_data_frame");
                        break;
                    }
                    if frame.fin {
                        if !dispatch_ws_message(&mut stream, frame.opcode, frame.payload, &state) {
                            break;
                        }
                    } else {
                        if frame.payload.len() > MAX_WS_MESSAGE_BYTES {
                            let _ = write_ws_close(&mut stream, 1009, "message_too_large");
                            break;
                        }
                        frag_opcode = Some(frame.opcode);
                        frag_buf = frame.payload;
                    }
                }
                0x0 => {
                    if frag_opcode.is_none() {
                        let _ = write_ws_close(&mut stream, 1002, "unexpected_continuation");
                        break;
                    }
                    if frag_buf.len().saturating_add(frame.payload.len()) > MAX_WS_MESSAGE_BYTES {
                        let _ = write_ws_close(&mut stream, 1009, "message_too_large");
                        break;
                    }
                    frag_buf.extend_from_slice(&frame.payload);
                    if frame.fin {
                        let op = frag_opcode.take().unwrap_or(0x1);
                        let complete = std::mem::take(&mut frag_buf);
                        if !dispatch_ws_message(&mut stream, op, complete, &state) {
                            break;
                        }
                    }
                }
                _ => {
                    let _ = write_ws_close(&mut stream, 1002, "unsupported_opcode");
                    break;
                }
            },
            Ok(None) => {}
            Err(WsReadError::Protocol(code, reason)) => {
                let _ = write_ws_close(&mut stream, code, reason);
                break;
            }
            Err(WsReadError::Io) => break,
        }
    }
}

fn parse_byte_range(header: &str, total: usize) -> Option<Result<(usize, usize), ()>> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    if total == 0 {
        return Some(Err(()));
    }
    let start_trimmed = start_s.trim();
    let end_trimmed = end_s.trim();
    if start_trimmed.is_empty() {
        // Suffix byte range: `bytes=-N`
        let suffix: usize = end_trimmed.parse().ok()?;
        if suffix == 0 {
            return Some(Err(()));
        }
        let start = total.saturating_sub(suffix);
        return Some(Ok((start, total - 1)));
    }
    let start: usize = start_trimmed.parse().ok()?;
    if start >= total {
        return Some(Err(()));
    }
    let end = if end_trimmed.is_empty() {
        total - 1
    } else {
        let parsed: usize = end_trimmed.parse().ok()?;
        parsed.min(total - 1)
    };
    if end < start {
        return Some(Err(()));
    }
    Some(Ok((start, end)))
}

fn extract_query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key && !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn extract_bearer_token(req: &RecordedRequest) -> Option<String> {
    if let Some(auth) = req.headers.get("authorization") {
        let trimmed = auth.trim();
        if trimmed.len() > 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
            let tok = trimmed[7..].trim();
            if !tok.is_empty() {
                return Some(tok.to_owned());
            }
        }
    }
    extract_query_param(&req.query, "token").map(str::to_owned)
}

fn extract_media_bytes(req: &RecordedRequest) -> Vec<u8> {
    let is_json = req
        .headers
        .get("content-type")
        .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("application/json"));
    if is_json {
        if let Ok(val) = serde_json::from_slice::<Value>(&req.body) {
            if let Some(b64) = val.get("data_b64").and_then(Value::as_str) {
                if let Ok(decoded) = B64.decode(b64) {
                    return decoded;
                }
            }
        }
    }
    req.body.clone()
}

fn resolve_media_get_id(path: &str) -> Option<&str> {
    if let Some(id) = path.strip_prefix("/api/v1/media/blob/") {
        return (!id.is_empty()).then_some(id);
    }
    if let Some(id) = path.strip_prefix("/api/v1/media/stream/") {
        return (!id.is_empty()).then_some(id);
    }
    if let Some(rest) = path.strip_prefix("/api/v1/media/") {
        if !rest.is_empty() && !rest.contains('/') && !rest.starts_with("upload") {
            return Some(rest);
        }
    }
    None
}

fn is_protected_route(path: &str) -> bool {
    if matches!(
        path,
        "/api/v1/auth/2fa/verify"
            | "/api/v1/auth/two-factor/verify"
            | "/api/v1/auth/pair/init"
            | "/api/v1/auth/pair/verify"
            | "/api/v1/auth/login"
            | "/api/v1/auth/register"
            | "/api/v1/auth/kdf-params"
    ) || path.starts_with("/api/v1/auth/pair/poll")
    {
        return false;
    }
    path == "/api/v1/auth/me"
        || path.starts_with("/api/v1/auth/sessions")
        || path == "/api/v1/auth/2fa"
        || path.starts_with("/api/v1/auth/two-factor/")
        || path == "/api/v1/auth/pair/accept"
        || path == "/api/v1/auth/upgrade-split-key"
        || path == "/api/v1/auth/change-password"
        || path.starts_with("/api/v1/chats")
        || path.starts_with("/api/v1/messages")
        || path.starts_with("/api/v1/sync")
        || path.starts_with("/api/v1/user-vault")
        || path.starts_with("/api/v1/vault")
        || path.starts_with("/api/v1/folders")
        || path.starts_with("/api/v1/contacts")
        || path.starts_with("/api/v1/settings")
}

fn handle_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<ServerState>>,
    shutdown: Arc<AtomicBool>,
) {
    if state.lock().map(|g| g.offline).unwrap_or(false) {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }

    let req = match read_http_request(&mut stream) {
        Ok(r) => r,
        Err(HttpReadError::Closed) => return,
        Err(HttpReadError::BadRequest(msg)) => {
            let body = json!({"error": "bad_request", "message": msg}).to_string();
            write_http_response(&mut stream, 400, "application/json", &[], body.as_bytes());
            return;
        }
        Err(HttpReadError::PayloadTooLarge) => {
            let body = json!({"error": "payload_too_large"}).to_string();
            write_http_response(&mut stream, 413, "application/json", &[], body.as_bytes());
            return;
        }
    };

    // Record request & check injected delay / fault.
    let (delay_ms, fault) = {
        let mut guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.recorded.push(req.clone());

        let delay = guard
            .delays
            .iter()
            .find(|(prefix, _)| req.path.starts_with(prefix.as_str()))
            .map(|(_, ms)| *ms)
            .unwrap_or(0);

        let mut matched_fault = None;
        for f in &mut guard.faults {
            if req.path.starts_with(&f.path_prefix) {
                match f.remaining_hits {
                    Some(0) => continue,
                    Some(ref mut rem) => {
                        *rem -= 1;
                        matched_fault = Some(f.clone());
                        break;
                    }
                    None => {
                        matched_fault = Some(f.clone());
                        break;
                    }
                }
            }
        }
        (delay, matched_fault)
    };

    if delay_ms > 0 {
        thread::sleep(Duration::from_millis(delay_ms));
    }

    // RFC 6455 WebSocket upgrade check.
    if req
        .headers
        .get("upgrade")
        .is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
    {
        if let Some(ws_key) = req.headers.get("sec-websocket-key").cloned() {
            handle_websocket(stream, &ws_key, state, shutdown);
            return;
        }
    }

    if let Some(f) = fault {
        let mut extras = Vec::new();
        if let Some(ms) = f.retry_after_ms {
            extras.push(("Retry-After", ms.div_ceil(1000).to_string()));
        }
        write_http_response(
            &mut stream,
            f.status,
            "application/json",
            &extras,
            f.body.as_bytes(),
        );
        return;
    }

    // Binary / raw media upload routes MUST execute before JSON-only parsing.
    if req.method == "POST" && req.path == "/api/v1/media/upload" {
        let blob_bytes = extract_media_bytes(&req);
        let digest = hex_sha256(&blob_bytes);
        let media_id = format!("cas_{}", &digest[..16]);
        let size = blob_bytes.len();
        if let Ok(mut g) = state.lock() {
            g.media_blobs.insert(media_id.clone(), blob_bytes.clone());
            g.media_blobs.insert(digest.clone(), blob_bytes);
        }
        let body = json!({
            "media_id": media_id,
            "file_id": media_id,
            "sha256": digest,
            "size": size
        })
        .to_string();
        write_http_response(&mut stream, 200, "application/json", &[], body.as_bytes());
        return;
    }

    if matches!(req.method.as_str(), "PUT" | "POST")
        && req.path.starts_with("/api/v1/media/upload/")
        && req.path.contains("/part/")
    {
        let rest = req.path.trim_start_matches("/api/v1/media/upload/");
        if let Some((upload_id, part_s)) = rest.split_once("/part/") {
            let (status, resp_json) = match part_s.parse::<usize>() {
                Ok(part_index) => {
                    let bytes = extract_media_bytes(&req);
                    let mut g = state.lock().expect("state lock");
                    if let Some(session) = g.uploads.get_mut(upload_id) {
                        session.parts.insert(part_index, bytes);
                        let received: Vec<usize> = session.parts.keys().copied().collect();
                        (
                            200,
                            json!({
                                "upload_id": session.upload_id,
                                "part_index": part_index,
                                "received_parts": received
                            }),
                        )
                    } else {
                        (404, json!({"error": "upload_not_found"}))
                    }
                }
                Err(_) => (400, json!({"error": "invalid_part_index"})),
            };
            let body = resp_json.to_string();
            write_http_response(&mut stream, status, "application/json", &[], body.as_bytes());
            return;
        }
    }

    // Unified GET media retrieval (200 / 206 / 416 / 404).
    if req.method == "GET" {
        if let Some(file_id) = resolve_media_get_id(&req.path) {
            let maybe_blob = state
                .lock()
                .ok()
                .and_then(|g| g.media_blobs.get(file_id).cloned());
            let Some(blob) = maybe_blob else {
                let body = json!({"error": "media_not_found"}).to_string();
                write_http_response(&mut stream, 404, "application/json", &[], body.as_bytes());
                return;
            };
            if let Some(range_hdr) = req.headers.get("range") {
                match parse_byte_range(range_hdr, blob.len()) {
                    Some(Ok((start, end))) => {
                        let slice = &blob[start..=end];
                        let cr = format!("bytes {start}-{end}/{}", blob.len());
                        write_http_response(
                            &mut stream,
                            206,
                            "application/octet-stream",
                            &[
                                ("Accept-Ranges", "bytes".to_owned()),
                                ("Content-Range", cr),
                            ],
                            slice,
                        );
                        return;
                    }
                    Some(Err(())) | None => {
                        let cr = format!("bytes */{}", blob.len());
                        let body = json!({"error": "range_not_satisfiable"}).to_string();
                        write_http_response(
                            &mut stream,
                            416,
                            "application/json",
                            &[("Content-Range", cr)],
                            body.as_bytes(),
                        );
                        return;
                    }
                }
            }
            write_http_response(
                &mut stream,
                200,
                "application/octet-stream",
                &[("Accept-Ranges", "bytes".to_owned())],
                &blob,
            );
            return;
        }
    }

    // Stateful JSON API routes backing F01..F34 loopback flows.
    let req_json: Value = serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
    let (status, resp_json) = route_api_request(&req, &req_json, &state);
    let bytes = resp_json.to_string().into_bytes();
    write_http_response(&mut stream, status, "application/json", &[], &bytes);
}

fn compute_device_sync_code(request_id: &str, pub_b64: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"DEVICE_SYNC_CODE_DOMAIN_V1");
    hasher.update(request_id.as_bytes());
    hasher.update(pub_b64.as_bytes());
    let digest = hasher.finalize();
    let num = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{num:06}")
}

fn stored_message_to_json(m: &StoredMessage) -> Value {
    json!({
        "message_id": m.message_id,
        "chat_id": m.chat_id,
        "pts": m.pts,
        "text": m.text,
        "content": m.content,
        "client_nonce": m.client_nonce,
        "delivery_status": "sent",
        "timestamp_ms": m.timestamp_ms
    })
}

fn route_api_request(
    req: &RecordedRequest,
    body: &Value,
    state: &Arc<Mutex<ServerState>>,
) -> (u16, Value) {
    let method = req.method.as_str();
    let path = req.path.as_str();
    let query = req.query.as_str();
    let ts = now_ms();
    let bearer_token = extract_bearer_token(req);

    // Enforce Bearer revocation / invalidation on all protected routes.
    let caller_session: Option<SessionRecord> = {
        let g = state.lock().expect("state lock");
        if let Some(ref tok) = bearer_token {
            if is_protected_route(path) {
                let is_revoked = g.revoked_tokens.contains(tok)
                    || g.sessions_by_token.get(tok).is_some_and(|s| s.revoked)
                    || (tok.starts_with("tok_") && !g.sessions_by_token.contains_key(tok));
                if is_revoked {
                    return (
                        401,
                        json!({"error": "unauthorized", "message": "session_revoked"}),
                    );
                }
            }
            g.sessions_by_token.get(tok).cloned()
        } else {
            None
        }
    };

    match (method, path) {
        // Health & status
        ("GET", "/health") | ("GET", "/healthz") | ("GET", "/api/v1/health") => {
            (200, json!({"ok": true, "version": "1.6.7", "ts_ms": ts}))
        }
        ("GET", "/api/v1/status") => {
            (200, json!({"ok": true, "version": "1.6.7", "status": "ready", "ts_ms": ts}))
        }

        // Auth KDF params
        ("GET", "/api/v1/auth/kdf-params") => {
            let ident = extract_query_param(query, "identifier")
                .or_else(|| extract_query_param(query, "username"));
            let g = state.lock().expect("state lock");
            if let Some(name) = ident {
                if let Some(uid) = g.username_to_id.get(name) {
                    if let Some(user) = g.users_by_id.get(uid) {
                        return (
                            200,
                            json!({
                                "auth_version": user.auth_version,
                                "kdf_alg": if user.auth_version >= 2 { "split_key_v2" } else { "legacy_v1" },
                                "auth_salt_b64": user.auth_salt_b64,
                                "kdf_params": user.kdf_params
                            }),
                        );
                    }
                }
            }
            (
                200,
                json!({
                    "auth_version": 2,
                    "kdf_alg": "split_key_v2",
                    "auth_salt_b64": B64.encode([0x42u8; 16]),
                    "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
                }),
            )
        }

        // Auth register
        ("POST", "/api/v1/auth/register") => {
            let mut g = state.lock().expect("state lock");
            let username = body
                .get("username")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            if let Some(ref uname) = username {
                if g.username_to_id.contains_key(uname) {
                    return (409, json!({"error": "user_already_exists"}));
                }
            }
            g.next_user_id += 1;
            let uid = g.next_user_id;
            let uname = username.unwrap_or_else(|| format!("user_{uid}"));
            let auth_version = body.get("auth_version").and_then(Value::as_i64).unwrap_or(2);
            let auth_salt_b64 = body
                .get("auth_salt_b64")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| B64.encode([0x42u8; 16]));
            let auth_verifier_b64 = body
                .get("auth_verifier_b64")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let password = body
                .get("password")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let kdf_params = body
                .get("kdf_params")
                .cloned()
                .unwrap_or_else(|| json!({ "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }));
            let device_name = body
                .get("device_name")
                .and_then(Value::as_str)
                .unwrap_or("iOS E2E Primary");

            let user = UserRecord {
                user_id: uid,
                username: uname.clone(),
                auth_version,
                password,
                auth_salt_b64,
                auth_verifier_b64,
                kdf_params,
                vault_envelope: body.get("vault_envelope").cloned(),
                two_factor_password: None,
                two_factor_hint: None,
                two_factor_recovery_email: None,
            };
            g.username_to_id.insert(uname.clone(), uid);
            g.users_by_id.insert(uid, user);
            let sess = g.mint_session(uid, device_name, ts);
            (
                200,
                json!({
                    "user_id": uid,
                    "username": uname,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": sess.expires_at_ms,
                    "auth_version": auth_version,
                    "vault_unlocked": true,
                    "requires_2fa": false
                }),
            )
        }

        // Auth login
        ("POST", "/api/v1/auth/login") => {
            let mut g = state.lock().expect("state lock");
            let username_opt = body
                .get("username")
                .or_else(|| body.get("identifier"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let device_name = body
                .get("device_name")
                .and_then(Value::as_str)
                .unwrap_or("iOS Device")
                .to_owned();

            if let Some(ref uname) = username_opt {
                if let Some(&uid) = g.username_to_id.get(uname) {
                    let user = g.users_by_id.get(&uid).cloned().expect("user record");
                    if let Some(ref expected_v) = user.auth_verifier_b64 {
                        if let Some(provided_v) = body
                            .get("auth_verifier_b64")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            if provided_v != expected_v {
                                return (401, json!({"error": "invalid_credentials"}));
                            }
                        } else if let Some(ref expected_pw) = user.password {
                            let provided_pw =
                                body.get("password").and_then(Value::as_str).unwrap_or("");
                            if provided_pw != expected_pw {
                                return (401, json!({"error": "invalid_credentials"}));
                            }
                        }
                    } else if let Some(ref expected_pw) = user.password {
                        let provided_pw =
                            body.get("password").and_then(Value::as_str).unwrap_or("");
                        if provided_pw != expected_pw {
                            return (401, json!({"error": "invalid_credentials"}));
                        }
                    }

                    if user.two_factor_password.is_some() {
                        let seq = g.alloc_seq();
                        let challenge_token = format!("tfa_{uid}_{seq}");
                        g.two_factor_challenges.insert(
                            challenge_token.clone(),
                            TwoFactorChallenge {
                                user_id: uid,
                                device_name,
                                expires_at_ms: ts + 300_000,
                            },
                        );
                        return (
                            200,
                            json!({
                                "requires_2fa": true,
                                "challenge_token": challenge_token,
                                "hint": user.two_factor_hint,
                                "user_id": uid
                            }),
                        );
                    }

                    let sess = g.mint_session(uid, &device_name, ts);
                    return (
                        200,
                        json!({
                            "user_id": uid,
                            "username": user.username,
                            "session_id": sess.session_id,
                            "token": sess.token,
                            "expires_at": sess.expires_at_ms,
                            "auth_version": user.auth_version,
                            "vault_envelope": user.vault_envelope,
                            "vault_unlocked": true,
                            "requires_2fa": false
                        }),
                    );
                }
            }

            // Auto-provision user for unseeded login flows.
            g.next_user_id += 1;
            let uid = g.next_user_id;
            let uname = username_opt.unwrap_or_else(|| format!("user_{uid}"));
            let auth_version = body.get("auth_version").and_then(Value::as_i64).unwrap_or(2);
            let user = UserRecord {
                user_id: uid,
                username: uname.clone(),
                auth_version,
                password: body
                    .get("password")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                auth_salt_b64: body
                    .get("auth_salt_b64")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| B64.encode([0x42u8; 16])),
                auth_verifier_b64: body
                    .get("auth_verifier_b64")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                kdf_params: body
                    .get("kdf_params")
                    .cloned()
                    .unwrap_or_else(|| json!({ "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 })),
                vault_envelope: body.get("vault_envelope").cloned(),
                two_factor_password: None,
                two_factor_hint: None,
                two_factor_recovery_email: None,
            };
            g.username_to_id.insert(uname.clone(), uid);
            g.users_by_id.insert(uid, user);
            let sess = g.mint_session(uid, &device_name, ts);
            (
                200,
                json!({
                    "user_id": uid,
                    "username": uname,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": sess.expires_at_ms,
                    "auth_version": auth_version,
                    "vault_unlocked": true,
                    "requires_2fa": false
                }),
            )
        }

        // Auth upgrade / change-password / me
        ("POST", "/api/v1/auth/upgrade-split-key") | ("POST", "/api/v1/auth/change-password") => {
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let user = g.ensure_user_by_id(uid);
            user.auth_version = body.get("auth_version").and_then(Value::as_i64).unwrap_or(2);
            if let Some(salt) = body.get("auth_salt_b64").and_then(Value::as_str) {
                user.auth_salt_b64 = salt.to_owned();
            }
            if let Some(v) = body.get("auth_verifier_b64").and_then(Value::as_str) {
                user.auth_verifier_b64 = Some(v.to_owned());
            }
            if let Some(kdf) = body.get("kdf_params") {
                user.kdf_params = kdf.clone();
            }
            if let Some(env) = body.get("vault_envelope") {
                user.vault_envelope = Some(env.clone());
            }
            let av = user.auth_version;
            (200, json!({"ok": true, "upgraded": true, "auth_version": av}))
        }
        ("GET", "/api/v1/auth/me") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            if let Some(u) = g.users_by_id.get(&uid) {
                (
                    200,
                    json!({
                        "user_id": u.user_id,
                        "username": u.username,
                        "auth_version": u.auth_version,
                        "two_factor_enabled": u.two_factor_password.is_some()
                    }),
                )
            } else {
                (200, json!({"user_id": uid, "username": format!("user_{uid}"), "auth_version": 2}))
            }
        }

        // Auth sessions listing & revocation
        ("GET", "/api/v1/auth/sessions") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let current_sid = caller_session.as_ref().map(|s| s.session_id.as_str());
            let mut list: Vec<Value> = g
                .sessions_by_token
                .values()
                .filter(|s| s.user_id == uid && !s.revoked)
                .map(|s| {
                    json!({
                        "session_id": s.session_id,
                        "device_name": s.device_name,
                        "created_at_ms": s.created_at_ms,
                        "expires_at_ms": s.expires_at_ms,
                        "is_current": current_sid == Some(s.session_id.as_str())
                    })
                })
                .collect();
            list.sort_by(|a, b| {
                a["session_id"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["session_id"].as_str().unwrap_or(""))
            });
            (200, json!({"sessions": list}))
        }
        ("DELETE", "/api/v1/auth/sessions/others") => {
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let current_sid = caller_session
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            let mut revoked_tokens = Vec::new();
            for sess in g.sessions_by_token.values_mut() {
                if sess.user_id == uid && sess.session_id != current_sid && !sess.revoked {
                    sess.revoked = true;
                    revoked_tokens.push(sess.token.clone());
                }
            }
            let revoked_count = revoked_tokens.len();
            for tok in revoked_tokens {
                g.revoked_tokens.insert(tok);
            }
            (200, json!({"ok": true, "revoked_count": revoked_count}))
        }
        ("DELETE", p) if p.starts_with("/api/v1/auth/sessions/") => {
            let sid = p.trim_start_matches("/api/v1/auth/sessions/");
            let mut g = state.lock().expect("state lock");
            let Some(tok) = g.session_id_to_token.get(sid).cloned() else {
                return (404, json!({"error": "session_not_found"}));
            };
            if let Some(sess) = g.sessions_by_token.get_mut(&tok) {
                sess.revoked = true;
            }
            g.revoked_tokens.insert(tok);
            let gen_num = g.alloc_seq();
            (
                200,
                json!({
                    "revoked": true,
                    "session_id": sid,
                    "account_generation": gen_num
                }),
            )
        }

        // Cloud 2FA status / set / disable / verify
        ("GET", "/api/v1/auth/2fa") | ("GET", "/api/v1/auth/two-factor/status") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let user = g.users_by_id.get(&uid);
            (
                200,
                json!({
                    "enabled": user.and_then(|u| u.two_factor_password.as_ref()).is_some(),
                    "hint": user.and_then(|u| u.two_factor_hint.clone()),
                    "recovery_email": user.and_then(|u| u.two_factor_recovery_email.clone())
                }),
            )
        }
        ("POST", "/api/v1/auth/2fa")
        | ("PUT", "/api/v1/auth/2fa")
        | ("POST", "/api/v1/auth/two-factor/set") => {
            let new_pw = body
                .get("new_password")
                .or_else(|| body.get("password"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if new_pw.is_empty() || new_pw.len() > 1024 {
                return (400, json!({"error": "invalid_password"}));
            }
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let user = g.ensure_user_by_id(uid);
            if let Some(ref existing) = user.two_factor_password {
                let current_pw = body
                    .get("current_password")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if current_pw != existing {
                    return (401, json!({"error": "invalid_current_password"}));
                }
            }
            user.two_factor_password = Some(new_pw.to_owned());
            user.two_factor_hint = body.get("hint").and_then(Value::as_str).map(str::to_owned);
            user.two_factor_recovery_email = body
                .get("recovery_email")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (
                200,
                json!({
                    "enabled": true,
                    "hint": user.two_factor_hint,
                    "recovery_email": user.two_factor_recovery_email
                }),
            )
        }
        ("DELETE", "/api/v1/auth/2fa") | ("POST", "/api/v1/auth/two-factor/disable") => {
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let user = g.ensure_user_by_id(uid);
            if let Some(ref existing) = user.two_factor_password {
                let current_pw = body
                    .get("current_password")
                    .or_else(|| body.get("password"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if current_pw != existing {
                    return (401, json!({"error": "invalid_current_password"}));
                }
            }
            user.two_factor_password = None;
            user.two_factor_hint = None;
            (200, json!({"enabled": false}))
        }
        ("POST", "/api/v1/auth/2fa/verify") | ("POST", "/api/v1/auth/two-factor/verify") => {
            let challenge_token = body
                .get("challenge_token")
                .and_then(Value::as_str)
                .unwrap_or("");
            let cloud_pw = body
                .get("cloud_password")
                .or_else(|| body.get("password"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut g = state.lock().expect("state lock");
            let Some(chal) = g.two_factor_challenges.get(challenge_token).cloned() else {
                return (401, json!({"error": "invalid_challenge"}));
            };
            if ts > chal.expires_at_ms {
                return (401, json!({"error": "challenge_expired"}));
            }
            let expected = g
                .users_by_id
                .get(&chal.user_id)
                .and_then(|u| u.two_factor_password.clone());
            if expected.as_deref() != Some(cloud_pw) {
                return (401, json!({"error": "invalid_cloud_password"}));
            }
            g.two_factor_challenges.remove(challenge_token);
            let dev_name = body
                .get("device_name")
                .and_then(Value::as_str)
                .unwrap_or(&chal.device_name)
                .to_owned();
            let sess = g.mint_session(chal.user_id, &dev_name, ts);
            (
                200,
                json!({
                    "verified": true,
                    "user_id": chal.user_id,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": sess.expires_at_ms
                }),
            )
        }

        // QR Device Pairing
        ("POST", "/api/v1/auth/pair/init") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let pair_token = format!("pair_{ts}_{seq}");
            let device_name = body
                .get("device_name")
                .and_then(Value::as_str)
                .unwrap_or("Secondary Device")
                .to_owned();
            let expires_at_ms = ts + 300_000;
            g.qr_pairs.insert(
                pair_token.clone(),
                QrPairRecord {
                    pair_token: pair_token.clone(),
                    device_name,
                    status: "pending".to_owned(),
                    accepted_by_user_id: None,
                    requires_2fa: false,
                    issued_session: None,
                    expires_at_ms,
                },
            );
            (
                200,
                json!({
                    "pair_token": pair_token,
                    "qr_url": format!("tg://login?token={pair_token}"),
                    "expires_at_ms": expires_at_ms
                }),
            )
        }
        ("POST", "/api/v1/auth/pair/accept") => {
            let pair_token = body.get("pair_token").and_then(Value::as_str).unwrap_or("");
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let has_2fa = g
                .users_by_id
                .get(&uid)
                .and_then(|u| u.two_factor_password.as_ref())
                .is_some();
            let Some(rec) = g.qr_pairs.get(pair_token).cloned() else {
                return (400, json!({"error": "invalid_or_expired_pair_token"}));
            };
            if ts > rec.expires_at_ms || rec.status != "pending" {
                return (400, json!({"error": "invalid_or_expired_pair_token"}));
            }
            let issued = if has_2fa {
                None
            } else {
                let sess = g.mint_session(uid, &rec.device_name, ts);
                Some(json!({
                    "user_id": uid,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": sess.expires_at_ms
                }))
            };
            if let Some(entry) = g.qr_pairs.get_mut(pair_token) {
                entry.status = "confirmed".to_owned();
                entry.accepted_by_user_id = Some(uid);
                entry.requires_2fa = has_2fa;
                entry.issued_session = issued;
            }
            (200, json!({"status": "confirmed", "requires_2fa": has_2fa}))
        }
        ("GET", p) if p.starts_with("/api/v1/auth/pair/poll/") => {
            let pair_token = p.trim_start_matches("/api/v1/auth/pair/poll/");
            let g = state.lock().expect("state lock");
            let Some(rec) = g.qr_pairs.get(pair_token) else {
                return (404, json!({"error": "pair_token_not_found"}));
            };
            (
                200,
                json!({
                    "pair_token": rec.pair_token,
                    "status": rec.status,
                    "requires_2fa": rec.requires_2fa,
                    "session": rec.issued_session,
                    "expires_at_ms": rec.expires_at_ms
                }),
            )
        }
        ("POST", "/api/v1/auth/pair/poll") => {
            let pair_token = body.get("pair_token").and_then(Value::as_str).unwrap_or("");
            let g = state.lock().expect("state lock");
            let Some(rec) = g.qr_pairs.get(pair_token) else {
                return (404, json!({"error": "pair_token_not_found"}));
            };
            (
                200,
                json!({
                    "pair_token": rec.pair_token,
                    "status": rec.status,
                    "requires_2fa": rec.requires_2fa,
                    "session": rec.issued_session,
                    "expires_at_ms": rec.expires_at_ms
                }),
            )
        }
        ("POST", "/api/v1/auth/pair/verify") => {
            let pair_token = body.get("pair_token").and_then(Value::as_str).unwrap_or("");
            let cloud_pw = body.get("cloud_password").and_then(Value::as_str);
            let mut g = state.lock().expect("state lock");
            let Some(rec) = g.qr_pairs.get(pair_token).cloned() else {
                return (404, json!({"error": "pair_token_not_found"}));
            };
            if rec.status == "pending" {
                return (400, json!({"error": "pair_not_confirmed"}));
            }
            let uid = rec.accepted_by_user_id.unwrap_or_else(|| g.default_user_id());
            if rec.requires_2fa {
                let Some(pw) = cloud_pw.filter(|s| !s.is_empty()) else {
                    return (401, json!({"error": "two_factor_required"}));
                };
                let expected = g
                    .users_by_id
                    .get(&uid)
                    .and_then(|u| u.two_factor_password.clone());
                if expected.as_deref() != Some(pw) {
                    return (401, json!({"error": "invalid_cloud_password"}));
                }
            }
            let session_json = if let Some(existing) = rec.issued_session {
                existing
            } else {
                let sess = g.mint_session(uid, &rec.device_name, ts);
                json!({
                    "user_id": uid,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": sess.expires_at_ms
                })
            };
            (
                200,
                json!({
                    "verified": true,
                    "user_id": session_json["user_id"],
                    "session_id": session_json["session_id"],
                    "token": session_json["token"],
                    "expires_at": session_json["expires_at"],
                    "session": session_json
                }),
            )
        }

        // Email / Phone OTP & Alipay OAuth & Passkeys
        ("POST", "/api/v1/auth/email/send-code") => (
            200,
            json!({
                "ok": true,
                "sent": true,
                "expires_at_ms": ts + 600_000,
                "resend_after_ms": ts + 60_000
            }),
        ),
        ("POST", "/api/v1/auth/email/verify-code")
        | ("POST", "/api/v1/auth/email/register")
        | ("POST", "/api/v1/auth/phone/verify-code")
        | ("POST", "/api/v1/auth/phone/login")
        | ("POST", "/api/v1/auth/passkey/register")
        | ("POST", "/api/v1/auth/passkey/login")
        | ("POST", "/api/v1/auth/passkey/authenticate") => {
            let mut g = state.lock().expect("state lock");
            g.next_user_id += 1;
            let uid = g.next_user_id;
            let sess = g.mint_session(uid, "iOS Auth Device", ts);
            (
                200,
                json!({
                    "ok": true,
                    "authenticated": true,
                    "user_id": uid,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": sess.expires_at_ms
                }),
            )
        }
        ("POST", "/api/v1/auth/phone/send-code") => (
            200,
            json!({
                "ok": true,
                "sent": true,
                "phone_code_hash": format!("pch_{ts}"),
                "expires_at_ms": ts + 300_000,
                "resend_after_ms": ts + 60_000
            }),
        ),
        ("POST", "/api/v1/auth/alipay/start") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            (
                200,
                json!({
                    "attempt_id": format!("alipay_{seq}"),
                    "state": format!("st_{ts}_{seq}"),
                    "auth_url": format!("https://openauth.alipay.com/oauth2/publicAppAuthorize.htm?state=st_{ts}_{seq}"),
                    "expires_at_ms": ts + 600_000
                }),
            )
        }
        ("GET", p) if p.starts_with("/api/v1/auth/alipay/poll/") => {
            let attempt_id = p.trim_start_matches("/api/v1/auth/alipay/poll/");
            let confirmed = state
                .lock()
                .ok()
                .and_then(|g| g.oauth_confirmed.get(attempt_id).cloned());
            match confirmed {
                Some(session) => (
                    200,
                    json!({
                        "attempt_id": attempt_id,
                        "status": "confirmed",
                        "session": session,
                        "expires_at_ms": ts + 600_000
                    }),
                ),
                None => (
                    200,
                    json!({
                        "attempt_id": attempt_id,
                        "status": "pending",
                        "expires_at_ms": ts + 600_000
                    }),
                ),
            }
        }
        ("POST", "/api/v1/auth/alipay/callback") | ("POST", "/api/v1/auth/alipay/exchange") => {
            let mut g = state.lock().expect("state lock");
            g.next_user_id += 1;
            let uid = g.next_user_id;
            let sess = g.mint_session(uid, "iOS Alipay", ts);
            (
                200,
                json!({
                    "authenticated": true,
                    "user_id": uid,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": sess.expires_at_ms
                }),
            )
        }

        // Chats, invites & join
        ("GET", "/api/v1/chats") => {
            let g = state.lock().expect("state lock");
            let mut chats: Vec<Value> = g
                .chats
                .values()
                .map(|c| {
                    json!({
                        "chat_id": c.chat_id,
                        "title": c.title,
                        "chat_type": c.chat_type,
                        "member_count": c.member_count
                    })
                })
                .collect();
            chats.sort_by_key(|v| v["chat_id"].as_i64().unwrap_or(0));
            (200, json!({"chats": chats, "total": chats.len()}))
        }
        ("POST", "/api/v1/chats") | ("POST", "/api/v1/chats/create") => {
            let mut g = state.lock().expect("state lock");
            g.next_chat_id += 1;
            let cid = g.next_chat_id;
            let title = body
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Chat")
                .to_owned();
            let chat_type = body.get("chat_type").and_then(Value::as_u64).unwrap_or(0);
            let rec = ChatRecord {
                chat_id: cid,
                title: title.clone(),
                chat_type,
                member_count: 2,
            };
            g.chats.insert(cid, rec);
            g.chat_pts.entry(cid).or_insert(0);
            (
                200,
                json!({
                    "chat_id": cid,
                    "title": title,
                    "chat_type": chat_type,
                    "member_count": 2
                }),
            )
        }
        ("POST", "/api/v1/chats/join") => {
            let code = body.get("code").and_then(Value::as_str).unwrap_or("");
            let mut g = state.lock().expect("state lock");
            let Some(inv) = g.chat_invites.get_mut(code) else {
                return (404, json!({"error": "invalid_invite"}));
            };
            if inv.revoked || inv.max_uses.is_some_and(|m| inv.uses >= m) {
                return (400, json!({"error": "invalid_invite"}));
            }
            inv.uses += 1;
            let cid = inv.chat_id;
            if let Some(chat) = g.chats.get_mut(&cid) {
                chat.member_count += 1;
            }
            (200, json!({"chat_id": cid, "joined": true}))
        }
        ("POST", p) if p.starts_with("/api/v1/chats/") && p.ends_with("/invites") => {
            let cid: i64 = p
                .trim_start_matches("/api/v1/chats/")
                .trim_end_matches("/invites")
                .parse()
                .unwrap_or(11);
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let invite_id = format!("inv_{seq}");
            let code = format!("join_{cid}_{seq}");
            let max_uses = body.get("max_uses").and_then(Value::as_u64);
            let rec = ChatInviteRecord {
                invite_id: invite_id.clone(),
                chat_id: cid,
                code: code.clone(),
                max_uses,
                uses: 0,
                revoked: false,
            };
            g.chat_invites.insert(code.clone(), rec);
            (
                200,
                json!({
                    "invite_id": invite_id,
                    "chat_id": cid,
                    "code": code,
                    "max_uses": max_uses,
                    "uses": 0,
                    "revoked": false
                }),
            )
        }
        ("GET", p) if p.starts_with("/api/v1/chats/") && p.ends_with("/invites") => {
            let cid: i64 = p
                .trim_start_matches("/api/v1/chats/")
                .trim_end_matches("/invites")
                .parse()
                .unwrap_or(11);
            let g = state.lock().expect("state lock");
            let invites: Vec<Value> = g
                .chat_invites
                .values()
                .filter(|i| i.chat_id == cid)
                .map(|i| {
                    json!({
                        "invite_id": i.invite_id,
                        "chat_id": i.chat_id,
                        "code": i.code,
                        "max_uses": i.max_uses,
                        "uses": i.uses,
                        "revoked": i.revoked
                    })
                })
                .collect();
            (200, json!({"invites": invites}))
        }
        ("DELETE", p) | ("POST", p)
            if p.starts_with("/api/v1/chats/") && p.contains("/invites/") =>
        {
            let tail = p.split("/invites/").nth(1).unwrap_or("");
            let iid = tail.trim_end_matches("/revoke");
            let mut g = state.lock().expect("state lock");
            let mut found = false;
            for inv in g.chat_invites.values_mut() {
                if inv.invite_id == iid || inv.code == iid {
                    inv.revoked = true;
                    found = true;
                }
            }
            if found {
                (200, json!({"revoked": true, "invite_id": iid}))
            } else {
                (404, json!({"error": "invite_not_found"}))
            }
        }

        // Messages & Per-Chat PTS Journal
        ("POST", p)
            if (p.starts_with("/api/v1/chats/") && p.ends_with("/messages"))
                || p == "/api/v1/messages/send" =>
        {
            let cid = if p.starts_with("/api/v1/chats/") {
                p.trim_start_matches("/api/v1/chats/")
                    .trim_end_matches("/messages")
                    .parse::<i64>()
                    .unwrap_or(1)
            } else {
                body.get("chat_id").and_then(Value::as_i64).unwrap_or(1)
            };
            let client_nonce = body.get("client_nonce").and_then(Value::as_u64);
            let mut g = state.lock().expect("state lock");
            if let Some(nonce) = client_nonce {
                if let Some(existing) = g.chat_nonce_map.get(&(cid, nonce)).cloned() {
                    return (200, stored_message_to_json(&existing));
                }
            }
            g.next_msg_id += 1;
            let mid = g.next_msg_id;
            let pts = {
                let entry = g.chat_pts.entry(cid).or_insert(0);
                *entry += 1;
                *entry
            };
            let msg = StoredMessage {
                message_id: mid,
                chat_id: cid,
                pts,
                text: body
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                content: body.get("content").cloned().unwrap_or_else(|| json!({})),
                client_nonce,
                timestamp_ms: ts,
            };
            g.chat_messages.entry(cid).or_default().push(msg.clone());
            if let Some(nonce) = client_nonce {
                g.chat_nonce_map.insert((cid, nonce), msg.clone());
            }
            (200, stored_message_to_json(&msg))
        }
        ("GET", p)
            if (p.starts_with("/api/v1/chats/") && p.ends_with("/messages"))
                || p == "/api/v1/messages/history" =>
        {
            let cid = if p.starts_with("/api/v1/chats/") {
                p.trim_start_matches("/api/v1/chats/")
                    .trim_end_matches("/messages")
                    .parse::<i64>()
                    .unwrap_or(1)
            } else {
                extract_query_param(query, "chat_id")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(1)
            };
            let g = state.lock().expect("state lock");
            let msgs: Vec<Value> = g
                .chat_messages
                .get(&cid)
                .map(|v| v.iter().map(stored_message_to_json).collect())
                .unwrap_or_default();
            (200, json!({"chat_id": cid, "messages": msgs}))
        }

        // PTS sync difference
        ("POST", "/api/v1/sync/get_difference") | ("GET", "/api/v1/sync/difference") => {
            let chat_id = body
                .get("chat_id")
                .and_then(Value::as_i64)
                .or_else(|| extract_query_param(query, "chat_id").and_then(|s| s.parse().ok()))
                .unwrap_or(1);
            let req_pts = body
                .get("pts")
                .and_then(Value::as_u64)
                .or_else(|| extract_query_param(query, "pts").and_then(|s| s.parse().ok()))
                .unwrap_or(0);
            let limit = body
                .get("limit")
                .and_then(Value::as_u64)
                .or_else(|| extract_query_param(query, "limit").and_then(|s| s.parse().ok()))
                .unwrap_or(100)
                .max(1) as usize;

            let g = state.lock().expect("state lock");
            let floor = g.journal_floors.get(&chat_id).copied().unwrap_or(1);
            let current_pts = g.chat_pts.get(&chat_id).copied().unwrap_or(0).max(floor.saturating_sub(1));
            let snapshot_required = req_pts.saturating_add(1) < floor;

            if snapshot_required {
                return (
                    200,
                    json!({
                        "chat_id": chat_id,
                        "pts": req_pts,
                        "new_pts": floor,
                        "current_pts": current_pts.max(floor),
                        "has_more": true,
                        "snapshot_required": true,
                        "messages": [],
                        "other_updates": [],
                        "updates": []
                    }),
                );
            }

            let mut matching: Vec<&StoredMessage> = g
                .chat_messages
                .get(&chat_id)
                .map(|v| v.iter().filter(|m| m.pts > req_pts).collect())
                .unwrap_or_default();
            matching.sort_by_key(|m| m.pts);
            let has_more = matching.len() > limit;
            matching.truncate(limit);
            let new_pts = matching.last().map(|m| m.pts).unwrap_or(req_pts);
            let records: Vec<Value> = matching.into_iter().map(stored_message_to_json).collect();
            (
                200,
                json!({
                    "chat_id": chat_id,
                    "pts": req_pts,
                    "new_pts": new_pts,
                    "current_pts": current_pts.max(new_pts),
                    "has_more": has_more,
                    "snapshot_required": false,
                    "messages": records,
                    "other_updates": [],
                    "updates": records
                }),
            )
        }

        // Resumable chunked media upload init / status / complete
        ("POST", "/api/v1/media/upload/init") => {
            let total_size = body.get("total_size").and_then(Value::as_u64).unwrap_or(0);
            let part_size = body
                .get("part_size")
                .and_then(Value::as_u64)
                .unwrap_or(65_536);
            if total_size > MAX_UPLOAD_TOTAL_SIZE {
                return (400, json!({"error": "upload_size_exceeded"}));
            }
            if part_size == 0 || part_size > MAX_UPLOAD_PART_SIZE {
                return (400, json!({"error": "invalid_part_size"}));
            }
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let upload_id = format!("upl_{seq}");
            let expected_sha256 = body
                .get("expected_sha256")
                .and_then(Value::as_str)
                .map(str::to_owned);
            g.uploads.insert(
                upload_id.clone(),
                UploadSession {
                    upload_id: upload_id.clone(),
                    total_size,
                    part_size,
                    expected_sha256,
                    parts: BTreeMap::new(),
                },
            );
            (
                200,
                json!({
                    "upload_id": upload_id,
                    "part_size": part_size,
                    "total_size": total_size
                }),
            )
        }
        ("GET", p) if p.starts_with("/api/v1/media/upload/") && !p.contains("/part/") => {
            let upload_id = p.trim_start_matches("/api/v1/media/upload/");
            let g = state.lock().expect("state lock");
            let Some(session) = g.uploads.get(upload_id) else {
                return (404, json!({"error": "upload_not_found"}));
            };
            let received: Vec<usize> = session.parts.keys().copied().collect();
            (
                200,
                json!({
                    "upload_id": session.upload_id,
                    "total_size": session.total_size,
                    "part_size": session.part_size,
                    "received_parts": received
                }),
            )
        }
        ("POST", p) if p.starts_with("/api/v1/media/upload/") && p.ends_with("/complete") => {
            let upload_id = p
                .trim_start_matches("/api/v1/media/upload/")
                .trim_end_matches("/complete");
            let mut g = state.lock().expect("state lock");
            let Some(session) = g.uploads.get(upload_id).cloned() else {
                return (404, json!({"error": "upload_not_found"}));
            };
            let expected_parts = if session.total_size == 0 {
                0usize
            } else {
                session.total_size.div_ceil(session.part_size) as usize
            };
            if session.parts.len() < expected_parts {
                return (400, json!({"error": "missing_upload_parts"}));
            }
            let mut full = Vec::new();
            for bytes in session.parts.values() {
                full.extend_from_slice(bytes);
            }
            let digest = hex_sha256(&full);
            if let Some(ref exp) = session.expected_sha256 {
                if exp != &digest {
                    return (400, json!({"error": "checksum_mismatch"}));
                }
            }
            let file_id = format!("cas_{}", &digest[..16]);
            let size = full.len();
            g.media_blobs.insert(file_id.clone(), full.clone());
            g.media_blobs.insert(digest.clone(), full);
            g.uploads.remove(upload_id);
            (
                200,
                json!({
                    "file_id": file_id,
                    "media_id": file_id,
                    "sha256": digest,
                    "size": size
                }),
            )
        }

        // User Vault & X25519 Device Sync (canonical + /api/v1/vault aliases)
        ("GET", "/api/v1/user-vault/envelope") | ("GET", "/api/v1/vault/envelope") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let env = g
                .users_by_id
                .get(&uid)
                .and_then(|u| u.vault_envelope.clone());
            (200, json!({"envelope": env}))
        }
        ("PUT", "/api/v1/user-vault/envelope") | ("PUT", "/api/v1/vault/envelope") => {
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let user = g.ensure_user_by_id(uid);
            user.vault_envelope = Some(body.clone());
            (200, json!({"ok": true, "saved": true}))
        }
        ("GET", "/api/v1/user-vault/items") | ("GET", "/api/v1/vault/items") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let items: Vec<Value> = g
                .vault_items
                .iter()
                .filter(|((u, _, _), _)| *u == uid)
                .map(|(_, v)| v.clone())
                .collect();
            (200, json!({"items": items}))
        }
        ("POST", "/api/v1/user-vault/items")
        | ("PUT", "/api/v1/user-vault/items")
        | ("POST", "/api/v1/vault/items")
        | ("PUT", "/api/v1/vault/items") => {
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let cat = body
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or("general")
                .to_owned();
            let key = body
                .get("item_key")
                .and_then(Value::as_str)
                .unwrap_or("default")
                .to_owned();
            g.vault_items.insert((uid, cat.clone(), key.clone()), body.clone());
            (200, json!({"ok": true, "category": cat, "item_key": key}))
        }
        ("DELETE", p)
            if p.starts_with("/api/v1/user-vault/items/") || p.starts_with("/api/v1/vault/items/") =>
        {
            let rest = p
                .strip_prefix("/api/v1/user-vault/items/")
                .or_else(|| p.strip_prefix("/api/v1/vault/items/"))
                .unwrap_or("");
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let (cat, key) = rest.split_once('/').unwrap_or(("general", rest));
            g.vault_items.remove(&(uid, cat.to_owned(), key.to_owned()));
            (200, json!({"deleted": true}))
        }
        ("POST", "/api/v1/user-vault/device-sync/request")
        | ("POST", "/api/v1/vault/device-sync/request") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let request_id = format!("dsync_{seq}");
            let device_name = body
                .get("device_name")
                .and_then(Value::as_str)
                .unwrap_or("Secondary Device")
                .to_owned();
            let pub_b64 = body
                .get("ephemeral_x25519_pub_b64")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let verification_code = compute_device_sync_code(&request_id, &pub_b64);
            let expires_at_ms = ts + 600_000;
            g.device_syncs.insert(
                request_id.clone(),
                DeviceSyncRecord {
                    request_id: request_id.clone(),
                    device_name: device_name.clone(),
                    ephemeral_x25519_pub_b64: pub_b64.clone(),
                    verification_code: verification_code.clone(),
                    status: "pending".to_owned(),
                    wrapped_key_b64: None,
                    expires_at_ms,
                },
            );
            (
                200,
                json!({
                    "request_id": request_id,
                    "device_name": device_name,
                    "ephemeral_x25519_pub_b64": pub_b64,
                    "verification_code": verification_code,
                    "status": "pending",
                    "expires_at_ms": expires_at_ms
                }),
            )
        }
        ("GET", "/api/v1/user-vault/device-sync/pending")
        | ("GET", "/api/v1/vault/device-sync/pending") => {
            let g = state.lock().expect("state lock");
            let pending: Vec<Value> = g
                .device_syncs
                .values()
                .filter(|r| r.status == "pending" && ts <= r.expires_at_ms)
                .map(|r| {
                    json!({
                        "request_id": r.request_id,
                        "device_name": r.device_name,
                        "ephemeral_x25519_pub_b64": r.ephemeral_x25519_pub_b64,
                        "verification_code": r.verification_code,
                        "status": r.status,
                        "expires_at_ms": r.expires_at_ms
                    })
                })
                .collect();
            (200, json!({"requests": pending}))
        }
        ("POST", p)
            if p == "/api/v1/user-vault/device-sync/approve"
                || p == "/api/v1/vault/device-sync/approve"
                || ((p.starts_with("/api/v1/user-vault/device-sync/")
                    || p.starts_with("/api/v1/vault/device-sync/"))
                    && p.ends_with("/approve")) =>
        {
            let req_id = body
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    p.strip_prefix("/api/v1/user-vault/device-sync/")
                        .or_else(|| p.strip_prefix("/api/v1/vault/device-sync/"))
                        .unwrap_or("")
                        .trim_end_matches("/approve")
                        .to_owned()
                });
            let mut g = state.lock().expect("state lock");
            let Some(rec) = g.device_syncs.get_mut(&req_id) else {
                return (404, json!({"error": "device_sync_not_found"}));
            };
            if rec.status == "denied" {
                return (400, json!({"error": "device_sync_denied"}));
            }
            if ts > rec.expires_at_ms {
                return (400, json!({"error": "device_sync_expired"}));
            }
            rec.status = "approved".to_owned();
            rec.wrapped_key_b64 = body
                .get("wrapped_key_b64")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (
                200,
                json!({
                    "request_id": rec.request_id,
                    "status": rec.status,
                    "wrapped_key_b64": rec.wrapped_key_b64
                }),
            )
        }
        ("POST", p)
            if p == "/api/v1/user-vault/device-sync/deny"
                || p == "/api/v1/vault/device-sync/deny"
                || ((p.starts_with("/api/v1/user-vault/device-sync/")
                    || p.starts_with("/api/v1/vault/device-sync/"))
                    && p.ends_with("/deny")) =>
        {
            let req_id = body
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    p.strip_prefix("/api/v1/user-vault/device-sync/")
                        .or_else(|| p.strip_prefix("/api/v1/vault/device-sync/"))
                        .unwrap_or("")
                        .trim_end_matches("/deny")
                        .to_owned()
                });
            let mut g = state.lock().expect("state lock");
            let Some(rec) = g.device_syncs.get_mut(&req_id) else {
                return (404, json!({"error": "device_sync_not_found"}));
            };
            rec.status = "denied".to_owned();
            (200, json!({"request_id": rec.request_id, "status": "denied"}))
        }
        ("GET", p) | ("POST", p)
            if p.starts_with("/api/v1/user-vault/device-sync/status")
                || p.starts_with("/api/v1/vault/device-sync/status")
                || p.starts_with("/api/v1/user-vault/device-sync/dsync_")
                || p.starts_with("/api/v1/vault/device-sync/dsync_") =>
        {
            let req_id = body
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    p.rsplit('/').next().unwrap_or("").to_owned()
                });
            let g = state.lock().expect("state lock");
            let Some(rec) = g.device_syncs.get(&req_id) else {
                return (404, json!({"error": "device_sync_not_found"}));
            };
            (
                200,
                json!({
                    "request_id": rec.request_id,
                    "device_name": rec.device_name,
                    "ephemeral_x25519_pub_b64": rec.ephemeral_x25519_pub_b64,
                    "verification_code": rec.verification_code,
                    "status": rec.status,
                    "wrapped_key_b64": rec.wrapped_key_b64,
                    "expires_at_ms": rec.expires_at_ms
                }),
            )
        }

        // Updates check
        ("GET", "/api/v1/updates/check") => (
            200,
            json!({
                "latest_version": "1.6.7",
                "store_url": "https://apps.apple.com/app/fabushi",
                "binary_self_update_allowed": false
            }),
        ),

        // Explicit suite routes for remaining namespaces (all known prefixes explicitly enumerated)
        (_, p)
            if p.starts_with("/api/v1/chats/")
                || p.starts_with("/api/v1/messages/")
                || p.starts_with("/api/v1/folders")
                || p.starts_with("/api/v1/contacts")
                || p.starts_with("/api/v1/peers/")
                || p.starts_with("/api/v1/blocked")
                || p.starts_with("/api/v1/settings")
                || p.starts_with("/api/v1/fabushi/")
                || p.starts_with("/api/v1/agent/")
                || p.starts_with("/api/v1/tasks")
                || p.starts_with("/api/v1/magpie/v1/")
                || p.starts_with("/api/v1/mcp/")
                || p.starts_with("/api/v1/computers")
                || p.starts_with("/api/v1/remote/")
                || p.starts_with("/api/v1/tailnet")
                || p.starts_with("/api/v1/miniapps")
                || p.starts_with("/api/v1/stickers")
                || p.starts_with("/api/v1/gifs")
                || p.starts_with("/api/v1/stories")
                || p.starts_with("/api/v1/calls")
                || p.starts_with("/api/v1/webrtc")
                || p.starts_with("/api/v1/talk")
                || p.starts_with("/api/v1/search")
                || p.starts_with("/api/v1/e2ee")
                || p.starts_with("/api/v1/wallet")
                || p.starts_with("/api/v1/actions")
                || p.starts_with("/api/v1/devices")
                || p.starts_with("/api/v1/push") =>
        {
            (
                200,
                json!({
                    "ok": true,
                    "path": path,
                    "method": method,
                    "timestamp_ms": ts
                }),
            )
        }

        // Strict fail-closed 404 on unknown routes (never fake 200 ok:true!)
        _ => (
            404,
            json!({
                "error": "not_found",
                "message": format!("unrecognized loopback route: {method} {path}"),
                "path": path,
                "method": method
            }),
        ),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
