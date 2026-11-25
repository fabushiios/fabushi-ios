use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures::{SinkExt, StreamExt};
use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use bot_browser::cdp::{CdpBridge, CdpTransport};
use bot_browser::orchestrator::{
    BotBrowserOrchestrator, OrchestratorConfig, SessionTransportFactory,
};
use bot_browser::vault::{CredentialVault, MemoryMasterKeyStore};
use bot_browser::{
    BotBrowserError, BotBrowserEvent, BrowserBackendKind, BrowserCookie, BrowserSessionBackend,
    CdpBrowserBackend, KeyEventKind, KeyInputEvent, NormalizedOrigin, PointerButton,
    PointerEventKind, PointerInputEvent, SameSitePolicy, SemanticLocator,
};

#[derive(Clone, Debug)]
struct RecordedCall {
    method: String,
    params: Value,
}

struct MockPageState {
    history: Vec<(i64, String, String)>,
    history_idx: usize,
    cookies: Vec<Value>,
    inserted_texts: Vec<String>,
    focused_field: String,
    username_val: String,
    password_val: String,
    search_val: String,
    clicked_count: usize,
    eval_delay: Option<Duration>,
    next_id: i64,
}

impl Default for MockPageState {
    fn default() -> Self {
        Self {
            history: vec![(1, "https://example.com/start".into(), "Start Page".into())],
            history_idx: 0,
            cookies: vec![json!({
                "name": "session_id",
                "value": "init_secret_cookie",
                "domain": "example.com",
                "path": "/",
                "secure": true,
                "httpOnly": true
            })],
            inserted_texts: Vec::new(),
            focused_field: String::new(),
            username_val: String::new(),
            password_val: String::new(),
            search_val: String::new(),
            clicked_count: 0,
            eval_delay: None,
            next_id: 2,
        }
    }
}

struct StatefulMockTransport {
    state: Arc<Mutex<MockPageState>>,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl StatefulMockTransport {
    fn new(state: Arc<Mutex<MockPageState>>) -> Self {
        Self {
            state,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().await.clone()
    }
}

#[async_trait]
impl CdpTransport for StatefulMockTransport {
    async fn send_command(&self, method: &str, params: Value) -> Result<Value, BotBrowserError> {
        self.calls.lock().await.push(RecordedCall {
            method: method.to_string(),
            params: params.clone(),
        });

        let delay = self.state.lock().await.eval_delay;
        if let Some(d) = delay {
            if method == "Page.navigate" || method == "Runtime.evaluate" {
                tokio::time::sleep(d).await;
            }
        }

        let mut st = self.state.lock().await;
        match method {
            "Page.navigate" => {
                let url = params
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://example.com/")
                    .to_string();
                let id = st.next_id;
                st.next_id += 1;
                let title = format!("Title for {url}");
                let keep = st.history_idx + 1;
                st.history.truncate(keep);
                st.history.push((id, url, title));
                st.history_idx = st.history.len() - 1;
                Ok(json!({ "frameId": "main" }))
            }
            "Page.getNavigationHistory" => {
                let entries: Vec<Value> = st
                    .history
                    .iter()
                    .map(|(id, url, title)| json!({ "id": id, "url": url, "title": title }))
                    .collect();
                Ok(json!({
                    "currentIndex": st.history_idx,
                    "entries": entries
                }))
            }
            "Page.navigateToHistoryEntry" => {
                let target_id = params.get("entryId").and_then(|v| v.as_i64()).unwrap_or(0);
                if let Some(pos) = st.history.iter().position(|(id, _, _)| *id == target_id) {
                    st.history_idx = pos;
                    Ok(json!({}))
                } else {
                    Err(BotBrowserError::Cdp("history entry not found".into()))
                }
            }
            "Page.reload" => Ok(json!({})),
            "Runtime.evaluate" => {
                let expr = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if expr.contains("throw_test_exception") {
                    return Ok(json!({
                        "exceptionDetails": {
                            "text": "Uncaught",
                            "exception": { "description": "ReferenceError: boom" }
                        }
                    }));
                }
                if expr.contains("document.readyState") {
                    return Ok(json!({ "result": { "value": "complete" } }));
                }
                if expr.contains("window.location.href") && expr.contains("document.title") && !expr.contains("interactive_elements") {
                    let (_, url, title) = &st.history[st.history_idx];
                    return Ok(json!({
                        "result": {
                            "value": json!({ "url": url, "title": title }).to_string()
                        }
                    }));
                }
                if expr.contains("window.location.href") && !expr.contains("interactive_elements") {
                    let (_, url, _) = &st.history[st.history_idx];
                    return Ok(json!({ "result": { "value": url } }));
                }
                if expr.contains("document.title") && !expr.contains("interactive_elements") {
                    let (_, _, title) = &st.history[st.history_idx];
                    return Ok(json!({ "result": { "value": title } }));
                }
                if expr.contains("interactive_elements") {
                    let (_, url, title) = &st.history[st.history_idx];
                    let origin = NormalizedOrigin::parse(url)
                        .map(|o| o.as_str().to_string())
                        .unwrap_or_default();
                    return Ok(json!({
                        "result": {
                            "value": json!({
                                "url": url,
                                "origin": origin,
                                "title": title,
                                "text_excerpt": "Sign in to continue",
                                "redacted_field_count": 0,
                                "interactive_elements": [
                                    {
                                        "ref_id": "el_1",
                                        "tag": "input",
                                        "input_type": "text",
                                        "name": "username",
                                        "test_id": "login-user",
                                        "role": "textbox",
                                        "text": "",
                                        "value": st.username_val,
                                        "is_password": false,
                                        "selector": "[data-testid=\"login-user\"]"
                                    },
                                    {
                                        "ref_id": "el_2",
                                        "tag": "input",
                                        "input_type": "password",
                                        "name": "password",
                                        "test_id": "login-pass",
                                        "role": "textbox",
                                        "text": "",
                                        "value": "LEAK_IF_NOT_REDACTED",
                                        "is_password": false,
                                        "selector": "[data-testid=\"login-pass\"]"
                                    }
                                ]
                            }).to_string()
                        }
                    }));
                }
                if expr.contains("shouldFocus") && expr.contains("loc") {
                    if expr.contains("missing-id") {
                        return Ok(json!({ "result": { "value": json!({ "found": false }).to_string() } }));
                    }
                    if expr.contains("login-pass") {
                        st.focused_field = "password".into();
                        return Ok(json!({
                            "result": {
                                "value": json!({
                                    "found": true,
                                    "strategy": "test_id",
                                    "tag": "input",
                                    "is_sensitive": true,
                                    "selector": "[data-testid=\"login-pass\"]",
                                    "rect": { "x": 10.0, "y": 50.0, "width": 200.0, "height": 32.0, "device_pixel_ratio": 1.0 },
                                    "center_x": 110.0,
                                    "center_y": 66.0,
                                    "text": ""
                                }).to_string()
                            }
                        }));
                    }
                    if expr.contains("Search Box") || expr.contains("</script>") {
                        st.focused_field = "search".into();
                        return Ok(json!({
                            "result": {
                                "value": json!({
                                    "found": true,
                                    "strategy": "aria_label",
                                    "tag": "input",
                                    "is_sensitive": false,
                                    "selector": "[aria-label=\"Search Box\"]",
                                    "rect": { "x": 10.0, "y": 10.0, "width": 200.0, "height": 32.0, "device_pixel_ratio": 1.0 },
                                    "center_x": 110.0,
                                    "center_y": 26.0,
                                    "text": st.search_val
                                }).to_string()
                            }
                        }));
                    }
                    if expr.contains("status-banner") {
                        return Ok(json!({
                            "result": {
                                "value": json!({
                                    "found": true,
                                    "strategy": "test_id",
                                    "tag": "div",
                                    "is_sensitive": false,
                                    "selector": "[data-testid=\"status-banner\"]",
                                    "rect": { "x": 0.0, "y": 0.0, "width": 100.0, "height": 20.0, "device_pixel_ratio": 1.0 },
                                    "center_x": 50.0,
                                    "center_y": 10.0,
                                    "text": "Status: Ready"
                                }).to_string()
                            }
                        }));
                    }
                    return Ok(json!({
                        "result": {
                            "value": json!({
                                "found": true,
                                "strategy": "test_id",
                                "tag": "button",
                                "is_sensitive": false,
                                "selector": "[data-testid=\"submit-btn\"]",
                                "rect": { "x": 20.0, "y": 100.0, "width": 100.0, "height": 40.0, "device_pixel_ratio": 1.0 },
                                "center_x": 70.0,
                                "center_y": 120.0,
                                "text": "Submit"
                            }).to_string()
                        }
                    }));
                }
                if expr.contains("document.activeElement") {
                    return Ok(json!({ "result": { "value": true } }));
                }
                Ok(json!({ "result": { "value": { "evaluated": true } } }))
            }
            "Input.insertText" => {
                let text = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                st.inserted_texts.push(text.clone());
                if st.focused_field == "password" {
                    st.password_val = text;
                } else if st.focused_field == "search" {
                    st.search_val = text;
                } else {
                    st.username_val = text;
                }
                Ok(json!({}))
            }
            "Input.dispatchMouseEvent" => {
                if params.get("type").and_then(|v| v.as_str()) == Some("mousePressed") {
                    st.clicked_count += 1;
                }
                Ok(json!({}))
            }
            "Input.dispatchKeyEvent" => Ok(json!({})),
            "Page.captureScreenshot" => {
                let mut img = RgbaImage::new(4, 3);
                img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
                let mut png = Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(img)
                    .write_to(&mut png, ImageFormat::Png)
                    .unwrap();
                let b64 = BASE64_STANDARD.encode(png.into_inner());
                Ok(json!({ "data": b64 }))
            }
            "Network.getCookies" => Ok(json!({ "cookies": st.cookies })),
            "Network.setCookies" => {
                let incoming = params
                    .get("cookies")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                st.cookies.extend(incoming);
                Ok(json!({}))
            }
            _ => Err(BotBrowserError::Cdp(format!("unexpected method {method}"))),
        }
    }

    async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
        Ok("mock://cdp".to_string())
    }
}

#[tokio::test]
async fn cdp_backend_full_navigation_locator_fill_cookie_and_input_ops() {
    let state = Arc::new(Mutex::new(MockPageState::default()));
    let transport = Arc::new(StatefulMockTransport::new(Arc::clone(&state)));
    let backend = CdpBrowserBackend::new_mock(transport.clone());

    assert_eq!(backend.backend_kind(), BrowserBackendKind::MockCdp);

    // 1. Navigation & history
    let nav1 = backend
        .navigate("https://example.com/login")
        .await
        .expect("navigate 1");
    assert_eq!(nav1.url, "https://example.com/login");
    assert_eq!(nav1.origin, "https://example.com");
    assert_eq!(nav1.backend, BrowserBackendKind::MockCdp);

    let nav2 = backend
        .navigate("https://example.com/home")
        .await
        .expect("navigate 2");
    assert_eq!(nav2.url, "https://example.com/home");

    backend.go_back().await.expect("go_back");
    assert_eq!(backend.current_url().await.unwrap(), "https://example.com/login");

    backend.go_forward().await.expect("go_forward");
    assert_eq!(backend.current_url().await.unwrap(), "https://example.com/home");

    backend.reload().await.expect("reload");
    assert_eq!(backend.page_title().await.unwrap(), "Title for https://example.com/home");

    // 2. Redacted snapshot preserves test_id & role, redacts password
    let snap = backend.snapshot_dom().await.expect("snapshot_dom");
    assert_eq!(snap.redacted_field_count, 1);
    let pass_node = snap
        .interactive_elements
        .iter()
        .find(|n| n.test_id.as_deref() == Some("login-pass"))
        .expect("password node");
    assert!(pass_node.is_password);
    assert_eq!(pass_node.value.as_deref(), Some("[REDACTED]"));
    assert_eq!(pass_node.role.as_deref(), Some("textbox"));

    // 3. eval_js + exception handling
    let eval_ok = backend.eval_js("1 + 1").await.expect("eval ok");
    assert_eq!(eval_ok["evaluated"], true);
    let eval_err = backend.eval_js("throw_test_exception").await;
    assert!(matches!(eval_err, Err(BotBrowserError::Cdp(_))));

    // 4. Click locator + wait_for + extract_text
    let click_out = backend
        .click_locator(&SemanticLocator::by_test_id("submit-btn"))
        .await
        .expect("click");
    assert!(click_out.ok);
    assert_eq!(click_out.matched_strategy.as_deref(), Some("test_id"));
    assert_eq!(click_out.x, Some(70.0));
    assert_eq!(click_out.y, Some(120.0));

    let wait_out = backend
        .wait_for_locator(&SemanticLocator::by_test_id("status-banner"), 500)
        .await
        .expect("wait");
    assert!(wait_out.ok);
    let text = backend
        .extract_text_locator(&SemanticLocator::by_test_id("status-banner"))
        .await
        .expect("extract");
    assert_eq!(text, "Status: Ready");

    // 5. Secret fill ONLY uses Input.insertText and never leaks secret in Runtime.evaluate / ActionOutcome / metrics
    let secret_pw = "Super$ecretPass!999";
    let pass_loc = SemanticLocator::by_test_id("login-pass");
    let non_secret_attempt = backend.fill_locator(&pass_loc, "oops", false).await;
    assert!(non_secret_attempt.is_err(), "must refuse non-secret fill on sensitive locator");

    let extract_secret_attempt = backend.extract_text_locator(&pass_loc).await;
    assert!(extract_secret_attempt.is_err(), "must refuse extracting sensitive field");

    let fill_out = backend
        .fill_locator(&pass_loc, secret_pw, true)
        .await
        .expect("secret fill");
    assert!(fill_out.ok);
    assert!(!format!("{fill_out:?}").contains(secret_pw));

    // Verify injection safety on non-secret locator with quotes/</script>/newlines
    let search_loc = SemanticLocator::by_aria_label("Search Box\"</script>\n");
    backend
        .fill_locator(&search_loc, "rust browser", false)
        .await
        .expect("fill search");

    // Verify secret NEVER appeared in any Runtime.evaluate call
    let calls = transport.calls().await;
    for call in &calls {
        if call.method == "Runtime.evaluate" {
            let expr = call.params.get("expression").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                !expr.contains(secret_pw),
                "Runtime.evaluate must NEVER contain secret password"
            );
        }
    }
    assert!(calls.iter().any(|c| {
        c.method == "Input.insertText"
            && c.params.get("text").and_then(|v| v.as_str()) == Some(secret_pw)
    }));

    // 6. Key, scroll, pointer, key input events
    backend.press_key("Enter").await.expect("press_key");
    backend.scroll(0.0, 240.0).await.expect("scroll");
    backend
        .dispatch_pointer_event(PointerInputEvent {
            kind: PointerEventKind::Move,
            x: 15.0,
            y: 25.0,
            button: PointerButton::None,
            click_count: 0,
            delta_x: 0.0,
            delta_y: 0.0,
            modifiers: 0,
        })
        .await
        .expect("pointer move");
    backend
        .dispatch_key_event(KeyInputEvent {
            kind: KeyEventKind::Char,
            key: "Unidentified".into(),
            code: None,
            text: Some("你好".into()),
            windows_virtual_key_code: None,
            modifiers: 0,
        })
        .await
        .expect("unicode char via insertText");

    // 7. Screenshot PNG & offscreen RGBA8 frame buffer
    let png = backend.capture_screenshot().await.expect("screenshot");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let frame = backend.capture_frame_rgba().await.expect("frame rgba");
    assert_eq!(frame.width, 4);
    assert_eq!(frame.height, 3);
    assert_eq!(frame.rgba_bytes.len(), 4 * 3 * 4);
    assert_eq!(&frame.rgba_bytes[0..4], &[255, 0, 0, 255]);

    // 8. Cookies export/import & Debug redaction
    let mut new_cookie = BrowserCookie::new("auth_token", "tok_secret_abc", ".x.com");
    new_cookie.http_only = true;
    new_cookie.secure = true;
    new_cookie.same_site = Some(SameSitePolicy::Lax);
    assert!(!format!("{new_cookie:?}").contains("tok_secret_abc"));
    assert_eq!(
        backend.import_cookies(&[new_cookie]).await.expect("import cookies"),
        1
    );
    let x_cookies = backend
        .export_cookies(Some("x.com"))
        .await
        .expect("export x.com cookies");
    assert_eq!(x_cookies.len(), 1);
    assert_eq!(x_cookies[0].name, "auth_token");
    assert_eq!(x_cookies[0].value, "tok_secret_abc");

    // 9. Telemetry metrics check (zero secret leakage)
    let metrics = backend.performance_metrics();
    assert_eq!(metrics.backend, Some(BrowserBackendKind::MockCdp));
    assert!(metrics.total_operations >= 10);
    assert!(metrics.failed_operations >= 2);
    assert!(!format!("{metrics:?}").contains(secret_pw));
}

#[tokio::test]
async fn cdp_backend_execution_is_non_blocking_for_concurrent_ui_task() {
    let state = Arc::new(Mutex::new(MockPageState {
        eval_delay: Some(Duration::from_millis(80)),
        ..Default::default()
    }));
    let transport = Arc::new(StatefulMockTransport::new(state));
    let backend = Arc::new(CdpBrowserBackend::new_mock(transport));

    let ticks = Arc::new(AtomicUsize::new(0));
    let ticks_bg = Arc::clone(&ticks);
    let ticker = tokio::spawn(async move {
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ticks_bg.fetch_add(1, Ordering::SeqCst);
        }
    });

    let b = Arc::clone(&backend);
    let nav_res = b.navigate("https://example.com/slow").await;
    assert!(nav_res.is_ok());
    let ticks_during_nav = ticks.load(Ordering::SeqCst);
    assert!(
        ticks_during_nav >= 10,
        "expected UI ticker to advance >= 10 ticks while async CDP navigate ran, got {ticks_during_nav}"
    );
    let _ = ticker.await;
}

#[tokio::test]
async fn ws_cdp_backend_connects_over_local_loopback_and_locator_priority_works() {
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_url = format!("ws://127.0.0.1:{ws_port}/devtools/page/1");

    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http_listener.local_addr().unwrap().port();
    let http_endpoint = format!("http://127.0.0.1:{http_port}");

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = http_listener.accept() => {
                    let ws_url_c = ws_url.clone();
                    tokio::spawn(async move {
                        let mut buf = [0u8; 1024];
                        let _ = sock.read(&mut buf).await;
                        let body = json!([{
                            "id": "p1",
                            "type": "page",
                            "webSocketDebuggerUrl": ws_url_c
                        }]).to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                    });
                }
                Ok((stream, _)) = ws_listener.accept() => {
                    tokio::spawn(async move {
                        let Ok(mut ws) = accept_async(stream).await else { return; };
                        while let Some(Ok(Message::Text(txt))) = ws.next().await {
                            let Ok(req) = serde_json::from_str::<Value>(&txt) else { continue; };
                            let id = req.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                            let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
                            let result = match method {
                                "Page.navigate" => json!({ "frameId": "main" }),
                                "Runtime.evaluate" => json!({ "result": { "value": "complete" } }),
                                _ => json!({}),
                            };
                            let _ = ws.send(Message::Text(json!({ "id": id, "result": result }).to_string())).await;
                        }
                    });
                }
            }
        }
    });

    let backend = CdpBrowserBackend::connect_cdp(
        BrowserBackendKind::ChromeCdp,
        http_endpoint,
        Duration::from_secs(5),
    );
    let res = backend.navigate("https://example.org/test").await.expect("ws nav");
    assert_eq!(res.backend, BrowserBackendKind::ChromeCdp);
    assert_eq!(res.origin, "https://example.org");
    let _ = shutdown_tx.send(());
}

struct MockFactory(Arc<StatefulMockTransport>);

#[async_trait]
impl SessionTransportFactory for MockFactory {
    async fn create_transport(
        &self,
        _session_id: &str,
        _data_dir: &std::path::Path,
        _origin_hint: Option<&NormalizedOrigin>,
        _event_tx: &tokio::sync::broadcast::Sender<BotBrowserEvent>,
    ) -> Result<Arc<dyn CdpTransport>, BotBrowserError> {
        Ok(Arc::clone(&self.0) as Arc<dyn CdpTransport>)
    }
}

#[tokio::test]
async fn orchestrator_ensure_session_backend_and_cookie_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let vault = Arc::new(CredentialVault::open_in_memory(&MemoryMasterKeyStore::new()).unwrap());
    let state = Arc::new(Mutex::new(MockPageState::default()));
    let transport = Arc::new(StatefulMockTransport::new(Arc::clone(&state)));
    let orch = BotBrowserOrchestrator::with_transport_factory(
        OrchestratorConfig::new(tmp.path()),
        vault,
        Arc::new(MockFactory(transport)),
    );

    let origin = NormalizedOrigin::parse("https://example.com").unwrap();
    let b1 = orch
        .ensure_session_backend("sess-1", Some(&origin))
        .await
        .unwrap();
    let b2 = orch
        .ensure_session_backend("sess-1", Some(&origin))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&b1, &b2));

    let saved_count = orch
        .persist_session_cookies("sess-1", "sess-1", Some(&origin))
        .await
        .unwrap();
    assert_eq!(saved_count, 1);

    state.lock().await.cookies.clear();
    let restored = orch
        .restore_session_cookies("sess-1", "sess-1", Some(&origin))
        .await
        .unwrap();
    assert_eq!(restored, 1);
    let exported = b1.export_cookies(Some("example.com")).await.unwrap();
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].name, "session_id");
    assert_eq!(exported[0].value, "init_secret_cookie");

    // Ensure CdpBridge::send_guarded_command is directly usable too.
    let bridge = CdpBridge::new(Arc::clone(b1.bridge().transport()));
    let res = bridge
        .send_guarded_command("Page.reload", json!({ "ignoreCache": true }))
        .await;
    assert!(res.is_ok());

    let mut unused_map = HashMap::new();
    unused_map.insert("k", "v");
    assert_eq!(unused_map.get("k"), Some(&"v"));
}

#[tokio::test]
async fn ws_cdp_browser_url_with_uuid_attaches_target_and_sends_session_id() {
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_url = format!("ws://127.0.0.1:{ws_port}/devtools/browser/test-uuid-1234");

    let recorded_requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded_srv = Arc::clone(&recorded_requests);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((stream, _)) = ws_listener.accept() => {
                    let reqs = Arc::clone(&recorded_srv);
                    tokio::spawn(async move {
                        let Ok(mut ws) = accept_async(stream).await else { return; };
                        while let Some(Ok(Message::Text(txt))) = ws.next().await {
                            let Ok(req) = serde_json::from_str::<Value>(&txt) else { continue; };
                            reqs.lock().await.push(req.clone());
                            let id = req.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                            let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
                            let session_id = req.get("sessionId").and_then(|v| v.as_str());
                            let resp = match method {
                                "Target.getTargets" => json!({
                                    "id": id,
                                    "result": {
                                        "targetInfos": [{
                                            "targetId": "page-target-1",
                                            "type": "page",
                                            "url": "about:blank"
                                        }]
                                    }
                                }),
                                "Target.attachToTarget" => json!({
                                    "id": id,
                                    "result": { "sessionId": "attached-sess-99" }
                                }),
                                "Page.navigate" => {
                                    if session_id == Some("attached-sess-99") {
                                        json!({ "id": id, "result": { "frameId": "main" } })
                                    } else {
                                        json!({ "id": id, "error": { "code": -32601, "message": "'Page.navigate' wasn't found" } })
                                    }
                                }
                                "Runtime.evaluate" => {
                                    if session_id == Some("attached-sess-99") {
                                        json!({ "id": id, "result": { "result": { "value": "complete" } } })
                                    } else {
                                        json!({ "id": id, "error": { "code": -32601, "message": "'Runtime.evaluate' wasn't found" } })
                                    }
                                }
                                _ => json!({ "id": id, "result": {} }),
                            };
                            let _ = ws.send(Message::Text(resp.to_string())).await;
                        }
                    });
                }
            }
        }
    });

    let backend = CdpBrowserBackend::connect_cdp(
        BrowserBackendKind::ChromeCdp,
        ws_url,
        Duration::from_secs(5),
    );
    let nav = backend
        .navigate("https://example.org/chrome-uuid")
        .await
        .expect("browser uuid ws connect + attach must succeed");
    assert_eq!(nav.origin, "https://example.org");

    let eval_res = backend.eval_js("document.readyState").await.expect("eval with sessionId");
    assert_eq!(eval_res, "complete");

    let reqs = recorded_requests.lock().await;
    assert!(
        reqs.iter()
            .any(|r| r.get("method").and_then(|v| v.as_str()) == Some("Target.getTargets"))
    );
    let attach = reqs
        .iter()
        .find(|r| r.get("method").and_then(|v| v.as_str()) == Some("Target.attachToTarget"))
        .expect("Target.attachToTarget must be called");
    assert_eq!(attach["params"]["targetId"], "page-target-1");
    assert_eq!(attach["params"]["flatten"], true);

    let nav_req = reqs
        .iter()
        .find(|r| r.get("method").and_then(|v| v.as_str()) == Some("Page.navigate"))
        .expect("Page.navigate must be called");
    assert_eq!(nav_req["sessionId"], "attached-sess-99");

    let eval_req = reqs
        .iter()
        .find(|r| r.get("method").and_then(|v| v.as_str()) == Some("Runtime.evaluate"))
        .expect("Runtime.evaluate must be called");
    assert_eq!(eval_req["sessionId"], "attached-sess-99");

    let _ = shutdown_tx.send(());
}

struct SensitivityAndScrollMockTransport {
    fail_wheel: bool,
    scroll_eval_throws: bool,
    last_eval_scripts: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl CdpTransport for SensitivityAndScrollMockTransport {
    async fn send_command(&self, method: &str, params: Value) -> Result<Value, BotBrowserError> {
        match method {
            "Input.dispatchMouseEvent" if self.fail_wheel => {
                Err(BotBrowserError::Cdp("mouseWheel unsupported".into()))
            }
            "Input.dispatchMouseEvent" => Ok(json!({})),
            "Runtime.evaluate" => {
                let expr = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.last_eval_scripts.lock().await.push(expr.clone());
                if expr.contains("window.scrollBy") && self.scroll_eval_throws {
                    return Ok(json!({
                        "exceptionDetails": {
                            "text": "Uncaught",
                            "exception": { "description": "ReferenceError: scroll failed" }
                        }
                    }));
                }
                if expr.contains("interactive_elements") {
                    return Ok(json!({
                        "result": {
                            "value": json!({
                                "url": "https://example.com/settings",
                                "origin": "https://example.com",
                                "title": "Settings",
                                "text_excerpt": "API Settings",
                                "redacted_field_count": 0,
                                "interactive_elements": [
                                    {
                                        "ref_id": "el_1",
                                        "tag": "input",
                                        "input_type": "text",
                                        "name": "api_key",
                                        "text": "",
                                        "value": "sk-live-secret-123",
                                        "is_password": false,
                                        "selector": "input[name=\"api_key\"]"
                                    },
                                    {
                                        "ref_id": "el_2",
                                        "tag": "input",
                                        "input_type": "hidden",
                                        "name": "auth_token",
                                        "text": "",
                                        "value": "raw-auth-token-456",
                                        "is_password": false,
                                        "selector": "input[name=\"auth_token\"]"
                                    },
                                    {
                                        "ref_id": "el_3",
                                        "tag": "input",
                                        "input_type": "text",
                                        "name": "ct0",
                                        "text": "",
                                        "value": "raw-ct0-789",
                                        "is_password": false,
                                        "selector": "input[name=\"ct0\"]"
                                    }
                                ]
                            }).to_string()
                        }
                    }));
                }
                if expr.contains("shouldFocus") && expr.contains("loc") {
                    if expr.contains("status-banner") {
                        return Ok(json!({
                            "result": {
                                "value": json!({
                                    "found": true,
                                    "strategy": "test_id",
                                    "tag": "div",
                                    "is_sensitive": false,
                                    "selector": "[data-testid=\"status-banner\"]",
                                    "rect": { "x": 0.0, "y": 0.0, "width": 50.0, "height": 20.0, "device_pixel_ratio": 1.0 },
                                    "center_x": 25.0,
                                    "center_y": 10.0,
                                    "text": "OK"
                                }).to_string()
                            }
                        }));
                    }
                    // Generic CSS locator resolving to a sensitive api_key/auth_token element
                    return Ok(json!({
                        "result": {
                            "value": json!({
                                "found": true,
                                "strategy": "css_selector",
                                "tag": "input",
                                "is_sensitive": true,
                                "selector": "input[name=\"api_key\"]",
                                "rect": { "x": 0.0, "y": 0.0, "width": 100.0, "height": 20.0, "device_pixel_ratio": 1.0 },
                                "center_x": 50.0,
                                "center_y": 10.0,
                                "text": "sk-live-secret-123"
                            }).to_string()
                        }
                    }));
                }
                Ok(json!({ "result": { "value": true } }))
            }
            _ => Ok(json!({})),
        }
    }

    async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
        Ok("mock://sensitivity".into())
    }
}

#[tokio::test]
async fn cdp_backend_defensive_boundaries_scroll_wait_cookies_and_sensitive_redaction() {
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(SensitivityAndScrollMockTransport {
        fail_wheel: true,
        scroll_eval_throws: true,
        last_eval_scripts: Arc::clone(&scripts),
    });
    let backend = CdpBrowserBackend::new_mock(transport);

    // 1. wait_for_locator with u64::MAX must NOT panic
    let wait_ok = backend
        .wait_for_locator(&SemanticLocator::by_test_id("status-banner"), u64::MAX)
        .await
        .expect("u64::MAX timeout must not panic");
    assert!(wait_ok.ok);

    // 2. scroll rejects non-finite floats and propagates JS fallback exceptionDetails
    let inf_err = backend.scroll(f64::INFINITY, 0.0).await.unwrap_err();
    assert!(matches!(inf_err, BotBrowserError::Cdp(_)));
    let nan_err = backend.scroll(0.0, f64::NAN).await.unwrap_err();
    assert!(matches!(nan_err, BotBrowserError::Cdp(_)));
    let js_scroll_err = backend.scroll(0.0, 100.0).await.unwrap_err();
    assert!(
        matches!(js_scroll_err, BotBrowserError::Cdp(ref msg) if msg.contains("JS exception")),
        "expected JS exception from scroll fallback, got {js_scroll_err:?}"
    );

    // 3. snapshot_dom redacts api_key, auth_token, and ct0
    let snap = backend.snapshot_dom().await.expect("snapshot_dom");
    assert_eq!(snap.redacted_field_count, 3);
    for name in ["api_key", "auth_token", "ct0"] {
        let el = snap
            .interactive_elements
            .iter()
            .find(|n| n.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing element {name}"));
        assert!(el.is_password, "{name} must be flagged sensitive");
        assert_eq!(el.value.as_deref(), Some("[REDACTED]"));
    }

    // 4. extract_text_locator refuses sensitive locator hints AND sensitive resolved elements
    for hint_loc in [
        SemanticLocator::by_name("api_key"),
        SemanticLocator::by_name("auth_token"),
        SemanticLocator::by_name("ct0"),
        SemanticLocator::by_name("user_pin"),
    ] {
        assert!(
            backend.extract_text_locator(&hint_loc).await.is_err(),
            "must refuse extract_text_locator for {}",
            hint_loc.display_label()
        );
    }
    assert!(
        backend
            .extract_text_locator(&SemanticLocator::from_css("input"))
            .await
            .is_err(),
        "generic CSS locator resolving to sensitive field must refuse extract_text_locator"
    );

    // 5. Generic CSS fill_locator refuses non-secret fill on sensitive element
    assert!(
        backend
            .fill_locator(&SemanticLocator::from_css("input"), "newval", false)
            .await
            .is_err(),
        "generic CSS locator resolving to sensitive field must refuse non-secret fill"
    );

    // Verify generated JS scripts include api_key, auth_token, ct0 sensitivity markers
    let recorded = scripts.lock().await;
    let snap_js = recorded
        .iter()
        .find(|s| s.contains("isSensitiveField"))
        .expect("snapshot JS");
    for marker in ["api_key", "apikey", "auth_token", "access_token", "ct0"] {
        assert!(snap_js.contains(marker), "snapshot JS missing {marker}");
    }
    let resolve_js = recorded
        .iter()
        .find(|s| s.contains("function isSensitive("))
        .expect("resolve locator JS");
    for marker in ["api_key", "apikey", "auth_token", "access_token", "ct0"] {
        assert!(resolve_js.contains(marker), "resolve JS missing {marker}");
    }

    // 6. import_cookies rejects CRLF / semicolon / null / control chars in name, value, domain, path
    let mut bad_path = BrowserCookie::new("sid", "v1", "example.com");
    bad_path.path = "/\r\nSet-Cookie: evil=1".into();
    assert!(backend.import_cookies(&[bad_path]).await.is_err());

    let mut bad_semi_path = BrowserCookie::new("sid", "v1", "example.com");
    bad_semi_path.path = "/; HttpOnly".into();
    assert!(backend.import_cookies(&[bad_semi_path]).await.is_err());

    let bad_val = BrowserCookie::new("sid", "v1\0evil", "example.com");
    assert!(backend.import_cookies(&[bad_val]).await.is_err());
}

#[tokio::test]
async fn snapshot_redacts_sensitive_textarea_and_contenteditable_text_and_records_scroll_fallback_error()
{
    struct TextareaContentEditableTransport {
        last_eval_scripts: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl CdpTransport for TextareaContentEditableTransport {
        async fn send_command(
            &self,
            method: &str,
            params: Value,
        ) -> Result<Value, BotBrowserError> {
            match method {
                "Input.dispatchMouseEvent" => Err(BotBrowserError::Cdp("mouseWheel unsupported".into())),
                "Runtime.evaluate" => {
                    let expr = params
                        .get("expression")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    self.last_eval_scripts.lock().await.push(expr.to_string());
                    if expr.contains("window.scrollBy") {
                        return Ok(json!({
                            "exceptionDetails": {
                                "exception": { "description": "ReferenceError: scrollBy failed" }
                            }
                        }));
                    }
                    if expr.contains("interactive_elements") {
                        return Ok(json!({
                            "result": {
                                "value": json!({
                                    "url": "https://example.com/settings",
                                    "origin": "https://example.com",
                                    "title": "API Settings",
                                    "text_excerpt": "API Settings sk-textarea-secret sk-ce-secret Public footer",
                                    "redacted_field_count": 0,
                                    "interactive_elements": [
                                        {
                                            "ref_id": "el_1",
                                            "tag": "textarea",
                                            "input_type": null,
                                            "name": "api_key",
                                            "text": "sk-textarea-secret",
                                            "value": "sk-textarea-secret",
                                            "is_password": false,
                                            "selector": "textarea[name=\"api_key\"]"
                                        },
                                        {
                                            "ref_id": "el_2",
                                            "tag": "div",
                                            "input_type": null,
                                            "name": null,
                                            "test_id": "auth_token",
                                            "text": "sk-ce-secret",
                                            "value": null,
                                            "is_password": false,
                                            "selector": "[data-testid=\"auth_token\"]"
                                        },
                                        {
                                            "ref_id": "el_3",
                                            "tag": "textarea",
                                            "input_type": null,
                                            "name": "bio",
                                            "text": "Public bio text",
                                            "value": "Public bio text",
                                            "is_password": false,
                                            "selector": "textarea[name=\"bio\"]"
                                        }
                                    ]
                                }).to_string()
                            }
                        }));
                    }
                    Ok(json!({ "result": { "value": true } }))
                }
                _ => Ok(json!({})),
            }
        }

        async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
            Ok("mock://textarea-ce".into())
        }
    }

    let scripts = Arc::new(Mutex::new(Vec::new()));
    let backend = CdpBrowserBackend::new_mock(Arc::new(TextareaContentEditableTransport {
        last_eval_scripts: Arc::clone(&scripts),
    }));

    // 1. Sensitive <textarea> and [contenteditable="true"] redact BOTH node.value AND node.text,
    //    and scrub plaintext secrets from text_excerpt.
    let snap = backend.snapshot_dom().await.expect("snapshot_dom");
    assert_eq!(snap.redacted_field_count, 2);

    let ta = snap
        .interactive_elements
        .iter()
        .find(|n| n.name.as_deref() == Some("api_key"))
        .expect("api_key textarea");
    assert!(ta.is_password);
    assert_eq!(ta.value.as_deref(), Some("[REDACTED]"));
    assert_eq!(ta.text, "[REDACTED]");

    let ce = snap
        .interactive_elements
        .iter()
        .find(|n| n.test_id.as_deref() == Some("auth_token"))
        .expect("auth_token contenteditable");
    assert!(ce.is_password);
    assert_eq!(ce.value.as_deref(), Some("[REDACTED]"));
    assert_eq!(ce.text, "[REDACTED]");

    let bio = snap
        .interactive_elements
        .iter()
        .find(|n| n.name.as_deref() == Some("bio"))
        .expect("public bio textarea");
    assert!(!bio.is_password);
    assert_eq!(bio.text, "Public bio text");

    assert!(
        !snap.text_excerpt.contains("sk-textarea-secret"),
        "text_excerpt leaked textarea secret: {}",
        snap.text_excerpt
    );
    assert!(
        !snap.text_excerpt.contains("sk-ce-secret"),
        "text_excerpt leaked contenteditable secret: {}",
        snap.text_excerpt
    );
    assert!(snap.text_excerpt.contains("Public footer"));

    // Verify generated capture_redacted_snapshot JS redacts text & textExcerpt on sensitive elements
    {
        let recorded = scripts.lock().await;
        let snap_js = recorded
            .iter()
            .find(|s| s.contains("isSensitiveField"))
            .expect("snapshot JS");
        assert!(snap_js.contains("const text = sensitive"));
        assert!(snap_js.contains("leakedStrings"));
    }

    // 2. Scroll JS fallback exceptionDetails error flows through record_op failure telemetry
    let scroll_err = backend.scroll(0.0, 120.0).await.unwrap_err();
    assert!(
        matches!(scroll_err, BotBrowserError::Cdp(ref msg) if msg.contains("JS exception")),
        "expected JS exception from scroll fallback, got {scroll_err:?}"
    );
    let metrics = backend.performance_metrics();
    assert!(
        metrics.recent_steps.iter().any(|m| {
            m.operation == "scroll"
                && !m.success
                && m.detail
                    .as_deref()
                    .is_some_and(|d| d.contains("JS exception"))
        }),
        "expected failed scroll JS fallback telemetry in recent_steps: {:?}",
        metrics.recent_steps
    );
}

#[tokio::test]
async fn page_navigate_error_text_and_chrome_error_url_are_rejected_while_downloads_and_clean_nav_succeed()
{
    struct NavResponseTransport {
        nav_result: Value,
        eval_url: String,
    }
    #[async_trait]
    impl CdpTransport for NavResponseTransport {
        async fn send_command(&self, method: &str, _params: Value) -> Result<Value, BotBrowserError> {
            match method {
                "Page.navigate" => Ok(self.nav_result.clone()),
                "Runtime.evaluate" => Ok(json!({
                    "result": {
                        "value": json!({
                            "url": self.eval_url,
                            "title": "Page Title"
                        }).to_string()
                    }
                })),
                _ => Ok(json!({})),
            }
        }
        async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
            Ok("mock://nav-error-text".into())
        }
    }

    // 1. errorText: "net::ERR_CONNECTION_REFUSED" -> CdpBridge::navigate & CdpBrowserBackend::navigate return Err
    let refused_transport = Arc::new(NavResponseTransport {
        nav_result: json!({ "frameId": "main", "errorText": "net::ERR_CONNECTION_REFUSED" }),
        eval_url: "chrome-error://chromewebdata/".into(),
    });
    let bridge = CdpBridge::new(refused_transport.clone());
    let bridge_err = bridge.navigate("http://127.0.0.1:1/").await.unwrap_err();
    assert!(
        matches!(bridge_err, BotBrowserError::Cdp(ref msg) if msg.contains("net::ERR_CONNECTION_REFUSED")),
        "expected Cdp errorText error from bridge, got {bridge_err:?}"
    );
    let backend_refused = CdpBrowserBackend::new_mock(refused_transport);
    let backend_err = backend_refused.navigate("http://127.0.0.1:1/").await.unwrap_err();
    assert!(
        matches!(backend_err, BotBrowserError::Cdp(ref msg) if msg.contains("net::ERR_CONNECTION_REFUSED")),
        "expected Cdp errorText error from backend, got {backend_err:?}"
    );
    assert!(backend_refused.performance_metrics().failed_operations >= 1);

    // 2. isDownload: true + errorText: "net::ERR_ABORTED" -> Ok
    let download_transport = Arc::new(NavResponseTransport {
        nav_result: json!({ "frameId": "main", "errorText": "net::ERR_ABORTED", "isDownload": true }),
        eval_url: "https://example.com/start".into(),
    });
    let download_backend = CdpBrowserBackend::new_mock(download_transport);
    let download_res = download_backend
        .navigate("https://example.com/report.csv")
        .await
        .expect("download navigation with ERR_ABORTED must succeed");
    assert_eq!(download_res.url, "https://example.com/start");

    // 3. Non-download net::ERR_ABORTED -> Err
    let aborted_transport = Arc::new(NavResponseTransport {
        nav_result: json!({ "frameId": "main", "errorText": "net::ERR_ABORTED", "isDownload": false }),
        eval_url: "https://example.com/start".into(),
    });
    let aborted_backend = CdpBrowserBackend::new_mock(aborted_transport);
    assert!(matches!(
        aborted_backend.navigate("https://example.com/cancelled").await,
        Err(BotBrowserError::Cdp(ref m)) if m.contains("net::ERR_ABORTED")
    ));

    // 4. Empty or null errorText -> Ok
    for nav_result in [
        json!({ "frameId": "main", "errorText": "" }),
        json!({ "frameId": "main", "errorText": null }),
    ] {
        let ok_transport = Arc::new(NavResponseTransport {
            nav_result,
            eval_url: "https://example.com/ok".into(),
        });
        let ok_backend = CdpBrowserBackend::new_mock(ok_transport);
        let ok_res = ok_backend
            .navigate("https://example.com/ok")
            .await
            .expect("empty/null errorText must succeed");
        assert_eq!(ok_res.url, "https://example.com/ok");
    }

    // 5. Missing errorText but landing on chrome-error://chromewebdata/ -> secondary guard returns Err
    let chrome_err_transport = Arc::new(NavResponseTransport {
        nav_result: json!({ "frameId": "main" }),
        eval_url: "chrome-error://chromewebdata/".into(),
    });
    let chrome_err_backend = CdpBrowserBackend::new_mock(chrome_err_transport);
    let err = chrome_err_backend
        .navigate("https://example.com/late-fail")
        .await
        .unwrap_err();
    assert!(
        matches!(err, BotBrowserError::Cdp(ref msg) if msg.contains("chrome-error://")),
        "expected chrome-error:// guard error, got {err:?}"
    );
    assert!(chrome_err_backend.performance_metrics().failed_operations >= 1);
}

#[tokio::test]
async fn wait_for_locator_retries_transient_navigation_cdp_errors_and_recovers() {
    struct TransientNavWaitTransport {
        attempts: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl CdpTransport for TransientNavWaitTransport {
        async fn send_command(&self, method: &str, _params: Value) -> Result<Value, BotBrowserError> {
            if method != "Runtime.evaluate" {
                return Ok(json!({}));
            }
            let idx = self.attempts.fetch_add(1, Ordering::SeqCst);
            match idx {
                0 => Err(BotBrowserError::Cdp(
                    r#"CDP error in 'Runtime.evaluate': {"code":-32000,"message":"Inspected target navigated or closed"}"#.into(),
                )),
                1 => Err(BotBrowserError::Cdp(
                    r#"CDP error in 'Runtime.evaluate': {"code":-32000,"message":"Execution context was destroyed."}"#.into(),
                )),
                2 => Err(BotBrowserError::Cdp(
                    r#"CDP error in 'Runtime.evaluate': {"code":-32000,"message":"Cannot find context with specified id"}"#.into(),
                )),
                3 => Ok(json!({ "result": { "value": json!({ "found": false }).to_string() } })),
                _ => Ok(json!({
                    "result": {
                        "value": json!({
                            "found": true,
                            "strategy": "test_id",
                            "tag": "button",
                            "is_sensitive": false,
                            "selector": "[data-testid=\"done\"]",
                            "center_x": 10.0,
                            "center_y": 20.0
                        }).to_string()
                    }
                })),
            }
        }
        async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
            Ok("mock://transient-wait".into())
        }
    }

    let attempts = Arc::new(AtomicUsize::new(0));
    let backend = CdpBrowserBackend::new_mock(Arc::new(TransientNavWaitTransport {
        attempts: Arc::clone(&attempts),
    }));
    let outcome = backend
        .wait_for_locator(&SemanticLocator::by_test_id("done"), 2_000)
        .await
        .expect("transient -32000 navigation errors must retry and recover");
    assert!(outcome.ok);
    assert_eq!(attempts.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn wait_for_locator_caps_consecutive_transient_cdp_errors_on_permanent_target_closure() {
    struct AlwaysTransientCdpTransport {
        attempts: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl CdpTransport for AlwaysTransientCdpTransport {
        async fn send_command(&self, method: &str, _params: Value) -> Result<Value, BotBrowserError> {
            if method == "Runtime.evaluate" {
                self.attempts.fetch_add(1, Ordering::SeqCst);
                return Err(BotBrowserError::Cdp(
                    r#"CDP error in 'Runtime.evaluate': {"code":-32000,"message":"Inspected target navigated or closed"}"#.into(),
                ));
            }
            Ok(json!({}))
        }
        async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
            Ok("mock://always-transient".into())
        }
    }

    let attempts = Arc::new(AtomicUsize::new(0));
    let backend = CdpBrowserBackend::new_mock(Arc::new(AlwaysTransientCdpTransport {
        attempts: Arc::clone(&attempts),
    }));
    let started = std::time::Instant::now();
    let err = backend
        .wait_for_locator(&SemanticLocator::by_test_id("done"), u64::MAX)
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert!(matches!(err, BotBrowserError::Cdp(_)), "got {err:?}");
    assert!(
        elapsed < Duration::from_millis(1_500),
        "permanent target closure with u64::MAX must cap retries quickly, took {elapsed:?}"
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 21);
}

#[tokio::test]
async fn wait_for_locator_fails_immediately_on_non_transient_errors() {
    enum Mode {
        Io,
        SessionMissing,
        TargetClosed,
        JsException,
    }
    struct ImmediateFailTransport {
        mode: Mode,
        attempts: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl CdpTransport for ImmediateFailTransport {
        async fn send_command(&self, method: &str, _params: Value) -> Result<Value, BotBrowserError> {
            if method != "Runtime.evaluate" {
                return Ok(json!({}));
            }
            self.attempts.fetch_add(1, Ordering::SeqCst);
            match self.mode {
                Mode::Io => Err(BotBrowserError::Io("websocket disconnected".into())),
                Mode::SessionMissing => Err(BotBrowserError::Cdp(
                    r#"CDP error in 'Runtime.evaluate': {"code":-32001,"message":"Session with given id not found."}"#.into(),
                )),
                Mode::TargetClosed => Err(BotBrowserError::Cdp(
                    r#"CDP error in 'Runtime.evaluate': {"code":-32000,"message":"Target closed"}"#.into(),
                )),
                Mode::JsException => Ok(json!({
                    "exceptionDetails": {
                        "exception": { "description": "ReferenceError: boom" }
                    }
                })),
            }
        }
        async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
            Ok("mock://immediate-fail".into())
        }
    }

    for mode in [
        Mode::Io,
        Mode::SessionMissing,
        Mode::TargetClosed,
        Mode::JsException,
    ] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let backend = CdpBrowserBackend::new_mock(Arc::new(ImmediateFailTransport {
            mode,
            attempts: Arc::clone(&attempts),
        }));
        let err = backend
            .wait_for_locator(&SemanticLocator::by_test_id("done"), 2_000)
            .await
            .unwrap_err();
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "non-transient error {err:?} must fail in 1 attempt without retrying"
        );
    }
}

#[tokio::test]
async fn wait_for_locator_respects_tight_timeout_during_transient_cdp_errors() {
    struct AlwaysTransientShortTransport;
    #[async_trait]
    impl CdpTransport for AlwaysTransientShortTransport {
        async fn send_command(&self, method: &str, _params: Value) -> Result<Value, BotBrowserError> {
            if method == "Runtime.evaluate" {
                return Err(BotBrowserError::Cdp(
                    r#"CDP error in 'Runtime.evaluate': {"code":-32000,"message":"Inspected target navigated or closed"}"#.into(),
                ));
            }
            Ok(json!({}))
        }
        async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
            Ok("mock://short-timeout".into())
        }
    }

    let backend = CdpBrowserBackend::new_mock(Arc::new(AlwaysTransientShortTransport));
    let started = std::time::Instant::now();
    let err = backend
        .wait_for_locator(&SemanticLocator::by_test_id("done"), 50)
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert!(matches!(err, BotBrowserError::Cdp(_)), "got {err:?}");
    assert!(
        elapsed < Duration::from_millis(300),
        "tight 50ms timeout must not be exceeded during transient -32000 retries, took {elapsed:?}"
    );
}
