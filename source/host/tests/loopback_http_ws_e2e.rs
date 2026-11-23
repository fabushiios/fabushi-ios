use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use gpui_telegram_core::types::{ChatId, MessageId, PeerId, UserId};
use gpui_telegram_proto::message::{MessageContent, MessageItem};
use gpui_telegram_proto::sync::{PtsUpdate, SyncGetDifferenceResponse, UpdateEvent};
use gpui_telegram_proto::ws::WsServerFrame;
use mahayana_app_host::{
    fabushi_host_create, fabushi_host_destroy, fabushi_host_dispatch, fabushi_host_free_string,
    fabushi_host_poll_events, telegram_bridge_register_event_callback,
};
use serde_json::{json, Value};
use tempfile::tempdir;

unsafe fn dispatch(handle: *mut c_void, payload: &Value) -> Value {
    let req_c = CString::new(payload.to_string()).unwrap();
    let ptr = unsafe { fabushi_host_dispatch(handle, req_c.as_ptr()) };
    assert!(!ptr.is_null());
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
    unsafe { fabushi_host_free_string(ptr) };
    serde_json::from_str(&text).unwrap()
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(_q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    ws.on_upgrade(handle_ws_socket)
}

async fn handle_ws_socket(mut socket: WebSocket) {
    // Push a real UpdateEvent::NewMessage frame over TCP WebSocket immediately upon connection!
    let msg1 = MessageItem::new(
        MessageId(8801),
        ChatId(300),
        UserId(99),
        PeerId::User(UserId(99)),
        MessageContent::plain_text("Live WS push from loopback server".to_string()),
    );
    let frame1 = WsServerFrame::Update(UpdateEvent::NewMessage(msg1));
    let _ = socket
        .send(Message::Text(serde_json::to_string(&frame1).unwrap().into()))
        .await;

    // Also push a contiguous PtsPush frame!
    let msg2 = MessageItem::new(
        MessageId(8802),
        ChatId(300),
        UserId(99),
        PeerId::User(UserId(99)),
        MessageContent::plain_text("Live PTS push frame".to_string()),
    );
    let pts_update = PtsUpdate::new(
        ChatId(300),
        42,
        1,
        UpdateEvent::NewMessage(msg2),
        Utc::now().timestamp_millis(),
    );
    let frame2 = WsServerFrame::PtsPush(pts_update);
    let _ = socket
        .send(Message::Text(serde_json::to_string(&frame2).unwrap().into()))
        .await;

    // Keep socket open briefly while test consumes events
    tokio::time::sleep(Duration::from_secs(5)).await;
}

#[test]
fn test_real_loopback_http_and_ws_end_to_end() {
    let dir = tempdir().unwrap();

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async {
        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route(
                "/api/v1/auth/login",
                post(|| async {
                    Json(json!({
                        "user_id": 77,
                        "username": "alice_ios",
                        "session_id": "sess-loopback-1",
                        "token": "tok-loopback-secret",
                        "first_name": "Alice"
                    }))
                }),
            )
            .route(
                "/api/v1/chats",
                get(|| async {
                    Json(json!([
                        {
                            "chat_id": 300,
                            "title": "Loopback Engineering",
                            "chat_type": 2,
                            "unread_count": 1,
                            "last_message": "Ready for v1.6.8"
                        }
                    ]))
                }),
            )
            .route(
                "/api/v1/messages/send",
                post(|Json(body): Json<Value>| async move {
                    let chat_id = body.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(300);
                    let text = body
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let nonce = body.get("client_nonce").and_then(|v| v.as_u64());
                    let now = Utc::now();
                    Json(json!({
                        "message_id": 4201,
                        "id": 4201,
                        "pts": 41,
                        "pts_count": 1,
                        "duplicate": false,
                        "date_ms": now.timestamp_millis(),
                        "date": now.to_rfc3339(),
                        "sender_id": 77,
                        "chat_id": chat_id,
                        "text": text,
                        "client_nonce": nonce,
                        "status": "sent"
                    }))
                }),
            )
            .route(
                "/api/v1/sync/get_difference",
                post(|| async {
                    let synced_msg = MessageItem::new(
                        MessageId(4202),
                        ChatId(300),
                        UserId(77),
                        PeerId::User(UserId(77)),
                        MessageContent::plain_text("Synced via getDifference".to_string()),
                    );
                    let resp = SyncGetDifferenceResponse {
                        chat_id: ChatId(300),
                        new_messages: vec![synced_msg],
                        other_updates: vec![],
                        pts: 41,
                        current_pts: 41,
                        updates: vec![],
                        has_more: false,
                        snapshot_required: false,
                    };
                    Json(resp)
                }),
            )
            .route(
                "/api/v1/search/messages",
                get(|| async {
                    Json(json!({
                        "query": "v1.6.8",
                        "total": 1,
                        "hits": [{
                            "chat_id": 300,
                            "message_id": 4201,
                            "sender_id": 77,
                            "date_ms": Utc::now().timestamp_millis(),
                            "text": "Hello from real loopback HTTP client",
                            "snippet_html": "<b>Hello</b>"
                        }]
                    }))
                }),
            )
            .route("/api/v1/ws", get(ws_handler));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}")
        })
        .to_string(),
    )
    .unwrap();

    struct WsCallbackState {
        got_ws_push: AtomicBool,
    }
    let cb_state = Arc::new(WsCallbackState {
        got_ws_push: AtomicBool::new(false),
    });

    unsafe extern "C" fn on_ws_event(
        event_type: *const c_char,
        payload_json: *const c_char,
        user_ctx: *mut c_void,
    ) {
        if !event_type.is_null() && !payload_json.is_null() && !user_ctx.is_null() {
            let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
            let payload = unsafe { CStr::from_ptr(payload_json) }.to_string_lossy();
            if ev == "newMessage" && payload.contains("Live WS push from loopback server") {
                let st = unsafe { &*(user_ctx as *const WsCallbackState) };
                st.got_ws_push.store(true, Ordering::SeqCst);
            }
        }
    }

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        let raw_ctx = Arc::into_raw(Arc::clone(&cb_state)) as *mut c_void;
        telegram_bridge_register_event_callback(host, Some(on_ws_event), raw_ctx);

        // 1. Real HTTP login
        let login = dispatch(
            host,
            &json!({
                "id": "e2e-1",
                "method": "auth.login",
                "params": {"username": "alice_ios", "password": "pw", "device_name": "iPhone"}
            }),
        );
        assert_eq!(login["ok"], true, "login failed: {login}");
        assert_eq!(login["result"]["user_id"], 77);

        // 2. Real HTTP chats.list
        let chats = dispatch(host, &json!({"id": "e2e-2", "method": "chats.list"}));
        assert_eq!(chats["ok"], true, "chats.list failed: {chats}");
        assert_eq!(chats["result"][0]["title"], "Loopback Engineering");

        // 3. Real HTTP messages.send
        let sent = dispatch(
            host,
            &json!({
                "id": "e2e-3",
                "method": "messages.send",
                "params": {"chat_id": 300, "text": "Hello from real loopback HTTP client", "client_nonce": 77701}
            }),
        );
        assert_eq!(sent["ok"], true, "messages.send failed: {sent}");
        assert_eq!(sent["result"]["id"], 4201);

        // 4. Real HTTP sync.getDifference
        let diff = dispatch(
            host,
            &json!({
                "id": "e2e-4",
                "method": "sync.getDifference",
                "params": {"chat_id": 300, "limit": 50}
            }),
        );
        assert_eq!(diff["ok"], true, "sync.getDifference failed: {diff}");
        assert_eq!(diff["result"]["pts"], 41);

        // 5. Real HTTP search.messages
        let search = dispatch(
            host,
            &json!({
                "id": "e2e-5",
                "method": "search.messages",
                "params": {"query": "v1.6.8", "chat_id": 300, "limit": 10}
            }),
        );
        assert_eq!(search["ok"], true, "search.messages failed: {search}");
        assert_eq!(search["result"]["total"], 1);

        // 6. Real TCP WebSocket connect -> receive server-pushed Update & PtsPush via fabushi_host_poll_events AND C callback!
        let ws_conn = dispatch(host, &json!({"id": "e2e-6", "method": "session.connectWs"}));
        assert_eq!(ws_conn["ok"], true);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_ws_msg_in_poll = false;
        let mut saw_pts_msg_in_poll = false;
        while Instant::now() < deadline && (!saw_ws_msg_in_poll || !saw_pts_msg_in_poll) {
            let ptr = fabushi_host_poll_events(host, 32, 300);
            let val: Value =
                serde_json::from_str(CStr::from_ptr(ptr).to_str().unwrap()).unwrap();
            fabushi_host_free_string(ptr);
            assert_eq!(val["ok"], true);
            if let Some(arr) = val["result"]["events"].as_array() {
                for ev in arr {
                    let s = ev.to_string();
                    if s.contains("Live WS push from loopback server") {
                        saw_ws_msg_in_poll = true;
                    }
                    if s.contains("Live PTS push frame") {
                        saw_pts_msg_in_poll = true;
                    }
                }
            }
        }

        assert!(
            saw_ws_msg_in_poll,
            "fabushi_host_poll_events did not receive server WS UpdateEvent::NewMessage"
        );
        assert!(
            saw_pts_msg_in_poll,
            "fabushi_host_poll_events did not receive server WS PtsPush frame"
        );
        assert!(
            cb_state.got_ws_push.load(Ordering::SeqCst),
            "Legacy C event callback did not receive server WS push"
        );

        // Verify WS-pushed message was also persisted into LocalStore
        let local_msgs = dispatch(
            host,
            &json!({"method": "messages.localList", "params": {"chat_id": 300}}),
        );
        assert!(
            local_msgs.to_string().contains("Live WS push from loopback server"),
            "Expected WS-pushed message in LocalStore: {local_msgs}"
        );

        fabushi_host_destroy(host);
        drop(Arc::from_raw(raw_ctx as *const WsCallbackState));
    }
}

#[test]
fn test_reentrant_ws_callback_unregister_and_dispatch_no_deadlock_no_post_destroy_uaf() {
    let dir = tempdir().unwrap();

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|| async {
                    Json(json!({
                        "user_id": 88,
                        "username": "reent_user",
                        "session_id": "sess-reent-1",
                        "token": "tok-reent-secret",
                        "first_name": "Reent"
                    }))
                }),
            )
            .route(
                "/api/v1/ws",
                get(|ws: WebSocketUpgrade| async move {
                    ws.on_upgrade(|mut socket: WebSocket| async move {
                        let m1 = MessageItem::new(
                            MessageId(9101),
                            ChatId(400),
                            UserId(99),
                            PeerId::User(UserId(99)),
                            MessageContent::plain_text("WS_PUSH_FIRST".to_string()),
                        );
                        let _ = socket
                            .send(Message::Text(
                                serde_json::to_string(&WsServerFrame::Update(
                                    UpdateEvent::NewMessage(m1),
                                ))
                                .unwrap()
                                .into(),
                            ))
                            .await;
                        tokio::time::sleep(Duration::from_millis(120)).await;
                        let m2 = MessageItem::new(
                            MessageId(9102),
                            ChatId(400),
                            UserId(99),
                            PeerId::User(UserId(99)),
                            MessageContent::plain_text("WS_PUSH_SECOND".to_string()),
                        );
                        let _ = socket
                            .send(Message::Text(
                                serde_json::to_string(&WsServerFrame::Update(
                                    UpdateEvent::NewMessage(m2),
                                ))
                                .unwrap()
                                .into(),
                            ))
                            .await;
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    })
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}")
        })
        .to_string(),
    )
    .unwrap();

    struct ReentrantWsCtx {
        host: usize,
        new_msg_callbacks: AtomicUsize,
        ping_succeeded: AtomicBool,
    }

    unsafe extern "C" fn on_reentrant_ws(
        event_type: *const c_char,
        payload_json: *const c_char,
        user_ctx: *mut c_void,
    ) {
        if event_type.is_null() || payload_json.is_null() || user_ctx.is_null() {
            return;
        }
        let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
        let payload = unsafe { CStr::from_ptr(payload_json) }.to_string_lossy();
        if ev == "newMessage" && payload.contains("WS_PUSH_") {
            let st = unsafe { &*(user_ctx as *const ReentrantWsCtx) };
            st.new_msg_callbacks.fetch_add(1, Ordering::SeqCst);
            // Re-entrantly unregister callback AND dispatch session.ping from inside pump callback!
            // MUST NOT deadlock event_sink mutex or block Tokio runtime (B-03 / BLOCKING-2)!
            unsafe {
                telegram_bridge_register_event_callback(
                    st.host as *mut c_void,
                    None,
                    std::ptr::null_mut(),
                );
                let ping = dispatch(
                    st.host as *mut c_void,
                    &json!({"id": "cb-ping", "method": "session.ping", "params": {"nonce": 777}}),
                );
                if ping["ok"] == true && ping["result"]["nonce"] == 777 {
                    st.ping_succeeded.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        let ctx = Arc::new(ReentrantWsCtx {
            host: host as usize,
            new_msg_callbacks: AtomicUsize::new(0),
            ping_succeeded: AtomicBool::new(false),
        });
        let raw_ctx = Arc::into_raw(Arc::clone(&ctx)) as *mut c_void;
        telegram_bridge_register_event_callback(host, Some(on_reentrant_ws), raw_ctx);

        let login = dispatch(
            host,
            &json!({
                "id": "r-login",
                "method": "auth.login",
                "params": {"username": "reent_user", "password": "pw", "device_name": "iPhone"}
            }),
        );
        assert_eq!(login["ok"], true);
        assert_eq!(dispatch(host, &json!({"id": "r-ws", "method": "session.connectWs"}))["ok"], true);
        // Give the dedicated fabushi-event-pump thread a moment to drain WS_PUSH_FIRST and fire the callback directly
        thread::sleep(Duration::from_millis(80));

        // Wait until BOTH WS pushes arrive in poll_events (proving 2nd push was processed without firing callback!)
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_first = false;
        let mut saw_second = false;
        while Instant::now() < deadline && (!saw_first || !saw_second) {
            let ptr = fabushi_host_poll_events(host, 32, 200);
            let val: Value =
                serde_json::from_str(CStr::from_ptr(ptr).to_str().unwrap()).unwrap();
            fabushi_host_free_string(ptr);
            if let Some(arr) = val["result"]["events"].as_array() {
                for ev in arr {
                    let s = ev.to_string();
                    if s.contains("WS_PUSH_FIRST") {
                        saw_first = true;
                    }
                    if s.contains("WS_PUSH_SECOND") {
                        saw_second = true;
                    }
                }
            }
        }
        assert!(saw_first && saw_second, "Expected both WS pushes in event queue");
        assert!(
            ctx.ping_succeeded.load(Ordering::SeqCst),
            "Re-entrant dispatch(session.ping) inside C callback must succeed"
        );
        assert_eq!(
            ctx.new_msg_callbacks.load(Ordering::SeqCst),
            1,
            "Callback unregistered on 1st push MUST NOT fire on 2nd push"
        );

        // Synchronous pump join & callback teardown inside fabushi_host_destroy:
        // freeing ctx immediately afterward must NEVER trigger post-destroy UAF or Runtime::drop panic!
        fabushi_host_destroy(host);
        drop(Arc::from_raw(raw_ctx as *const ReentrantWsCtx));
    }
}

#[test]
fn test_messages_send_permanent_403_no_dedup_poison_and_offline_queued_outbox_status() {
    let dir = tempdir().unwrap();

    let reject_mode = Arc::new(AtomicBool::new(true));
    let reject_for_srv = Arc::clone(&reject_mode);

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|| async {
                    Json(json!({
                        "user_id": 55,
                        "username": "sender_ios",
                        "session_id": "sess-send-1",
                        "token": "tok-send-secret",
                        "first_name": "Sender"
                    }))
                }),
            )
            .route(
                "/api/v1/messages/send",
                post(move |Json(body): Json<Value>| {
                    let reject = Arc::clone(&reject_for_srv);
                    async move {
                        if reject.load(Ordering::SeqCst) {
                            return (
                                StatusCode::FORBIDDEN,
                                Json(json!({ "error": "forbidden by channel policy" })),
                            )
                                .into_response();
                        }
                        let chat_id = body.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(500);
                        let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        let nonce = body.get("client_nonce").and_then(|v| v.as_u64());
                        let now = Utc::now();
                        (
                            StatusCode::OK,
                            Json(json!({
                                "message_id": 6001,
                                "id": 6001,
                                "pts": 10,
                                "pts_count": 1,
                                "duplicate": false,
                                "date_ms": now.timestamp_millis(),
                                "date": now.to_rfc3339(),
                                "sender_id": 55,
                                "chat_id": chat_id,
                                "text": text,
                                "client_nonce": nonce,
                                "status": "sent"
                            })),
                        )
                            .into_response()
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}")
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        assert_eq!(
            dispatch(
                host,
                &json!({
                    "method": "auth.login",
                    "params": {"username": "sender_ios", "password": "pw", "device_name": "iPhone"}
                })
            )["ok"],
            true
        );

        // 1. Server returns permanent HTTP 403 -> MUST fail closed, NOT poison 24h nonce_dedup, NOT index into search!
        let rejected = dispatch(
            host,
            &json!({
                "id": "m-403",
                "method": "messages.send",
                "params": {
                    "chat_id": 500,
                    "text": "ForbiddenUniqueTokenXYZ403",
                    "client_nonce": 888403
                }
            }),
        );
        assert_eq!(rejected["ok"], false, "HTTP 403 must fail closed: {rejected}");
        assert_eq!(rejected["error"]["code"], "permission_denied");

        let search_rejected = dispatch(
            host,
            &json!({
                "method": "search.localMessages",
                "params": {"query": "ForbiddenUniqueTokenXYZ403"}
            }),
        );
        assert_eq!(
            search_rejected["result"]["total"], 0,
            "Rejected HTTP 403 message MUST NOT be indexed in Tantivy/FTS5: {search_rejected}"
        );

        // 2. Flip server to HTTP 200 -> retrying with EXACT SAME client_nonce MUST succeed (proves dedup was not poisoned!)
        reject_mode.store(false, Ordering::SeqCst);
        let retried = dispatch(
            host,
            &json!({
                "id": "m-200",
                "method": "messages.send",
                "params": {
                    "chat_id": 500,
                    "text": "ForbiddenUniqueTokenXYZ403",
                    "client_nonce": 888403
                }
            }),
        );
        assert_eq!(retried["ok"], true, "Retry after 403 must succeed: {retried}");
        assert_eq!(retried["result"]["id"], 6001);
        assert_eq!(retried["result"]["status"], "sent");
        assert_ne!(retried["result"]["duplicate"], true);

        // 3. Offline retryable send returns "queued_outbox", AND dedup replay ALSO returns "queued_outbox" (never lies "sent"!)
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "outbox.setNetworkState", "params": {"online": false}})
            )["ok"],
            true
        );
        let queued = dispatch(
            host,
            &json!({
                "id": "m-off-1",
                "method": "messages.send",
                "params": {
                    "chat_id": 500,
                    "text": "Queued offline payload",
                    "client_nonce": 888503
                }
            }),
        );
        assert_eq!(queued["ok"], true, "Offline send should queue cleanly: {queued}");
        assert_eq!(queued["result"]["status"], "queued_outbox");

        let replay_queued = dispatch(
            host,
            &json!({
                "id": "m-off-2",
                "method": "messages.send",
                "params": {
                    "chat_id": 500,
                    "text": "Queued offline payload",
                    "client_nonce": 888503
                }
            }),
        );
        assert_eq!(replay_queued["ok"], true);
        assert_eq!(replay_queued["result"]["duplicate"], true);
        assert_eq!(
            replay_queued["result"]["status"], "queued_outbox",
            "Dedup replay of queued offline message must accurately report queued_outbox: {replay_queued}"
        );

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_cross_account_secret_wipe_no_plaintext_token_in_sqlite_and_switch_account() {
    let dir_enc = tempdir().unwrap();
    let dir_nopass = tempdir().unwrap();

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|Json(body): Json<Value>| async move {
                    let user = body.get("username").and_then(|v| v.as_str()).unwrap_or("u1");
                    if user == "bob_ios" {
                        Json(json!({
                            "user_id": 202,
                            "username": "bob_ios",
                            "session_id": "sess-bob-2",
                            "token": "SUPER_SECRET_BEARER_BOB_999",
                            "first_name": "Bob"
                        }))
                    } else {
                        Json(json!({
                            "user_id": 101,
                            "username": "alice_ios",
                            "session_id": "sess-alice-1",
                            "token": "SUPER_SECRET_BEARER_ALICE_777",
                            "first_name": "Alice"
                        }))
                    }
                }),
            )
            .route("/api/v1/auth/logout", post(|| async { Json(json!({"ok": true})) }))
            .route(
                "/health",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    "ok"
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    // A. Host WITH Keychain storage_passphrase: token encrypted via ChaCha20-Poly1305, ZERO plaintext in host_state.db!
    let cfg_enc = CString::new(
        json!({
            "app_data_dir": dir_enc.path().to_string_lossy(),
            "server_url": format!("http://{addr}"),
            "storage_passphrase": "keychain-256-bit-master-passphrase",
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_enc.as_ptr());
        assert!(!host.is_null());

        // Login Account 1 (Alice) and Account 2 (Bob)
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "alice_ios", "password": "pw", "device_name": "iPhone"}})
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "bob_ios", "password": "pw", "device_name": "iPhone"}})
            )["ok"],
            true
        );

        // Inspect host_state.db directly: ZERO plaintext bearer tokens anywhere in host_kv!
        let conn = rusqlite::Connection::open(dir_enc.path().join("host_state.db")).unwrap();
        let mut stmt = conn.prepare("SELECT value FROM host_kv").unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!rows.is_empty());
        for val in &rows {
            assert!(
                !val.contains("SUPER_SECRET_BEARER_ALICE_777")
                    && !val.contains("SUPER_SECRET_BEARER_BOB_999"),
                "Plaintext bearer token MUST NEVER be stored in host_state.db: {val}"
            );
            assert!(val.contains("encrypted_token_b64"), "Expected AEAD encrypted_token_b64: {val}");
        }
        drop(stmt);
        drop(conn);

        // Unlock vault for Account 2 (Bob)
        let init_v = dispatch(
            host,
            &json!({"method": "vault.initSplitKey", "params": {"user_id": 202, "password": "pw", "fast_kdf": true}}),
        );
        assert_eq!(init_v["ok"], true);
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], true);

        // Unknown account switch MUST fail closed with not_found (WITHOUT clearing current session/vault!)
        let unknown = dispatch(
            host,
            &json!({"method": "session.switchAccount", "params": {"account_id": "999999"}}),
        );
        assert_eq!(unknown["ok"], false);
        assert_eq!(unknown["error"]["code"], "not_found");
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], true);

        // Start a slow in-flight RPC for Account 2, then switchAccount to Account 1 (101) ->
        // MUST cancel Account 2's in-flight RPC AND lock vault_unlocked_key!
        let host_addr = host as usize;
        let inflight = thread::spawn(move || {
            dispatch(
                host_addr as *mut c_void,
                &json!({"id": "acct2-slow", "method": "auth.health"}),
            )
        });
        thread::sleep(Duration::from_millis(80));

        let switched = dispatch(
            host,
            &json!({"method": "session.switchAccount", "params": {"account_id": "101"}}),
        );
        assert_eq!(switched["ok"], true, "switchAccount to 101 failed: {switched}");
        assert_eq!(switched["result"]["user_id"], 101);

        let inflight_res = inflight.join().unwrap();
        assert_eq!(
            inflight_res["ok"], false,
            "In-flight RPC from previous account must fail after switchAccount: {inflight_res}"
        );
        assert!(
            matches!(
                inflight_res["error"]["code"].as_str(),
                Some("cancelled" | "stale_generation")
            ),
            "Expected cancelled or stale_generation, got: {inflight_res}"
        );

        // Vault MUST be locked after switchAccount (zero cross-account key leakage!)
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], false);
        let vault_blocked = dispatch(
            host,
            &json!({"method": "vault.listForOrigin", "params": {"origin": "https://github.com"}}),
        );
        assert_eq!(vault_blocked["ok"], false);
        assert_eq!(vault_blocked["error"]["code"], "unauthorized");

        // Unlock again and verify auth.logout ALSO locks vault
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 101, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "auth.logout"}))["ok"], true);
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], false);

        // Unlock again and verify session.clearSession ALSO locks vault
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 101, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "session.clearSession"}))["ok"], true);
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], false);

        fabushi_host_destroy(host);

        // Reopen with same storage_passphrase -> AEAD-encrypted token decrypts cleanly on switchAccount!
        let host_reopen = fabushi_host_create(cfg_enc.as_ptr());
        assert!(!host_reopen.is_null());
        let sw_reopen = dispatch(
            host_reopen,
            &json!({"method": "session.switchAccount", "params": {"account_id": "202"}}),
        );
        assert_eq!(sw_reopen["ok"], true, "Cross-restart encrypted switchAccount failed: {sw_reopen}");
        assert_eq!(sw_reopen["result"]["user_id"], 202);
        fabushi_host_destroy(host_reopen);
    }

    // B. Host WITHOUT storage_passphrase: ZERO plaintext token in host_state.db (metadata only!)
    let cfg_nopass = CString::new(
        json!({
            "app_data_dir": dir_nopass.path().to_string_lossy(),
            "server_url": format!("http://{addr}")
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_nopass.as_ptr());
        assert!(!host.is_null());
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "alice_ios", "password": "pw", "device_name": "iPhone"}})
            )["ok"],
            true
        );
        let conn = rusqlite::Connection::open(dir_nopass.path().join("host_state.db")).unwrap();
        let val: String = conn
            .query_row(
                "SELECT value FROM host_kv WHERE namespace = 'accounts' AND key = '101'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !val.contains("SUPER_SECRET_BEARER_ALICE_777"),
            "Host without storage_passphrase MUST NOT store plaintext token in SQLite: {val}"
        );
        assert!(!val.contains("\"token\""));
        drop(conn);
        fabushi_host_destroy(host);
    }
}

#[test]
fn test_set_server_url_live_reconfig_atomic_invalid_url_and_concurrent_safety() {
    let dir = tempdir().unwrap();

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let (addr_a, addr_b) = server_rt.block_on(async {
        let app_a = Router::new().route("/health", get(|| async { "ok-a" }));
        let app_b = Router::new().route("/health", get(|| async { "ok-b" }));
        let listener_a = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener_b = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let la = listener_a.local_addr().unwrap();
        let lb = listener_b.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener_a, app_a).await;
        });
        tokio::spawn(async move {
            let _ = axum::serve(listener_b, app_b).await;
        });
        (la, lb)
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr_a}"),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        // Unlock vault and verify health on Server A
        assert_eq!(dispatch(host, &json!({"method": "auth.health"}))["result"]["healthy"], true);
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 1, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        let gen_before = dispatch(host, &json!({"method": "generation.current"}))["result"]["generation"]
            .as_u64()
            .unwrap();

        // 1. Invalid URL (including malformed bracketed IPv6 ports, NB2-02) MUST fail atomically
        //    without breaking live host or wiping session/vault!
        for bad_url in [
            "",
            "   ",
            "ftp://invalid.example",
            "http://bad host:8080",
            "http://[::1]:bad",
            "http://[::1]:",
            "http://[::1",
        ] {
            let bad = dispatch(
                host,
                &json!({"method": "session.setServerUrl", "params": {"server_url": bad_url}}),
            );
            assert_eq!(bad["ok"], false, "Invalid URL '{bad_url}' must fail closed: {bad}");
            assert_eq!(bad["error"]["code"], "invalid_arguments");
        }
        assert_eq!(
            dispatch(host, &json!({"method": "session.getServerUrl"}))["result"]["server_url"],
            format!("http://{addr_a}")
        );
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], true);
        assert_eq!(dispatch(host, &json!({"method": "auth.health"}))["result"]["healthy"], true);

        // 2. Live switch to Server B bumps generation, wipes account scope, and serves health from Server B
        let switched = dispatch(
            host,
            &json!({
                "method": "session.setServerUrl",
                "params": {"server_url": format!("http://{addr_b}")}
            }),
        );
        assert_eq!(switched["ok"], true, "setServerUrl to Server B failed: {switched}");
        assert_eq!(switched["result"]["server_url"], format!("http://{addr_b}"));
        assert!(switched["result"]["generation"].as_u64().unwrap() > gen_before);
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], false);
        assert_eq!(dispatch(host, &json!({"method": "auth.health"}))["result"]["healthy"], true);

        // 3. Concurrent setServerUrl calls serialize cleanly with zero panics or orphaned pumps
        let host_addr = host as usize;
        let url_a = format!("http://{addr_a}");
        let url_b = format!("http://{addr_b}");
        let mut threads = Vec::new();
        for i in 0..4 {
            let target = if i % 2 == 0 { url_a.clone() } else { url_b.clone() };
            threads.push(thread::spawn(move || {
                dispatch(
                    host_addr as *mut c_void,
                    &json!({"method": "session.setServerUrl", "params": {"server_url": target}}),
                )
            }));
        }
        for t in threads {
            let r = t.join().expect("concurrent setServerUrl must not panic");
            assert_eq!(r["ok"], true, "concurrent setServerUrl failed: {r}");
        }
        assert_eq!(dispatch(host, &json!({"method": "auth.health"}))["result"]["healthy"], true);

        // 4. Valid bracketed IPv6 authorities (with and without port, NB2-02) are accepted cleanly
        for ok_ipv6 in ["http://[::1]", "http://[::1]:8080"] {
            let ok_v6 = dispatch(
                host,
                &json!({"method": "session.setServerUrl", "params": {"server_url": ok_ipv6}}),
            );
            assert_eq!(ok_v6["ok"], true, "Valid IPv6 URL '{ok_ipv6}' must be accepted: {ok_v6}");
            assert_eq!(ok_v6["result"]["server_url"], ok_ipv6);
        }

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_b2_01_reentrant_set_server_url_from_event_pump_callback_fails_closed_no_deadlock() {
    let dir = tempdir().unwrap();

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|| async {
                    Json(json!({
                        "user_id": 501,
                        "username": "pump_reent",
                        "session_id": "sess-pump-1",
                        "token": "tok-pump-501",
                        "first_name": "Pump"
                    }))
                }),
            )
            .route("/ws", get(ws_handler));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}"),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    struct PumpReentrantCtx {
        host: usize,
        invoked: AtomicBool,
        rejected_conflict: AtomicBool,
    }

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        let ctx = Arc::new(PumpReentrantCtx {
            host: host as usize,
            invoked: AtomicBool::new(false),
            rejected_conflict: AtomicBool::new(false),
        });

        unsafe extern "C" fn on_pump_ws_event(
            event_type: *const c_char,
            _payload: *const c_char,
            user_ctx: *mut c_void,
        ) {
            if event_type.is_null() || user_ctx.is_null() {
                return;
            }
            let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
            if ev == "newMessage" {
                let st = unsafe { &*(user_ctx as *const PumpReentrantCtx) };
                if !st.invoked.swap(true, Ordering::SeqCst) {
                    // Invoke session.setServerUrl synchronously from inside fabushi-event-pump!
                    // MUST fail closed immediately with conflict (ZERO AB-BA deadlock, B2-01)!
                    let res = unsafe {
                        dispatch(
                            st.host as *mut c_void,
                            &json!({
                                "id": "reent-url-1",
                                "method": "session.setServerUrl",
                                "params": {"server_url": "http://127.0.0.1:19876"}
                            }),
                        )
                    };
                    if res["ok"] == false && res["error"]["code"] == "conflict" {
                        st.rejected_conflict.store(true, Ordering::SeqCst);
                    }
                }
            }
        }

        let raw_ctx = Arc::into_raw(Arc::clone(&ctx)) as *mut c_void;
        telegram_bridge_register_event_callback(host, Some(on_pump_ws_event), raw_ctx);

        assert_eq!(
            dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "pump_reent", "password": "pw"}})
            )["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "session.connectWs"}))["ok"], true);

        let deadline = Instant::now() + Duration::from_secs(5);
        while !ctx.invoked.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(ctx.invoked.load(Ordering::SeqCst), "WS newMessage callback must run on event pump");
        assert!(
            ctx.rejected_conflict.load(Ordering::SeqCst),
            "Re-entrant session.setServerUrl from event pump callback must fail closed with conflict"
        );

        // Original server URL preserved and host remains healthy and responsive
        assert_eq!(
            dispatch(host, &json!({"method": "session.getServerUrl"}))["result"]["server_url"],
            format!("http://{addr}")
        );
        assert_eq!(dispatch(host, &json!({"method": "session.ping"}))["ok"], true);

        fabushi_host_destroy(host);
        drop(Arc::from_raw(raw_ctx as *const PumpReentrantCtx));
    }
}

#[test]
fn test_b2_02_direct_auth_and_set_auth_session_wipe_vault_magpie_bump_gen_stale_login_and_logout_delete() {
    let dir = tempdir().unwrap();

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|Json(body): Json<Value>| async move {
                    let user = body.get("username").and_then(|v| v.as_str()).unwrap_or("alice");
                    match user {
                        "bob" => Json(json!({
                            "user_id": 202,
                            "username": "bob",
                            "session_id": "sess-bob",
                            "token": "TOKEN_BOB_202",
                            "first_name": "Bob"
                        })),
                        "slow_charlie" => {
                            tokio::time::sleep(Duration::from_millis(350)).await;
                            Json(json!({
                                "user_id": 303,
                                "username": "slow_charlie",
                                "session_id": "sess-charlie",
                                "token": "TOKEN_CHARLIE_303",
                                "first_name": "Charlie"
                            }))
                        }
                        _ => Json(json!({
                            "user_id": 101,
                            "username": "alice",
                            "session_id": "sess-alice",
                            "token": "TOKEN_ALICE_101",
                            "first_name": "Alice"
                        })),
                    }
                }),
            )
            .route("/api/v1/auth/logout", post(|| async { Json(json!({"ok": true})) }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}"),
            "storage_passphrase": "keychain-b2-02-passphrase",
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        // 1. Account A (Alice) logs in, unlocks Vault, and masks a secret via magpie.maskText
        let login_a = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "alice", "password": "pw"}}),
        );
        assert_eq!(login_a["ok"], true);
        let gen_a = login_a["generation"].as_u64().unwrap();
        assert!(gen_a >= 2, "auth.login must bump generation, got {gen_a}");

        assert_eq!(
            dispatch(
                host,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 101, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], true);

        let alice_secret = "Key=sk-ant-api03-alicesecret998877 email=alice@secretcorp.io";
        let masked_a = dispatch(
            host,
            &json!({"method": "magpie.maskText", "params": {"text": alice_secret}}),
        );
        assert_eq!(masked_a["ok"], true);
        let masked_a_str = masked_a["result"]["masked_text"].as_str().unwrap().to_string();
        assert!(!masked_a_str.contains("alicesecret998877"));
        assert_eq!(
            dispatch(host, &json!({"method": "magpie.unmaskText", "params": {"text": masked_a_str}}))["result"]["text"],
            alice_secret
        );

        // 2. Account B (Bob) logs in DIRECTLY via auth.login WITHOUT prior auth.logout!
        let login_b = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "bob", "password": "pw"}}),
        );
        assert_eq!(login_b["ok"], true);
        let gen_b = login_b["generation"].as_u64().unwrap();
        assert!(gen_b > gen_a, "Direct auth.login as Account B must bump generation ({gen_b} > {gen_a})");
        assert_eq!(
            dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"],
            false,
            "Account A's unlocked Vault key MUST be wiped on direct Account B auth.login (B2-02)"
        );
        let unmasked_as_b = dispatch(
            host,
            &json!({"method": "magpie.unmaskText", "params": {"text": masked_a_str}}),
        )["result"]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            !unmasked_as_b.contains("alicesecret998877") && !unmasked_as_b.contains("alice@secretcorp.io"),
            "Account B MUST NOT unmask Account A's Magpie MemoryVault secrets: {unmasked_as_b}"
        );
        assert_eq!(
            dispatch(host, &json!({"method": "session.listAccounts"}))["result"]["active_account_id"],
            "202"
        );

        // 3. Unlock Vault & mask secret under Account B, then directly call session.setAuthSession as Account C (404)
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 202, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        let bob_secret = "Key=sk-ant-api03-bobsecret445566 email=bob@secretcorp.io";
        let masked_b_str = dispatch(
            host,
            &json!({"method": "magpie.maskText", "params": {"text": bob_secret}}),
        )["result"]["masked_text"]
            .as_str()
            .unwrap()
            .to_string();

        let set_c = dispatch(
            host,
            &json!({
                "method": "session.setAuthSession",
                "params": {
                    "user_id": 404,
                    "username": "carol",
                    "session_id": "sess-carol",
                    "token": "TOKEN_CAROL_404",
                    "first_name": "Carol"
                }
            }),
        );
        assert_eq!(set_c["ok"], true);
        let gen_c = set_c["generation"].as_u64().unwrap();
        assert!(gen_c > gen_b);
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], false);
        let unmasked_as_c = dispatch(
            host,
            &json!({"method": "magpie.unmaskText", "params": {"text": masked_b_str}}),
        )["result"]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            !unmasked_as_c.contains("bobsecret445566") && !unmasked_as_c.contains("bob@secretcorp.io"),
            "Account C MUST NOT unmask Account B's Magpie MemoryVault secrets: {unmasked_as_c}"
        );
        assert_eq!(
            dispatch(host, &json!({"method": "session.listAccounts"}))["result"]["active_account_id"],
            "404"
        );

        // 4. Stale in-flight auth.login (slow_charlie, 350ms) raced by concurrent generation.bump ->
        //    MUST fail closed with stale_generation and MUST NOT overwrite active user 404 or persist 303!
        let host_addr = host as usize;
        let stale_login = thread::spawn(move || {
            dispatch(
                host_addr as *mut c_void,
                &json!({"id": "stale-charlie", "method": "auth.login", "params": {"username": "slow_charlie", "password": "pw"}}),
            )
        });
        thread::sleep(Duration::from_millis(80));
        let bumped = dispatch(host, &json!({"method": "generation.bump"}));
        assert_eq!(bumped["ok"], true);

        let stale_res = stale_login.join().unwrap();
        assert_eq!(
            stale_res["ok"], false,
            "Stale in-flight auth.login must fail closed: {stale_res}"
        );
        assert_eq!(stale_res["error"]["code"], "stale_generation");
        assert_eq!(
            dispatch(host, &json!({"method": "session.wsStatus"}))["result"]["user_id"],
            404,
            "Stale auth.login must NOT overwrite active session user_id 404"
        );
        let accts_after_stale = dispatch(host, &json!({"method": "session.listAccounts"}));
        assert_eq!(accts_after_stale["result"]["active_account_id"], "404");
        assert!(
            !accts_after_stale["result"]["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["account_id"] == "303"),
            "Stale auth.login must NOT persist account 303: {accts_after_stale}"
        );

        // 5. auth.logout MUST delete ("accounts", uid) from host_state.db so logged-out account cannot be re-entered!
        assert_eq!(
            dispatch(host, &json!({"method": "session.switchAccount", "params": {"account_id": "202"}}))["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "auth.logout"}))["ok"], true);

        let conn = rusqlite::Connection::open(dir.path().join("host_state.db")).unwrap();
        let bob_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM host_kv WHERE namespace = 'accounts' AND key = '202'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bob_rows, 0, "auth.logout MUST delete ('accounts', '202') from host_state.db (B2-02)");
        drop(conn);

        let accts_after_logout = dispatch(host, &json!({"method": "session.listAccounts"}));
        assert_eq!(accts_after_logout["result"]["active_account_id"], Value::Null);
        assert!(
            !accts_after_logout["result"]["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["account_id"] == "202"),
            "Logged-out account 202 must not appear in session.listAccounts"
        );
        let reenter = dispatch(
            host,
            &json!({"method": "session.switchAccount", "params": {"account_id": "202"}}),
        );
        assert_eq!(reenter["ok"], false);
        assert_eq!(reenter["error"]["code"], "not_found");

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_b2_03_cross_server_token_replay_blocked_and_no_passphrase_restart_fails_closed() {
    let dir_enc = tempdir().unwrap();
    let dir_nopass = tempdir().unwrap();

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let (addr_a, addr_b) = server_rt.block_on(async {
        let make_app = |tag: &'static str| {
            Router::new()
                .route("/health", get(move || async move { tag }))
                .route(
                    "/api/v1/auth/login",
                    post(move |Json(body): Json<Value>| async move {
                        let user = body.get("username").and_then(|v| v.as_str()).unwrap_or("alice");
                        if user == "bob" {
                            Json(json!({
                                "user_id": 202,
                                "username": "bob",
                                "session_id": format!("sess-bob-{tag}"),
                                "token": format!("TOKEN_BOB_{tag}"),
                                "first_name": "Bob"
                            }))
                        } else {
                            Json(json!({
                                "user_id": 101,
                                "username": "alice",
                                "session_id": format!("sess-alice-{tag}"),
                                "token": format!("TOKEN_ALICE_{tag}"),
                                "first_name": "Alice"
                            }))
                        }
                    }),
                )
        };
        let la_l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let lb_l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let la = la_l.local_addr().unwrap();
        let lb = lb_l.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(la_l, make_app("SERVER_A")).await;
        });
        tokio::spawn(async move {
            let _ = axum::serve(lb_l, make_app("SERVER_B")).await;
        });
        (la, lb)
    });

    // A. Cross-server bearer token replay MUST fail closed with unauthorized without mutating live session
    let cfg_enc = CString::new(
        json!({
            "app_data_dir": dir_enc.path().to_string_lossy(),
            "server_url": format!("http://{addr_a}"),
            "storage_passphrase": "keychain-b2-03-passphrase",
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_enc.as_ptr());
        assert!(!host.is_null());

        // Login Alice on Server A
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "alice", "password": "pw"}})
            )["ok"],
            true
        );

        // Switch host to Server B and login Bob + unlock Bob's Vault
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "session.setServerUrl", "params": {"server_url": format!("http://{addr_b}")}})
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "bob", "password": "pw"}})
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 202, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        let gen_bob = dispatch(host, &json!({"method": "generation.current"}))["result"]["generation"]
            .as_u64()
            .unwrap();

        // Attempting to switchAccount to Server A's Alice (101) while on Server B MUST fail closed!
        let cross_srv = dispatch(
            host,
            &json!({"method": "session.switchAccount", "params": {"account_id": "101"}}),
        );
        assert_eq!(
            cross_srv["ok"], false,
            "Cross-server switchAccount MUST fail closed (B2-03): {cross_srv}"
        );
        assert_eq!(cross_srv["error"]["code"], "unauthorized");

        // Bob's live session, unlocked vault, and generation MUST remain 100% intact!
        assert_eq!(dispatch(host, &json!({"method": "session.wsStatus"}))["result"]["user_id"], 202);
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], true);
        assert_eq!(
            dispatch(host, &json!({"method": "generation.current"}))["result"]["generation"],
            gen_bob
        );

        fabushi_host_destroy(host);
    }

    // B. Host WITHOUT storage_passphrase: restart + switchAccount to metadata-only account MUST fail closed!
    let cfg_nopass = CString::new(
        json!({
            "app_data_dir": dir_nopass.path().to_string_lossy(),
            "server_url": format!("http://{addr_a}"),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host1 = fabushi_host_create(cfg_nopass.as_ptr());
        assert!(!host1.is_null());
        assert_eq!(
            dispatch(
                host1,
                &json!({"method": "auth.login", "params": {"username": "alice", "password": "pw"}})
            )["ok"],
            true
        );
        fabushi_host_destroy(host1);

        // Restart new host without storage_passphrase -> Alice's RAM token is gone
        let host2 = fabushi_host_create(cfg_nopass.as_ptr());
        assert!(!host2.is_null());
        assert_eq!(
            dispatch(
                host2,
                &json!({"method": "auth.login", "params": {"username": "bob", "password": "pw"}})
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 202, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        let gen_bob2 = dispatch(host2, &json!({"method": "generation.current"}))["result"]["generation"]
            .as_u64()
            .unwrap();

        let no_tok = dispatch(
            host2,
            &json!({"method": "session.switchAccount", "params": {"account_id": "101"}}),
        );
        assert_eq!(
            no_tok["ok"], false,
            "switchAccount without recoverable token MUST fail closed (B2-03): {no_tok}"
        );
        assert_eq!(no_tok["error"]["code"], "unauthorized");
        assert_eq!(dispatch(host2, &json!({"method": "session.wsStatus"}))["result"]["user_id"], 202);
        assert_eq!(dispatch(host2, &json!({"method": "vault.status"}))["result"]["unlocked"], true);
        assert_eq!(
            dispatch(host2, &json!({"method": "generation.current"}))["result"]["generation"],
            gen_bob2
        );

        fabushi_host_destroy(host2);
    }
}

#[test]
fn test_nb2_05_messages_send_http_401_and_413_clean_outbox_and_allow_same_nonce_retry() {
    let dir = tempdir().unwrap();
    let send_attempt = Arc::new(AtomicUsize::new(0));
    let send_attempt_srv = Arc::clone(&send_attempt);

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async move {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|| async {
                    Json(json!({
                        "user_id": 77,
                        "username": "sender_4xx",
                        "session_id": "sess-4xx",
                        "token": "tok-4xx",
                        "first_name": "Sender"
                    }))
                }),
            )
            .route(
                "/api/v1/messages/send",
                post(move |Json(body): Json<Value>| {
                    let attempts = Arc::clone(&send_attempt_srv);
                    async move {
                        let n = attempts.fetch_add(1, Ordering::SeqCst);
                        match n {
                            0 => (
                                StatusCode::UNAUTHORIZED,
                                Json(json!({"error": "token expired"})),
                            )
                                .into_response(),
                            1 => (
                                StatusCode::PAYLOAD_TOO_LARGE,
                                Json(json!({"error": "payload too large"})),
                            )
                                .into_response(),
                            _ => (
                                StatusCode::OK,
                                Json(json!({
                                    "message_id": 6002,
                                    "id": 6002,
                                    "pts": 55,
                                    "pts_count": 1,
                                    "duplicate": false,
                                    "date_ms": Utc::now().timestamp_millis(),
                                    "date": Utc::now().to_rfc3339(),
                                    "sender_id": 77,
                                    "chat_id": body["chat_id"],
                                    "text": body["text"],
                                    "client_nonce": body["client_nonce"],
                                    "status": "sent"
                                })),
                            )
                                .into_response(),
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}")
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        assert_eq!(
            dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "sender_4xx", "password": "pw"}})
            )["ok"],
            true
        );

        let payload = json!({
            "method": "messages.send",
            "params": {"chat_id": 500, "text": "Retryable after 401 and 413", "client_nonce": 888401}
        });

        let r1 = dispatch(host, &payload);
        assert_eq!(r1["ok"], false, "HTTP 401 must fail closed: {r1}");
        assert_eq!(r1["error"]["code"], "unauthorized");

        let r2 = dispatch(host, &payload);
        assert_eq!(r2["ok"], false, "HTTP 413 must fail closed without stale Pending outbox conflict: {r2}");
        assert_eq!(r2["error"]["code"], "api_error");

        let r3 = dispatch(host, &payload);
        assert_eq!(r3["ok"], true, "Retry with identical client_nonce after 401/413 cleanup must succeed: {r3}");
        assert_eq!(r3["result"]["id"], 6002);
        assert_eq!(r3["result"]["status"], "sent");

        fabushi_host_destroy(host);
    }
}
