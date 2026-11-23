//! Ephemeral real-socket HTTP/1.1 + RFC 6455 WebSocket loopback protocol server
//! bound to `127.0.0.1:0` with HTTP 206/416 range streaming, deterministic fault injection,
//! genuine FIPS 180-4 SHA-1 WebSocket handshakes, and clean wake-on-drop shutdown.

use crate::contracts::sha1_digest;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const WS_MAGIC_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

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
    next_user_id: u64,
    next_chat_id: i64,
    next_msg_id: i64,
}

impl ServerState {
    fn new() -> Self {
        Self {
            next_user_id: 1000,
            next_chat_id: 10,
            next_msg_id: 1,
            ..Self::default()
        }
    }
}

pub struct LoopbackServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    state: Arc<Mutex<ServerState>>,
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

        let shutdown_clone = Arc::clone(&shutdown);
        let state_clone = Arc::clone(&state);

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
                            thread::spawn(move || {
                                handle_connection(stream, st, sd);
                            });
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

fn read_http_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end;
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => return None,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = pos + 4;
                    break;
                }
                if buf.len() > 1_048_576 {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }

    let head_str = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = head_str.lines();
    let req_line = lines.next()?;
    let mut parts = req_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let raw_target = parts.next()?;
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

    let content_length: usize = headers
        .get("content-length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    body.truncate(content_length);

    Some(RecordedRequest {
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

fn read_ws_frame(stream: &mut TcpStream) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 2];
    match stream.read_exact(&mut head) {
        Ok(()) => {}
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err),
    }

    let opcode = head[0] & 0x0F;
    let masked = (head[1] & 0x80) != 0;
    let mut payload_len = (head[1] & 0x7F) as u64;
    if payload_len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext)?;
        payload_len = u64::from(u16::from_be_bytes(ext));
    } else if payload_len == 127 {
        let mut ext = [0u8; 8];
        stream.read_exact(&mut ext)?;
        payload_len = u64::from_be_bytes(ext);
    }

    let mask_key = if masked {
        let mut key = [0u8; 4];
        stream.read_exact(&mut key)?;
        Some(key)
    } else {
        None
    };

    let mut payload = vec![0u8; payload_len as usize];
    stream.read_exact(&mut payload)?;
    if let Some(key) = mask_key {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= key[i % 4];
        }
    }
    Ok(Some((opcode, payload)))
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
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));

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
            let _ = write_ws_frame(&mut stream, 0x8, &[]);
            break;
        }

        for msg in queued {
            if write_ws_frame(&mut stream, 0x1, msg.as_bytes()).is_err() {
                return;
            }
        }

        match read_ws_frame(&mut stream) {
            Ok(Some((0x1, payload))) => {
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
                if write_ws_frame(&mut stream, 0x1, echo.as_bytes()).is_err() {
                    break;
                }
            }
            Ok(Some((0x9, payload))) => {
                // Ping -> Pong
                if write_ws_frame(&mut stream, 0xA, &payload).is_err() {
                    break;
                }
            }
            Ok(Some((0x8, _))) => {
                // Close handshake
                let _ = write_ws_frame(&mut stream, 0x8, &[]);
                break;
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(_) => break,
        }
    }
}

fn parse_byte_range(header: &str, total: usize) -> Option<Result<(usize, usize), ()>> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    if total == 0 {
        return Some(Err(()));
    }
    let start: usize = start_s.trim().parse().ok()?;
    if start >= total {
        return Some(Err(()));
    }
    let end = if end_s.trim().is_empty() {
        total - 1
    } else {
        let parsed: usize = end_s.trim().parse().ok()?;
        parsed.min(total - 1)
    };
    if end < start {
        return Some(Err(()));
    }
    Some(Ok((start, end)))
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

    let Some(req) = read_http_request(&mut stream) else {
        return;
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

    // Media HTTP 206 Partial Content / 416 Range Not Satisfiable handler.
    if req.path.starts_with("/api/v1/media/blob/") {
        let file_id = req.path.trim_start_matches("/api/v1/media/blob/");
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

    // Stateful JSON API routes backing F01..F34 loopback flows.
    let req_json: Value = serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
    let (status, resp_json) = route_api_request(&req.method, &req.path, &req_json, &state);
    let bytes = resp_json.to_string().into_bytes();
    write_http_response(&mut stream, status, "application/json", &[], &bytes);
}

fn route_api_request(
    method: &str,
    path: &str,
    body: &Value,
    state: &Arc<Mutex<ServerState>>,
) -> (u16, Value) {
    let ts = now_ms();
    match (method, path) {
        ("GET", "/health") => (200, json!({"ok": true, "version": "1.6.7", "ts_ms": ts})),
        ("GET", "/api/v1/auth/kdf-params") => (
            200,
            json!({
                "auth_version": 2,
                "kdf_alg": "split_key_v2",
                "auth_salt_b64": B64.encode([0x42u8; 16]),
                "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
            }),
        ),
        ("POST", "/api/v1/auth/register") | ("POST", "/api/v1/auth/login") => {
            let uid = state
                .lock()
                .map(|mut g| {
                    g.next_user_id += 1;
                    g.next_user_id
                })
                .unwrap_or(1001);
            (
                200,
                json!({
                    "user_id": uid,
                    "session_id": format!("sess_{uid}"),
                    "token": format!("tok_{uid}_{ts}"),
                    "expires_at": ts + 86_400_000,
                    "auth_version": body.get("auth_version").and_then(Value::as_i64).unwrap_or(2),
                    "vault_unlocked": true,
                    "requires_2fa": false
                }),
            )
        }
        ("POST", "/api/v1/auth/email/send-code") => (
            200,
            json!({
                "ok": true,
                "sent": true,
                "expires_at_ms": ts + 600_000,
                "resend_after_ms": ts + 60_000
            }),
        ),
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
        ("POST", "/api/v1/sync/get_difference") => {
            let chat_id = body.get("chat_id").and_then(Value::as_i64).unwrap_or(1);
            let req_pts = body.get("pts").and_then(Value::as_u64).unwrap_or(0);
            let floor = state
                .lock()
                .ok()
                .and_then(|g| g.journal_floors.get(&chat_id).copied())
                .unwrap_or(1);
            let snapshot_required = req_pts.saturating_add(1) < floor;
            (
                200,
                json!({
                    "chat_id": chat_id,
                    "pts": req_pts,
                    "new_pts": if snapshot_required { floor } else { req_pts },
                    "current_pts": floor,
                    "has_more": snapshot_required,
                    "snapshot_required": snapshot_required,
                    "messages": [],
                    "other_updates": [],
                    "updates": []
                }),
            )
        }
        ("POST", "/api/v1/chats") => {
            let cid = state
                .lock()
                .map(|mut g| {
                    g.next_chat_id += 1;
                    g.next_chat_id
                })
                .unwrap_or(11);
            (
                200,
                json!({
                    "chat_id": cid,
                    "title": body.get("title").and_then(Value::as_str).unwrap_or("Chat"),
                    "chat_type": body.get("chat_type").and_then(Value::as_u64).unwrap_or(0),
                    "member_count": 2
                }),
            )
        }
        ("POST", p) if p.starts_with("/api/v1/chats/") && p.ends_with("/messages") => {
            let mid = state
                .lock()
                .map(|mut g| {
                    g.next_msg_id += 1;
                    g.next_msg_id
                })
                .unwrap_or(1);
            (
                200,
                json!({
                    "message_id": mid,
                    "pts": mid as u64,
                    "delivery_status": "sent",
                    "timestamp_ms": ts
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
        ("POST", "/api/v1/media/upload") => {
            let digest = hex_sha256(body.to_string().as_bytes());
            (
                200,
                json!({
                    "file_id": format!("cas_{}", &digest[..16]),
                    "sha256": digest
                }),
            )
        }
        ("GET", "/api/v1/updates/check") => (
            200,
            json!({
                "latest_version": "1.6.7",
                "store_url": "https://apps.apple.com/app/fabushi",
                "binary_self_update_allowed": false
            }),
        ),
        _ => (
            200,
            json!({
                "ok": true,
                "path": path,
                "method": method,
                "timestamp_ms": ts
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
