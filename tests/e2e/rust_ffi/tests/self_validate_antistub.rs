//! Self-validation & anti-stub negative-oracle test suite executed in CI `self-validate` mode.
//!
//! Zero `unsafe` code, zero process-env mutation UB, zero skipped assertions. Proves:
//! 1. Ephemeral real-socket HTTP/1.1 (multi-packet + RFC 9112 chunked + proxy forms) + HTTP 206/416 +
//!    Instant View HTML + canonical Passkey/Computer/Vault/Alipay/Phone/Logout/Serde routes + fail-closed 404 +
//!    Split-Key V2 login & atomic password rotation + chat `client_nonce` idempotency + strict RFC 6455 WebSocket
//!    (revocation `401` before `101`, unmasked `1002`, invalid UTF-8 `1007`, fragmented reassembly, interleaved Ping/Pong,
//!    16-bit extended length split across timeout).
//! 2. All pure-Rust domain & boundary oracles (RFC 6238 TOTP, WCAG >138 luma, 5-bit 63B waveform, 24 MiB vault cap, etc.) are exact.
//! 3. `FabushiE2EHarness::connect_production()` fails closed when `FABUSHI_HOST_LIB` is unset/empty/missing,
//!    points to `mutant_stub`, points to a symlinked `mutant_stub`, or points to a copied/renamed `mutant_stub`
//!    (caught via `fabushi_e2e_is_mutant_stub` sentinel symbol), AND verifies Layer 3 startup behavioral probe.
//! 4. Every anti-stub oracle catches all 8 deliberate defects injected by `mutant_stub`.
//! 5. Against the ACTUAL compiled baseline `8128951` `libmahayana_app_host.so` (required by default; skipped ONLY when
//!    BOTH `FABUSHI_ALLOW_SKIP_BASELINE_PROOF=1` and `FABUSHI_REQUIRE_BASELINE_PROOF=0`), proves all 6 `fabushi_host_*`
//!    symbols are missing and legacy defects (`acknowledged`, `sample_chats`, missing `retryable`/`id`/`generation`) are caught.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use fabushi_e2e_ffi::contracts::{
    CANONICAL_26_SETTINGS_SECTIONS, CANONICAL_29_TOOLSETS, DEFAULT_SERVER_URL_V167, ImeEvent,
    ImeState, MAX_VAULT_CIPHERTEXT_BYTES, TotpAlgorithm, TranscriptBubbleInput,
    assert_no_sample_chats, assert_not_acknowledged_stub, assert_not_hardcoded_localhost,
    compute_wcag_accent_fill, encode_waveform_5bit, extract_context_reference_badges,
    normalize_phone, validate_envelope, validate_error_envelope, verify_26_settings_sections,
    verify_29_toolsets, verify_argon2id_params, verify_bubble_grouping_900s,
    verify_config_4tier_cascade, verify_controlled_banner_hex,
    verify_honest_capability_disposition, verify_ime_commit_fence, verify_loop_budget_5_hops,
    verify_magpie_circuit_rest_ms, verify_mcp_audit_redaction, verify_otp_code,
    verify_outbox_backoff_bounds, verify_pts_monotonic_and_snapshot_fence, verify_rfc6238_totp,
    verify_slash_command_catalog, verify_sliding_replay_window_128, verify_talk_vad_bargein,
    verify_utf16_entity_bounds, verify_vault_size_cap, verify_voice_gesture,
    verify_waveform_5bit_63bytes, verify_wcag_accent_luma_guard,
};
use fabushi_e2e_ffi::ffi_client::{
    FfiHostClient, LegacyHost8128951Probe, REQUIRED_HOST_SYMBOLS, is_mutant_stub_path,
    probe_required_symbols,
};
use fabushi_e2e_ffi::loopback_server::{LoopbackServer, compute_websocket_accept};
use fabushi_e2e_ffi::{
    ENV_FABUSHI_ALLOW_SKIP_BASELINE_PROOF, ENV_FABUSHI_BASELINE_HOST_LIB, ENV_FABUSHI_HOST_LIB,
    ENV_FABUSHI_MUTANT_STUB_LIB, ENV_FABUSHI_REQUIRE_BASELINE_PROOF, FabushiE2EHarness,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

fn raw_http_request(addr: std::net::SocketAddr, request: &str) -> (u16, String, Vec<u8>) {
    raw_http_bytes(addr, request.as_bytes())
}

fn raw_http_bytes(addr: std::net::SocketAddr, request: &[u8]) -> (u16, String, Vec<u8>) {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .expect("connect to loopback server");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set read timeout");
    stream.write_all(request).expect("write HTTP request");
    stream.flush().expect("flush HTTP request");

    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("HTTP response header boundary");
    let head = String::from_utf8_lossy(&response[..header_end]).into_owned();
    let body = response[header_end + 4..].to_vec();
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("parse HTTP status");
    (status, head, body)
}

fn raw_json_request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<&Value>,
) -> (u16, String, Value) {
    let body_str = body.map(|v| v.to_string()).unwrap_or_default();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\n");
    if let Some(tok) = bearer {
        req.push_str(&format!("Authorization: Bearer {tok}\r\n"));
    }
    if body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", body_str.len()));
    }
    req.push_str("Connection: close\r\n\r\n");
    req.push_str(&body_str);
    let (status, head, raw_body) = raw_http_request(addr, &req);
    let parsed: Value = serde_json::from_slice(&raw_body).unwrap_or_else(|e| {
        panic!(
            "expected valid JSON from {method} {path} (status={status}), err={e}, raw={}",
            String::from_utf8_lossy(&raw_body)
        )
    });
    (status, head, parsed)
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn open_ws_client(addr: std::net::SocketAddr) -> TcpStream {
    open_ws_client_path(addr, "/api/v1/ws")
}

fn open_ws_client_path(addr: std::net::SocketAddr, path_and_query: &str) -> TcpStream {
    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("connect WS TCP");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set WS read timeout");
    let upgrade_req = format!(
        "GET {path_and_query} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream
        .write_all(upgrade_req.as_bytes())
        .expect("write WS upgrade");
    stream.flush().expect("flush WS upgrade");

    let mut head_buf = Vec::new();
    let mut byte = [0u8; 1];
    while !head_buf.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).expect("read WS handshake byte");
        head_buf.push(byte[0]);
    }
    let handshake_str = String::from_utf8_lossy(&head_buf);
    assert!(
        handshake_str.starts_with("HTTP/1.1 101 Switching Protocols"),
        "expected 101 upgrade for {path_and_query}, got: {handshake_str}"
    );
    assert!(handshake_str.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
    stream
}

fn encode_masked_ws_frame(fin: bool, opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + payload.len());
    out.push((if fin { 0x80 } else { 0x00 }) | (opcode & 0x0F));
    if payload.len() <= 125 {
        out.push(0x80 | (payload.len() as u8));
    } else if payload.len() <= 65_535 {
        out.push(0x80 | 126);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    for (i, &b) in payload.iter().enumerate() {
        out.push(b ^ mask[i % 4]);
    }
    out
}

fn read_server_ws_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut hdr = [0u8; 2];
    stream.read_exact(&mut hdr).expect("read server WS hdr");
    assert_eq!(hdr[1] & 0x80, 0, "RFC 6455 §5.1 server frames must be unmasked");
    let opcode = hdr[0] & 0x0F;
    let len7 = hdr[1] & 0x7F;
    let len: usize = if len7 <= 125 {
        usize::from(len7)
    } else if len7 == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext).expect("read 16-bit WS len");
        usize::from(u16::from_be_bytes(ext))
    } else {
        let mut ext = [0u8; 8];
        stream.read_exact(&mut ext).expect("read 64-bit WS len");
        u64::from_be_bytes(ext) as usize
    };
    let mut payload = vec![0u8; len];
    stream
        .read_exact(&mut payload)
        .expect("read server WS payload");
    (opcode, payload)
}

fn require_mutant_stub_path() -> PathBuf {
    let raw = std::env::var(ENV_FABUSHI_MUTANT_STUB_LIB).unwrap_or_else(|_| {
        panic!(
            "FAIL-CLOSED: {ENV_FABUSHI_MUTANT_STUB_LIB} must be set to run mutant_stub negative-oracle tests"
        )
    });
    let path = PathBuf::from(raw.trim());
    assert!(
        path.exists(),
        "FAIL-CLOSED: mutant_stub cdylib does not exist at {}",
        path.display()
    );
    path
}

fn load_mutant_mode(mode: &str) -> FfiHostClient {
    let stub_path = require_mutant_stub_path();
    let cfg = json!({"mutant_mode": mode}).to_string();
    FfiHostClient::load_for_antistub_oracle(&stub_path, &cfg)
        .unwrap_or_else(|e| panic!("failed to load mutant_stub in mode `{mode}`: {e}"))
}

#[test]
fn self_validate_loopback_http_206_416_and_fault_injection() {
    let server = LoopbackServer::start().expect("start ephemeral loopback server");
    let addr = server.addr();

    // 1. Health & POST register
    let (status, _head, body) = raw_http_request(
        addr,
        &format!("GET /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(status, 200);
    let health: Value = serde_json::from_slice(&body).expect("valid health JSON");
    assert_eq!(health["ok"], true);
    assert_eq!(health["version"], "1.6.7");

    // 2. Seed 4096-byte blob and verify HTTP 206 Partial Content + HTTP 416 Range Not Satisfiable
    let blob: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
    server.seed_media_blob("blob_alpha", blob.clone());

    let (st_206, head_206, slice_206) = raw_http_request(
        addr,
        &format!(
            "GET /api/v1/media/blob/blob_alpha HTTP/1.1\r\nHost: {addr}\r\nRange: bytes=0-1023\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(st_206, 206);
    assert!(
        head_206.contains("Content-Range: bytes 0-1023/4096"),
        "missing Content-Range header: {head_206}"
    );
    assert_eq!(slice_206.as_slice(), &blob[0..=1023]);

    let (st_416, head_416, _body_416) = raw_http_request(
        addr,
        &format!(
            "GET /api/v1/media/blob/blob_alpha HTTP/1.1\r\nHost: {addr}\r\nRange: bytes=9999-10050\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(st_416, 416);
    assert!(head_416.contains("Content-Range: bytes */4096"));

    // 3. Deterministic fault injection: 1x 429 + 1x 503 then recovery to 200
    server.inject_http_status(
        "/api/v1/auth/login",
        429,
        json!({"error": "rate_limited"}),
        Some(5000),
        Some(1),
    );
    server.inject_http_status(
        "/api/v1/auth/login",
        503,
        json!({"error": "upstream_unavailable"}),
        None,
        Some(1),
    );

    let login_payload = json!({
        "username": "fault_probe",
        "auth_verifier_b64": B64.encode([0x31u8; 32])
    })
    .to_string();
    let post_req = format!(
        "POST /api/v1/auth/login HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{login_payload}",
        login_payload.len()
    );
    let (st1, h1, _) = raw_http_request(addr, &post_req);
    assert_eq!(st1, 429);
    assert!(h1.contains("Retry-After: 5"));

    let (st2, _, _) = raw_http_request(addr, &post_req);
    assert_eq!(st2, 503);

    let (st3, _, b3) = raw_http_request(addr, &post_req);
    assert_eq!(st3, 200);
    let login_resp: Value = serde_json::from_slice(&b3).expect("valid login JSON");
    assert!(login_resp["token"].as_str().is_some());

    assert_eq!(server.request_count_for_path("/api/v1/auth/login"), 3);
    assert!(server.recorded_requests().len() >= 6);
}

#[test]
fn self_validate_loopback_rfc6455_websocket_handshake_and_frames() {
    // Verify canonical RFC 6455 §1.3 SHA-1 WebSocket accept vector.
    assert_eq!(
        compute_websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );

    let server = LoopbackServer::start().expect("start loopback server");
    server.queue_ws_push_frame(r#"{"kind":"pts_push","pts":7}"#);

    let mut stream = open_ws_client(server.addr());

    // Read queued server push frame
    let (push_op, push_payload) = read_server_ws_frame(&mut stream);
    assert_eq!(push_op, 0x1);
    assert_eq!(
        String::from_utf8_lossy(&push_payload),
        r#"{"kind":"pts_push","pts":7}"#
    );

    // Send masked client text frame and receive server ws_ack
    let client_frame =
        encode_masked_ws_frame(true, 0x1, b"hello_fabushi_ws", [0x37, 0xFA, 0x21, 0x3D]);
    stream.write_all(&client_frame).expect("send masked WS text");
    stream.flush().expect("flush masked WS text");

    let (ack_op, ack_payload) = read_server_ws_frame(&mut stream);
    assert_eq!(ack_op, 0x1);
    let ack_json: Value = serde_json::from_slice(&ack_payload).expect("valid ack JSON");
    assert_eq!(ack_json["type"], "ws_ack");
    assert_eq!(ack_json["echo"], "hello_fabushi_ws");

    // Send masked Close frame cleanly
    let close_frame = encode_masked_ws_frame(true, 0x8, &[], [0x01, 0x02, 0x03, 0x04]);
    let _ = stream.write_all(&close_frame);
}

#[test]
fn self_validate_loopback_chunked_multipacket_media_upload_206_and_stateful_routes() {
    let server = LoopbackServer::start().expect("start loopback server");
    let addr = server.addr();

    // 1. Chunked HTTP media upload -> exact SHA-256 -> HTTP 206 / 416 read-back
    let payload = b"Fabushi-Chunked-Payload-0123456789ABCDEF";
    assert_eq!(payload.len(), 40);
    let expected_sha = sha256_hex_bytes(payload);
    let chunked_req = format!(
        "POST /api/v1/media/upload HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
         a;ext=demo\r\nFabushi-Ch\r\n\
         e\r\nunked-Payload-\r\n\
         10\r\n0123456789ABCDEF\r\n\
         0\r\nX-Trailer: verified\r\n\r\n"
    );
    let (up_status, _, up_body) = raw_http_request(addr, &chunked_req);
    assert_eq!(up_status, 200);
    let up_json: Value = serde_json::from_slice(&up_body).expect("upload JSON");
    assert_eq!(up_json["sha256"].as_str(), Some(expected_sha.as_str()));
    assert_ne!(
        up_json["sha256"].as_str(),
        Some(sha256_hex_bytes(b"{}").as_str()),
        "media/upload must hash real uploaded bytes, never empty JSON"
    );
    let media_id = up_json["media_id"].as_str().expect("media_id");
    assert!(!media_id.is_empty());

    let (st_206, head_206, slice_206) = raw_http_request(
        addr,
        &format!(
            "GET /api/v1/media/{media_id} HTTP/1.1\r\nHost: {addr}\r\nRange: bytes=0-15\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(st_206, 206);
    assert!(head_206.contains("Content-Range: bytes 0-15/40"));
    assert_eq!(slice_206.as_slice(), &payload[0..=15]);

    let (st_416, head_416, _) = raw_http_request(
        addr,
        &format!(
            "GET /api/v1/media/{media_id} HTTP/1.1\r\nHost: {addr}\r\nRange: bytes=100-120\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(st_416, 416);
    assert!(head_416.contains("Content-Range: bytes */40"));

    // 2. Multi-packet Content-Length HTTP body split across TCP writes
    let reg_body = json!({
        "username": "multipacket_alice",
        "auth_version": 2,
        "auth_salt_b64": B64.encode([0x11u8; 16]),
        "auth_verifier_b64": B64.encode([0x22u8; 32]),
        "device_name": "Primary iPhone"
    })
    .to_string();
    let reg_head = format!(
        "POST /api/v1/auth/register HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reg_body.len()
    );
    let mut split_stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("connect split TCP");
    split_stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("timeout");
    split_stream.write_all(reg_head.as_bytes()).expect("head");
    split_stream
        .write_all(&reg_body.as_bytes()[..20])
        .expect("part 1");
    split_stream.flush().expect("flush 1");
    std::thread::sleep(Duration::from_millis(25));
    split_stream
        .write_all(&reg_body.as_bytes()[20..])
        .expect("part 2");
    split_stream.flush().expect("flush 2");

    let mut split_resp = Vec::new();
    let _ = split_stream.read_to_end(&mut split_resp);
    let split_hdr_end = split_resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("split resp header");
    let split_head = String::from_utf8_lossy(&split_resp[..split_hdr_end]);
    assert!(
        split_head.starts_with("HTTP/1.1 200 OK"),
        "expected 200 OK on multi-packet register, got: {split_head}"
    );
    let split_json: Value =
        serde_json::from_slice(&split_resp[split_hdr_end + 4..]).expect("split resp JSON");
    let owner_tok = split_json["token"].as_str().expect("owner token").to_owned();
    assert!(!split_json["session_id"].as_str().expect("sid").is_empty());
    assert!(
        server
            .recorded_requests()
            .iter()
            .any(|r| r.path == "/api/v1/auth/register" && r.body == reg_body.as_bytes()),
        "multi-packet Content-Length body must be recorded intact"
    );

    // 3. Fail-closed 404 on unknown route
    let (st_404, _, body_404) = raw_http_request(
        addr,
        &format!("GET /api/v1/__unknown__ HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(st_404, 404);
    let err_404: Value = serde_json::from_slice(&body_404).expect("404 JSON");
    assert_eq!(err_404["error"], "not_found");

    // 4. Stateful shared tables: Cloud 2FA toggle -> login requires_2fa
    let tfa_body = json!({"new_password": "Cloud2FAPassword!"}).to_string();
    let (st_tfa, _, _) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/auth/2fa HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{tfa_body}",
            tfa_body.len()
        ),
    );
    assert_eq!(st_tfa, 200);

    let login_body = json!({
        "username": "multipacket_alice",
        "auth_verifier_b64": B64.encode([0x22u8; 32]),
        "device_name": "Sibling iPad"
    })
    .to_string();
    let (st_login, _, body_login) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/auth/login HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{login_body}",
            login_body.len()
        ),
    );
    assert_eq!(st_login, 200);
    let login_json: Value = serde_json::from_slice(&body_login).expect("login 2fa JSON");
    assert_eq!(login_json["requires_2fa"], true);
    let chal = login_json["challenge_token"]
        .as_str()
        .expect("challenge_token")
        .to_owned();

    // Complete 2FA verify to mint sibling session, then revoke sibling -> 401!
    let v_body =
        json!({"challenge_token": chal, "cloud_password": "Cloud2FAPassword!", "device_name": "Sibling iPad"})
            .to_string();
    let (st_v, _, body_v) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/auth/2fa/verify HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{v_body}",
            v_body.len()
        ),
    );
    assert_eq!(st_v, 200);
    let v_json: Value = serde_json::from_slice(&body_v).expect("2fa verify JSON");
    let sib_sid = v_json["session_id"].as_str().expect("sib sid").to_owned();
    let sib_tok = v_json["token"].as_str().expect("sib tok").to_owned();

    let (st_rev, _, _) = raw_http_request(
        addr,
        &format!(
            "DELETE /api/v1/auth/sessions/{sib_sid} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(st_rev, 200);
    let (st_401, _, _) = raw_http_request(
        addr,
        &format!(
            "GET /api/v1/chats HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {sib_tok}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(st_401, 401, "revoked sibling session token MUST return HTTP 401");

    // 5. QR pair init -> poll (pending) -> accept -> poll (confirmed)
    let pair_init_body = json!({"device_name": "iPad Mini"}).to_string();
    let (st_pi, _, b_pi) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/auth/pair/init HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{pair_init_body}",
            pair_init_body.len()
        ),
    );
    assert_eq!(st_pi, 200);
    let pt = serde_json::from_slice::<Value>(&b_pi).expect("pair init")["pair_token"]
        .as_str()
        .expect("pair_token")
        .to_owned();
    let (_, _, b_pp1) = raw_http_request(
        addr,
        &format!("GET /api/v1/auth/pair/poll/{pt} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&b_pp1).expect("poll 1")["status"],
        "pending"
    );
    let pa_body = json!({"pair_token": pt}).to_string();
    let (st_pa, _, _) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/auth/pair/accept HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{pa_body}",
            pa_body.len()
        ),
    );
    assert_eq!(st_pa, 200);
    let (_, _, b_pp2) = raw_http_request(
        addr,
        &format!("GET /api/v1/auth/pair/poll/{pt} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&b_pp2).expect("poll 2")["status"],
        "confirmed"
    );

    // 6. Per-chat PTS journal & sync/get_difference + snapshot_required
    let ch_body = json!({"title": "PTS Room", "chat_type": 1}).to_string();
    let (st_ch, _, b_ch) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/chats HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ch_body}",
            ch_body.len()
        ),
    );
    assert_eq!(st_ch, 200);
    let cid = serde_json::from_slice::<Value>(&b_ch).expect("chat")["chat_id"]
        .as_i64()
        .expect("chat_id");
    for idx in 1..=2u64 {
        let mb = json!({"text": format!("msg_{idx}"), "client_nonce": 900 + idx}).to_string();
        let (st_m, _, _) = raw_http_request(
            addr,
            &format!(
                "POST /api/v1/chats/{cid}/messages HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{mb}",
                mb.len()
            ),
        );
        assert_eq!(st_m, 200);
    }
    let diff_body = json!({"chat_id": cid, "pts": 0, "limit": 10}).to_string();
    let (st_d, _, b_d) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/sync/get_difference HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{diff_body}",
            diff_body.len()
        ),
    );
    assert_eq!(st_d, 200);
    let diff_json: Value = serde_json::from_slice(&b_d).expect("diff JSON");
    assert_eq!(diff_json["snapshot_required"], false);
    let updates = diff_json["updates"].as_array().expect("updates");
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0]["pts"], 1);
    assert_eq!(updates[1]["pts"], 2);

    server.set_journal_floor(cid, 50);
    let (_, _, b_d2) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/sync/get_difference HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{diff_body}",
            diff_body.len()
        ),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&b_d2).expect("diff2")["snapshot_required"],
        true
    );

    // 7. X25519 device-sync request -> pending -> approve -> status
    let ds_body = json!({
        "device_name": "iPad Pro",
        "ephemeral_x25519_pub_b64": B64.encode([0x55u8; 32])
    })
    .to_string();
    let (st_ds, _, b_ds) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/user-vault/device-sync/request HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ds_body}",
            ds_body.len()
        ),
    );
    assert_eq!(st_ds, 200);
    let req_id = serde_json::from_slice::<Value>(&b_ds).expect("ds")["request_id"]
        .as_str()
        .expect("request_id")
        .to_owned();
    let (_, _, b_pend) = raw_http_request(
        addr,
        &format!(
            "GET /api/v1/user-vault/device-sync/pending HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(
        serde_json::from_slice::<Value>(&b_pend).expect("pending")["requests"]
            .as_array()
            .expect("requests")
            .iter()
            .any(|r| r["request_id"].as_str() == Some(req_id.as_str()))
    );
    let ap_body = json!({"request_id": req_id, "wrapped_key_b64": B64.encode([0x66u8; 48])}).to_string();
    let (st_ap, _, _) = raw_http_request(
        addr,
        &format!(
            "POST /api/v1/user-vault/device-sync/approve HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ap_body}",
            ap_body.len()
        ),
    );
    assert_eq!(st_ap, 200);
    let (_, _, b_st) = raw_http_request(
        addr,
        &format!(
            "GET /api/v1/user-vault/device-sync/status/{req_id} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&b_st).expect("status")["status"],
        "approved"
    );
}

#[test]
fn self_validate_loopback_ws_unmasked_1002_fragmented_extended_and_split_read() {
    let server = LoopbackServer::start().expect("start loopback server");
    let addr = server.addr();

    // Sub-test A: Unmasked client frame MUST be rejected with RFC 6455 Close code 1002
    {
        let mut stream = open_ws_client(addr);
        let unmasked = [0x81u8, 0x05, b'h', b'e', b'l', b'l', b'o'];
        stream.write_all(&unmasked).expect("write unmasked frame");
        stream.flush().expect("flush unmasked frame");
        let (op, payload) = read_server_ws_frame(&mut stream);
        assert_eq!(op, 0x8, "unmasked client frame must trigger Close opcode 0x8");
        assert!(payload.len() >= 2, "Close frame must contain 2-byte status code");
        assert_eq!(
            u16::from_be_bytes([payload[0], payload[1]]),
            1002,
            "unmasked client frame must return RFC 6455 protocol error 1002"
        );
    }

    // Sub-test B: Fragmented message (FIN=0) + interleaved Ping (0x9) -> Pong (0xA) + Continuation (0x0 FIN=1)
    {
        let mut stream = open_ws_client(addr);
        let f1 = encode_masked_ws_frame(false, 0x1, b"hello_", [0x11, 0x22, 0x33, 0x44]);
        let ping = encode_masked_ws_frame(true, 0x9, b"p1", [0x55, 0x66, 0x77, 0x88]);
        stream.write_all(&f1).expect("write frag 1");
        stream.write_all(&ping).expect("write interleaved ping");
        stream.flush().expect("flush frag1+ping");

        let (pong_op, pong_payload) = read_server_ws_frame(&mut stream);
        assert_eq!(pong_op, 0xA, "interleaved Ping must receive Pong 0xA");
        assert_eq!(pong_payload.as_slice(), b"p1");

        let f2 = encode_masked_ws_frame(true, 0x0, b"reassembled", [0x99, 0xAA, 0xBB, 0xCC]);
        stream.write_all(&f2).expect("write frag 2");
        stream.flush().expect("flush frag 2");

        let (ack_op, ack_payload) = read_server_ws_frame(&mut stream);
        assert_eq!(ack_op, 0x1);
        let ack_json: Value = serde_json::from_slice(&ack_payload).expect("ack JSON");
        assert_eq!(ack_json["type"], "ws_ack");
        assert_eq!(ack_json["echo"], "hello_reassembled");

        let close = encode_masked_ws_frame(true, 0x8, &[], [0x01, 0x02, 0x03, 0x04]);
        let _ = stream.write_all(&close);
    }

    // Sub-test C: Extended 16-bit length (126) + 1-byte split across 50ms poll timeout (zero desync!)
    {
        let mut stream = open_ws_client(addr);
        let big_text: String = "F".repeat(300);
        let big_frame =
            encode_masked_ws_frame(true, 0x1, big_text.as_bytes(), [0xA1, 0xB2, 0xC3, 0xD4]);
        // Write ONLY byte 0, pause > WS_POLL_TIMEOUT (50ms), then write rest of frame.
        stream.write_all(&big_frame[..1]).expect("write 1st byte");
        stream.flush().expect("flush 1st byte");
        std::thread::sleep(Duration::from_millis(65));
        stream.write_all(&big_frame[1..]).expect("write rest of frame");
        stream.flush().expect("flush rest of frame");

        let (ack_op, ack_payload) = read_server_ws_frame(&mut stream);
        assert_eq!(ack_op, 0x1);
        let ack_json: Value = serde_json::from_slice(&ack_payload).expect("big ack JSON");
        assert_eq!(ack_json["type"], "ws_ack");
        assert_eq!(ack_json["echo"].as_str(), Some(big_text.as_str()));

        let close = encode_masked_ws_frame(true, 0x8, &[], [0x09, 0x08, 0x07, 0x06]);
        let _ = stream.write_all(&close);
    }
}

#[test]
fn self_validate_domain_and_boundary_oracles() {
    // 1. RFC 6238 TOTP exact vectors (SHA1 T=59 -> 287082 / 94287082; SHA256 T=59 -> 46119246)
    verify_rfc6238_totp(
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
        59,
        30,
        6,
        TotpAlgorithm::Sha1,
        "287082",
    )
    .expect("RFC 6238 SHA1 6-digit vector");
    verify_rfc6238_totp(
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
        59,
        30,
        8,
        TotpAlgorithm::Sha1,
        "94287082",
    )
    .expect("RFC 6238 SHA1 8-digit vector");
    verify_rfc6238_totp(
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA",
        59,
        30,
        8,
        TotpAlgorithm::Sha256,
        "46119246",
    )
    .expect("RFC 6238 SHA256 8-digit vector");

    // 2. WCAG >138 luma accent guard
    let (hex_blue, _, adj_blue) = compute_wcag_accent_fill("#0A84FF").expect("blue");
    assert_eq!(hex_blue, "#0A84FF");
    assert!(!adj_blue);
    let (hex_138, _, adj_138) = compute_wcag_accent_fill("#8A8A8A").expect("luma 138");
    assert_eq!(hex_138, "#8A8A8A");
    assert!(!adj_138);
    let (_, _, adj_139) = compute_wcag_accent_fill("#8B8B8B").expect("luma 139");
    assert!(adj_139);
    verify_wcag_accent_luma_guard("#FFFF00", "#A9A906", true).expect("yellow max darken");
    verify_wcag_accent_luma_guard("#FFFFFF", "#A9A9A9", true).expect("white max darken");

    // 3. 5-bit 100-bar 63-byte waveform
    let samples: Vec<u8> = (0..100).map(|i| (i % 32) as u8).collect();
    let encoded = encode_waveform_5bit(&samples);
    assert_eq!(encoded.len(), 63);
    verify_waveform_5bit_63bytes(&samples, &encoded).expect("5-bit 63B roundtrip");

    // 4. Vault 24 MiB cap & Argon2id bounds
    verify_vault_size_cap("password", "github:main", MAX_VAULT_CIPHERTEXT_BYTES, 128)
        .expect("exact 24 MiB vault boundary");
    assert!(
        verify_vault_size_cap(
            "password",
            "github:main",
            MAX_VAULT_CIPHERTEXT_BYTES + 1,
            128
        )
        .is_err()
    );
    assert!(verify_vault_size_cap("password", "../escape", 16, 12).is_err());
    verify_argon2id_params(65536, 3, 4, 16, 32).expect("default Split-Key V2 Argon2id");
    assert!(verify_argon2id_params(512, 3, 4, 16, 32).is_err());
    assert!(verify_argon2id_params(65536, 3, 4, 15, 32).is_err());

    // 5. OTP, Phone, 900s bubble grouping, UTF-16 entity bounds, IME fence, Voice gesture, @ badges
    verify_otp_code("049281").expect("valid 6-digit OTP");
    assert!(verify_otp_code("12345").is_err());
    assert_eq!(normalize_phone("+1 (415) 555-0199").unwrap(), "+14155550199");
    assert!(normalize_phone("123456").is_err());

    let layout = verify_bubble_grouping_900s(&[
        TranscriptBubbleInput {
            sender_id: 1,
            outgoing: true,
            timestamp_secs: 1000,
            is_service: false,
        },
        TranscriptBubbleInput {
            sender_id: 1,
            outgoing: true,
            timestamp_secs: 1900,
            is_service: false,
        },
        TranscriptBubbleInput {
            sender_id: 1,
            outgoing: true,
            timestamp_secs: 2801,
            is_service: false,
        },
    ]);
    assert!(layout[0].attached_next && !layout[0].has_tail);
    assert!(layout[1].attached_prev && !layout[1].attached_next && layout[1].has_tail);
    assert!(!layout[2].attached_prev && layout[2].inner_radius == 16.0);

    verify_utf16_entity_bounds("你好🚀世界", 2, 2).expect("surrogate pair UTF-16 bounds");
    assert!(verify_utf16_entity_bounds("你好🚀世界", 5, 2).is_err());

    assert_eq!(
        verify_ime_commit_fence(ImeState::Composing, ImeEvent::KeyEnter, 0).action,
        "ime_candidate_chosen"
    );
    assert_eq!(
        verify_ime_commit_fence(ImeState::Committed, ImeEvent::KeyEnter, 20).action,
        "none"
    );
    assert_eq!(
        verify_ime_commit_fence(ImeState::Committed, ImeEvent::KeyEnter, 120).action,
        "send_message"
    );
    assert_eq!(verify_voice_gesture(-220.0, 0.0, 2.0), "slide_canceling");
    assert_eq!(verify_voice_gesture(0.0, -90.0, 2.0), "locked");
    assert_eq!(verify_voice_gesture(0.0, 0.0, 0.49), "discarded");

    let badges = extract_context_reference_badges(
        "Inspect @file:src/lib.rs, @dir:tests/e2e; @diff @staged @git:HEAD~1 (@url:https://example.com)",
    );
    assert_eq!(badges.len(), 6);
    assert!(
        extract_context_reference_badges("email@file.com @filex @dir2 @gitx @url_fake").is_empty()
    );

    // 6. PTS, Outbox backoff, 5-hop loop budget, MCP audit redaction, 4-tier config, replay window,
    //    29 toolsets, 26 settings, Magpie rest ladder, Talk VAD, Controlled Banner, Capability Ledger
    verify_pts_monotonic_and_snapshot_fence(0, 1, &[1, 2, 3], false).expect("monotonic PTS");
    verify_pts_monotonic_and_snapshot_fence(1, 5001, &[], true).expect("snapshot_required fence");
    verify_outbox_backoff_bounds(0, 500).expect("retry 0 backoff");
    verify_outbox_backoff_bounds(6, 30_000).expect("retry 6 cap");
    verify_loop_budget_5_hops(5, 5, false).expect("hop 5 ok");
    verify_loop_budget_5_hops(6, 5, true).expect("hop 6 exhausted");
    verify_mcp_audit_redaction(
        &json!({"text": "SuperSecret123", "password": "pw!"}),
        "{\"text\":\"<14 chars>\",\"password\":\"<3 chars>\"}",
    )
    .expect("clean audit redaction");
    assert!(
        verify_mcp_audit_redaction(
            &json!({"password": "pw_leaked"}),
            "args contained pw_leaked"
        )
        .is_err()
    );
    assert_eq!(
        verify_config_4tier_cascade(None, None, None),
        (DEFAULT_SERVER_URL_V167.to_owned(), "default_fallback")
    );
    assert_eq!(
        verify_config_4tier_cascade(Some("http://1.2.3.4/"), Some("http://env"), None),
        ("http://1.2.3.4".to_owned(), "cli_argument")
    );

    let mut highest = 0u64;
    let mut bitmap = 0u128;
    assert!(!verify_sliding_replay_window_128(&mut highest, &mut bitmap, 0));
    assert!(verify_sliding_replay_window_128(&mut highest, &mut bitmap, 10));
    assert!(verify_sliding_replay_window_128(&mut highest, &mut bitmap, 8));
    assert!(!verify_sliding_replay_window_128(&mut highest, &mut bitmap, 8));
    assert!(verify_sliding_replay_window_128(&mut highest, &mut bitmap, 137));
    assert!(verify_sliding_replay_window_128(&mut highest, &mut bitmap, 11));
    assert!(!verify_sliding_replay_window_128(&mut highest, &mut bitmap, 9));

    let toolsets: Vec<String> = CANONICAL_29_TOOLSETS.iter().map(|s| (*s).to_owned()).collect();
    verify_29_toolsets(&toolsets).expect("29 toolsets");
    let sections: Vec<String> = CANONICAL_26_SETTINGS_SECTIONS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    verify_26_settings_sections(&sections).expect("26 settings");
    verify_controlled_banner_hex("human", "#1E6BFF").expect("human hex");
    verify_controlled_banner_hex("ai_mcp", "#0F766E").expect("ai_mcp hex");
    verify_controlled_banner_hex("mixed", "#4338CA").expect("mixed hex");
    verify_magpie_circuit_rest_ms("rate_limit", 0, 5_000).expect("5s rate rest");
    verify_magpie_circuit_rest_ms("overload", 0, 30_000).expect("30s overload rest");
    verify_magpie_circuit_rest_ms("credit", 3, 14_400_000).expect("4h credit rest");
    assert_eq!(verify_talk_vad_bargein(3, 0, 320).unwrap(), (true, false));
    assert!(verify_talk_vad_bargein(0, 25, 321).is_err());

    let fake_catalog: Vec<Value> = [
        "Session",
        "Safety & Control",
        "Inspection & Export",
        "Model & Provider",
        "Tools & Skills",
        "Memory & Persona",
        "Workflow & Automation",
        "Modes & UI",
    ]
    .iter()
    .enumerate()
    .flat_map(|(ci, cat)| {
        (0..7).map(move |i| json!({"name": format!("cmd_{ci}_{i}"), "category": cat}))
    })
    .collect();
    verify_slash_command_catalog(&fake_catalog).expect("56 >= 49 slash commands");

    verify_honest_capability_disposition(&[
        json!({"id": "OCRM-001", "disposition": "upstream_planned"}),
        json!({"id": "FBCP-001", "disposition": "upstream_planned"}),
        json!({"id": "TON_ONCHAIN_SIGNING", "disposition": "upstream_planned"}),
        json!({"id": "TELEGRAM_PASSPORT", "disposition": "upstream_planned"}),
        json!({"id": "APNS_HTTP2_SERVER_DELIVERY", "disposition": "server_blocked"}),
        json!({"id": "WEBRTC_NAT_SFU_TRANSPORT", "disposition": "server_blocked"}),
        json!({"id": "STOREKIT_SERVER_BINDING", "disposition": "server_blocked"}),
    ])
    .expect("honest capability ledger");
}

#[test]
fn self_validate_connect_production_fails_closed_without_real_host() {
    let stub_path = require_mutant_stub_path();

    // 1. Pure env-value parser tests (ZERO process-env mutation / ZERO unsafe!)
    let err_none = FabushiE2EHarness::try_connect_production_from_path_opt(None)
        .err()
        .expect("None FABUSHI_HOST_LIB must fail closed");
    assert!(err_none.contains(ENV_FABUSHI_HOST_LIB) && err_none.contains("is not set"));

    for empty in ["", "   "] {
        let err_empty = FabushiE2EHarness::try_connect_production_from_path_opt(Some(empty))
            .err()
            .expect("empty FABUSHI_HOST_LIB must fail closed");
        assert!(err_empty.contains(ENV_FABUSHI_HOST_LIB) && err_empty.contains("is empty"));
    }

    if std::env::var(ENV_FABUSHI_HOST_LIB).is_err() {
        let err_env = FabushiE2EHarness::try_connect_production()
            .err()
            .expect("unset FABUSHI_HOST_LIB must fail closed");
        assert!(err_env.contains(ENV_FABUSHI_HOST_LIB));
    }

    // 2. Nonexistent path -> MUST fail closed
    let err_missing = FabushiE2EHarness::try_connect_production_from_path_opt(Some(
        "/nonexistent/fabushi/libmahayana_app_host.so",
    ))
    .err()
    .expect("nonexistent host lib must fail closed");
    assert!(err_missing.contains("does not exist"));

    // 3. Layer 1: Direct mutant_stub path -> MUST be rejected before dlopen
    let err_stub = FabushiE2EHarness::try_connect_production_at(&stub_path)
        .err()
        .expect("connect_production MUST refuse mutant_stub");
    assert!(
        err_stub.contains("refuses mutant_stub"),
        "expected mutant_stub refusal, got: {err_stub}"
    );

    // 4. Layer 2: Copied & renamed mutant_stub -> MUST be rejected via fabushi_e2e_is_mutant_stub sentinel!
    let tmp = TempDir::new().expect("temp dir");
    let copied = tmp
        .path()
        .join(format!("libfake_renamed_host.{}", std::env::consts::DLL_EXTENSION));
    std::fs::copy(&stub_path, &copied).expect("copy mutant_stub to neutral filename");
    assert!(
        !is_mutant_stub_path(&copied),
        "neutral filename must not match path heuristic"
    );
    let err_renamed = FabushiE2EHarness::try_connect_production_at(&copied)
        .err()
        .expect("renamed mutant_stub MUST still be rejected via sentinel symbol");
    assert!(
        err_renamed.contains("Refusing to load negative-oracle mutant_stub cdylib in production mode")
            && err_renamed.contains("fabushi_e2e_is_mutant_stub"),
        "unexpected renamed sentinel error: {err_renamed}"
    );

    // 5. Layer 1 (canonicalize): Symlinked mutant_stub -> MUST be rejected!
    #[cfg(unix)]
    {
        let sym_path = tmp
            .path()
            .join(format!("libneutral_symlink.{}", std::env::consts::DLL_EXTENSION));
        std::os::unix::fs::symlink(&stub_path, &sym_path).expect("create symlink to mutant_stub");
        assert!(
            is_mutant_stub_path(&sym_path),
            "canonicalize must detect symlink target containing mutant_stub"
        );
        let err_sym = FabushiE2EHarness::try_connect_production_at(&sym_path)
            .err()
            .expect("symlinked mutant_stub MUST be rejected");
        assert!(
            err_sym.contains("refuses mutant_stub"),
            "unexpected symlink error: {err_sym}"
        );
    }

    // 6. Layer 3: Startup behavioral anti-stub probe directly catches broken modes
    let err_ack = load_mutant_mode("acknowledged_wildcard")
        .verify_startup_antistub_probe()
        .err()
        .expect("startup probe MUST catch acknowledged_wildcard");
    assert!(err_ack.contains("acknowledged"), "unexpected err_ack: {err_ack}");

    let err_retry = load_mutant_mode("missing_retryable")
        .verify_startup_antistub_probe()
        .err()
        .expect("startup probe MUST catch missing_retryable");
    assert!(err_retry.contains("retryable"), "unexpected err_retry: {err_retry}");

    let err_meta = load_mutant_mode("missing_id_or_generation")
        .verify_startup_antistub_probe()
        .err()
        .expect("startup probe MUST catch missing_id_or_generation");
    assert!(!err_meta.is_empty());
}

#[test]
fn self_validate_mutant_stub_catches_all_eight_defect_modes() {
    // Mode 1: acknowledged_wildcard
    let m1 = load_mutant_mode("acknowledged_wildcard");
    let req1 = json!({"id": "m1", "method": "nonexistent.probe", "generation": 1, "params": {}});
    let raw1 = m1.dispatch_raw(&req1.to_string()).expect("m1 dispatch");
    let err1 = validate_envelope(&raw1, "m1")
        .err()
        .expect("oracle MUST catch fail-open status:acknowledged");
    assert!(err1.contains("acknowledged"), "unexpected err1: {err1}");

    // Mode 2: sample_chats
    let m2 = load_mutant_mode("sample_chats");
    let req2 = json!({"id": "m2", "method": "chats.list", "generation": 1, "params": {}});
    let raw2 = m2.dispatch_raw(&req2.to_string()).expect("m2 dispatch");
    let parsed2 = validate_envelope(&raw2, "m2").expect("envelope valid JSON");
    let err2 = assert_no_sample_chats(parsed2.result.as_ref().unwrap())
        .err()
        .expect("oracle MUST catch synthetic sample_chats");
    assert!(err2.contains("sample_chats"), "unexpected err2: {err2}");

    // Mode 3: missing_retryable
    let m3 = load_mutant_mode("missing_retryable");
    let req3 = json!({"id": "m3", "method": "nonexistent.cmd", "generation": 1, "params": {}});
    let raw3 = m3.dispatch_raw(&req3.to_string()).expect("m3 dispatch");
    let err3 = validate_error_envelope(&raw3, "m3", &["unknown_command"])
        .err()
        .expect("oracle MUST catch missing error.retryable");
    assert!(err3.contains("retryable"), "unexpected err3: {err3}");

    // Mode 4: stale_generation_mutation
    let m4 = load_mutant_mode("stale_generation_mutation");
    let bump = json!({"id": "m4-b", "method": "generation.bump", "generation": 1, "params": {}});
    let raw_b = m4.dispatch_raw(&bump.to_string()).expect("m4 bump");
    let parsed_b = validate_envelope(&raw_b, "m4-b").expect("bump envelope");
    assert_eq!(parsed_b.generation, 2);
    let stale = json!({"id": "m4-s", "method": "messages.send", "generation": 1, "params": {}});
    let raw_s = m4.dispatch_raw(&stale.to_string()).expect("m4 stale");
    let err4 = validate_error_envelope(&raw_s, "m4-s", &["stale_generation"])
        .err()
        .expect("oracle MUST catch stale generation accepted/mutated");
    assert!(err4.contains("expected fail-closed"), "unexpected err4: {err4}");

    // Mode 5: hardcoded_localhost
    let m5 = load_mutant_mode("hardcoded_localhost");
    let req5 = json!({"id": "m5", "method": "settings.config_resolve", "generation": 1, "params": {}});
    let raw5 = m5.dispatch_raw(&req5.to_string()).expect("m5 dispatch");
    let parsed5 = validate_envelope(&raw5, "m5").expect("m5 envelope");
    let url5 = parsed5.result.as_ref().unwrap()["server_url"]
        .as_str()
        .unwrap();
    let err5 = assert_not_hardcoded_localhost(url5)
        .err()
        .expect("oracle MUST catch hardcoded 127.0.0.1:8443");
    assert!(err5.contains("127.0.0.1:8443"), "unexpected err5: {err5}");

    // Mode 6: missing_id_or_generation
    let m6 = load_mutant_mode("missing_id_or_generation");
    let req6 = json!({"id": "m6", "method": "generation.current", "generation": 1, "params": {}});
    let raw6 = m6.dispatch_raw(&req6.to_string()).expect("m6 dispatch");
    let err6 = validate_envelope(&raw6, "m6")
        .err()
        .expect("oracle MUST catch missing id/generation");
    assert!(err6.contains("missing string `id`"), "unexpected err6: {err6}");

    // Mode 7: missing_symbols_probe (poll_events returns NULL, cancel returns false)
    let m7 = load_mutant_mode("missing_symbols_probe");
    assert!(m7.poll_events_raw(8, 10).is_err());
    assert!(!m7.cancel("req-1"));

    // Mode 8: unhandled_panic_marker
    let m8 = load_mutant_mode("unhandled_panic_marker");
    let req8 = json!({"id": "m8", "method": "debug.trigger_panic", "generation": 1, "params": {}});
    let raw8 = m8.dispatch_raw(&req8.to_string()).expect("m8 dispatch");
    let err8 = validate_envelope(&raw8, "m8")
        .err()
        .expect("oracle MUST catch non-JSON panic payload");
    assert!(err8.contains("not valid JSON"), "unexpected err8: {err8}");
}

#[test]
fn self_validate_baseline_8128951_host_negative_proof() {
    let allow_skip = std::env::var(ENV_FABUSHI_ALLOW_SKIP_BASELINE_PROOF)
        .map(|v| v.trim() == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let require_baseline = std::env::var(ENV_FABUSHI_REQUIRE_BASELINE_PROOF)
        .map(|v| v.trim() == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);

    let baseline_path = match std::env::var(ENV_FABUSHI_BASELINE_HOST_LIB) {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
        _ => {
            if allow_skip && !require_baseline {
                eprintln!(
                    "NOTE: baseline 8128951 negative proof explicitly disabled via \
                     {ENV_FABUSHI_ALLOW_SKIP_BASELINE_PROOF}=1 and {ENV_FABUSHI_REQUIRE_BASELINE_PROOF}=0"
                );
                return;
            }
            panic!(
                "FAIL-CLOSED: {ENV_FABUSHI_BASELINE_HOST_LIB} must be set for baseline 8128951 \
                 negative proof (default/CI requires real baseline proof; zero silent skip)"
            );
        }
    };

    assert!(
        baseline_path.exists(),
        "FAIL-CLOSED: baseline 8128951 host library missing at {}",
        baseline_path.display()
    );

    // Proof A: All 6 target `fabushi_host_*` symbols MUST be missing in baseline 8128951!
    let missing_symbols =
        probe_required_symbols(&baseline_path).expect("probe symbols on 8128951 lib");
    assert_eq!(
        missing_symbols.len(),
        REQUIRED_HOST_SYMBOLS.len(),
        "expected ALL 6 fabushi_host_* symbols missing in 8128951, found missing={missing_symbols:?}"
    );
    let prod_err = FabushiE2EHarness::try_connect_production_at(&baseline_path)
        .err()
        .expect("connect_production MUST fail closed against 8128951 baseline");
    assert!(
        prod_err.contains("missing symbol fabushi_host_create"),
        "expected missing fabushi_host_create error, got: {prod_err}"
    );

    // Load actual baseline 8128951 host via its legacy C-ABI to prove every legacy stub is caught.
    let tmp = TempDir::new().expect("temp dir for baseline host");
    let legacy = LegacyHost8128951Probe::load(&baseline_path, tmp.path())
        .expect("load 8128951 legacy host via telegram_app_host_create_test");

    // Proof B: Unknown method returns fail-open `{"status":"acknowledged"}` -> CAUGHT!
    let unknown_req = json!({
        "id": "baseline-unknown-1",
        "method": "nonexistent.probe_method",
        "generation": 1,
        "params": {"probe": true}
    });
    let unknown_raw = legacy
        .dispatch(&unknown_req.to_string())
        .expect("legacy dispatch unknown method");
    let unknown_val: Value =
        serde_json::from_str(&unknown_raw).expect("legacy unknown returns JSON");
    assert_eq!(unknown_val["ok"], true);
    assert_eq!(unknown_val["result"]["status"], "acknowledged");
    assert!(
        assert_not_acknowledged_stub(&unknown_val["result"]).is_err(),
        "oracle MUST catch 8128951 fail-open acknowledged wildcard"
    );
    assert!(
        validate_envelope(&unknown_raw, "baseline-unknown-1").is_err(),
        "validate_envelope MUST reject 8128951 unknown method response"
    );

    // Proof C: `telegram.getChats` returns hardcoded `sample_chats` -> CAUGHT!
    let chats_req = json!({
        "id": "baseline-chats-1",
        "method": "telegram.getChats",
        "generation": 1,
        "params": {}
    });
    let chats_raw = legacy
        .dispatch(&chats_req.to_string())
        .expect("legacy dispatch telegram.getChats");
    let chats_val: Value = serde_json::from_str(&chats_raw).expect("legacy chats JSON");
    let chats_err = assert_no_sample_chats(&chats_val["result"])
        .err()
        .expect("oracle MUST catch 8128951 hardcoded sample_chats");
    assert!(chats_err.contains("sample_chats"));

    // Proof D: Error responses (missing command, malformed JSON, panic) lack `id`, `generation`, and `error.retryable` -> CAUGHT!
    for (label, raw_req) in [
        (
            "missing_command",
            r#"{"id":"err-1","generation":1,"params":{}}"#,
        ),
        ("malformed_json", r#"{not-valid-json"#),
        (
            "debug_panic",
            r#"{"id":"err-3","method":"debug.trigger_panic","generation":1}"#,
        ),
    ] {
        let resp_raw = legacy
            .dispatch(raw_req)
            .unwrap_or_else(|e| panic!("legacy dispatch failed for {label}: {e}"));
        let val: Value = serde_json::from_str(&resp_raw)
            .unwrap_or_else(|e| panic!("legacy error JSON invalid for {label}: {e}"));
        assert_eq!(val["ok"], false, "{label} should return ok=false");
        assert!(val.get("id").is_none(), "{label} lacks id in 8128951");
        assert!(
            val.get("generation").is_none(),
            "{label} lacks generation in 8128951"
        );
        assert!(
            val["error"].get("retryable").is_none(),
            "{label} lacks error.retryable in 8128951"
        );
        assert!(
            validate_error_envelope(&resp_raw, "", &[]).is_err(),
            "validate_error_envelope MUST reject non-compliant 8128951 error for {label}"
        );
    }

    // Ensure Base64 import used for explicit sanity check
    assert_eq!(B64.encode(b"8128951"), "ODEyODk1MQ==");
}

#[test]
fn self_validate_loopback_blocking1_canonical_routes_and_fail_closed_404() {
    let server = LoopbackServer::start().expect("start loopback server");
    let addr = server.addr();

    // 1. Instant View HTML routes: GET /article/v167 and GET /iv/s13 -> 200 text/html;
    //    bogus sibling prefixes /article_evil & /iv_evil -> 404 not_found
    for iv_path in ["/article/v167", "/iv/s13"] {
        let (status, head, body) = raw_http_request(
            addr,
            &format!("GET {iv_path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
        );
        assert_eq!(status, 200, "Instant View {iv_path} must return 200 OK");
        assert!(
            head.to_ascii_lowercase().contains("content-type: text/html"),
            "Instant View {iv_path} must return text/html Content-Type, got: {head}"
        );
        let html = String::from_utf8(body).expect("Instant View body must be valid UTF-8 HTML");
        let html_lower = html.to_ascii_lowercase();
        assert!(
            html_lower.contains("<article") && html_lower.contains("<p"),
            "Instant View {iv_path} must return article HTML markup, got: {html}"
        );
    }
    for bad_iv in ["/article_evil", "/iv_evil"] {
        let (st, _, err_json) = raw_json_request(addr, "GET", bad_iv, None, None);
        assert_eq!(st, 404, "{bad_iv} must fail closed with 404: {err_json}");
        assert_eq!(err_json["error"], "not_found");
    }

    // 1b. HTTP proxy wire forms (CONNECT, HEAD /health, RFC 9112 absolute-form URI) & HttpClient serde compatibility
    let (st_conn, head_conn, _) = raw_http_request(
        addr,
        "CONNECT 127.0.0.1:443 HTTP/1.1\r\nHost: 127.0.0.1:443\r\n\r\n",
    );
    assert_eq!(st_conn, 200, "CONNECT proxy probe must return 200: {head_conn}");

    let (st_head, _, head_body) = raw_http_request(
        addr,
        &format!("HEAD /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(st_head, 200);
    assert!(head_body.is_empty(), "HEAD /health must return empty body");

    let (st_abs, _, abs_body) = raw_http_request(
        addr,
        &format!(
            "GET http://{addr}/api/v1/status HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(st_abs, 200, "absolute-form /api/v1/status must return 200");
    let status_json: Value = serde_json::from_slice(&abs_body).expect("status JSON");
    assert_eq!(status_json["server"], "gpui_telegram_server");
    assert_eq!(status_json["status"], "healthy");
    assert_eq!(status_json["version"], "1.6.7");

    let (st_wk, _, wk_json) = raw_json_request(
        addr,
        "GET",
        "/.well-known/oauth-authorization-server",
        None,
        None,
    );
    assert_eq!(st_wk, 200);
    assert_eq!(wk_json["supported_mcp_versions"][0], "2025-11-25");

    // Seed an authenticated Split-Key V2 user for protected route verification
    let (st_reg, _, reg_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/register",
        None,
        Some(&json!({
            "username": "routes_v167_alice",
            "auth_version": 2,
            "auth_salt_b64": B64.encode([0x1Au8; 16]),
            "auth_verifier_b64": B64.encode([0x2Bu8; 32]),
            "device_name": "iPhone 16 Pro"
        })),
    );
    assert_eq!(st_reg, 200);
    let owner_tok = reg_json["token"].as_str().expect("owner token").to_owned();
    assert!(
        reg_json["expires_at"].as_str().is_some_and(|s| s.ends_with('Z')),
        "register expires_at must be RFC 3339 string: {reg_json}"
    );
    assert!(reg_json["expires_at_ms"].as_i64().is_some());

    // Verify /api/v1/auth/me and /api/v1/agent/runs serde fields + /api/v1/magpie/v1
    let (st_me, _, me_json) =
        raw_json_request(addr, "GET", "/api/v1/auth/me", Some(&owner_tok), None);
    assert_eq!(st_me, 200);
    assert!(!me_json["session_id"].as_str().unwrap_or("").is_empty());

    let (st_run, _, run_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/agent/runs",
        Some(&owner_tok),
        Some(&json!({"prompt": "summarize v1.6.7"})),
    );
    assert_eq!(st_run, 200);
    assert!(!run_json["session_id"].as_str().unwrap_or("").is_empty());
    assert_eq!(run_json["status"], "completed");
    assert_eq!(run_json["prompt"], "summarize v1.6.7");
    assert!(!run_json["response"].as_str().unwrap_or("").is_empty());

    let (st_mag, _, mag_json) =
        raw_json_request(addr, "GET", "/api/v1/magpie/v1", Some(&owner_tok), None);
    assert_eq!(st_mag, 200, "/api/v1/magpie/v1 exact path must return 200: {mag_json}");
    assert!(mag_json["data"].is_array());

    // 2. Canonical Passkey lifecycle: register -> GET /passkeys & /passkey/list -> challenge -> verify -> replay 400
    let (st_pk_reg, _, pk_reg) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/passkey/register",
        Some(&owner_tok),
        Some(&json!({
            "credential_id": "cred_v167_001",
            "public_key": B64.encode([0x44u8; 65]),
            "device_name": "iPhone 16 Pro"
        })),
    );
    assert_eq!(st_pk_reg, 200, "passkey/register must return 200: {pk_reg}");
    assert_eq!(pk_reg["credential_id"], "cred_v167_001");

    for list_path in ["/api/v1/auth/passkeys", "/api/v1/auth/passkey/list"] {
        let (st_list, _, list_json) =
            raw_json_request(addr, "GET", list_path, Some(&owner_tok), None);
        assert_eq!(st_list, 200, "{list_path} must return 200: {list_json}");
        let passkeys = list_json["passkeys"]
            .as_array()
            .expect("passkeys array in list response");
        assert!(
            passkeys.iter().any(|p| p["credential_id"] == "cred_v167_001"),
            "{list_path} must contain registered cred_v167_001: {list_json}"
        );
    }

    let (st_chal, _, chal_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/passkey/challenge",
        None,
        Some(&json!({"username": "routes_v167_alice"})),
    );
    assert_eq!(st_chal, 200, "passkey/challenge must return 200: {chal_json}");
    assert_eq!(chal_json["timeout_ms"], 300_000);
    let challenge_str = chal_json["challenge"]
        .as_str()
        .filter(|s| !s.is_empty())
        .expect("non-empty passkey challenge")
        .to_owned();

    let verify_payload = json!({
        "challenge": challenge_str,
        "credential_id": "cred_v167_001",
        "authenticator_data": B64.encode([0x01u8; 37]),
        "client_data_json": B64.encode(format!(r#"{{"type":"webauthn.get","challenge":"{challenge_str}"}}"#)),
        "signature": B64.encode([0x99u8; 64])
    });
    let (st_ver1, _, ver1_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/passkey/verify",
        None,
        Some(&verify_payload),
    );
    assert_eq!(st_ver1, 200, "first passkey/verify must succeed: {ver1_json}");
    let pk_tok = ver1_json["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .expect("passkey verify must return non-empty Bearer token")
        .to_owned();

    // Verify passkey-minted token works on protected /api/v1/auth/me
    let (st_pk_me, _, _) = raw_json_request(addr, "GET", "/api/v1/auth/me", Some(&pk_tok), None);
    assert_eq!(st_pk_me, 200, "passkey-minted token must access /api/v1/auth/me");

    // Replay of consumed single-use challenge MUST fail closed with HTTP 400 invalid_challenge
    let (st_ver2, _, ver2_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/passkey/verify",
        None,
        Some(&verify_payload),
    );
    assert_eq!(st_ver2, 400, "replayed passkey challenge must return 400: {ver2_json}");
    assert_eq!(ver2_json["error"], "invalid_challenge");

    // 3. Singular /api/v1/computer/{lease,action,lease/emergency_takeover,lease/acquire} -> all 200 JSON;
    //    /api/v1/computer_bogus/lease -> 404 not_found
    let (st_lease, _, lease_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/computer/lease",
        Some(&owner_tok),
        Some(&json!({"device_id": "mac_mini_v167"})),
    );
    assert_eq!(st_lease, 200, "/api/v1/computer/lease must return 200: {lease_json}");
    let lease_gen = lease_json["generation"].as_u64().expect("lease generation");

    let (st_action, _, action_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/computer/action",
        Some(&owner_tok),
        Some(&json!({"device_id": "mac_mini_v167", "generation": lease_gen, "action": "screenshot"})),
    );
    assert_eq!(st_action, 200, "/api/v1/computer/action must return 200: {action_json}");
    assert_eq!(action_json["status"], "action_executed");

    let (st_takeover, _, takeover_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/computer/lease/emergency_takeover",
        Some(&owner_tok),
        Some(&json!({"device_id": "mac_mini_v167"})),
    );
    assert_eq!(
        st_takeover, 200,
        "/api/v1/computer/lease/emergency_takeover must return 200: {takeover_json}"
    );
    assert_eq!(takeover_json["status"], "emergency_takeover_executed");

    let (st_acq, _, acq_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/computer/lease/acquire",
        Some(&owner_tok),
        Some(&json!({"device_id": "mac_mini_v167"})),
    );
    assert_eq!(
        st_acq, 200,
        "/api/v1/computer/lease/acquire must return 200: {acq_json}"
    );
    assert_eq!(acq_json["is_valid"], true);

    let (st_cbogus, _, cbogus_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/computer_bogus/lease",
        Some(&owner_tok),
        Some(&json!({})),
    );
    assert_eq!(st_cbogus, 404, "/api/v1/computer_bogus/lease must return 404: {cbogus_json}");
    assert_eq!(cbogus_json["error"], "not_found");

    // 4. PUT / GET /api/v1/user-vault/items/{cat}/{key} stateful round-trip + query device-sync status + media/phone routes
    let expected_ct = B64.encode(b"encrypted_github_secret_v167");
    let (st_vput, _, vput_json) = raw_json_request(
        addr,
        "PUT",
        "/api/v1/user-vault/items/password/github.com%3Aalice",
        Some(&owner_tok),
        Some(&json!({
            "category": "password",
            "item_key": "github.com:alice",
            "ciphertext_b64": expected_ct,
            "nonce_b64": B64.encode([0xABu8; 12])
        })),
    );
    assert_eq!(st_vput, 200, "PUT user-vault item subpath must return 200: {vput_json}");

    let (st_vget, _, vget_json) = raw_json_request(
        addr,
        "GET",
        "/api/v1/user-vault/items/password/github.com:alice",
        Some(&owner_tok),
        None,
    );
    assert_eq!(st_vget, 200, "GET user-vault item subpath must return 200: {vget_json}");
    let got_ct = vget_json
        .get("ciphertext_b64")
        .or_else(|| vget_json.get("item").and_then(|v| v.get("ciphertext_b64")))
        .and_then(Value::as_str);
    assert_eq!(
        got_ct,
        Some(expected_ct.as_str()),
        "GET /api/v1/user-vault/items/password/github.com:alice must return stored ciphertext: {vget_json}"
    );

    let (st_dsreq, _, dsreq_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/user-vault/device-sync/request",
        Some(&owner_tok),
        Some(&json!({"device_name": "iPad", "ephemeral_x25519_pub_b64": B64.encode([0x55u8; 32])})),
    );
    assert_eq!(st_dsreq, 200);
    let dsync_id = dsreq_json["request_id"].as_str().expect("dsync request_id");
    let (st_dsq, _, dsq_json) = raw_json_request(
        addr,
        "GET",
        &format!("/api/v1/user-vault/device-sync/status?request_id={dsync_id}"),
        Some(&owner_tok),
        None,
    );
    assert_eq!(st_dsq, 200, "query device-sync status must return 200: {dsq_json}");
    assert_eq!(dsq_json["status"], "pending");

    let (st_psend, _, psend_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/phone/send-code",
        None,
        Some(&json!({"phone": "+14155550199"})),
    );
    assert_eq!(st_psend, 200);
    let (st_pver, _, pver_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/phone/verify",
        None,
        Some(&json!({
            "phone": "+14155550199",
            "code": "123456",
            "phone_code_hash": psend_json["phone_code_hash"]
        })),
    );
    assert_eq!(st_pver, 200, "/api/v1/auth/phone/verify must return 200: {pver_json}");

    let (st_uinit, _, uinit_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/media/upload/init",
        Some(&owner_tok),
        Some(&json!({"filename": "clip.mp4", "total_size": 1024, "part_size": 512})),
    );
    assert_eq!(st_uinit, 200);
    let upl_id = uinit_json["upload_id"].as_str().expect("upload_id");
    let (st_ust, _, ust_json) = raw_json_request(
        addr,
        "GET",
        &format!("/api/v1/media/upload/{upl_id}/status"),
        Some(&owner_tok),
        None,
    );
    assert_eq!(st_ust, 200, "media upload /status must return 200: {ust_json}");
    let (st_miv, _, miv_json) = raw_json_request(
        addr,
        "GET",
        "/api/v1/media/instant-view?url=/article/v167",
        Some(&owner_tok),
        None,
    );
    assert_eq!(st_miv, 200, "/api/v1/media/instant-view must return 200: {miv_json}");
    let (st_mnf, _, mnf_json) = raw_json_request(
        addr,
        "GET",
        "/api/v1/media/nonexistent_blob_xyz",
        Some(&owner_tok),
        None,
    );
    assert_eq!(st_mnf, 404);
    assert_eq!(mnf_json["error"], "media_not_found");

    // 5. POST /api/v1/auth/alipay/cancel -> 200 JSON (status == "cancelled") & blocks subsequent poll/exchange
    let (st_as, _, as_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/start",
        None,
        Some(&json!({"client_mode": "ipad_qr"})),
    );
    assert_eq!(st_as, 200);
    let cancel_aid = as_json["attempt_id"].as_str().expect("attempt_id").to_owned();
    let cancel_state = as_json["state"].as_str().expect("state").to_owned();
    let (st_ac, _, ac_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/cancel",
        None,
        Some(&json!({"attempt_id": cancel_aid})),
    );
    assert_eq!(st_ac, 200, "/api/v1/auth/alipay/cancel must return 200: {ac_json}");
    assert_eq!(ac_json["status"], "cancelled");

    let (st_cpoll, _, cpoll_json) = raw_json_request(
        addr,
        "GET",
        &format!("/api/v1/auth/alipay/poll/{cancel_aid}"),
        None,
        None,
    );
    assert_eq!(st_cpoll, 409, "polling cancelled Alipay attempt must fail closed: {cpoll_json}");
    assert_eq!(cpoll_json["error"], "oauth_cancelled");

    let (st_cexch, _, cexch_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/exchange",
        None,
        Some(&json!({"attempt_id": cancel_aid, "state": cancel_state, "auth_code": "code_after_cancel"})),
    );
    assert_eq!(st_cexch, 409, "exchanging cancelled Alipay attempt must fail closed: {cexch_json}");
    assert_eq!(cexch_json["error"], "oauth_cancelled");

    // 6. POST /api/v1/auth/logout -> 200 JSON AND revokes caller Bearer token across HTTP and WS
    let (st_lo, _, lo_json) =
        raw_json_request(addr, "POST", "/api/v1/auth/logout", Some(&pk_tok), Some(&json!({})));
    assert_eq!(st_lo, 200, "/api/v1/auth/logout must return 200: {lo_json}");
    let (st_after_lo, _, _) =
        raw_json_request(addr, "GET", "/api/v1/auth/me", Some(&pk_tok), None);
    assert_eq!(
        st_after_lo, 401,
        "token revoked by /api/v1/auth/logout must return 401 on /api/v1/auth/me"
    );

    // 7. Fail-closed 404 MUST remain intact for unrecognized API routes across all namespaces
    for bad_path in [
        "/api/v1/__unknown__",
        "/api/v1/auth/__unknown__",
        "/api/v1/user-vault/__unknown__",
        "/api/v2/chats",
    ] {
        let (st_404, _, err_json) =
            raw_json_request(addr, "GET", bad_path, Some(&owner_tok), None);
        assert_eq!(st_404, 404, "{bad_path} must STILL return 404");
        assert_eq!(err_json["error"], "not_found");
    }

    let (st_404_post, _, err_post) = raw_json_request(
        addr,
        "POST",
        "/api/v1/__unknown_mutation__",
        Some(&owner_tok),
        Some(&json!({"probe": true})),
    );
    assert_eq!(st_404_post, 404, "unknown POST route must STILL return 404");
    assert_eq!(err_post["error"], "not_found");
}

#[test]
fn self_validate_loopback_blocking2_split_key_v2_login_and_password_rotation() {
    let server = LoopbackServer::start().expect("start loopback server");
    let addr = server.addr();

    let old_salt = B64.encode([0x11u8; 16]);
    let old_verifier = B64.encode([0x22u8; 32]);
    let wrong_verifier = B64.encode([0xEEu8; 32]);
    let new_salt = B64.encode([0x33u8; 16]);
    let new_verifier = B64.encode([0x77u8; 32]);
    let rewrapped_env = json!({
        "key_version": 2,
        "kdf_alg": "split_key_v2",
        "wrapped_dek_b64": B64.encode([0xCCu8; 48]),
        "nonce_b64": B64.encode([0x12u8; 12])
    });

    // 1. Register Split-Key V2 user (NO plaintext password field!)
    let (st_reg, _, reg) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/register",
        None,
        Some(&json!({
            "username": "split_v2_rotator",
            "auth_version": 2,
            "auth_salt_b64": old_salt,
            "auth_verifier_b64": old_verifier,
            "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 },
            "device_name": "Primary iPhone"
        })),
    );
    assert_eq!(st_reg, 200);
    let owner_tok = reg["token"].as_str().expect("owner token").to_owned();

    // 2. Split-Key V2 login WITHOUT auth_verifier_b64 MUST fail closed with 401!
    for (label, payload) in [
        (
            "password_only_no_verifier",
            json!({"username": "split_v2_rotator", "password": "WrongPassword!"}),
        ),
        (
            "empty_verifier",
            json!({"username": "split_v2_rotator", "auth_verifier_b64": ""}),
        ),
        (
            "wrong_verifier",
            json!({"username": "split_v2_rotator", "auth_verifier_b64": wrong_verifier}),
        ),
    ] {
        let (st_bad, _, bad_json) =
            raw_json_request(addr, "POST", "/api/v1/auth/login", None, Some(&payload));
        assert_eq!(
            st_bad, 401,
            "Split-Key V2 login `{label}` must be rejected with 401, got: {bad_json}"
        );
        assert!(
            bad_json.get("token").and_then(Value::as_str).is_none(),
            "rejected Split-Key V2 login `{label}` must never mint a token: {bad_json}"
        );
    }

    // Valid Split-Key V2 login succeeds before rotation
    let (st_ok1, _, ok1) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(&json!({"username": "split_v2_rotator", "auth_verifier_b64": old_verifier})),
    );
    assert_eq!(st_ok1, 200, "valid old verifier login must succeed: {ok1}");
    assert!(!ok1["token"].as_str().unwrap_or("").is_empty());

    // 3. change-password with WRONG current_auth_verifier_b64 MUST be rejected with 401!
    let (st_chg_bad, _, chg_bad) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/change-password",
        Some(&owner_tok),
        Some(&json!({
            "current_auth_verifier_b64": wrong_verifier,
            "new_auth_salt_b64": new_salt,
            "new_auth_verifier_b64": new_verifier,
            "new_kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 },
            "rewrapped_vault_envelope": rewrapped_env
        })),
    );
    assert_eq!(
        st_chg_bad, 401,
        "change-password with wrong current_auth_verifier_b64 must return 401: {chg_bad}"
    );

    // 4. change-password with VALID current_auth_verifier_b64 rotates salt, verifier & vault envelope,
    //    revokes prior sessions, and mints a fresh session token
    let (st_chg_ok, _, chg_ok) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/change-password",
        Some(&owner_tok),
        Some(&json!({
            "current_auth_verifier_b64": old_verifier,
            "new_auth_salt_b64": new_salt,
            "new_auth_verifier_b64": new_verifier,
            "new_kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 },
            "rewrapped_vault_envelope": rewrapped_env
        })),
    );
    assert_eq!(st_chg_ok, 200, "valid change-password must return 200: {chg_ok}");
    assert_eq!(chg_ok["changed"], true);
    assert_eq!(chg_ok["auth_version"], 2);
    assert_eq!(chg_ok["key_version"], 2);
    let fresh_tok = chg_ok["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .expect("fresh token from change-password")
        .to_owned();
    assert_ne!(fresh_tok, owner_tok);

    // Prior session token MUST be revoked on both HTTP and WS; fresh token works immediately
    let (st_old_chats, _, _) =
        raw_json_request(addr, "GET", "/api/v1/chats", Some(&owner_tok), None);
    assert_eq!(st_old_chats, 401, "prior session token must be revoked after change-password");
    let (st_fresh_env, _, fresh_env) = raw_json_request(
        addr,
        "GET",
        "/api/v1/user-vault/envelope",
        Some(&fresh_tok),
        None,
    );
    assert_eq!(st_fresh_env, 200);
    assert_eq!(fresh_env["envelope"], rewrapped_env);
    assert_eq!(fresh_env["key_version"], 2);

    // 5. KDF params reflect new salt, OLD verifier is now rejected (401), NEW verifier succeeds (200)
    let (st_kdf, _, kdf_json) = raw_json_request(
        addr,
        "GET",
        "/api/v1/auth/kdf-params?identifier=split_v2_rotator",
        None,
        None,
    );
    assert_eq!(st_kdf, 200);
    assert_eq!(kdf_json["auth_salt_b64"], new_salt);

    let (st_old_login, _, old_login) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(&json!({"username": "split_v2_rotator", "auth_verifier_b64": old_verifier})),
    );
    assert_eq!(
        st_old_login, 401,
        "old verifier MUST return 401 after rotation: {old_login}"
    );

    let (st_new_login, _, new_login) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(&json!({"username": "split_v2_rotator", "auth_verifier_b64": new_verifier})),
    );
    assert_eq!(
        st_new_login, 200,
        "new verifier MUST succeed after rotation: {new_login}"
    );
    let rotated_tok = new_login["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .expect("new token after rotation")
        .to_owned();
    assert_eq!(new_login["vault_envelope"], rewrapped_env);

    let (st_env, _, env_json) = raw_json_request(
        addr,
        "GET",
        "/api/v1/user-vault/envelope",
        Some(&rotated_tok),
        None,
    );
    assert_eq!(st_env, 200);
    assert_eq!(env_json["envelope"], rewrapped_env);
}

#[test]
fn self_validate_loopback_blocking2_alipay_ws_revocation_and_chat_idempotency() {
    let server = LoopbackServer::start().expect("start loopback server");
    let addr = server.addr();

    // 1. Alipay /start emits `fbs_` state + qr_payload + poll_after_ms == 1000 + expires_in_ms == 600_000;
    //    confirm_alipay_attempt token (`tok_alipay_confirmed`) MUST succeed on protected APIs (NOT 401!)
    let (st_qr, _, qr_start) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/start",
        None,
        Some(&json!({"client_mode": "ipad_qr", "redirect_scheme": "fabushi"})),
    );
    assert_eq!(st_qr, 200);
    let attempt_qr = qr_start["attempt_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .expect("attempt_id")
        .to_owned();
    let state_qr = qr_start["state"].as_str().expect("state").to_owned();
    assert!(
        state_qr.starts_with("fbs_"),
        "Alipay /start state must have `fbs_` prefix, got: {state_qr}"
    );
    assert_eq!(qr_start["auth_url"], qr_start["qr_payload"]);
    assert!(!qr_start["qr_payload"].as_str().unwrap_or("").is_empty());
    assert_eq!(qr_start["poll_after_ms"], 1000);
    assert_eq!(qr_start["expires_in_ms"], 600_000);

    let (st_p1, _, p1) = raw_json_request(
        addr,
        "GET",
        &format!("/api/v1/auth/alipay/poll/{attempt_qr}"),
        None,
        None,
    );
    assert_eq!(st_p1, 200);
    assert_eq!(p1["status"], "pending");

    server.confirm_alipay_attempt(
        &attempt_qr,
        json!({
            "user_id": 2001,
            "session_id": "sess_alipay_confirmed_1",
            "token": "tok_alipay_confirmed",
            "expires_at": 4_102_444_800_000i64
        }),
    );

    let (st_p2, _, p2) = raw_json_request(
        addr,
        "GET",
        &format!("/api/v1/auth/alipay/poll/{attempt_qr}"),
        None,
        None,
    );
    assert_eq!(st_p2, 200);
    assert_eq!(p2["status"], "confirmed");
    assert_eq!(p2["session"]["token"], "tok_alipay_confirmed");

    // Protected calls with `tok_alipay_confirmed` MUST return 200 (NOT 401 session_revoked!)
    let (st_ali_me, _, _) = raw_json_request(
        addr,
        "GET",
        "/api/v1/auth/me",
        Some("tok_alipay_confirmed"),
        None,
    );
    assert_eq!(
        st_ali_me, 200,
        "confirm_alipay_attempt token must succeed on /api/v1/auth/me (not 401)"
    );
    let (st_ali_chats, _, _) = raw_json_request(
        addr,
        "GET",
        "/api/v1/chats",
        Some("tok_alipay_confirmed"),
        None,
    );
    assert_eq!(
        st_ali_chats, 200,
        "confirm_alipay_attempt token must succeed on /api/v1/chats (not 401)"
    );

    // 2. Alipay /exchange: CSRF mismatch rejected (400), valid exchange mints usable token (200),
    //    and replay of the same state or auth_code is rejected (400)
    let (st_ex_start, _, ex_start) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/start",
        None,
        Some(&json!({"client_mode": "ios_oauth", "redirect_scheme": "fabushi"})),
    );
    assert_eq!(st_ex_start, 200);
    let attempt_ex = ex_start["attempt_id"].as_str().expect("attempt_ex").to_owned();
    let state_ex = ex_start["state"].as_str().expect("state_ex").to_owned();
    assert!(state_ex.starts_with("fbs_"));
    assert_ne!(state_ex, state_qr);

    let (st_csrf, _, csrf_json) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/exchange",
        None,
        Some(&json!({
            "attempt_id": attempt_ex,
            "state": "fbs_forged_csrf_mismatch",
            "auth_code": "code_forged_1"
        })),
    );
    assert_eq!(st_csrf, 400, "forged Alipay CSRF state must return 400: {csrf_json}");

    let (st_ex_ok, _, ex_ok) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/exchange",
        None,
        Some(&json!({
            "attempt_id": attempt_ex,
            "state": state_ex,
            "auth_code": "alipay_code_once_42"
        })),
    );
    assert_eq!(st_ex_ok, 200, "valid Alipay /exchange must succeed: {ex_ok}");
    let exch_tok = ex_ok["token"]
        .as_str()
        .or_else(|| ex_ok["session"]["token"].as_str())
        .filter(|s| !s.is_empty())
        .expect("Alipay /exchange must emit non-empty token")
        .to_owned();

    let (st_ex_me, _, _) =
        raw_json_request(addr, "GET", "/api/v1/auth/me", Some(&exch_tok), None);
    assert_eq!(
        st_ex_me, 200,
        "Alipay /exchange token must succeed on protected /api/v1/auth/me"
    );

    // Replay #1: exact same state + auth_code -> 400
    let (st_rep1, _, rep1) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/exchange",
        None,
        Some(&json!({
            "attempt_id": attempt_ex,
            "state": state_ex,
            "auth_code": "alipay_code_once_42"
        })),
    );
    assert_eq!(st_rep1, 400, "replayed Alipay state+code must return 400: {rep1}");

    // Replay #2: same consumed state with a different auth_code -> 400
    let (st_rep2, _, rep2) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/exchange",
        None,
        Some(&json!({
            "attempt_id": attempt_ex,
            "state": state_ex,
            "auth_code": "alipay_code_fresh_99"
        })),
    );
    assert_eq!(st_rep2, 400, "replayed consumed Alipay state must return 400: {rep2}");

    // Replay #3: fresh attempt/state reusing already-consumed auth_code -> 400
    let (st_ex3, _, ex3_start) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/start",
        None,
        Some(&json!({"client_mode": "ios_oauth"})),
    );
    assert_eq!(st_ex3, 200);
    let (st_rep3, _, rep3) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/alipay/exchange",
        None,
        Some(&json!({
            "attempt_id": ex3_start["attempt_id"],
            "state": ex3_start["state"],
            "auth_code": "alipay_code_once_42"
        })),
    );
    assert_eq!(st_rep3, 400, "replayed consumed Alipay auth_code must return 400: {rep3}");

    // 3. WebSocket upgrade `/api/v1/ws?token=<revoked>` after session revocation MUST return HTTP 401 (NOT 101!)
    let verifier = B64.encode([0x55u8; 32]);
    let (st_u, _, u_reg) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/register",
        None,
        Some(&json!({
            "username": "ws_revocation_owner",
            "auth_version": 2,
            "auth_salt_b64": B64.encode([0x44u8; 16]),
            "auth_verifier_b64": verifier,
            "device_name": "Owner iPhone"
        })),
    );
    assert_eq!(st_u, 200);
    let owner_tok = u_reg["token"].as_str().expect("owner tok").to_owned();

    let (st_sib, _, sib_login) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(&json!({
            "username": "ws_revocation_owner",
            "auth_verifier_b64": verifier,
            "device_name": "Sibling iPad"
        })),
    );
    assert_eq!(st_sib, 200);
    let sib_sid = sib_login["session_id"].as_str().expect("sib_sid").to_owned();
    let sib_tok = sib_login["token"].as_str().expect("sib_tok").to_owned();

    // Positive control BEFORE revocation: valid ?token=<sib_tok> upgrades to 101 cleanly
    {
        let mut ws = open_ws_client_path(addr, &format!("/api/v1/ws?token={sib_tok}"));
        let close = encode_masked_ws_frame(true, 0x8, &[], [0x11, 0x22, 0x33, 0x44]);
        let _ = ws.write_all(&close);
    }

    // Revoke sibling session
    let (st_rev, _, _) = raw_json_request(
        addr,
        "DELETE",
        &format!("/api/v1/auth/sessions/{sib_sid}"),
        Some(&owner_tok),
        None,
    );
    assert_eq!(st_rev, 200);

    // Query-param WS upgrade with revoked token MUST return HTTP 401 (NOT 101!)
    let ws_revoked_query = format!(
        "GET /api/v1/ws?token={sib_tok} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    let (st_ws_q, head_ws_q, body_ws_q) = raw_http_request(addr, &ws_revoked_query);
    assert_eq!(
        st_ws_q, 401,
        "WS upgrade /api/v1/ws?token=<revoked> MUST return 401, got {st_ws_q}: {head_ws_q}"
    );
    assert!(!head_ws_q.contains("101 Switching Protocols"));
    let ws_err_q: Value = serde_json::from_slice(&body_ws_q).expect("WS 401 JSON body");
    assert_eq!(ws_err_q["error"], "unauthorized");

    // Authorization: Bearer header WS upgrade with revoked token MUST also return HTTP 401
    let ws_revoked_hdr = format!(
        "GET /api/v1/ws HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {sib_tok}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    let (st_ws_h, head_ws_h, _) = raw_http_request(addr, &ws_revoked_hdr);
    assert_eq!(
        st_ws_h, 401,
        "WS upgrade with revoked Bearer header MUST return 401, got {st_ws_h}: {head_ws_h}"
    );

    // Unrecognized WS path MUST fail closed with HTTP 404
    let ws_bogus_path = format!(
        "GET /api/v1/ws_bogus HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    let (st_ws_404, _, _) = raw_http_request(addr, &ws_bogus_path);
    assert_eq!(st_ws_404, 404, "WS upgrade on /api/v1/ws_bogus must return 404");

    // 4. Repeated POST /api/v1/chats with identical client_nonce returns EXACT same chat_id (string & integer nonces)
    let idem_payload = json!({
        "client_nonce": "grp_idem_reg_42",
        "chat_type": 1,
        "title": "Idempotent Group",
        "member_ids": [42, 42]
    });
    let (st_c1, _, c1) = raw_json_request(
        addr,
        "POST",
        "/api/v1/chats",
        Some(&owner_tok),
        Some(&idem_payload),
    );
    assert_eq!(st_c1, 200);
    let cid1 = c1["chat_id"].as_i64().expect("c1 chat_id");
    assert_eq!(c1["member_count"], 2, "deduplicated [42, 42] + creator = 2 members");

    let (st_c2, _, c2) = raw_json_request(
        addr,
        "POST",
        "/api/v1/chats",
        Some(&owner_tok),
        Some(&idem_payload),
    );
    assert_eq!(st_c2, 200);
    assert_eq!(
        c2["chat_id"].as_i64(),
        Some(cid1),
        "repeated POST /api/v1/chats with same client_nonce MUST return exact same chat_id"
    );
    assert_eq!(c2["deduplicated"], true);

    // Alias /api/v1/chats/create with same client_nonce also returns exact same chat_id
    let (st_c3, _, c3) = raw_json_request(
        addr,
        "POST",
        "/api/v1/chats/create",
        Some(&owner_tok),
        Some(&idem_payload),
    );
    assert_eq!(st_c3, 200);
    assert_eq!(c3["chat_id"].as_i64(), Some(cid1));
    assert_eq!(c3["deduplicated"], true);

    // Integer client_nonce deduplication
    let int_payload = json!({
        "client_nonce": 4242,
        "chat_type": 1,
        "title": "Integer Nonce Group",
        "member_ids": [42, 43]
    });
    let (st_ci1, _, ci1) = raw_json_request(
        addr,
        "POST",
        "/api/v1/chats",
        Some(&owner_tok),
        Some(&int_payload),
    );
    assert_eq!(st_ci1, 200);
    let cid_int = ci1["chat_id"].as_i64().expect("ci1 chat_id");
    assert_ne!(cid_int, cid1, "distinct client_nonce must create a new chat_id");
    let (st_ci2, _, ci2) = raw_json_request(
        addr,
        "POST",
        "/api/v1/chats",
        Some(&owner_tok),
        Some(&int_payload),
    );
    assert_eq!(st_ci2, 200);
    assert_eq!(ci2["chat_id"].as_i64(), Some(cid_int));
    assert_eq!(ci2["deduplicated"], true);

    // GET /api/v1/chats must list cid1 exactly once (no duplicate rows created)
    let (st_cl, _, cl) = raw_json_request(addr, "GET", "/api/v1/chats", Some(&owner_tok), None);
    assert_eq!(st_cl, 200);
    let count_cid1 = cl["chats"]
        .as_array()
        .expect("chats array")
        .iter()
        .filter(|ch| ch["chat_id"].as_i64() == Some(cid1))
        .count();
    assert_eq!(count_cid1, 1, "idempotent chat must appear exactly once in /api/v1/chats");

    // 5. Chal1 Advisory #2 wire hardening: strict RFC 9112 chunk hex (+a -> 400) & WS invalid UTF-8 -> 1007
    let bad_chunk_req = format!(
        "POST /api/v1/media/upload HTTP/1.1\r\nHost: {addr}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n+a\r\n0123456789\r\n0\r\n\r\n"
    );
    let (st_bad_chunk, _, bad_chunk_body) = raw_http_request(addr, &bad_chunk_req);
    assert_eq!(st_bad_chunk, 400, "signed chunk size `+a` must return 400");
    let bad_chunk_json: Value = serde_json::from_slice(&bad_chunk_body).expect("400 JSON");
    assert_eq!(bad_chunk_json["message"], "invalid_chunk_size");

    {
        let mut ws = open_ws_client(addr);
        let invalid_utf8 =
            encode_masked_ws_frame(true, 0x1, &[0xFF, 0xFE], [0xAA, 0xBB, 0xCC, 0xDD]);
        ws.write_all(&invalid_utf8).expect("write invalid utf8 text");
        ws.flush().expect("flush invalid utf8 text");
        let (close_op, close_payload) = read_server_ws_frame(&mut ws);
        assert_eq!(close_op, 0x8, "invalid UTF-8 text frame must trigger Close opcode 0x8");
        assert!(close_payload.len() >= 2);
        assert_eq!(
            u16::from_be_bytes([close_payload[0], close_payload[1]]),
            1007,
            "invalid UTF-8 text frame must return RFC 6455 close code 1007"
        );
    }
}

#[test]
fn self_validate_loopback_hardening_non_ascii_utf8_prefix_safety() {
    let server = LoopbackServer::start();
    let addr = server.addr();

    // Multi-byte UTF-8 straddling byte 7 (`全` at 5..8) and byte 8 (`球` at 7..10)
    let req_utf8_7 = format!("GET /iv/a全b HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (st1, h1, b1) = raw_http_request(addr, &req_utf8_7);
    assert_eq!(st1, 200);
    assert!(h1.to_ascii_lowercase().contains("text/html"));
    assert!(String::from_utf8_lossy(&b1).contains("a全b"));

    let req_utf8_8 =
        format!("GET /iv/全球法布施 HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (st2, h2, b2) = raw_http_request(addr, &req_utf8_8);
    assert_eq!(st2, 200);
    assert!(h2.to_ascii_lowercase().contains("text/html"));
    assert!(String::from_utf8_lossy(&b2).contains("全球法布施"));

    // Raw invalid UTF-8 bytes in request target must not panic connection thread
    let mut invalid_req = Vec::new();
    invalid_req.extend_from_slice(b"GET /api/v1/\xFF\xFE\xFD HTTP/1.1\r\nHost: ");
    invalid_req.extend_from_slice(addr.to_string().as_bytes());
    invalid_req.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
    let (st_invalid, _, _) = raw_http_bytes(addr, &invalid_req);
    assert_eq!(st_invalid, 404);

    // Non-ASCII Authorization header straddling byte 7 (`测` at 6..9)
    let non_ascii_auth = format!(
        "GET /api/v1/status HTTP/1.1\r\nHost: {addr}\r\nAuthorization: 令牌测试token\r\nConnection: close\r\n\r\n"
    );
    let (st_auth, _, _) = raw_http_request(addr, &non_ascii_auth);
    assert_eq!(st_auth, 200);

    // Uppercase absolute-form HTTP/HTTPS proxy targets and bare scheme targets
    let abs_http = format!(
        "GET HTTP://{addr}/api/v1/status HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    );
    let (st_abs_http, _, _) = raw_http_request(addr, &abs_http);
    assert_eq!(st_abs_http, 200);

    let abs_https = format!(
        "GET HTTPS://{addr}/api/v1/status HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    );
    let (st_abs_https, _, _) = raw_http_request(addr, &abs_https);
    assert_eq!(st_abs_https, 200);

    let bare_http = format!("GET http:// HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (st_bare_http, _, _) = raw_http_request(addr, &bare_http);
    assert_eq!(st_bare_http, 404);

    let bare_https = format!("GET https:// HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (st_bare_https, _, _) = raw_http_request(addr, &bare_https);
    assert_eq!(st_bare_https, 404);

    // Confirm server remains healthy with zero mutex poisoning
    let (st_health, _, health_json) = raw_json_request(addr, "GET", "/health", None, None);
    assert_eq!(st_health, 200);
    assert_eq!(health_json["ok"], true);
}

#[test]
fn self_validate_loopback_hardening_percent_decoded_query_params() {
    let server = LoopbackServer::start();
    let addr = server.addr();

    let unique_salt = B64.encode([0x9Au8; 16]);
    let verifier = B64.encode([0x55u8; 32]);
    let (st_reg, _, reg) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/register",
        None,
        Some(&json!({
            "username": "alice@example.com",
            "auth_version": 2,
            "auth_salt_b64": unique_salt,
            "auth_verifier_b64": verifier
        })),
    );
    assert_eq!(st_reg, 200);
    let tok = reg["token"].as_str().expect("token").to_owned();

    // Percent-encoded `@` in identifier query param must resolve exact registered salt
    let (st_kdf, _, kdf) = raw_json_request(
        addr,
        "GET",
        "/api/v1/auth/kdf-params?identifier=alice%40example.com",
        None,
        None,
    );
    assert_eq!(st_kdf, 200);
    assert_eq!(kdf["auth_salt_b64"].as_str(), Some(unique_salt.as_str()));

    // Create chat and post CJK message, then search via percent-encoded UTF-8 query
    let (st_chat, _, chat) = raw_json_request(
        addr,
        "POST",
        "/api/v1/chats",
        Some(&tok),
        Some(&json!({"title": "Dharma Search Room"})),
    );
    assert_eq!(st_chat, 200);
    let cid = chat["chat_id"].as_i64().expect("chat_id");

    let (st_msg, _, _) = raw_json_request(
        addr,
        "POST",
        &format!("/api/v1/chats/{cid}/messages"),
        Some(&tok),
        Some(&json!({"text": "全球法布施 v1.6.8"})),
    );
    assert_eq!(st_msg, 200);

    let (st_search, _, search) = raw_json_request(
        addr,
        "GET",
        "/api/v1/search/messages?q=%E5%85%A8%E7%90%83%E6%B3%95",
        Some(&tok),
        None,
    );
    assert_eq!(st_search, 200);
    assert_eq!(search["query"], "全球法布施");
    assert_eq!(search["total"], 1);

    // Percent-encoded whitespace `%20%20` trims to empty string
    let (st_ws_q, _, ws_q) = raw_json_request(
        addr,
        "GET",
        "/api/v1/search/messages?q=%20%20",
        Some(&tok),
        None,
    );
    assert_eq!(st_ws_q, 200);
    assert_eq!(ws_q["query"], "");

    // Percent-encoded `?token=` works before logout and is rejected after logout on HTTP + WS
    let encoded_tok = tok.replace('_', "%5F");
    let (st_me_ok, _, me_ok) = raw_json_request(
        addr,
        "GET",
        &format!("/api/v1/auth/me?token={encoded_tok}"),
        None,
        None,
    );
    assert_eq!(st_me_ok, 200);
    assert_eq!(me_ok["username"], "alice@example.com");

    let (st_logout, _, _) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/logout",
        Some(&tok),
        Some(&json!({})),
    );
    assert_eq!(st_logout, 200);

    let (st_me_revoked, _, me_revoked) = raw_json_request(
        addr,
        "GET",
        &format!("/api/v1/auth/me?token={encoded_tok}"),
        None,
        None,
    );
    assert_eq!(st_me_revoked, 401);
    assert_eq!(me_revoked["error"], "unauthorized");

    let ws_revoked_req = format!(
        "GET /api/v1/ws?token={encoded_tok} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    let (st_ws_revoked, _, _) = raw_http_request(addr, &ws_revoked_req);
    assert_eq!(st_ws_revoked, 401);
}

#[test]
fn self_validate_loopback_hardening_reject_empty_whitespace_unseeded_auth() {
    let server = LoopbackServer::start();
    let addr = server.addr();

    // Unseeded POST /api/v1/auth/login with empty or whitespace-only credentials MUST fail closed with 401
    for payload in [
        json!({}),
        json!({"username": "u_empty", "password": "", "auth_verifier_b64": ""}),
        json!({"username": "u_ws", "password": "   ", "auth_verifier_b64": " \t "}),
    ] {
        let (st, _, resp) = raw_json_request(addr, "POST", "/api/v1/auth/login", None, Some(&payload));
        assert_eq!(st, 401, "expected 401 for empty/ws login payload {payload}");
        assert_eq!(resp["error"], "invalid_credentials");
        assert!(resp.get("token").is_none());
    }

    // POST /api/v1/auth/register with missing/whitespace username or missing/whitespace credentials MUST fail closed with 400
    for payload in [
        json!({}),
        json!({"username": "   ", "password": "ValidPassword1!"}),
        json!({"username": "u_nocreds", "password": "", "auth_verifier_b64": "   "}),
    ] {
        let (st, _, resp) =
            raw_json_request(addr, "POST", "/api/v1/auth/register", None, Some(&payload));
        assert_eq!(st, 400, "expected 400 for invalid register payload {payload}");
        assert!(resp.get("token").is_none());
    }

    // POST /api/v1/auth/phone/verify with omitted `code` MUST fail closed with 400 invalid_code
    let (st_phone, _, phone_resp) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/phone/verify",
        None,
        Some(&json!({"phone": "+14155550199"})),
    );
    assert_eq!(st_phone, 400);
    assert_eq!(phone_resp["error"], "invalid_code");

    // Positive controls: valid unseeded login & valid register succeed with tokens
    let (st_ok_login, _, ok_login) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(&json!({
            "username": "valid_unseeded",
            "auth_verifier_b64": B64.encode([0x44u8; 32])
        })),
    );
    assert_eq!(st_ok_login, 200);
    assert!(ok_login["token"].as_str().is_some());

    let (st_ok_reg, _, ok_reg) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/register",
        None,
        Some(&json!({
            "username": "valid_registered",
            "auth_verifier_b64": B64.encode([0x45u8; 32])
        })),
    );
    assert_eq!(st_ok_reg, 200);
    assert!(ok_reg["token"].as_str().is_some());
}

#[test]
fn self_validate_loopback_hardening_media_routes_enforce_token_revocation() {
    let server = LoopbackServer::start();
    let addr = server.addr();

    let verifier = B64.encode([0x77u8; 32]);
    let (st_reg, _, reg) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/register",
        None,
        Some(&json!({
            "username": "media_owner",
            "auth_version": 2,
            "auth_verifier_b64": verifier,
            "device_name": "Primary iPhone"
        })),
    );
    assert_eq!(st_reg, 200);
    let owner_tok = reg["token"].as_str().expect("owner token").to_owned();

    let (st_sib, _, sib) = raw_json_request(
        addr,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(&json!({
            "username": "media_owner",
            "auth_verifier_b64": verifier,
            "device_name": "Sibling iPad"
        })),
    );
    assert_eq!(st_sib, 200);
    let sib_tok = sib["token"].as_str().expect("sibling token").to_owned();
    let sib_sid = sib["session_id"].as_str().expect("sibling session_id").to_owned();

    server.seed_media_blob("rev_blob", b"fabushi_v168_media_payload_bytes".to_vec());

    let (st_init, _, init_resp) = raw_json_request(
        addr,
        "POST",
        "/api/v1/media/upload/init",
        Some(&owner_tok),
        Some(&json!({"filename": "clip.bin", "total_size": 16, "part_size": 16})),
    );
    assert_eq!(st_init, 200);
    let upl_id = init_resp["upload_id"].as_str().expect("upload_id").to_owned();

    // Sibling token works before revocation
    let pre_get = format!(
        "GET /api/v1/media/blob/rev_blob HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {sib_tok}\r\nConnection: close\r\n\r\n"
    );
    let (st_pre_blob, _, _) = raw_http_request(addr, &pre_get);
    assert_eq!(st_pre_blob, 200);

    let (st_pre_stat, _, _) = raw_json_request(
        addr,
        "GET",
        &format!("/api/v1/media/upload/{upl_id}/status"),
        Some(&sib_tok),
        None,
    );
    assert_eq!(st_pre_stat, 200);

    // Owner revokes sibling session
    let (st_rev, _, _) = raw_json_request(
        addr,
        "DELETE",
        &format!("/api/v1/auth/sessions/{sib_sid}"),
        Some(&owner_tok),
        None,
    );
    assert_eq!(st_rev, 200);

    // Revoked sibling token MUST get 401 across ALL media routes (binary, range, HEAD, query token, JSON, nonexistent)
    let raw_media_checks = [
        format!(
            "POST /api/v1/media/upload HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {sib_tok}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd"
        ),
        format!(
            "PUT /api/v1/media/upload/{upl_id}/part/0 HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {sib_tok}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd"
        ),
        format!(
            "GET /api/v1/media/blob/rev_blob HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {sib_tok}\r\nRange: bytes=0-3\r\nConnection: close\r\n\r\n"
        ),
        format!(
            "GET /api/v1/media/stream/rev_blob?token={sib_tok} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        ),
        format!(
            "GET /api/v1/media/nonexistent_blob_xyz HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {sib_tok}\r\nConnection: close\r\n\r\n"
        ),
    ];
    for req in &raw_media_checks {
        let (st, _, raw_body) = raw_http_request(addr, req);
        assert_eq!(st, 401, "expected 401 for revoked media request: {req}");
        let v: Value = serde_json::from_slice(&raw_body).expect("401 json");
        assert_eq!(v["error"], "unauthorized");
        assert_eq!(v["message"], "session_revoked");
    }

    let head_revoked = format!(
        "HEAD /api/v1/media/blob/rev_blob HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {sib_tok}\r\nConnection: close\r\n\r\n"
    );
    let (st_head, _, head_body) = raw_http_request(addr, &head_revoked);
    assert_eq!(st_head, 401);
    assert!(head_body.is_empty(), "HEAD 401 must have empty body");

    for (method, path, payload) in [
        (
            "POST",
            "/api/v1/media/upload/init".to_owned(),
            Some(json!({"filename": "x.bin", "total_size": 8, "part_size": 8})),
        ),
        (
            "GET",
            format!("/api/v1/media/upload/{upl_id}/status"),
            None,
        ),
        (
            "POST",
            format!("/api/v1/media/upload/{upl_id}/complete"),
            Some(json!({})),
        ),
        (
            "POST",
            "/api/v1/media/edit".to_owned(),
            Some(json!({"media_id": "rev_blob"})),
        ),
        ("GET", "/api/v1/media/instant-view?url=/article/v168".to_owned(), None),
    ] {
        let (st, _, resp) = raw_json_request(addr, method, &path, Some(&sib_tok), payload.as_ref());
        assert_eq!(st, 401, "expected 401 for revoked {method} {path}");
        assert_eq!(resp["error"], "unauthorized");
    }

    // Active owner token and unauthenticated loopback fetch still succeed (200)
    let owner_get = format!(
        "GET /api/v1/media/blob/rev_blob HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {owner_tok}\r\nConnection: close\r\n\r\n"
    );
    let (st_owner, _, owner_body) = raw_http_request(addr, &owner_get);
    assert_eq!(st_owner, 200);
    assert_eq!(owner_body, b"fabushi_v168_media_payload_bytes");

    let unauth_get =
        format!("GET /api/v1/media/blob/rev_blob HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (st_unauth, _, _) = raw_http_request(addr, &unauth_get);
    assert_eq!(st_unauth, 200);
}

