use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::Query;
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
                            "last_message": "Ready for v1.6.7"
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
                        "query": "v1.6.7",
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
                "params": {"query": "v1.6.7", "chat_id": 300, "limit": 10}
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
