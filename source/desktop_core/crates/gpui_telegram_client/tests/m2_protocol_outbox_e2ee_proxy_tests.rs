use futures::{SinkExt, StreamExt};
use gpui_telegram_client::e2ee::keys::{
    b64_encode, random_32, x25519_pub_from_secret, DeviceIdentityKeys,
};
use gpui_telegram_client::e2ee::x3dh::{x3dh_initiator, x3dh_responder};
use gpui_telegram_client::media_transfer::{
    index_file_chunks, read_chunk_at, FileTransferRecord, TransferStatus,
};
use gpui_telegram_client::net::ws_channel::WsChannel;
use gpui_telegram_client::net::ws_transport::WsTransport;
use gpui_telegram_client::{
    compute_base_backoff_ms, compute_jittered_backoff_ms, ClientConfig, ClientProxyConfig,
    DoubleRatchetSession, HttpClient, LocalStore, NetworkState, OutboxStatus,
    RichSendMessageRequest, SenderKeyState, TelegramClient,
};
use gpui_telegram_core::types::{ChatId, MessageId, PeerId, UserId};
use gpui_telegram_proto::contracts::chats::ChatFolderDto;
use gpui_telegram_proto::e2ee::{
    DevicePreKeyBundle, EncryptedEnvelope, EncryptedPayloadKind, OneTimePreKeyDto,
    PublishPreKeysRequest, SignedPreKeyDto, UserPreKeyBundlesResponse,
};
use gpui_telegram_proto::message::{MessageContent, MessageItem};
use gpui_telegram_proto::ws::{WsClientFrame, WsServerFrame};
use parking_lot::Mutex;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message as WsMessage;

fn sha256_hex_local(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let mut out = String::with_capacity(64);
    for b in h.finalize() {
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

async fn read_http_request(stream: &mut TcpStream) -> (String, String, String, Vec<u8>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut header_end = None;
    while header_end.is_none() {
        let n = stream.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = Some(pos + 4);
        }
    }
    let hend = header_end.unwrap_or(buf.len());
    let headers_str = String::from_utf8_lossy(&buf[..hend]).to_string();
    let content_len = headers_str
        .lines()
        .find_map(|l| {
            let lower = l.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while buf.len() < hend + content_len {
        let n = stream.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let first_line = headers_str.lines().next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let body = if buf.len() >= hend {
        buf[hend..].to_vec()
    } else {
        Vec::new()
    };
    (method, path, headers_str, body)
}

async fn write_json_response(stream: &mut TcpStream, status: u16, reason: &str, json_body: &str) {
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json_body}",
        json_body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.flush().await;
}

fn make_rich_req(chat_id: i64, text: &str, nonce: Option<u64>) -> RichSendMessageRequest {
    RichSendMessageRequest {
        chat_id,
        text: Some(text.to_string()),
        content: Some(MessageContent::plain_text(text)),
        entities: Vec::new(),
        reply_to: None,
        quote_text: None,
        topic_id: None,
        client_nonce: nonce,
        forward_from_chat_id: None,
        forward_from_msg_id: None,
        silent: false,
        scheduled_at_ms: None,
        inline_keyboard: None,
        send_as_peer_id: None,
        skip_server_bot_reply: false,
    }
}

fn make_test_auth(user_id: u64, username: &str, token: &str) -> gpui_telegram_client::AuthResponse {
    gpui_telegram_client::AuthResponse {
        user_id,
        username: username.to_string(),
        first_name: Some(username.to_string()),
        token: token.to_string(),
        session_id: format!("sess_{user_id}_{username}"),
        created_at: None,
        expires_at: None,
    }
}

#[test]
fn test_f6_outbox_persistence_crash_recovery_backoff_and_reconnect_flush() {
    // 1. Verify exponential backoff & jitter bounds
    assert_eq!(compute_base_backoff_ms(0), 500);
    assert_eq!(compute_base_backoff_ms(1), 1000);
    assert_eq!(compute_base_backoff_ms(6), 30_000);
    assert_eq!(compute_base_backoff_ms(12), 30_000);
    for r in 0..8 {
        let j = compute_jittered_backoff_ms(r);
        assert!((250..=30_000).contains(&j), "jitter out of bounds: {j}");
    }

    // 2. Verify valid/invalid status transitions
    assert!(OutboxStatus::Pending.can_transition_to(OutboxStatus::InFlight));
    assert!(OutboxStatus::InFlight.can_transition_to(OutboxStatus::Delivered));
    assert!(OutboxStatus::InFlight.can_transition_to(OutboxStatus::Pending));
    assert!(OutboxStatus::InFlight.can_transition_to(OutboxStatus::Failed));
    assert!(!OutboxStatus::Delivered.can_transition_to(OutboxStatus::Pending));

    let dir = tempfile::tempdir().expect("tempdir");
    let rt = tokio::runtime::Runtime::new().expect("rt");

    // 3. Spin up mock server that rejects first attempt with 503 then accepts with 200 OK
    let fail_mode = Arc::new(AtomicBool::new(true));
    let delivered_nonces = Arc::new(Mutex::new(Vec::<u64>::new()));
    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();

    let fail_srv = Arc::clone(&fail_mode);
    let nonces_srv = Arc::clone(&delivered_nonces);
    rt.spawn(async move {
        let mut next_msg_id = 5000i64;
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let fail = Arc::clone(&fail_srv);
            let nonces = Arc::clone(&nonces_srv);
            tokio::spawn(async move {
                let (method, path, _, body) = read_http_request(&mut stream).await;
                if method == "POST"
                    && (path == "/api/v1/messages/send" || path == "/api/v1/chats/42/messages")
                {
                    let parsed: serde_json::Value =
                        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                    let nonce = parsed["client_nonce"].as_u64().unwrap_or(0);
                    let text_val = parsed["text"].as_str().unwrap_or("").to_string();
                    if text_val == "bad_permanent" {
                        write_json_response(
                            &mut stream,
                            400,
                            "Bad Request",
                            r#"{"error":"invalid message"}"#,
                        )
                        .await;
                        return;
                    }
                    if fail.load(Ordering::SeqCst) {
                        write_json_response(
                            &mut stream,
                            503,
                            "Service Unavailable",
                            r#"{"error":"transient outage"}"#,
                        )
                        .await;
                        return;
                    }
                    next_msg_id += 1;
                    nonces.lock().push(nonce);
                    let resp = json!({
                        "message_id": next_msg_id,
                        "id": next_msg_id,
                        "pts": 11,
                        "pts_count": 1,
                        "duplicate": false,
                        "date_ms": 1700000000000i64,
                        "date": "2026-11-17T00:00:00Z",
                        "sender_id": 100,
                        "chat_id": 42,
                        "text": text_val,
                        "client_nonce": nonce,
                        "status": "sent"
                    });
                    write_json_response(&mut stream, 200, "OK", &resp.to_string()).await;
                } else {
                    write_json_response(&mut stream, 200, "OK", "{}").await;
                }
            });
        }
    });

    let server_url = format!("http://{addr}");
    let cfg = ClientConfig {
        server_url: server_url.clone(),
        ..Default::default()
    };

    // 4. First session: create in-flight + pending records and verify persistence & nonce seeding
    let now_ms = chrono::Utc::now().timestamp_millis();
    let high_nonce = (now_ms as u64) + 50_000;
    {
        let client1 = TelegramClient::open_with_dir(cfg.clone(), dir.path()).unwrap();
        client1.set_auth_session(make_test_auth(100, "alice", "tok_a"));

        // Enqueue a high-nonce message directly into InFlight (simulating crash mid-send)
        let req1 = make_rich_req(42, "crashed mid flight", Some(high_nonce));
        let rec1 = client1
            .outbox
            .store()
            .enqueue_in_flight(&req1, high_nonce, now_ms)
            .unwrap();
        assert_eq!(rec1.status, OutboxStatus::InFlight);

        // Permanent 400 failure -> Failed immediately
        let perm_req = make_rich_req(42, "bad_permanent", Some(high_nonce + 1));
        let perm_res = rt.block_on(client1.send_message_durable(perm_req));
        assert!(perm_res.is_err());
        let perm_rec = client1
            .outbox
            .store()
            .get(high_nonce + 1)
            .unwrap()
            .expect("perm rec");
        assert_eq!(perm_rec.status, OutboxStatus::Failed);

        // Transient 503 failure while online -> stays Pending with retry_count=1
        let trans_req = make_rich_req(42, "retry me later", Some(high_nonce + 2));
        let trans_res = rt.block_on(client1.send_message_durable(trans_req));
        assert!(trans_res.is_err());
        let trans_rec = client1
            .outbox
            .store()
            .get(high_nonce + 2)
            .unwrap()
            .expect("trans rec");
        assert_eq!(trans_rec.status, OutboxStatus::Pending);
        assert_eq!(trans_rec.retry_count, 1);
    }

    // 5. Reopen client on same directory -> crash recovery resets InFlight -> Pending & seeds nonce
    let client2 = TelegramClient::open_with_dir(cfg, dir.path()).unwrap();
    let recovered = client2
        .outbox
        .store()
        .recover_in_flight(chrono::Utc::now().timestamp_millis())
        .unwrap();
    assert!(recovered >= 1, "crashed InFlight record must recover to Pending");
    let rec1_after = client2.outbox.store().get(high_nonce).unwrap().unwrap();
    assert_eq!(rec1_after.status, OutboxStatus::Pending);
    assert!(
        client2.next_client_nonce() > high_nonce + 2,
        "next_nonce must be seeded above persisted max outbox nonce"
    );
    client2.set_auth_session(make_test_auth(100, "alice", "tok_a"));

    // 6. Server recovers & NetworkStatusListener notifies Online -> immediate flush!
    fail_mode.store(false, Ordering::SeqCst);
    client2.network_listener.set_online(false);
    assert_eq!(client2.network_listener.state(), NetworkState::Offline);
    client2.network_listener.set_online(true);
    let _ = rt.block_on(client2.outbox.notify_reconnected_and_flush());
    let _ = rt.block_on(client2.outbox.flush_due());

    let r1 = client2.outbox.store().get(high_nonce).unwrap().unwrap();
    let r2 = client2.outbox.store().get(high_nonce + 2).unwrap().unwrap();
    assert_eq!(r1.status, OutboxStatus::Delivered);
    assert!(r1.server_message_id.is_some());
    assert_eq!(r2.status, OutboxStatus::Delivered);
    assert!(r2.server_message_id.is_some());

    // Calling flush_due again does NOT duplicate already-Delivered messages
    let count_before = delivered_nonces.lock().len();
    let _ = rt.block_on(client2.outbox.flush_due());
    assert_eq!(delivered_nonces.lock().len(), count_before);

    // Clear on account scope change wipes outbox
    client2.local_store.clear_all();
    assert!(client2.outbox.store().get(high_nonce).unwrap().is_none());
}

#[test]
fn test_f7_chunked_transfer_seek_sha256_resume_and_404_reinit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rt = tokio::runtime::Runtime::new().expect("rt");

    // Create 150 KB file on disk (with part_size 32 KB -> 5 parts)
    let file_path = dir.path().join("large_archive.bin");
    let raw_bytes: Vec<u8> = (0..(150 * 1024)).map(|i| ((i * 31 + 7) % 256) as u8).collect();
    std::fs::write(&file_path, &raw_bytes).unwrap();

    // Verify disk seek & per-chunk SHA-256 indexing without loading entire file
    let (total_size, part_size, total_parts, full_sha256, chunk_hashes) =
        index_file_chunks(&file_path, Some(32 * 1024)).unwrap();
    assert_eq!(total_size, raw_bytes.len() as u64);
    assert_eq!(part_size, 32 * 1024);
    assert_eq!(total_parts, 5);
    assert_eq!(full_sha256, sha256_hex_local(&raw_bytes));
    let p2_bytes = read_chunk_at(&file_path, 2, part_size, total_size).unwrap();
    assert_eq!(p2_bytes, &raw_bytes[(64 * 1024)..(96 * 1024)]);
    assert_eq!(sha256_hex_local(&p2_bytes), chunk_hashes[2]);

    // Mock server tracks uploaded parts per upload_id; simulates interruption on first run
    struct UploadSrvState {
        init_count: usize,
        uploaded_parts: HashMap<String, HashSet<u32>>,
        part_upload_calls: Vec<(String, u32)>,
        fail_part2_remaining: usize,
    }
    let srv_state = Arc::new(Mutex::new(UploadSrvState {
        init_count: 0,
        uploaded_parts: HashMap::new(),
        part_upload_calls: Vec::new(),
        fail_part2_remaining: 3,
    }));

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    let srv_ref = Arc::clone(&srv_state);
    let total_size_srv = total_size;
    let full_sha_srv = full_sha256.clone();

    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let st = Arc::clone(&srv_ref);
            let fsha = full_sha_srv.clone();
            tokio::spawn(async move {
                let (method, path, _, body) = read_http_request(&mut stream).await;
                if method == "POST" && path == "/api/v1/media/upload/init" {
                    let resp_str = {
                        let mut guard = st.lock();
                        guard.init_count += 1;
                        let uid = format!("up_{}", guard.init_count);
                        guard.uploaded_parts.insert(uid.clone(), HashSet::new());
                        json!({
                            "upload_id": uid,
                            "chunk_size": 32 * 1024,
                            "expires_at_ms": 1800000000000i64
                        })
                        .to_string()
                    };
                    write_json_response(&mut stream, 200, "OK", &resp_str).await;
                } else if method == "GET"
                    && path.starts_with("/api/v1/media/upload/")
                    && path.ends_with("/status")
                {
                    let uid = path
                        .trim_start_matches("/api/v1/media/upload/")
                        .trim_end_matches("/status")
                        .to_string();
                    let status_resp = {
                        let guard = st.lock();
                        guard.uploaded_parts.get(&uid).map(|parts| {
                            let uploaded: Vec<bool> =
                                (0..5u32).map(|p| parts.contains(&p)).collect();
                            let missing: Vec<u32> = (0..5u32).filter(|p| !parts.contains(p)).collect();
                            json!({
                                "upload_id": uid,
                                "total_size": total_size_srv,
                                "part_size": 32 * 1024,
                                "total_parts": 5,
                                "uploaded_parts": uploaded,
                                "missing_parts": missing,
                                "is_complete": missing.is_empty()
                            })
                            .to_string()
                        })
                    };
                    if let Some(resp_str) = status_resp {
                        write_json_response(&mut stream, 200, "OK", &resp_str).await;
                    } else {
                        write_json_response(&mut stream, 404, "Not Found", r#"{"error":"expired"}"#)
                            .await;
                    }
                } else if method == "PUT" && path.starts_with("/api/v1/media/upload/") {
                    let rest = path.trim_start_matches("/api/v1/media/upload/");
                    let segs: Vec<&str> = rest.split('/').collect();
                    let uid = segs[0].to_string();
                    let part_idx: u32 = segs.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let should_fail = {
                        let mut guard = st.lock();
                        guard.part_upload_calls.push((uid.clone(), part_idx));
                        if part_idx == 2 && guard.fail_part2_remaining > 0 {
                            guard.fail_part2_remaining -= 1;
                            true
                        } else {
                            guard.uploaded_parts.entry(uid).or_default().insert(part_idx);
                            false
                        }
                    };
                    if should_fail {
                        write_json_response(
                            &mut stream,
                            500,
                            "Internal Error",
                            r#"{"error":"dropped"}"#,
                        )
                        .await;
                        return;
                    }
                    let _ = body;
                    write_json_response(&mut stream, 200, "OK", r#"{"ok":true}"#).await;
                } else if method == "POST" && path.ends_with("/complete") {
                    let uid = path
                        .trim_start_matches("/api/v1/media/upload/")
                        .trim_end_matches("/complete");
                    let resp = json!({
                        "file_id": format!("file_{uid}"),
                        "sha256": fsha,
                        "size_bytes": 150 * 1024,
                        "mime_type": "application/octet-stream"
                    });
                    write_json_response(&mut stream, 200, "OK", &resp.to_string()).await;
                } else {
                    write_json_response(&mut stream, 404, "Not Found", "{}").await;
                }
            });
        }
    });

    let cfg = ClientConfig {
        server_url: format!("http://{addr}"),
        ..Default::default()
    };
    let client = TelegramClient::open_with_dir(cfg.clone(), dir.path()).unwrap();
    client.set_auth_session(make_test_auth(101, "bob", "tok_b"));

    // First attempt fails on part 2 -> parts 0 and 1 are persisted in `file_transfers`
    let first = rt.block_on(client.upload_file_resumable(
        &file_path,
        "application/octet-stream",
        Some(10),
        Some("xfer_1"),
    ));
    assert!(first.is_err(), "first run should fail on injected part 2 error");

    let rec_mid = client.chunked_transfers.store().get("xfer_1").unwrap().unwrap();
    assert_eq!(rec_mid.status, TransferStatus::Paused);
    assert_eq!(rec_mid.uploaded_parts, vec![true, true, false, false, false]);
    assert_eq!(rec_mid.chunk_hashes, chunk_hashes);

    // Reopen client & resume same transfer -> queries `/status`, skips parts 0 & 1, uploads only 2,3,4
    let calls_before_resume = srv_state.lock().part_upload_calls.len();
    let client2 = TelegramClient::open_with_dir(cfg.clone(), dir.path()).unwrap();
    client2.set_auth_session(make_test_auth(101, "bob", "tok_b"));
    let second = rt
        .block_on(client2.upload_file_resumable(
            &file_path,
            "application/octet-stream",
            Some(10),
            Some("xfer_1"),
        ))
        .expect("resume must complete");
    assert_eq!(second.file_id, "file_up_1");

    let resumed_parts: Vec<u32> = srv_state.lock().part_upload_calls[calls_before_resume..]
        .iter()
        .map(|(_, p)| *p)
        .collect();
    assert_eq!(
        resumed_parts,
        vec![2, 3, 4],
        "must only upload missing parts 2,3,4"
    );

    // Simulate expired server session (404 on /status) -> auto re-init and complete
    let rec_expired = FileTransferRecord {
        transfer_id: "xfer_404".to_string(),
        upload_id: Some("unknown_expired_id".to_string()),
        status: TransferStatus::Paused,
        ..rec_mid
    };
    client2
        .chunked_transfers
        .store()
        .insert(&rec_expired)
        .unwrap();
    let reinit_res = rt
        .block_on(client2.upload_file_resumable(
            &file_path,
            "application/octet-stream",
            Some(10),
            Some("xfer_404"),
        ))
        .expect("404 on status must re-init upload");
    assert_eq!(reinit_res.file_id, "file_up_2");
}

#[test]
fn test_f8_e2ee_x3dh_double_ratchet_multi_device_and_empty_plaintext() {
    // 1. Direct X3DH + Double Ratchet edge cases (empty plaintext, out-of-order, replay reject)
    let alice_id = DeviceIdentityKeys::generate().unwrap();
    let bob_id = DeviceIdentityKeys::generate().unwrap();
    let bob_spk_sec = random_32().unwrap();
    let bob_spk_pub = x25519_pub_from_secret(&bob_spk_sec);
    let spk_sig = bob_id.sign_signed_prekey(&bob_spk_pub);
    assert!(gpui_telegram_client::verify_signed_prekey(
        &bob_id.ed25519_pub,
        &bob_spk_pub,
        &spk_sig
    )
    .is_ok());
    let bob_otpk_sec = random_32().unwrap();
    let bob_otpk_pub = x25519_pub_from_secret(&bob_otpk_sec);

    let bundle = DevicePreKeyBundle {
        user_id: 2,
        device_id: "bob_1".into(),
        device_name: Some("Bob".into()),
        identity_key: b64_encode(&bob_id.x25519_pub),
        identity_signing_key: b64_encode(&bob_id.ed25519_pub),
        signed_prekey: SignedPreKeyDto {
            key_id: 1,
            public_key: b64_encode(&bob_spk_pub),
            signature: b64_encode(&spk_sig),
            created_at_ms: 1_700_000_000_000,
        },
        one_time_prekey: Some(OneTimePreKeyDto {
            key_id: 10,
            public_key: b64_encode(&bob_otpk_pub),
        }),
    };

    let ek_a_sec = random_32().unwrap();
    let ek_a_pub = x25519_pub_from_secret(&ek_a_sec);
    let init = x3dh_initiator(&alice_id.x25519_sec, &ek_a_sec, &bundle).unwrap();
    let resp = x3dh_responder(
        &bob_id.x25519_sec,
        &bob_spk_sec,
        Some(&bob_otpk_sec),
        &alice_id.x25519_pub,
        &ek_a_pub,
    )
    .unwrap();
    assert_eq!(*init.shared_secret, *resp.shared_secret);
    assert_eq!(init.associated_data, resp.associated_data);

    let mut alice_dr = DoubleRatchetSession::init_alice(
        *init.shared_secret,
        bob_spk_pub,
        init.associated_data.clone(),
    )
    .unwrap();
    let mut bob_dr = DoubleRatchetSession::init_bob(
        *resp.shared_secret,
        bob_spk_sec,
        resp.associated_data,
    );

    // Empty plaintext & out-of-order decryption
    let (h0, c0) = alice_dr.encrypt(b"").unwrap();
    let (h1, c1) = alice_dr.encrypt(b"msg-1").unwrap();
    let (h2, c2) = alice_dr.encrypt(b"msg-2").unwrap();
    assert_eq!(bob_dr.decrypt(&h2, &c2).unwrap(), b"msg-2");
    assert_eq!(bob_dr.decrypt(&h0, &c0).unwrap(), b"");
    assert_eq!(bob_dr.decrypt(&h1, &c1).unwrap(), b"msg-1");
    assert!(bob_dr.decrypt(&h1, &c1).is_err(), "replay must fail");

    // 2. Multi-device E2eeManager flow across 2 Alice devices (A1, A2) and 2 Bob devices (B1, B2)
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let prekey_store: Arc<Mutex<HashMap<(u64, String), PublishPreKeysRequest>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let srv_store = Arc::clone(&prekey_store);

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let store = Arc::clone(&srv_store);
            tokio::spawn(async move {
                let (method, path, headers, body) = read_http_request(&mut stream).await;
                let uid: u64 = headers
                    .lines()
                    .find_map(|l| {
                        let lower = l.to_ascii_lowercase();
                        lower.strip_prefix("authorization:").and_then(|v| {
                            v.trim().strip_prefix("bearer tok_").and_then(|s| s.parse().ok())
                        })
                    })
                    .unwrap_or(0);
                if method == "POST" && path == "/api/v1/e2ee/prekeys" {
                    let req: PublishPreKeysRequest = serde_json::from_slice(&body).unwrap();
                    let count = req.one_time_prekeys.len();
                    let dev = req.device_id.clone();
                    store.lock().insert((uid, dev.clone()), req);
                    let resp = json!({
                        "success": true,
                        "user_id": uid,
                        "device_id": dev,
                        "remaining_one_time_prekeys": count,
                        "updated_at_ms": 1700000000000i64
                    });
                    write_json_response(&mut stream, 200, "OK", &resp.to_string()).await;
                } else if method == "GET" && path.starts_with("/api/v1/e2ee/devices/") {
                    let target_uid: u64 = path
                        .trim_start_matches("/api/v1/e2ee/devices/")
                        .trim_end_matches("/prekeys")
                        .parse()
                        .unwrap_or(0);
                    let resp_str = {
                        let mut guard = store.lock();
                        let mut devices = Vec::new();
                        for ((u, dev), req) in guard.iter_mut() {
                            if *u == target_uid {
                                let otpk = if req.one_time_prekeys.is_empty() {
                                    None
                                } else {
                                    Some(req.one_time_prekeys.remove(0))
                                };
                                devices.push(DevicePreKeyBundle {
                                    user_id: target_uid,
                                    device_id: dev.clone(),
                                    device_name: req.device_name.clone(),
                                    identity_key: req.identity_key.clone(),
                                    identity_signing_key: req.identity_signing_key.clone(),
                                    signed_prekey: req.signed_prekey.clone(),
                                    one_time_prekey: otpk,
                                });
                            }
                        }
                        let resp = UserPreKeyBundlesResponse {
                            user_id: target_uid,
                            devices,
                        };
                        serde_json::to_string(&resp).unwrap()
                    };
                    write_json_response(&mut stream, 200, "OK", &resp_str).await;
                } else {
                    write_json_response(&mut stream, 404, "Not Found", "{}").await;
                }
            });
        }
    });

    let cfg = ClientConfig {
        server_url: format!("http://{addr}"),
        ..Default::default()
    };
    let d_a1 = tempfile::tempdir().unwrap();
    let d_a2 = tempfile::tempdir().unwrap();
    let d_b1 = tempfile::tempdir().unwrap();
    let d_b2 = tempfile::tempdir().unwrap();

    let c_a1 = TelegramClient::open_with_dir(cfg.clone(), d_a1.path()).unwrap();
    c_a1.set_device_id("dev_a1");
    c_a1.set_auth_session(make_test_auth(1, "alice", "tok_1"));

    let c_a2 = TelegramClient::open_with_dir(cfg.clone(), d_a2.path()).unwrap();
    c_a2.set_device_id("dev_a2");
    c_a2.set_auth_session(make_test_auth(1, "alice", "tok_1"));

    let c_b1 = TelegramClient::open_with_dir(cfg.clone(), d_b1.path()).unwrap();
    c_b1.set_device_id("dev_b1");
    c_b1.set_auth_session(make_test_auth(2, "bob", "tok_2"));

    let c_b2 = TelegramClient::open_with_dir(cfg.clone(), d_b2.path()).unwrap();
    c_b2.set_device_id("dev_b2");
    c_b2.set_auth_session(make_test_auth(2, "bob", "tok_2"));

    for c in [&c_a1, &c_a2, &c_b1, &c_b2] {
        rt.block_on(c.e2ee_init_and_publish_prekeys(5)).unwrap();
    }

    // A1 encrypts DM to Bob -> fans out to B1, B2 AND sender's own other device A2
    let env = rt.block_on(c_a1.e2ee_encrypt_dm(2, b"multi-device secret")).unwrap();
    assert_eq!(c_b1.e2ee_decrypt_envelope(&env).unwrap(), b"multi-device secret");
    assert_eq!(c_b2.e2ee_decrypt_envelope(&env).unwrap(), b"multi-device secret");
    assert_eq!(c_a2.e2ee_decrypt_envelope(&env).unwrap(), b"multi-device secret");

    // Group Sender Keys across A1, B1, B2
    let (dist_opt, grp_env) = rt
        .block_on(c_a1.e2ee_encrypt_group(777, &[1, 2], b"group hello"))
        .unwrap();
    let dist_env = dist_opt.expect("first group send distributes sender key");
    assert_eq!(c_b1.e2ee_decrypt_envelope(&dist_env).unwrap(), b"");
    assert_eq!(c_b2.e2ee_decrypt_envelope(&dist_env).unwrap(), b"");
    assert_eq!(c_b1.e2ee_decrypt_envelope(&grp_env).unwrap(), b"group hello");
    assert_eq!(c_b2.e2ee_decrypt_envelope(&grp_env).unwrap(), b"group hello");

    // Persistence across restart for B1
    drop(c_b1);
    let c_b1_reloaded = TelegramClient::open_with_dir(cfg, d_b1.path()).unwrap();
    c_b1_reloaded.set_device_id("dev_b1");
    c_b1_reloaded.set_auth_session(make_test_auth(2, "bob", "tok_2"));
    let (_, grp_env2) = rt
        .block_on(c_a1.e2ee_encrypt_group(777, &[1, 2], b"after restart"))
        .unwrap();
    assert_eq!(
        c_b1_reloaded.e2ee_decrypt_envelope(&grp_env2).unwrap(),
        b"after restart"
    );

    // Direct SenderKeyState unit sanity
    let (sk, dist_msg) = SenderKeyState::create_own(99, 1, "d1".into()).unwrap();
    assert_eq!(sk.chat_id, 99);
    assert_eq!(dist_msg.chat_id, 99);
}

#[test]
fn test_f9_proxy_http_connect_and_socks5_tunneling_http_and_ws() {
    let rt = tokio::runtime::Runtime::new().expect("rt");

    // 1. Backend HTTP + WebSocket server
    let backend_listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let backend_addr = backend_listener.local_addr().unwrap();
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = backend_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut peek_buf = [0u8; 2048];
                let n = stream.peek(&mut peek_buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&peek_buf[..n]);
                if head.to_ascii_lowercase().contains("upgrade: websocket") {
                    if let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await {
                        let ack = WsServerFrame::Ack {
                            client_nonce: 9001,
                            message_id: 55,
                            pts: 7,
                            correlation_id: 0,
                        };
                        let _ = ws
                            .send(WsMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                            .await;
                        while let Some(Ok(msg)) = ws.next().await {
                            if let WsMessage::Text(t) = msg {
                                if let Ok(WsClientFrame::Ping { nonce }) = serde_json::from_str(&t) {
                                    let pong = WsServerFrame::Pong { nonce };
                                    let _ = ws
                                        .send(WsMessage::Text(
                                            serde_json::to_string(&pong).unwrap().into(),
                                        ))
                                        .await;
                                }
                            }
                        }
                    }
                    return;
                }
                let (_m, path, _, _) = read_http_request(&mut stream).await;
                if path == "/health" {
                    write_json_response(&mut stream, 200, "OK", r#"{"ok":true}"#).await;
                } else {
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        r#"{"status":"healthy","version":"m2"}"#,
                    )
                    .await;
                }
            });
        }
    });

    // 2. Local HTTP CONNECT Proxy
    let http_proxy_hits = Arc::new(AtomicUsize::new(0));
    let http_proxy_listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let http_proxy_addr = http_proxy_listener.local_addr().unwrap();
    let http_hits_srv = Arc::clone(&http_proxy_hits);
    rt.spawn(async move {
        loop {
            let Ok((mut client_stream, _)) = http_proxy_listener.accept().await else {
                break;
            };
            let hits = Arc::clone(&http_hits_srv);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    let n = client_stream.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let req_str = String::from_utf8_lossy(&buf);
                let first_line = req_str.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("");
                let target = parts.next().unwrap_or("");
                hits.fetch_add(1, Ordering::SeqCst);
                if method.eq_ignore_ascii_case("CONNECT") {
                    if let Ok(mut remote) = TcpStream::connect(target).await {
                        let _ = client_stream
                            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                            .await;
                        let _ = tokio::io::copy_bidirectional(&mut client_stream, &mut remote).await;
                    }
                } else if let Ok(url) = reqwest::Url::parse(target) {
                    let host_port = format!(
                        "{}:{}",
                        url.host_str().unwrap(),
                        url.port_or_known_default().unwrap()
                    );
                    if let Ok(mut remote) = TcpStream::connect(&host_port).await {
                        let path_q = if let Some(q) = url.query() {
                            format!("{}?{}", url.path(), q)
                        } else {
                            url.path().to_string()
                        };
                        let rebuilt = req_str.replacen(target, &path_q, 1);
                        let _ = remote.write_all(rebuilt.as_bytes()).await;
                        let _ = tokio::io::copy_bidirectional(&mut client_stream, &mut remote).await;
                    }
                }
            });
        }
    });

    // 3. Local SOCKS5 Proxy (supports username/password auth)
    let socks_proxy_hits = Arc::new(AtomicUsize::new(0));
    let socks_listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let socks_addr = socks_listener.local_addr().unwrap();
    let socks_hits_srv = Arc::clone(&socks_proxy_hits);
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = socks_listener.accept().await else {
                break;
            };
            let hits = Arc::clone(&socks_hits_srv);
            tokio::spawn(async move {
                let mut hdr = [0u8; 2];
                if stream.read_exact(&mut hdr).await.is_err() || hdr[0] != 0x05 {
                    return;
                }
                let mut methods = vec![0u8; hdr[1] as usize];
                if stream.read_exact(&mut methods).await.is_err() {
                    return;
                }
                if methods.contains(&0x02) {
                    let _ = stream.write_all(&[0x05, 0x02]).await;
                    let mut auth_ver = [0u8; 2];
                    if stream.read_exact(&mut auth_ver).await.is_err() {
                        return;
                    }
                    let mut u = vec![0u8; auth_ver[1] as usize];
                    let _ = stream.read_exact(&mut u).await;
                    let mut plen = [0u8; 1];
                    let _ = stream.read_exact(&mut plen).await;
                    let mut p = vec![0u8; plen[0] as usize];
                    let _ = stream.read_exact(&mut p).await;
                    if u == b"alice" && p == b"secret" {
                        let _ = stream.write_all(&[0x01, 0x00]).await;
                    } else {
                        let _ = stream.write_all(&[0x01, 0x01]).await;
                        return;
                    }
                } else {
                    let _ = stream.write_all(&[0x05, 0x00]).await;
                }

                let mut req_hdr = [0u8; 4];
                if stream.read_exact(&mut req_hdr).await.is_err() {
                    return;
                }
                let dest = match req_hdr[3] {
                    0x01 => {
                        let mut ip = [0u8; 4];
                        let _ = stream.read_exact(&mut ip).await;
                        let mut port = [0u8; 2];
                        let _ = stream.read_exact(&mut port).await;
                        format!(
                            "{}.{}.{}.{}:{}",
                            ip[0],
                            ip[1],
                            ip[2],
                            ip[3],
                            u16::from_be_bytes(port)
                        )
                    }
                    0x03 => {
                        let mut len = [0u8; 1];
                        let _ = stream.read_exact(&mut len).await;
                        let mut host = vec![0u8; len[0] as usize];
                        let _ = stream.read_exact(&mut host).await;
                        let mut port = [0u8; 2];
                        let _ = stream.read_exact(&mut port).await;
                        format!(
                            "{}:{}",
                            String::from_utf8_lossy(&host),
                            u16::from_be_bytes(port)
                        )
                    }
                    _ => return,
                };
                hits.fetch_add(1, Ordering::SeqCst);
                if let Ok(mut remote) = TcpStream::connect(&dest).await {
                    let _ = stream
                        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                        .await;
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
                }
            });
        }
    });

    let base_url = format!("http://{backend_addr}");
    let http_client = HttpClient::new_with_proxy(
        base_url.clone(),
        Duration::from_secs(5),
        Some(rt.handle().clone()),
        None,
    );

    // Test HTTP CONNECT Proxy: HTTP + probe + WS
    let http_proxy = ClientProxyConfig::http("127.0.0.1", http_proxy_addr.port());
    http_client.set_proxy(Some(http_proxy.clone())).unwrap();
    let http_probe = rt.block_on(http_client.probe_proxy(&http_proxy)).unwrap();
    assert!(http_probe < 5000);
    assert!(rt.block_on(http_client.check_health()).unwrap());
    assert!(http_proxy_hits.load(Ordering::SeqCst) >= 2);

    let ws_chan = WsChannel::new();
    let mut rx_http = ws_chan.subscribe();
    let ws_http = WsTransport::new_with_runtime(base_url.clone(), ws_chan.clone(), Some(rt.handle().clone()))
        .with_proxy(Some(http_proxy));
    ws_http.set_token(Some("tok_ws".into()));
    ws_http.start();
    let frame1 = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), rx_http.recv())
            .await
            .unwrap()
            .unwrap()
    });
    assert!(matches!(frame1, WsServerFrame::Ack { client_nonce: 9001, .. }));
    ws_http.stop();

    // Test SOCKS5 Proxy with auth: HTTP + probe + WS
    let socks_proxy = ClientProxyConfig::socks5("127.0.0.1", socks_addr.port())
        .with_auth("alice", "secret");
    http_client.set_proxy(Some(socks_proxy.clone())).unwrap();
    let socks_probe = rt.block_on(http_client.probe_proxy(&socks_proxy)).unwrap();
    assert!(socks_probe < 5000);
    assert!(rt.block_on(http_client.check_health()).unwrap());
    assert!(socks_proxy_hits.load(Ordering::SeqCst) >= 2);

    let ws_chan2 = WsChannel::new();
    let mut rx_socks = ws_chan2.subscribe();
    let ws_socks = WsTransport::new_with_runtime(base_url, ws_chan2, Some(rt.handle().clone()))
        .with_proxy(Some(socks_proxy));
    ws_socks.set_token(Some("tok_ws2".into()));
    ws_socks.start();
    let frame2 = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), rx_socks.recv())
            .await
            .unwrap()
            .unwrap()
    });
    assert!(matches!(frame2, WsServerFrame::Ack { client_nonce: 9001, .. }));
    ws_socks.stop();
}

#[test]
fn test_f10_save_draft_unwrap_folder_cache_and_ttl_purge() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rt = tokio::runtime::Runtime::new().expect("rt");

    // 1. Verify HttpClient::save_draft unwraps `{"draft": {...}}` server envelope
    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (method, path, _, body) = read_http_request(&mut stream).await;
                if method == "PUT" && path == "/api/v1/chats/77/draft" {
                    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let resp = json!({ "draft": parsed });
                    write_json_response(&mut stream, 200, "OK", &resp.to_string()).await;
                } else {
                    write_json_response(&mut stream, 404, "Not Found", "{}").await;
                }
            });
        }
    });

    let http = HttpClient::new_with_proxy(
        format!("http://{addr}"),
        Duration::from_secs(5),
        Some(rt.handle().clone()),
        None,
    );
    let draft = gpui_telegram_proto::contracts::DraftDto {
        chat_id: 77,
        text: "unsent draft text".into(),
        entities: Vec::new(),
        reply_to_msg_id: None,
        topic_id: None,
        date_ms: 1_700_000_000_000,
    };
    let saved = rt.block_on(http.save_draft("tok", 77, &draft)).unwrap();
    assert_eq!(saved.text, "unsent draft text");
    assert_eq!(saved.chat_id, 77);

    // 2. Verify LocalStore folder caching + delta upsert/remove + TTL purge
    let store = LocalStore::open_with_dir(dir.path()).unwrap();
    let folder = ChatFolderDto {
        id: 5,
        title: "Work".into(),
        emoticon: Some("💼".into()),
        include_chats: vec![77],
        exclude_chats: vec![],
        pinned_chats: vec![77],
        include_contacts: true,
        include_non_contacts: false,
        include_groups: true,
        include_channels: false,
        include_bots: false,
        exclude_muted: false,
        exclude_read: false,
        exclude_archived: false,
        sort_order: 0,
    };
    store.upsert_folder(folder);
    assert_eq!(store.get_folders().len(), 1);
    assert_eq!(store.get_folders()[0].title, "Work");
    store.remove_folder(5);
    assert!(store.get_folders().is_empty());

    // Insert 1 expired TTL message and 1 non-expired message
    let mut expired = MessageItem::new(
        MessageId(101),
        ChatId(77),
        UserId(1),
        PeerId::Chat(ChatId(77)),
        MessageContent::plain_text("burn after reading"),
    );
    expired.ttl_seconds = Some(5);
    expired.expires_at_ms = Some(1_000);

    let mut alive = MessageItem::new(
        MessageId(102),
        ChatId(77),
        UserId(1),
        PeerId::Chat(ChatId(77)),
        MessageContent::plain_text("stay alive"),
    );
    alive.ttl_seconds = Some(60);
    alive.expires_at_ms = Some(50_000);

    store.insert_message(expired);
    store.insert_message(alive);
    let purged = store.purge_expired_messages(10_000);
    assert_eq!(purged, vec![(ChatId(77), vec![MessageId(101)])]);
    let remaining = store.get_messages(ChatId(77)).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, MessageId(102));
}

#[test]
fn test_regression_issue1_sender_key_distribution_spoofing_and_message_sender_mismatch_rejected() {
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let prekey_store: Arc<Mutex<HashMap<(u64, String), PublishPreKeysRequest>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let srv_store = Arc::clone(&prekey_store);

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let store = Arc::clone(&srv_store);
            tokio::spawn(async move {
                let (method, path, headers, body) = read_http_request(&mut stream).await;
                let uid: u64 = headers
                    .lines()
                    .find_map(|l| {
                        let lower = l.to_ascii_lowercase();
                        lower.strip_prefix("authorization:").and_then(|v| {
                            v.trim().strip_prefix("bearer tok_").and_then(|s| s.parse().ok())
                        })
                    })
                    .unwrap_or(0);
                if method == "POST" && path == "/api/v1/e2ee/prekeys" {
                    let req: PublishPreKeysRequest = serde_json::from_slice(&body).unwrap();
                    let count = req.one_time_prekeys.len();
                    let dev = req.device_id.clone();
                    store.lock().insert((uid, dev.clone()), req);
                    let resp = json!({
                        "success": true,
                        "user_id": uid,
                        "device_id": dev,
                        "remaining_one_time_prekeys": count,
                        "updated_at_ms": 1700000000000i64
                    });
                    write_json_response(&mut stream, 200, "OK", &resp.to_string()).await;
                } else if method == "GET" && path.starts_with("/api/v1/e2ee/devices/") {
                    let target_uid: u64 = path
                        .trim_start_matches("/api/v1/e2ee/devices/")
                        .trim_end_matches("/prekeys")
                        .parse()
                        .unwrap_or(0);
                    let resp_str = {
                        let mut guard = store.lock();
                        let mut devices = Vec::new();
                        for ((u, dev), req) in guard.iter_mut() {
                            if *u == target_uid {
                                let otpk = if req.one_time_prekeys.is_empty() {
                                    None
                                } else {
                                    Some(req.one_time_prekeys.remove(0))
                                };
                                devices.push(DevicePreKeyBundle {
                                    user_id: target_uid,
                                    device_id: dev.clone(),
                                    device_name: req.device_name.clone(),
                                    identity_key: req.identity_key.clone(),
                                    identity_signing_key: req.identity_signing_key.clone(),
                                    signed_prekey: req.signed_prekey.clone(),
                                    one_time_prekey: otpk,
                                });
                            }
                        }
                        serde_json::to_string(&UserPreKeyBundlesResponse {
                            user_id: target_uid,
                            devices,
                        })
                        .unwrap()
                    };
                    write_json_response(&mut stream, 200, "OK", &resp_str).await;
                } else {
                    write_json_response(&mut stream, 404, "Not Found", "{}").await;
                }
            });
        }
    });

    let cfg = ClientConfig {
        server_url: format!("http://{addr}"),
        ..Default::default()
    };
    let d_a = tempfile::tempdir().unwrap();
    let d_b = tempfile::tempdir().unwrap();
    let d_c = tempfile::tempdir().unwrap();

    let c_a = TelegramClient::open_with_dir(cfg.clone(), d_a.path()).unwrap();
    c_a.set_device_id("dev_a1");
    c_a.set_auth_session(make_test_auth(1, "alice", "tok_1"));

    let c_b = TelegramClient::open_with_dir(cfg.clone(), d_b.path()).unwrap();
    c_b.set_device_id("dev_b1");
    c_b.set_auth_session(make_test_auth(2, "bob", "tok_2"));

    let c_c = TelegramClient::open_with_dir(cfg, d_c.path()).unwrap();
    c_c.set_device_id("dev_c1");
    c_c.set_auth_session(make_test_auth(3, "charlie", "tok_3"));

    for c in [&c_a, &c_b, &c_c] {
        rt.block_on(c.e2ee_init_and_publish_prekeys(5)).unwrap();
    }

    // 1. Alice distributes her legitimate group SenderKey to Bob & Charlie
    let (dist_opt, grp1) = rt
        .block_on(c_a.e2ee_encrypt_group(888, &[1, 2, 3], b"legit alice 1"))
        .unwrap();
    let dist_env = dist_opt.unwrap();
    assert_eq!(c_b.e2ee_decrypt_envelope(&dist_env).unwrap(), b"");
    assert_eq!(c_b.e2ee_decrypt_envelope(&grp1).unwrap(), b"legit alice 1");

    // 2. Charlie establishes a pairwise DR session to Bob, then crafts a spoofed SenderKeyDistributionMessage
    // claiming sender_user_id=1 (Alice) and sender_device_id="dev_a1".
    let _ = rt
        .block_on(c_c.e2ee_encrypt_dm(2, b"charlie setup"))
        .unwrap();
    let (mut fake_alice_sk, mut spoof_dist) =
        SenderKeyState::create_own(888, 1, "dev_a1".to_string()).unwrap();
    spoof_dist.sender_user_id = 1;
    spoof_dist.sender_device_id = "dev_a1".to_string();
    let spoof_bytes = serde_json::to_vec(&spoof_dist).unwrap();
    let spoof_env = c_c
        .e2ee_manager()
        .encrypt_pairwise_for_targets(&[(2, "dev_b1".to_string())], &spoof_bytes, true)
        .unwrap();

    // Bob MUST reject the spoofed SenderKeyDistribution because envelope sender is 3:dev_c1 != 1:dev_a1
    let spoof_res = c_b.e2ee_decrypt_envelope(&spoof_env);
    assert!(
        spoof_res.is_err(),
        "Spoofed SenderKeyDistribution must be rejected"
    );

    // Forged GroupSenderKey signed by fake_alice_sk must also fail, while Alice's authentic key remains intact
    let forged_ct = fake_alice_sk.encrypt_group_message(b"IMPERSONATED").unwrap();
    let c_id = c_a.e2ee_manager().identity().unwrap();
    let forged_env = EncryptedEnvelope {
        version: 1,
        sender_user_id: 1,
        sender_device_id: "dev_a1".to_string(),
        sender_identity_key: b64_encode(&c_id.x25519_pub),
        sender_signing_key: b64_encode(&c_id.ed25519_pub),
        kind: EncryptedPayloadKind::GroupSenderKey(forged_ct),
    };
    assert!(c_b.e2ee_decrypt_envelope(&forged_env).is_err());

    let (_, grp2) = rt
        .block_on(c_a.e2ee_encrypt_group(888, &[1, 2, 3], b"legit alice 2"))
        .unwrap();
    assert_eq!(c_b.e2ee_decrypt_envelope(&grp2).unwrap(), b"legit alice 2");

    // 3. try_decrypt_message_item must reject MessageItem whose sender_id != env.sender_user_id
    let dm_env = rt.block_on(c_a.e2ee_encrypt_dm(2, b"authentic dm")).unwrap();
    let mismatched_item = MessageItem::new(
        MessageId(500),
        ChatId(99),
        UserId(3),
        PeerId::User(UserId(2)),
        MessageContent::Encrypted(dm_env.clone()),
    );
    assert_eq!(
        c_b.e2ee_manager().try_decrypt_message_item(&mismatched_item),
        None
    );
    let authentic_item = MessageItem::new(
        MessageId(501),
        ChatId(99),
        UserId(1),
        PeerId::User(UserId(2)),
        MessageContent::Encrypted(dm_env),
    );
    assert_eq!(
        c_b.e2ee_manager()
            .try_decrypt_message_item(&authentic_item)
            .as_deref(),
        Some("authentic dm")
    );
}

#[test]
fn test_regression_issue2_ws_transport_stop_start_proxy_hotswap_no_livelock() {
    let rt = tokio::runtime::Runtime::new().expect("rt");

    let backend_ws_count = Arc::new(AtomicUsize::new(0));
    let backend_listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let backend_addr = backend_listener.local_addr().unwrap();
    let ws_cnt_srv = Arc::clone(&backend_ws_count);
    rt.spawn(async move {
        loop {
            let Ok((stream, _)) = backend_listener.accept().await else {
                break;
            };
            let cnt = Arc::clone(&ws_cnt_srv);
            tokio::spawn(async move {
                if let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await {
                    let conn_num = cnt.fetch_add(1, Ordering::SeqCst) + 1;
                    let ack = WsServerFrame::Ack {
                        correlation_id: 0,
                        client_nonce: conn_num as u64,
                        message_id: conn_num as i64,
                        pts: 1,
                    };
                    let _ = ws
                        .send(WsMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                        .await;
                    while let Some(Ok(msg)) = ws.next().await {
                        if let WsMessage::Text(t) = msg {
                            if let Ok(WsClientFrame::Ping { nonce }) = serde_json::from_str(&t) {
                                let pong = WsServerFrame::Pong { nonce };
                                let _ = ws
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&pong).unwrap().into(),
                                    ))
                                    .await;
                            }
                        }
                    }
                }
            });
        }
    });

    // SOCKS5 proxy
    let socks_hits = Arc::new(AtomicUsize::new(0));
    let socks_listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let socks_addr = socks_listener.local_addr().unwrap();
    let socks_hits_srv = Arc::clone(&socks_hits);
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = socks_listener.accept().await else {
                break;
            };
            let hits = Arc::clone(&socks_hits_srv);
            tokio::spawn(async move {
                let mut hdr = [0u8; 2];
                if stream.read_exact(&mut hdr).await.is_err() || hdr[0] != 0x05 {
                    return;
                }
                let mut methods = vec![0u8; hdr[1] as usize];
                if stream.read_exact(&mut methods).await.is_err() {
                    return;
                }
                let _ = stream.write_all(&[0x05, 0x00]).await;
                let mut req_hdr = [0u8; 4];
                if stream.read_exact(&mut req_hdr).await.is_err() {
                    return;
                }
                let dest = match req_hdr[3] {
                    0x01 => {
                        let mut ip = [0u8; 4];
                        let _ = stream.read_exact(&mut ip).await;
                        let mut port = [0u8; 2];
                        let _ = stream.read_exact(&mut port).await;
                        format!(
                            "{}.{}.{}.{}:{}",
                            ip[0],
                            ip[1],
                            ip[2],
                            ip[3],
                            u16::from_be_bytes(port)
                        )
                    }
                    0x03 => {
                        let mut len = [0u8; 1];
                        let _ = stream.read_exact(&mut len).await;
                        let mut host = vec![0u8; len[0] as usize];
                        let _ = stream.read_exact(&mut host).await;
                        let mut port = [0u8; 2];
                        let _ = stream.read_exact(&mut port).await;
                        format!(
                            "{}:{}",
                            String::from_utf8_lossy(&host),
                            u16::from_be_bytes(port)
                        )
                    }
                    _ => return,
                };
                hits.fetch_add(1, Ordering::SeqCst);
                if let Ok(mut remote) = TcpStream::connect(&dest).await {
                    let _ = stream
                        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                        .await;
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
                }
            });
        }
    });

    let ws_chan = WsChannel::new();
    let mut rx = ws_chan.subscribe();
    let transport = WsTransport::new_with_runtime(
        format!("http://{backend_addr}"),
        ws_chan.clone(),
        Some(rt.handle().clone()),
    );
    transport.set_token(Some("tok_hotswap".into()));
    transport.start();

    let first = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap()
    });
    assert!(matches!(first, WsServerFrame::Ack { client_nonce: 1, .. }));
    assert!(transport.is_connected());

    // Immediately hot-swap proxy with stop()+start() back-to-back
    let socks_proxy = ClientProxyConfig::socks5("127.0.0.1", socks_addr.port());
    transport.set_proxy(Some(socks_proxy));
    transport.stop();
    transport.start();

    let second = rt.block_on(async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let rem = deadline.saturating_duration_since(tokio::time::Instant::now());
            let f = tokio::time::timeout(rem, rx.recv())
                .await
                .expect("must connect via proxy after stop()+start()")
                .unwrap();
            if matches!(f, WsServerFrame::Ack { client_nonce: 2, .. }) {
                break f;
            }
        }
    });
    assert!(matches!(second, WsServerFrame::Ack { client_nonce: 2, .. }));

    // Settle for 2.2s (> 1s reconnect ping-pong cadence) and confirm zero churn
    rt.block_on(async {
        tokio::time::sleep(Duration::from_millis(2200)).await;
    });
    assert!(
        transport.is_connected(),
        "Transport must remain Connected without live-lock, got {:?}",
        transport.status()
    );
    assert_eq!(socks_hits.load(Ordering::SeqCst), 1);
    assert_eq!(backend_ws_count.load(Ordering::SeqCst), 2);

    assert!(ws_chan.send(WsClientFrame::Ping { nonce: 777 }));
    let pong = rt.block_on(async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let rem = deadline.saturating_duration_since(tokio::time::Instant::now());
            let f = tokio::time::timeout(rem, rx.recv()).await.unwrap().unwrap();
            if matches!(f, WsServerFrame::Pong { nonce: 777 }) {
                break f;
            }
        }
    });
    assert!(matches!(pong, WsServerFrame::Pong { nonce: 777 }));
    transport.stop();
}

#[test]
fn test_regression_issue3_outbox_reconnect_during_inflight_flush_wakes_backed_off_item() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rt = tokio::runtime::Runtime::new().expect("rt");

    let inflight_started = Arc::new(tokio::sync::Notify::new());
    let inflight_started_srv = Arc::clone(&inflight_started);
    let delivered_nonces = Arc::new(Mutex::new(Vec::<u64>::new()));
    let nonces_srv = Arc::clone(&delivered_nonces);

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        let mut next_msg_id = 8000i64;
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let notify = Arc::clone(&inflight_started_srv);
            let nonces = Arc::clone(&nonces_srv);
            tokio::spawn(async move {
                let (method, path, _, body) = read_http_request(&mut stream).await;
                if method == "POST"
                    && (path == "/api/v1/messages/send" || path == "/api/v1/chats/42/messages")
                {
                    let parsed: serde_json::Value =
                        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                    let nonce = parsed["client_nonce"].as_u64().unwrap_or(0);
                    if nonce == 50_001 {
                        notify.notify_waiters();
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    next_msg_id += 1;
                    nonces.lock().push(nonce);
                    let resp = json!({
                        "message_id": next_msg_id,
                        "id": next_msg_id,
                        "pts": 20,
                        "pts_count": 1,
                        "duplicate": false,
                        "date_ms": 1700000000000i64,
                        "date": "2026-11-17T00:00:00Z",
                        "sender_id": 100,
                        "chat_id": 42,
                        "text": parsed["text"].as_str().unwrap_or(""),
                        "client_nonce": nonce,
                        "status": "sent"
                    });
                    write_json_response(&mut stream, 200, "OK", &resp.to_string()).await;
                } else {
                    write_json_response(&mut stream, 200, "OK", "{}").await;
                }
            });
        }
    });

    let cfg = ClientConfig {
        server_url: format!("http://{addr}"),
        ..Default::default()
    };
    let client = TelegramClient::open_with_dir(cfg, dir.path()).unwrap();
    client.set_auth_session(make_test_auth(100, "alice", "tok_race"));
    // `set_auth_session` starts the internal outbox loop; pause it while seeding 50_001/50_002
    // so the background 250ms loop cannot claim 50_001 prematurely before line 1550.
    client.outbox.stop();
    client.network_listener.set_online(true);

    let now_ms = chrono::Utc::now().timestamp_millis();
    client
        .outbox
        .store()
        .enqueue(&make_rich_req(42, "in flight", Some(50_001)), 50_001, now_ms)
        .unwrap();
    client
        .outbox
        .store()
        .enqueue(&make_rich_req(42, "backed off", Some(50_002)), 50_002, now_ms)
        .unwrap();
    // Put 50_002 into 25-second backoff
    let _ = client
        .outbox
        .store()
        .claim_next_due(now_ms)
        .unwrap();
    // Note: claim_next_due claimed 50_001 (lowest nonce); reset 50_001 back to Pending, and put 50_002 into 25s retry
    client.outbox.store().recover_in_flight(now_ms).unwrap();
    let conn_check = client.outbox.store().get(50_002).unwrap().unwrap();
    assert_eq!(conn_check.status, OutboxStatus::Pending);
    // Use claim + schedule_retry to place 50_002 in 25s backoff cleanly via public store API
    let c1 = client.outbox.store().claim_next_due(now_ms).unwrap().unwrap();
    assert_eq!(c1.client_nonce, 50_001);
    let c2 = client.outbox.store().claim_next_due(now_ms).unwrap().unwrap();
    assert_eq!(c2.client_nonce, 50_002);
    client
        .outbox
        .store()
        .schedule_retry(50_002, 4, now_ms + 25_000, "503 simulated", now_ms)
        .unwrap();
    client
        .outbox
        .store()
        .schedule_retry(50_001, 0, 0, "ready", now_ms)
        .unwrap();

    // Start background OutboxWorker and wait until 50_001 HTTP request is in-flight
    client.outbox.start(rt.handle());
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), inflight_started.notified())
            .await
            .expect("50_001 must enter in-flight HTTP call");
        // Flap Offline -> Online WHILE 50_001 is still awaiting the 300ms HTTP response
        client.network_listener.set_online(false);
        client.network_listener.set_online(true);

        let start = tokio::time::Instant::now();
        loop {
            let rec = client.outbox.store().get(50_002).unwrap().unwrap();
            if rec.status == OutboxStatus::Delivered {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_millis(1500),
                "50_002 stalled in {:?} past 1.5s despite reconnect during in-flight flush_due",
                rec.status
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    });
    client.outbox.stop();
}

#[test]
fn test_regression_issue4_chunked_mid_upload_404_reinit_and_complete_error_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rt = tokio::runtime::Runtime::new().expect("rt");

    let file_path = dir.path().join("mid404.bin");
    let raw_bytes: Vec<u8> = (0..(100 * 1024)).map(|i| ((i * 17 + 3) % 256) as u8).collect();
    std::fs::write(&file_path, &raw_bytes).unwrap();
    let full_sha = sha256_hex_local(&raw_bytes);

    #[derive(Clone, Copy)]
    enum Mode {
        ReinitOnceOnPart1,
        Always404OnPart1,
        Complete400,
        Complete503,
    }
    struct Srv {
        mode: Mode,
        init_count: usize,
        evicted_up1: bool,
    }
    let srv = Arc::new(Mutex::new(Srv {
        mode: Mode::ReinitOnceOnPart1,
        init_count: 0,
        evicted_up1: false,
    }));

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    let srv_ref = Arc::clone(&srv);
    let fsha = full_sha.clone();

    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let st = Arc::clone(&srv_ref);
            let sha = fsha.clone();
            tokio::spawn(async move {
                let (method, path, _, _body) = read_http_request(&mut stream).await;
                if method == "POST" && path == "/api/v1/media/upload/init" {
                    let resp_str = {
                        let mut g = st.lock();
                        g.init_count += 1;
                        let uid = format!("up_{}", g.init_count);
                        json!({
                            "upload_id": uid,
                            "chunk_size": 32 * 1024,
                            "expires_at_ms": 1800000000000i64
                        })
                        .to_string()
                    };
                    write_json_response(&mut stream, 200, "OK", &resp_str).await;
                } else if method == "PUT" && path.starts_with("/api/v1/media/upload/") {
                    let rest = path.trim_start_matches("/api/v1/media/upload/");
                    let segs: Vec<&str> = rest.split('/').collect();
                    let uid = segs[0];
                    let part_idx: u32 = segs.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let return_404 = {
                        let mut g = st.lock();
                        match g.mode {
                            Mode::ReinitOnceOnPart1 => {
                                if uid == "up_1" && part_idx == 1 {
                                    g.evicted_up1 = true;
                                    true
                                } else {
                                    false
                                }
                            }
                            Mode::Always404OnPart1 => part_idx == 1,
                            Mode::Complete400 | Mode::Complete503 => false,
                        }
                    };
                    if return_404 {
                        write_json_response(
                            &mut stream,
                            404,
                            "Not Found",
                            r#"{"error":"staging evicted mid-upload"}"#,
                        )
                        .await;
                    } else {
                        write_json_response(&mut stream, 200, "OK", r#"{"ok":true}"#).await;
                    }
                } else if method == "POST" && path.ends_with("/complete") {
                    let uid = path
                        .trim_start_matches("/api/v1/media/upload/")
                        .trim_end_matches("/complete");
                    let mode = st.lock().mode;
                    match mode {
                        Mode::Complete400 => {
                            write_json_response(
                                &mut stream,
                                400,
                                "Bad Request",
                                r#"{"error":"checksum mismatch"}"#,
                            )
                            .await;
                        }
                        Mode::Complete503 => {
                            write_json_response(
                                &mut stream,
                                503,
                                "Service Unavailable",
                                r#"{"error":"cas temporary outage"}"#,
                            )
                            .await;
                        }
                        _ => {
                            let resp = json!({
                                "file_id": format!("file_{uid}"),
                                "sha256": sha,
                                "size_bytes": 100 * 1024,
                                "mime_type": "application/octet-stream"
                            });
                            write_json_response(&mut stream, 200, "OK", &resp.to_string()).await;
                        }
                    }
                } else {
                    write_json_response(&mut stream, 404, "Not Found", "{}").await;
                }
            });
        }
    });

    let cfg = ClientConfig {
        server_url: format!("http://{addr}"),
        ..Default::default()
    };
    let client = TelegramClient::open_with_dir(cfg, dir.path()).unwrap();
    client.set_auth_session(make_test_auth(101, "bob", "tok_chunk"));

    // Case A: Mid-upload 404 on PUT /part/1 transparently re-initializes and completes on up_2
    let res_a = rt
        .block_on(client.upload_file_resumable(
            &file_path,
            "application/octet-stream",
            Some(10),
            Some("xfer_mid404"),
        ))
        .expect("mid-upload 404 must re-init and succeed");
    assert_eq!(res_a.file_id, "file_up_2");
    assert_eq!(srv.lock().init_count, 2);
    let rec_a = client.chunked_transfers.store().get("xfer_mid404").unwrap().unwrap();
    assert_eq!(rec_a.status, TransferStatus::Completed);

    // Case B: Persistent 404 on PUT /part/1 triggers bounded 1-retry guard and marks Failed
    srv.lock().mode = Mode::Always404OnPart1;
    let init_before_b = srv.lock().init_count;
    let res_b = rt.block_on(client.upload_file_resumable(
        &file_path,
        "application/octet-stream",
        Some(10),
        Some("xfer_loop404"),
    ));
    assert!(res_b.is_err());
    assert_eq!(
        srv.lock().init_count - init_before_b,
        2,
        "must re-init at most once on repeated mid-upload 404"
    );
    let rec_b = client.chunked_transfers.store().get("xfer_loop404").unwrap().unwrap();
    assert_eq!(rec_b.status, TransferStatus::Failed);

    // Case C: /complete 400 -> Failed (not stuck in InProgress)
    srv.lock().mode = Mode::Complete400;
    let res_c = rt.block_on(client.upload_file_resumable(
        &file_path,
        "application/octet-stream",
        Some(10),
        Some("xfer_comp400"),
    ));
    assert!(res_c.is_err());
    let rec_c = client.chunked_transfers.store().get("xfer_comp400").unwrap().unwrap();
    assert_eq!(rec_c.status, TransferStatus::Failed);

    // Case D: /complete 503 -> Paused (not stuck in InProgress)
    srv.lock().mode = Mode::Complete503;
    let res_d = rt.block_on(client.upload_file_resumable(
        &file_path,
        "application/octet-stream",
        Some(10),
        Some("xfer_comp503"),
    ));
    assert!(res_d.is_err());
    let rec_d = client.chunked_transfers.store().get("xfer_comp503").unwrap().unwrap();
    assert_eq!(rec_d.status, TransferStatus::Paused);
}

#[test]
fn test_regression_issue6_seed_nonce_near_i64_max_and_durable_send_without_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let cfg = ClientConfig {
        server_url: "http://127.0.0.1:1".to_string(),
        ..Default::default()
    };

    let near_max = (i64::MAX as u64) - 500;
    {
        let client1 = TelegramClient::open_with_dir(cfg.clone(), dir.path()).unwrap();
        let req = make_rich_req(42, "near max nonce", Some(near_max));
        client1
            .outbox
            .store()
            .enqueue(&req, near_max, 1_700_000_000_000)
            .unwrap();
    }

    // Reopen without auth token
    let client2 = TelegramClient::open_with_dir(cfg, dir.path()).unwrap();
    let next_n = client2.next_client_nonce();
    assert!(
        next_n > near_max && next_n <= i64::MAX as u64,
        "seeded nonce {next_n} must be strictly > persisted_max {near_max} and <= i64::MAX"
    );

    // send_message_durable without token must still enqueue in outbox
    client2.network_listener.set_online(true);
    let no_tok_nonce = 777_001u64;
    let res = rt.block_on(client2.send_message_durable(make_rich_req(
        42,
        "queued without token",
        Some(no_tok_nonce),
    )));
    assert!(res.is_err());
    let rec = client2
        .outbox
        .store()
        .get(no_tok_nonce)
        .unwrap()
        .expect("message must be persisted in outbox even when token is missing");
    assert_eq!(rec.status, OutboxStatus::Pending);
}

#[test]
fn t5_48_gap_r12b_2_4_client_store_transfer_and_outbox_epoch_races() {
    use gpui_telegram_client::{
        ClientSyncCoordinator, NetworkStatusListener, OutboxStore, OutboxWorker,
    };
    use gpui_telegram_proto::contracts::chats::ChatDetailDto;
    use gpui_telegram_proto::message::ReactionCount;

    let rt = tokio::runtime::Runtime::new().expect("rt");
    let dir = tempfile::tempdir().expect("tempdir");

    // Precompute valid X3DH bundle for peer user 500 ("dev_500")
    let peer_id = DeviceIdentityKeys::generate().unwrap();
    let peer_spk_sec = random_32().unwrap();
    let peer_spk_pub = x25519_pub_from_secret(&peer_spk_sec);
    let peer_spk_sig = peer_id.sign_signed_prekey(&peer_spk_pub);
    let peer_otpk_sec = random_32().unwrap();
    let peer_otpk_pub = x25519_pub_from_secret(&peer_otpk_sec);
    let bundle_resp = UserPreKeyBundlesResponse {
        user_id: 500,
        devices: vec![DevicePreKeyBundle {
            user_id: 500,
            device_id: "dev_500".to_string(),
            device_name: Some("Peer Device".to_string()),
            identity_key: b64_encode(&peer_id.x25519_pub),
            identity_signing_key: b64_encode(&peer_id.ed25519_pub),
            signed_prekey: SignedPreKeyDto {
                key_id: 1,
                public_key: b64_encode(&peer_spk_pub),
                signature: b64_encode(&peer_spk_sig),
                created_at_ms: 1_700_000_000_000,
            },
            one_time_prekey: Some(OneTimePreKeyDto {
                key_id: 1,
                public_key: b64_encode(&peer_otpk_pub),
            }),
        }],
    };
    let bundle_json = serde_json::to_string(&bundle_resp).unwrap();

    #[derive(Default)]
    struct SrvState {
        part0_should_sleep: bool,
        complete_hits: usize,
        outbox_send_hits: usize,
    }
    let srv_state = Arc::new(Mutex::new(SrvState::default()));
    let srv_state_bg = Arc::clone(&srv_state);

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let st = Arc::clone(&srv_state_bg);
            let bundle_json = bundle_json.clone();
            tokio::spawn(async move {
                let (method, path, _headers, _body) = read_http_request(&mut stream).await;
                if method == "POST" && path == "/api/v1/chats/101/leave" {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    write_json_response(&mut stream, 200, "OK", r#"{"success":true}"#).await;
                } else if method == "POST" && path == "/api/v1/chats/102/clear" {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    write_json_response(&mut stream, 200, "OK", r#"{"success":true}"#).await;
                } else if method == "GET" && path.starts_with("/api/v1/chats/101/messages") {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    let body = json!([{
                        "message_id": 42,
                        "sender_id": 1001,
                        "text": "user_a_msg",
                        "reactions": [{ "emoji": "🔥", "count": 1, "chosen": true }],
                        "reactions_authoritative": true
                    }])
                    .to_string();
                    write_json_response(&mut stream, 200, "OK", &body).await;
                } else if method == "POST" && path == "/api/v1/chats/103/read" {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        r#"{"success":true,"chat_id":103,"pts":50,"read_inbox_max_id":50,"read_outbox_max_id":0,"unread_count":0}"#,
                    )
                    .await;
                } else if method == "POST" && path == "/api/v1/sync/get_difference" {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    let body = json!({
                        "chat_id": 101,
                        "new_messages": [],
                        "other_updates": [],
                        "pts": 99,
                        "current_pts": 99,
                        "updates": [],
                        "has_more": false
                    })
                    .to_string();
                    write_json_response(&mut stream, 200, "OK", &body).await;
                } else if method == "GET" && path == "/api/v1/media/stream/media-a" {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    let payload = b"USER_A_SECRET_MEDIA";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let _ = stream.write_all(payload).await;
                    let _ = stream.flush().await;
                } else if method == "GET" && path.starts_with("/api/v1/e2ee/devices/500/prekeys") {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    write_json_response(&mut stream, 200, "OK", &bundle_json).await;
                } else if method == "POST" && path == "/api/v1/e2ee/prekeys" {
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        r#"{"success":true,"user_id":2002,"device_id":"dev","remaining_one_time_prekeys":10,"updated_at_ms":1700000000000}"#,
                    )
                    .await;
                } else if method == "POST" && path == "/api/v1/media/upload/init" {
                    let sleep_init = !st.lock().part0_should_sleep;
                    if sleep_init {
                        tokio::time::sleep(Duration::from_millis(140)).await;
                    }
                    // Deliberately return chunk_size: 4 != initial 32*1024 to hit the
                    // `record.part_size != server_part_size` store.insert branch in manager.rs!
                    let body = json!({
                        "upload_id": "up_race_1",
                        "chunk_size": 4,
                        "expires_at_ms": 1800000000000i64
                    })
                    .to_string();
                    write_json_response(&mut stream, 200, "OK", &body).await;
                } else if method == "PUT" && path.starts_with("/api/v1/media/upload/") {
                    if st.lock().part0_should_sleep {
                        tokio::time::sleep(Duration::from_millis(140)).await;
                    }
                    write_json_response(&mut stream, 200, "OK", r#"{"ok":true}"#).await;
                } else if method == "POST" && path.ends_with("/complete") {
                    st.lock().complete_hits += 1;
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        r#"{"file_id":"bad_complete","sha256":"00","size_bytes":8,"mime_type":"application/octet-stream"}"#,
                    )
                    .await;
                } else if method == "POST"
                    && (path == "/api/v1/messages/send" || path.contains("/messages"))
                {
                    st.lock().outbox_send_hits += 1;
                    write_json_response(&mut stream, 200, "OK", r#"{"message_id":999}"#).await;
                } else {
                    write_json_response(&mut stream, 200, "OK", "{}").await;
                }
            });
        }
    });

    let cfg = ClientConfig {
        server_url: format!("http://{addr}"),
        ..Default::default()
    };
    let client = Arc::new(TelegramClient::open_with_dir(cfg.clone(), dir.path()).unwrap());

    // ------------------------------------------------------------------
    // Sub-case A: GAP-R12B-2 post-.await local_store / sync / E2EE races
    // ------------------------------------------------------------------
    client.set_auth_session(make_test_auth(1001, "alice", "tok-a"));
    client.sync_coordinator.set_pts(ChatId(101), 20);
    let chat_dto = |cid: i64, title: &str, read_max: i64| ChatDetailDto {
        chat_id: cid,
        title: title.to_string(),
        kind: "private".to_string(),
        chat_type: 0,
        creator_id: 2002,
        username: None,
        about: None,
        is_group: false,
        is_channel: false,
        is_bot: false,
        is_forum: false,
        is_archived: false,
        is_pinned: false,
        is_muted: false,
        mute_until_ms: None,
        pts: 1,
        last_message_id: 0,
        last_message_date_ms: 0,
        unread_count: 0,
        read_inbox_max_id: read_max,
        read_outbox_max_id: 0,
        member_count: 2,
        pinned_message_id: None,
        draft: None,
    };

    let c1 = Arc::clone(&client);
    let h_leave = rt.spawn(async move {
        let _ = c1.leave_or_delete_chat(101).await;
    });
    let c2 = Arc::clone(&client);
    let h_clear = rt.spawn(async move {
        let _ = c2.clear_chat_history(102).await;
    });
    let c3 = Arc::clone(&client);
    let h_msgs = rt.spawn(async move {
        let _ = c3.get_chat_messages_with_token("tok-a", 101, None).await;
    });
    let c4 = Arc::clone(&client);
    let h_read = rt.spawn(async move {
        let _ = c4.mark_chat_read(103, 50, None).await;
    });
    let c5 = Arc::clone(&client);
    let h_sync = rt.spawn(async move {
        let _ = c5.sync_chat_difference(ChatId(101), 20).await;
    });
    let c6 = Arc::clone(&client);
    let h_media = rt.spawn(async move {
        let _ = c6.get_media_bytes("media-a").await;
    });
    let c7 = Arc::clone(&client);
    let h_e2ee = rt.spawn(async move {
        let _ = c7.e2ee_encrypt_dm(500, b"user_a_secret_e2ee").await;
    });

    rt.block_on(async {
        tokio::time::sleep(Duration::from_millis(25)).await;
    });

    // Switch to User B while all 7 User A requests are sleeping mid-HTTP
    client.set_auth_session(make_test_auth(2002, "bob", "tok-b"));
    client
        .local_store
        .upsert_chat(chat_dto(101, "User B Chat 101", 0));
    client
        .local_store
        .upsert_chat(chat_dto(103, "User B Chat 103", 10));
    let mut b_msg = MessageItem::new(
        MessageId(99),
        ChatId(102),
        UserId(2002),
        PeerId::Chat(ChatId(102)),
        MessageContent::plain_text("user_b_msg"),
    );
    b_msg.reactions = vec![ReactionCount::new("❤️", 1, true)];
    client.local_store.insert_message(b_msg);
    client.local_store.update_reactions(
        ChatId(101),
        MessageId(42),
        vec![ReactionCount::new("❤️", 1, true)],
    );
    client.sync_coordinator.set_pts(ChatId(101), 1);

    rt.block_on(async {
        let _ = tokio::join!(h_leave, h_clear, h_msgs, h_read, h_sync, h_media, h_e2ee);
    });

    assert!(
        client.local_store.get_chat(ChatId(101)).is_some(),
        "User A's late leave_or_delete_chat must not delete User B's Chat 101"
    );
    let b_msgs_102 = client.local_store.get_messages(ChatId(102)).unwrap();
    assert!(
        b_msgs_102.iter().any(|m| m.id == MessageId(99)),
        "User A's late clear_chat_history must not wipe User B's msg 99"
    );
    let rx_101_42 = client
        .local_store
        .get_reactions(ChatId(101), MessageId(42))
        .expect("User B reaction on (101, 42) must remain");
    assert_eq!(rx_101_42[0].emoji, "❤️");
    assert_eq!(
        client
            .local_store
            .get_chat(ChatId(103))
            .expect("User B Chat 103")
            .read_inbox_max_id,
        10,
        "User A's late mark_chat_read(50) must not overwrite User B's read_inbox_max_id=10"
    );
    assert_eq!(
        client.sync_coordinator.last_pts(ChatId(101)),
        1,
        "User A's late sync_chat_difference must not overwrite User B's pts=1"
    );
    assert!(
        client.local_store.get_media_cas("media-a").is_none(),
        "User A's late get_media_bytes must not write into User B's CAS"
    );
    assert!(
        client
            .e2ee
            .store()
            .get_session(500, "dev_500")
            .unwrap()
            .is_none(),
        "User A's late e2ee_encrypt_dm must not write X3DH session into User B's E2eeStore"
    );

    // ------------------------------------------------------------------
    // Sub-case B: GAP-R12B-4 ChunkedTransferManager across clear_session
    // ------------------------------------------------------------------
    let upload_file_path = dir.path().join("xfer_race.bin");
    std::fs::write(&upload_file_path, b"abcdefgh").unwrap();

    // Case B1: clear_session() mid-/init (hits record.part_size != server_part_size)
    let cu1 = Arc::clone(&client);
    let p1 = upload_file_path.clone();
    let h_up1 = rt.spawn(async move {
        cu1.upload_file_resumable(&p1, "application/octet-stream", Some(101), Some("xfer-race-init"))
            .await
    });
    rt.block_on(async {
        tokio::time::sleep(Duration::from_millis(30)).await;
    });
    client.clear_session();
    let res_up1 = rt.block_on(h_up1).unwrap();
    assert!(res_up1.is_err(), "cleared session upload during /init must return Err");
    assert!(
        client
            .chunked_transfer_manager()
            .store()
            .get("xfer-race-init")
            .unwrap()
            .is_none(),
        "late /init return must NOT re-insert row into cleared FileTransferStore"
    );

    // Case B2: set_auth_session(User B) mid-/part/0 -> never calls /complete, store stays empty
    client.set_auth_session(make_test_auth(1001, "alice", "tok-a"));
    srv_state.lock().part0_should_sleep = true;
    srv_state.lock().complete_hits = 0;
    let cu2 = Arc::clone(&client);
    let p2 = upload_file_path;
    let h_up2 = rt.spawn(async move {
        cu2.upload_file_resumable(&p2, "application/octet-stream", Some(101), Some("xfer-race-part"))
            .await
    });
    rt.block_on(async {
        tokio::time::sleep(Duration::from_millis(30)).await;
    });
    client.set_auth_session(make_test_auth(2002, "bob", "tok-b"));
    let res_up2 = rt.block_on(h_up2).unwrap();
    assert!(res_up2.is_err(), "switched session upload during /part/0 must return Err");
    assert_eq!(srv_state.lock().complete_hits, 0, "/complete must never be called after epoch bump");
    assert!(
        client
            .chunked_transfer_manager()
            .store()
            .get("xfer-race-part")
            .unwrap()
            .is_none(),
        "late /part/0 return must NOT insert Cancelled/Failed row into User B's FileTransferStore"
    );

    // ------------------------------------------------------------------
    // Sub-case C: r11b_2 Minor 1 (OutboxWorker token_provider epoch race)
    //             & Minor 2 (LocalStore::clear_all clears mem_media_bytes)
    // ------------------------------------------------------------------
    let mem_store = LocalStore::new();
    mem_store
        .put_media_cas("m1", b"secret-bytes", Some("application/octet-stream"))
        .unwrap();
    assert!(mem_store.get_media_cas("m1").is_some());
    mem_store.clear_all();
    assert!(
        mem_store.get_media_cas("m1").is_none(),
        "LocalStore::clear_all must clear RAM mem_media_bytes"
    );

    let outbox_dir = tempfile::tempdir().expect("outbox tempdir");
    let outbox_store = OutboxStore::open(outbox_dir.path()).unwrap();
    let net = NetworkStatusListener::new(NetworkState::Online);
    let http = HttpClient::new(format!("http://{addr}"));
    let worker_epoch = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let worker_epoch_tp = Arc::clone(&worker_epoch);
    let tp_return_some = Arc::new(AtomicBool::new(true));
    let tp_return_some_cl = Arc::clone(&tp_return_some);
    let worker = OutboxWorker::new_with_session_epoch(
        outbox_store.clone(),
        net,
        http,
        Arc::new(move || {
            // Simulate concurrent session switch right inside token_provider()
            worker_epoch_tp.fetch_add(1, Ordering::SeqCst);
            if tp_return_some_cl.load(Ordering::SeqCst) {
                Some("tok-stale".to_string())
            } else {
                None
            }
        }),
        LocalStore::new(),
        ClientSyncCoordinator::new(),
        Arc::clone(&worker_epoch),
    );

    srv_state.lock().outbox_send_hits = 0;
    let nonce_a = 880_001u64;
    let req_a = make_rich_req(42, "epoch race some token", Some(nonce_a));
    outbox_store.enqueue(&req_a, nonce_a, 1_700_000_000_000).unwrap();
    let _ = rt.block_on(worker.flush_due()).unwrap();
    assert_eq!(srv_state.lock().outbox_send_hits, 0, "must send 0 HTTP requests on post-token_provider epoch mismatch");
    let rec_a = outbox_store.get(nonce_a).unwrap().unwrap();
    assert_eq!(rec_a.retry_count, 0);
    assert!(rec_a.last_error.is_none(), "must not record failure across epoch bump: {:?}", rec_a.last_error);

    tp_return_some.store(false, Ordering::SeqCst);
    let nonce_b = 880_002u64;
    let req_b = make_rich_req(42, "epoch race none token", Some(nonce_b));
    outbox_store.enqueue(&req_b, nonce_b, 1_700_000_000_000).unwrap();
    let _ = rt.block_on(worker.flush_due()).unwrap();
    assert_eq!(srv_state.lock().outbox_send_hits, 0);
    let rec_b = outbox_store.get(nonce_b).unwrap().unwrap();
    assert_eq!(rec_b.retry_count, 0);
    assert!(rec_b.last_error.is_none(), "must not record missing-token error across epoch bump: {:?}", rec_b.last_error);
}

#[test]
fn t5_48_subcase_d_deferred_async_futures_and_inflight_tailnet_races() {
    use gpui_telegram_proto::computer_contracts::{
        CloudAuthMethod, ComputerNodeKind, ComputerRunPolicy, UpdateBotComputerRequest,
        UpdateComputerPolicyRequest, UpsertCloudComputerRequest,
    };
    use gpui_telegram_proto::tailnet_contracts::{
        TailnetNetMap, TailnetPeerDescriptor, TailnetRelayMode,
    };

    let rt = tokio::runtime::Runtime::new().expect("rt");
    let dir = tempfile::tempdir().expect("tempdir");

    let recorded_hits: Arc<Mutex<Vec<(String, String, String)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let hold_inflight = Arc::new(AtomicBool::new(false));
    let reg_arrived = Arc::new(AtomicUsize::new(0));
    let netmap_arrived = Arc::new(AtomicUsize::new(0));
    let policy_arrived = Arc::new(AtomicUsize::new(0));

    let recorded_hits_srv = Arc::clone(&recorded_hits);
    let hold_inflight_srv = Arc::clone(&hold_inflight);
    let reg_arrived_srv = Arc::clone(&reg_arrived);
    let netmap_arrived_srv = Arc::clone(&netmap_arrived);
    let policy_arrived_srv = Arc::clone(&policy_arrived);

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let hits = Arc::clone(&recorded_hits_srv);
            let hold = Arc::clone(&hold_inflight_srv);
            let reg_arr = Arc::clone(&reg_arrived_srv);
            let nm_arr = Arc::clone(&netmap_arrived_srv);
            let pol_arr = Arc::clone(&policy_arrived_srv);
            tokio::spawn(async move {
                let (method, path, headers, _body) = read_http_request(&mut stream).await;
                let auth_hdr = headers
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                hits.lock()
                    .push((method.clone(), path.clone(), auth_hdr.clone()));

                if path == "/api/v1/tailnet/register" {
                    reg_arr.fetch_add(1, Ordering::SeqCst);
                    while hold.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    let nm = TailnetNetMap {
                        owner_user_id: 1001,
                        seq: 42,
                        generated_at_ms: 1_700_000_000_000,
                        derp_region: "iad".into(),
                        derp_ws_path: "/api/v1/tailnet/derp".into(),
                        stun_http_path: "/api/v1/tailnet/stun".into(),
                        self_peer: Some(TailnetPeerDescriptor {
                            node_id: "node-user-a-self".into(),
                            device_id: "dev-user-a".into(),
                            display_name: "User A Mac".into(),
                            tailnet_ip: "100.64.0.10".into(),
                            lan_ip: "192.168.1.10".into(),
                            os_name: "macos".into(),
                            online: true,
                            relay_mode: TailnetRelayMode::DerpRelay,
                            ..Default::default()
                        }),
                        peers: vec![TailnetPeerDescriptor {
                            node_id: "node-user-a".into(),
                            device_id: "dev-peer-a".into(),
                            display_name: "User A Peer".into(),
                            tailnet_ip: "100.64.0.11".into(),
                            online: true,
                            relay_mode: TailnetRelayMode::DerpRelay,
                            ..Default::default()
                        }],
                    };
                    let body = serde_json::to_string(&nm).unwrap();
                    write_json_response(&mut stream, 200, "OK", &body).await;
                } else if path == "/api/v1/tailnet/netmap" {
                    nm_arr.fetch_add(1, Ordering::SeqCst);
                    while hold.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    let nm = TailnetNetMap {
                        owner_user_id: 1001,
                        seq: 43,
                        generated_at_ms: 1_700_000_000_001,
                        derp_region: "iad".into(),
                        derp_ws_path: "/api/v1/tailnet/derp".into(),
                        stun_http_path: "/api/v1/tailnet/stun".into(),
                        self_peer: Some(TailnetPeerDescriptor {
                            node_id: "node-user-a-self".into(),
                            device_id: "dev-user-a".into(),
                            display_name: "User A Mac".into(),
                            tailnet_ip: "100.64.0.10".into(),
                            online: true,
                            relay_mode: TailnetRelayMode::DerpRelay,
                            ..Default::default()
                        }),
                        peers: vec![TailnetPeerDescriptor {
                            node_id: "node-user-a".into(),
                            device_id: "dev-peer-a".into(),
                            display_name: "User A Peer".into(),
                            tailnet_ip: "100.64.0.12".into(),
                            online: true,
                            relay_mode: TailnetRelayMode::DerpRelay,
                            ..Default::default()
                        }],
                    };
                    let body = serde_json::to_string(&nm).unwrap();
                    write_json_response(&mut stream, 200, "OK", &body).await;
                } else if path.contains("/policy") {
                    pol_arr.fetch_add(1, Ordering::SeqCst);
                    while hold.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    let dto = json!({
                        "node_id": "node-shared-target",
                        "owner_user_id": 1001,
                        "kind": ComputerNodeKind::CloudServer,
                        "display_name": "User A Node",
                        "ip_address": "10.0.0.1",
                        "online": true,
                        "is_current_device": false,
                        "tier_badge": "cloud",
                        "run_policy": ComputerRunPolicy::Disabled,
                        "authorized_controllers": [],
                        "bound_bot_ids": [],
                        "last_seen_ms": 1700000000000i64
                    })
                    .to_string();
                    write_json_response(&mut stream, 200, "OK", &dto).await;
                } else {
                    write_json_response(&mut stream, 200, "OK", r#"{}"#).await;
                }
            });
        }
    });

    let mut cfg = ClientConfig::default();
    cfg.server_url = format!("http://{addr}");
    let client = Arc::new(TelegramClient::open_with_dir(cfg, dir.path()).unwrap());

    // ------------------------------------------------------------------
    // Part 1: Deferred-future switch (User A -> User B, and User A -> clear_session)
    // Construct futures BEFORE switch; poll AFTER switch. Zero HTTP requests allowed!
    // ------------------------------------------------------------------
    let policy_req = UpdateComputerPolicyRequest {
        run_policy: Some(ComputerRunPolicy::Disabled),
        ..Default::default()
    };
    let cloud_req = UpsertCloudComputerRequest {
        node_id: None,
        display_name: "Node A".into(),
        host: "10.0.0.5".into(),
        port: 22,
        username: "root".into(),
        auth_method: CloudAuthMethod::SshPassword,
        secret: Some("pw".into()),
        agent_endpoint_url: None,
        workdir: None,
        bind_bot_id: None,
        verify_connection: false,
    };
    let bot_req = UpdateBotComputerRequest {
        bound_node_id: Some("node-a".into()),
        ..Default::default()
    };

    for use_clear in [false, true] {
        recorded_hits.lock().clear();
        client.set_auth_session(make_test_auth(1001, "alice", "tok-a"));

        let f_policy = client.update_computer_policy_async("node-a", &policy_req);
        let f_reg_tn = client.register_tailnet_async();
        let f_get_nm = client.get_tailnet_netmap_async();
        let f_del = client.delete_computer_async("node-a");
        let f_rev = client.revoke_mcp_grant_async("fam-a");
        let f_ups = client.upsert_cloud_computer_async(&cloud_req);
        let f_dis = client.disconnect_all_remote_sessions_async();
        let f_code = client.set_remote_assist_code_async("123456");
        let f_bot = client.update_bot_computer_profile_async(77, &bot_req);
        let f_app = client.approve_computer_command_async("cmd-a", true);

        if use_clear {
            client.clear_session();
        } else {
            client.set_auth_session(make_test_auth(2002, "bob", "tok-b"));
        }

        rt.block_on(async {
            assert!(f_policy.await.is_err(), "update_computer_policy_async must fail after switch");
            assert!(f_reg_tn.await.is_err(), "register_tailnet_async must fail after switch");
            assert!(f_get_nm.await.is_err(), "get_tailnet_netmap_async must fail after switch");
            assert!(f_del.await.is_err(), "delete_computer_async must fail after switch");
            assert!(f_rev.await.is_err(), "revoke_mcp_grant_async must fail after switch");
            assert!(f_ups.await.is_err(), "upsert_cloud_computer_async must fail after switch");
            assert!(f_dis.await.is_err(), "disconnect_all_remote_sessions_async must fail after switch");
            assert!(f_code.await.is_err(), "set_remote_assist_code_async must fail after switch");
            assert!(f_bot.await.is_err(), "update_bot_computer_profile_async must fail after switch");
            assert!(f_app.await.is_err(), "approve_computer_command_async must fail after switch");
        });

        let hits = recorded_hits.lock().clone();
        assert!(
            hits.is_empty(),
            "deferred futures (use_clear={use_clear}) must send 0 HTTP requests after session switch: {hits:?}"
        );
    }

    // ------------------------------------------------------------------
    // Part 2: In-flight AtomicBool-gated tailnet / apply_netmap / policy races
    // ------------------------------------------------------------------
    recorded_hits.lock().clear();
    hold_inflight.store(true, Ordering::SeqCst);
    reg_arrived.store(0, Ordering::SeqCst);
    netmap_arrived.store(0, Ordering::SeqCst);
    policy_arrived.store(0, Ordering::SeqCst);

    client.set_auth_session(make_test_auth(1001, "alice", "tok-a"));
    let old_epoch = client.session_epoch();

    let c1 = Arc::clone(&client);
    let c2 = Arc::clone(&client);
    let c3 = Arc::clone(&client);
    let pol_req_clone = policy_req.clone();

    let h_reg = rt.spawn(async move { c1.register_tailnet_async().await });
    let h_nm = rt.spawn(async move { c2.get_tailnet_netmap_async().await });
    let h_pol = rt.spawn(async move {
        c3.update_computer_policy_async("node-shared-target", &pol_req_clone)
            .await
    });

    rt.block_on(async {
        for _ in 0..200 {
            if reg_arrived.load(Ordering::SeqCst) >= 1
                && netmap_arrived.load(Ordering::SeqCst) >= 1
                && policy_arrived.load(Ordering::SeqCst) >= 1
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    assert!(reg_arrived.load(Ordering::SeqCst) >= 1);
    assert!(netmap_arrived.load(Ordering::SeqCst) >= 1);
    assert!(policy_arrived.load(Ordering::SeqCst) >= 1);

    // Switch to User B while User A's tailnet & policy requests are held in-flight at server
    client.set_auth_session(make_test_auth(2002, "bob", "tok-b"));

    // Verify stale *_with_token calls with (tok-a, old_epoch) are also rejected immediately
    rt.block_on(async {
        assert!(client.register_tailnet_with_token("tok-a", old_epoch).await.is_err());
        assert!(client.get_tailnet_netmap_with_token("tok-a", old_epoch).await.is_err());
        assert!(
            client
                .update_computer_policy_with_token("tok-a", old_epoch, "node-a", &policy_req)
                .await
                .is_err()
        );
    });

    hold_inflight.store(false, Ordering::SeqCst);
    rt.block_on(async {
        let _ = tokio::join!(h_reg, h_nm, h_pol);
    });

    let tn_status = client.tailnet_status();
    assert!(
        tn_status.tailnet_ip.is_empty(),
        "User A's stale netmap must NOT set User B's tailnet_ip: {:?}",
        tn_status.tailnet_ip
    );
    assert_eq!(
        tn_status.netmap_seq, 0,
        "User A's stale netmap must NOT set User B's netmap_seq"
    );
    assert!(
        tn_status.peers.is_empty(),
        "User A's stale peers must NOT populate User B's tailnet peers: {:?}",
        tn_status.peers
    );
    assert!(
        !recorded_hits
            .lock()
            .iter()
            .any(|(_, _, auth)| auth.contains("tok-b")),
        "zero Bearer tok-b requests should have been sent: {:?}",
        *recorded_hits.lock()
    );
}

#[test]
fn t5_48_subcase_e_deferred_client_modules_token_isolation() {
    use gpui_telegram_client::LoginRequest;
    use gpui_telegram_proto::contracts::chats::CreateChatRequest;

    let rt = tokio::runtime::Runtime::new().expect("rt");
    let dir = tempfile::tempdir().expect("tempdir");

    let recorded_hits: Arc<Mutex<Vec<(String, String, String, String)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let recorded_hits_srv = Arc::clone(&recorded_hits);

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let hits = Arc::clone(&recorded_hits_srv);
            tokio::spawn(async move {
                let (method, path, headers, body) = read_http_request(&mut stream).await;
                let auth_hdr = headers
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let body_str = String::from_utf8_lossy(&body).to_string();
                hits.lock()
                    .push((method.clone(), path.clone(), auth_hdr, body_str));

                if method == "POST" && path == "/api/v1/tasks" {
                    write_json_response(&mut stream, 200, "OK", r#"{"task_id":"t-1","status":"pending"}"#).await;
                } else if method == "POST" && path == "/api/v1/mcp/servers" {
                    write_json_response(&mut stream, 200, "OK", r#"{"server_id":"mcp-1","ok":true}"#).await;
                } else if method == "GET" && path.starts_with("/api/v1/search/messages") {
                    write_json_response(&mut stream, 200, "OK", r#"{"query":"probe","total":0,"hits":[],"results":[]}"#).await;
                } else if method == "GET" && path.starts_with("/api/v1/miniapps") {
                    write_json_response(&mut stream, 200, "OK", r#"[]"#).await;
                } else if method == "POST" && path == "/api/v1/chats/create" {
                    let body = json!({
                        "chat_id": 700,
                        "title": "Pos Chat",
                        "kind": "group",
                        "chat_type": 1,
                        "creator_id": 2002,
                        "username": null,
                        "about": null,
                        "is_group": true,
                        "is_channel": false,
                        "is_bot": false,
                        "is_forum": false,
                        "is_archived": false,
                        "is_pinned": false,
                        "is_muted": false,
                        "mute_until_ms": null,
                        "pts": 1,
                        "last_message_id": 0,
                        "last_message_date_ms": 0,
                        "unread_count": 0,
                        "read_inbox_max_id": 0,
                        "read_outbox_max_id": 0,
                        "member_count": 1,
                        "pinned_message_id": null,
                        "draft": null
                    })
                    .to_string();
                    write_json_response(&mut stream, 200, "OK", &body).await;
                } else if method == "PUT" && path.starts_with("/api/v1/chats/") && path.contains("/messages/") {
                    write_json_response(&mut stream, 200, "OK", r#"{}"#).await;
                } else if method == "POST" && path == "/api/v1/messages/send" {
                    let body = json!({
                        "message_id": 901,
                        "id": 901,
                        "pts": 10,
                        "pts_count": 1,
                        "duplicate": false,
                        "date_ms": 1700000000000i64,
                        "date": "2026-11-24T00:00:00Z",
                        "sender_id": 2002,
                        "chat_id": 700,
                        "text": "pos",
                        "status": "sent"
                    })
                    .to_string();
                    write_json_response(&mut stream, 200, "OK", &body).await;
                } else if method == "GET" && path == "/api/v1/auth/me" {
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        r#"{"user_id":2002,"username":"bob","first_name":"Bob","session_id":"sess-b","device_name":null,"account_generation":null,"created_at":null,"expires_at":null}"#,
                    )
                    .await;
                } else if method == "GET" && path == "/api/v1/auth/sessions" {
                    write_json_response(&mut stream, 200, "OK", r#"[]"#).await;
                } else if method == "GET" && path == "/api/v1/user-vault/envelope" {
                    write_json_response(&mut stream, 200, "OK", r#"{"envelope":null}"#).await;
                } else if method == "GET" && path.starts_with("/api/v1/hermes/status") {
                    write_json_response(&mut stream, 200, "OK", r#"{"healthy":true,"engine":"hermes"}"#).await;
                } else if method == "POST" && path == "/api/v1/auth/login" {
                    let body = json!({
                        "user_id": 2002,
                        "username": "bob",
                        "session_id": "sess-b",
                        "token": "tok-b",
                        "first_name": "Bob",
                        "created_at": null,
                        "expires_at": null
                    })
                    .to_string();
                    write_json_response(&mut stream, 200, "OK", &body).await;
                } else if method == "POST" && path == "/api/v1/auth/logout" {
                    write_json_response(&mut stream, 200, "OK", r#"{}"#).await;
                } else {
                    write_json_response(&mut stream, 200, "OK", r#"{}"#).await;
                }
            });
        }
    });

    let mut cfg = ClientConfig::default();
    cfg.server_url = format!("http://{addr}");
    let client = Arc::new(TelegramClient::open_with_dir(cfg, dir.path()).unwrap());

    let create_chat_req = CreateChatRequest {
        title: "Secret Group A".to_string(),
        ..Default::default()
    };

    let task_payload = json!({"title": "task-a"});
    let mcp_payload = json!({"name": "mcp-a"});

    // 1. Negative controls: User A -> User B AND User A -> clear_session()
    for use_clear in [false, true] {
        recorded_hits.lock().clear();
        client.set_auth_session(make_test_auth(1001, "alice", "tok-a"));
        client.outbox.stop();
        client.network_listener.set_online(false);

        let f_task = client.create_task(&task_payload);
        let f_mcp = client.register_mcp_server(&mcp_payload);
        let f_search = client.search_messages("probe", None, None, 10);
        let f_apps = client.list_mini_apps(None, None);
        let f_chat = client.create_chat(&create_chat_req);
        let f_edit = client.edit_message(700, 1, "edited-a", vec![]);
        let f_send = client.send_message_durable(make_rich_req(700, "secret-a", Some(99001)));
        let f_text = client.send_text_message(700, "text-a", None);
        let f_msgs_wt = client.get_chat_messages_with_token("tok-a", 700, None);
        let f_me = client.get_me();
        let f_sessions = client.list_sessions();
        let f_logout = client.logout();
        let f_vault = client.get_user_vault_envelope();
        let f_hermes = client.hermes_status();
        let f_flush = client.outbox.flush_due();
        let f_reconn = client.outbox.notify_reconnected_and_flush();

        if use_clear {
            client.clear_session();
            client.network_listener.set_online(false);
        } else {
            client.set_auth_session(make_test_auth(2002, "bob", "tok-b"));
            client.outbox.stop();
            client.network_listener.set_online(false);
        }

        let bob_nonce = 99010u64 + use_clear as u64;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let future_retry_ms = now_ms + 60_000;
        if !use_clear {
            client
                .outbox
                .enqueue(&make_rich_req(700, "bob-pending", Some(bob_nonce)), bob_nonce)
                .unwrap();
            let claimed = client
                .outbox
                .store()
                .claim_next_due(now_ms)
                .unwrap()
                .expect("bob pending record claimed");
            assert_eq!(claimed.client_nonce, bob_nonce);
            client
                .outbox
                .store()
                .schedule_retry(bob_nonce, 1, future_retry_ms, "deferred", now_ms)
                .unwrap();
        }

        rt.block_on(async {
            assert!(f_task.await.is_err(), "create_task must fail after switch");
            assert!(f_mcp.await.is_err(), "register_mcp_server must fail after switch");
            assert!(f_search.await.is_err(), "search_messages must fail after switch");
            assert!(f_apps.await.is_err(), "list_mini_apps must fail after switch");
            assert!(f_chat.await.is_err(), "create_chat must fail after switch");
            assert!(f_edit.await.is_err(), "edit_message must fail after switch");
            assert!(f_send.await.is_err(), "send_message_durable must fail after switch");
            assert!(f_text.await.is_err(), "send_text_message must fail after switch");
            assert!(f_msgs_wt.await.is_err(), "get_chat_messages_with_token must fail after switch");
            assert!(f_me.await.is_err(), "get_me must fail after switch");
            assert!(f_sessions.await.is_err(), "list_sessions must fail after switch");
            assert!(f_logout.await.is_err(), "logout must fail after switch");
            assert!(f_vault.await.is_err(), "get_user_vault_envelope must fail after switch");
            assert!(f_hermes.await.is_err(), "hermes_status must fail after switch");
            assert!(
                matches!(
                    f_flush.await,
                    Err(gpui_telegram_core::error::TelegramError::GenerationFenced { .. })
                ),
                "deferred flush_due must fail closed with GenerationFenced"
            );
            assert!(
                matches!(
                    f_reconn.await,
                    Err(gpui_telegram_core::error::TelegramError::GenerationFenced { .. })
                ),
                "deferred notify_reconnected_and_flush must fail closed with GenerationFenced"
            );
        });

        let hits = recorded_hits.lock().clone();
        assert!(
            hits.is_empty(),
            "deferred futures (use_clear={use_clear}) must send 0 HTTP requests: {hits:?}"
        );
        assert!(
            !client.network_listener.is_online(),
            "deferred notify_reconnected_and_flush must NOT flip network online"
        );
        if !use_clear {
            let bob_rec = client.outbox.store().get(bob_nonce).unwrap().unwrap();
            assert_eq!(bob_rec.status, OutboxStatus::Pending);
            assert!(
                bob_rec.next_retry_at_ms > now_ms,
                "deferred notify_reconnected_and_flush must NOT reset User B retry timestamp"
            );
        }
        client.outbox.store().clear();
        client.network_listener.set_online(true);
        assert!(
            client.local_store.get_messages(ChatId(700)).unwrap().is_empty(),
            "deferred send_message_durable/send_text_message must NOT stage into local_store"
        );
        if !use_clear {
            assert_eq!(
                client.token().as_deref(),
                Some("tok-b"),
                "deferred logout() must NOT clear User B's session"
            );
        }
    }

    // 2. Positive controls: Category B pre-login + User B (tok-b) undisturbed
    recorded_hits.lock().clear();
    client.clear_session();
    rt.block_on(async {
        let _proxies = client.list_proxies().await.expect("Category B list_proxies");
        let auth = client
            .login(&LoginRequest {
                username: "bob".to_string(),
                password: "pw".to_string(),
                device_name: "test".to_string(),
            })
            .await
            .expect("Category B login");
        assert_eq!(auth.token, "tok-b");
    });
    assert_eq!(client.token().as_deref(), Some("tok-b"));

    recorded_hits.lock().clear();
    let pos_chat_req = CreateChatRequest {
        title: "Pos Chat".to_string(),
        ..Default::default()
    };
    rt.block_on(async {
        assert!(client.create_task(&json!({"title": "task-b"})).await.is_ok());
        assert!(client.register_mcp_server(&json!({"name": "mcp-b"})).await.is_ok());
        assert!(client.search_messages("probe", None, None, 10).await.is_ok());
        assert!(client.list_mini_apps(None, None).await.is_ok());
        assert!(client.create_chat(&pos_chat_req).await.is_ok());
        assert!(client.edit_message(700, 1, "pos-edit", vec![]).await.is_ok());
        assert!(
            client
                .send_message_durable(make_rich_req(700, "pos", Some(99002)))
                .await
                .is_ok()
        );
        assert!(client.get_me().await.is_ok());
        assert!(client.list_sessions().await.is_ok());
        assert!(client.get_user_vault_envelope().await.is_ok());
        assert!(client.hermes_status().await.is_ok());
    });

    let pos_hits = recorded_hits.lock().clone();
    assert_eq!(pos_hits.len(), 11, "all 11 positive requests must fire: {pos_hits:?}");
    assert!(
        pos_hits.iter().all(|(_, _, auth, _)| auth == "authorization: Bearer tok-b" || auth.to_ascii_lowercase() == "authorization: bearer tok-b"),
        "every positive control request must carry User B's Bearer tok-b: {pos_hits:?}"
    );
    assert!(
        client.local_store.get_chat(ChatId(700)).is_some(),
        "positive control create_chat must persist Chat 700 into local_store"
    );

    // Positive control for OutboxWorker::notify_reconnected_and_flush under User B
    client.outbox.stop();
    recorded_hits.lock().clear();
    client.network_listener.set_online(false);
    client
        .outbox
        .enqueue(&make_rich_req(700, "pos-outbox", Some(99003)), 99003)
        .unwrap();
    let flushed = rt
        .block_on(client.outbox.notify_reconnected_and_flush())
        .expect("positive notify_reconnected_and_flush must succeed");
    assert!(client.network_listener.is_online());
    assert_eq!(flushed.len(), 1);
    assert_eq!(
        client.outbox.store().get(99003).unwrap().unwrap().status,
        OutboxStatus::Delivered
    );
    let outbox_hits = recorded_hits.lock().clone();
    assert!(
        outbox_hits.iter().any(|(m, p, auth, _)| {
            m == "POST"
                && p == "/api/v1/messages/send"
                && auth.to_ascii_lowercase() == "authorization: bearer tok-b"
        }),
        "positive outbox flush must send POST /api/v1/messages/send with Bearer tok-b: {outbox_hits:?}"
    );
}

#[test]
fn test_proxy_host_validation_rejects_metacharacters_and_untrimmed_whitespace() {
    use gpui_telegram_client::net::proxy::ClientProxyType;
    use gpui_telegram_client::ProxyEndpointRecord;

    let invalid_hosts = [
        "",
        "evil.com@127.0.0.1",
        "127.0.0.1/path",
        "127.0.0.1\\path",
        "127.0.0.1?q=1",
        "127.0.0.1#frag",
        "http://127.0.0.1",
        "127.0.0.1:8080",
        "[::1]",
        " 127.0.0.1 ",
        "127.0.0.1\r\n",
        "bad host",
        "127.0.0.1%2f",
        "evil.com%40127.0.0.1",
        "127.0.0.1%0d%0a",
        "127.0.0.1,evil.com",
        "127.0.0.1;evil.com",
        "127.0.0.1&port=1",
        "127.0.0.1=evil",
    ];

    for bad_host in invalid_hosts {
        for kind in ["HTTP", "SOCKS5", "MTProto"] {
            let rec = ProxyEndpointRecord {
                id: 1,
                kind: kind.to_string(),
                host: bad_host.to_string(),
                port: 1080,
                secret_or_user: String::new(),
                username: None,
                password: None,
                ping_ms: 0,
                enabled: true,
            };
            assert!(
                ClientProxyConfig::validate_record(&rec).is_err(),
                "validate_record must reject kind={kind} host={bad_host:?}"
            );
            assert!(
                ClientProxyConfig::from_record(&rec).is_none(),
                "from_record must return None for kind={kind} host={bad_host:?}"
            );
        }

        for proxy_type in [ClientProxyType::Http, ClientProxyType::Socks5] {
            let cfg = ClientProxyConfig {
                proxy_type,
                host: bad_host.to_string(),
                port: 1080,
                username: None,
                password: None,
                fallback_to_direct: false,
            };
            assert!(
                cfg.validate().is_err(),
                "ClientProxyConfig::validate must reject {proxy_type:?} host={bad_host:?}"
            );
            assert!(
                cfg.to_reqwest_proxy().is_err(),
                "ClientProxyConfig::to_reqwest_proxy must reject {proxy_type:?} host={bad_host:?}"
            );
        }
    }

    let valid_hosts = ["127.0.0.1", "proxy.example.com", "::1", "2001:db8::1"];
    for good_host in valid_hosts {
        for kind in ["HTTP", "SOCKS5"] {
            let rec = ProxyEndpointRecord {
                id: 2,
                kind: kind.to_string(),
                host: good_host.to_string(),
                port: 1080,
                secret_or_user: String::new(),
                username: None,
                password: None,
                ping_ms: 0,
                enabled: true,
            };
            let validated = ClientProxyConfig::validate_record(&rec)
                .unwrap_or_else(|e| panic!("valid {kind} host {good_host:?} failed: {e}"));
            let cfg = validated.expect("enabled HTTP/SOCKS5 must return Some(cfg)");
            assert_eq!(cfg.host, good_host);
            assert!(cfg.validate().is_ok());
            assert!(cfg.to_reqwest_proxy().unwrap().is_some());
            assert!(ClientProxyConfig::from_record(&rec).is_some());
        }

        let mt_rec = ProxyEndpointRecord {
            id: 3,
            kind: "MTProto".to_string(),
            host: good_host.to_string(),
            port: 443,
            secret_or_user: "ee1122".to_string(),
            username: None,
            password: None,
            ping_ms: 0,
            enabled: true,
        };
        assert_eq!(
            ClientProxyConfig::validate_record(&mt_rec).unwrap(),
            None,
            "valid MTProto host {good_host:?} must pass validate_record with Ok(None)"
        );
    }
}

#[test]
fn t5_54_tailnet_p2p_session_fence_blocks_approval_pending_and_late_exec_response() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STD, Engine as _};
    use gpui_telegram_client::net::tailnet::{
        open_tswg_packet, seal_tswg_packet, ReplayWindow, TailnetIdentity,
    };
    use gpui_telegram_core::error::TelegramError;
    use gpui_telegram_proto::computer_contracts::{
        RemoteComputerCommandRequest, RemoteComputerCommandResult, RemoteComputerOp,
    };
    use gpui_telegram_proto::tailnet_contracts::{
        DerpClientFrame, DerpServerFrame, PeerApiFrame, TailnetNetMap, TailnetPeerDescriptor,
        TailnetRelayMode,
    };
    use x25519_dalek::StaticSecret;

    let rt = tokio::runtime::Runtime::new().expect("rt");
    let peer_node_seed = [8u8; 32];
    let peer_id = TailnetIdentity::from_seeds([7u8; 32], peer_node_seed, [9u8; 32]);
    let peer_node_sec = StaticSecret::from(peer_node_seed);

    let http_fallback_hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let approval_sent: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let auto_send_exec_after_approval: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let latest_req: Arc<Mutex<Option<(String, [u8; 32])>>> = Arc::new(Mutex::new(None));
    let inject_tx_slot: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<PeerApiFrame>>>> =
        Arc::new(Mutex::new(None));

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();

    let fb_srv = Arc::clone(&http_fallback_hits);
    let app_sent_srv = Arc::clone(&approval_sent);
    let auto_exec_srv = Arc::clone(&auto_send_exec_after_approval);
    let latest_req_srv = Arc::clone(&latest_req);
    let inject_slot_srv = Arc::clone(&inject_tx_slot);
    let peer_id_srv = peer_id.clone();

    rt.spawn(async move {
        let mut seq_counter = 1u64;
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let fb_hits = Arc::clone(&fb_srv);
            let app_sent = Arc::clone(&app_sent_srv);
            let auto_exec = Arc::clone(&auto_exec_srv);
            let latest_req = Arc::clone(&latest_req_srv);
            let inject_slot = Arc::clone(&inject_slot_srv);
            let peer_id = peer_id_srv.clone();
            let peer_sec = peer_node_sec.clone();
            seq_counter += 100;
            let mut local_seq = seq_counter;

            tokio::spawn(async move {
                let mut peek_buf = [0u8; 64];
                let pn = stream.peek(&mut peek_buf).await.unwrap_or(0);
                let peek_str = String::from_utf8_lossy(&peek_buf[..pn]);
                if peek_str.starts_with("GET /api/v1/tailnet/derp") {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    let (inj_tx, mut inj_rx) =
                        tokio::sync::mpsc::unbounded_channel::<PeerApiFrame>();
                    *inject_slot.lock() = Some(inj_tx);

                    let mut client_pub_opt: Option<[u8; 32]> = None;
                    let mut replay_win: HashMap<String, ReplayWindow> = HashMap::new();

                    let peer_desc = TailnetPeerDescriptor {
                        node_id: "node-peer-a".to_string(),
                        device_id: "dev-peer-a".to_string(),
                        display_name: "Peer A".to_string(),
                        node_public_key: peer_id.node_public_key_hex.clone(),
                        disco_public_key: peer_id.disco_public_key_hex.clone(),
                        derp_region: "local".to_string(),
                        derp_connected: true,
                        online: true,
                        relay_mode: TailnetRelayMode::DerpRelay,
                        ..Default::default()
                    };

                    loop {
                        tokio::select! {
                            Some(injected) = inj_rx.recv() => {
                                if let Some(client_pub) = client_pub_opt {
                                    local_seq += 1;
                                    let pt = serde_json::to_vec(&injected).unwrap();
                                    let pkt = seal_tswg_packet(
                                        &peer_sec,
                                        &peer_id.node_public_bytes,
                                        &client_pub,
                                        local_seq,
                                        &pt,
                                    )
                                    .unwrap();
                                    let frame = DerpServerFrame::RecvPacket {
                                        src_node_key: peer_id.node_public_key_hex.clone(),
                                        packet_b64: BASE64_STD.encode(&pkt),
                                    };
                                    let _ = ws
                                        .send(WsMessage::Text(serde_json::to_string(&frame).unwrap().into()))
                                        .await;
                                }
                            }
                            msg_opt = ws.next() => {
                                let Some(Ok(msg)) = msg_opt else {
                                    break;
                                };
                                let WsMessage::Text(txt) = msg else {
                                    continue;
                                };
                                let Ok(cli_frame) = serde_json::from_str::<DerpClientFrame>(&txt) else {
                                    continue;
                                };
                                match cli_frame {
                                    DerpClientFrame::Hello { node_public_key, .. } => {
                                        if let Ok(bytes) = (0..node_public_key.len())
                                            .step_by(2)
                                            .map(|i| u8::from_str_radix(&node_public_key[i..i + 2], 16))
                                            .collect::<Result<Vec<u8>, _>>()
                                        {
                                            if bytes.len() == 32 {
                                                let mut arr = [0u8; 32];
                                                arr.copy_from_slice(&bytes);
                                                client_pub_opt = Some(arr);
                                            }
                                        }
                                        let netmap = TailnetNetMap {
                                            owner_user_id: 1,
                                            seq: 1,
                                            generated_at_ms: 1_700_000_000_000,
                                            derp_region: "local".to_string(),
                                            derp_ws_path: "/api/v1/tailnet/derp".to_string(),
                                            stun_http_path: "/api/v1/tailnet/stun".to_string(),
                                            self_peer: None,
                                            peers: vec![peer_desc.clone()],
                                        };
                                        let welcome = DerpServerFrame::Welcome {
                                            derp_region: "local".to_string(),
                                            server_time_ms: 1_700_000_000_000,
                                            netmap,
                                        };
                                        let _ = ws
                                            .send(WsMessage::Text(serde_json::to_string(&welcome).unwrap().into()))
                                            .await;
                                    }
                                    DerpClientFrame::SendPacket { packet_b64, .. } => {
                                        let Ok(pkt) = BASE64_STD.decode(packet_b64.as_bytes()) else {
                                            continue;
                                        };
                                        let Ok((_, pt)) = open_tswg_packet(
                                            &peer_sec,
                                            &peer_id.node_public_bytes,
                                            &pkt,
                                            &mut replay_win,
                                        ) else {
                                            continue;
                                        };
                                        let Ok(PeerApiFrame::ExecRequest { request_id, .. }) =
                                            serde_json::from_slice::<PeerApiFrame>(&pt)
                                        else {
                                            continue;
                                        };
                                        let Some(client_pub) = client_pub_opt else {
                                            continue;
                                        };
                                        *latest_req.lock() = Some((request_id.clone(), client_pub));

                                        local_seq += 1;
                                        let pending = PeerApiFrame::ApprovalPending {
                                            request_id: request_id.clone(),
                                            command_id: format!("cmd-{request_id}"),
                                            message: "Waiting for approval".to_string(),
                                        };
                                        let pending_pkt = seal_tswg_packet(
                                            &peer_sec,
                                            &peer_id.node_public_bytes,
                                            &client_pub,
                                            local_seq,
                                            &serde_json::to_vec(&pending).unwrap(),
                                        )
                                        .unwrap();
                                        let recv_pending = DerpServerFrame::RecvPacket {
                                            src_node_key: peer_id.node_public_key_hex.clone(),
                                            packet_b64: BASE64_STD.encode(&pending_pkt),
                                        };
                                        let _ = ws
                                            .send(WsMessage::Text(
                                                serde_json::to_string(&recv_pending).unwrap().into(),
                                            ))
                                            .await;
                                        app_sent.fetch_add(1, Ordering::SeqCst);

                                        if auto_exec.load(Ordering::SeqCst) {
                                            local_seq += 1;
                                            let resp_frame = PeerApiFrame::ExecResponse {
                                                request_id: request_id.clone(),
                                                result: RemoteComputerCommandResult {
                                                    command_id: request_id,
                                                    node_id: "node-peer-a".to_string(),
                                                    ok: true,
                                                    exit_code: 0,
                                                    stdout: "p2p-ok".to_string(),
                                                    stderr: String::new(),
                                                    entries: Vec::new(),
                                                    frame_data_url: None,
                                                    executed_by: "dev-peer-a".to_string(),
                                                    duration_ms: 1,
                                                },
                                            };
                                            let resp_pkt = seal_tswg_packet(
                                                &peer_sec,
                                                &peer_id.node_public_bytes,
                                                &client_pub,
                                                local_seq,
                                                &serde_json::to_vec(&resp_frame).unwrap(),
                                            )
                                            .unwrap();
                                            let recv_resp = DerpServerFrame::RecvPacket {
                                                src_node_key: peer_id.node_public_key_hex.clone(),
                                                packet_b64: BASE64_STD.encode(&resp_pkt),
                                            };
                                            let _ = ws
                                                .send(WsMessage::Text(
                                                    serde_json::to_string(&recv_resp).unwrap().into(),
                                                ))
                                                .await;
                                        }
                                    }
                                    DerpClientFrame::KeepAlive { client_time_ms, .. } => {
                                        let ack = DerpServerFrame::KeepAliveAck {
                                            client_time_ms,
                                            server_time_ms: 1_700_000_000_000,
                                        };
                                        let _ = ws
                                            .send(WsMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                                            .await;
                                    }
                                    DerpClientFrame::CommandResult { .. } => {}
                                }
                            }
                        }
                    }
                    return;
                }

                let (method, path, _headers, _body) = read_http_request(&mut stream).await;
                if path == "/api/v1/tailnet/register" || path == "/api/v1/tailnet/netmap" {
                    let peer_desc = TailnetPeerDescriptor {
                        node_id: "node-peer-a".to_string(),
                        device_id: "dev-peer-a".to_string(),
                        display_name: "Peer A".to_string(),
                        node_public_key: peer_id.node_public_key_hex.clone(),
                        disco_public_key: peer_id.disco_public_key_hex.clone(),
                        derp_region: "local".to_string(),
                        derp_connected: true,
                        online: true,
                        relay_mode: TailnetRelayMode::DerpRelay,
                        ..Default::default()
                    };
                    let netmap = TailnetNetMap {
                        owner_user_id: 1,
                        seq: 1,
                        generated_at_ms: 1_700_000_000_000,
                        derp_region: "local".to_string(),
                        derp_ws_path: "/api/v1/tailnet/derp".to_string(),
                        stun_http_path: "/api/v1/tailnet/stun".to_string(),
                        self_peer: None,
                        peers: vec![peer_desc],
                    };
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        &serde_json::to_string(&netmap).unwrap(),
                    )
                    .await;
                } else if path == "/api/v1/tailnet/stun" {
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        r#"{"observed_ip":"127.0.0.1","observed_port":12345,"server_time_ms":1700000000000}"#,
                    )
                    .await;
                } else if path == "/api/v1/computers/exec"
                    || path == "/api/v1/computers/node-peer-a/test"
                {
                    fb_hits.lock().push(format!("{method} {path}"));
                    write_json_response(&mut stream, 500, "Internal Server Error", "{}").await;
                } else {
                    write_json_response(&mut stream, 200, "OK", "{}").await;
                }
            });
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = ClientConfig::default();
    cfg.server_url = format!("http://{addr}");
    let client = Arc::new(TelegramClient::open_with_dir(cfg, dir.path()).unwrap());
    client.set_tailnet_force_derp_only(true);

    let wait_derp = |c: &TelegramClient| {
        rt.block_on(async {
            for _ in 0..100 {
                if c.tailnet_status().derp_connected {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            false
        })
    };

    let wait_approval = |target: usize| {
        rt.block_on(async {
            for _ in 0..150 {
                if approval_sent.load(Ordering::SeqCst) >= target {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
            false
        })
    };

    let req = RemoteComputerCommandRequest {
        node_id: "node-peer-a".to_string(),
        caller_device_id: None,
        caller_bot_id: None,
        caller_label: "Test".to_string(),
        operation: RemoteComputerOp::TestConnection,
    };

    // Sub-case A: ApprovalPending + clear_session() (exec + test_connection)
    client.set_auth_session(make_test_auth(1, "alice", "tok-a"));
    client.start_tailnet_mesh();
    assert!(wait_derp(&client), "DERP did not connect for Sub-case A1");
    let epoch_a1 = client.session_epoch();
    let c1 = Arc::clone(&client);
    let r1 = req.clone();
    let jh_a1 = rt.spawn(async move {
        c1.execute_remote_computer_op_with_token("tok-a", epoch_a1, &r1)
            .await
    });
    assert!(wait_approval(1), "ApprovalPending 1 not seen");
    client.clear_session();
    let res_a1 = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), jh_a1).await })
        .expect("Sub-case A1 must complete promptly on clear_session")
        .unwrap();
    assert!(
        matches!(res_a1, Err(TelegramError::Auth(_))),
        "Sub-case A1 must return Auth/Unauthorized, got: {res_a1:?}"
    );

    client.set_auth_session(make_test_auth(1, "alice", "tok-a"));
    client.start_tailnet_mesh();
    assert!(wait_derp(&client), "DERP did not connect for Sub-case A2");
    let epoch_a2 = client.session_epoch();
    let c2 = Arc::clone(&client);
    let jh_a2 = rt.spawn(async move {
        c2.test_computer_connection_with_token("tok-a", epoch_a2, "node-peer-a")
            .await
    });
    assert!(wait_approval(2), "ApprovalPending 2 not seen");
    client.clear_session();
    let res_a2 = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), jh_a2).await })
        .expect("Sub-case A2 must complete promptly on clear_session")
        .unwrap();
    assert!(
        matches!(res_a2, Err(TelegramError::Auth(_))),
        "Sub-case A2 must return Auth/Unauthorized, got: {res_a2:?}"
    );
    assert!(http_fallback_hits.lock().is_empty());

    // Sub-case B: ApprovalPending + set_auth_session(User B)
    client.set_auth_session(make_test_auth(1, "alice", "tok-a"));
    client.start_tailnet_mesh();
    assert!(wait_derp(&client), "DERP did not connect for Sub-case B1");
    let epoch_b1 = client.session_epoch();
    let c3 = Arc::clone(&client);
    let r3 = req.clone();
    let jh_b1 = rt.spawn(async move {
        c3.execute_remote_computer_op_with_token("tok-a", epoch_b1, &r3)
            .await
    });
    assert!(wait_approval(3), "ApprovalPending 3 not seen");
    client.set_auth_session(make_test_auth(2, "bob", "tok-b"));
    let res_b1 = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), jh_b1).await })
        .expect("Sub-case B1 must complete promptly on user switch")
        .unwrap();
    assert!(
        matches!(res_b1, Err(TelegramError::Auth(_))),
        "Sub-case B1 must return Auth/Unauthorized, got: {res_b1:?}"
    );
    assert_eq!(client.token().as_deref(), Some("tok-b"));

    client.set_auth_session(make_test_auth(1, "alice", "tok-a"));
    client.start_tailnet_mesh();
    assert!(wait_derp(&client), "DERP did not connect for Sub-case B2");
    let epoch_b2 = client.session_epoch();
    let c4 = Arc::clone(&client);
    let jh_b2 = rt.spawn(async move {
        c4.test_computer_connection_with_token("tok-a", epoch_b2, "node-peer-a")
            .await
    });
    assert!(wait_approval(4), "ApprovalPending 4 not seen");
    client.set_auth_session(make_test_auth(2, "bob", "tok-b"));
    let res_b2 = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), jh_b2).await })
        .expect("Sub-case B2 must complete promptly on user switch")
        .unwrap();
    assert!(
        matches!(res_b2, Err(TelegramError::Auth(_))),
        "Sub-case B2 must return Auth/Unauthorized, got: {res_b2:?}"
    );
    assert!(http_fallback_hits.lock().is_empty());

    // Sub-case C1: Late ExecResponse racing clear_session() / stop()
    client.set_auth_session(make_test_auth(1, "alice", "tok-a"));
    client.start_tailnet_mesh();
    assert!(wait_derp(&client), "DERP did not connect for Sub-case C1a");
    let epoch_c1 = client.session_epoch();
    let c5 = Arc::clone(&client);
    let r5 = req.clone();
    let jh_c1 = rt.spawn(async move {
        c5.execute_remote_computer_op_with_token("tok-a", epoch_c1, &r5)
            .await
    });
    assert!(wait_approval(5), "ApprovalPending 5 not seen");
    let req_id_c1 = latest_req.lock().as_ref().unwrap().0.clone();
    let secret_exec_c1 = PeerApiFrame::ExecResponse {
        request_id: req_id_c1.clone(),
        result: RemoteComputerCommandResult {
            command_id: req_id_c1,
            node_id: "node-peer-a".to_string(),
            ok: true,
            exit_code: 0,
            stdout: "p2p-secret".to_string(),
            stderr: String::new(),
            entries: Vec::new(),
            frame_data_url: None,
            executed_by: "dev-peer-a".to_string(),
            duration_ms: 1,
        },
    };
    client.clear_session();
    if let Some(tx) = inject_tx_slot.lock().as_ref() {
        let _ = tx.send(secret_exec_c1);
    }
    let res_c1 = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), jh_c1).await })
        .unwrap()
        .unwrap();
    assert!(
        matches!(res_c1, Err(TelegramError::Auth(_))),
        "Sub-case C1a must block late ExecResponse after clear_session: {res_c1:?}"
    );

    // Direct execute_peer_op racing stop() + ExecResponse -> must return None
    client.set_auth_session(make_test_auth(1, "alice", "tok-a"));
    client.start_tailnet_mesh();
    assert!(wait_derp(&client), "DERP did not connect for Sub-case C1b");
    let tn_c1b = Arc::clone(&client.tailnet);
    let r_c1b = req.clone();
    let jh_c1b = rt.spawn(async move { tn_c1b.execute_peer_op(&r_c1b).await });
    assert!(wait_approval(6), "ApprovalPending 6 not seen");
    let req_id_c1b = latest_req.lock().as_ref().unwrap().0.clone();
    client.tailnet.stop();
    if let Some(tx) = inject_tx_slot.lock().as_ref() {
        let _ = tx.send(PeerApiFrame::ExecResponse {
            request_id: req_id_c1b.clone(),
            result: RemoteComputerCommandResult {
                command_id: req_id_c1b,
                node_id: "node-peer-a".to_string(),
                ok: true,
                exit_code: 0,
                stdout: "p2p-secret".to_string(),
                stderr: String::new(),
                entries: Vec::new(),
                frame_data_url: None,
                executed_by: "dev-peer-a".to_string(),
                duration_ms: 1,
            },
        });
    }
    let res_c1b = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), jh_c1b).await })
        .unwrap()
        .unwrap();
    assert!(
        res_c1b.is_none(),
        "execute_peer_op must return None after stop(): {res_c1b:?}"
    );

    // Sub-case C2: set_token(Some("tok-b")) WITHOUT stopping tailnet + ExecResponse delivered
    client.set_auth_session(make_test_auth(1, "alice", "tok-a"));
    client.start_tailnet_mesh();
    assert!(wait_derp(&client), "DERP did not connect for Sub-case C2a");
    let epoch_c2a = client.session_epoch();
    let c6 = Arc::clone(&client);
    let r6 = req.clone();
    let jh_c2a = rt.spawn(async move {
        c6.execute_remote_computer_op_with_token("tok-a", epoch_c2a, &r6)
            .await
    });
    assert!(wait_approval(7), "ApprovalPending 7 not seen");
    let req_id_c2a = latest_req.lock().as_ref().unwrap().0.clone();
    client.set_token(Some("tok-b".to_string()));
    if let Some(tx) = inject_tx_slot.lock().as_ref() {
        let _ = tx.send(PeerApiFrame::ExecResponse {
            request_id: req_id_c2a.clone(),
            result: RemoteComputerCommandResult {
                command_id: req_id_c2a,
                node_id: "node-peer-a".to_string(),
                ok: true,
                exit_code: 0,
                stdout: "p2p-secret".to_string(),
                stderr: String::new(),
                entries: Vec::new(),
                frame_data_url: None,
                executed_by: "dev-peer-a".to_string(),
                duration_ms: 1,
            },
        });
    }
    let res_c2a = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), jh_c2a).await })
        .unwrap()
        .unwrap();
    assert!(
        matches!(res_c2a, Err(TelegramError::Auth(_))),
        "Sub-case C2a must block ExecResponse when token rotated via set_token: {res_c2a:?}"
    );

    client.set_token(Some("tok-a".to_string()));
    let epoch_c2b = client.session_epoch();
    let c7 = Arc::clone(&client);
    let jh_c2b = rt.spawn(async move {
        c7.test_computer_connection_with_token("tok-a", epoch_c2b, "node-peer-a")
            .await
    });
    assert!(wait_approval(8), "ApprovalPending 8 not seen");
    let req_id_c2b = latest_req.lock().as_ref().unwrap().0.clone();
    client.set_token(Some("tok-b".to_string()));
    if let Some(tx) = inject_tx_slot.lock().as_ref() {
        let _ = tx.send(PeerApiFrame::ExecResponse {
            request_id: req_id_c2b.clone(),
            result: RemoteComputerCommandResult {
                command_id: req_id_c2b,
                node_id: "node-peer-a".to_string(),
                ok: true,
                exit_code: 0,
                stdout: "p2p-secret".to_string(),
                stderr: String::new(),
                entries: Vec::new(),
                frame_data_url: None,
                executed_by: "dev-peer-a".to_string(),
                duration_ms: 1,
            },
        });
    }
    let res_c2b = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), jh_c2b).await })
        .unwrap()
        .unwrap();
    assert!(
        matches!(res_c2b, Err(TelegramError::Auth(_))),
        "Sub-case C2b must block test_computer_connection ExecResponse on set_token: {res_c2b:?}"
    );
    assert!(http_fallback_hits.lock().is_empty());

    // Sub-case D: Positive control under User B
    client.set_auth_session(make_test_auth(2, "bob", "tok-b"));
    client.start_tailnet_mesh();
    assert!(wait_derp(&client), "DERP did not connect for Sub-case D");
    auto_send_exec_after_approval.store(true, Ordering::SeqCst);

    let ep_d1 = client.session_epoch();
    let pos_exec = rt
        .block_on(client.execute_remote_computer_op_with_token("tok-b", ep_d1, &req))
        .expect("positive execute_remote_computer_op_with_token must succeed");
    assert!(pos_exec.ok && pos_exec.stdout == "p2p-ok");

    let ep_d2 = client.session_epoch();
    let pos_test = rt
        .block_on(client.test_computer_connection_with_token("tok-b", ep_d2, "node-peer-a"))
        .expect("positive test_computer_connection_with_token must succeed");
    assert!(pos_test.ok && pos_test.stdout == "p2p-ok");
    assert!(http_fallback_hits.lock().is_empty());

    client.stop_tailnet_mesh();
}

#[test]
fn t5_55_tailnet_derp_ws_loop_generation_fence() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STD, Engine as _};
    use gpui_telegram_client::net::tailnet::{
        open_tswg_packet, seal_tswg_packet, ReplayWindow, TailnetIdentity,
    };
    use gpui_telegram_proto::computer_contracts::{
        RemoteComputerCommandRequest, RemoteComputerCommandResult, RemoteComputerOp,
    };
    use gpui_telegram_proto::tailnet_contracts::{
        DerpClientFrame, DerpServerFrame, PeerApiFrame, TailnetNetMap, TailnetPeerDescriptor,
        TailnetRelayMode,
    };
    use x25519_dalek::StaticSecret;

    #[derive(Clone, Copy)]
    enum SlowDerpAction {
        Return401,
        UpgradeAndSendStaleFrames,
    }

    let rt = tokio::runtime::Runtime::new().expect("rt");
    let peer_node_seed = [11u8; 32];
    let peer_id = TailnetIdentity::from_seeds([10u8; 32], peer_node_seed, [12u8; 32]);
    let peer_node_sec = StaticSecret::from(peer_node_seed);

    let (gen1_tx, gen1_rx) = tokio::sync::oneshot::channel::<SlowDerpAction>();
    let (gen3_tx, gen3_rx) = tokio::sync::oneshot::channel::<SlowDerpAction>();
    let gen1_rx_slot = Arc::new(tokio::sync::Mutex::new(Some(gen1_rx)));
    let gen3_rx_slot = Arc::new(tokio::sync::Mutex::new(Some(gen3_rx)));
    let gen1_arrived = Arc::new(AtomicUsize::new(0));
    let gen3_arrived = Arc::new(AtomicUsize::new(0));
    let gen1_arrived_notify = Arc::new(tokio::sync::Notify::new());
    let gen3_arrived_notify = Arc::new(tokio::sync::Notify::new());

    let stale_ws_sendpacket_hits = Arc::new(AtomicUsize::new(0));
    let live_ws_sendpacket_hits = Arc::new(AtomicUsize::new(0));
    let http_fallback_hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();

    let gen1_rx_srv = Arc::clone(&gen1_rx_slot);
    let gen3_rx_srv = Arc::clone(&gen3_rx_slot);
    let gen1_arr_srv = Arc::clone(&gen1_arrived);
    let gen3_arr_srv = Arc::clone(&gen3_arrived);
    let gen1_ntf_srv = Arc::clone(&gen1_arrived_notify);
    let gen3_ntf_srv = Arc::clone(&gen3_arrived_notify);
    let stale_hits_srv = Arc::clone(&stale_ws_sendpacket_hits);
    let live_hits_srv = Arc::clone(&live_ws_sendpacket_hits);
    let fb_srv = Arc::clone(&http_fallback_hits);
    let peer_id_srv = peer_id.clone();

    rt.spawn(async move {
        let mut seq_counter = 10u64;
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let gen1_rx_srv = Arc::clone(&gen1_rx_srv);
            let gen3_rx_srv = Arc::clone(&gen3_rx_srv);
            let gen1_arr_srv = Arc::clone(&gen1_arr_srv);
            let gen3_arr_srv = Arc::clone(&gen3_arr_srv);
            let gen1_ntf_srv = Arc::clone(&gen1_ntf_srv);
            let gen3_ntf_srv = Arc::clone(&gen3_ntf_srv);
            let stale_hits_srv = Arc::clone(&stale_hits_srv);
            let live_hits_srv = Arc::clone(&live_hits_srv);
            let fb_hits = Arc::clone(&fb_srv);
            let peer_id = peer_id_srv.clone();
            let peer_sec = peer_node_sec.clone();
            seq_counter += 100;
            let mut local_seq = seq_counter;

            tokio::spawn(async move {
                let mut peek_buf = [0u8; 2048];
                let mut pn = 0usize;
                for _ in 0..50 {
                    pn = stream.peek(&mut peek_buf).await.unwrap_or(0);
                    if peek_buf[..pn].windows(2).any(|w| w == b"\r\n") {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                let first_line = String::from_utf8_lossy(&peek_buf[..pn])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();

                if first_line.starts_with("GET /api/v1/tailnet/derp") {
                    if first_line.contains("token=tok-gen1-slow") {
                        gen1_arr_srv.fetch_add(1, Ordering::SeqCst);
                        gen1_ntf_srv.notify_one();
                        let rx_opt = gen1_rx_srv.lock().await.take();
                        let action = match rx_opt {
                            Some(rx) => rx.await.unwrap_or(SlowDerpAction::Return401),
                            None => SlowDerpAction::Return401,
                        };
                        match action {
                            SlowDerpAction::Return401 => {
                                let _ = stream
                                    .write_all(
                                        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                    )
                                    .await;
                                let _ = stream.flush().await;
                                return;
                            }
                            SlowDerpAction::UpgradeAndSendStaleFrames => {}
                        }
                        return;
                    }

                    if first_line.contains("token=tok-gen3-slow") {
                        gen3_arr_srv.fetch_add(1, Ordering::SeqCst);
                        gen3_ntf_srv.notify_one();
                        let rx_opt = gen3_rx_srv.lock().await.take();
                        let action = match rx_opt {
                            Some(rx) => rx.await.unwrap_or(SlowDerpAction::Return401),
                            None => SlowDerpAction::Return401,
                        };
                        if matches!(action, SlowDerpAction::UpgradeAndSendStaleFrames) {
                            if let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await {
                                let stale_peer = TailnetPeerDescriptor {
                                    node_id: "node-peer-gen3".to_string(),
                                    device_id: "dev-peer-gen3".to_string(),
                                    display_name: "Peer Gen3".to_string(),
                                    node_public_key: peer_id.node_public_key_hex.clone(),
                                    disco_public_key: peer_id.disco_public_key_hex.clone(),
                                    derp_region: "local".to_string(),
                                    derp_connected: true,
                                    online: true,
                                    relay_mode: TailnetRelayMode::DerpRelay,
                                    ..Default::default()
                                };
                                let stale_welcome = DerpServerFrame::Welcome {
                                    derp_region: "local".to_string(),
                                    server_time_ms: 1_700_000_000_000,
                                    netmap: TailnetNetMap {
                                        owner_user_id: 1,
                                        seq: 999,
                                        generated_at_ms: 1_700_000_000_000,
                                        derp_region: "local".to_string(),
                                        derp_ws_path: "/api/v1/tailnet/derp".to_string(),
                                        stun_http_path: "/api/v1/tailnet/stun".to_string(),
                                        self_peer: None,
                                        peers: vec![stale_peer.clone()],
                                    },
                                };
                                let _ = ws
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&stale_welcome).unwrap().into(),
                                    ))
                                    .await;
                                let stale_update = DerpServerFrame::NetMapUpdate {
                                    netmap: TailnetNetMap {
                                        owner_user_id: 1,
                                        seq: 1000,
                                        generated_at_ms: 1_700_000_000_000,
                                        derp_region: "local".to_string(),
                                        derp_ws_path: "/api/v1/tailnet/derp".to_string(),
                                        stun_http_path: "/api/v1/tailnet/stun".to_string(),
                                        self_peer: None,
                                        peers: vec![stale_peer],
                                    },
                                };
                                let _ = ws
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&stale_update).unwrap().into(),
                                    ))
                                    .await;
                                while let Some(Ok(msg)) = ws.next().await {
                                    if let WsMessage::Text(txt) = msg {
                                        if let Ok(DerpClientFrame::SendPacket { .. }) =
                                            serde_json::from_str::<DerpClientFrame>(&txt)
                                        {
                                            stale_hits_srv.fetch_add(1, Ordering::SeqCst);
                                        }
                                    }
                                }
                            }
                        }
                        return;
                    }

                    let is_gen4 = first_line.contains("token=tok-gen4-fast");
                    let (peer_node_id, peer_dev_id, map_seq) = if is_gen4 {
                        ("node-peer-gen4", "dev-peer-gen4", 400u64)
                    } else {
                        ("node-peer-gen2", "dev-peer-gen2", 200u64)
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    let mut client_pub_opt: Option<[u8; 32]> = None;
                    let mut replay_win: HashMap<String, ReplayWindow> = HashMap::new();
                    let peer_desc = TailnetPeerDescriptor {
                        node_id: peer_node_id.to_string(),
                        device_id: peer_dev_id.to_string(),
                        display_name: "Peer Live".to_string(),
                        node_public_key: peer_id.node_public_key_hex.clone(),
                        disco_public_key: peer_id.disco_public_key_hex.clone(),
                        derp_region: "local".to_string(),
                        derp_connected: true,
                        online: true,
                        relay_mode: TailnetRelayMode::DerpRelay,
                        ..Default::default()
                    };

                    while let Some(Ok(msg)) = ws.next().await {
                        let WsMessage::Text(txt) = msg else {
                            continue;
                        };
                        let Ok(cli_frame) = serde_json::from_str::<DerpClientFrame>(&txt) else {
                            continue;
                        };
                        match cli_frame {
                            DerpClientFrame::Hello { node_public_key, .. } => {
                                if let Ok(bytes) = (0..node_public_key.len())
                                    .step_by(2)
                                    .map(|i| u8::from_str_radix(&node_public_key[i..i + 2], 16))
                                    .collect::<Result<Vec<u8>, _>>()
                                {
                                    if let Ok(arr) = <[u8; 32]>::try_from(bytes) {
                                        client_pub_opt = Some(arr);
                                    }
                                }
                                let welcome = DerpServerFrame::Welcome {
                                    derp_region: "local".to_string(),
                                    server_time_ms: 1_700_000_000_000,
                                    netmap: TailnetNetMap {
                                        owner_user_id: 2,
                                        seq: map_seq,
                                        generated_at_ms: 1_700_000_000_000,
                                        derp_region: "local".to_string(),
                                        derp_ws_path: "/api/v1/tailnet/derp".to_string(),
                                        stun_http_path: "/api/v1/tailnet/stun".to_string(),
                                        self_peer: None,
                                        peers: vec![peer_desc.clone()],
                                    },
                                };
                                let _ = ws
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&welcome).unwrap().into(),
                                    ))
                                    .await;
                            }
                            DerpClientFrame::KeepAlive { client_time_ms, .. } => {
                                let ack = DerpServerFrame::KeepAliveAck {
                                    client_time_ms,
                                    server_time_ms: 1_700_000_000_000,
                                };
                                let _ = ws
                                    .send(WsMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                                    .await;
                            }
                            DerpClientFrame::SendPacket { packet_b64, .. } => {
                                live_hits_srv.fetch_add(1, Ordering::SeqCst);
                                let Ok(pkt) = BASE64_STD.decode(packet_b64.as_bytes()) else {
                                    continue;
                                };
                                let Ok((_, pt)) = open_tswg_packet(
                                    &peer_sec,
                                    &peer_id.node_public_bytes,
                                    &pkt,
                                    &mut replay_win,
                                ) else {
                                    continue;
                                };
                                let Ok(PeerApiFrame::ExecRequest { request_id, .. }) =
                                    serde_json::from_slice::<PeerApiFrame>(&pt)
                                else {
                                    continue;
                                };
                                let Some(client_pub) = client_pub_opt else {
                                    continue;
                                };
                                local_seq += 1;
                                let resp_frame = PeerApiFrame::ExecResponse {
                                    request_id: request_id.clone(),
                                    result: RemoteComputerCommandResult {
                                        command_id: request_id,
                                        node_id: peer_node_id.to_string(),
                                        ok: true,
                                        exit_code: 0,
                                        stdout: "gen4-p2p-ok".to_string(),
                                        stderr: String::new(),
                                        entries: Vec::new(),
                                        frame_data_url: None,
                                        executed_by: peer_dev_id.to_string(),
                                        duration_ms: 1,
                                    },
                                };
                                let resp_pkt = seal_tswg_packet(
                                    &peer_sec,
                                    &peer_id.node_public_bytes,
                                    &client_pub,
                                    local_seq,
                                    &serde_json::to_vec(&resp_frame).unwrap(),
                                )
                                .unwrap();
                                let recv_resp = DerpServerFrame::RecvPacket {
                                    src_node_key: peer_id.node_public_key_hex.clone(),
                                    packet_b64: BASE64_STD.encode(&resp_pkt),
                                };
                                let _ = ws
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&recv_resp).unwrap().into(),
                                    ))
                                    .await;
                            }
                            DerpClientFrame::CommandResult { .. } => {}
                        }
                    }
                    return;
                }

                let (method, path, headers, _body) = read_http_request(&mut stream).await;
                if path == "/api/v1/tailnet/register" || path == "/api/v1/tailnet/netmap" {
                    let (owner_uid, seq, node_id, dev_id) = if headers.contains("tok-gen4-fast") {
                        (2u64, 400u64, "node-peer-gen4", "dev-peer-gen4")
                    } else if headers.contains("tok-gen3-slow") {
                        (1u64, 300u64, "node-peer-gen3", "dev-peer-gen3")
                    } else if headers.contains("tok-gen2-fast") {
                        (2u64, 200u64, "node-peer-gen2", "dev-peer-gen2")
                    } else {
                        (1u64, 100u64, "node-peer-gen1", "dev-peer-gen1")
                    };
                    let peer_desc = TailnetPeerDescriptor {
                        node_id: node_id.to_string(),
                        device_id: dev_id.to_string(),
                        display_name: "Peer".to_string(),
                        node_public_key: peer_id.node_public_key_hex.clone(),
                        disco_public_key: peer_id.disco_public_key_hex.clone(),
                        derp_region: "local".to_string(),
                        derp_connected: true,
                        online: true,
                        relay_mode: TailnetRelayMode::DerpRelay,
                        ..Default::default()
                    };
                    let netmap = TailnetNetMap {
                        owner_user_id: owner_uid,
                        seq,
                        generated_at_ms: 1_700_000_000_000,
                        derp_region: "local".to_string(),
                        derp_ws_path: "/api/v1/tailnet/derp".to_string(),
                        stun_http_path: "/api/v1/tailnet/stun".to_string(),
                        self_peer: None,
                        peers: vec![peer_desc],
                    };
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        &serde_json::to_string(&netmap).unwrap(),
                    )
                    .await;
                } else if path == "/api/v1/tailnet/stun" {
                    write_json_response(
                        &mut stream,
                        200,
                        "OK",
                        r#"{"observed_ip":"127.0.0.1","observed_port":12345,"server_time_ms":1700000000000}"#,
                    )
                    .await;
                } else if path == "/api/v1/computers/exec"
                    || (path.starts_with("/api/v1/computers/") && path.ends_with("/test"))
                {
                    fb_hits.lock().push(format!("{method} {path}"));
                    write_json_response(&mut stream, 500, "Internal Server Error", "{}").await;
                } else {
                    write_json_response(&mut stream, 200, "OK", "{}").await;
                }
            });
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = ClientConfig::default();
    cfg.server_url = format!("http://{addr}");
    let client = Arc::new(TelegramClient::open_with_dir(cfg, dir.path()).unwrap());
    client.set_tailnet_force_derp_only(true);

    // Part (A): Late 401 Failure (PROBE T3)
    client.set_auth_session(make_test_auth(1, "alice", "tok-gen1-slow"));
    client.start_tailnet_mesh();
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while gen1_arrived.load(Ordering::SeqCst) == 0 {
                gen1_arrived_notify.notified().await;
            }
        })
        .await
        .expect("gen 1 slow DERP connect did not arrive");
    });

    client.stop_tailnet_mesh();
    client.set_auth_session(make_test_auth(2, "bob", "tok-gen2-fast"));
    client.start_tailnet_mesh();
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let st = client.tailnet_status();
                if st.derp_connected && st.netmap_seq == 200 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        })
        .await
        .expect("gen 2 DERP did not connect");
    });

    let _ = gen1_tx.send(SlowDerpAction::Return401);
    rt.block_on(async {
        tokio::time::sleep(Duration::from_millis(150)).await;
    });
    let st_a = client.tailnet_status();
    assert!(st_a.running, "gen 2 must remain running after stale gen 1 401");
    assert!(st_a.derp_connected, "gen 2 derp_connected must not be clobbered by gen 1 401");
    assert_eq!(st_a.relay_mode, TailnetRelayMode::DerpRelay);
    assert_eq!(st_a.netmap_seq, 200);

    // Part (B): Late HTTP 101 Upgrade + Stale Welcome/NetMapUpdate + Outbound P2P Routing
    client.stop_tailnet_mesh();
    client.set_auth_session(make_test_auth(1, "alice", "tok-gen3-slow"));
    client.start_tailnet_mesh();
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while gen3_arrived.load(Ordering::SeqCst) == 0 {
                gen3_arrived_notify.notified().await;
            }
        })
        .await
        .expect("gen 3 slow DERP connect did not arrive");
    });

    client.stop_tailnet_mesh();
    client.set_auth_session(make_test_auth(2, "bob", "tok-gen4-fast"));
    client.start_tailnet_mesh();
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let st = client.tailnet_status();
                if st.derp_connected
                    && st.netmap_seq == 400
                    && st.peers.iter().any(|p| p.node_id == "node-peer-gen4")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        })
        .await
        .expect("gen 4 DERP did not connect");
    });

    let _ = gen3_tx.send(SlowDerpAction::UpgradeAndSendStaleFrames);
    rt.block_on(async {
        tokio::time::sleep(Duration::from_millis(150)).await;
    });
    let st_b = client.tailnet_status();
    assert!(st_b.derp_connected);
    assert_eq!(st_b.relay_mode, TailnetRelayMode::DerpRelay);
    assert_eq!(st_b.netmap_seq, 400, "stale gen 3 netmap must not overwrite gen 4");
    assert!(st_b.peers.iter().any(|p| p.node_id == "node-peer-gen4"));
    assert!(!st_b.peers.iter().any(|p| p.node_id == "node-peer-gen3"));

    let ep4 = client.session_epoch();
    let req4 = RemoteComputerCommandRequest {
        node_id: "node-peer-gen4".to_string(),
        caller_device_id: None,
        caller_bot_id: None,
        caller_label: "Test".to_string(),
        operation: RemoteComputerOp::TestConnection,
    };
    let res4 = rt
        .block_on(client.execute_remote_computer_op_with_token("tok-gen4-fast", ep4, &req4))
        .expect("gen 4 P2P execute_remote_computer_op_with_token must succeed");
    assert!(res4.ok && res4.stdout == "gen4-p2p-ok");
    assert_eq!(live_ws_sendpacket_hits.load(Ordering::SeqCst), 1);
    assert_eq!(stale_ws_sendpacket_hits.load(Ordering::SeqCst), 0);
    assert!(http_fallback_hits.lock().is_empty());

    client.stop_tailnet_mesh();
}

#[test]
fn test_mtproto_secret_validation_and_tg_deep_link_encoding_and_socks5_creds() {
    use gpui_telegram_client::ProxyEndpointRecord;
    use gpui_telegram_core::core::deep_links::DeepLinksRouterDeepLinks;

    let invalid_secrets = [
        "u&port=99",
        "u@a/b?c#d",
        "u,a;b",
        "u\r\nx",
        "u\0x",
        "u:1",
        " ee1122 ",
        "=",
        "==",
        "a=b",
    ];
    for bad in invalid_secrets {
        let rec = ProxyEndpointRecord {
            id: 1,
            kind: "MTProto".into(),
            host: "mt.example.com".into(),
            port: 443,
            secret_or_user: bad.into(),
            username: None,
            password: None,
            ping_ms: 0,
            enabled: true,
        };
        assert!(
            ClientProxyConfig::validate_record(&rec).is_err(),
            "MTProto secret {bad:?} must be rejected"
        );
    }

    let valid_secrets = [
        "",
        "ee1122",
        "0123456789abcdef0123456789abcdef",
        "dd0123456789abcdef0123456789abcdef",
        "ee0123456789abcdef0123456789abcdef676f6f676c652e636f6d",
        "7jK-_1234567890abcdef",
        "7gAAAAAAAAAAAAAAAAAAAAB=",
        "7gAAAAAAAAAAAAAAAAAAAA==",
    ];
    for good in valid_secrets {
        let rec = ProxyEndpointRecord {
            id: 2,
            kind: "MTProto".into(),
            host: "mt.example.com".into(),
            port: 443,
            secret_or_user: good.into(),
            username: None,
            password: None,
            ping_ms: 0,
            enabled: true,
        };
        assert_eq!(
            ClientProxyConfig::validate_record(&rec)
                .unwrap_or_else(|e| panic!("MTProto secret {good:?} should be valid: {e}")),
            None
        );
    }

    let s5_full = ProxyEndpointRecord {
        id: 3,
        kind: "SOCKS5".into(),
        host: "127.0.0.1".into(),
        port: 1080,
        secret_or_user: "u_legacy".into(),
        username: Some("realuser".into()),
        password: Some("realpass".into()),
        ping_ms: 0,
        enabled: true,
    };
    assert_eq!(
        s5_full.tg_deep_link(),
        "tg://socks?server=127.0.0.1&port=1080&user=realuser&pass=realpass"
    );

    let s5_legacy = ProxyEndpointRecord {
        id: 4,
        kind: "socks5".into(),
        host: "127.0.0.1".into(),
        port: 1080,
        secret_or_user: "u_legacy".into(),
        username: None,
        password: None,
        ping_ms: 0,
        enabled: true,
    };
    assert_eq!(
        s5_legacy.tg_deep_link(),
        "tg://socks?server=127.0.0.1&port=1080&user=u_legacy"
    );

    let s5_injected = ProxyEndpointRecord {
        id: 5,
        kind: "SOCKS5".into(),
        host: "::1".into(),
        port: 1080,
        secret_or_user: String::new(),
        username: Some("alice&port=9999#frag".into()),
        password: Some("p=1&a=2".into()),
        ping_ms: 0,
        enabled: true,
    };
    let s5_link = s5_injected.tg_deep_link();
    assert!(!s5_link.contains("&port=9999"), "link={s5_link}");
    assert!(!s5_link.contains("#frag"), "link={s5_link}");
    let parsed_s5 = DeepLinksRouterDeepLinks::parse_command(&s5_link, true);
    assert_eq!(parsed_s5.params.get("server").map(String::as_str), Some("::1"));
    assert_eq!(parsed_s5.params.get("port").map(String::as_str), Some("1080"));
    assert_eq!(
        parsed_s5.params.get("user").map(String::as_str),
        Some("alice&port=9999#frag")
    );
    assert_eq!(
        parsed_s5.params.get("pass").map(String::as_str),
        Some("p=1&a=2")
    );

    let mt_empty = ProxyEndpointRecord {
        id: 6,
        kind: "mtproto".into(),
        host: "2001:db8::1".into(),
        port: 443,
        secret_or_user: String::new(),
        username: None,
        password: None,
        ping_ms: 0,
        enabled: true,
    };
    assert_eq!(
        mt_empty.tg_deep_link(),
        "tg://proxy?server=2001%3Adb8%3A%3A1&port=443&secret=ee000000000000000000000000000001"
    );

    let mt_unvalidated = ProxyEndpointRecord {
        id: 7,
        kind: "MTProto".into(),
        host: "mt.example.com".into(),
        port: 443,
        secret_or_user: "u\r\n&port=80#frag".into(),
        username: None,
        password: None,
        ping_ms: 0,
        enabled: true,
    };
    let mt_link = mt_unvalidated.tg_deep_link();
    assert!(!mt_link.contains("&port=80"), "link={mt_link}");
    assert!(!mt_link.contains("#frag"), "link={mt_link}");
    assert!(!mt_link.contains('\r'), "link={mt_link}");
    assert!(!mt_link.contains('\n'), "link={mt_link}");
    let parsed_mt = DeepLinksRouterDeepLinks::parse_command(&mt_link, true);
    assert_eq!(
        parsed_mt.params.get("secret").map(String::as_str),
        Some("u\r\n&port=80#frag")
    );

    use gpui_telegram_client::PersistedClientState;

    for kind in ["Direct", "None", " direct ", "NONE"] {
        let direct_rec = ProxyEndpointRecord {
            id: 8,
            kind: kind.into(),
            host: "149.154.167.50".into(),
            port: 443,
            secret_or_user: String::new(),
            username: None,
            password: None,
            ping_ms: 0,
            enabled: true,
        };
        assert_eq!(
            direct_rec.tg_deep_link(),
            "",
            "kind {kind:?} must emit empty tg_deep_link"
        );
    }
    let demo = PersistedClientState::new_demo();
    assert_eq!(demo.proxies[0].tg_deep_link(), "");

    let s5_utf8 = ProxyEndpointRecord {
        id: 9,
        kind: "SOCKS5".into(),
        host: "127.0.0.1".into(),
        port: 1080,
        secret_or_user: String::new(),
        username: Some("alice_密码".into()),
        password: Some("密码ü🔐".into()),
        ping_ms: 0,
        enabled: true,
    };
    let utf8_link = s5_utf8.tg_deep_link();
    let parsed_utf8 = DeepLinksRouterDeepLinks::parse_command(&utf8_link, true);
    assert_eq!(
        parsed_utf8.params.get("user").map(String::as_str),
        Some("alice_密码")
    );
    assert_eq!(
        parsed_utf8.params.get("pass").map(String::as_str),
        Some("密码ü🔐")
    );
}

// [T5-56] TailnetEngine::apply_netmap_if atomically fences stale netmap writes against clear_session().
#[test]
fn t5_56_apply_netmap_if_epoch_clear_race() {
    use gpui_telegram_proto::tailnet_contracts::{
        TailnetNetMap, TailnetPeerDescriptor, TailnetRelayMode,
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
        .unwrap();

    for iter in 0..50u64 {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cfg = ClientConfig::default();
        cfg.server_url = "http://127.0.0.1:1".to_string();
        let client = Arc::new(TelegramClient::open_with_dir(cfg, dir.path()).unwrap());
        client.set_auth_session(make_test_auth(10 + iter, "alice", "tok-a"));
        let old_epoch = client.session_epoch();

        let peer = TailnetPeerDescriptor {
            node_id: format!("node-stale-{iter}"),
            device_id: format!("dev-stale-{iter}"),
            display_name: "StalePeer".to_string(),
            node_public_key: "01".repeat(32),
            disco_public_key: "02".repeat(32),
            derp_region: "local".to_string(),
            derp_connected: true,
            online: true,
            relay_mode: TailnetRelayMode::DerpRelay,
            ..Default::default()
        };
        let stale_netmap = TailnetNetMap {
            owner_user_id: 10 + iter,
            seq: 900 + iter,
            generated_at_ms: 1_700_000_000_000,
            derp_region: "local".to_string(),
            derp_ws_path: "/api/v1/tailnet/derp".to_string(),
            stun_http_path: "/api/v1/tailnet/stun".to_string(),
            self_peer: Some(TailnetPeerDescriptor {
                tailnet_ip: "100.64.0.99".to_string(),
                ..peer.clone()
            }),
            peers: vec![peer],
        };

        let c_apply = Arc::clone(&client);
        let c_clear = Arc::clone(&client);
        let applied = rt.block_on(async move {
            let h_apply = tokio::spawn(async move {
                c_apply.tailnet.apply_netmap_if(stale_netmap, || {
                    c_apply.ensure_pinned_session(old_epoch, "tok-a").is_ok()
                })
            });
            let h_clear = tokio::spawn(async move {
                c_clear.clear_session();
            });

            let (applied, clear_res) = tokio::time::timeout(Duration::from_secs(3), async {
                let (a, b) = tokio::join!(h_apply, h_clear);
                (a.expect("apply task panic"), b)
            })
            .await
            .expect("apply_netmap_if vs clear_session deadlocked");
            clear_res.expect("clear task panic");
            applied
        });

        let status = client.tailnet_status();
        assert_eq!(
            status.netmap_seq, 0,
            "iter={iter} applied={applied}: cleared session must never retain stale netmap_seq"
        );
        assert!(
            status.tailnet_ip.is_empty(),
            "iter={iter} applied={applied}: cleared session must have empty tailnet_ip"
        );
        drop(client);
    }
}

// [T5-57] Tight-loop 300-iter start_tailnet_mesh() vs clear_session() never leaks running=true post-logout or deadlocks.
#[test]
fn t5_57_start_tailnet_mesh_vs_clear_session_race() {
    let mut cfg = ClientConfig::default();
    cfg.server_url = "http://127.0.0.1:1".to_string();

    for iter in 0..300u64 {
        let client = Arc::new(TelegramClient::new(cfg.clone()));
        client.set_auth_session(make_test_auth(100 + iter, "alice", "tok-a"));

        let barrier = Arc::new(std::sync::Barrier::new(2));
        let c_start = Arc::clone(&client);
        let c_clear = Arc::clone(&client);
        let b_start = Arc::clone(&barrier);
        let b_clear = Arc::clone(&barrier);

        std::thread::scope(|s| {
            s.spawn(move || {
                b_start.wait();
                c_start.start_tailnet_mesh();
            });
            s.spawn(move || {
                b_clear.wait();
                c_clear.clear_session();
            });
        });

        assert!(
            client.token().is_none(),
            "iter={iter}: token must be None after clear_session"
        );
        assert!(
            !client.tailnet_status().running,
            "iter={iter}: TailnetEngine must NOT remain running after clear_session"
        );
    }
}

// [T5-58-Client] Tight-loop 300-iter set_auth_session/set_token/clear_session race keeps client.token() == ws_transport.token() with zero deadlocks.
#[test]
fn t5_58_client_auth_ws_token_atomic_race() {
    let mut cfg = ClientConfig::default();
    cfg.server_url = "http://127.0.0.1:1".to_string();

    for iter in 0..300u64 {
        let client = Arc::new(TelegramClient::new(cfg.clone()));
        client.set_auth_session(make_test_auth(100 + iter, "alice", "tok-init"));

        let barrier = Arc::new(std::sync::Barrier::new(3));
        let c_sess = Arc::clone(&client);
        let c_tok = Arc::clone(&client);
        let c_clr = Arc::clone(&client);
        let b1 = Arc::clone(&barrier);
        let b2 = Arc::clone(&barrier);
        let b3 = Arc::clone(&barrier);

        let tok_sess = format!("tok-sess-{iter}");
        let tok_manual = if iter % 2 == 0 {
            Some(format!("tok-manual-{iter}"))
        } else {
            None
        };

        std::thread::scope(|s| {
            s.spawn(move || {
                b1.wait();
                c_sess.set_auth_session(make_test_auth(200 + iter, "bob", &tok_sess));
            });
            s.spawn(move || {
                b2.wait();
                c_tok.set_token(tok_manual);
            });
            s.spawn(move || {
                b3.wait();
                c_clr.clear_session();
            });
        });

        let c_token = client.token();
        let ws_token = client.ws_transport.token();
        assert_eq!(
            c_token, ws_token,
            "iter={iter}: client.token() ({c_token:?}) must equal ws_transport.token() ({ws_token:?})"
        );
    }
}
