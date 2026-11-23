//! Self-validation & anti-stub negative-oracle test suite executed in CI `self-validate` mode.
//!
//! Zero `unsafe` code, zero process-env mutation UB, zero skipped assertions. Proves:
//! 1. Ephemeral real-socket HTTP/1.1 (multi-packet + RFC 9112 chunked) + HTTP 206/416 + fault injection +
//!    real media upload hashing + stateful multi-device tables + fail-closed 404 + strict RFC 6455 WebSocket
//!    (unmasked `1002`, fragmented reassembly, interleaved Ping/Pong, 16-bit extended length split across timeout).
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

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn open_ws_client(addr: std::net::SocketAddr) -> TcpStream {
    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("connect WS TCP");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set WS read timeout");
    let upgrade_req = format!(
        "GET /api/v1/ws HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
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
    assert!(handshake_str.starts_with("HTTP/1.1 101 Switching Protocols"));
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

    let post_req = format!(
        "POST /api/v1/auth/login HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
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
