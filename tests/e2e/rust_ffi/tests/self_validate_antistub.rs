//! Self-validation & anti-stub negative-oracle test suite executed in CI `self-validate` mode.
//!
//! Zero skipped assertions. Proves:
//! 1. Ephemeral real-socket HTTP/1.1 + HTTP 206/416 + fault injection + RFC 6455 WebSocket loopback server works cleanly.
//! 2. All pure-Rust domain & boundary oracles (RFC 6238 TOTP, WCAG >138 luma, 5-bit 63B waveform, 24 MiB vault cap, etc.) are exact.
//! 3. `FabushiE2EHarness::connect_production()` fails closed when `FABUSHI_HOST_LIB` is unset, missing, or points to `mutant_stub`.
//! 4. Every anti-stub oracle catches all 8 deliberate defects injected by `mutant_stub`.
//! 5. Against the ACTUAL compiled baseline `8128951` `libmahayana_app_host.so` (required when `FABUSHI_REQUIRE_BASELINE_PROOF=1`),
//!    proves all 6 `fabushi_host_*` symbols are missing and legacy defects (`acknowledged`, `sample_chats`, missing `retryable`/`id`/`generation`) are caught.

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
    FfiHostClient, LegacyHost8128951Probe, REQUIRED_HOST_SYMBOLS, probe_required_symbols,
};
use fabushi_e2e_ffi::loopback_server::{LoopbackServer, compute_websocket_accept};
use fabushi_e2e_ffi::{
    ENV_FABUSHI_BASELINE_HOST_LIB, ENV_FABUSHI_HOST_LIB, ENV_FABUSHI_MUTANT_STUB_LIB,
    ENV_FABUSHI_REQUIRE_BASELINE_PROOF, FabushiE2EHarness,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tempfile::TempDir;

static ENV_MUTEX: Mutex<()> = Mutex::new(());

fn raw_http_request(addr: std::net::SocketAddr, request: &str) -> (u16, String, Vec<u8>) {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .expect("connect to loopback server");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set read timeout");
    stream
        .write_all(request.as_bytes())
        .expect("write HTTP request");
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
    let _guard = ENV_MUTEX.lock().expect("lock ENV_MUTEX");
    let prev = std::env::var("FABUSHI_MUTANT_MODE").ok();
    // SAFETY: Protected by global `ENV_MUTEX` so no concurrent env mutation occurs within this test binary.
    unsafe {
        std::env::set_var("FABUSHI_MUTANT_MODE", mode);
    }
    let cfg = json!({"mutant_mode": mode}).to_string();
    let client = FfiHostClient::load_for_antistub_oracle(&stub_path, &cfg)
        .unwrap_or_else(|e| panic!("failed to load mutant_stub in mode `{mode}`: {e}"));
    // SAFETY: Restoring previous value while still holding `ENV_MUTEX`.
    unsafe {
        match prev {
            Some(v) => std::env::set_var("FABUSHI_MUTANT_MODE", v),
            None => std::env::remove_var("FABUSHI_MUTANT_MODE"),
        }
    }
    client
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

    let mut stream = TcpStream::connect_timeout(&server.addr(), Duration::from_secs(2))
        .expect("connect WS TCP");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set WS read timeout");

    let upgrade_req = format!(
        "GET /api/v1/ws HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        server.addr()
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

    // Read queued server push frame
    let mut frame_hdr = [0u8; 2];
    stream.read_exact(&mut frame_hdr).expect("read push frame hdr");
    assert_eq!(frame_hdr[0] & 0x0F, 0x1);
    let push_len = (frame_hdr[1] & 0x7F) as usize;
    let mut push_payload = vec![0u8; push_len];
    stream
        .read_exact(&mut push_payload)
        .expect("read push payload");
    assert_eq!(
        String::from_utf8_lossy(&push_payload),
        r#"{"kind":"pts_push","pts":7}"#
    );

    // Send masked client text frame and receive server ws_ack
    let client_msg = b"hello_fabushi_ws";
    let mask = [0x37u8, 0xFA, 0x21, 0x3D];
    let mut client_frame = vec![0x81u8, 0x80 | (client_msg.len() as u8)];
    client_frame.extend_from_slice(&mask);
    for (i, &b) in client_msg.iter().enumerate() {
        client_frame.push(b ^ mask[i % 4]);
    }
    stream.write_all(&client_frame).expect("send masked WS text");
    stream.flush().expect("flush masked WS text");

    let mut ack_hdr = [0u8; 2];
    stream.read_exact(&mut ack_hdr).expect("read ack frame hdr");
    let ack_len = (ack_hdr[1] & 0x7F) as usize;
    let mut ack_payload = vec![0u8; ack_len];
    stream.read_exact(&mut ack_payload).expect("read ack payload");
    let ack_json: Value = serde_json::from_slice(&ack_payload).expect("valid ack JSON");
    assert_eq!(ack_json["type"], "ws_ack");
    assert_eq!(ack_json["echo"], "hello_fabushi_ws");

    // Send masked Close frame cleanly
    let close_frame = [0x88u8, 0x80, 0x01, 0x02, 0x03, 0x04];
    let _ = stream.write_all(&close_frame);
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
    let _guard = ENV_MUTEX.lock().expect("lock ENV_MUTEX");
    let prev = std::env::var(ENV_FABUSHI_HOST_LIB).ok();

    // 1. Unset FABUSHI_HOST_LIB -> MUST fail closed
    // SAFETY: Serialized by `ENV_MUTEX`.
    unsafe {
        std::env::remove_var(ENV_FABUSHI_HOST_LIB);
    }
    let err_unset = FabushiE2EHarness::try_connect_production()
        .err()
        .expect("unset FABUSHI_HOST_LIB must fail closed");
    assert!(err_unset.contains(ENV_FABUSHI_HOST_LIB));

    // 2. Nonexistent path -> MUST fail closed
    let err_missing = FabushiE2EHarness::try_connect_production_at(Path::new(
        "/nonexistent/fabushi/libmahayana_app_host.so",
    ))
    .err()
    .expect("nonexistent host lib must fail closed");
    assert!(err_missing.contains("does not exist"));

    // 3. mutant_stub path -> MUST be explicitly rejected!
    let err_stub = FabushiE2EHarness::try_connect_production_at(&stub_path)
        .err()
        .expect("connect_production MUST refuse mutant_stub");
    assert!(
        err_stub.contains("refuses mutant_stub"),
        "expected mutant_stub refusal, got: {err_stub}"
    );

    // SAFETY: Restoring original env while holding `ENV_MUTEX`.
    unsafe {
        if let Some(v) = prev {
            std::env::set_var(ENV_FABUSHI_HOST_LIB, v);
        }
    }
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
    let require_baseline = std::env::var(ENV_FABUSHI_REQUIRE_BASELINE_PROOF)
        .map(|v| v.trim() == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let baseline_path = match std::env::var(ENV_FABUSHI_BASELINE_HOST_LIB) {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
        _ => {
            if require_baseline {
                panic!(
                    "FAIL-CLOSED: {ENV_FABUSHI_REQUIRE_BASELINE_PROOF}=1 requires {ENV_FABUSHI_BASELINE_HOST_LIB} pointing to compiled 8128951 libmahayana_app_host.so"
                );
            }
            panic!(
                "FAIL-CLOSED: {ENV_FABUSHI_BASELINE_HOST_LIB} must be set for baseline negative proof (zero silent skip)"
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
