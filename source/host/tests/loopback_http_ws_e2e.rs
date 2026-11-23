use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
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

        // 1. Invalid URL (including malformed bracketed IPv6 & RFC 3986 ports, NB2-02/NB3-03/NB4-02)
        //    MUST fail atomically without breaking live host or wiping session/vault!
        for bad_url in [
            "",
            "   ",
            "ftp://invalid.example",
            "http://bad host:8080",
            "http://[::1]:bad",
            "http://[::1]:",
            "http://[::1",
            "http://[::1]junk:8080",
            "http://[::1]junk",
            "http://[::1]]:8080",
            "http://[]",
            "http://foo[::1]:8080",
            "http://[::1]:+8080",
            "http://[::1]:0",
            "http://example.com:+80",
            "http://example.com:0",
            "http://example.com:80:90",
            "http://2001:db8::1:8080",
            "http://2001:db8::1",
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

        // CI Challenger NB3-02: Even if rec.server_url in host_state.db is directly tampered
        // from Server A to Server B, ChaCha20-Poly1305 AEAD AAD binding MUST fail closed!
        {
            let conn = rusqlite::Connection::open(dir_enc.path().join("host_state.db")).unwrap();
            let raw_alice: String = conn
                .query_row(
                    "SELECT value FROM host_kv WHERE namespace = 'accounts' AND key = '101'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let mut alice_json: Value = serde_json::from_str(&raw_alice).unwrap();
            alice_json["server_url"] = json!(format!("http://{addr_b}"));
            let tampered_json = serde_json::to_string(&alice_json).unwrap();
            conn.execute(
                "UPDATE host_kv SET value = ?1 WHERE namespace = 'accounts' AND key = '101'",
                [tampered_json],
            )
            .unwrap();
        }
        let tampered_switch = dispatch(
            host,
            &json!({"method": "session.switchAccount", "params": {"account_id": "101"}}),
        );
        assert_eq!(
            tampered_switch["ok"], false,
            "Tampered rec.server_url in host_state.db MUST fail closed via AEAD AAD mismatch: {tampered_switch}"
        );
        assert_eq!(tampered_switch["error"]["code"], "crypto_error");
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

#[test]
fn test_b3_01_stale_inflight_logout_fenced_against_account_switch_and_host_destroy() {
    let dir1 = tempdir().unwrap();
    let dir2 = tempdir().unwrap();

    let logout_entered = Arc::new(AtomicBool::new(false));
    let logout_release = Arc::new(AtomicBool::new(false));
    let entered_srv = Arc::clone(&logout_entered);
    let release_srv = Arc::clone(&logout_release);

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async move {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|Json(body): Json<Value>| async move {
                    let user = body.get("username").and_then(|v| v.as_str()).unwrap_or("alice");
                    if user == "bob" {
                        Json(json!({
                            "user_id": 202,
                            "username": "bob",
                            "session_id": "sess-bob-b301",
                            "token": "TOKEN_BOB_B301",
                            "first_name": "Bob"
                        }))
                    } else {
                        Json(json!({
                            "user_id": 101,
                            "username": "alice",
                            "session_id": "sess-alice-b301",
                            "token": "TOKEN_ALICE_B301",
                            "first_name": "Alice"
                        }))
                    }
                }),
            )
            .route(
                "/api/v1/auth/logout",
                post(move || {
                    let entered = Arc::clone(&entered_srv);
                    let release = Arc::clone(&release_srv);
                    async move {
                        entered.store(true, Ordering::SeqCst);
                        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                        while !release.load(Ordering::SeqCst)
                            && tokio::time::Instant::now() < deadline
                        {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                        Json(json!({"ok": true}))
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

    // Part A: Slow in-flight auth.logout (Account A = 101) raced by session.switchAccount (Account B = 202)
    // AND slow in-flight auth.logout returning HTTP 200 after a concurrent generation.bump.
    let cfg1 = CString::new(
        json!({
            "app_data_dir": dir1.path().to_string_lossy(),
            "server_url": format!("http://{addr}"),
            "storage_passphrase": "keychain-b3-01-passphrase",
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg1.as_ptr());
        assert!(!host.is_null());

        // Seed Bob (202) then switch active session to Alice (101)
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
                &json!({"method": "auth.login", "params": {"username": "alice", "password": "pw"}})
            )["ok"],
            true
        );

        // Sub-case A1: Slow auth.logout (Account A = 101) raced by session.switchAccount("202")
        logout_entered.store(false, Ordering::SeqCst);
        logout_release.store(false, Ordering::SeqCst);
        let host_addr = host as usize;
        let slow_logout_a1 = thread::spawn(move || {
            dispatch(
                host_addr as *mut c_void,
                &json!({"id": "slow-logout-a1", "method": "auth.logout"}),
            )
        });

        let wait_deadline = Instant::now() + Duration::from_secs(3);
        while !logout_entered.load(Ordering::SeqCst) && Instant::now() < wait_deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            logout_entered.load(Ordering::SeqCst),
            "Slow auth.logout must reach loopback /api/v1/auth/logout"
        );

        let switched = dispatch(
            host,
            &json!({"id": "switch-to-bob", "method": "session.switchAccount", "params": {"account_id": "202"}}),
        );
        assert_eq!(switched["ok"], true, "switchAccount(202) failed: {switched}");
        assert_eq!(switched["result"]["user_id"], 202);
        let gen_after_switch = switched["result"]["generation"].as_u64().unwrap();

        logout_release.store(true, Ordering::SeqCst);
        let logout_a1_res = slow_logout_a1.join().expect("logout thread must not panic");
        assert_eq!(
            logout_a1_res["ok"], false,
            "Stale auth.logout raced by switchAccount MUST fail closed (B3-01): {logout_a1_res}"
        );
        assert_eq!(logout_a1_res["error"]["code"], "stale_generation");

        // Account B (202) MUST stay active, persisted in host_state.db, with generation unchanged
        assert_eq!(dispatch(host, &json!({"method": "session.wsStatus"}))["result"]["user_id"], 202);
        let accts = dispatch(host, &json!({"method": "session.listAccounts"}));
        assert_eq!(accts["result"]["active_account_id"], "202");
        assert_eq!(
            dispatch(host, &json!({"method": "generation.current"}))["result"]["generation"],
            gen_after_switch,
            "Stale auth.logout MUST NOT bump generation a second time"
        );
        {
            let conn = rusqlite::Connection::open(dir1.path().join("host_state.db")).unwrap();
            let bob_rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM host_kv WHERE namespace = 'accounts' AND key = '202'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                bob_rows, 1,
                "Stale auth.logout MUST NOT delete newly switched Account B from host_state.db"
            );
        }

        // Sub-case A2: Slow auth.logout returning HTTP 200 after concurrent generation.bump
        // (exercises the Ok(()) HTTP path fenced inside session_lock)
        logout_entered.store(false, Ordering::SeqCst);
        logout_release.store(false, Ordering::SeqCst);
        let slow_logout_a2 = thread::spawn(move || {
            dispatch(
                host_addr as *mut c_void,
                &json!({"id": "slow-logout-a2", "method": "auth.logout"}),
            )
        });

        let wait_deadline2 = Instant::now() + Duration::from_secs(3);
        while !logout_entered.load(Ordering::SeqCst) && Instant::now() < wait_deadline2 {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(logout_entered.load(Ordering::SeqCst));

        let bumped = dispatch(host, &json!({"method": "generation.bump"}));
        assert_eq!(bumped["ok"], true);
        let gen_bumped = bumped["result"]["generation"].as_u64().unwrap();
        logout_release.store(true, Ordering::SeqCst);

        let logout_a2_res = slow_logout_a2.join().expect("logout A2 thread must not panic");
        assert_eq!(logout_a2_res["ok"], false);
        assert_eq!(logout_a2_res["error"]["code"], "stale_generation");
        assert_eq!(dispatch(host, &json!({"method": "session.wsStatus"}))["result"]["user_id"], 202);
        assert_eq!(
            dispatch(host, &json!({"method": "generation.current"}))["result"]["generation"],
            gen_bumped
        );

        fabushi_host_destroy(host);
    }

    // Part B: Slow in-flight auth.logout raced by fabushi_host_destroy MUST fail closed
    // with host_closed and perform ZERO SQLite mutations or WAL writes post-destroy!
    let cfg2 = CString::new(
        json!({
            "app_data_dir": dir2.path().to_string_lossy(),
            "server_url": format!("http://{addr}"),
            "storage_passphrase": "keychain-b3-01-destroy-passphrase",
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host2 = fabushi_host_create(cfg2.as_ptr());
        assert!(!host2.is_null());

        let login_res = dispatch(
            host2,
            &json!({"method": "auth.login", "params": {"username": "alice", "password": "pw"}}),
        );
        assert_eq!(login_res["ok"], true);
        let gen_before_destroy = login_res["generation"].as_u64().unwrap();

        logout_entered.store(false, Ordering::SeqCst);
        logout_release.store(false, Ordering::SeqCst);
        let host2_addr = host2 as usize;
        let logout_during_destroy = thread::spawn(move || {
            dispatch(
                host2_addr as *mut c_void,
                &json!({"id": "logout-destroy", "method": "auth.logout"}),
            )
        });

        let wait_deadline3 = Instant::now() + Duration::from_secs(3);
        while !logout_entered.load(Ordering::SeqCst) && Instant::now() < wait_deadline3 {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(logout_entered.load(Ordering::SeqCst));

        fabushi_host_destroy(host2);
        logout_release.store(true, Ordering::SeqCst);

        let destroy_logout_res = logout_during_destroy
            .join()
            .expect("logout thread during destroy must not panic");
        assert_eq!(
            destroy_logout_res["ok"], false,
            "In-flight auth.logout raced by fabushi_host_destroy MUST fail closed (B3-01): {destroy_logout_res}"
        );
        assert_eq!(destroy_logout_res["error"]["code"], "host_closed");

        // Verify host_state.db-wal is truncated (0 bytes) before opening a new inspection connection
        let wal_len = std::fs::metadata(dir2.path().join("host_state.db-wal"))
            .map(|m| m.len())
            .unwrap_or(0);
        assert_eq!(
            wal_len, 0,
            "host_state.db-wal must remain cleanly truncated (0 bytes) after destroy"
        );

        // Verify auth.logout performed ZERO post-destroy mutations to host_state.db
        let conn = rusqlite::Connection::open(dir2.path().join("host_state.db")).unwrap();
        let alice_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM host_kv WHERE namespace = 'accounts' AND key = '101'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            alice_rows, 1,
            "Post-destroy auth.logout MUST NOT delete ('accounts', '101') from host_state.db"
        );
        let active_meta: Option<String> = conn
            .query_row(
                "SELECT value FROM host_meta WHERE key = 'active_account_id'",
                [],
                |r| r.get(0),
            )
            .ok();
        assert_eq!(
            active_meta.as_deref(),
            Some("101"),
            "Post-destroy auth.logout MUST NOT delete active_account_id in host_state.db"
        );
        let persisted_gen: String = conn
            .query_row(
                "SELECT value FROM host_meta WHERE key = 'account_generation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            persisted_gen,
            gen_before_destroy.to_string(),
            "Post-destroy auth.logout MUST NOT bump persisted account_generation in host_state.db"
        );
    }
}

#[test]
fn test_b3_02_session_set_token_wipes_scope_clears_identity_and_null_revokes_bearer() {
    let dir = tempdir().unwrap();
    let recorded_auth_headers: Arc<std::sync::Mutex<Vec<String>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let headers_srv = Arc::clone(&recorded_auth_headers);

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
                        "user_id": 101,
                        "username": "alice_settok",
                        "session_id": "sess-alice-settok",
                        "token": "TOK_ALICE_101",
                        "first_name": "Alice"
                    }))
                }),
            )
            .route(
                "/api/v1/auth/me",
                get(move |headers: HeaderMap| {
                    let log = Arc::clone(&headers_srv);
                    async move {
                        let auth_val = headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        log.lock().unwrap().push(auth_val);
                        Json(json!({
                            "user_id": 202,
                            "username": "token_caller",
                            "first_name": "Caller",
                            "session_id": "sess-me-b302"
                        }))
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
            "server_url": format!("http://{addr}"),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        // 1. Account A logs in, unlocks Vault, masks a secret in Magpie, and calls auth.me
        let login_a = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "alice_settok", "password": "pw"}}),
        );
        assert_eq!(login_a["ok"], true);
        let gen_a = login_a["generation"].as_u64().unwrap();
        assert_eq!(dispatch(host, &json!({"method": "session.wsStatus"}))["result"]["user_id"], 101);

        assert_eq!(
            dispatch(
                host,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 101, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], true);

        let secret_a = "Key=sk-ant-api03-settoksecret1122 email=alice_tok@secret.io";
        let masked_a = dispatch(
            host,
            &json!({"method": "magpie.maskText", "params": {"text": secret_a}}),
        )["result"]["masked_text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!masked_a.contains("settoksecret1122"));
        assert_eq!(
            dispatch(host, &json!({"method": "magpie.unmaskText", "params": {"text": masked_a}}))["result"]["text"],
            secret_a
        );
        assert_eq!(dispatch(host, &json!({"method": "auth.me"}))["ok"], true);
        assert_eq!(
            recorded_auth_headers.lock().unwrap().last().map(String::as_str),
            Some("Bearer TOK_ALICE_101")
        );

        // 2. session.setToken({"token": "TOK_B"}) MUST wipe Vault & Magpie, clear stale user_id,
        //    delete active_account_id, bump generation, and authenticate HTTP with TOK_B!
        let set_b = dispatch(
            host,
            &json!({"method": "session.setToken", "params": {"token": "TOK_B"}}),
        );
        assert_eq!(set_b["ok"], true, "session.setToken(TOK_B) failed: {set_b}");
        assert_eq!(set_b["result"]["has_token"], true);
        let gen_b = set_b["result"]["generation"].as_u64().unwrap();
        assert!(gen_b > gen_a, "session.setToken(TOK_B) must bump generation ({gen_b} > {gen_a})");
        assert_eq!(set_b["generation"], gen_b);

        assert_eq!(
            dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"],
            false,
            "session.setToken(TOK_B) MUST wipe Account A's unlocked Vault key (B3-02)"
        );
        let unmasked_after_b = dispatch(
            host,
            &json!({"method": "magpie.unmaskText", "params": {"text": masked_a}}),
        )["result"]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            !unmasked_after_b.contains("settoksecret1122")
                && !unmasked_after_b.contains("alice_tok@secret.io"),
            "session.setToken(TOK_B) MUST wipe Magpie MemoryVault: {unmasked_after_b}"
        );
        assert_eq!(
            dispatch(host, &json!({"method": "session.wsStatus"}))["result"]["user_id"],
            Value::Null,
            "session.setToken(TOK_B) MUST clear stale user_id to prevent split-brain identity (B3-02)"
        );
        assert_eq!(
            dispatch(host, &json!({"method": "session.listAccounts"}))["result"]["active_account_id"],
            Value::Null
        );
        assert_eq!(dispatch(host, &json!({"method": "auth.me"}))["ok"], true);
        assert_eq!(
            recorded_auth_headers.lock().unwrap().last().map(String::as_str),
            Some("Bearer TOK_B")
        );

        // 3. Re-login Account A (populating BOTH auth_session AND auth_token), unlock Vault, mask secret,
        //    then call session.setToken({"token": null}) -> MUST truly revoke client.token()!
        let login_a2 = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "alice_settok", "password": "pw"}}),
        );
        assert_eq!(login_a2["ok"], true);
        let gen_a2 = login_a2["generation"].as_u64().unwrap();
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "vault.initSplitKey", "params": {"user_id": 101, "password": "pw", "fast_kdf": true}})
            )["ok"],
            true
        );
        let secret_a2 = "Key=sk-ant-api03-nullclearsecret7788 email=clear@secret.io";
        let masked_a2 = dispatch(
            host,
            &json!({"method": "magpie.maskText", "params": {"text": secret_a2}}),
        )["result"]["masked_text"]
            .as_str()
            .unwrap()
            .to_string();

        let me_calls_before_clear = recorded_auth_headers.lock().unwrap().len();
        let clear_null = dispatch(
            host,
            &json!({"method": "session.setToken", "params": {"token": null}}),
        );
        assert_eq!(clear_null["ok"], true, "session.setToken(null) failed: {clear_null}");
        assert_eq!(clear_null["result"]["has_token"], false);
        let gen_cleared = clear_null["result"]["generation"].as_u64().unwrap();
        assert!(gen_cleared > gen_a2);
        assert_eq!(clear_null["generation"], gen_cleared);
        assert_eq!(dispatch(host, &json!({"method": "vault.status"}))["result"]["unlocked"], false);
        let unmasked_after_null = dispatch(
            host,
            &json!({"method": "magpie.unmaskText", "params": {"text": masked_a2}}),
        )["result"]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            !unmasked_after_null.contains("nullclearsecret7788")
                && !unmasked_after_null.contains("clear@secret.io"),
            "session.setToken(null) MUST wipe Magpie MemoryVault: {unmasked_after_null}"
        );
        assert_eq!(
            dispatch(host, &json!({"method": "session.wsStatus"}))["result"]["user_id"],
            Value::Null
        );
        assert_eq!(
            dispatch(host, &json!({"method": "session.listAccounts"}))["result"]["active_account_id"],
            Value::Null
        );

        // Subsequent authenticated HTTP MUST fail closed with unauthorized (zero Authorization header sent!)
        let me_after_clear = dispatch(host, &json!({"method": "auth.me"}));
        assert_eq!(
            me_after_clear["ok"], false,
            "auth.me after session.setToken(null) MUST fail closed (B3-02): {me_after_clear}"
        );
        assert_eq!(me_after_clear["error"]["code"], "unauthorized");
        assert_eq!(
            recorded_auth_headers.lock().unwrap().len(),
            me_calls_before_clear,
            "Zero authenticated HTTP requests may leak stale auth_session bearer token after setToken(null)"
        );

        // 4. Empty/whitespace token string behaves identically to null
        let login_a3 = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "alice_settok", "password": "pw"}}),
        );
        assert_eq!(login_a3["ok"], true);
        let gen_a3 = login_a3["generation"].as_u64().unwrap();
        let clear_ws = dispatch(
            host,
            &json!({"method": "session.setToken", "params": {"token": "   "}}),
        );
        assert_eq!(clear_ws["ok"], true);
        assert_eq!(clear_ws["result"]["has_token"], false);
        assert!(clear_ws["result"]["generation"].as_u64().unwrap() > gen_a3);
        assert_eq!(
            dispatch(host, &json!({"method": "auth.me"}))["error"]["code"],
            "unauthorized"
        );

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_b4_02_zero_cross_account_bearer_token_leakage_under_concurrent_switch_login_set_token() {
    let dir = tempdir().unwrap();
    let wire_log: Arc<std::sync::Mutex<Vec<(String, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let round_a_hits = Arc::new(AtomicUsize::new(0));
    let send_as_a_hits = Arc::new(AtomicUsize::new(0));

    let wire_log_del = Arc::clone(&wire_log);
    let round_a_hits_del = Arc::clone(&round_a_hits);
    let wire_log_me = Arc::clone(&wire_log);
    let wire_log_send_as = Arc::clone(&wire_log);
    let send_as_hits_srv = Arc::clone(&send_as_a_hits);

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async move {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|Json(body): Json<Value>| async move {
                    let user = body["username"].as_str().unwrap_or("");
                    if user == "bob_b402" {
                        Json(json!({
                            "user_id": 202,
                            "username": "bob_b402",
                            "session_id": "sess-bob-b402",
                            "token": "TOK_B",
                            "first_name": "Bob"
                        }))
                    } else {
                        Json(json!({
                            "user_id": 101,
                            "username": "alice_b402",
                            "session_id": "sess-alice-b402",
                            "token": "TOK_A",
                            "first_name": "Alice"
                        }))
                    }
                }),
            )
            .route(
                "/api/v1/auth/sessions/{session_id}",
                delete(move |Path(session_id): Path<String>, headers: HeaderMap| {
                    let log = Arc::clone(&wire_log_del);
                    let hits = Arc::clone(&round_a_hits_del);
                    async move {
                        let auth_val = headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        if session_id.starts_with("acctA-") {
                            hits.fetch_add(1, Ordering::SeqCst);
                        }
                        log.lock().unwrap().push((session_id, auth_val));
                        StatusCode::OK
                    }
                }),
            )
            .route(
                "/api/v1/auth/me",
                get(move |headers: HeaderMap| {
                    let log = Arc::clone(&wire_log_me);
                    async move {
                        let auth_val = headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        log.lock().unwrap().push(("auth.me".to_string(), auth_val));
                        Json(json!({
                            "user_id": 101,
                            "username": "caller_b402",
                            "first_name": "Caller",
                            "session_id": "sess-me-b402"
                        }))
                    }
                }),
            )
            .route(
                "/api/v1/chats/{chat_id}/send_as",
                get(move |Path(chat_id): Path<i64>, headers: HeaderMap| {
                    let log = Arc::clone(&wire_log_send_as);
                    let hits = Arc::clone(&send_as_hits_srv);
                    async move {
                        let auth_val = headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        hits.fetch_add(1, Ordering::SeqCst);
                        log.lock()
                            .unwrap()
                            .push((format!("chats.sendAsPeers:{chat_id}"), auth_val));
                        Json(json!([
                            {
                                "peer_id": 101,
                                "title": "Alice",
                                "username": "alice_b402",
                                "kind": "user",
                                "is_default": true
                            }
                        ]))
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

    // Enable storage_passphrase so persist_account_record runs full HKDF + ChaCha20-Poly1305 AEAD
    // + synchronous SQLite WAL disk I/O inside finalize_authenticated_session!
    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "storage_passphrase": "b402-aead-passphrase",
            "server_url": format!("http://{addr}"),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());
        let host_addr = host as usize;

        // Prime Account B ("202") in host_state.db so session.switchAccount("202") can restore it
        let prime_b = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "bob_b402", "password": "pw"}}),
        );
        assert_eq!(prime_b["ok"], true, "Prime login Bob failed: {prime_b}");

        let mut total_a_wire_hits = 0usize;
        let mut total_stale_rejections = 0usize;

        // Exercise ALL 3 session-installing paths under continuous concurrent Account A RPCs:
        // Rounds 0, 1: auth.login (Account B TOK_B)
        // Rounds 2, 3: session.switchAccount("202") (Account B TOK_B)
        // Rounds 4, 5: session.setToken({"token": "TOK_C"}) (Token-only TOK_C)
        for round in 0..6usize {
            let login_a = dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "alice_b402", "password": "pw"}}),
            );
            assert_eq!(login_a["ok"], true, "Round {round} login Alice failed: {login_a}");
            let gen_a = login_a["generation"].as_u64().unwrap();

            // Baseline verification at gen_a
            let base_me = dispatch(
                host,
                &json!({"method": "auth.me", "generation": gen_a}),
            );
            assert_eq!(base_me["ok"], true);
            assert_eq!(
                wire_log.lock().unwrap().last().map(|(_, auth)| auth.as_str()),
                Some("Bearer TOK_A")
            );

            round_a_hits.store(0, Ordering::SeqCst);
            let round_log_start = wire_log.lock().unwrap().len();
            let round_stop = Arc::new(AtomicBool::new(false));
            let stale_counter = Arc::new(AtomicUsize::new(0));
            let barrier = Arc::new(Barrier::new(5));

            let mut workers = Vec::new();
            for w in 0..4usize {
                let stop_w = Arc::clone(&round_stop);
                let stale_w = Arc::clone(&stale_counter);
                let bar_w = Arc::clone(&barrier);
                workers.push(thread::spawn(move || {
                    bar_w.wait();
                    let mut seq = 0usize;
                    while !stop_w.load(Ordering::Acquire) {
                        let resp = match seq % 3 {
                            0 => dispatch(
                                host_addr as *mut c_void,
                                &json!({
                                    "id": format!("b402-r{round}-w{w}-s{seq}"),
                                    "method": "session.revoke",
                                    "generation": gen_a,
                                    "params": { "session_id": format!("acctA-r{round}-g{gen_a}-w{w}-{seq}") }
                                }),
                            ),
                            1 => dispatch(
                                host_addr as *mut c_void,
                                &json!({
                                    "id": format!("b402-r{round}-w{w}-m{seq}"),
                                    "method": "auth.me",
                                    "generation": gen_a
                                }),
                            ),
                            _ => dispatch(
                                host_addr as *mut c_void,
                                &json!({
                                    "id": format!("b402-r{round}-w{w}-p{seq}"),
                                    "method": "chats.sendAsPeers",
                                    "generation": gen_a,
                                    "params": { "chat_id": 777 }
                                }),
                            ),
                        };
                        if resp["ok"] == false && resp["error"]["code"] == "stale_generation" {
                            stale_w.fetch_add(1, Ordering::Relaxed);
                        }
                        seq += 1;
                    }
                }));
            }

            // Release all 4 workers and wait until genuine Account A traffic is actively hitting the server
            barrier.wait();
            let active_deadline = Instant::now() + Duration::from_secs(2);
            while round_a_hits.load(Ordering::SeqCst) < 4 && Instant::now() < active_deadline {
                thread::sleep(Duration::from_millis(2));
            }
            assert!(
                round_a_hits.load(Ordering::SeqCst) >= 4,
                "Round {round} workers must actively execute Account A RPCs before transition"
            );

            // Execute the racing session transition while all 4 workers hammer generation == gen_a
            let (transition_resp, expected_after_auth) = match round {
                0 | 1 => (
                    dispatch(
                        host,
                        &json!({"method": "auth.login", "params": {"username": "bob_b402", "password": "pw"}}),
                    ),
                    "Bearer TOK_B",
                ),
                2 | 3 => (
                    dispatch(
                        host,
                        &json!({"method": "session.switchAccount", "params": {"account_id": "202"}}),
                    ),
                    "Bearer TOK_B",
                ),
                _ => (
                    dispatch(
                        host,
                        &json!({"method": "session.setToken", "params": {"token": "TOK_C"}}),
                    ),
                    "Bearer TOK_C",
                ),
            };
            assert_eq!(
                transition_resp["ok"], true,
                "Round {round} transition failed: {transition_resp}"
            );
            let new_gen = transition_resp["generation"].as_u64().unwrap();
            assert!(new_gen > gen_a, "Round {round} must advance generation");

            // Keep hammering with stale gen_a for 25ms AFTER transition completes
            thread::sleep(Duration::from_millis(25));
            round_stop.store(true, Ordering::Release);
            for w in workers {
                w.join().expect("B4-02 worker thread must not panic");
            }

            // Inspect EVERY wire request observed during the round: ZERO gen_a RPCs may EVER
            // carry Bearer TOK_B or Bearer TOK_C!
            {
                let entries = wire_log.lock().unwrap();
                for (tag, auth_header) in &entries[round_log_start..] {
                    assert_eq!(
                        auth_header, "Bearer TOK_A",
                        "CROSS-ACCOUNT BEARER TOKEN LEAKAGE in round {round} on tag='{tag}': got '{auth_header}' instead of 'Bearer TOK_A' (B4-02)"
                    );
                }
            }

            total_a_wire_hits += round_a_hits.load(Ordering::SeqCst);
            total_stale_rejections += stale_counter.load(Ordering::Relaxed);

            // Verify the new session genuinely transmits TOK_B / TOK_C at new_gen
            let verify_new = dispatch(
                host,
                &json!({"method": "auth.me", "generation": new_gen}),
            );
            assert_eq!(verify_new["ok"], true, "Round {round} post-transition auth.me failed: {verify_new}");
            assert_eq!(
                wire_log.lock().unwrap().last().map(|(_, auth)| auth.as_str()),
                Some(expected_after_auth),
                "Round {round} post-transition auth.me must authenticate with {expected_after_auth}"
            );
        }

        assert!(
            total_a_wire_hits >= 24,
            "Expected >= 24 real Account A wire hits across 6 rounds, got {total_a_wire_hits}"
        );
        assert!(
            send_as_a_hits.load(Ordering::SeqCst) > 0,
            "Expected direct fenced_client_token arm (chats.sendAsPeers) to execute real wire requests (CI NB5-02)"
        );
        assert!(
            total_stale_rejections > 0,
            "Expected concurrent stale_generation rejections across 6 rounds, got 0"
        );

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_nb4_01_persistence_rollback_preserves_storage_error_while_raced_network_error_returns_stale_generation() {
    let dir = tempdir().unwrap();
    let slow_entered = Arc::new(AtomicBool::new(false));
    let slow_release = Arc::new(AtomicBool::new(false));
    let slow_entered_srv = Arc::clone(&slow_entered);
    let slow_release_srv = Arc::clone(&slow_release);

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async move {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(move |Json(body): Json<Value>| {
                    let entered = Arc::clone(&slow_entered_srv);
                    let release = Arc::clone(&slow_release_srv);
                    async move {
                        match body["username"].as_str().unwrap_or("") {
                            "charlie_kv_fail" => (
                                StatusCode::OK,
                                Json(json!({
                                    "user_id": 303,
                                    "username": "charlie_kv_fail",
                                    "session_id": "sess-charlie-303",
                                    "token": "TOK_CHARLIE_303",
                                    "first_name": "Charlie"
                                })),
                            ),
                            "dave_meta_fail" => (
                                StatusCode::OK,
                                Json(json!({
                                    "user_id": 404,
                                    "username": "dave_meta_fail",
                                    "session_id": "sess-dave-404",
                                    "token": "TOK_DAVE_404",
                                    "first_name": "Dave"
                                })),
                            ),
                            "slow_net_fail" => {
                                entered.store(true, Ordering::SeqCst);
                                let deadline = Instant::now() + Duration::from_secs(5);
                                while !release.load(Ordering::SeqCst) && Instant::now() < deadline {
                                    tokio::time::sleep(Duration::from_millis(5)).await;
                                }
                                (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    Json(json!({ "error": "injected_upstream_500" })),
                                )
                            }
                            _ => (
                                StatusCode::OK,
                                Json(json!({
                                    "user_id": 101,
                                    "username": "alice_nb401",
                                    "session_id": "sess-alice-101",
                                    "token": "TOK_ALICE_101",
                                    "first_name": "Alice"
                                })),
                            ),
                        }
                    }
                }),
            )
            .route(
                "/api/v1/auth/me",
                get(|| async {
                    Json(json!({
                        "user_id": 101,
                        "username": "alice_nb401",
                        "first_name": "Alice",
                        "session_id": "sess-alice-101"
                    }))
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
            "server_url": format!("http://{addr}"),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());
        let host_addr = host as usize;

        let login_alice = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "alice_nb401", "password": "pw"}}),
        );
        assert_eq!(login_alice["ok"], true);
        let gen_alice = login_alice["generation"].as_u64().unwrap();

        // Inject deterministic SQLite failures on host_kv ('303') and host_meta ('active_account_id' = '404')
        {
            let conn = rusqlite::Connection::open(dir.path().join("host_state.db")).unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_charlie_kv
                 BEFORE INSERT ON host_kv
                 WHEN NEW.namespace = 'accounts' AND NEW.key = '303'
                 BEGIN SELECT RAISE(ABORT, 'injected_kv_failure'); END;

                 CREATE TRIGGER fail_dave_meta_ins
                 BEFORE INSERT ON host_meta
                 WHEN NEW.key = 'active_account_id' AND NEW.value = '404'
                 BEGIN SELECT RAISE(ABORT, 'injected_meta_failure'); END;

                 CREATE TRIGGER fail_dave_meta_upd
                 BEFORE UPDATE ON host_meta
                 WHEN NEW.key = 'active_account_id' AND NEW.value = '404'
                 BEGIN SELECT RAISE(ABORT, 'injected_meta_failure'); END;",
            )
            .unwrap();
        }

        // 1. persist_account_record kv_set failure MUST preserve storage_error (NOT stale_generation),
        //    bump generation on rollback, keep live_client cleared, and NOT persist account 303.
        let charlie_res = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "charlie_kv_fail", "password": "pw"}}),
        );
        assert_eq!(charlie_res["ok"], false);
        assert_eq!(
            charlie_res["error"]["code"], "storage_error",
            "Rollback storage error on kv_set MUST NOT be masked as stale_generation (NB4-01): {charlie_res}"
        );
        let gen_after_charlie = charlie_res["generation"].as_u64().unwrap();
        assert!(gen_after_charlie > gen_alice);
        assert_eq!(
            dispatch(host, &json!({"method": "auth.me"}))["error"]["code"],
            "unauthorized",
            "Rolled-back auth.login MUST leave live_client token cleared (B4-02)"
        );
        let switch_charlie = dispatch(
            host,
            &json!({"method": "session.switchAccount", "params": {"account_id": "303"}}),
        );
        assert_eq!(
            switch_charlie["ok"], false,
            "Rolled-back account 303 must fail closed on switchAccount (CI NB5-01): {switch_charlie}"
        );
        assert_eq!(switch_charlie["error"]["code"], "not_found");
        let list_after_charlie = dispatch(host, &json!({"method": "session.listAccounts"}));
        assert_eq!(list_after_charlie["result"]["active_account_id"], Value::Null);
        let charlie_present = list_after_charlie["result"]["accounts"]
            .as_array()
            .expect("accounts array")
            .iter()
            .any(|a| a["account_id"] == "303");
        assert!(
            !charlie_present,
            "Rolled-back account 303 must be absent from session.listAccounts (CI NB5-01): {list_after_charlie}"
        );

        // 2. set_meta("active_account_id") failure MUST preserve storage_error (NOT stale_generation),
        //    bump generation on rollback, and leave live_client cleared.
        let dave_res = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "dave_meta_fail", "password": "pw"}}),
        );
        assert_eq!(dave_res["ok"], false);
        assert_eq!(
            dave_res["error"]["code"], "storage_error",
            "Rollback storage error on set_meta MUST NOT be masked as stale_generation (NB4-01): {dave_res}"
        );
        let gen_after_dave = dave_res["generation"].as_u64().unwrap();
        assert!(gen_after_dave > gen_after_charlie);
        assert_eq!(
            dispatch(host, &json!({"method": "auth.me"}))["error"]["code"],
            "unauthorized"
        );
        assert_eq!(
            dispatch(host, &json!({"method": "session.listAccounts"}))["result"]["active_account_id"],
            Value::Null
        );

        // Clean up injected SQLite triggers
        {
            let conn = rusqlite::Connection::open(dir.path().join("host_state.db")).unwrap();
            conn.execute_batch(
                "DROP TRIGGER fail_charlie_kv;
                 DROP TRIGGER fail_dave_meta_ins;
                 DROP TRIGGER fail_dave_meta_upd;",
            )
            .unwrap();
        }

        // 3. Contrast case: a slow failing network auth.login where THIS handler did NOT bump
        //    generation, raced by another thread bumping generation, MUST STILL return stale_generation!
        let slow_thread = thread::spawn(move || {
            dispatch(
                host_addr as *mut c_void,
                &json!({"method": "auth.login", "params": {"username": "slow_net_fail", "password": "pw"}}),
            )
        });
        let enter_deadline = Instant::now() + Duration::from_secs(3);
        while !slow_entered.load(Ordering::SeqCst) && Instant::now() < enter_deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(slow_entered.load(Ordering::SeqCst));
        let bumped = dispatch(host, &json!({"method": "generation.bump"}));
        assert_eq!(bumped["ok"], true);
        slow_release.store(true, Ordering::SeqCst);

        let slow_res = slow_thread.join().expect("slow_net_fail thread must not panic");
        assert_eq!(slow_res["ok"], false);
        assert_eq!(
            slow_res["error"]["code"], "stale_generation",
            "Network failure raced by another thread's generation bump MUST still fail closed with stale_generation (NB4-01): {slow_res}"
        );

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_b5_01_per_poll_fence_blocks_poll2_bearer_token_leak_on_multistep_rpcs() {
    let dir = tempdir().unwrap();

    let wire_log: Arc<std::sync::Mutex<Vec<(String, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let count_gate_armed = Arc::new(AtomicBool::new(false));
    let count_entered = Arc::new(AtomicBool::new(false));
    let count_release = Arc::new(AtomicBool::new(false));
    let publish_hits = Arc::new(AtomicUsize::new(0));

    let sync_gate_armed = Arc::new(AtomicBool::new(false));
    let sync_entered = Arc::new(AtomicBool::new(false));
    let sync_release = Arc::new(AtomicBool::new(false));
    let sync_round_calls = Arc::new(AtomicUsize::new(0));
    let sync_total_hits = Arc::new(AtomicUsize::new(0));

    let log_count = Arc::clone(&wire_log);
    let gate_armed_srv = Arc::clone(&count_gate_armed);
    let entered_srv = Arc::clone(&count_entered);
    let release_srv = Arc::clone(&count_release);

    let log_pub = Arc::clone(&wire_log);
    let pub_hits_srv = Arc::clone(&publish_hits);

    let log_sync = Arc::clone(&wire_log);
    let sync_armed_srv = Arc::clone(&sync_gate_armed);
    let sync_entered_srv = Arc::clone(&sync_entered);
    let sync_release_srv = Arc::clone(&sync_release);
    let sync_round_srv = Arc::clone(&sync_round_calls);
    let sync_total_srv = Arc::clone(&sync_total_hits);

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let addr = server_rt.block_on(async move {
        let app = Router::new()
            .route(
                "/api/v1/auth/login",
                post(|Json(body): Json<Value>| async move {
                    if body["username"].as_str() == Some("bob_b501") {
                        Json(json!({
                            "user_id": 202,
                            "username": "bob_b501",
                            "session_id": "sess-bob-b501",
                            "token": "TOK_B",
                            "first_name": "Bob"
                        }))
                    } else {
                        Json(json!({
                            "user_id": 101,
                            "username": "alice_b501",
                            "session_id": "sess-alice-b501",
                            "token": "TOK_A",
                            "first_name": "Alice"
                        }))
                    }
                }),
            )
            .route(
                "/api/v1/e2ee/prekeys/count",
                get(move |headers: HeaderMap| {
                    let log = Arc::clone(&log_count);
                    let armed = Arc::clone(&gate_armed_srv);
                    let entered = Arc::clone(&entered_srv);
                    let release = Arc::clone(&release_srv);
                    async move {
                        let auth_val = headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        log.lock()
                            .unwrap()
                            .push(("GET /e2ee/prekeys/count".to_string(), auth_val));
                        if armed.load(Ordering::SeqCst) {
                            entered.store(true, Ordering::SeqCst);
                            let deadline = Instant::now() + Duration::from_secs(5);
                            while !release.load(Ordering::SeqCst) && Instant::now() < deadline {
                                tokio::time::sleep(Duration::from_millis(2)).await;
                            }
                        }
                        // Return low count & no signed prekey so replenish_prekeys_if_low MUST
                        // proceed to Step 2: init_and_publish_prekeys -> POST /api/v1/e2ee/prekeys.
                        Json(json!({
                            "user_id": 101,
                            "device_id": "dev-b501",
                            "remaining_one_time_prekeys": 0,
                            "has_signed_prekey": false
                        }))
                    }
                }),
            )
            .route(
                "/api/v1/e2ee/prekeys",
                post(move |headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = Arc::clone(&log_pub);
                    let hits = Arc::clone(&pub_hits_srv);
                    async move {
                        let auth_val = headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        hits.fetch_add(1, Ordering::SeqCst);
                        log.lock()
                            .unwrap()
                            .push(("POST /e2ee/prekeys".to_string(), auth_val));
                        Json(json!({
                            "success": true,
                            "user_id": 101,
                            "device_id": "dev-b501",
                            "remaining_one_time_prekeys": 5,
                            "updated_at_ms": Utc::now().timestamp_millis()
                        }))
                    }
                }),
            )
            .route(
                "/api/v1/sync/get_difference",
                post(move |headers: HeaderMap, Json(body): Json<Value>| {
                    let log = Arc::clone(&log_sync);
                    let armed = Arc::clone(&sync_armed_srv);
                    let entered = Arc::clone(&sync_entered_srv);
                    let release = Arc::clone(&sync_release_srv);
                    let round_calls = Arc::clone(&sync_round_srv);
                    let total_hits = Arc::clone(&sync_total_srv);
                    async move {
                        let cid = body["chat_id"].as_i64().unwrap_or(0);
                        let auth_val = headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        total_hits.fetch_add(1, Ordering::SeqCst);
                        let call_idx = round_calls.fetch_add(1, Ordering::SeqCst);
                        log.lock()
                            .unwrap()
                            .push((format!("POST /sync/get_difference:{cid}"), auth_val));
                        if armed.load(Ordering::SeqCst) && call_idx == 0 {
                            entered.store(true, Ordering::SeqCst);
                            let deadline = Instant::now() + Duration::from_secs(5);
                            while !release.load(Ordering::SeqCst) && Instant::now() < deadline {
                                tokio::time::sleep(Duration::from_millis(2)).await;
                            }
                        }
                        Json(json!({
                            "chat_id": cid,
                            "new_messages": [],
                            "other_updates": [],
                            "pts": 10,
                            "current_pts": 10,
                            "updates": [],
                            "has_more": false,
                            "snapshot_required": false
                        }))
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
            "storage_passphrase": "b501-per-poll-passphrase",
            "server_url": format!("http://{addr}"),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());
        let host_addr = host as usize;

        // Prime Bob ("202") in host_state.db so session.switchAccount("202") works
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "bob_b501", "password": "pw"}})
            )["ok"],
            true
        );
        // Login Alice ("101", TOK_A)
        let login_a = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "alice_b501", "password": "pw"}}),
        );
        assert_eq!(login_a["ok"], true);
        let gen_a0 = login_a["generation"].as_u64().unwrap();

        // === Scenario A1 (Non-Tautology Baseline): e2ee.replenishPrekeys genuinely triggers
        //     Step 2 POST /api/v1/e2ee/prekeys with Bearer TOK_A ===
        let base_replenish = dispatch(
            host,
            &json!({
                "method": "e2ee.replenishPrekeys",
                "generation": gen_a0,
                "params": { "min_threshold": 10, "batch_size": 5 }
            }),
        );
        assert_eq!(
            base_replenish["ok"], true,
            "Baseline e2ee.replenishPrekeys must succeed: {base_replenish}"
        );
        assert_eq!(
            publish_hits.load(Ordering::SeqCst),
            1,
            "Non-tautology proof: baseline replenishPrekeys MUST execute Step 2 POST /e2ee/prekeys"
        );
        {
            let entries = wire_log.lock().unwrap();
            assert!(
                entries
                    .iter()
                    .any(|(ep, auth)| ep == "GET /e2ee/prekeys/count" && auth == "Bearer TOK_A"),
                "Baseline must send Step 1 GET with Bearer TOK_A: {entries:?}"
            );
            assert!(
                entries
                    .iter()
                    .any(|(ep, auth)| ep == "POST /e2ee/prekeys" && auth == "Bearer TOK_A"),
                "Baseline must send Step 2 POST with Bearer TOK_A: {entries:?}"
            );
        }

        // === Scenario A2 (Direct Poll-2 Fence WITHOUT token.cancel()): race e2ee.replenishPrekeys
        //     with generation.bump at the Step-1 barrier ===
        {
            let log_start = wire_log.lock().unwrap().len();
            let pub_before = publish_hits.load(Ordering::SeqCst);
            count_entered.store(false, Ordering::SeqCst);
            count_release.store(false, Ordering::SeqCst);
            count_gate_armed.store(true, Ordering::SeqCst);

            let worker = thread::spawn(move || {
                dispatch(
                    host_addr as *mut c_void,
                    &json!({
                        "id": "b501-e2ee-genbump",
                        "method": "e2ee.replenishPrekeys",
                        "generation": gen_a0,
                        "params": { "min_threshold": 10, "batch_size": 5 }
                    }),
                )
            });

            let deadline = Instant::now() + Duration::from_secs(3);
            while !count_entered.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            assert!(
                count_entered.load(Ordering::SeqCst),
                "Step 1 GET /e2ee/prekeys/count must enter barrier"
            );

            // generation.bump advances generation WITHOUT cancelling in_flight_cancels, directly
            // exercising PerPollFencedFuture's Poll-2+ fence when the GET response completes!
            let bumped = dispatch(host, &json!({"method": "generation.bump"}));
            assert_eq!(bumped["ok"], true);
            count_release.store(true, Ordering::SeqCst);

            let res = worker.join().expect("e2ee genbump worker must not panic");
            count_gate_armed.store(false, Ordering::SeqCst);
            assert_eq!(res["ok"], false, "Raced replenishPrekeys must fail closed: {res}");
            assert_eq!(
                res["error"]["code"], "stale_generation",
                "Poll-2 PerPollFencedFuture MUST fail closed with stale_generation on generation.bump: {res}"
            );
            assert_eq!(
                publish_hits.load(Ordering::SeqCst),
                pub_before,
                "PerPollFencedFuture MUST block Step 2 POST /e2ee/prekeys on Poll 2 after generation bump (B5-01)"
            );
            let entries = wire_log.lock().unwrap();
            assert!(
                !entries[log_start..]
                    .iter()
                    .any(|(ep, _)| ep == "POST /e2ee/prekeys"),
                "Zero Step 2 POST /e2ee/prekeys allowed after generation.bump: {:?}",
                &entries[log_start..]
            );
        }

        // === Scenario A3: Race e2ee.replenishPrekeys at Step-1 barrier against ALL 3 account
        //     transitions (switchAccount("202"), auth.login(Bob), session.setToken("TOK_C")) ===
        for round in 0..3usize {
            let login_round = dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "alice_b501", "password": "pw"}}),
            );
            assert_eq!(login_round["ok"], true);
            let gen_a = login_round["generation"].as_u64().unwrap();

            let log_start = wire_log.lock().unwrap().len();
            count_entered.store(false, Ordering::SeqCst);
            count_release.store(false, Ordering::SeqCst);
            count_gate_armed.store(true, Ordering::SeqCst);

            let worker = thread::spawn(move || {
                dispatch(
                    host_addr as *mut c_void,
                    &json!({
                        "id": format!("b501-e2ee-r{round}"),
                        "method": "e2ee.replenishPrekeys",
                        "generation": gen_a,
                        "params": { "min_threshold": 10, "batch_size": 5 }
                    }),
                )
            });

            let deadline = Instant::now() + Duration::from_secs(3);
            while !count_entered.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            assert!(count_entered.load(Ordering::SeqCst));

            // Execute account transition WHILE Worker is parked at the Step-1 barrier, then
            // release the barrier so the server task finishes cleanly without hanging:
            // worker_res["ok"] == false is 100% deterministic (zero OS-scheduler flakiness).
            let (trans_resp, expected_new_auth) = match round {
                0 => (
                    dispatch(
                        host,
                        &json!({"method": "session.switchAccount", "params": {"account_id": "202"}}),
                    ),
                    "Bearer TOK_B",
                ),
                1 => (
                    dispatch(
                        host,
                        &json!({"method": "auth.login", "params": {"username": "bob_b501", "password": "pw"}}),
                    ),
                    "Bearer TOK_B",
                ),
                _ => (
                    dispatch(
                        host,
                        &json!({"method": "session.setToken", "params": {"token": "TOK_C"}}),
                    ),
                    "Bearer TOK_C",
                ),
            };
            count_release.store(true, Ordering::SeqCst);
            assert_eq!(trans_resp["ok"], true, "Round {round} transition failed: {trans_resp}");
            let new_gen = trans_resp["generation"].as_u64().unwrap();

            let worker_res = worker.join().expect("e2ee race worker must not panic");
            count_gate_armed.store(false, Ordering::SeqCst);
            assert_eq!(
                worker_res["ok"], false,
                "Round {round} Account A replenishPrekeys must fail closed: {worker_res}"
            );
            assert!(
                matches!(
                    worker_res["error"]["code"].as_str(),
                    Some("stale_generation" | "cancelled")
                ),
                "Round {round} expected stale_generation or cancelled, got: {worker_res}"
            );

            // ZERO requests from Account A's RPC may EVER carry Account B / TOK_C's bearer token!
            {
                let entries = wire_log.lock().unwrap();
                for (ep, auth) in &entries[log_start..] {
                    assert_eq!(
                        auth, "Bearer TOK_A",
                        "POLL-2+ CROSS-ACCOUNT BEARER TOKEN LEAK in e2ee.replenishPrekeys round {round} on {ep}: got '{auth}' (B5-01)"
                    );
                }
            }

            // Confirm post-transition replenishPrekeys at new_gen genuinely uses the new token
            let verify_after = dispatch(
                host,
                &json!({
                    "method": "e2ee.replenishPrekeys",
                    "generation": new_gen,
                    "params": { "min_threshold": 10, "batch_size": 5 }
                }),
            );
            assert_eq!(verify_after["ok"], true, "Post-transition replenishPrekeys failed: {verify_after}");
            assert_eq!(
                wire_log.lock().unwrap().last().map(|(ep, auth)| (ep.as_str(), auth.as_str())),
                Some(("POST /e2ee/prekeys", expected_new_auth))
            );
        }

        // === Scenario A4: Tight simultaneous-release Poll-2 race (releasing Step-1 response at
        //     the exact moment switchAccount / setToken executes, exercising Poll 2 vs session_lock.write()) ===
        for sim_round in 0..6usize {
            let login_sim = dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "alice_b501", "password": "pw"}}),
            );
            assert_eq!(login_sim["ok"], true);
            let gen_a = login_sim["generation"].as_u64().unwrap();

            let log_start = wire_log.lock().unwrap().len();
            count_entered.store(false, Ordering::SeqCst);
            count_release.store(false, Ordering::SeqCst);
            count_gate_armed.store(true, Ordering::SeqCst);

            let worker = thread::spawn(move || {
                dispatch(
                    host_addr as *mut c_void,
                    &json!({
                        "id": format!("b501-e2ee-sim-{sim_round}"),
                        "method": "e2ee.replenishPrekeys",
                        "generation": gen_a,
                        "params": { "min_threshold": 10, "batch_size": 5 }
                    }),
                )
            });

            let deadline = Instant::now() + Duration::from_secs(3);
            while !count_entered.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            assert!(count_entered.load(Ordering::SeqCst));

            // Release Step-1 simultaneously with account transition so Poll 2 genuinely races session_lock.write()
            count_release.store(true, Ordering::SeqCst);
            let trans = if sim_round % 2 == 0 {
                dispatch(
                    host,
                    &json!({"method": "session.switchAccount", "params": {"account_id": "202"}}),
                )
            } else {
                dispatch(
                    host,
                    &json!({"method": "session.setToken", "params": {"token": "TOK_C"}}),
                )
            };
            assert_eq!(trans["ok"], true);
            let _ = worker.join().expect("simultaneous e2ee race worker must not panic");
            count_gate_armed.store(false, Ordering::SeqCst);

            let entries = wire_log.lock().unwrap();
            for (ep, auth) in &entries[log_start..] {
                assert_eq!(
                    auth, "Bearer TOK_A",
                    "SIMULTANEOUS POLL-2+ BEARER TOKEN LEAK in round {sim_round} on {ep}: got '{auth}' (B5-01)"
                );
            }
        }

        // === Scenario B: sync.syncAllGaps across 2 pending gap chats with barrier after Chat 1 ===
        let seed_two_gaps = |host: *mut c_void, c1: i64, c2: i64| {
            for cid in [c1, c2] {
                assert_eq!(
                    dispatch(host, &json!({"method": "sync.setPts", "params": {"chat_id": cid, "pts": 1}}))["ok"],
                    true
                );
                let frame = WsServerFrame::PtsPush(PtsUpdate::new(
                    ChatId(cid),
                    5,
                    1,
                    UpdateEvent::ReadHistoryIn {
                        max_id: MessageId(10),
                    },
                    1_700_000_000_000,
                ));
                let applied = dispatch(
                    host,
                    &json!({"method": "sync.applyWsFrame", "params": serde_json::to_value(frame).unwrap()}),
                );
                assert_eq!(applied["ok"], true, "sync.applyWsFrame failed: {applied}");
            }
            let pending = dispatch(host, &json!({"method": "sync.pendingGaps"}));
            assert_eq!(
                pending["result"]["gap_chats"].as_array().map(|a| a.len()),
                Some(2),
                "Expected exactly 2 pending gap chats: {pending}"
            );
        };

        // Login Alice and run non-tautology baseline across 2 gap chats
        let login_sync_base = dispatch(
            host,
            &json!({"method": "auth.login", "params": {"username": "alice_b501", "password": "pw"}}),
        );
        assert_eq!(login_sync_base["ok"], true);
        let gen_sync_base = login_sync_base["generation"].as_u64().unwrap();
        seed_two_gaps(host, 701, 702);
        sync_total_hits.store(0, Ordering::SeqCst);
        sync_round_calls.store(0, Ordering::SeqCst);
        let base_sync = dispatch(
            host,
            &json!({"method": "sync.syncAllGaps", "generation": gen_sync_base, "params": {"limit": 50}}),
        );
        assert_eq!(base_sync["ok"], true, "Baseline syncAllGaps failed: {base_sync}");
        assert_eq!(
            sync_total_hits.load(Ordering::SeqCst),
            2,
            "Non-tautology proof: baseline syncAllGaps MUST dispatch both gap chats"
        );
        assert_eq!(
            dispatch(host, &json!({"method": "sync.pendingGaps"}))["result"]["gap_chats"]
                .as_array()
                .map(|a| a.len()),
            Some(0)
        );

        // Scenario B1: Direct Poll-2 fence on sync.syncAllGaps via generation.bump (NO token.cancel())
        {
            seed_two_gaps(host, 703, 704);
            sync_entered.store(false, Ordering::SeqCst);
            sync_release.store(false, Ordering::SeqCst);
            sync_round_calls.store(0, Ordering::SeqCst);
            sync_gate_armed.store(true, Ordering::SeqCst);

            let worker = thread::spawn(move || {
                dispatch(
                    host_addr as *mut c_void,
                    &json!({
                        "id": "b501-sync-genbump",
                        "method": "sync.syncAllGaps",
                        "generation": gen_sync_base,
                        "params": { "limit": 50 }
                    }),
                )
            });

            let deadline = Instant::now() + Duration::from_secs(3);
            while !sync_entered.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            assert!(sync_entered.load(Ordering::SeqCst));

            assert_eq!(dispatch(host, &json!({"method": "generation.bump"}))["ok"], true);
            sync_release.store(true, Ordering::SeqCst);

            let res = worker.join().expect("sync genbump worker must not panic");
            sync_gate_armed.store(false, Ordering::SeqCst);
            assert_eq!(res["ok"], false);
            assert_eq!(
                res["error"]["code"], "stale_generation",
                "Poll-2 PerPollFencedFuture MUST abort sync.syncAllGaps before Chat 2 on generation.bump: {res}"
            );
            assert_eq!(
                sync_round_calls.load(Ordering::SeqCst),
                1,
                "Chat 2 MUST NOT be dispatched after Poll-1 generation bump (B5-01)"
            );
        }

        // Drain any remaining gap chat cleanly before Scenario B2
        assert_eq!(
            dispatch(host, &json!({"method": "sync.syncAllGaps", "params": {"limit": 50}}))["ok"],
            true
        );

        // Scenario B2: Race sync.syncAllGaps across 2 gap chats against session.setToken("TOK_B")
        {
            let login_sync2 = dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "alice_b501", "password": "pw"}}),
            );
            assert_eq!(login_sync2["ok"], true);
            let gen_sync2 = login_sync2["generation"].as_u64().unwrap();
            seed_two_gaps(host, 705, 706);

            let log_start = wire_log.lock().unwrap().len();
            sync_entered.store(false, Ordering::SeqCst);
            sync_release.store(false, Ordering::SeqCst);
            sync_round_calls.store(0, Ordering::SeqCst);
            sync_gate_armed.store(true, Ordering::SeqCst);

            let worker = thread::spawn(move || {
                dispatch(
                    host_addr as *mut c_void,
                    &json!({
                        "id": "b501-sync-settoken",
                        "method": "sync.syncAllGaps",
                        "generation": gen_sync2,
                        "params": { "limit": 50 }
                    }),
                )
            });

            let deadline = Instant::now() + Duration::from_secs(3);
            while !sync_entered.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            assert!(sync_entered.load(Ordering::SeqCst));

            let set_b = dispatch(
                host,
                &json!({"method": "session.setToken", "params": {"token": "TOK_B"}}),
            );
            sync_release.store(true, Ordering::SeqCst);
            assert_eq!(set_b["ok"], true);

            let res = worker.join().expect("sync setToken worker must not panic");
            sync_gate_armed.store(false, Ordering::SeqCst);
            assert_eq!(res["ok"], false);
            assert!(
                matches!(
                    res["error"]["code"].as_str(),
                    Some("stale_generation" | "cancelled")
                ),
                "Expected stale_generation or cancelled on raced syncAllGaps, got: {res}"
            );
            let entries = wire_log.lock().unwrap();
            for (ep, auth) in &entries[log_start..] {
                assert_eq!(
                    auth, "Bearer TOK_A",
                    "POLL-2+ CROSS-ACCOUNT BEARER TOKEN LEAK in sync.syncAllGaps on {ep}: got '{auth}' (B5-01)"
                );
            }
        }

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_b5_02_local_store_outbox_and_e2ee_wiped_across_token_logout_clear_and_server_switch() {
    let dir = tempdir().unwrap();
    let sent_log: Arc<std::sync::Mutex<Vec<(String, String, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    async fn quiet_ws_handler(
        ws: WebSocketUpgrade,
        Query(_q): Query<std::collections::HashMap<String, String>>,
    ) -> impl IntoResponse {
        ws.on_upgrade(|mut socket: WebSocket| async move {
            while let Some(Ok(_)) = socket.recv().await {}
        })
    }

    let spawn_mock_server = |label: &'static str,
                             sent_log_ref: Arc<std::sync::Mutex<Vec<(String, String, String)>>>| {
        async move {
            let log_send = Arc::clone(&sent_log_ref);
            let app = Router::new()
                .route("/health", get(|| async { Json(json!({"status": "ok"})) }))
                .route(
                    "/api/v1/auth/login",
                    post(|Json(body): Json<Value>| async move {
                        if body["username"].as_str() == Some("bob_b502") {
                            Json(json!({
                                "user_id": 202,
                                "username": "bob_b502",
                                "session_id": "sess-bob-b502",
                                "token": "TOK_B",
                                "first_name": "Bob"
                            }))
                        } else {
                            Json(json!({
                                "user_id": 101,
                                "username": "alice_b502",
                                "session_id": "sess-alice-b502",
                                "token": "TOK_A",
                                "first_name": "Alice"
                            }))
                        }
                    }),
                )
                .route("/api/v1/auth/logout", post(|| async { StatusCode::OK }))
                .route("/api/v1/ws", get(quiet_ws_handler))
                .route(
                    "/api/v1/messages/send",
                    post(move |headers: HeaderMap, Json(body): Json<Value>| {
                        let log = Arc::clone(&log_send);
                        async move {
                            let auth_val = headers
                                .get(header::AUTHORIZATION)
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                            let chat_id = body["chat_id"].as_i64().unwrap_or(0);
                            let text = body["text"].as_str().unwrap_or("").to_string();
                            let nonce = body["client_nonce"].as_u64();
                            log.lock().unwrap().push((
                                label.to_string(),
                                auth_val,
                                text.clone(),
                            ));
                            let now = Utc::now();
                            Json(json!({
                                "message_id": 9101,
                                "id": 9101,
                                "pts": 20,
                                "pts_count": 1,
                                "duplicate": false,
                                "date_ms": now.timestamp_millis(),
                                "date": now.to_rfc3339(),
                                "sender_id": 202,
                                "chat_id": chat_id,
                                "text": text,
                                "client_nonce": nonce,
                                "status": "sent"
                            }))
                        }
                    }),
                );

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let local_addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            local_addr
        }
    };

    let addr_a = server_rt.block_on(spawn_mock_server("server_a", Arc::clone(&sent_log)));
    let addr_b = server_rt.block_on(spawn_mock_server("server_b", Arc::clone(&sent_log)));

    let cache_db_path = dir.path().join("client_cache.db");
    let query_cache_counts = |db_path: &std::path::Path| -> (i64, i64, i64) {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let outbox_cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM outbox_messages", [], |r| r.get(0))
            .unwrap();
        let msg_cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        let e2ee_cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM e2ee_identity", [], |r| r.get(0))
            .unwrap();
        (outbox_cnt, msg_cnt, e2ee_cnt)
    };

    let stage_account_a = |host: *mut c_void, chat_id: i64, nonce: u64, secret: &str| -> (String, String) {
        unsafe {
            let login = dispatch(
                host,
                &json!({"method": "auth.login", "params": {"username": "alice_b502", "password": "pw"}}),
            );
            assert_eq!(login["ok"], true, "Alice login failed: {login}");
            assert_eq!(
                dispatch(host, &json!({"method": "outbox.setNetworkState", "params": {"online": false}}))["ok"],
                true
            );
            let enq = dispatch(
                host,
                &json!({
                    "method": "outbox.enqueue",
                    "params": {"chat_id": chat_id, "text": secret, "client_nonce": nonce}
                }),
            );
            assert_eq!(enq["ok"], true, "outbox.enqueue failed: {enq}");

            let id = dispatch(host, &json!({"method": "e2ee.identity"}));
            assert_eq!(id["ok"], true, "e2ee.identity failed: {id}");
            let x_pub = id["result"]["x25519_pub_b64"].as_str().unwrap().to_string();
            let e_pub = id["result"]["ed25519_pub_b64"].as_str().unwrap().to_string();

            assert!(
                dispatch(host, &json!({"method": "outbox.pendingCount"}))["result"]["pending_count"]
                    .as_u64()
                    .unwrap()
                    >= 1,
                "Precondition: Account A must have >= 1 pending outbox message"
            );
            let local_list = dispatch(
                host,
                &json!({"method": "messages.localList", "params": {"chat_id": chat_id}}),
            );
            assert!(
                !local_list["result"].as_array().unwrap().is_empty(),
                "Precondition: Account A messages.localList must be non-empty: {local_list}"
            );
            let (ob_c, msg_c, e2ee_c) = query_cache_counts(&cache_db_path);
            assert!(ob_c >= 1 && msg_c >= 1 && e2ee_c == 1);
            (x_pub, e_pub)
        }
    };

    let assert_wiped = |host: *mut c_void,
                        chat_id: i64,
                        prev_x: &str,
                        prev_e: &str,
                        label: &str| {
        unsafe {
            assert_eq!(
                dispatch(host, &json!({"method": "outbox.pendingCount"}))["result"]["pending_count"],
                0,
                "{label}: outbox.pendingCount MUST be 0 after wipe (B5-02)"
            );
            let local_after = dispatch(
                host,
                &json!({"method": "messages.localList", "params": {"chat_id": chat_id}}),
            );
            assert!(
                local_after["result"].as_array().unwrap().is_empty(),
                "{label}: messages.localList MUST be empty after wipe (B5-02): {local_after}"
            );
            let (ob_c, msg_c, e2ee_c) = query_cache_counts(&cache_db_path);
            assert_eq!(
                (ob_c, msg_c, e2ee_c),
                (0, 0, 0),
                "{label}: client_cache.db outbox/messages/e2ee_identity MUST be empty (B5-02)"
            );
            let fresh_id = dispatch(host, &json!({"method": "e2ee.identity"}));
            assert_eq!(fresh_id["ok"], true);
            assert_ne!(
                fresh_id["result"]["x25519_pub_b64"].as_str().unwrap(),
                prev_x,
                "{label}: E2EE X25519 identity key MUST be regenerated after wipe (B5-02)"
            );
            assert_ne!(
                fresh_id["result"]["ed25519_pub_b64"].as_str().unwrap(),
                prev_e,
                "{label}: E2EE Ed25519 identity key MUST be regenerated after wipe (B5-02)"
            );
        }
    };

    let cfg_a = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr_a}"),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_a.as_ptr());
        assert!(!host.is_null());

        // 1. session.setToken("TOK_B") wipes outbox, messages, and E2EE identity; re-enabling
        //    network + connectWs transmits ZERO Account A messages with Bearer TOK_B!
        let (x1, e1) = stage_account_a(host, 8101, 81_001, "SECRET_A_SET_TOKEN");
        assert_eq!(
            dispatch(host, &json!({"method": "session.setToken", "params": {"token": "TOK_B"}}))["ok"],
            true
        );
        assert_wiped(host, 8101, &x1, &e1, "session.setToken(TOK_B)");
        assert_eq!(
            dispatch(host, &json!({"method": "outbox.setNetworkState", "params": {"online": true}}))["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "session.connectWs"}))["ok"], true);
        assert_eq!(dispatch(host, &json!({"method": "outbox.retryNow"}))["ok"], true);
        thread::sleep(Duration::from_millis(120));
        // Non-tautology: verify Account B CAN genuinely deliver its own message with Bearer TOK_B
        let send_b = dispatch(
            host,
            &json!({"method": "messages.send", "params": {"chat_id": 8101, "text": "GENUINE_B_MSG"}}),
        );
        assert_eq!(send_b["ok"], true, "Genuine Account B send failed: {send_b}");
        {
            let log = sent_log.lock().unwrap();
            assert!(
                log.iter()
                    .any(|(srv, auth, txt)| srv == "server_a" && auth == "Bearer TOK_B" && txt == "GENUINE_B_MSG"),
                "Non-tautology proof: genuine Account B message must arrive with Bearer TOK_B: {log:?}"
            );
            assert!(
                !log.iter().any(|(_, _, txt)| txt == "SECRET_A_SET_TOKEN"),
                "Account A queued outbox message MUST NEVER transmit under Bearer TOK_B (B5-02): {log:?}"
            );
        }

        // 2. Token-only TOK_A -> auth.login(Bob) ALSO wipes token-only outbox/messages/E2EE
        //    even though LocalStore owner_scope was None during the token-only session!
        assert_eq!(
            dispatch(host, &json!({"method": "session.setToken", "params": {"token": "TOK_A"}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(host, &json!({"method": "outbox.setNetworkState", "params": {"online": false}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host,
                &json!({
                    "method": "outbox.enqueue",
                    "params": {"chat_id": 8102, "text": "SECRET_TOK_ONLY_A", "client_nonce": 81_002}
                })
            )["ok"],
            true
        );
        let id_tok = dispatch(host, &json!({"method": "e2ee.identity"}));
        let x_tok = id_tok["result"]["x25519_pub_b64"].as_str().unwrap().to_string();
        let e_tok = id_tok["result"]["ed25519_pub_b64"].as_str().unwrap().to_string();
        assert!(
            dispatch(host, &json!({"method": "outbox.pendingCount"}))["result"]["pending_count"]
                .as_u64()
                .unwrap()
                >= 1,
            "Precondition (CI NB6-01): token-only Account A must have >= 1 pending outbox message"
        );
        let local_tok = dispatch(
            host,
            &json!({"method": "messages.localList", "params": {"chat_id": 8102}}),
        );
        assert!(
            !local_tok["result"].as_array().unwrap().is_empty(),
            "Precondition (CI NB6-01): token-only Account A messages.localList must be non-empty: {local_tok}"
        );
        let (ob_tok, msg_tok, e2ee_tok) = query_cache_counts(&cache_db_path);
        assert!(
            ob_tok >= 1 && msg_tok >= 1 && e2ee_tok == 1,
            "Precondition (CI NB6-01): token-only Account A client_cache.db must be populated: ({ob_tok}, {msg_tok}, {e2ee_tok})"
        );
        assert_eq!(
            dispatch(host, &json!({"method": "auth.login", "params": {"username": "bob_b502", "password": "pw"}}))["ok"],
            true
        );
        assert_wiped(host, 8102, &x_tok, &e_tok, "token-only -> auth.login(Bob)");
        assert_eq!(
            dispatch(host, &json!({"method": "outbox.setNetworkState", "params": {"online": true}}))["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "session.connectWs"}))["ok"], true);
        thread::sleep(Duration::from_millis(100));
        assert!(
            !sent_log
                .lock()
                .unwrap()
                .iter()
                .any(|(_, _, txt)| txt == "SECRET_TOK_ONLY_A"),
            "Token-only Account A outbox message MUST NEVER transmit under Bob's Bearer TOK_B (B5-02)"
        );

        // 3. auth.logout wipes outbox, messages, and E2EE identity
        let (x3, e3) = stage_account_a(host, 8103, 81_003, "SECRET_A_LOGOUT");
        assert_eq!(dispatch(host, &json!({"method": "auth.logout"}))["ok"], true);
        assert_wiped(host, 8103, &x3, &e3, "auth.logout");

        // 4. session.clearSession wipes outbox, messages, and E2EE identity
        let (x4, e4) = stage_account_a(host, 8104, 81_004, "SECRET_A_CLEAR");
        assert_eq!(dispatch(host, &json!({"method": "session.clearSession"}))["ok"], true);
        assert_wiped(host, 8104, &x4, &e4, "session.clearSession");

        // 4b. session.setToken(null) wipes outbox, messages, and E2EE identity (CI NB6-01)
        let (x4b, e4b) = stage_account_a(host, 81041, 81_0041, "SECRET_A_SET_TOKEN_NULL");
        let tok_null = dispatch(
            host,
            &json!({"method": "session.setToken", "params": {"token": Value::Null}}),
        );
        assert_eq!(tok_null["ok"], true, "session.setToken(null) failed: {tok_null}");
        assert_eq!(tok_null["result"]["has_token"], false);
        assert_wiped(host, 81041, &x4b, &e4b, "session.setToken(null)");

        // 5. session.setServerUrl (Server A -> Server B) wipes both old and new LocalStores;
        //    authenticating on Server B with TOK_SERVER_B transmits ZERO Server A messages!
        let (x5, e5) = stage_account_a(host, 8105, 81_005, "SECRET_SERVER_A");
        let switched = dispatch(
            host,
            &json!({"method": "session.setServerUrl", "params": {"server_url": format!("http://{addr_b}")}}),
        );
        assert_eq!(switched["ok"], true, "setServerUrl Server A -> Server B failed: {switched}");
        assert_wiped(host, 8105, &x5, &e5, "session.setServerUrl(Server A -> Server B)");
        assert_eq!(
            dispatch(host, &json!({"method": "session.setToken", "params": {"token": "TOK_SERVER_B"}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(host, &json!({"method": "outbox.setNetworkState", "params": {"online": true}}))["ok"],
            true
        );
        assert_eq!(dispatch(host, &json!({"method": "session.connectWs"}))["ok"], true);
        assert_eq!(dispatch(host, &json!({"method": "outbox.retryNow"}))["ok"], true);
        thread::sleep(Duration::from_millis(120));
        let send_srv_b = dispatch(
            host,
            &json!({"method": "messages.send", "params": {"chat_id": 8105, "text": "GENUINE_SERVER_B"}}),
        );
        assert_eq!(send_srv_b["ok"], true);
        {
            let log = sent_log.lock().unwrap();
            assert!(
                log.iter().any(|(srv, auth, txt)| {
                    srv == "server_b" && auth == "Bearer TOK_SERVER_B" && txt == "GENUINE_SERVER_B"
                }),
                "Non-tautology proof: genuine Server B send must reach server_b with TOK_SERVER_B: {log:?}"
            );
            assert!(
                !log.iter().any(|(srv, _, txt)| srv == "server_b" && txt == "SECRET_SERVER_A"),
                "Server A queued outbox message MUST NEVER transmit to Server B (B5-02): {log:?}"
            );
        }

        // 6. Same-account shutdown() + cold restart MUST still preserve offline outbox, cached
        //    messages, and E2EE identity!
        let (x6, e6) = stage_account_a(host, 8106, 81_006, "KEEP_ON_SAME_ACCOUNT_RESTART");
        fabushi_host_destroy(host);

        let cfg_b = CString::new(
            json!({
                "app_data_dir": dir.path().to_string_lossy(),
                "server_url": format!("http://{addr_b}"),
                "test_mode": true
            })
            .to_string(),
        )
        .unwrap();
        let host2 = fabushi_host_create(cfg_b.as_ptr());
        assert!(!host2.is_null());
        assert_eq!(
            dispatch(host2, &json!({"method": "outbox.pendingCount"}))["result"]["pending_count"],
            1,
            "Same-account shutdown() + restart MUST preserve offline outbox persistence"
        );
        let kept_msgs = dispatch(
            host2,
            &json!({"method": "messages.localList", "params": {"chat_id": 8106}}),
        );
        assert!(
            kept_msgs["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m.to_string().contains("KEEP_ON_SAME_ACCOUNT_RESTART")),
            "Same-account restart MUST preserve cached messages: {kept_msgs}"
        );
        let kept_id = dispatch(host2, &json!({"method": "e2ee.identity"}));
        assert_eq!(kept_id["result"]["x25519_pub_b64"].as_str().unwrap(), x6);
        assert_eq!(kept_id["result"]["ed25519_pub_b64"].as_str().unwrap(), e6);

        fabushi_host_destroy(host2);
    }
}

#[test]
fn test_b6_01_cold_restart_token_only_unauth_staging_and_server_switch_wipe_local_store_and_search()
{
    let dir = tempdir().unwrap();
    let sent_log: Arc<std::sync::Mutex<Vec<(String, String, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    async fn quiet_ws_b601(
        ws: WebSocketUpgrade,
        Query(_q): Query<std::collections::HashMap<String, String>>,
    ) -> impl IntoResponse {
        ws.on_upgrade(|mut socket: WebSocket| async move {
            while let Some(Ok(_)) = socket.recv().await {}
        })
    }

    let spawn_server = |label: &'static str,
                        sent_log_ref: Arc<std::sync::Mutex<Vec<(String, String, String)>>>| {
        async move {
            let log_send = Arc::clone(&sent_log_ref);
            let app = Router::new()
                .route("/health", get(|| async { Json(json!({"status": "ok"})) }))
                .route(
                    "/api/v1/auth/login",
                    post(|Json(body): Json<Value>| async move {
                        if body["username"].as_str() == Some("bob_b601") {
                            Json(json!({
                                "user_id": 202,
                                "username": "bob_b601",
                                "session_id": "sess-bob-b601",
                                "token": "TOK_B",
                                "first_name": "Bob"
                            }))
                        } else {
                            Json(json!({
                                "user_id": 101,
                                "username": "alice_b601",
                                "session_id": "sess-alice-b601",
                                "token": "TOK_A",
                                "first_name": "Alice"
                            }))
                        }
                    }),
                )
                .route("/api/v1/auth/logout", post(|| async { StatusCode::OK }))
                .route("/api/v1/ws", get(quiet_ws_b601))
                .route(
                    "/api/v1/messages/send",
                    post(move |headers: HeaderMap, Json(body): Json<Value>| {
                        let log = Arc::clone(&log_send);
                        async move {
                            let auth_val = headers
                                .get(header::AUTHORIZATION)
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                            let chat_id = body["chat_id"].as_i64().unwrap_or(0);
                            let text = body["text"].as_str().unwrap_or("").to_string();
                            let nonce = body["client_nonce"].as_u64();
                            log.lock().unwrap().push((
                                label.to_string(),
                                auth_val,
                                text.clone(),
                            ));
                            let now = Utc::now();
                            Json(json!({
                                "message_id": 9301,
                                "id": 9301,
                                "pts": 25,
                                "pts_count": 1,
                                "duplicate": false,
                                "date_ms": now.timestamp_millis(),
                                "date": now.to_rfc3339(),
                                "sender_id": 202,
                                "chat_id": chat_id,
                                "text": text,
                                "client_nonce": nonce,
                                "status": "sent"
                            }))
                        }
                    }),
                );

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let local_addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            local_addr
        }
    };

    let addr_a = server_rt.block_on(spawn_server("server_a", Arc::clone(&sent_log)));
    let addr_b = server_rt.block_on(spawn_server("server_b", Arc::clone(&sent_log)));

    let cache_db_path = dir.path().join("client_cache.db");
    let host_db_path = dir.path().join("host_state.db");

    let query_cache_counts = |db_path: &std::path::Path| -> (i64, i64, i64) {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let ob: i64 = conn
            .query_row("SELECT COUNT(*) FROM outbox_messages", [], |r| r.get(0))
            .unwrap();
        let msg: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        let e2ee: i64 = conn
            .query_row("SELECT COUNT(*) FROM e2ee_identity", [], |r| r.get(0))
            .unwrap();
        (ob, msg, e2ee)
    };

    let query_host_counts = |db_path: &std::path::Path| -> (i64, i64, i64) {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let idx: i64 = conn
            .query_row("SELECT COUNT(*) FROM host_messages_index", [], |r| r.get(0))
            .unwrap();
        let dedup: i64 = conn
            .query_row("SELECT COUNT(*) FROM nonce_dedup", [], |r| r.get(0))
            .unwrap();
        let pts: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_pts", [], |r| r.get(0))
            .unwrap();
        (idx, dedup, pts)
    };

    let make_cfg = |srv_addr: std::net::SocketAddr| {
        CString::new(
            json!({
                "app_data_dir": dir.path().to_string_lossy(),
                "server_url": format!("http://{srv_addr}"),
                "test_mode": true
            })
            .to_string(),
        )
        .unwrap()
    };

    let cfg_a = make_cfg(addr_a);
    let cfg_b = make_cfg(addr_b);

    unsafe {
        // =========================================================================
        // 1A. Trace 6A (Clean Token-Only Shutdown): shutdown() MUST wipe client_cache.db
        //     and account-scoped host tables immediately when active session is token-only.
        // =========================================================================
        let host1 = fabushi_host_create(cfg_a.as_ptr());
        assert!(!host1.is_null());
        assert_eq!(
            dispatch(host1, &json!({"method": "session.setToken", "params": {"token": "TOK_A"}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(host1, &json!({"method": "outbox.setNetworkState", "params": {"online": false}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host1,
                &json!({
                    "method": "outbox.enqueue",
                    "params": {"chat_id": 9101, "text": "SECRET_TOK_A_SHUTDOWN", "client_nonce": 91_001}
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host1,
                &json!({
                    "method": "messages.send",
                    "params": {
                        "chat_id": 9101,
                        "text": "SECRET_SEARCH_TOK_A_SHUTDOWN",
                        "client_nonce": 91_002,
                        "local_only": true
                    }
                })
            )["ok"],
            true
        );
        assert_eq!(dispatch(host1, &json!({"method": "e2ee.identity"}))["ok"], true);
        assert_eq!(
            dispatch(
                host1,
                &json!({
                    "method": "media.casPut",
                    "params": {"file_id": "tok_a_file", "data_b64": "c2VjcmV0LW1lZGlhLWJsb2I="}
                })
            )["ok"],
            true
        );
        let (ob1, msg1, e2ee1) = query_cache_counts(&cache_db_path);
        assert!(ob1 >= 1 && msg1 >= 1 && e2ee1 == 1);
        assert!(
            dispatch(
                host1,
                &json!({"method": "search.localMessages", "params": {"query": "SECRET_SEARCH_TOK_A_SHUTDOWN"}})
            )["result"]["total"]
                .as_u64()
                .unwrap()
                >= 1
        );
        fabushi_host_destroy(host1);

        // Immediately after token-only shutdown, both client_cache.db and host_state.db account tables MUST be wiped!
        assert_eq!(
            query_cache_counts(&cache_db_path),
            (0, 0, 0),
            "Token-only shutdown() MUST wipe client_cache.db (B6-01)"
        );
        assert_eq!(
            query_host_counts(&host_db_path),
            (0, 0, 0),
            "Token-only shutdown() MUST wipe account-scoped host tables (NB6-01)"
        );

        // =========================================================================
        // 1B. Trace 6A (Simulated iOS Jetsam / SIGKILL during token-only session):
        //     client_cache.db retains Account A's pending outbox/messages/E2EE with
        //     cache_meta owner_scope absent and host_meta local_store_owner_scope absent.
        //     Cold-restarted host2 logging in Bob MUST wipe everything BEFORE OutboxWorker starts!
        // =========================================================================
        let host_jet = fabushi_host_create(cfg_a.as_ptr());
        assert!(!host_jet.is_null());
        assert_eq!(
            dispatch(host_jet, &json!({"method": "auth.login", "params": {"username": "alice_b601", "password": "pw"}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(host_jet, &json!({"method": "outbox.setNetworkState", "params": {"online": false}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host_jet,
                &json!({
                    "method": "outbox.enqueue",
                    "params": {"chat_id": 9102, "text": "SECRET_JETSAM_TOK_A", "client_nonce": 91_003}
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host_jet,
                &json!({
                    "method": "messages.send",
                    "params": {
                        "chat_id": 9102,
                        "text": "SECRET_JETSAM_SEARCH_A",
                        "client_nonce": 91_004,
                        "local_only": true
                    }
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(host_jet, &json!({"method": "sync.setPts", "params": {"chat_id": 9102, "pts": 77}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host_jet,
                &json!({
                    "method": "multi_agent.workspaceSet",
                    "params": {"chat_id": 9102, "key": "plan", "value": {"secret": "alice_plan"}}
                })
            )["ok"],
            true
        );
        let id_jet = dispatch(host_jet, &json!({"method": "e2ee.identity"}));
        let x_jet = id_jet["result"]["x25519_pub_b64"].as_str().unwrap().to_string();
        let e_jet = id_jet["result"]["ed25519_pub_b64"].as_str().unwrap().to_string();
        fabushi_host_destroy(host_jet);

        // Reproduce exact on-disk state after iOS Jetsam kills a token-only session:
        // cache_meta owner_scope is absent AND host_meta local_store_owner_scope is absent,
        // while outbox_messages / messages / e2ee_identity / search remain populated on disk!
        {
            let c_conn = rusqlite::Connection::open(&cache_db_path).unwrap();
            c_conn.execute("DELETE FROM cache_meta", []).unwrap();
            let h_conn = rusqlite::Connection::open(&host_db_path).unwrap();
            h_conn
                .execute(
                    "DELETE FROM host_meta WHERE key IN ('local_store_owner_scope', 'active_account_id')",
                    [],
                )
                .unwrap();
        }
        let (ob_pre, msg_pre, e2ee_pre) = query_cache_counts(&cache_db_path);
        assert!(
            ob_pre >= 1 && msg_pre >= 1 && e2ee_pre == 1,
            "Precondition: simulated Jetsam client_cache.db must hold pending Account A rows"
        );

        let host2 = fabushi_host_create(cfg_a.as_ptr());
        assert!(!host2.is_null());
        // Login Bob directly (WITHOUT connectWs — set_auth_session immediately starts OutboxWorker online!)
        let bob_login = dispatch(
            host2,
            &json!({"method": "auth.login", "params": {"username": "bob_b601", "password": "pw"}}),
        );
        assert_eq!(bob_login["ok"], true, "Bob login failed: {bob_login}");
        thread::sleep(Duration::from_millis(160));

        {
            let log = sent_log.lock().unwrap();
            assert!(
                !log.iter().any(|(_, _, txt)| txt == "SECRET_JETSAM_TOK_A"),
                "Account A Jetsam outbox message MUST NEVER transmit under Bob's Bearer TOK_B (B6-01): {log:?}"
            );
        }
        assert_eq!(
            dispatch(host2, &json!({"method": "outbox.pendingCount"}))["result"]["pending_count"],
            0,
            "Jetsam Account A outbox MUST be wiped on Bob login (B6-01)"
        );
        assert!(
            dispatch(host2, &json!({"method": "messages.localList", "params": {"chat_id": 9102}}))["result"]
                .as_array()
                .unwrap()
                .is_empty(),
            "Jetsam Account A cached messages MUST be wiped on Bob login (B6-01)"
        );
        assert_eq!(
            query_cache_counts(&cache_db_path),
            (0, 0, 0),
            "Jetsam client_cache.db tables MUST be empty before Bob generates fresh E2EE identity"
        );
        let id_bob = dispatch(host2, &json!({"method": "e2ee.identity"}));
        assert_ne!(id_bob["result"]["x25519_pub_b64"].as_str().unwrap(), x_jet);
        assert_ne!(id_bob["result"]["ed25519_pub_b64"].as_str().unwrap(), e_jet);
        assert_eq!(
            dispatch(
                host2,
                &json!({"method": "search.localMessages", "params": {"query": "SECRET_JETSAM_SEARCH_A"}})
            )["result"]["total"],
            0,
            "Account A local search (Tantivy + FTS5) MUST be wiped on Bob login (NB6-01)"
        );
        assert_eq!(
            dispatch(host2, &json!({"method": "sync.lastPts", "params": {"chat_id": 9102}}))["result"]["pts"],
            0,
            "Account A chat PTS MUST be cleared on Bob login (NB6-01)"
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({"method": "multi_agent.workspaceGet", "params": {"chat_id": 9102, "key": "plan"}})
            )["result"],
            Value::Null,
            "Account A multi_agent_workspace MUST be cleared on Bob login (NB6-01)"
        );

        // Anti-tautology: verify Bob's own message delivers cleanly with Bearer TOK_B,
        // AND same-user re-login preserves Bob's local search index!
        let bob_send = dispatch(
            host2,
            &json!({"method": "messages.send", "params": {"chat_id": 9102, "text": "GENUINE_BOB_AFTER_JETSAM"}}),
        );
        assert_eq!(bob_send["ok"], true, "Genuine Bob send failed: {bob_send}");
        assert!(
            sent_log.lock().unwrap().iter().any(|(srv, auth, txt)| {
                srv == "server_a" && auth == "Bearer TOK_B" && txt == "GENUINE_BOB_AFTER_JETSAM"
            }),
            "Anti-tautology: Bob's own message must arrive carrying Bearer TOK_B"
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({
                    "method": "search.indexMessage",
                    "params": {"message_id": 9901, "chat_id": 9102, "sender_id": 202, "body": "BOB_KEEP_SAME_USER_RELOGIN"}
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(host2, &json!({"method": "auth.login", "params": {"username": "bob_b601", "password": "pw"}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({"method": "search.localMessages", "params": {"query": "BOB_KEEP_SAME_USER_RELOGIN"}})
            )["result"]["total"],
            1,
            "Same-user re-login MUST preserve Bob's local search cache"
        );

        // =========================================================================
        // 1C. Trace 6B (Single-Process Post-Logout Unauthenticated Staging -> auth.login(Bob))
        // =========================================================================
        assert_eq!(dispatch(host2, &json!({"method": "auth.logout"}))["ok"], true);
        assert_eq!(
            dispatch(host2, &json!({"method": "outbox.setNetworkState", "params": {"online": false}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({
                    "method": "outbox.enqueue",
                    "params": {"chat_id": 9103, "text": "SECRET_UNAUTH_STAGED", "client_nonce": 91_005}
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({
                    "method": "messages.send",
                    "params": {
                        "chat_id": 9103,
                        "text": "SECRET_UNAUTH_SEARCH",
                        "client_nonce": 91_006,
                        "local_only": true
                    }
                })
            )["ok"],
            true
        );
        let id_unauth = dispatch(host2, &json!({"method": "e2ee.identity"}));
        let x_unauth = id_unauth["result"]["x25519_pub_b64"].as_str().unwrap().to_string();
        let e_unauth = id_unauth["result"]["ed25519_pub_b64"].as_str().unwrap().to_string();
        let (ob_u, msg_u, e2ee_u) = query_cache_counts(&cache_db_path);
        assert!(ob_u >= 1 && msg_u >= 1 && e2ee_u == 1);

        assert_eq!(
            dispatch(host2, &json!({"method": "auth.login", "params": {"username": "bob_b601", "password": "pw"}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(host2, &json!({"method": "outbox.setNetworkState", "params": {"online": true}}))["ok"],
            true
        );
        assert_eq!(dispatch(host2, &json!({"method": "outbox.retryNow"}))["ok"], true);
        thread::sleep(Duration::from_millis(120));

        assert!(
            !sent_log.lock().unwrap().iter().any(|(_, _, txt)| txt == "SECRET_UNAUTH_STAGED"),
            "Unauthenticated staged outbox message MUST NEVER transmit under Bob's Bearer TOK_B (B6-01)"
        );
        assert_eq!(
            dispatch(host2, &json!({"method": "outbox.pendingCount"}))["result"]["pending_count"],
            0
        );
        assert!(
            dispatch(host2, &json!({"method": "messages.localList", "params": {"chat_id": 9103}}))["result"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let id_after_unauth = dispatch(host2, &json!({"method": "e2ee.identity"}));
        assert_ne!(id_after_unauth["result"]["x25519_pub_b64"].as_str().unwrap(), x_unauth);
        assert_ne!(id_after_unauth["result"]["ed25519_pub_b64"].as_str().unwrap(), e_unauth);
        assert_eq!(
            dispatch(
                host2,
                &json!({"method": "search.localMessages", "params": {"query": "SECRET_UNAUTH_SEARCH"}})
            )["result"]["total"],
            0
        );

        // =========================================================================
        // 1D. Synthetic user_id == 0 Guard on session.setAuthSession
        // =========================================================================
        let zero_uid = dispatch(
            host2,
            &json!({
                "method": "session.setAuthSession",
                "params": {
                    "user_id": 0,
                    "username": "synthetic_zero",
                    "session_id": "sess-zero",
                    "token": "TOK_ZERO"
                }
            }),
        );
        assert_eq!(zero_uid["ok"], false, "Synthetic user_id == 0 must fail closed: {zero_uid}");
        assert_eq!(zero_uid["error"]["code"], "invalid_arguments");

        // =========================================================================
        // 1E. Trace 6C (Cold Restart with Different config.server_url):
        //     Launching host3 against Server B on same app_data_dir MUST wipe Server A
        //     LocalStore, search, media CAS, and active_account_id BEFORE any login,
        //     while preserving device settings!
        // =========================================================================
        assert_eq!(
            dispatch(host2, &json!({"method": "outbox.setNetworkState", "params": {"online": false}}))["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({
                    "method": "outbox.enqueue",
                    "params": {"chat_id": 9104, "text": "SECRET_SERVER_A_COLD", "client_nonce": 91_007}
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({
                    "method": "messages.send",
                    "params": {
                        "chat_id": 9104,
                        "text": "SECRET_SERVER_A_SEARCH",
                        "client_nonce": 91_008,
                        "local_only": true
                    }
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({
                    "method": "media.casPut",
                    "params": {"file_id": "file_sa", "data_b64": "c2VjcmV0LW1lZGlhLWJsb2I="}
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host2,
                &json!({"method": "settings.localSet", "params": {"key": "theme", "value": "Solarized"}})
            )["ok"],
            true
        );
        let id_sa = dispatch(host2, &json!({"method": "e2ee.identity"}));
        let x_sa = id_sa["result"]["x25519_pub_b64"].as_str().unwrap().to_string();
        let e_sa = id_sa["result"]["ed25519_pub_b64"].as_str().unwrap().to_string();
        fabushi_host_destroy(host2);

        let host3 = fabushi_host_create(cfg_b.as_ptr());
        assert!(!host3.is_null());
        // BEFORE ANY LOGIN on Server B:
        assert_eq!(
            dispatch(host3, &json!({"method": "outbox.pendingCount"}))["result"]["pending_count"],
            0,
            "Cold-restart server switch MUST wipe Server A outbox (B6-01)"
        );
        assert!(
            dispatch(host3, &json!({"method": "messages.localList", "params": {"chat_id": 9104}}))["result"]
                .as_array()
                .unwrap()
                .is_empty(),
            "Cold-restart server switch MUST wipe Server A cached messages (B6-01)"
        );
        assert_eq!(
            dispatch(
                host3,
                &json!({"method": "search.localMessages", "params": {"query": "SECRET_SERVER_A_SEARCH"}})
            )["result"]["total"],
            0,
            "Cold-restart server switch MUST wipe Server A local search (NB6-01)"
        );
        let cas_after = dispatch(
            host3,
            &json!({"method": "media.casGet", "params": {"file_id": "file_sa"}}),
        );
        assert_eq!(cas_after["ok"], false);
        assert_eq!(cas_after["error"]["code"], "not_found");
        assert_eq!(
            dispatch(host3, &json!({"method": "session.listAccounts"}))["result"]["active_account_id"],
            Value::Null,
            "Cold-restart server switch MUST clear active_account_id (B6-01)"
        );
        let id_sb = dispatch(host3, &json!({"method": "e2ee.identity"}));
        assert_ne!(id_sb["result"]["x25519_pub_b64"].as_str().unwrap(), x_sa);
        assert_ne!(id_sb["result"]["ed25519_pub_b64"].as_str().unwrap(), e_sa);
        // Device settings MUST be preserved across server switch!
        assert_eq!(
            dispatch(host3, &json!({"method": "settings.localGet", "params": {"key": "theme"}}))["result"]["value"],
            "Solarized"
        );

        fabushi_host_destroy(host3);
    }
}

#[test]
fn test_b6_02_outbox_worker_fenced_against_account_switch_and_shutdown_wal_writes() {
    use mahayana_app_host::{FabushiAppHost, HostConfig};

    let dir = tempdir().unwrap();
    let wire_log: Arc<std::sync::Mutex<Vec<(String, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let m1_entered = Arc::new(AtomicBool::new(false));
    let m1_release = Arc::new(AtomicBool::new(false));
    let next_msg_id = Arc::new(AtomicUsize::new(9500));

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    async fn quiet_ws_b602(
        ws: WebSocketUpgrade,
        Query(_q): Query<std::collections::HashMap<String, String>>,
    ) -> impl IntoResponse {
        ws.on_upgrade(|mut socket: WebSocket| async move {
            while let Some(Ok(_)) = socket.recv().await {}
        })
    }

    let addr = server_rt.block_on({
        let log_send = Arc::clone(&wire_log);
        let entered = Arc::clone(&m1_entered);
        let release = Arc::clone(&m1_release);
        let msg_seq = Arc::clone(&next_msg_id);
        async move {
            let app = Router::new()
                .route("/health", get(|| async { Json(json!({"status": "ok"})) }))
                .route(
                    "/api/v1/auth/login",
                    post(|Json(body): Json<Value>| async move {
                        if body["username"].as_str() == Some("bob_b602") {
                            Json(json!({
                                "user_id": 202,
                                "username": "bob_b602",
                                "session_id": "sess-bob-b602",
                                "token": "TOK_B",
                                "first_name": "Bob"
                            }))
                        } else {
                            Json(json!({
                                "user_id": 101,
                                "username": "alice_b602",
                                "session_id": "sess-alice-b602",
                                "token": "TOK_A",
                                "first_name": "Alice"
                            }))
                        }
                    }),
                )
                .route("/api/v1/auth/logout", post(|| async { StatusCode::OK }))
                .route("/api/v1/ws", get(quiet_ws_b602))
                .route(
                    "/api/v1/messages/send",
                    post(move |headers: HeaderMap, Json(body): Json<Value>| {
                        let log = Arc::clone(&log_send);
                        let ent = Arc::clone(&entered);
                        let rel = Arc::clone(&release);
                        let seq = Arc::clone(&msg_seq);
                        async move {
                            let auth_val = headers
                                .get(header::AUTHORIZATION)
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                            let chat_id = body["chat_id"].as_i64().unwrap_or(0);
                            let text = body["text"].as_str().unwrap_or("").to_string();
                            let nonce = body["client_nonce"].as_u64();
                            log.lock().unwrap().push((auth_val, text.clone()));

                            if text.starts_with("HOLD_M1_") {
                                ent.store(true, Ordering::SeqCst);
                                let deadline = Instant::now() + Duration::from_secs(5);
                                while !rel.load(Ordering::SeqCst) && Instant::now() < deadline {
                                    tokio::time::sleep(Duration::from_millis(5)).await;
                                }
                            }

                            let mid = seq.fetch_add(1, Ordering::SeqCst) as i64;
                            let now = Utc::now();
                            Json(json!({
                                "message_id": mid,
                                "id": mid,
                                "pts": mid,
                                "pts_count": 1,
                                "duplicate": false,
                                "date_ms": now.timestamp_millis(),
                                "date": now.to_rfc3339(),
                                "sender_id": 101,
                                "chat_id": chat_id,
                                "text": text,
                                "client_nonce": nonce,
                                "status": "sent"
                            }))
                        }
                    }),
                );

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let local_addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            local_addr
        }
    });

    // =========================================================================
    // 2A. Multi-Message OutboxWorker Background Race Across ALL 3 Account Transitions
    //     (auth.login(Bob), session.switchAccount("202"), session.setToken("TOK_B"))
    // =========================================================================
    let cfg = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}"),
            "storage_passphrase": "b602-test-passphrase",
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg.as_ptr());
        assert!(!host.is_null());

        // Pre-seed Bob's encrypted account record so session.switchAccount("202") succeeds cleanly.
        assert_eq!(
            dispatch(host, &json!({"method": "auth.login", "params": {"username": "bob_b602", "password": "pw"}}))["ok"],
            true
        );

        let transitions = [
            ("auth_login", 0u8),
            ("switch_account", 1u8),
            ("set_token", 2u8),
        ];

        for (label, mode) in transitions {
            m1_entered.store(false, Ordering::SeqCst);
            m1_release.store(false, Ordering::SeqCst);

            let base_nonce = 920_000u64 + (mode as u64) * 100;
            let chat_id = 9200i64 + (mode as i64);

            assert_eq!(
                dispatch(host, &json!({"method": "auth.login", "params": {"username": "alice_b602", "password": "pw"}}))["ok"],
                true
            );
            assert_eq!(
                dispatch(host, &json!({"method": "outbox.setNetworkState", "params": {"online": false}}))["ok"],
                true
            );
            assert_eq!(
                dispatch(
                    host,
                    &json!({
                        "method": "outbox.enqueue",
                        "params": {
                            "chat_id": chat_id,
                            "text": format!("HOLD_M1_{label}"),
                            "client_nonce": base_nonce + 1
                        }
                    })
                )["ok"],
                true
            );
            assert_eq!(
                dispatch(
                    host,
                    &json!({
                        "method": "outbox.enqueue",
                        "params": {
                            "chat_id": chat_id,
                            "text": format!("SECRET_M2_{label}"),
                            "client_nonce": base_nonce + 2
                        }
                    })
                )["ok"],
                true
            );

            // Go online + wake OutboxWorker (already started by auth.login -> set_auth_session)
            // so background Task 1 picks up M1 and parks at barrier with network_listener staying
            // continuously Online across the account transition!
            assert_eq!(
                dispatch(host, &json!({"method": "outbox.setNetworkState", "params": {"online": true}}))["ok"],
                true
            );
            assert_eq!(dispatch(host, &json!({"method": "outbox.retryNow"}))["ok"], true);

            let wait_start = Instant::now();
            while !m1_entered.load(Ordering::SeqCst) {
                assert!(
                    wait_start.elapsed() < Duration::from_secs(4),
                    "[{label}] Timed out waiting for background OutboxWorker M1 to enter barrier"
                );
                thread::sleep(Duration::from_millis(5));
            }

            // While Task 1 is mid-HTTP on M1 (with M2 still Pending behind it), execute account transition!
            let switch_res = match mode {
                0 => dispatch(
                    host,
                    &json!({"method": "auth.login", "params": {"username": "bob_b602", "password": "pw"}}),
                ),
                1 => dispatch(
                    host,
                    &json!({"method": "session.switchAccount", "params": {"account_id": "202"}}),
                ),
                _ => dispatch(
                    host,
                    &json!({"method": "session.setToken", "params": {"token": "TOK_B"}}),
                ),
            };
            assert_eq!(
                switch_res["ok"], true,
                "[{label}] Account transition failed: {switch_res}"
            );
            if mode == 2 {
                // Start Bob's OutboxWorker & wait for WS Connected BEFORE releasing M1 barrier
                // so network_listener is Online and Bob's worker is live during M1 completion!
                assert_eq!(dispatch(host, &json!({"method": "session.connectWs"}))["ok"], true);
                let ws_wait = Instant::now();
                while dispatch(host, &json!({"method": "session.wsStatus"}))["result"]["connected"]
                    != true
                {
                    assert!(
                        ws_wait.elapsed() < Duration::from_secs(4),
                        "[{label}] Timed out waiting for WS Connected before releasing M1"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
            }
            assert_eq!(
                dispatch(host, &json!({"method": "outbox.setNetworkState", "params": {"online": true}}))["ok"],
                true
            );

            // Release M1 HTTP response so any unfenced Task 1 would wake, loop to claim M2, and leak TOK_B!
            m1_release.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(250));

            // Anti-tautology: verify Bob's own background OutboxWorker genuinely delivers M_BOB with Bearer TOK_B!
            let bob_msg = format!("GENUINE_M_BOB_{label}");
            assert_eq!(
                dispatch(
                    host,
                    &json!({
                        "method": "outbox.enqueue",
                        "params": {
                            "chat_id": chat_id,
                            "text": bob_msg,
                            "client_nonce": base_nonce + 50
                        }
                    })
                )["ok"],
                true
            );
            assert_eq!(dispatch(host, &json!({"method": "outbox.retryNow"}))["ok"], true);

            let bob_wait = Instant::now();
            loop {
                let delivered = wire_log
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(auth, txt)| auth == "Bearer TOK_B" && txt == &bob_msg);
                if delivered {
                    break;
                }
                assert!(
                    bob_wait.elapsed() < Duration::from_secs(4),
                    "[{label}] Anti-tautology: Bob's own background outbox message failed to deliver with Bearer TOK_B: {:?}",
                    wire_log.lock().unwrap()
                );
                thread::sleep(Duration::from_millis(10));
            }

            // Strict zero-leak verification across ALL wire entries:
            let entries = wire_log.lock().unwrap().clone();
            assert!(
                !entries.iter().any(|(auth, txt)| {
                    auth == "Bearer TOK_B"
                        && (txt.starts_with("HOLD_M1_") || txt.starts_with("SECRET_M2_"))
                }),
                "[{label}] Background OutboxWorker leaked Account A message under Bearer TOK_B (B6-02): {entries:?}"
            );
            let m2_text = format!("SECRET_M2_{label}");
            assert!(
                !entries.iter().any(|(_, txt)| txt == &m2_text),
                "[{label}] Account A M2 MUST NEVER transmit after account transition (B6-02): {entries:?}"
            );
        }

        fabushi_host_destroy(host);
    }

    // =========================================================================
    // 2B. Post-shutdown() client_cache.db-wal Silence Proof (Keeping Arc<FabushiAppHost>
    //     and Tokio Runtime Alive Across shutdown() + Barrier Release!)
    // =========================================================================
    let dir2 = tempdir().unwrap();
    let host_arc = Arc::new(
        FabushiAppHost::from_config(HostConfig {
            app_data_dir: Some(dir2.path().to_string_lossy().into_owned()),
            server_url: Some(format!("http://{addr}")),
            test_mode: true,
            ..Default::default()
        })
        .expect("from_config for B6-02 shutdown WAL test"),
    );

    let dispatch_arc = |h: &FabushiAppHost, payload: &Value| -> Value {
        serde_json::from_str(&h.dispatch(&payload.to_string())).unwrap()
    };

    m1_entered.store(false, Ordering::SeqCst);
    m1_release.store(false, Ordering::SeqCst);

    assert_eq!(
        dispatch_arc(
            &host_arc,
            &json!({"method": "auth.login", "params": {"username": "alice_b602", "password": "pw"}})
        )["ok"],
        true
    );
    assert_eq!(
        dispatch_arc(
            &host_arc,
            &json!({"method": "outbox.setNetworkState", "params": {"online": false}})
        )["ok"],
        true
    );
    assert_eq!(
        dispatch_arc(
            &host_arc,
            &json!({
                "method": "outbox.enqueue",
                "params": {"chat_id": 9299, "text": "HOLD_M1_SHUTDOWN", "client_nonce": 929_001}
            })
        )["ok"],
        true
    );
    assert_eq!(
        dispatch_arc(
            &host_arc,
            &json!({
                "method": "outbox.enqueue",
                "params": {"chat_id": 9299, "text": "PENDING_M2_SHUTDOWN", "client_nonce": 929_002}
            })
        )["ok"],
        true
    );
    assert_eq!(
        dispatch_arc(
            &host_arc,
            &json!({"method": "outbox.setNetworkState", "params": {"online": true}})
        )["ok"],
        true
    );
    assert_eq!(dispatch_arc(&host_arc, &json!({"method": "outbox.retryNow"}))["ok"], true);

    let wait_sd = Instant::now();
    while !m1_entered.load(Ordering::SeqCst) {
        assert!(
            wait_sd.elapsed() < Duration::from_secs(4),
            "Timed out waiting for OutboxWorker HOLD_M1_SHUTDOWN to enter HTTP barrier"
        );
        thread::sleep(Duration::from_millis(5));
    }

    // Invoke shutdown() while retaining Arc<FabushiAppHost> (Tokio runtime stays alive!)
    host_arc.shutdown();

    let cache_db2 = dir2.path().join("client_cache.db");
    let wal_path2 = dir2.path().join("client_cache.db-wal");
    let wal_len_before = std::fs::metadata(&wal_path2).map(|m| m.len()).unwrap_or(0);
    assert_eq!(
        wal_len_before, 0,
        "client_cache.db-wal must be 0 bytes immediately after shutdown() checkpoint_all_wals()"
    );

    let snapshot_outbox_ro = |db_path: &std::path::Path| -> Vec<(i64, String, Option<i64>, i64)> {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT client_nonce, status, server_message_id, retry_count
                 FROM outbox_messages ORDER BY client_nonce ASC",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };

    let rows_before = snapshot_outbox_ro(&cache_db2);
    assert_eq!(rows_before.len(), 2, "Precondition: 2 outbox rows persisted on disk");
    let wal_len_after_snap = std::fs::metadata(&wal_path2).map(|m| m.len()).unwrap_or(0);
    assert_eq!(wal_len_after_snap, 0);

    // Release HTTP barrier so server returns HTTP 200 OK while Arc<FabushiAppHost> is STILL alive!
    m1_release.store(true, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(300));

    let wal_len_after = std::fs::metadata(&wal_path2).map(|m| m.len()).unwrap_or(0);
    assert_eq!(
        wal_len_after, 0,
        "client_cache.db-wal MUST remain 0 bytes (zero post-shutdown OutboxWorker WAL writes, B6-02)"
    );
    let rows_after = snapshot_outbox_ro(&cache_db2);
    assert_eq!(
        rows_after, rows_before,
        "outbox_messages MUST NOT be mutated after shutdown() when HTTP response completes (B6-02)"
    );
    assert!(
        !wire_log
            .lock()
            .unwrap()
            .iter()
            .any(|(_, txt)| txt == "PENDING_M2_SHUTDOWN"),
        "Stopped OutboxWorker MUST NEVER claim/send PENDING_M2_SHUTDOWN after shutdown() (B6-02)"
    );

    drop(host_arc);
}



