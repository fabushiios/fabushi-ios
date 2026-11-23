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
use uuid::Uuid;

const WS_MAGIC_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;
const HTTP_READ_DEADLINE: Duration = Duration::from_secs(5);
const SHUTDOWN_POLL_SLICE: Duration = Duration::from_millis(100);
const MAX_HTTP_TRAILER_LINES: usize = 64;
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
    first_name: String,
    auth_version: i64,
    key_version: u64,
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
struct AlipayAttemptRecord {
    attempt_id: String,
    state: String,
    status: String, // "pending" | "confirmed" | "consumed" | "cancelled" | "expired"
    issued_session: Option<Value>,
    expires_at_ms: i64,
}

#[derive(Debug, Clone)]
struct PasskeyRecord {
    credential_id: String,
    user_id: u64,
    public_key: String,
    device_name: String,
    sign_count: u64,
    created_at_ms: i64,
}

#[derive(Debug, Clone)]
struct PasskeyChallengeRecord {
    user_id: Option<u64>,
    expires_at_ms: i64,
}

#[derive(Debug, Clone)]
struct PhoneOtpRecord {
    expected_code: String,
    attempts: u32,
    expires_at_ms: i64,
}

#[derive(Debug, Clone)]
struct ComputerLeaseRecord {
    lease_id: String,
    generation: u64,
    override_epoch: u64,
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
    filename: Value,
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
    alipay_attempts: HashMap<String, AlipayAttemptRecord>,
    alipay_state_to_attempt: HashMap<String, String>,
    consumed_alipay_codes: HashSet<String>,
    passkeys: HashMap<String, PasskeyRecord>,
    passkey_challenges: HashMap<String, PasskeyChallengeRecord>,
    phone_challenges: HashMap<String, PhoneOtpRecord>,
    computer_leases: HashMap<String, ComputerLeaseRecord>,
    users_by_id: HashMap<u64, UserRecord>,
    username_to_id: HashMap<String, u64>,
    sessions_by_token: HashMap<String, SessionRecord>,
    session_id_to_token: HashMap<String, String>,
    revoked_tokens: HashSet<String>,
    two_factor_challenges: HashMap<String, TwoFactorChallenge>,
    qr_pairs: HashMap<String, QrPairRecord>,
    chats: HashMap<i64, ChatRecord>,
    chats_by_nonce: HashMap<String, i64>,
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

    fn is_token_rejected(&self, tok: &str) -> bool {
        self.revoked_tokens.contains(tok)
            || self.sessions_by_token.get(tok).is_some_and(|s| s.revoked)
            || (tok.starts_with("tok_") && !self.sessions_by_token.contains_key(tok))
            || (!self.revoked_tokens.is_empty() && !self.sessions_by_token.contains_key(tok))
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
            first_name: format!("User {user_id}"),
            auth_version: 2,
            key_version: 1,
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

    pub fn confirm_alipay_attempt(&self, attempt_id: &str, mut session: Value) {
        if let Ok(mut guard) = self.state.lock() {
            let ts = now_ms();
            let user_id = session
                .get("user_id")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| {
                    guard.next_user_id += 1;
                    guard.next_user_id
                });
            guard.ensure_user_by_id(user_id);

            let token_opt = session
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);

            if let Some(tok) = token_opt {
                let session_id = session
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("sess_alipay_{}", guard.alloc_seq()));
                let expires_at_ms = session
                    .get("expires_at")
                    .or_else(|| session.get("expires_at_ms"))
                    .and_then(Value::as_i64)
                    .unwrap_or(ts + 86_400_000);
                let device_name = session
                    .get("device_name")
                    .and_then(Value::as_str)
                    .unwrap_or("iOS Alipay")
                    .to_owned();

                guard.revoked_tokens.remove(&tok);
                let rec = SessionRecord {
                    session_id: session_id.clone(),
                    user_id,
                    token: tok.clone(),
                    device_name,
                    created_at_ms: ts,
                    expires_at_ms,
                    revoked: false,
                };
                guard
                    .session_id_to_token
                    .insert(session_id.clone(), tok.clone());
                guard.sessions_by_token.insert(tok, rec);

                if let Some(obj) = session.as_object_mut() {
                    obj.entry("user_id".to_owned())
                        .or_insert_with(|| json!(user_id));
                    obj.entry("session_id".to_owned())
                        .or_insert_with(|| json!(session_id));
                    obj.entry("expires_at".to_owned())
                        .or_insert_with(|| json!(expires_at_ms));
                }
            } else {
                let sess = guard.mint_session(user_id, "iOS Alipay", ts);
                if let Some(obj) = session.as_object_mut() {
                    obj.insert("user_id".to_owned(), json!(user_id));
                    obj.insert("session_id".to_owned(), json!(sess.session_id));
                    obj.insert("token".to_owned(), json!(sess.token));
                    obj.insert("expires_at".to_owned(), json!(sess.expires_at_ms));
                } else {
                    session = json!({
                        "user_id": user_id,
                        "session_id": sess.session_id,
                        "token": sess.token,
                        "expires_at": sess.expires_at_ms
                    });
                }
            }

            if let Some(attempt) = guard.alipay_attempts.get_mut(attempt_id) {
                attempt.status = "confirmed".to_owned();
                attempt.issued_session = Some(session);
            } else {
                let fallback_state = format!("fbs_{}", Uuid::new_v4().simple());
                guard
                    .alipay_state_to_attempt
                    .insert(fallback_state.clone(), attempt_id.to_owned());
                guard.alipay_attempts.insert(
                    attempt_id.to_owned(),
                    AlipayAttemptRecord {
                        attempt_id: attempt_id.to_owned(),
                        state: fallback_state,
                        status: "confirmed".to_owned(),
                        issued_session: Some(session),
                        expires_at_ms: ts + 600_000,
                    },
                );
            }
        }
    }

    pub fn expire_alipay_attempt(&self, attempt_id: &str) {
        if let Ok(mut guard) = self.state.lock() {
            if let Some(attempt) = guard.alipay_attempts.get_mut(attempt_id) {
                attempt.expires_at_ms = now_ms() - 1;
                attempt.status = "expired".to_owned();
            }
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

fn ms_to_rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    // Howard Hinnant's civil-from-days algorithm (UTC Gregorian calendar).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_owned())
}

fn matches_segment_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn normalize_request_target(raw_target: &str) -> (String, String) {
    // Normalize RFC 9112 §3.2.2 absolute-form HTTP proxy targets (http://host[:port]/path?q).
    let normalized = if raw_target.len() > 7
        && (raw_target[..7].eq_ignore_ascii_case("http://")
            || (raw_target.len() > 8 && raw_target[..8].eq_ignore_ascii_case("https://")))
    {
        let scheme_end = raw_target.find("://").unwrap_or(0);
        let after_scheme = &raw_target[scheme_end + 3..];
        let slash_pos = after_scheme.find('/');
        let query_pos = after_scheme.find('?');
        match (slash_pos, query_pos) {
            (Some(s), Some(q)) if s < q => after_scheme[s..].to_owned(),
            (Some(s), None) => after_scheme[s..].to_owned(),
            (_, Some(q)) => format!("/{}", &after_scheme[q..]),
            (None, None) => "/".to_owned(),
        }
    } else {
        raw_target.to_owned()
    };

    match normalized.split_once('?') {
        Some((p, q)) => (p.to_owned(), q.to_owned()),
        None => (normalized, String::new()),
    }
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

fn write_http_response_mode(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra_headers: &[(&str, String)],
    body: &[u8],
    head_only: bool,
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
    if !head_only {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
}

fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra_headers: &[(&str, String)],
    body: &[u8],
) {
    write_http_response_mode(stream, status, content_type, extra_headers, body, false);
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
    shutdown: &AtomicBool,
) -> Result<usize, HttpReadError> {
    let mut tmp = [0u8; 8192];
    let to_read = max_read.clamp(1, tmp.len());
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return Err(HttpReadError::Closed);
        }
        let Some(rem) = remaining_timeout(deadline) else {
            return Err(HttpReadError::BadRequest(timeout_err));
        };
        let _ = stream.set_read_timeout(Some(rem.min(SHUTDOWN_POLL_SLICE)));
        match stream.read(&mut tmp[..to_read]) {
            Ok(0) => return Err(HttpReadError::BadRequest(eof_err)),
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                return Ok(n);
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if shutdown.load(Ordering::SeqCst) {
                    return Err(HttpReadError::Closed);
                }
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
    shutdown: &AtomicBool,
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
            shutdown,
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
    shutdown: &AtomicBool,
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
            shutdown,
        )?;
    }
    Ok(())
}

fn read_crlf_line_with_deadline(
    stream: &mut TcpStream,
    raw: &mut Vec<u8>,
    cursor: &mut usize,
    deadline: Instant,
    shutdown: &AtomicBool,
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
            shutdown,
        )?;
    }
}

fn read_chunked_body_with_deadline(
    stream: &mut TcpStream,
    initial: &[u8],
    deadline: Instant,
    shutdown: &AtomicBool,
) -> Result<Vec<u8>, HttpReadError> {
    let mut raw = initial.to_vec();
    let mut cursor = 0usize;
    let mut decoded = Vec::new();

    loop {
        let size_line =
            read_crlf_line_with_deadline(stream, &mut raw, &mut cursor, deadline, shutdown)?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        if size_hex.is_empty() || !size_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(HttpReadError::BadRequest("invalid_chunk_size"));
        }
        let chunk_size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| HttpReadError::BadRequest("invalid_chunk_size"))?;

        if chunk_size == 0 {
            // Consume optional trailers until empty CRLF line (capped to prevent DoS).
            let mut trailer_count = 0usize;
            loop {
                let trailer = read_crlf_line_with_deadline(
                    stream,
                    &mut raw,
                    &mut cursor,
                    deadline,
                    shutdown,
                )?;
                if trailer.is_empty() {
                    return Ok(decoded);
                }
                trailer_count += 1;
                if trailer_count > MAX_HTTP_TRAILER_LINES {
                    return Err(HttpReadError::BadRequest("too_many_chunk_trailers"));
                }
            }
        }

        if decoded.len().saturating_add(chunk_size) > MAX_HTTP_BODY_BYTES {
            return Err(HttpReadError::PayloadTooLarge);
        }

        ensure_raw_bytes_with_deadline(stream, &mut raw, cursor, chunk_size + 2, deadline, shutdown)?;
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

fn read_http_request(
    stream: &mut TcpStream,
    shutdown: &AtomicBool,
) -> Result<RecordedRequest, HttpReadError> {
    let deadline = Instant::now() + HTTP_READ_DEADLINE;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            return Err(HttpReadError::Closed);
        }
        let Some(rem) = remaining_timeout(deadline) else {
            return Err(HttpReadError::BadRequest("header_timeout"));
        };
        let _ = stream.set_read_timeout(Some(rem.min(SHUTDOWN_POLL_SLICE)));
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
                if shutdown.load(Ordering::SeqCst) {
                    return Err(HttpReadError::Closed);
                }
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
    let (path, query) = normalize_request_target(raw_target);

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
        read_chunked_body_with_deadline(stream, &buf[header_end..], deadline, shutdown)?
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
            read_exact_len_with_deadline(stream, &mut body, content_length, deadline, shutdown)?;
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
    fn ensure_bytes(
        &mut self,
        stream: &mut TcpStream,
        needed: usize,
        shutdown: &AtomicBool,
    ) -> Result<(), WsReadError> {
        let deadline = Instant::now() + WS_FRAME_BODY_DEADLINE;
        let mut tmp = [0u8; 4096];
        while self.buf.len() < needed {
            if shutdown.load(Ordering::SeqCst) {
                return Err(WsReadError::Io);
            }
            let Some(rem) = remaining_timeout(deadline) else {
                return Err(WsReadError::Io);
            };
            let _ = stream.set_read_timeout(Some(rem.min(SHUTDOWN_POLL_SLICE)));
            match stream.read(&mut tmp) {
                Ok(0) => {
                    return Err(WsReadError::Io);
                }
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if shutdown.load(Ordering::SeqCst) || Instant::now() >= deadline {
                        return Err(WsReadError::Io);
                    }
                }
                Err(_) => return Err(WsReadError::Io),
            }
        }
        Ok(())
    }

    fn read_frame(
        &mut self,
        stream: &mut TcpStream,
        shutdown: &AtomicBool,
    ) -> Result<Option<WsFrame>, WsReadError> {
        if self.buf.is_empty() {
            if shutdown.load(Ordering::SeqCst) {
                return Err(WsReadError::Io);
            }
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

        self.ensure_bytes(stream, 2, shutdown)?;
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
            self.ensure_bytes(stream, 4, shutdown)?;
            (4usize, u64::from(u16::from_be_bytes([self.buf[2], self.buf[3]])))
        } else {
            self.ensure_bytes(stream, 10, shutdown)?;
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
        self.ensure_bytes(stream, total, shutdown)?;

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
        let Ok(text) = String::from_utf8(payload) else {
            let _ = write_ws_close(stream, 1007, "invalid_utf8");
            return false;
        };
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
    ws_token: Option<String>,
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
        let (is_offline, is_revoked, queued) = state
            .lock()
            .map(|mut g| {
                let rev = ws_token
                    .as_deref()
                    .is_some_and(|tok| g.is_token_rejected(tok));
                if g.offline || rev {
                    (g.offline, rev, Vec::new())
                } else {
                    (false, false, g.ws_push_queue.drain(..).collect::<Vec<String>>())
                }
            })
            .unwrap_or((false, false, Vec::new()));

        if is_revoked {
            let _ = write_ws_close(&mut stream, 4001, "session_revoked");
            break;
        }
        if is_offline {
            let _ = write_ws_close(&mut stream, 1001, "offline");
            break;
        }

        for msg in queued {
            if write_ws_frame(&mut stream, 0x1, msg.as_bytes()).is_err() {
                return;
            }
        }

        match reader.read_frame(&mut stream, &shutdown) {
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
            let trimmed = v.trim();
            if k == key && !trimmed.is_empty() {
                return Some(trimmed);
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
    if let Some(id) = path.strip_prefix("/api/v1/media/download/") {
        return (!id.is_empty()).then_some(id);
    }
    if let Some(rest) = path.strip_prefix("/api/v1/media/") {
        if !rest.is_empty()
            && !rest.contains('/')
            && !rest.starts_with("upload")
            && rest != "edit"
            && rest != "instant-view"
            && rest != "instant_view"
        {
            return Some(rest);
        }
    }
    None
}

fn is_protected_route(path: &str) -> bool {
    if matches!(
        path,
        "/health"
            | "/healthz"
            | "/api/v1/health"
            | "/api/v1/status"
            | "/api/v1/auth/2fa/verify"
            | "/api/v1/auth/two-factor/verify"
            | "/api/v1/auth/pair/init"
            | "/api/v1/auth/pair/verify"
            | "/api/v1/auth/login"
            | "/api/v1/auth/register"
            | "/api/v1/auth/kdf-params"
            | "/api/v1/auth/kdf_params"
            | "/api/v1/auth/passkey/challenge"
            | "/api/v1/auth/passkeys/challenge"
            | "/api/v1/auth/passkey/verify"
            | "/api/v1/auth/passkeys/verify"
            | "/api/v1/auth/passkey/login"
            | "/api/v1/auth/passkey/authenticate"
    ) || matches_segment_prefix(path, "/api/v1/auth/pair/poll")
        || matches_segment_prefix(path, "/api/v1/auth/email")
        || matches_segment_prefix(path, "/api/v1/auth/phone")
        || matches_segment_prefix(path, "/api/v1/auth/alipay")
        || matches_segment_prefix(path, "/article")
        || matches_segment_prefix(path, "/iv")
        || matches_segment_prefix(path, "/.well-known")
    {
        return false;
    }
    matches!(
        path,
        "/api/v1/auth/me"
            | "/api/v1/auth/logout"
            | "/api/v1/auth/pair/accept"
            | "/api/v1/auth/upgrade-split-key"
            | "/api/v1/auth/upgrade_split_key"
            | "/api/v1/auth/change-password"
            | "/api/v1/auth/change_password"
    ) || matches_segment_prefix(path, "/api/v1/auth/sessions")
        || matches_segment_prefix(path, "/api/v1/auth/2fa")
        || matches_segment_prefix(path, "/api/v1/auth/two-factor")
        || matches_segment_prefix(path, "/api/v1/auth/passkey")
        || matches_segment_prefix(path, "/api/v1/auth/passkeys")
        || matches_segment_prefix(path, "/api/v1/chats")
        || matches_segment_prefix(path, "/api/v1/messages")
        || matches_segment_prefix(path, "/api/v1/sync")
        || matches_segment_prefix(path, "/api/v1/user-vault")
        || matches_segment_prefix(path, "/api/v1/vault")
        || matches_segment_prefix(path, "/api/v1/folders")
        || matches_segment_prefix(path, "/api/v1/contacts")
        || matches_segment_prefix(path, "/api/v1/settings")
        || matches_segment_prefix(path, "/api/v1/agent")
        || matches_segment_prefix(path, "/api/v1/tasks")
        || matches_segment_prefix(path, "/api/v1/mcp")
        || matches_segment_prefix(path, "/api/v1/computer")
        || matches_segment_prefix(path, "/api/v1/computers")
        || matches_segment_prefix(path, "/api/v1/talk")
        || matches_segment_prefix(path, "/api/v1/webrtc")
        || matches_segment_prefix(path, "/api/v1/search")
        || matches_segment_prefix(path, "/api/v1/wallet")
        || matches_segment_prefix(path, "/api/v1/e2ee")
        || matches_segment_prefix(path, "/api/v1/push")
        || matches_segment_prefix(path, "/api/v1/stories")
        || matches_segment_prefix(path, "/api/v1/ws")
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

    let req = match read_http_request(&mut stream, &shutdown) {
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

    // HTTP proxy CONNECT probe (RFC 9110 §9.3.6).
    if req.method == "CONNECT" {
        let _ = stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
        let _ = stream.flush();
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }

    // RFC 6455 WebSocket upgrade check (with path validation, fault injection, and Bearer revocation gate BEFORE 101).
    if req
        .headers
        .get("upgrade")
        .is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
    {
        if let Some(ws_key) = req.headers.get("sec-websocket-key").cloned() {
            if req.path != "/api/v1/ws" && req.path != "/ws" {
                let body = json!({
                    "error": "not_found",
                    "message": format!("unrecognized websocket path: {}", req.path),
                    "path": req.path
                })
                .to_string();
                write_http_response(&mut stream, 404, "application/json", &[], body.as_bytes());
                let _ = stream.shutdown(Shutdown::Both);
                return;
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
                let _ = stream.shutdown(Shutdown::Both);
                return;
            }
            let ws_token = extract_bearer_token(&req);
            if let Some(ref tok) = ws_token {
                let rejected = state
                    .lock()
                    .map(|g| g.is_token_rejected(tok))
                    .unwrap_or(false);
                if rejected {
                    let body = json!({
                        "error": "unauthorized",
                        "message": "session_revoked"
                    })
                    .to_string();
                    write_http_response(
                        &mut stream,
                        401,
                        "application/json",
                        &[],
                        body.as_bytes(),
                    );
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
            }
            handle_websocket(stream, &ws_key, ws_token, state, shutdown);
            return;
        }
    }

    let is_head = req.method == "HEAD";

    if let Some(f) = fault {
        let mut extras = Vec::new();
        if let Some(ms) = f.retry_after_ms {
            extras.push(("Retry-After", ms.div_ceil(1000).to_string()));
        }
        write_http_response_mode(
            &mut stream,
            f.status,
            "application/json",
            &extras,
            f.body.as_bytes(),
            is_head,
        );
        return;
    }

    // Instant View HTML article routes MUST execute before JSON dispatch (strictly segment-bounded).
    if matches!(req.method.as_str(), "GET" | "HEAD")
        && (matches_segment_prefix(&req.path, "/article")
            || matches_segment_prefix(&req.path, "/iv"))
    {
        let slug = req
            .path
            .strip_prefix("/article/")
            .or_else(|| req.path.strip_prefix("/iv/"))
            .filter(|s| !s.is_empty())
            .unwrap_or("v167");
        let html = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Fabushi v1.6.7 Instant View</title></head><body><article><h1>Fabushi v1.6.7</h1><p>Instant View content for {slug}.</p></article></body></html>"
        );
        write_http_response_mode(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            &[],
            html.as_bytes(),
            is_head,
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
                    let bytes_received = bytes.len();
                    let mut g = state.lock().expect("state lock");
                    if let Some(session) = g.uploads.get_mut(upload_id) {
                        session.parts.insert(part_index, bytes);
                        let received: Vec<usize> = session.parts.keys().copied().collect();
                        (
                            200,
                            json!({
                                "upload_id": session.upload_id,
                                "part_index": part_index,
                                "bytes_received": bytes_received,
                                "status": "accepted",
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

    // Unified GET/HEAD media retrieval (200 / 206 / 416 / 404).
    if matches!(req.method.as_str(), "GET" | "HEAD") {
        if let Some(file_id) = resolve_media_get_id(&req.path) {
            let maybe_blob = state
                .lock()
                .ok()
                .and_then(|g| g.media_blobs.get(file_id).cloned());
            let Some(blob) = maybe_blob else {
                let body = json!({"error": "media_not_found"}).to_string();
                write_http_response_mode(
                    &mut stream,
                    404,
                    "application/json",
                    &[],
                    body.as_bytes(),
                    is_head,
                );
                return;
            };
            if let Some(range_hdr) = req.headers.get("range") {
                match parse_byte_range(range_hdr, blob.len()) {
                    Some(Ok((start, end))) => {
                        let slice = &blob[start..=end];
                        let cr = format!("bytes {start}-{end}/{}", blob.len());
                        write_http_response_mode(
                            &mut stream,
                            206,
                            "application/octet-stream",
                            &[
                                ("Accept-Ranges", "bytes".to_owned()),
                                ("Content-Range", cr),
                            ],
                            slice,
                            is_head,
                        );
                        return;
                    }
                    Some(Err(())) | None => {
                        let cr = format!("bytes */{}", blob.len());
                        let body = json!({"error": "range_not_satisfiable"}).to_string();
                        write_http_response_mode(
                            &mut stream,
                            416,
                            "application/json",
                            &[("Content-Range", cr)],
                            body.as_bytes(),
                            is_head,
                        );
                        return;
                    }
                }
            }
            write_http_response_mode(
                &mut stream,
                200,
                "application/octet-stream",
                &[("Accept-Ranges", "bytes".to_owned())],
                &blob,
                is_head,
            );
            return;
        }
    }

    // Stateful JSON API routes backing F01..F34 loopback flows.
    let req_json: Value = serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
    let (status, resp_json) = route_api_request(&req, &req_json, &state);
    let bytes = resp_json.to_string().into_bytes();
    write_http_response_mode(
        &mut stream,
        status,
        "application/json",
        &[],
        &bytes,
        is_head,
    );
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
        "status": "sent",
        "timestamp_ms": m.timestamp_ms
    })
}

fn extract_client_nonce_key(body: &Value) -> Option<String> {
    match body.get("client_nonce") {
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn poll_alipay_attempt(
    state: &Arc<Mutex<ServerState>>,
    attempt_id: &str,
    ts: i64,
) -> (u16, Value) {
    let mut g = state.lock().expect("state lock");
    let Some(attempt) = g.alipay_attempts.get_mut(attempt_id) else {
        return (404, json!({"error": "oauth_attempt_not_found"}));
    };
    if attempt.status == "cancelled" {
        return (
            409,
            json!({
                "error": "oauth_cancelled",
                "attempt_id": attempt.attempt_id,
                "state": attempt.state,
                "status": "cancelled"
            }),
        );
    }
    if attempt.status == "confirmed" || attempt.status == "consumed" {
        return (
            200,
            json!({
                "attempt_id": attempt.attempt_id,
                "state": attempt.state,
                "status": "confirmed",
                "session": attempt.issued_session,
                "expires_at_ms": attempt.expires_at_ms
            }),
        );
    }
    if ts > attempt.expires_at_ms || attempt.status == "expired" {
        attempt.status = "expired".to_owned();
        return (
            400,
            json!({
                "error": "oauth_expired",
                "attempt_id": attempt.attempt_id,
                "state": attempt.state,
                "status": "expired"
            }),
        );
    }
    (
        200,
        json!({
            "attempt_id": attempt.attempt_id,
            "state": attempt.state,
            "status": "pending",
            "poll_after_ms": 1000,
            "expires_at_ms": attempt.expires_at_ms
        }),
    )
}

fn cancel_alipay_attempt(
    state: &Arc<Mutex<ServerState>>,
    attempt_id: Option<&str>,
    state_tok: Option<&str>,
) -> (u16, Value) {
    let mut g = state.lock().expect("state lock");
    let resolved_id = if let Some(aid) = attempt_id {
        Some(aid.to_owned())
    } else if let Some(st) = state_tok {
        g.alipay_state_to_attempt.get(st).cloned()
    } else {
        None
    };
    let Some(aid) = resolved_id else {
        return (404, json!({"error": "oauth_attempt_not_found"}));
    };
    let Some(attempt) = g.alipay_attempts.get_mut(&aid) else {
        return (404, json!({"error": "oauth_attempt_not_found"}));
    };
    attempt.status = "cancelled".to_owned();
    attempt.issued_session = None;
    (
        200,
        json!({
            "ok": true,
            "attempt_id": attempt.attempt_id,
            "state": attempt.state,
            "status": "cancelled",
            "cancelled": true
        }),
    )
}

const ALLOWED_FALLBACK_NAMESPACES: &[&str] = &[
    "/api/v1/chats",
    "/api/v1/messages",
    "/api/v1/topics",
    "/api/v1/folders",
    "/api/v1/contacts",
    "/api/v1/peers",
    "/api/v1/users",
    "/api/v1/blocked",
    "/api/v1/settings",
    "/api/v1/fabushi",
    "/api/v1/agent",
    "/api/v1/multi-agent",
    "/api/v1/multi_agent",
    "/api/v1/coding",
    "/api/v1/tasks",
    "/api/v1/magpie",
    "/api/v1/mcp",
    "/api/v1/computer",
    "/api/v1/computers",
    "/api/v1/remote",
    "/api/v1/tailnet",
    "/api/v1/miniapps",
    "/api/v1/webmcp",
    "/api/v1/stickers",
    "/api/v1/gifs",
    "/api/v1/stories",
    "/api/v1/calls",
    "/api/v1/webrtc",
    "/api/v1/talk",
    "/api/v1/search",
    "/api/v1/e2ee",
    "/api/v1/wallet",
    "/api/v1/actions",
    "/api/v1/devices",
    "/api/v1/push",
    "/api/v1/updates",
    "/api/v1/proxy",
];

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

    // Enforce Bearer revocation / invalidation on all protected routes via unified ServerState rule.
    let caller_session: Option<SessionRecord> = {
        let g = state.lock().expect("state lock");
        if let Some(ref tok) = bearer_token {
            if is_protected_route(path) && g.is_token_rejected(tok) {
                return (
                    401,
                    json!({"error": "unauthorized", "message": "session_revoked"}),
                );
            }
            g.sessions_by_token.get(tok).cloned()
        } else {
            None
        }
    };

    match (method, path) {
        // Health, status & OAuth 2.1 MCP discovery
        ("GET" | "HEAD", "/health")
        | ("GET" | "HEAD", "/healthz")
        | ("GET" | "HEAD", "/api/v1/health") => {
            (200, json!({"ok": true, "version": "1.6.7", "ts_ms": ts}))
        }
        ("GET" | "HEAD", "/api/v1/status") => (
            200,
            json!({
                "ok": true,
                "server": "gpui_telegram_server",
                "status": "healthy",
                "version": "1.6.7",
                "ts_ms": ts
            }),
        ),
        ("GET", "/.well-known/oauth-authorization-server")
        | ("GET", "/.well-known/oauth-protected-resource") => (
            200,
            json!({
                "issuer": "http://127.0.0.1",
                "authorization_endpoint": "/api/v1/mcp/oauth/authorize",
                "token_endpoint": "/api/v1/mcp/oauth/token",
                "registration_endpoint": "/api/v1/mcp/oauth/register",
                "code_challenge_methods_supported": ["S256"],
                "supported_mcp_versions": ["2025-11-25"],
                "scopes_supported": [
                    "mcp:read",
                    "mcp:write",
                    "mcp:tools",
                    "mcp:resources",
                    "mcp:prompts",
                    "mcp:tasks",
                    "mcp:sampling",
                    "mcp:roots",
                    "mcp:notifications",
                    "mcp:elicitation",
                    "mcp:computer",
                    "mcp:agent",
                    "mcp:vault",
                    "mcp:audit"
                ]
            }),
        ),

        // Auth KDF params
        ("GET" | "POST", "/api/v1/auth/kdf-params")
        | ("GET" | "POST", "/api/v1/auth/kdf_params") => {
            let ident = extract_query_param(query, "identifier")
                .or_else(|| extract_query_param(query, "username"))
                .or_else(|| body.get("identifier").and_then(Value::as_str))
                .or_else(|| body.get("username").and_then(Value::as_str));
            let g = state.lock().expect("state lock");
            if let Some(name) = ident {
                if let Some(uid) = g.username_to_id.get(name) {
                    if let Some(user) = g.users_by_id.get(uid) {
                        return (
                            200,
                            json!({
                                "auth_version": user.auth_version,
                                "key_version": user.key_version,
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
                    "key_version": 1,
                    "kdf_alg": "split_key_v2",
                    "auth_salt_b64": B64.encode([0x42u8; 16]),
                    "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
                }),
            )
        }

        // Auth register (Split-Key V2 never persists plaintext password fallback; emits RFC 3339 expires_at + expires_at_ms)
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
            let first_name = body
                .get("first_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| uname.clone());
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
            let password = if auth_verifier_b64.is_some() {
                None
            } else {
                body.get("password")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let vault_envelope = body.get("vault_envelope").cloned();
            let key_version = body
                .get("key_version")
                .and_then(Value::as_u64)
                .or_else(|| {
                    vault_envelope
                        .as_ref()
                        .and_then(|e| e.get("key_version"))
                        .and_then(Value::as_u64)
                })
                .unwrap_or(1);
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
                first_name: first_name.clone(),
                auth_version,
                key_version,
                password,
                auth_salt_b64,
                auth_verifier_b64,
                kdf_params,
                vault_envelope,
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
                    "first_name": first_name,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "created_at": ms_to_rfc3339(sess.created_at_ms),
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms,
                    "auth_version": auth_version,
                    "key_version": key_version,
                    "vault_unlocked": true,
                    "requires_2fa": false
                }),
            )
        }

        // Auth login (strict fail-closed verification for Split-Key V2 users)
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
                    let provided_v = body
                        .get("auth_verifier_b64")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());
                    let provided_pw = body
                        .get("password")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());

                    if let Some(ref expected_v) = user.auth_verifier_b64 {
                        if let Some(pv) = provided_v {
                            if pv != expected_v {
                                return (401, json!({"error": "invalid_credentials"}));
                            }
                        } else if let Some(ref expected_pw) = user.password {
                            if provided_pw != Some(expected_pw.as_str()) {
                                return (401, json!({"error": "invalid_credentials"}));
                            }
                        } else {
                            // Fail closed: Split-Key V2 user MUST supply matching auth_verifier_b64.
                            return (401, json!({"error": "invalid_credentials"}));
                        }
                    } else if let Some(ref expected_pw) = user.password {
                        if provided_pw != Some(expected_pw.as_str()) {
                            return (401, json!({"error": "invalid_credentials"}));
                        }
                    } else {
                        // Fail closed: existing user without password/verifier cannot authenticate with empty credentials.
                        return (401, json!({"error": "invalid_credentials"}));
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
                            "first_name": user.first_name,
                            "session_id": sess.session_id,
                            "token": sess.token,
                            "created_at": ms_to_rfc3339(sess.created_at_ms),
                            "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                            "expires_at_ms": sess.expires_at_ms,
                            "auth_version": user.auth_version,
                            "key_version": user.key_version,
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
            let auth_verifier_b64 = body
                .get("auth_verifier_b64")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let password = if auth_verifier_b64.is_some() {
                None
            } else {
                body.get("password")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let user = UserRecord {
                user_id: uid,
                username: uname.clone(),
                first_name: uname.clone(),
                auth_version,
                key_version: 1,
                password,
                auth_salt_b64: body
                    .get("auth_salt_b64")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| B64.encode([0x42u8; 16])),
                auth_verifier_b64,
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
                    "first_name": uname,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "created_at": ms_to_rfc3339(sess.created_at_ms),
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms,
                    "auth_version": auth_version,
                    "key_version": 1,
                    "vault_unlocked": true,
                    "requires_2fa": false
                }),
            )
        }

        // Auth logout (revokes caller Bearer token across HTTP and WS)
        ("POST", "/api/v1/auth/logout") => {
            let mut g = state.lock().expect("state lock");
            let mut revoked_sid = None;
            if let Some(ref tok) = bearer_token {
                if let Some(sess) = g.sessions_by_token.get_mut(tok) {
                    sess.revoked = true;
                    revoked_sid = Some(sess.session_id.clone());
                }
                g.revoked_tokens.insert(tok.clone());
            }
            let gen_num = g.alloc_seq();
            (
                200,
                json!({
                    "ok": true,
                    "logged_out": true,
                    "revoked": true,
                    "session_id": revoked_sid,
                    "account_generation": gen_num
                }),
            )
        }

        // Auth Split-Key V2 upgrade (preserves active session)
        ("POST", "/api/v1/auth/upgrade-split-key") | ("POST", "/api/v1/auth/upgrade_split_key") => {
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let existing = g.ensure_user_by_id(uid).clone();

            let current_pw = body
                .get("current_password")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let current_v = body
                .get("current_auth_verifier_b64")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            if let (Some(expected_pw), Some(provided_pw)) = (existing.password.as_deref(), current_pw)
            {
                if provided_pw != expected_pw {
                    return (401, json!({"error": "invalid_credentials"}));
                }
            }
            if let (Some(expected_v), Some(provided_v)) =
                (existing.auth_verifier_b64.as_deref(), current_v)
            {
                if provided_v != expected_v {
                    return (401, json!({"error": "invalid_credentials"}));
                }
            }

            let new_salt = body
                .get("new_auth_salt_b64")
                .or_else(|| body.get("auth_salt_b64"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let new_verifier = body
                .get("new_auth_verifier_b64")
                .or_else(|| body.get("auth_verifier_b64"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let new_kdf = body
                .get("new_kdf_params")
                .or_else(|| body.get("new_argon2_params"))
                .or_else(|| body.get("kdf_params"))
                .or_else(|| body.get("argon2_params"))
                .cloned();
            let new_env = body
                .get("rewrapped_vault_envelope")
                .or_else(|| body.get("vault_envelope"))
                .cloned();
            let new_key_version = body
                .get("key_version")
                .and_then(Value::as_u64)
                .or_else(|| {
                    new_env
                        .as_ref()
                        .and_then(|e| e.get("key_version"))
                        .and_then(Value::as_u64)
                });

            let user = g.ensure_user_by_id(uid);
            user.auth_version = body.get("auth_version").and_then(Value::as_i64).unwrap_or(2);
            if let Some(salt) = new_salt {
                user.auth_salt_b64 = salt;
            }
            if let Some(v) = new_verifier {
                user.auth_verifier_b64 = Some(v);
                user.password = None;
            }
            if let Some(kdf) = new_kdf {
                user.kdf_params = kdf;
            }
            if let Some(env) = new_env {
                user.vault_envelope = Some(env);
            }
            if let Some(kv) = new_key_version {
                user.key_version = kv;
            }
            let av = user.auth_version;
            let kv = user.key_version;
            let gen_num = g.alloc_seq();
            (
                200,
                json!({
                    "ok": true,
                    "upgraded": true,
                    "auth_version": av,
                    "key_version": kv,
                    "vault_unlocked": true,
                    "account_generation": gen_num
                }),
            )
        }

        // Atomic password rotation (verifies current credentials, rotates salt/verifier/envelope, revokes prior sessions, mints fresh token)
        ("POST", "/api/v1/auth/change-password") | ("POST", "/api/v1/auth/change_password") => {
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let existing = g.ensure_user_by_id(uid).clone();

            // 1. Fail-closed current credential verification BEFORE any state mutation.
            let current_v = body
                .get("current_auth_verifier_b64")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let current_pw = body
                .get("current_password")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());

            if let Some(ref expected_v) = existing.auth_verifier_b64 {
                if let Some(provided_v) = current_v {
                    if provided_v != expected_v {
                        return (401, json!({"error": "invalid_credentials"}));
                    }
                } else if let Some(ref expected_pw) = existing.password {
                    if current_pw != Some(expected_pw.as_str()) {
                        return (401, json!({"error": "invalid_credentials"}));
                    }
                } else {
                    return (401, json!({"error": "invalid_credentials"}));
                }
            } else if let Some(ref expected_pw) = existing.password {
                if current_pw != Some(expected_pw.as_str()) {
                    return (401, json!({"error": "invalid_credentials"}));
                }
            }

            // 2. Extract all replacement fields before mutating (guarantees atomicity on 400).
            let new_salt = body
                .get("new_auth_salt_b64")
                .or_else(|| body.get("auth_salt_b64"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let new_verifier = body
                .get("new_auth_verifier_b64")
                .or_else(|| body.get("auth_verifier_b64"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let new_password = body
                .get("new_password")
                .or_else(|| body.get("password"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let new_kdf = body
                .get("new_kdf_params")
                .or_else(|| body.get("new_argon2_params"))
                .or_else(|| body.get("kdf_params"))
                .or_else(|| body.get("argon2_params"))
                .cloned();
            let new_env = body
                .get("rewrapped_vault_envelope")
                .or_else(|| body.get("vault_envelope"))
                .cloned();
            let next_key_version = body
                .get("key_version")
                .and_then(Value::as_u64)
                .or_else(|| {
                    new_env
                        .as_ref()
                        .and_then(|e| e.get("key_version"))
                        .and_then(Value::as_u64)
                })
                .unwrap_or(existing.key_version.saturating_add(1));

            if new_verifier.is_none() && new_password.is_none() {
                return (
                    400,
                    json!({"error": "invalid_params", "message": "missing new credential"}),
                );
            }

            // 3. Apply atomic update to UserRecord.
            let user = g.ensure_user_by_id(uid);
            user.auth_version = body
                .get("auth_version")
                .and_then(Value::as_i64)
                .unwrap_or(if new_verifier.is_some() {
                    2
                } else {
                    user.auth_version
                });
            if let Some(salt) = new_salt {
                user.auth_salt_b64 = salt;
            }
            if let Some(v) = new_verifier {
                user.auth_verifier_b64 = Some(v);
                user.password = None;
            } else if let Some(pw) = new_password {
                user.password = Some(pw);
                user.auth_verifier_b64 = None;
            }
            if let Some(kdf) = new_kdf {
                user.kdf_params = kdf;
            }
            if let Some(env) = new_env {
                user.vault_envelope = Some(env);
            }
            user.key_version = next_key_version;
            let av = user.auth_version;
            let kv = user.key_version;

            // 4. Revoke all prior sessions for this user, bump generation, and issue a fresh token.
            let mut old_tokens = Vec::new();
            for sess in g.sessions_by_token.values_mut() {
                if sess.user_id == uid && !sess.revoked {
                    sess.revoked = true;
                    old_tokens.push(sess.token.clone());
                }
            }
            let revoked_count = old_tokens.len();
            for tok in old_tokens {
                g.revoked_tokens.insert(tok);
            }
            let device_name = caller_session
                .as_ref()
                .map(|s| s.device_name.as_str())
                .unwrap_or("iOS Device");
            let fresh_sess = g.mint_session(uid, device_name, ts);
            let gen_num = g.alloc_seq();

            (
                200,
                json!({
                    "ok": true,
                    "changed": true,
                    "upgraded": true,
                    "auth_version": av,
                    "key_version": kv,
                    "vault_unlocked": true,
                    "account_generation": gen_num,
                    "revoked_sessions_count": revoked_count,
                    "session_id": fresh_sess.session_id,
                    "token": fresh_sess.token,
                    "expires_at": ms_to_rfc3339(fresh_sess.expires_at_ms),
                    "expires_at_ms": fresh_sess.expires_at_ms
                }),
            )
        }

        // Auth me (compatible with gpui_telegram_client::UserMeResponse)
        ("GET", "/api/v1/auth/me") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let sid = caller_session
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_else(|| format!("sess_{uid}_default"));
            let device_name = caller_session
                .as_ref()
                .map(|s| s.device_name.clone())
                .unwrap_or_else(|| "iOS Device".to_owned());
            let created_ms = caller_session
                .as_ref()
                .map(|s| s.created_at_ms)
                .unwrap_or(ts);
            let expires_ms = caller_session
                .as_ref()
                .map(|s| s.expires_at_ms)
                .unwrap_or(ts + 86_400_000);
            if let Some(u) = g.users_by_id.get(&uid) {
                (
                    200,
                    json!({
                        "user_id": u.user_id,
                        "username": u.username,
                        "first_name": u.first_name,
                        "session_id": sid,
                        "device_name": device_name,
                        "account_generation": g.next_seq,
                        "created_at": ms_to_rfc3339(created_ms),
                        "expires_at": ms_to_rfc3339(expires_ms),
                        "expires_at_ms": expires_ms,
                        "auth_version": u.auth_version,
                        "key_version": u.key_version,
                        "two_factor_enabled": u.two_factor_password.is_some()
                    }),
                )
            } else {
                (
                    200,
                    json!({
                        "user_id": uid,
                        "username": format!("user_{uid}"),
                        "first_name": format!("User {uid}"),
                        "session_id": sid,
                        "device_name": device_name,
                        "account_generation": g.next_seq,
                        "created_at": ms_to_rfc3339(created_ms),
                        "expires_at": ms_to_rfc3339(expires_ms),
                        "expires_at_ms": expires_ms,
                        "auth_version": 2,
                        "key_version": 1,
                        "two_factor_enabled": false
                    }),
                )
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
            (200, json!({"user_id": uid, "sessions": list}))
        }
        ("DELETE", "/api/v1/auth/sessions/others")
        | ("POST", "/api/v1/auth/sessions/others")
        | ("POST", "/api/v1/auth/sessions/revoke-others") => {
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
        ("POST", p)
            if p.starts_with("/api/v1/auth/sessions/") && p.ends_with("/revoke") =>
        {
            let sid = p
                .trim_start_matches("/api/v1/auth/sessions/")
                .trim_end_matches("/revoke");
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
        ("GET", "/api/v1/auth/2fa")
        | ("GET", "/api/v1/auth/two-factor")
        | ("GET", "/api/v1/auth/two-factor/status") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let user = g.users_by_id.get(&uid);
            let enabled = user.and_then(|u| u.two_factor_password.as_ref()).is_some();
            let recovery_email = user.and_then(|u| u.two_factor_recovery_email.clone());
            (
                200,
                json!({
                    "enabled": enabled,
                    "has_password": enabled,
                    "hint": user.and_then(|u| u.two_factor_hint.clone()),
                    "recovery_email": recovery_email,
                    "has_recovery_email": recovery_email.as_deref().is_some_and(|s| !s.is_empty())
                }),
            )
        }
        ("POST" | "PUT", "/api/v1/auth/2fa")
        | ("POST" | "PUT", "/api/v1/auth/two-factor")
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
            let has_rec = user
                .two_factor_recovery_email
                .as_deref()
                .is_some_and(|s| !s.is_empty());
            (
                200,
                json!({
                    "enabled": true,
                    "has_password": true,
                    "hint": user.two_factor_hint,
                    "recovery_email": user.two_factor_recovery_email,
                    "has_recovery_email": has_rec
                }),
            )
        }
        ("DELETE", "/api/v1/auth/2fa")
        | ("DELETE", "/api/v1/auth/two-factor")
        | ("POST", "/api/v1/auth/two-factor/disable") => {
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
            (200, json!({"enabled": false, "has_password": false}))
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
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms
                }),
            )
        }

        // QR Device Pairing
        ("POST", "/api/v1/auth/pair/init") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let pair_token = format!("pair_{ts}_{seq}");
            let qr_url = format!("tg://login?token={pair_token}");
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
                    "qr_url": &qr_url,
                    "qr_payload": &qr_url,
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
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms
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
        ("GET" | "POST", "/api/v1/auth/pair/poll") => {
            let pair_token = body
                .get("pair_token")
                .and_then(Value::as_str)
                .or_else(|| extract_query_param(query, "pair_token"))
                .unwrap_or("");
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
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms
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
                    "expires_at_ms": session_json["expires_at_ms"],
                    "session": session_json
                }),
            )
        }

        // Email OTP
        ("POST", "/api/v1/auth/email/send-code") | ("POST", "/api/v1/auth/email/send_code") => (
            200,
            json!({
                "ok": true,
                "sent": true,
                "expires_at_ms": ts + 600_000,
                "resend_after_ms": ts + 60_000
            }),
        ),
        ("POST", "/api/v1/auth/email/verify-code")
        | ("POST", "/api/v1/auth/email/verify")
        | ("POST", "/api/v1/auth/email/verify_code")
        | ("POST", "/api/v1/auth/email/register") => {
            let mut g = state.lock().expect("state lock");
            g.next_user_id += 1;
            let uid = g.next_user_id;
            g.ensure_user_by_id(uid);
            let sess = g.mint_session(uid, "iOS Email Auth", ts);
            (
                200,
                json!({
                    "ok": true,
                    "authenticated": true,
                    "user_id": uid,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms
                }),
            )
        }

        // Phone OTP (stateful hash tracking, 5-attempt lockout, 2FA gate without premature token)
        ("POST", "/api/v1/auth/phone/send-code") | ("POST", "/api/v1/auth/phone/send_code") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let hash = format!("pch_{ts}_{seq}");
            let expires_at_ms = ts + 300_000;
            g.phone_challenges.insert(
                hash.clone(),
                PhoneOtpRecord {
                    expected_code: "123456".to_owned(),
                    attempts: 0,
                    expires_at_ms,
                },
            );
            (
                200,
                json!({
                    "ok": true,
                    "sent": true,
                    "phone_code_hash": hash,
                    "debug_code": "123456",
                    "expires_at_ms": expires_at_ms,
                    "resend_after_ms": ts + 60_000
                }),
            )
        }
        ("POST", "/api/v1/auth/phone/verify")
        | ("POST", "/api/v1/auth/phone/verify-code")
        | ("POST", "/api/v1/auth/phone/verify_code")
        | ("POST", "/api/v1/auth/phone/login") => {
            let code = body.get("code").and_then(Value::as_str).map(str::trim);
            if let Some(c) = code {
                if c.len() != 6 || !c.bytes().all(|b| b.is_ascii_digit()) {
                    return (400, json!({"error": "invalid_code"}));
                }
            }
            let hash_opt = body
                .get("phone_code_hash")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let mut g = state.lock().expect("state lock");
            if let Some(hash) = hash_opt {
                let Some(rec) = g.phone_challenges.get_mut(hash) else {
                    return (400, json!({"error": "invalid_phone_code_hash"}));
                };
                if rec.attempts >= 5 {
                    return (400, json!({"error": "too_many_attempts"}));
                }
                if ts > rec.expires_at_ms {
                    return (400, json!({"error": "code_expired"}));
                }
                if let Some(c) = code {
                    if c != rec.expected_code {
                        rec.attempts += 1;
                        if rec.attempts >= 5 {
                            return (400, json!({"error": "too_many_attempts"}));
                        }
                        return (400, json!({"error": "invalid_code"}));
                    }
                }
                g.phone_challenges.remove(hash);
            }

            g.next_user_id += 1;
            let uid = g.next_user_id;
            g.ensure_user_by_id(uid);
            let dev_name = body
                .get("device_name")
                .and_then(Value::as_str)
                .unwrap_or("iOS Phone Auth")
                .to_owned();
            if body
                .get("require_2fa_account")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let seq = g.alloc_seq();
                let challenge_token = format!("tfa_{uid}_{seq}");
                g.two_factor_challenges.insert(
                    challenge_token.clone(),
                    TwoFactorChallenge {
                        user_id: uid,
                        device_name: dev_name,
                        expires_at_ms: ts + 300_000,
                    },
                );
                return (
                    200,
                    json!({
                        "requires_2fa": true,
                        "challenge_token": challenge_token,
                        "user_id": uid
                    }),
                );
            }
            let sess = g.mint_session(uid, &dev_name, ts);
            (
                200,
                json!({
                    "ok": true,
                    "authenticated": true,
                    "user_id": uid,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms
                }),
            )
        }

        // Canonical WebAuthn / Passkey lifecycle (register, list, delete, single-use challenge, verify)
        ("POST", "/api/v1/auth/passkey/register")
        | ("POST", "/api/v1/auth/passkeys")
        | ("POST", "/api/v1/auth/passkey/register/complete") => {
            let credential_id = body
                .get("credential_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let public_key = body
                .get("public_key")
                .or_else(|| body.get("public_key_b64"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let (Some(cred_id), Some(pub_key)) = (credential_id, public_key) else {
                return (
                    400,
                    json!({"error": "invalid_passkey", "message": "credential_id and public_key are required"}),
                );
            };
            let device_name = body
                .get("device_name")
                .and_then(Value::as_str)
                .unwrap_or("iOS Passkey")
                .to_owned();
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            g.ensure_user_by_id(uid);
            let rec = PasskeyRecord {
                credential_id: cred_id.to_owned(),
                user_id: uid,
                public_key: pub_key.to_owned(),
                device_name: device_name.clone(),
                sign_count: 0,
                created_at_ms: ts,
            };
            g.passkeys.insert(cred_id.to_owned(), rec);
            (
                200,
                json!({
                    "ok": true,
                    "credential_id": cred_id,
                    "user_id": uid,
                    "device_name": device_name,
                    "sign_count": 0,
                    "created_at_ms": ts
                }),
            )
        }
        ("GET", "/api/v1/auth/passkey/list")
        | ("GET", "/api/v1/auth/passkeys")
        | ("GET", "/api/v1/auth/passkey") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let mut list: Vec<Value> = g
                .passkeys
                .values()
                .filter(|p| p.user_id == uid)
                .map(|p| {
                    json!({
                        "credential_id": p.credential_id,
                        "user_id": p.user_id,
                        "public_key": p.public_key,
                        "device_name": p.device_name,
                        "sign_count": p.sign_count,
                        "created_at_ms": p.created_at_ms
                    })
                })
                .collect();
            list.sort_by(|a, b| {
                a["credential_id"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["credential_id"].as_str().unwrap_or(""))
            });
            (200, json!({"passkeys": list, "total": list.len()}))
        }
        ("DELETE", p)
            if p.starts_with("/api/v1/auth/passkey/")
                || p.starts_with("/api/v1/auth/passkeys/") =>
        {
            let cred_id = p
                .strip_prefix("/api/v1/auth/passkeys/")
                .or_else(|| p.strip_prefix("/api/v1/auth/passkey/"))
                .unwrap_or("");
            let mut g = state.lock().expect("state lock");
            if g.passkeys.remove(cred_id).is_some() {
                (200, json!({"ok": true, "deleted": true, "credential_id": cred_id}))
            } else {
                (404, json!({"error": "unknown_credential"}))
            }
        }
        ("POST" | "GET", "/api/v1/auth/passkey/challenge")
        | ("POST" | "GET", "/api/v1/auth/passkeys/challenge") => {
            let mut g = state.lock().expect("state lock");
            let user_id = body
                .get("user_id")
                .and_then(Value::as_u64)
                .or_else(|| {
                    body.get("username")
                        .and_then(Value::as_str)
                        .or_else(|| extract_query_param(query, "username"))
                        .and_then(|uname| g.username_to_id.get(uname).copied())
                })
                .or_else(|| caller_session.as_ref().map(|s| s.user_id));
            let seq = g.alloc_seq();
            let challenge = format!("pkchal_{seq}_{}", Uuid::new_v4().simple());
            let timeout_ms: i64 = 300_000;
            let expires_at_ms = ts + timeout_ms;
            g.passkey_challenges.insert(
                challenge.clone(),
                PasskeyChallengeRecord {
                    user_id,
                    expires_at_ms,
                },
            );
            (
                200,
                json!({
                    "challenge": challenge,
                    "rp_id": "gpui.telegram.local",
                    "timeout_ms": timeout_ms,
                    "user_id": user_id,
                    "expires_at_ms": expires_at_ms
                }),
            )
        }
        ("POST", "/api/v1/auth/passkey/verify")
        | ("POST", "/api/v1/auth/passkeys/verify")
        | ("POST", "/api/v1/auth/passkey/login")
        | ("POST", "/api/v1/auth/passkey/authenticate") => {
            let challenge = body
                .get("challenge")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let credential_id = body
                .get("credential_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let Some(chal_str) = challenge else {
                return (400, json!({"error": "invalid_challenge"}));
            };
            let mut g = state.lock().expect("state lock");
            // Atomically consume single-use challenge to block replay attacks.
            let Some(chal_rec) = g.passkey_challenges.remove(chal_str) else {
                return (400, json!({"error": "invalid_challenge"}));
            };
            if ts > chal_rec.expires_at_ms {
                return (400, json!({"error": "invalid_challenge"}));
            }
            let Some(cred_id) = credential_id else {
                return (401, json!({"error": "unknown_credential"}));
            };
            let Some(pk) = g.passkeys.get_mut(cred_id) else {
                return (401, json!({"error": "unknown_credential"}));
            };
            if let Some(expected_uid) = chal_rec.user_id {
                if pk.user_id != expected_uid {
                    return (401, json!({"error": "unknown_credential"}));
                }
            }
            pk.sign_count += 1;
            let uid = pk.user_id;
            let sign_count = pk.sign_count;
            let device_name = pk.device_name.clone();
            let sess = g.mint_session(uid, &device_name, ts);
            (
                200,
                json!({
                    "ok": true,
                    "verified": true,
                    "authenticated": true,
                    "user_id": uid,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms,
                    "sign_count": sign_count
                }),
            )
        }

        // Alipay OAuth & QR state machine (fbs_<uuid>, qr_payload, single-use state & auth_code, cancel -> cancelled)
        ("POST", "/api/v1/auth/alipay/start") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let attempt_id = format!("alipay_{seq}");
            let state_tok = format!("fbs_{}", Uuid::new_v4().simple());
            let auth_url = format!(
                "https://openauth.alipay.com/oauth2/publicAppAuthorize.htm?state={state_tok}"
            );
            let expires_in_ms: i64 = 600_000;
            let expires_at_ms = ts + expires_in_ms;
            g.alipay_state_to_attempt
                .insert(state_tok.clone(), attempt_id.clone());
            g.alipay_attempts.insert(
                attempt_id.clone(),
                AlipayAttemptRecord {
                    attempt_id: attempt_id.clone(),
                    state: state_tok.clone(),
                    status: "pending".to_owned(),
                    issued_session: None,
                    expires_at_ms,
                },
            );
            (
                200,
                json!({
                    "attempt_id": attempt_id,
                    "state": state_tok,
                    "auth_url": &auth_url,
                    "qr_payload": &auth_url,
                    "poll_after_ms": 1000,
                    "expires_in_ms": expires_in_ms,
                    "expires_at_ms": expires_at_ms
                }),
            )
        }
        ("GET", p) if p.starts_with("/api/v1/auth/alipay/poll/") => {
            let attempt_id = p.trim_start_matches("/api/v1/auth/alipay/poll/");
            poll_alipay_attempt(state, attempt_id, ts)
        }
        ("GET" | "POST", "/api/v1/auth/alipay/poll") => {
            let attempt_id = body
                .get("attempt_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .or_else(|| extract_query_param(query, "attempt_id"))
                .unwrap_or("");
            poll_alipay_attempt(state, attempt_id, ts)
        }
        ("POST", "/api/v1/auth/alipay/cancel") => {
            let attempt_id = body
                .get("attempt_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let state_tok = body
                .get("state")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            cancel_alipay_attempt(state, attempt_id, state_tok)
        }
        ("POST", p) if p.starts_with("/api/v1/auth/alipay/cancel/") => {
            let attempt_id = p.trim_start_matches("/api/v1/auth/alipay/cancel/").trim();
            cancel_alipay_attempt(
                state,
                (!attempt_id.is_empty()).then_some(attempt_id),
                None,
            )
        }
        ("DELETE", p) if p.starts_with("/api/v1/auth/alipay/") => {
            let attempt_id = p.trim_start_matches("/api/v1/auth/alipay/").trim();
            cancel_alipay_attempt(
                state,
                (!attempt_id.is_empty()).then_some(attempt_id),
                None,
            )
        }
        ("POST" | "GET", "/api/v1/auth/alipay/callback")
        | ("POST", "/api/v1/auth/alipay/exchange") => {
            let state_tok_opt = body
                .get("state")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| extract_query_param(query, "state"));
            let Some(state_tok) = state_tok_opt else {
                return (
                    400,
                    json!({"error": "oauth_state_mismatch", "message": "missing oauth state"}),
                );
            };
            let auth_code_opt = body
                .get("auth_code")
                .or_else(|| body.get("code"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| extract_query_param(query, "auth_code"))
                .or_else(|| extract_query_param(query, "code"));
            let Some(auth_code) = auth_code_opt else {
                return (
                    400,
                    json!({"error": "invalid_params", "message": "missing auth_code"}),
                );
            };
            let expected_attempt = body
                .get("attempt_id")
                .or_else(|| body.get("expected_attempt_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| extract_query_param(query, "attempt_id"));

            let mut g = state.lock().expect("state lock");
            let Some(attempt_id) = g.alipay_state_to_attempt.get(state_tok).cloned() else {
                return (
                    400,
                    json!({"error": "oauth_state_mismatch", "message": "unknown oauth state"}),
                );
            };
            if let Some(exp) = expected_attempt {
                if exp != attempt_id {
                    return (
                        400,
                        json!({"error": "oauth_state_mismatch", "message": "attempt_id mismatch"}),
                    );
                }
            }
            let Some(attempt_snapshot) = g.alipay_attempts.get(&attempt_id).cloned() else {
                return (404, json!({"error": "oauth_attempt_not_found"}));
            };
            if attempt_snapshot.state != state_tok {
                return (
                    400,
                    json!({"error": "oauth_state_mismatch", "message": "state token mismatch"}),
                );
            }
            if attempt_snapshot.status == "cancelled" {
                return (
                    409,
                    json!({
                        "error": "oauth_cancelled",
                        "message": "oauth attempt cancelled",
                        "status": "cancelled"
                    }),
                );
            }
            if attempt_snapshot.status != "pending" {
                return (
                    400,
                    json!({
                        "error": "oauth_expired",
                        "message": "oauth state already consumed"
                    }),
                );
            }
            if ts > attempt_snapshot.expires_at_ms {
                if let Some(a) = g.alipay_attempts.get_mut(&attempt_id) {
                    a.status = "expired".to_owned();
                }
                return (400, json!({"error": "oauth_expired", "status": "expired"}));
            }
            if g.consumed_alipay_codes.contains(auth_code) {
                return (
                    400,
                    json!({
                        "error": "oauth_expired",
                        "message": "auth_code already consumed"
                    }),
                );
            }

            g.consumed_alipay_codes.insert(auth_code.to_owned());
            g.next_user_id += 1;
            let uid = g.next_user_id;
            g.ensure_user_by_id(uid);
            let sess = g.mint_session(uid, "iOS Alipay", ts);
            let session_json = json!({
                "user_id": uid,
                "session_id": sess.session_id,
                "token": sess.token,
                "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                "expires_at_ms": sess.expires_at_ms
            });
            if let Some(a) = g.alipay_attempts.get_mut(&attempt_id) {
                a.status = "consumed".to_owned();
                a.issued_session = Some(session_json.clone());
            }
            (
                200,
                json!({
                    "authenticated": true,
                    "attempt_id": attempt_id,
                    "user_id": uid,
                    "session_id": sess.session_id,
                    "token": sess.token,
                    "expires_at": ms_to_rfc3339(sess.expires_at_ms),
                    "expires_at_ms": sess.expires_at_ms,
                    "session": session_json
                }),
            )
        }

        // Chats, invites & join (with string/integer client_nonce idempotency + deduplicated member_count)
        ("GET", "/api/v1/chats") => {
            let g = state.lock().expect("state lock");
            let mut chats: Vec<Value> = g
                .chats
                .values()
                .map(|c| {
                    json!({
                        "id": c.chat_id,
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
            let nonce_key = extract_client_nonce_key(body);
            let mut g = state.lock().expect("state lock");
            if let Some(ref nk) = nonce_key {
                if let Some(&existing_cid) = g.chats_by_nonce.get(nk) {
                    if let Some(existing) = g.chats.get(&existing_cid) {
                        return (
                            200,
                            json!({
                                "id": existing.chat_id,
                                "chat_id": existing.chat_id,
                                "title": existing.title,
                                "chat_type": existing.chat_type,
                                "member_count": existing.member_count,
                                "deduplicated": true
                            }),
                        );
                    }
                }
            }
            let creator_uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let member_count: u64 =
                if let Some(arr) = body.get("member_ids").and_then(Value::as_array) {
                    let mut unique: HashSet<u64> = HashSet::new();
                    unique.insert(creator_uid);
                    for v in arr {
                        if let Some(id) = v.as_u64() {
                            unique.insert(id);
                        }
                    }
                    unique.len() as u64
                } else if let Some(arr) = body.get("recipients").and_then(Value::as_array) {
                    let mut unique: HashSet<u64> = HashSet::new();
                    unique.insert(creator_uid);
                    for v in arr {
                        if let Some(id) = v
                            .get("user_id")
                            .and_then(Value::as_u64)
                            .or_else(|| v.as_u64())
                        {
                            unique.insert(id);
                        }
                    }
                    unique.len() as u64
                } else {
                    2
                };

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
                member_count,
            };
            g.chats.insert(cid, rec);
            if let Some(nk) = nonce_key {
                g.chats_by_nonce.insert(nk, cid);
            }
            g.chat_pts.entry(cid).or_insert(0);
            (
                200,
                json!({
                    "id": cid,
                    "chat_id": cid,
                    "title": title,
                    "chat_type": chat_type,
                    "member_count": member_count,
                    "creator_id": creator_uid,
                    "deduplicated": false
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
            let max_uses = body
                .get("max_uses")
                .or_else(|| body.get("usage_limit"))
                .and_then(Value::as_u64);
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
                    "usage_limit": max_uses,
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
                        "usage_limit": i.max_uses,
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
        ("GET", p) if p.starts_with("/api/v1/chats/") && p.ends_with("/topics") => {
            let cid: i64 = p
                .trim_start_matches("/api/v1/chats/")
                .trim_end_matches("/topics")
                .parse()
                .unwrap_or(1);
            (200, json!({"chat_id": cid, "topics": []}))
        }
        ("GET", p)
            if p.starts_with("/api/v1/chats/")
                && !p.trim_start_matches("/api/v1/chats/").contains('/') =>
        {
            let cid_str = p.trim_start_matches("/api/v1/chats/");
            if let Ok(cid) = cid_str.parse::<i64>() {
                let g = state.lock().expect("state lock");
                if let Some(chat) = g.chats.get(&cid) {
                    return (
                        200,
                        json!({
                            "id": chat.chat_id,
                            "chat_id": chat.chat_id,
                            "title": chat.title,
                            "chat_type": chat.chat_type,
                            "member_count": chat.member_count,
                            "is_group": chat.chat_type > 0
                        }),
                    );
                }
                return (
                    200,
                    json!({
                        "id": cid,
                        "chat_id": cid,
                        "title": format!("Chat {cid}"),
                        "chat_type": 0,
                        "member_count": 2,
                        "is_group": false
                    }),
                );
            }
            (200, json!({"ok": true, "path": path, "method": method, "timestamp_ms": ts}))
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
            let sender_id = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            if let Some(nonce) = client_nonce {
                if let Some(existing) = g.chat_nonce_map.get(&(cid, nonce)).cloned() {
                    let mut out = stored_message_to_json(&existing);
                    if let Some(obj) = out.as_object_mut() {
                        obj.insert("sender_id".to_owned(), json!(sender_id));
                    }
                    return (200, out);
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
            let mut out = stored_message_to_json(&msg);
            if let Some(obj) = out.as_object_mut() {
                obj.insert("sender_id".to_owned(), json!(sender_id));
            }
            (200, out)
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
            let current_pts = g.chat_pts.get(&cid).copied().unwrap_or(0);
            let msgs: Vec<Value> = g
                .chat_messages
                .get(&cid)
                .map(|v| v.iter().map(stored_message_to_json).collect())
                .unwrap_or_default();
            (
                200,
                json!({
                    "chat_id": cid,
                    "messages": msgs,
                    "has_more": false,
                    "current_pts": current_pts
                }),
            )
        }
        ("POST", "/api/v1/messages/edit") => {
            let cid = body.get("chat_id").and_then(Value::as_i64).unwrap_or(1);
            let mid = body.get("message_id").and_then(Value::as_i64).unwrap_or(0);
            let new_text = body
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let mut g = state.lock().expect("state lock");
            let pts = {
                let entry = g.chat_pts.entry(cid).or_insert(0);
                *entry += 1;
                *entry
            };
            if let Some(msgs) = g.chat_messages.get_mut(&cid) {
                if let Some(m) = msgs.iter_mut().find(|m| m.message_id == mid) {
                    m.text = new_text.clone();
                }
            }
            (
                200,
                json!({
                    "ok": true,
                    "success": true,
                    "message_id": mid,
                    "text": new_text,
                    "pts": pts,
                    "edited_at_ms": ts,
                    "status": "edited"
                }),
            )
        }
        ("POST", "/api/v1/messages/delete") => {
            let cid = body.get("chat_id").and_then(Value::as_i64).unwrap_or(1);
            let revoke = body
                .get("revoke")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let ids: HashSet<i64> = body
                .get("message_ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            let mut g = state.lock().expect("state lock");
            let pts = {
                let entry = g.chat_pts.entry(cid).or_insert(0);
                *entry += 1;
                *entry
            };
            let mut deleted_count = ids.len();
            if let Some(msgs) = g.chat_messages.get_mut(&cid) {
                let before = msgs.len();
                msgs.retain(|m| !ids.contains(&m.message_id));
                let removed = before.saturating_sub(msgs.len());
                if removed > 0 {
                    deleted_count = removed;
                }
            }
            (
                200,
                json!({
                    "ok": true,
                    "success": true,
                    "pts": pts,
                    "deleted_count": deleted_count,
                    "revoke": revoke
                }),
            )
        }
        ("POST", "/api/v1/messages/reaction") => {
            let cid = body.get("chat_id").and_then(Value::as_i64).unwrap_or(1);
            let mut g = state.lock().expect("state lock");
            let pts = {
                let entry = g.chat_pts.entry(cid).or_insert(0);
                *entry += 1;
                *entry
            };
            (
                200,
                json!({
                    "ok": true,
                    "success": true,
                    "pts": pts,
                    "status": "reaction_updated"
                }),
            )
        }
        ("POST", "/api/v1/messages/read") => {
            let cid = body.get("chat_id").and_then(Value::as_i64).unwrap_or(1);
            let mut g = state.lock().expect("state lock");
            let pts = {
                let entry = g.chat_pts.entry(cid).or_insert(0);
                *entry += 1;
                *entry
            };
            (
                200,
                json!({
                    "ok": true,
                    "success": true,
                    "pts": pts,
                    "status": "history_read"
                }),
            )
        }

        // PTS sync difference & state/pts
        ("POST" | "GET", "/api/v1/sync/get_difference")
        | ("GET" | "POST", "/api/v1/sync/difference") => {
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
            let current_pts = g
                .chat_pts
                .get(&chat_id)
                .copied()
                .unwrap_or(0)
                .max(floor.saturating_sub(1));
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
        ("GET", "/api/v1/sync/state") | ("GET", "/api/v1/sync/pts") => {
            let chat_id = extract_query_param(query, "chat_id")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(1);
            let g = state.lock().expect("state lock");
            let current_pts = g.chat_pts.get(&chat_id).copied().unwrap_or(0);
            (
                200,
                json!({
                    "chat_id": chat_id,
                    "pts": current_pts,
                    "current_pts": current_pts,
                    "seq": g.next_seq
                }),
            )
        }

        // Resumable chunked media upload init / status / complete + edit + instant-view API
        ("POST", "/api/v1/media/upload/init") => {
            let total_size = body
                .get("total_size")
                .or_else(|| body.get("size_bytes"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let part_size = body
                .get("part_size")
                .or_else(|| body.get("chunk_size"))
                .and_then(Value::as_u64)
                .unwrap_or(65_536);
            if total_size > MAX_UPLOAD_TOTAL_SIZE {
                return (400, json!({"error": "upload_size_exceeded"}));
            }
            if part_size == 0 || part_size > MAX_UPLOAD_PART_SIZE {
                return (400, json!({"error": "invalid_part_size"}));
            }
            let filename = body
                .get("filename")
                .cloned()
                .unwrap_or_else(|| json!("upload.bin"));
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
                    filename: filename.clone(),
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
                    "filename": filename,
                    "part_size": part_size,
                    "chunk_size": part_size,
                    "total_size": total_size,
                    "size_bytes": total_size
                }),
            )
        }
        ("GET", p) if p.starts_with("/api/v1/media/upload/") && !p.contains("/part/") => {
            let raw_id = p.trim_start_matches("/api/v1/media/upload/");
            let upload_id = raw_id.strip_suffix("/status").unwrap_or(raw_id);
            let g = state.lock().expect("state lock");
            let Some(session) = g.uploads.get(upload_id) else {
                return (404, json!({"error": "upload_not_found"}));
            };
            let received: Vec<usize> = session.parts.keys().copied().collect();
            let bytes_received: usize = session.parts.values().map(Vec::len).sum();
            (
                200,
                json!({
                    "upload_id": session.upload_id,
                    "filename": session.filename,
                    "total_size": session.total_size,
                    "size_bytes": session.total_size,
                    "part_size": session.part_size,
                    "chunk_size": session.part_size,
                    "bytes_received": bytes_received,
                    "received_parts": received,
                    "status": "in_progress"
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
                    "upload_id": session.upload_id,
                    "filename": session.filename,
                    "file_id": file_id,
                    "media_id": file_id,
                    "sha256": digest,
                    "size": size,
                    "status": "completed"
                }),
            )
        }
        ("POST", "/api/v1/media/edit") => {
            let media_id = body
                .get("media_id")
                .or_else(|| body.get("file_id"))
                .and_then(Value::as_str)
                .unwrap_or("edited_media");
            (
                200,
                json!({
                    "ok": true,
                    "edited": true,
                    "media_id": media_id,
                    "file_id": media_id,
                    "timestamp_ms": ts
                }),
            )
        }
        ("GET" | "POST", "/api/v1/media/instant-view")
        | ("GET" | "POST", "/api/v1/media/instant_view") => {
            let url = body
                .get("url")
                .and_then(Value::as_str)
                .or_else(|| extract_query_param(query, "url"))
                .unwrap_or("/article/v167");
            (
                200,
                json!({
                    "ok": true,
                    "url": url,
                    "title": "Fabushi v1.6.7",
                    "blocks": [
                        {"type": "heading", "text": "Fabushi v1.6.7"},
                        {"type": "paragraph", "text": "Instant View article content"}
                    ]
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
            let mut out = serde_json::Map::new();
            out.insert("envelope".to_owned(), env.clone().unwrap_or(Value::Null));
            if let Some(Value::Object(map)) = env {
                for (k, v) in map {
                    if k != "envelope" {
                        out.insert(k, v);
                    }
                }
            }
            (200, Value::Object(out))
        }
        ("PUT" | "POST", "/api/v1/user-vault/envelope")
        | ("PUT" | "POST", "/api/v1/vault/envelope") => {
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let user = g.ensure_user_by_id(uid);
            let kdf_alg = body
                .get("kdf_alg")
                .and_then(Value::as_str)
                .unwrap_or("split_key_v2")
                .to_owned();
            user.vault_envelope = Some(body.clone());
            (200, json!({"ok": true, "saved": true, "kdf_alg": kdf_alg}))
        }
        ("GET", "/api/v1/user-vault/items") | ("GET", "/api/v1/vault/items") => {
            let cat_filter = extract_query_param(query, "category");
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let items: Vec<Value> = g
                .vault_items
                .iter()
                .filter(|((u, cat, _), _)| {
                    *u == uid && cat_filter.is_none_or(|f| f == cat.as_str())
                })
                .map(|(_, v)| v.clone())
                .collect();
            (200, json!({"items": items}))
        }
        ("POST" | "PUT", "/api/v1/user-vault/items")
        | ("POST" | "PUT", "/api/v1/vault/items") => {
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
            g.vault_items
                .insert((uid, cat.clone(), key.clone()), body.clone());
            (200, json!({"ok": true, "category": cat, "item_key": key}))
        }
        ("GET", p)
            if p.starts_with("/api/v1/user-vault/items/") || p.starts_with("/api/v1/vault/items/") =>
        {
            let rest = p
                .strip_prefix("/api/v1/user-vault/items/")
                .or_else(|| p.strip_prefix("/api/v1/vault/items/"))
                .unwrap_or("");
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            if let Some((raw_cat, raw_key)) = rest.split_once('/') {
                let cat = percent_decode(raw_cat);
                let key = percent_decode(raw_key);
                if let Some(item) = g.vault_items.get(&(uid, cat, key)) {
                    (200, item.clone())
                } else {
                    (404, json!({"error": "vault_item_not_found"}))
                }
            } else {
                let cat = percent_decode(rest);
                let items: Vec<Value> = g
                    .vault_items
                    .iter()
                    .filter(|((u, c, _), _)| *u == uid && c == &cat)
                    .map(|(_, v)| v.clone())
                    .collect();
                (200, json!({"category": cat, "items": items}))
            }
        }
        ("PUT" | "POST", p)
            if p.starts_with("/api/v1/user-vault/items/") || p.starts_with("/api/v1/vault/items/") =>
        {
            let rest = p
                .strip_prefix("/api/v1/user-vault/items/")
                .or_else(|| p.strip_prefix("/api/v1/vault/items/"))
                .unwrap_or("");
            let (raw_cat, raw_key) = rest.split_once('/').unwrap_or(("general", rest));
            let cat = percent_decode(raw_cat);
            let key = percent_decode(raw_key);
            let mut stored = body.clone();
            if let Some(obj) = stored.as_object_mut() {
                obj.entry("category".to_owned())
                    .or_insert_with(|| json!(cat));
                obj.entry("item_key".to_owned())
                    .or_insert_with(|| json!(key));
            }
            let mut g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            g.vault_items
                .insert((uid, cat.clone(), key.clone()), stored);
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
            let (raw_cat, raw_key) = rest.split_once('/').unwrap_or(("general", rest));
            let cat = percent_decode(raw_cat);
            let key = percent_decode(raw_key);
            g.vault_items.remove(&(uid, cat.clone(), key.clone()));
            (200, json!({"deleted": true, "category": cat, "item_key": key}))
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
        ("GET" | "POST", p)
            if p == "/api/v1/user-vault/device-sync/status"
                || p == "/api/v1/vault/device-sync/status"
                || p.starts_with("/api/v1/user-vault/device-sync/status/")
                || p.starts_with("/api/v1/vault/device-sync/status/")
                || ((p.starts_with("/api/v1/user-vault/device-sync/")
                    || p.starts_with("/api/v1/vault/device-sync/"))
                    && !matches!(
                        p.rsplit('/').next().unwrap_or(""),
                        "pending" | "request" | "approve" | "deny" | "status"
                    )) =>
        {
            let req_id = body
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| extract_query_param(query, "request_id"))
                .map(str::to_owned)
                .unwrap_or_else(|| p.rsplit('/').next().unwrap_or("").to_owned());
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
        (_, p)
            if matches_segment_prefix(p, "/api/v1/user-vault/obscura")
                || matches_segment_prefix(p, "/api/v1/vault/obscura") =>
        {
            (
                200,
                json!({
                    "ok": true,
                    "obscura": true,
                    "path": path,
                    "method": method,
                    "timestamp_ms": ts
                }),
            )
        }

        // Canonical Agent runs & turn (compatible with gpui_telegram_client::AgentRunResponse)
        ("POST", "/api/v1/agent/runs") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let session_id = body
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| caller_session.as_ref().map(|s| s.session_id.clone()))
                .unwrap_or_else(|| format!("agent_sess_{seq}"));
            let prompt = body
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let response = format!("Completed agent run for: {prompt}");
            (
                200,
                json!({
                    "run_id": format!("run_{seq}"),
                    "session_id": session_id,
                    "user_id": uid,
                    "status": "completed",
                    "prompt": prompt,
                    "response": response,
                    "outputs": []
                }),
            )
        }
        ("POST", "/api/v1/agent/turn") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let session_id = body
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| caller_session.as_ref().map(|s| s.session_id.clone()))
                .unwrap_or_else(|| format!("agent_sess_{seq}"));
            let prompt = body
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let response = format!("Executed turn for: {prompt}");
            (
                200,
                json!({
                    "turn_id": format!("turn_{seq}"),
                    "session_id": session_id,
                    "caller_id": uid,
                    "status": "executed",
                    "prompt": prompt,
                    "response": response,
                    "outputs": []
                }),
            )
        }

        // Canonical singular /api/v1/computer/* lease broker & action endpoints (gpui_telegram_server/src/server.rs:283-289)
        ("POST", "/api/v1/computer/lease") | ("POST", "/api/v1/computer/lease/acquire") => {
            let device_id = body
                .get("device_id")
                .or_else(|| body.get("node_id"))
                .and_then(Value::as_str)
                .unwrap_or("default_device")
                .to_owned();
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let entry = g
                .computer_leases
                .entry(device_id.clone())
                .or_insert_with(|| ComputerLeaseRecord {
                    lease_id: format!("lease_{seq}"),
                    generation: 0,
                    override_epoch: 0,
                });
            entry.generation += 1;
            entry.lease_id = format!("lease_{seq}");
            let lease_id = entry.lease_id.clone();
            let generation = entry.generation;
            let override_epoch = entry.override_epoch;
            (
                200,
                json!({
                    "lease_id": lease_id,
                    "target_device_id": device_id,
                    "device_id": device_id,
                    "generation": generation,
                    "override_epoch": override_epoch,
                    "state": "active",
                    "is_valid": true
                }),
            )
        }
        ("POST", "/api/v1/computer/lease/emergency_takeover") => {
            let device_id = body
                .get("device_id")
                .or_else(|| body.get("node_id"))
                .and_then(Value::as_str)
                .unwrap_or("default_device")
                .to_owned();
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let entry = g
                .computer_leases
                .entry(device_id.clone())
                .or_insert_with(|| ComputerLeaseRecord {
                    lease_id: format!("lease_{seq}"),
                    generation: 0,
                    override_epoch: 0,
                });
            entry.generation += 1;
            entry.override_epoch += 1;
            entry.lease_id = format!("lease_takeover_{seq}");
            let new_generation = entry.generation;
            let override_epoch = entry.override_epoch;
            (
                200,
                json!({
                    "status": "emergency_takeover_executed",
                    "action": "preempt_and_fence",
                    "target_device_id": device_id,
                    "new_generation": new_generation,
                    "generation": new_generation,
                    "override_epoch": override_epoch,
                    "latency_sub_10ms": true,
                    "elapsed_ms": 1
                }),
            )
        }
        ("POST", "/api/v1/computer/action") => {
            let device_id = body
                .get("device_id")
                .or_else(|| body.get("node_id"))
                .and_then(Value::as_str)
                .unwrap_or("default_device")
                .to_owned();
            let req_gen = body.get("generation").and_then(Value::as_u64);
            let g = state.lock().expect("state lock");
            if let Some(lease) = g.computer_leases.get(&device_id) {
                if let Some(rg) = req_gen {
                    if rg < lease.generation {
                        return (
                            403,
                            json!({
                                "error": "stale_generation",
                                "current_generation": lease.generation
                            }),
                        );
                    }
                }
                return (
                    200,
                    json!({
                        "status": "action_executed",
                        "generation": lease.generation,
                        "lease_id": lease.lease_id,
                        "target_device_id": device_id
                    }),
                );
            }
            (
                200,
                json!({
                    "status": "action_executed",
                    "generation": req_gen.unwrap_or(1),
                    "lease_id": "lease_default",
                    "target_device_id": device_id
                }),
            )
        }

        // Canonical search, WebRTC, Tasks, MCP, Talk & Magpie endpoints
        ("GET", "/api/v1/search/messages") => {
            let q = extract_query_param(query, "q").unwrap_or("");
            let cid_filter =
                extract_query_param(query, "chat_id").and_then(|s| s.parse::<i64>().ok());
            let q_lower = q.to_ascii_lowercase();
            let g = state.lock().expect("state lock");
            let mut results = Vec::new();
            for (&cid, msgs) in &g.chat_messages {
                if cid_filter.is_some_and(|f| f != cid) {
                    continue;
                }
                for m in msgs {
                    if q_lower.is_empty() || m.text.to_ascii_lowercase().contains(&q_lower) {
                        results.push(stored_message_to_json(m));
                    }
                }
            }
            let total = results.len();
            (200, json!({"query": q, "results": results, "total": total}))
        }
        ("POST", "/api/v1/webrtc/call/invite") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let caller_id = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            let callee_id = body
                .get("callee_id")
                .or_else(|| body.get("peer_id"))
                .cloned()
                .unwrap_or_else(|| json!(0));
            (
                200,
                json!({
                    "call_id": format!("call_{seq}"),
                    "caller_id": caller_id,
                    "callee_id": callee_id,
                    "status": "invited"
                }),
            )
        }
        ("POST", "/api/v1/webrtc/call/signal") => {
            let g = state.lock().expect("state lock");
            let uid = caller_session
                .as_ref()
                .map(|s| s.user_id)
                .unwrap_or_else(|| g.default_user_id());
            (
                200,
                json!({
                    "user_id": uid,
                    "signal": body,
                    "status": "relayed"
                }),
            )
        }
        ("GET", "/api/v1/webrtc/turn_credentials") => (
            200,
            json!({
                "urls": ["stun:stun.l.google.com:19302"],
                "username": "ephemeral_fabushi_user",
                "credential": "ephemeral_fabushi_credential",
                "ttl_seconds": 43_200
            }),
        ),
        ("GET", "/api/v1/tasks") => (
            200,
            json!({
                "tasks": [],
                "columns": ["inbox", "active", "review", "completed", "failed", "cancelled"],
                "total": 0
            }),
        ),
        ("POST", "/api/v1/tasks") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            let task_id = body
                .get("task_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("task_{seq}"));
            let task_state = body
                .get("initial_state")
                .and_then(Value::as_str)
                .unwrap_or("inbox");
            (
                200,
                json!({
                    "task_id": task_id,
                    "state": task_state,
                    "created": true
                }),
            )
        }
        ("POST", p) if p.starts_with("/api/v1/tasks/") && p.ends_with("/approve") => {
            let task_id = p
                .trim_start_matches("/api/v1/tasks/")
                .trim_end_matches("/approve");
            let approved = body
                .get("approved")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let decision = body
                .get("decision")
                .and_then(Value::as_str)
                .unwrap_or(if approved { "approved" } else { "rejected" });
            (
                200,
                json!({
                    "task_id": task_id,
                    "approved": approved,
                    "decision": decision,
                    "current_state": if approved { "active" } else { "cancelled" },
                    "transition_success": true,
                    "gate_id": format!("gate_{task_id}")
                }),
            )
        }
        ("GET", "/api/v1/mcp/servers") => (
            200,
            json!({"servers": [], "registered_tools_count": 0}),
        ),
        ("POST", "/api/v1/mcp/servers") => {
            let server_id = body
                .get("server_id")
                .and_then(Value::as_str)
                .unwrap_or("mcp_srv_1");
            (200, json!({"status": "registered", "server_id": server_id}))
        }
        ("GET", "/api/v1/mcp/tools") => (200, json!({"tools": [], "count": 0})),
        ("POST", "/api/v1/mcp/call") => {
            let Some(tool_name) = body
                .get("tool_name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                return (400, json!({"error": "missing_tool_name"}));
            };
            (
                200,
                json!({
                    "status": "executed",
                    "tool_name": tool_name,
                    "result": format!("executed:{tool_name}")
                }),
            )
        }
        ("POST", "/api/v1/mcp/permission/approve") => {
            let Some(request_id) = body
                .get("request_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                return (400, json!({"error": "missing_request_id"}));
            };
            let allow = body.get("allow").and_then(Value::as_bool).unwrap_or(true);
            (
                200,
                json!({
                    "status": if allow { "approved" } else { "rejected" },
                    "request_id": request_id
                }),
            )
        }
        ("POST", "/api/v1/talk/session") => {
            let mut g = state.lock().expect("state lock");
            let seq = g.alloc_seq();
            (
                200,
                json!({
                    "session_id": format!("talk_{seq}"),
                    "state": "listening",
                    "generation": seq,
                    "signal_seq": seq
                }),
            )
        }
        ("POST", "/api/v1/talk/barge_in") => (
            200,
            json!({
                "status": "barge_in_handled",
                "new_state": "listening",
                "flushed_audio_chunks": 0,
                "buffer_remaining": 0
            }),
        ),
        ("POST", "/api/v1/talk/audio") => (
            200,
            json!({"status": "queued", "buffered_chunks": 1}),
        ),
        ("GET", "/api/v1/magpie/v1") | ("GET", "/api/v1/magpie/v1/models") => (
            200,
            json!({
                "object": "list",
                "data": [
                    {"id": "gpt-4.1-mini", "object": "model", "owned_by": "magpie"}
                ]
            }),
        ),

        // Updates check
        ("GET", "/api/v1/updates/check") => (
            200,
            json!({
                "latest_version": "1.6.7",
                "store_url": "https://apps.apple.com/app/fabushi",
                "binary_self_update_allowed": false
            }),
        ),

        // Explicit segment-bounded suite routes for the 37 canonical namespaces
        // (explicitly EXCLUDING /api/v1/auth, /api/v1/user-vault, /api/v1/vault, /api/v1/sync, /api/v1/media!)
        (_, p)
            if ALLOWED_FALLBACK_NAMESPACES
                .iter()
                .any(|ns| matches_segment_prefix(p, ns)) =>
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
