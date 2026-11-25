use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bot_browser::backend::{
    ActionOutcome, BrowserSessionBackend, KeyInputEvent, NavigationResult, PointerInputEvent,
};
use bot_browser::cdp::CdpTransport;
use bot_browser::orchestrator::{
    BotBrowserOrchestrator, OrchestratorConfig, SessionTransportFactory,
};
use bot_browser::script_app::{
    AutomationScriptApp, BrowserControlMode, RecordedActionKind, ScriptAppMetadata, ScriptStep,
};
use bot_browser::types::{
    BotBrowserError, BotBrowserEvent, BrowserBackendKind, BrowserCookie, BrowserSnapshot,
    DomNodeSnapshot, NormalizedOrigin, SemanticLocator,
};
use bot_browser::vault::{CredentialVault, MemoryMasterKeyStore};
use gpui_telegram_agent::fabushi_bot::FabushiBotHost;
use gpui_telegram_agent::hermes::browser_session::{
    forward_events_to_mpsc, BrowserAutomationEvent, BrowserSessionConfig, BrowserSessionRegistry,
};
use gpui_telegram_agent::hermes::commands::CommandUiEffect;
use gpui_telegram_agent::hermes::providers::ProviderConfig;
use gpui_telegram_agent::hermes::toolsets::{
    all_builtin_tool_specs, HermesToolContext, HermesToolExecutor, ToolsetId, ToolsetPreset,
};
use gpui_telegram_agent::hermes::HermesTurnRequest;
use parking_lot::Mutex as SyncMutex;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};

// ============================================================================
// Local Loopback Stateful Web App Server (X-like Social + General Shop + 2FA)
// ============================================================================

#[derive(Default)]
struct MockWebSiteState {
    posts: Vec<String>,
    orders: Vec<String>,
    last_search_query: String,
    selected_item: String,
    customer_note: String,
    auth_verified: bool,
}

async fn spawn_mock_web_server() -> (String, Arc<SyncMutex<MockWebSiteState>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(SyncMutex::new(MockWebSiteState::default()));
    let state_srv = Arc::clone(&state);

    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let st = Arc::clone(&state_srv);
            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(8192);
                let mut tmp = [0u8; 4096];
                let mut content_len = 0usize;
                let mut header_end = None;

                loop {
                    let Ok(n) = sock.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if header_end.is_none() {
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                            let headers = String::from_utf8_lossy(&buf[..pos]);
                            for line in headers.lines() {
                                if let Some((k, v)) = line.split_once(':') {
                                    if k.trim().eq_ignore_ascii_case("content-length") {
                                        content_len = v.trim().parse::<usize>().unwrap_or(0);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(hend) = header_end {
                        if buf.len() >= hend + content_len {
                            break;
                        }
                    }
                }

                let raw = String::from_utf8_lossy(&buf);
                let first_line = raw.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("GET");
                let target = parts.next().unwrap_or("/");
                let path = target.split('?').next().unwrap_or("/");
                let body_str = header_end
                    .map(|h| String::from_utf8_lossy(&buf[h..]).to_string())
                    .unwrap_or_default();

                let resp_json = {
                    let mut guard = st.lock();
                    match (method, path) {
                        ("GET", "/x/home") => {
                            let latest = guard
                                .posts
                                .last()
                                .cloned()
                                .unwrap_or_else(|| "No posts yet".to_string());
                            json!({
                                "title": "X Home",
                                "text_excerpt": format!("X Timeline | Latest: {latest}"),
                                "elements": [
                                    { "ref_id": "compose_box", "tag": "textarea", "test_id": "tweetTextarea_0", "aria_label": "Post text", "selector": "[data-testid='tweetTextarea_0']", "text": "", "value": "", "is_password": false },
                                    { "ref_id": "post_btn", "tag": "button", "test_id": "tweetButton", "role": "button", "selector": "[data-testid='tweetButton']", "text": "Post", "value": null, "is_password": false },
                                    { "ref_id": "latest_post", "tag": "article", "test_id": "latest-post", "selector": "[data-testid='latest-post']", "text": latest, "value": null, "is_password": false }
                                ]
                            })
                        }
                        ("POST", "/x/post") => {
                            let parsed: Value =
                                serde_json::from_str(&body_str).unwrap_or_else(|_| json!({}));
                            let text = parsed["text"].as_str().unwrap_or("").to_string();
                            if !text.is_empty() {
                                guard.posts.push(text.clone());
                            }
                            json!({ "ok": true, "post": text, "count": guard.posts.len() })
                        }
                        ("GET", "/shop") => {
                            json!({
                                "title": "General Store",
                                "text_excerpt": format!("Search catalog | last={}", guard.last_search_query),
                                "elements": [
                                    { "ref_id": "search_q", "tag": "input", "name": "q", "selector": "input[name='q']", "text": "", "value": guard.last_search_query, "is_password": false },
                                    { "ref_id": "select_prod", "tag": "button", "test_id": "select-product", "selector": "[data-testid='select-product']", "text": "Ergonomic Keyboard", "value": null, "is_password": false }
                                ]
                            })
                        }
                        ("POST", "/shop/search") => {
                            let parsed: Value =
                                serde_json::from_str(&body_str).unwrap_or_else(|_| json!({}));
                            guard.last_search_query =
                                parsed["query"].as_str().unwrap_or("").to_string();
                            json!({ "ok": true, "query": guard.last_search_query })
                        }
                        ("GET", "/shop/checkout") => {
                            let latest_order = guard
                                .orders
                                .last()
                                .cloned()
                                .unwrap_or_else(|| "PENDING".to_string());
                            json!({
                                "title": "Checkout",
                                "text_excerpt": format!("Item: {} | Order: {}", guard.selected_item, latest_order),
                                "elements": [
                                    { "ref_id": "note", "tag": "input", "test_id": "customer-note", "selector": "[data-testid='customer-note']", "text": "", "value": guard.customer_note, "is_password": false },
                                    { "ref_id": "place", "tag": "button", "test_id": "place-order", "selector": "[data-testid='place-order']", "text": "Place Order", "value": null, "is_password": false },
                                    { "ref_id": "ord_id", "tag": "span", "test_id": "order-id", "selector": "[data-testid='order-id']", "text": latest_order, "value": null, "is_password": false }
                                ]
                            })
                        }
                        ("POST", "/shop/select") => {
                            guard.selected_item = "Ergonomic Keyboard".to_string();
                            json!({ "ok": true, "selected": guard.selected_item })
                        }
                        ("POST", "/shop/order") => {
                            let parsed: Value =
                                serde_json::from_str(&body_str).unwrap_or_else(|_| json!({}));
                            guard.customer_note =
                                parsed["note"].as_str().unwrap_or("").to_string();
                            let ord = format!("ORD-904{}", guard.orders.len() + 1);
                            guard.orders.push(ord.clone());
                            json!({ "ok": true, "order_id": ord })
                        }
                        ("GET", "/auth/gate") => {
                            let status = if guard.auth_verified {
                                "2FA Verified: Welcome Alice"
                            } else {
                                "Please complete 2FA verification"
                            };
                            json!({
                                "title": "Auth Gate",
                                "text_excerpt": status,
                                "elements": [
                                    { "ref_id": "status", "tag": "div", "test_id": "auth-status", "selector": "[data-testid='auth-status']", "text": status, "value": null, "is_password": false }
                                ]
                            })
                        }
                        ("GET", "/auth/dashboard") => {
                            let status = if guard.auth_verified {
                                "2FA Verified: Welcome Alice"
                            } else {
                                "Unverified"
                            };
                            json!({
                                "title": "Protected Dashboard",
                                "text_excerpt": status,
                                "elements": [
                                    { "ref_id": "dash_msg", "tag": "h1", "test_id": "dashboard-banner", "selector": "[data-testid='dashboard-banner']", "text": status, "value": null, "is_password": false }
                                ]
                            })
                        }
                        _ => json!({ "title": "Not Found", "text_excerpt": "404", "elements": [] }),
                    }
                };

                let body_bytes = serde_json::to_vec(&resp_json).unwrap();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body_bytes.len()
                );
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(&body_bytes).await;
            });
        }
    });

    (format!("http://{addr}"), state)
}

// ============================================================================
// Live HTTP-Backed BrowserSessionBackend (drives the real TcpListener server)
// ============================================================================

struct LiveHttpMockBackend {
    base_url: String,
    http: reqwest::Client,
    current_url: SyncMutex<String>,
    pending_inputs: SyncMutex<HashMap<String, String>>,
    scrolled: SyncMutex<Vec<(f64, f64)>>,
    keys_pressed: SyncMutex<Vec<String>>,
}

impl LiveHttpMockBackend {
    fn new(base_url: String, _site_state: Arc<SyncMutex<MockWebSiteState>>) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let initial = format!("{base_url}/x/home");
        Arc::new(Self {
            base_url,
            http,
            current_url: SyncMutex::new(initial),
            pending_inputs: SyncMutex::new(HashMap::new()),
            scrolled: SyncMutex::new(Vec::new()),
            keys_pressed: SyncMutex::new(Vec::new()),
        })
    }

    async fn fetch_page(&self) -> Result<Value, BotBrowserError> {
        let url = self.current_url.lock().clone();
        self.http
            .get(&url)
            .send()
            .await
            .map_err(|e| BotBrowserError::Cdp(e.to_string()))?
            .json::<Value>()
            .await
            .map_err(|e| BotBrowserError::Cdp(e.to_string()))
    }

    fn matches_node(loc: &SemanticLocator, node: &Value) -> bool {
        if let Some(ref tid) = loc.test_id {
            if node["test_id"].as_str() == Some(tid.as_str()) {
                return true;
            }
        }
        if let Some(ref al) = loc.aria_label {
            if node["aria_label"].as_str() == Some(al.as_str()) {
                return true;
            }
        }
        if let Some(ref name) = loc.name_attr {
            if node["name"].as_str() == Some(name.as_str()) {
                return true;
            }
        }
        if let Some(ref css) = loc.css_selector {
            if node["selector"].as_str() == Some(css.as_str()) {
                return true;
            }
            if let Some(tid) = node["test_id"].as_str() {
                if css.contains(tid) {
                    return true;
                }
            }
        }
        if let Some(ref role) = loc.role {
            if node["role"].as_str() == Some(role.as_str()) {
                if let Some(ref ta) = loc.text_anchor {
                    return node["text"].as_str().is_some_and(|t| t.contains(ta));
                }
                return true;
            }
        }
        if let Some(ref ta) = loc.text_anchor {
            if node["text"].as_str().is_some_and(|t| t.contains(ta)) {
                return true;
            }
        }
        false
    }
}

#[async_trait]
impl BrowserSessionBackend for LiveHttpMockBackend {
    fn backend_kind(&self) -> BrowserBackendKind {
        BrowserBackendKind::MockCdp
    }

    async fn navigate(&self, url: &str) -> Result<NavigationResult, BotBrowserError> {
        *self.current_url.lock() = url.to_string();
        let page = self.fetch_page().await?;
        let origin = NormalizedOrigin::parse(url)?.as_str().to_string();
        Ok(NavigationResult {
            url: url.to_string(),
            origin,
            title: page["title"].as_str().unwrap_or("").to_string(),
            duration_ms: 2,
            backend: BrowserBackendKind::MockCdp,
        })
    }

    async fn go_back(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }

    async fn go_forward(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }

    async fn reload(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }

    async fn current_url(&self) -> Result<String, BotBrowserError> {
        Ok(self.current_url.lock().clone())
    }

    async fn page_title(&self) -> Result<String, BotBrowserError> {
        let page = self.fetch_page().await?;
        Ok(page["title"].as_str().unwrap_or("").to_string())
    }

    async fn snapshot_dom(&self) -> Result<BrowserSnapshot, BotBrowserError> {
        let url = self.current_url.lock().clone();
        let origin = NormalizedOrigin::parse(&url)
            .map(|o| o.as_str().to_string())
            .unwrap_or_else(|_| url.clone());
        let page = self.fetch_page().await?;
        let elements = page["elements"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|el| DomNodeSnapshot {
                ref_id: el["ref_id"].as_str().unwrap_or("").to_string(),
                tag: el["tag"].as_str().unwrap_or("div").to_string(),
                input_type: None,
                name: el["name"].as_str().map(ToString::to_string),
                id: None,
                aria_label: el["aria_label"].as_str().map(ToString::to_string),
                placeholder: None,
                test_id: el["test_id"].as_str().map(ToString::to_string),
                role: el["role"].as_str().map(ToString::to_string),
                text: el["text"].as_str().unwrap_or("").to_string(),
                value: el["value"].as_str().map(ToString::to_string),
                is_password: el["is_password"].as_bool().unwrap_or(false),
                selector: el["selector"].as_str().unwrap_or("").to_string(),
                frame_origin: Some(origin.clone()),
                rect: None,
            })
            .collect();
        Ok(BrowserSnapshot {
            url,
            origin,
            title: page["title"].as_str().unwrap_or("").to_string(),
            text_excerpt: page["text_excerpt"].as_str().unwrap_or("").to_string(),
            interactive_elements: elements,
            redacted_field_count: 0,
        })
    }

    async fn eval_js(&self, _script: &str) -> Result<Value, BotBrowserError> {
        Ok(Value::Null)
    }

    async fn click_locator(&self, locator: &SemanticLocator) -> Result<ActionOutcome, BotBrowserError> {
        let key = locator.describe();
        if key.contains("tweetButton") {
            let text = self
                .pending_inputs
                .lock()
                .get("tweetTextarea_0")
                .cloned()
                .unwrap_or_default();
            let _ = self
                .http
                .post(format!("{}/x/post", self.base_url))
                .json(&json!({ "text": text }))
                .send()
                .await
                .map_err(|e| BotBrowserError::Cdp(e.to_string()))?;
        } else if key.contains("select-product") {
            let _ = self
                .http
                .post(format!("{}/shop/select", self.base_url))
                .json(&json!({}))
                .send()
                .await
                .map_err(|e| BotBrowserError::Cdp(e.to_string()))?;
            *self.current_url.lock() = format!("{}/shop/checkout", self.base_url);
        } else if key.contains("place-order") {
            let note = self
                .pending_inputs
                .lock()
                .get("customer-note")
                .cloned()
                .unwrap_or_default();
            let _ = self
                .http
                .post(format!("{}/shop/order", self.base_url))
                .json(&json!({ "note": note }))
                .send()
                .await
                .map_err(|e| BotBrowserError::Cdp(e.to_string()))?;
        } else {
            return Err(BotBrowserError::LocatorNotFound(locator.describe()));
        }

        Ok(ActionOutcome {
            ok: true,
            operation: "click".to_string(),
            duration_ms: 2,
            matched_strategy: Some("semantic".to_string()),
            matched_selector: Some(locator.describe()),
            x: Some(120.0),
            y: Some(200.0),
            extracted_text: None,
        })
    }

    async fn fill_locator(
        &self,
        locator: &SemanticLocator,
        value: &str,
        _is_secret: bool,
    ) -> Result<ActionOutcome, BotBrowserError> {
        let key = if let Some(ref tid) = locator.test_id {
            tid.clone()
        } else if let Some(ref n) = locator.name_attr {
            n.clone()
        } else {
            locator.describe()
        };
        self.pending_inputs.lock().insert(key.clone(), value.to_string());
        if key == "q" {
            let _ = self
                .http
                .post(format!("{}/shop/search", self.base_url))
                .json(&json!({ "query": value }))
                .send()
                .await
                .map_err(|e| BotBrowserError::Cdp(e.to_string()))?;
        }
        Ok(ActionOutcome {
            ok: true,
            operation: "fill".to_string(),
            duration_ms: 2,
            matched_strategy: Some("semantic".to_string()),
            matched_selector: Some(locator.describe()),
            x: None,
            y: None,
            extracted_text: None,
        })
    }

    async fn press_key(&self, key: &str) -> Result<ActionOutcome, BotBrowserError> {
        self.keys_pressed.lock().push(key.to_string());
        Ok(ActionOutcome {
            ok: true,
            operation: "press_key".to_string(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }

    async fn scroll(&self, delta_x: f64, delta_y: f64) -> Result<ActionOutcome, BotBrowserError> {
        self.scrolled.lock().push((delta_x, delta_y));
        Ok(ActionOutcome {
            ok: true,
            operation: "scroll".to_string(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }

    async fn wait_for_locator(
        &self,
        locator: &SemanticLocator,
        _timeout_ms: u64,
    ) -> Result<ActionOutcome, BotBrowserError> {
        let page = self.fetch_page().await?;
        let found = page["elements"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|n| Self::matches_node(locator, n)));
        if !found {
            return Err(BotBrowserError::LocatorNotFound(locator.describe()));
        }
        Ok(ActionOutcome {
            ok: true,
            operation: "wait_for".to_string(),
            duration_ms: 1,
            matched_strategy: Some("semantic".to_string()),
            matched_selector: Some(locator.describe()),
            x: None,
            y: None,
            extracted_text: None,
        })
    }

    async fn extract_text_locator(
        &self,
        locator: &SemanticLocator,
    ) -> Result<String, BotBrowserError> {
        let page = self.fetch_page().await?;
        let found = page["elements"]
            .as_array()
            .and_then(|arr| arr.iter().find(|n| Self::matches_node(locator, n)))
            .and_then(|n| n["text"].as_str())
            .map(ToString::to_string)
            .ok_or_else(|| BotBrowserError::LocatorNotFound(locator.describe()))?;
        Ok(found)
    }

    async fn capture_screenshot(&self) -> Result<Vec<u8>, BotBrowserError> {
        Ok(vec![137, 80, 78, 71, 13, 10, 26, 10])
    }

    async fn export_cookies(&self, _domain_filter: Option<&str>) -> Result<Vec<BrowserCookie>, BotBrowserError> {
        Ok(Vec::new())
    }

    async fn import_cookies(&self, _cookies: &[BrowserCookie]) -> Result<usize, BotBrowserError> {
        Ok(0)
    }

    async fn dispatch_pointer_event(&self, _event: PointerInputEvent) -> Result<(), BotBrowserError> {
        Ok(())
    }

    async fn dispatch_key_event(&self, _event: KeyInputEvent) -> Result<(), BotBrowserError> {
        Ok(())
    }
}

// ============================================================================
// Loopback OpenAI-Compatible Mock LLM Server
// ============================================================================

async fn spawn_mock_openai_server(
    responses: Vec<Value>,
) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let queue = Arc::new(SyncMutex::new(VecDeque::from(responses)));
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_srv = Arc::clone(&call_count);

    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let q = Arc::clone(&queue);
            let cnt = Arc::clone(&call_count_srv);
            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(8192);
                let mut tmp = [0u8; 4096];
                let mut content_len = 0usize;
                let mut header_end = None;
                loop {
                    let Ok(n) = sock.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if header_end.is_none() {
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                            let headers = String::from_utf8_lossy(&buf[..pos]);
                            for line in headers.lines() {
                                if let Some((k, v)) = line.split_once(':') {
                                    if k.trim().eq_ignore_ascii_case("content-length") {
                                        content_len = v.trim().parse::<usize>().unwrap_or(0);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(hend) = header_end {
                        if buf.len() >= hend + content_len {
                            break;
                        }
                    }
                }

                cnt.fetch_add(1, Ordering::SeqCst);
                let next_msg = q.lock().pop_front().unwrap_or_else(|| {
                    json!({
                        "id": "chatcmpl-end",
                        "choices": [{
                            "message": { "role": "assistant", "content": "Completed." },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
                    })
                });

                let body = serde_json::to_vec(&next_msg).unwrap();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            });
        }
    });

    (format!("http://{addr}/v1"), call_count)
}

fn full_turn_req(prompt: &str, session_id: &str) -> HermesTurnRequest {
    let mut req = HermesTurnRequest::new(prompt).with_session(session_id);
    req.preset = Some(ToolsetPreset::Full);
    req
}

fn assistant_tool_call_resp(call_id: &str, name: &str, args: Value) -> Value {
    json!({
        "id": format!("chatcmpl-{call_id}"),
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&args).unwrap()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 20, "completion_tokens": 12, "total_tokens": 32 }
    })
}

fn assistant_text_resp(text: &str) -> Value {
    json!({
        "id": "chatcmpl-final",
        "choices": [{
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 15, "completion_tokens": 8, "total_tokens": 23 }
    })
}

struct CountingTransportFactory {
    creations: Arc<AtomicUsize>,
}

#[async_trait]
impl SessionTransportFactory for CountingTransportFactory {
    async fn create_transport(
        &self,
        _session_id: &str,
        _data_dir: &std::path::Path,
        _origin_hint: Option<&NormalizedOrigin>,
        _event_tx: &broadcast::Sender<BotBrowserEvent>,
    ) -> Result<Arc<dyn CdpTransport>, BotBrowserError> {
        self.creations.fetch_add(1, Ordering::SeqCst);
        Err(BotBrowserError::Cdp("Should not spawn duplicate tab when registered".into()))
    }
}

struct StubCdpTransport;

#[async_trait]
impl CdpTransport for StubCdpTransport {
    async fn send_command(&self, method: &str, _params: Value) -> Result<Value, BotBrowserError> {
        match method {
            "Page.navigate" => Ok(json!({ "frameId": "main" })),
            "Runtime.evaluate" => Ok(json!({ "result": { "value": "ok" } })),
            _ => Ok(json!({})),
        }
    }

    async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
        Ok("http://127.0.0.1:0".to_string())
    }
}

struct StubTransportFactory {
    creations: Arc<AtomicUsize>,
}

#[async_trait]
impl SessionTransportFactory for StubTransportFactory {
    async fn create_transport(
        &self,
        _session_id: &str,
        _data_dir: &std::path::Path,
        _origin_hint: Option<&NormalizedOrigin>,
        _event_tx: &broadcast::Sender<BotBrowserEvent>,
    ) -> Result<Arc<dyn CdpTransport>, BotBrowserError> {
        self.creations.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(StubCdpTransport))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_m3_x_social_and_general_web_workflows_via_hermes_and_fabushi_host() {
    let dir = TempDir::new().unwrap();
    let (base_url, site_state) = spawn_mock_web_server().await;

    let host = FabushiBotHost::new_unified_ephemeral("mock-gpt").unwrap();

    // Attach a BotBrowserOrchestrator whose transport factory errors if a duplicate tab is spawned!
    let vault = Arc::new(
        CredentialVault::open(dir.path().join("vault.db"), &MemoryMasterKeyStore::new()).unwrap(),
    );
    let spawn_count = Arc::new(AtomicUsize::new(0));
    let orch = Arc::new(BotBrowserOrchestrator::with_transport_factory(
        OrchestratorConfig::new(dir.path()),
        vault,
        Arc::new(CountingTransportFactory {
            creations: Arc::clone(&spawn_count),
        }),
    ));
    host.set_bot_browser(orch);

    // Register active tab into BrowserSessionRegistry (and set as active fallback tab)
    let tab_id = "in_app_tab_main";
    let backend = LiveHttpMockBackend::new(base_url.clone(), Arc::clone(&site_state));
    let session = host
        .browser_registry()
        .register_active_backend(tab_id, Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>);
    assert_eq!(host.browser_registry().list_session_ids(), vec![tab_id.to_string()]);

    // Subscribe to both broadcast and mpsc forwarder helper
    let rx_broadcast = host.subscribe_browser_events();
    let (mpsc_tx, mut mpsc_rx) = mpsc::channel(128);
    let _fwd_handle = forward_events_to_mpsc(rx_broadcast, mpsc_tx);

    // Prepare mock OpenAI LLM sequence for Workflow 1 (X-like social) + Workflow 2 (General shop)
    let llm_steps = vec![
        // Workflow 1: X-like social post
        assistant_tool_call_resp(
            "c1",
            "browser_navigate",
            json!({ "url": format!("{base_url}/x/home?auth_token=secret_x_tok_999") }),
        ),
        assistant_tool_call_resp(
            "c2",
            "browser_wait_for",
            json!({ "test_id": "tweetTextarea_0", "timeout_ms": 2000 }),
        ),
        assistant_tool_call_resp(
            "c3",
            "browser_type",
            json!({
                "test_id": "tweetTextarea_0",
                "text": "Pure-Rust browser automation live!",
                "parameter_name_hint": "post_body"
            }),
        ),
        assistant_tool_call_resp(
            "c4",
            "browser_click",
            json!({ "test_id": "tweetButton" }),
        ),
        assistant_tool_call_resp(
            "c5",
            "browser_extract_text",
            json!({ "test_id": "latest-post", "output_key": "verified_post" }),
        ),
        assistant_text_resp("Posted to X and verified: Pure-Rust browser automation live!"),
        // Workflow 2: General e-commerce / order flow
        assistant_tool_call_resp(
            "g1",
            "browser_navigate",
            json!({ "url": format!("{base_url}/shop") }),
        ),
        assistant_tool_call_resp(
            "g2",
            "browser_type",
            json!({ "name_attr": "q", "text": "ergonomic keyboard", "submit": true }),
        ),
        assistant_tool_call_resp(
            "g3",
            "browser_scroll",
            json!({ "delta_x": 0.0, "delta_y": 240.0 }),
        ),
        assistant_tool_call_resp(
            "g4",
            "browser_click",
            json!({ "test_id": "select-product" }),
        ),
        assistant_tool_call_resp(
            "g5",
            "browser_type",
            json!({ "test_id": "customer-note", "text": "Deliver to desk 4B" }),
        ),
        assistant_tool_call_resp(
            "g6",
            "browser_click",
            json!({ "test_id": "place-order" }),
        ),
        assistant_tool_call_resp(
            "g7",
            "browser_extract_text",
            json!({ "test_id": "order-id", "output_key": "order_id" }),
        ),
        assistant_text_resp("Order placed: ORD-9041"),
    ];
    let (llm_url, _) = spawn_mock_openai_server(llm_steps).await;

    let engine = host.core_engine();
    engine.providers().register_provider(ProviderConfig::openai_compatible(
        "mock_openai",
        "Mock OpenAI",
        llm_url,
        "mock-gpt",
        "sk-mock-test",
    ));
    engine
        .providers()
        .set_active("mock_openai", Some("mock-gpt"))
        .unwrap();

    // Run Workflow 1 on a chat session_id != tab_id (verifies active tab fallback & zero duplicate tabs!)
    let res_x = engine
        .run_turn(full_turn_req("Publish post on X", "dm_chat_1"))
        .await
        .expect("X workflow turn succeeds");
    assert!(res_x.response_text.contains("Pure-Rust browser automation live!"));
    assert_eq!(site_state.lock().posts, vec!["Pure-Rust browser automation live!".to_string()]);
    assert_eq!(spawn_count.load(Ordering::SeqCst), 0, "must NOT spawn duplicate tab");
    assert_eq!(host.browser_registry().list_session_ids().len(), 1);

    // Verify trace redaction for Workflow 1 (`secret_x_tok_999` must NEVER appear in trace)
    let trace_x = session.last_completed_trace().expect("X trace recorded");
    let trace_x_json = serde_json::to_string(&trace_x).unwrap();
    assert!(!trace_x_json.contains("secret_x_tok_999"));
    assert!(trace_x_json.contains("[REDACTED]"));
    assert!(trace_x.actions.len() >= 5);

    // Run Workflow 2 explicitly targeting tab_id
    let res_shop = engine
        .run_turn(full_turn_req("Search keyboard and place order", tab_id))
        .await
        .expect("Shop workflow turn succeeds");
    assert!(res_shop.response_text.contains("ORD-9041"));
    assert_eq!(site_state.lock().orders, vec!["ORD-9041".to_string()]);
    assert_eq!(site_state.lock().last_search_query, "ergonomic keyboard");
    assert_eq!(spawn_count.load(Ordering::SeqCst), 0);

    // Verify telemetry event ordering over mpsc forwarder
    let mut events = Vec::new();
    while let Ok(ev) = mpsc_rx.try_recv() {
        events.push(ev);
    }
    assert!(events
        .iter()
        .any(|e| matches!(e, BrowserAutomationEvent::StepStarted { operation, .. } if operation == "browser_navigate")));
    assert!(events
        .iter()
        .any(|e| matches!(e, BrowserAutomationEvent::StepCompleted { operation, .. } if operation == "browser_extract_text")));
    assert!(events
        .iter()
        .any(|e| matches!(e, BrowserAutomationEvent::ControlModeChanged { mode: BrowserControlMode::AiControlling { .. }, .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, BrowserAutomationEvent::TraceCompleted { .. })));
}

#[tokio::test]
async fn test_m3_per_session_concurrency_lock_and_step_budget_termination() {
    let (base_url, site_state) = spawn_mock_web_server().await;
    let reg = Arc::new(BrowserSessionRegistry::with_config(BrowserSessionConfig {
        max_steps_per_run: 2,
        auto_pause_on_sensitive_origin: false,
        event_channel_capacity: 64,
    }));
    let backend = LiveHttpMockBackend::new(base_url.clone(), site_state);
    let sess = reg.register_backend("locked_tab", Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>);

    let host = FabushiBotHost::new_unified_ephemeral("mock-gpt").unwrap();
    host.set_browser_registry(Arc::clone(&reg));
    let engine = host.core_engine();

    // 1. Hold AI lock on `locked_tab` and verify overlapping AI turn AND replay are rejected!
    let mut outer_guard = sess.try_begin_ai_turn().expect("outer lock acquires");
    let (llm_url_1, _) = spawn_mock_openai_server(vec![
        assistant_tool_call_resp("lock_c1", "browser_navigate", json!({ "url": format!("{base_url}/x/home") })),
        assistant_text_resp("Should report locked error"),
    ])
    .await;
    engine.providers().register_provider(ProviderConfig::openai_compatible(
        "mock_lock",
        "Mock Lock",
        llm_url_1,
        "mock-gpt",
        "sk-mock",
    ));
    engine
        .providers()
        .set_active("mock_lock", Some("mock-gpt"))
        .unwrap();

    let locked_res = engine
        .run_turn(full_turn_req("Navigate while locked", "locked_tab"))
        .await
        .unwrap();
    assert_eq!(locked_res.tool_calls.len(), 1);
    assert!(!locked_res.tool_calls[0].success);
    assert!(locked_res.tool_calls[0].output.contains("locked"));

    // Overlapping replay is also rejected
    let dummy_app = AutomationScriptApp {
        schema_version: 1,
        app_id: "app_1".into(),
        slug: "app-1".into(),
        name: "App".into(),
        version: "1.0.0".into(),
        description: "".into(),
        category: "automation".into(),
        target_domains: vec![],
        parameters: vec![],
        steps: vec![ScriptStep::Scroll {
            delta_x: 0.0,
            delta_y: 10.0,
        }],
        required_capabilities: vec![],
        author_id: None,
        source_trace_summary: None,
    };
    let replay_err = host
        .replay_browser_script("locked_tab", &dummy_app, &BTreeMap::new(), None)
        .await
        .unwrap_err();
    assert!(replay_err.to_string().contains("locked"));

    outer_guard.finish();
    drop(outer_guard);

    // 2. Step-budget exhaustion: `max_steps_per_run = 2`, AI attempts 3 steps -> step 3 fails deterministically
    let mut ev_rx = host.subscribe_browser_events();
    let (llm_url_2, _) = spawn_mock_openai_server(vec![
        assistant_tool_call_resp("b1", "browser_navigate", json!({ "url": format!("{base_url}/shop") })),
        assistant_tool_call_resp("b2", "browser_scroll", json!({ "delta_x": 0.0, "delta_y": 50.0 })),
        assistant_tool_call_resp("b3", "browser_scroll", json!({ "delta_x": 0.0, "delta_y": 50.0 })),
        assistant_text_resp("Halted after budget"),
    ])
    .await;
    engine.providers().register_provider(ProviderConfig::openai_compatible(
        "mock_budget",
        "Mock Budget",
        llm_url_2,
        "mock-gpt",
        "sk-mock",
    ));
    engine
        .providers()
        .set_active("mock_budget", Some("mock-gpt"))
        .unwrap();

    let budget_res = engine
        .run_turn(full_turn_req("Run 3 steps", "locked_tab"))
        .await
        .unwrap();
    assert_eq!(budget_res.tool_calls.len(), 3);
    assert!(budget_res.tool_calls[0].success);
    assert!(budget_res.tool_calls[1].success);
    assert!(!budget_res.tool_calls[2].success);
    assert!(budget_res.tool_calls[2].output.contains("budget exhausted"));
    assert_eq!(sess.control_mode(), BrowserControlMode::Idle);

    let mut saw_step_failed = false;
    while let Ok(ev) = ev_rx.try_recv() {
        if let BrowserAutomationEvent::StepFailed { error, .. } = ev {
            if error.contains("budget exhausted") {
                saw_step_failed = true;
            }
        }
    }
    assert!(saw_step_failed, "expected StepFailed event on budget exhaustion");
}

#[tokio::test]
async fn test_m3_takeover_suspend_resume_refresh_and_cancel_abort() {
    let (base_url, site_state) = spawn_mock_web_server().await;
    let host = FabushiBotHost::new_unified_ephemeral("mock-gpt").unwrap();
    let backend = LiveHttpMockBackend::new(base_url.clone(), Arc::clone(&site_state));
    let sess = host.register_browser_session(
        "takeover_tab",
        Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>,
    );
    let mut ev_rx = host.subscribe_browser_events();

    let (llm_url, _) = spawn_mock_openai_server(vec![
        assistant_tool_call_resp("t1", "browser_navigate", json!({ "url": format!("{base_url}/auth/gate") })),
        assistant_tool_call_resp(
            "t2",
            "browser_request_takeover",
            json!({ "reason": "Complete 2FA verification", "wait_for_finish": true }),
        ),
        assistant_tool_call_resp(
            "t3",
            "browser_extract_text",
            json!({ "test_id": "dashboard-banner", "output_key": "banner" }),
        ),
        assistant_text_resp("Verified after takeover: 2FA Verified: Welcome Alice"),
    ])
    .await;

    let engine = host.core_engine();
    engine.providers().register_provider(ProviderConfig::openai_compatible(
        "mock_takeover",
        "Mock Takeover",
        llm_url,
        "mock-gpt",
        "sk-mock",
    ));
    engine
        .providers()
        .set_active("mock_takeover", Some("mock-gpt"))
        .unwrap();

    let engine_task = Arc::clone(&engine);
    let turn_handle = tokio::spawn(async move {
        engine_task
            .run_turn(full_turn_req("Sign into protected site", "takeover_tab"))
            .await
    });

    // Wait for TakeoverRequested / WaitingForTakeover
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), ev_rx.recv())
            .await
            .expect("timely takeover event")
            .unwrap();
        if matches!(ev, BrowserAutomationEvent::TakeoverRequested { .. }) {
            break;
        }
    }
    assert!(matches!(
        sess.control_mode(),
        BrowserControlMode::WaitingForTakeover { .. }
    ));

    // Confirm Notify suspension does not advance prematurely before user_resume_ai()
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(!turn_handle.is_finished(), "AI turn must remain suspended on Notify");

    // User takes over manually (must NOT deadlock on ai_lock!) and modifies page state
    sess.user_take_over();
    assert!(matches!(
        sess.control_mode(),
        BrowserControlMode::UserTakeover { .. }
    ));
    site_state.lock().auth_verified = true;
    backend
        .navigate(&format!("{base_url}/auth/dashboard"))
        .await
        .unwrap();

    // User hands control back to AI
    sess.user_resume_ai();

    let turn_res = turn_handle
        .await
        .unwrap()
        .expect("resumed AI turn finishes cleanly");
    assert_eq!(turn_res.tool_calls.len(), 3);
    // Verify takeover tool returned refreshed live URL and DOM snapshot
    let takeover_out = &turn_res.tool_calls[1].output;
    assert!(takeover_out.contains("/auth/dashboard"));
    assert!(turn_res.tool_calls[2]
        .output
        .contains("2FA Verified: Welcome Alice"));

    // Verify HumanCheckpoint recorded in BrowserActionTrace
    let trace = sess.last_completed_trace().expect("trace completed");
    assert!(trace
        .actions
        .iter()
        .any(|a| matches!(a.action, RecordedActionKind::HumanCheckpoint { .. })));

    // Now test user_cancel_ai() mid-takeover -> clean ControlCancelled abort, followed by new turn reset!
    let (llm_url_cancel, _) = spawn_mock_openai_server(vec![
        assistant_tool_call_resp(
            "tc1",
            "browser_request_takeover",
            json!({ "reason": "Waiting for captcha" }),
        ),
        assistant_text_resp("Cancelled"),
    ])
    .await;
    engine.providers().register_provider(ProviderConfig::openai_compatible(
        "mock_cancel",
        "Mock Cancel",
        llm_url_cancel,
        "mock-gpt",
        "sk-mock",
    ));
    engine
        .providers()
        .set_active("mock_cancel", Some("mock-gpt"))
        .unwrap();

    let engine_cancel = Arc::clone(&engine);
    let cancel_handle = tokio::spawn(async move {
        engine_cancel
            .run_turn(full_turn_req("Trigger takeover then cancel", "takeover_tab"))
            .await
    });

    loop {
        if matches!(
            sess.control_mode(),
            BrowserControlMode::WaitingForTakeover { .. }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    sess.user_cancel_ai();

    let cancelled_res = cancel_handle.await.unwrap().unwrap();
    assert!(!cancelled_res.tool_calls[0].success);
    assert!(cancelled_res.tool_calls[0]
        .output
        .to_ascii_lowercase()
        .contains("cancelled"));
    assert_eq!(sess.control_mode(), BrowserControlMode::Idle);

    // Subsequent turn on the same gate succeeds because try_begin_ai_turn calls gate.reset()
    let (llm_url_after, _) = spawn_mock_openai_server(vec![
        assistant_tool_call_resp(
            "ta1",
            "browser_navigate",
            json!({ "url": format!("{base_url}/auth/dashboard") }),
        ),
        assistant_text_resp("Recovered after cancel"),
    ])
    .await;
    engine.providers().register_provider(ProviderConfig::openai_compatible(
        "mock_after_cancel",
        "Mock After Cancel",
        llm_url_after,
        "mock-gpt",
        "sk-mock",
    ));
    engine
        .providers()
        .set_active("mock_after_cancel", Some("mock-gpt"))
        .unwrap();
    let recovered = engine
        .run_turn(full_turn_req("Navigate after earlier cancel", "takeover_tab"))
        .await
        .unwrap();
    assert!(recovered.tool_calls[0].success);
}

#[tokio::test]
async fn test_m3_distill_slash_command_tool_api_and_zero_llm_replay() {
    let (base_url, site_state) = spawn_mock_web_server().await;
    let host = FabushiBotHost::new_unified_ephemeral("mock-gpt").unwrap();
    let backend = LiveHttpMockBackend::new(base_url.clone(), Arc::clone(&site_state));
    let tab_id = "distill_tab";
    let sess = host.register_browser_session(
        tab_id,
        Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>,
    );
    let mut ev_rx = host.subscribe_browser_events();

    // Drive a multi-step X workflow via AI, then distill via `distill_browser_script_app` tool
    let (llm_url, llm_calls) = spawn_mock_openai_server(vec![
        assistant_tool_call_resp(
            "d1",
            "browser_navigate",
            json!({ "url": format!("{base_url}/x/home?api_key=sk-live-secret-should-redact") }),
        ),
        assistant_tool_call_resp(
            "d2",
            "browser_wait_for",
            json!({ "test_id": "tweetTextarea_0" }),
        ),
        assistant_tool_call_resp(
            "d3",
            "browser_type",
            json!({
                "test_id": "tweetTextarea_0",
                "text": "Automated announcement #1",
                "parameter_name_hint": "post_text"
            }),
        ),
        assistant_tool_call_resp("d4", "browser_click", json!({ "test_id": "tweetButton" })),
        assistant_tool_call_resp(
            "d5",
            "browser_extract_text",
            json!({ "test_id": "latest-post", "output_key": "latest_post" }),
        ),
        assistant_tool_call_resp(
            "d6",
            "distill_browser_script_app",
            json!({ "name": "X Auto Poster", "description": "Posts updates on X", "category": "social" }),
        ),
        assistant_text_resp("Distilled X Auto Poster."),
    ])
    .await;

    let engine = host.core_engine();
    engine.providers().register_provider(ProviderConfig::openai_compatible(
        "mock_distill",
        "Mock Distill",
        llm_url,
        "mock-gpt",
        "sk-mock",
    ));
    engine
        .providers()
        .set_active("mock_distill", Some("mock-gpt"))
        .unwrap();

    let ai_turn = engine
        .run_turn(full_turn_req("Post to X and distill script", tab_id))
        .await
        .unwrap();
    assert!(ai_turn.tool_calls.iter().all(|t| t.success));
    let calls_before_replay = llm_calls.load(Ordering::SeqCst);

    // Also test `/distill` slash command via HermesEngine::run_turn
    let cmd_res = engine
        .run_turn(HermesTurnRequest::new("/distill X Slash Distilled").with_session(tab_id))
        .await
        .unwrap();
    assert!(
        cmd_res
            .ui_effects
            .iter()
            .any(|e| matches!(e, CommandUiEffect::DistillBrowserScript { name, .. } if name == "X Slash Distilled")),
        "expected DistillBrowserScript UI effect, got {:?}",
        cmd_res.ui_effects
    );

    // Also test public Rust API distillation on FabushiBotHost
    let compiled_app = host
        .distill_browser_script(
            tab_id,
            ScriptAppMetadata {
                name: "X Public API App".into(),
                category: "social".into(),
                ..Default::default()
            },
        )
        .expect("public distill_browser_script succeeds");

    let serialized_app = serde_json::to_string(&compiled_app).unwrap();
    assert!(!serialized_app.contains("sk-live-secret-should-redact"));
    assert!(
        compiled_app
            .parameters
            .iter()
            .any(|p| p.name == "api_key" && p.is_secret),
        "expected redacted URL query param to compile into secret ScriptParameter: {:?}",
        compiled_app.parameters
    );

    // Verify ScriptDistilled event was emitted
    let mut distilled_events = 0usize;
    while let Ok(ev) = ev_rx.try_recv() {
        if matches!(ev, BrowserAutomationEvent::ScriptDistilled { .. }) {
            distilled_events += 1;
        }
    }
    assert!(distilled_events >= 3);

    // Clear posts on the mock server, then replay deterministically with ZERO LLM calls!
    site_state.lock().posts.clear();
    let mut params = BTreeMap::new();
    params.insert("api_key".to_string(), "replay-ephemeral-token".to_string());
    params.insert("post_text".to_string(), "Replayed without LLM!".to_string());

    let report = host
        .replay_browser_script(tab_id, &compiled_app, &params, None)
        .await
        .expect("deterministic replay succeeds");
    assert!(report.success, "replay failed: {:?}", report.error);
    assert!(!report.used_llm, "replay MUST NOT use LLM");
    assert_eq!(
        llm_calls.load(Ordering::SeqCst),
        calls_before_replay,
        "LLM server must receive zero requests during replay"
    );
    assert_eq!(
        site_state.lock().posts,
        vec!["Replayed without LLM!".to_string()]
    );
    assert_eq!(
        report.extracted_outputs.get("latest_post").map(String::as_str),
        Some("Replayed without LLM!")
    );
    assert_eq!(sess.control_mode(), BrowserControlMode::Idle);
}

#[tokio::test]
async fn test_m3_orchestrator_backward_compat_and_zero_chrome_invariants() {
    assert_eq!(ToolsetId::all().len(), 29);
    for spec in all_builtin_tool_specs()
        .into_iter()
        .filter(|s| s.toolset == ToolsetId::Browser)
    {
        assert!(
            !spec.description.to_ascii_lowercase().contains("chrome"),
            "Browser tool '{}' mentions chrome: {}",
            spec.name,
            spec.description
        );
    }

    let dir = TempDir::new().unwrap();
    let vault = Arc::new(
        CredentialVault::open(dir.path().join("vault.db"), &MemoryMasterKeyStore::new()).unwrap(),
    );
    let creations = Arc::new(AtomicUsize::new(0));
    let orch = Arc::new(BotBrowserOrchestrator::with_transport_factory(
        OrchestratorConfig::new(dir.path()),
        vault,
        Arc::new(StubTransportFactory {
            creations: Arc::clone(&creations),
        }),
    ));

    let registry = Arc::new(BrowserSessionRegistry::new());
    let ctx = HermesToolContext::new(dir.path())
        .with_session("orch_sess_1", 1)
        .with_bot_browser(orch)
        .with_browser_sessions(Arc::clone(&registry))
        .with_browser_turn_scope();

    let r1 = HermesToolExecutor::execute(
        "browser_navigate",
        json!({ "url": "https://portal.example.com/login" }),
        &ctx,
    )
    .await;
    assert!(r1.success, "{}", r1.output);

    let r2 = HermesToolExecutor::execute("browser_press_key", json!({ "key": "Tab" }), &ctx).await;
    assert!(r2.success, "{}", r2.output);

    let r3 = HermesToolExecutor::execute(
        "browser_scroll",
        json!({ "delta_x": 0.0, "delta_y": 150.0 }),
        &ctx,
    )
    .await;
    assert!(r3.success, "{}", r3.output);

    // Lazy orchestrator backend created exactly ONCE for `orch_sess_1`
    assert_eq!(creations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_m3_external_takeover_between_steps_refreshes_live_dom() {
    let (base_url, site_state) = spawn_mock_web_server().await;
    let host = FabushiBotHost::new_unified_ephemeral("mock-gpt").unwrap();
    let backend = LiveHttpMockBackend::new(base_url.clone(), Arc::clone(&site_state));
    let sess = host.register_browser_session(
        "ext_takeover_tab",
        Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>,
    );

    let dir = TempDir::new().unwrap();
    let ctx = HermesToolContext::new(dir.path())
        .with_session("ext_takeover_tab", 1)
        .with_browser_sessions(host.browser_registry())
        .with_browser_turn_scope();

    // Step 1: AI navigates to /auth/gate (AiControlling begins)
    let step1 = HermesToolExecutor::execute(
        "browser_navigate",
        json!({ "url": format!("{base_url}/auth/gate") }),
        &ctx,
    )
    .await;
    assert!(step1.success, "{}", step1.output);
    assert!(matches!(
        sess.control_mode(),
        BrowserControlMode::AiControlling { .. }
    ));

    // User externally clicks "Take Over" between steps!
    sess.user_take_over();
    assert!(matches!(
        sess.control_mode(),
        BrowserControlMode::UserTakeover { .. }
    ));

    // Next AI step starts and suspends cleanly on Notify without busy-wait
    let ctx_step2 = ctx.clone();
    let step2_handle = tokio::spawn(async move {
        HermesToolExecutor::execute(
            "browser_extract_text",
            json!({ "test_id": "dashboard-banner", "output_key": "banner" }),
            &ctx_step2,
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !step2_handle.is_finished(),
        "step 2 must suspend cleanly while UserTakeover is active"
    );

    // User navigates & updates page manually, then resumes AI
    site_state.lock().auth_verified = true;
    backend
        .navigate(&format!("{base_url}/auth/dashboard"))
        .await
        .unwrap();
    sess.user_resume_ai();

    let step2 = step2_handle.await.unwrap();
    assert!(step2.success, "{}", step2.output);
    assert!(step2.output.contains("2FA Verified: Welcome Alice"));
    let structured = step2.structured.as_ref().unwrap();
    assert_eq!(structured["resumed_from_takeover"], true);
    assert!(structured["refreshed_url"]
        .as_str()
        .is_some_and(|u| u.contains("/auth/dashboard")));

    // Drop ctx to finalize turn trace and confirm HumanCheckpoint was inserted automatically
    drop(ctx);
    let trace = sess.last_completed_trace().expect("trace finalized on drop");
    assert!(trace
        .actions
        .iter()
        .any(|a| matches!(a.action, RecordedActionKind::HumanCheckpoint { .. })));
}

// ============================================================================
// Iteration 2 Remediation Regression Tests
// ============================================================================

struct RemediationBackend {
    url: SyncMutex<String>,
    title: SyncMutex<String>,
    snapshot: SyncMutex<BrowserSnapshot>,
    extracted: SyncMutex<String>,
    cookies: SyncMutex<Vec<BrowserCookie>>,
    fills: SyncMutex<Vec<(SemanticLocator, String, bool)>>,
    clicks: SyncMutex<Vec<SemanticLocator>>,
    delay: Duration,
    current_url_delay: Duration,
    export_cookies_delay: Duration,
}

impl RemediationBackend {
    fn new(url: &str, delay: Duration) -> Arc<Self> {
        Self::with_delays(url, delay, Duration::ZERO, Duration::ZERO)
    }

    fn with_delays(
        url: &str,
        delay: Duration,
        current_url_delay: Duration,
        export_cookies_delay: Duration,
    ) -> Arc<Self> {
        let origin = NormalizedOrigin::parse(url)
            .map(|o| o.as_str().to_string())
            .unwrap_or_else(|_| url.to_string());
        Arc::new(Self {
            url: SyncMutex::new(url.to_string()),
            title: SyncMutex::new("Remediation Page".to_string()),
            snapshot: SyncMutex::new(BrowserSnapshot {
                url: url.to_string(),
                origin,
                title: "Remediation Page".to_string(),
                text_excerpt: "Clean page".to_string(),
                interactive_elements: Vec::new(),
                redacted_field_count: 0,
            }),
            extracted: SyncMutex::new("Clean extracted text".to_string()),
            cookies: SyncMutex::new(Vec::new()),
            fills: SyncMutex::new(Vec::new()),
            clicks: SyncMutex::new(Vec::new()),
            delay,
            current_url_delay,
            export_cookies_delay,
        })
    }
}

#[async_trait]
impl BrowserSessionBackend for RemediationBackend {
    fn backend_kind(&self) -> BrowserBackendKind {
        BrowserBackendKind::MockCdp
    }
    async fn navigate(&self, url: &str) -> Result<NavigationResult, BotBrowserError> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        *self.url.lock() = url.to_string();
        let origin = NormalizedOrigin::parse(url)?.as_str().to_string();
        Ok(NavigationResult {
            url: url.to_string(),
            origin,
            title: self.title.lock().clone(),
            duration_ms: 1,
            backend: BrowserBackendKind::MockCdp,
        })
    }
    async fn go_back(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }
    async fn go_forward(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }
    async fn reload(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }
    async fn current_url(&self) -> Result<String, BotBrowserError> {
        if !self.current_url_delay.is_zero() {
            tokio::time::sleep(self.current_url_delay).await;
        }
        Ok(self.url.lock().clone())
    }
    async fn page_title(&self) -> Result<String, BotBrowserError> {
        Ok(self.title.lock().clone())
    }
    async fn snapshot_dom(&self) -> Result<BrowserSnapshot, BotBrowserError> {
        Ok(self.snapshot.lock().clone())
    }
    async fn eval_js(&self, _script: &str) -> Result<Value, BotBrowserError> {
        Ok(Value::Null)
    }
    async fn click_locator(&self, locator: &SemanticLocator) -> Result<ActionOutcome, BotBrowserError> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.clicks.lock().push(locator.clone());
        Ok(ActionOutcome {
            ok: true,
            operation: "click".to_string(),
            duration_ms: 1,
            matched_strategy: Some("css".to_string()),
            matched_selector: locator.css_selector.clone(),
            x: Some(10.0),
            y: Some(10.0),
            extracted_text: None,
        })
    }
    async fn fill_locator(
        &self,
        locator: &SemanticLocator,
        value: &str,
        is_secret: bool,
    ) -> Result<ActionOutcome, BotBrowserError> {
        self.fills
            .lock()
            .push((locator.clone(), value.to_string(), is_secret));
        Ok(ActionOutcome {
            ok: true,
            operation: "fill".to_string(),
            duration_ms: 1,
            matched_strategy: Some("css".to_string()),
            matched_selector: locator.css_selector.clone(),
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn press_key(&self, _key: &str) -> Result<ActionOutcome, BotBrowserError> {
        Ok(ActionOutcome {
            ok: true,
            operation: "press_key".to_string(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn scroll(&self, _delta_x: f64, _delta_y: f64) -> Result<ActionOutcome, BotBrowserError> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        Ok(ActionOutcome {
            ok: true,
            operation: "scroll".to_string(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn wait_for_locator(
        &self,
        _locator: &SemanticLocator,
        _timeout_ms: u64,
    ) -> Result<ActionOutcome, BotBrowserError> {
        Ok(ActionOutcome {
            ok: true,
            operation: "wait_for".to_string(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn extract_text_locator(&self, _locator: &SemanticLocator) -> Result<String, BotBrowserError> {
        Ok(self.extracted.lock().clone())
    }
    async fn capture_screenshot(&self) -> Result<Vec<u8>, BotBrowserError> {
        Ok(vec![137, 80, 78, 71, 13, 10, 26, 10])
    }
    async fn export_cookies(&self, _domain_filter: Option<&str>) -> Result<Vec<BrowserCookie>, BotBrowserError> {
        if !self.export_cookies_delay.is_zero() {
            tokio::time::sleep(self.export_cookies_delay).await;
        }
        Ok(self.cookies.lock().clone())
    }
    async fn import_cookies(&self, _cookies: &[BrowserCookie]) -> Result<usize, BotBrowserError> {
        Ok(0)
    }
    async fn dispatch_pointer_event(&self, _event: PointerInputEvent) -> Result<(), BotBrowserError> {
        Ok(())
    }
    async fn dispatch_key_event(&self, _event: KeyInputEvent) -> Result<(), BotBrowserError> {
        Ok(())
    }
}

#[tokio::test]
async fn test_m3_vault_and_human_assist_active_tab_dedup_lock_and_takeover() {
    let dir = TempDir::new().unwrap();
    let host = FabushiBotHost::new_unified_ephemeral("mock-gpt").unwrap();

    let mut cfg = OrchestratorConfig::new(dir.path());
    cfg.qr_refresh_interval = Duration::from_millis(15);
    cfg.qr_max_wait = Duration::from_secs(3);
    let vault = Arc::new(
        CredentialVault::open(dir.path().join("vault.db"), &MemoryMasterKeyStore::new()).unwrap(),
    );
    let meta = vault
        .store_credential(
            "https://portal.example.com/login",
            "alice@portal.example.com",
            "SuperSecretPw!99",
            Some("JBSWY3DPEHPK3PXP"),
            Some("alice"),
            None,
        )
        .unwrap();

    let spawn_count = Arc::new(AtomicUsize::new(0));
    let orch = Arc::new(BotBrowserOrchestrator::with_transport_factory(
        cfg,
        vault,
        Arc::new(CountingTransportFactory {
            creations: Arc::clone(&spawn_count),
        }),
    ));
    host.set_bot_browser(Arc::clone(&orch));

    let backend = RemediationBackend::new("https://portal.example.com/login", Duration::ZERO);
    let sess = host.browser_registry().register_active_backend(
        "in_app_tab_vault",
        Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>,
    );

    // Chat context uses a DIFFERENT session_id ("dm_chat_vault") and omits `session_id` arg
    // to verify active_session_id fallback + 0 duplicate spawns!
    let ctx = HermesToolContext::new(dir.path())
        .with_session("dm_chat_vault", 1)
        .with_bot_browser(Arc::clone(&orch))
        .with_browser_sessions(host.browser_registry());

    // 1. vault_list resolves active tab with 0 spawns
    let list_res = HermesToolExecutor::execute("vault_list", json!({}), &ctx).await;
    assert!(list_res.success, "vault_list failed: {}", list_res.output);
    assert_eq!(spawn_count.load(Ordering::SeqCst), 0);
    assert_eq!(
        list_res.structured.as_ref().unwrap()["credentials"][0]["credential_id"],
        meta.credential_id
    );

    // 2. vault_fill fills directly via Registered backend + Vault without spawning duplicate tab
    let fill_res = HermesToolExecutor::execute(
        "vault_fill",
        json!({
            "credential_id": meta.credential_id,
            "username_selector": "#user",
            "password_selector": "#pass",
            "totp_selector": "#totp",
            "submit_selector": "#submit"
        }),
        &ctx,
    )
    .await;
    assert!(fill_res.success, "vault_fill failed: {}", fill_res.output);
    assert_eq!(spawn_count.load(Ordering::SeqCst), 0);
    assert!(!fill_res.output.contains("SuperSecretPw!99"));
    assert!(!fill_res.output.contains("JBSWY3DPEHPK3PXP"));
    {
        let fills = backend.fills.lock();
        assert_eq!(fills.len(), 3);
        assert!(fills.iter().all(|(_, _, is_secret)| *is_secret));
        assert_eq!(fills[0].1, "alice@portal.example.com");
        assert_eq!(fills[1].1, "SuperSecretPw!99");
        assert_eq!(fills[2].1.len(), 6);
    }
    assert_eq!(backend.clicks.lock().len(), 1);

    // 3. ai_lock prevents concurrent vault_list / vault_fill
    let mut lock_guard = sess.try_begin_ai_turn().unwrap();
    let locked_list = HermesToolExecutor::execute("vault_list", json!({}), &ctx).await;
    assert!(!locked_list.success);
    assert!(locked_list.output.contains("locked"));
    let locked_fill = HermesToolExecutor::execute(
        "vault_fill",
        json!({
            "credential_id": meta.credential_id,
            "username_selector": "#user",
            "password_selector": "#pass"
        }),
        &ctx,
    )
    .await;
    assert!(!locked_fill.success);
    assert!(locked_fill.output.contains("locked"));
    lock_guard.finish();
    drop(lock_guard);

    // 4. UserTakeover suspends vault_list & vault_fill on Notify, resumes on user_resume_ai(), and aborts on user_cancel_ai()
    sess.control_gate().start_ai("active");
    sess.user_take_over();
    let ctx_takeover = ctx.clone();
    let cred_id_clone = meta.credential_id.clone();
    let fill_wait = tokio::spawn(async move {
        HermesToolExecutor::execute(
            "vault_fill",
            json!({
                "credential_id": cred_id_clone,
                "username_selector": "#user",
                "password_selector": "#pass"
            }),
            &ctx_takeover,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(!fill_wait.is_finished(), "vault_fill must suspend during UserTakeover");
    sess.user_resume_ai();
    let resumed_fill = fill_wait.await.unwrap();
    assert!(resumed_fill.success, "{}", resumed_fill.output);
    assert_eq!(
        resumed_fill.structured.as_ref().unwrap()["resumed_from_takeover"],
        true
    );

    // Test user_cancel_ai() aborts suspended vault_list cleanly
    sess.control_gate().start_ai("active");
    sess.user_take_over();
    let ctx_cancel = ctx.clone();
    let list_wait = tokio::spawn(async move {
        HermesToolExecutor::execute("vault_list", json!({}), &ctx_cancel).await
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(!list_wait.is_finished());
    sess.user_cancel_ai();
    let cancelled_list = list_wait.await.unwrap();
    assert!(!cancelled_list.success);
    assert!(cancelled_list.output.to_ascii_lowercase().contains("cancelled"));
    sess.reset_control();

    // 5. human_assist_qr & human_assist_click & vault_request_card execute with 0 duplicate spawns
    let ctx_qr = ctx.clone();
    let backend_qr = Arc::clone(&backend);
    let qr_handle = tokio::spawn(async move {
        HermesToolExecutor::execute(
            "human_assist_qr",
            json!({ "qr_selector": "#qr", "auth_cookie_names": ["auth_token"] }),
            &ctx_qr,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    backend_qr.cookies.lock().push(BrowserCookie::new(
        "auth_token",
        "qr_cookie_ok",
        "portal.example.com",
    ));
    let qr_res = qr_handle.await.unwrap();
    assert!(qr_res.success, "human_assist_qr failed: {}", qr_res.output);
    assert_eq!(spawn_count.load(Ordering::SeqCst), 0);

    let ctx_click = ctx.clone();
    let sess_click = Arc::clone(&sess);
    let click_handle = tokio::spawn(async move {
        HermesToolExecutor::execute(
            "human_assist_click",
            json!({ "target_selector": "#captcha", "instruction": "Click bus" }),
            &ctx_click,
        )
        .await
    });
    for _ in 0..50 {
        if matches!(
            sess_click.control_mode(),
            BrowserControlMode::WaitingForTakeover { .. }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    sess_click.user_resume_ai();
    let click_res = click_handle.await.unwrap();
    assert!(click_res.success, "human_assist_click failed: {}", click_res.output);
    assert_eq!(spawn_count.load(Ordering::SeqCst), 0);

    let card_res = HermesToolExecutor::execute(
        "vault_request_card",
        json!({
            "username_selector": "#user",
            "password_selector": "#pass",
            "wait_for_finish": false
        }),
        &ctx,
    )
    .await;
    assert!(card_res.success, "vault_request_card failed: {}", card_res.output);
    assert!(matches!(
        sess.control_mode(),
        BrowserControlMode::WaitingForTakeover { .. }
    ));
    assert_eq!(spawn_count.load(Ordering::SeqCst), 0);
    sess.reset_control();
}

#[tokio::test]
async fn test_m3_url_query_preservation_and_all_secret_leak_vectors_remediated() {
    let dir = TempDir::new().unwrap();
    let host = FabushiBotHost::new_unified_ephemeral("mock-gpt").unwrap();
    let mut ev_rx = host.subscribe_browser_events();

    let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    let backend = RemediationBackend::new("https://shop.example.com/start", Duration::ZERO);
    *backend.extracted.lock() =
        format!("ct0=CT0_EXTRACTED_444 auth_token=AUTH_EXTRACTED_555 refresh_token=REF_EXTRACTED_666 {jwt}");
    *backend.snapshot.lock() = BrowserSnapshot {
        url: "https://shop.example.com/start?api_key=SNAP_KEY_123&q=rust#top".into(),
        origin: "https://shop.example.com".into(),
        title: "Shop".into(),
        text_excerpt: "Debug: ct0=CT0_EXCERPT_777 auth_token=AUTH_EXCERPT_888".into(),
        interactive_elements: vec![
            DomNodeSnapshot {
                ref_id: "el_pw".into(),
                tag: "input".into(),
                input_type: Some("text".into()),
                name: None,
                id: Some("password".into()),
                aria_label: None,
                placeholder: Some("Enter password".into()),
                test_id: None,
                role: None,
                text: String::new(),
                value: Some("NodeSecretValue999".into()),
                is_password: false,
                selector: "#f_nonhint".into(),
                frame_origin: None,
                rect: None,
            },
            DomNodeSnapshot {
                ref_id: "el_note".into(),
                tag: "input".into(),
                input_type: Some("text".into()),
                name: Some("note".into()),
                id: Some("note".into()),
                aria_label: None,
                placeholder: None,
                test_id: None,
                role: None,
                text: format!("token={jwt}"),
                value: Some("%70%61%73%73%77%6f%72%64=hunter2_secret".into()),
                is_password: false,
                selector: "#note".into(),
                frame_origin: None,
                rect: None,
            },
        ],
        redacted_field_count: 0,
    };

    let sess = host.browser_registry().register_active_backend(
        "redact_tab",
        Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>,
    );

    // Vector A: browser_navigate structured URL is redacted (direct tool execution check)
    let benign_url = "https://shop.example.com/items?country_code=US&currency_code=USD&promo_code=SAVE10&zipcode=10001&resident_id=42&q=rust&ct0=CT0_NAV_111&auth_token=AUTH_NAV_222#access_token=FRAG_NAV_333";
    let direct_ctx = HermesToolContext::new(dir.path())
        .with_session("redact_tab", 1)
        .with_browser_sessions(host.browser_registry())
        .with_browser_turn_scope();
    let direct_nav = HermesToolExecutor::execute(
        "browser_navigate",
        json!({ "url": benign_url }),
        &direct_ctx,
    )
    .await;
    assert!(direct_nav.success, "{}", direct_nav.output);
    let nav_struct = serde_json::to_string(&direct_nav.structured).unwrap();
    assert!(!nav_struct.contains("CT0_NAV_111"), "{nav_struct}");
    assert!(!nav_struct.contains("AUTH_NAV_222"), "{nav_struct}");
    assert!(!nav_struct.contains("FRAG_NAV_333"), "{nav_struct}");
    assert!(nav_struct.contains("country_code=US"), "{nav_struct}");
    assert!(nav_struct.contains("zipcode=10001"), "{nav_struct}");
    assert!(nav_struct.contains("resident_id=42"), "{nav_struct}");
    drop(direct_ctx);
    while ev_rx.try_recv().is_ok() {}

    // Mock LLM executes:
    // 1. browser_navigate with benign params + secret params
    // 2. browser_snapshot
    // 3. browser_extract_text
    // 4. browser_click
    // 5. distill_browser_script_app (with save_as_skill: true)
    let (llm_url, _) = spawn_mock_openai_server(vec![
        assistant_tool_call_resp("r1", "browser_navigate", json!({ "url": benign_url })),
        assistant_tool_call_resp("r2", "browser_snapshot", json!({})),
        assistant_tool_call_resp(
            "r3",
            "browser_extract_text",
            json!({ "test_id": "summary", "output_key": "summary" }),
        ),
        assistant_tool_call_resp("r4", "browser_click", json!({ "test_id": "buy-btn" })),
        assistant_tool_call_resp(
            "r5",
            "distill_browser_script_app",
            json!({ "name": "Shop Benign Params", "save_as_skill": true }),
        ),
        assistant_text_resp("Done."),
    ])
    .await;

    let engine = host.core_engine();
    engine.providers().register_provider(ProviderConfig::openai_compatible(
        "mock_redact",
        "Mock Redact",
        llm_url,
        "mock-gpt",
        "sk-mock",
    ));
    engine
        .providers()
        .set_active("mock_redact", Some("mock-gpt"))
        .unwrap();

    let turn_res = engine
        .run_turn(full_turn_req("Run redaction check", "redact_tab"))
        .await
        .unwrap();
    assert_eq!(turn_res.tool_calls.len(), 5);
    assert!(turn_res.tool_calls.iter().all(|t| t.success));

    let nav_out = &turn_res.tool_calls[0].output;
    assert!(!nav_out.contains("CT0_NAV_111"), "{nav_out}");
    assert!(!nav_out.contains("AUTH_NAV_222"), "{nav_out}");
    assert!(!nav_out.contains("FRAG_NAV_333"), "{nav_out}");

    // Vector C (snapshot): id=password / placeholder / percent-encoded / JWT / multi-secret excerpt redacted
    let snap_out = &turn_res.tool_calls[1].output;
    assert!(!snap_out.contains("SNAP_KEY_123"), "{snap_out}");
    assert!(!snap_out.contains("CT0_EXCERPT_777"), "{snap_out}");
    assert!(!snap_out.contains("AUTH_EXCERPT_888"), "{snap_out}");
    assert!(!snap_out.contains("NodeSecretValue999"), "{snap_out}");
    assert!(!snap_out.contains("hunter2_secret"), "{snap_out}");
    assert!(!snap_out.contains(jwt), "{snap_out}");
    assert!(snap_out.contains("q=rust#top"), "{snap_out}");

    // Vector C (extract_text): multi-secret line & JWT redacted
    let ext_out = &turn_res.tool_calls[2].output;
    assert!(!ext_out.contains("CT0_EXTRACTED_444"), "{ext_out}");
    assert!(!ext_out.contains("AUTH_EXTRACTED_555"), "{ext_out}");
    assert!(!ext_out.contains("REF_EXTRACTED_666"), "{ext_out}");
    assert!(!ext_out.contains(jwt), "{ext_out}");

    // Skill saved via SkillsManager::save_distilled_browser_script_skill format
    let saved_skill = engine
        .skills()
        .skill_view("shop-benign-params")
        .expect("distilled skill saved");
    assert!(saved_skill.raw_markdown.contains("## AutomationScriptApp"));

    // Benign query parameters preserved in distilled AutomationScriptApp
    let app = sess.last_distilled_app().expect("last distilled app cached");
    let app_json = serde_json::to_string(&app).unwrap();
    assert!(app_json.contains("country_code=US"), "{app_json}");
    assert!(app_json.contains("currency_code=USD"), "{app_json}");
    assert!(app_json.contains("promo_code=SAVE10"), "{app_json}");
    assert!(app_json.contains("zipcode=10001"), "{app_json}");
    assert!(app_json.contains("resident_id=42"), "{app_json}");

    // Vector B: replay_script_app StepCompleted.url is redacted
    *backend.url.lock() =
        "https://shop.example.com/done?ct0=REPLAY_CT0_999&auth_token=REPLAY_AUTH_000&q=rust#access_token=REPLAY_FRAG_111".into();
    let mut params = BTreeMap::new();
    params.insert("ct0".to_string(), "ephemeral_ct0".to_string());
    params.insert("auth_token".to_string(), "ephemeral_auth".to_string());
    params.insert("access_token".to_string(), "ephemeral_frag".to_string());
    let simple_app = AutomationScriptApp {
        schema_version: 1,
        app_id: "app_replay_url".into(),
        slug: "app-replay-url".into(),
        name: "Replay URL Redact".into(),
        version: "1.0.0".into(),
        description: "".into(),
        category: "automation".into(),
        target_domains: vec![],
        parameters: vec![],
        steps: vec![ScriptStep::Scroll {
            delta_x: 0.0,
            delta_y: 10.0,
        }],
        required_capabilities: vec![],
        author_id: None,
        source_trace_summary: None,
    };
    host.replay_browser_script("redact_tab", &simple_app, &params, None)
        .await
        .unwrap();

    // Verify TraceCompleted emitted ONCE (not duplicated) and StepCompleted.url redacted
    let mut trace_completed_count = 0usize;
    let mut saw_redacted_replay_step = false;
    while let Ok(ev) = ev_rx.try_recv() {
        match ev {
            BrowserAutomationEvent::TraceCompleted { .. } => trace_completed_count += 1,
            BrowserAutomationEvent::StepCompleted {
                operation,
                url: Some(u),
                ..
            } if operation == "scroll" => {
                assert!(!u.contains("REPLAY_CT0_999"), "{u}");
                assert!(!u.contains("REPLAY_AUTH_000"), "{u}");
                assert!(!u.contains("REPLAY_FRAG_111"), "{u}");
                assert!(u.contains("q=rust"), "{u}");
                saw_redacted_replay_step = true;
            }
            _ => {}
        }
    }
    assert_eq!(trace_completed_count, 1, "TraceCompleted must not be duplicated");
    assert!(saw_redacted_replay_step);

    // Vector D: Orchestrator-routed browser_type refuses `input[name='ct0']` and `input[name='auth_token']` + emits StepFailed
    let vault = Arc::new(
        CredentialVault::open(dir.path().join("vault2.db"), &MemoryMasterKeyStore::new()).unwrap(),
    );
    let orch_creations = Arc::new(AtomicUsize::new(0));
    let orch = Arc::new(BotBrowserOrchestrator::with_transport_factory(
        OrchestratorConfig::new(dir.path()),
        vault,
        Arc::new(StubTransportFactory {
            creations: Arc::clone(&orch_creations),
        }),
    ));
    let orch_reg = Arc::new(BrowserSessionRegistry::new());
    let orch_ctx = HermesToolContext::new(dir.path())
        .with_session("orch_type_tab", 1)
        .with_bot_browser(orch)
        .with_browser_sessions(Arc::clone(&orch_reg))
        .with_browser_turn_scope();
    let mut orch_ev_rx = orch_reg.subscribe();

    let ct0_res = HermesToolExecutor::execute(
        "browser_type",
        json!({ "selector": "input[name='ct0']", "text": "leak" }),
        &orch_ctx,
    )
    .await;
    assert!(!ct0_res.success);
    let auth_res = HermesToolExecutor::execute(
        "browser_type",
        json!({ "selector": "input[name='auth_token']", "text": "leak" }),
        &orch_ctx,
    )
    .await;
    assert!(!auth_res.success);

    let mut step_failed_count = 0usize;
    while let Ok(ev) = orch_ev_rx.try_recv() {
        if matches!(
            ev,
            BrowserAutomationEvent::StepFailed { operation, .. } if operation == "browser_type"
        ) {
            step_failed_count += 1;
        }
    }
    assert_eq!(step_failed_count, 2);
}

#[tokio::test]
async fn test_m3_request_takeover_step_budget_and_replay_control_mode_and_abort_guards() {
    let dir = TempDir::new().unwrap();
    let reg = Arc::new(BrowserSessionRegistry::with_config(BrowserSessionConfig {
        max_steps_per_run: 2,
        auto_pause_on_sensitive_origin: true,
        event_channel_capacity: 128,
    }));

    // 1. browser_request_takeover enforces max_steps_per_run & emits StepFailed
    let fast_backend = RemediationBackend::new("https://example.com/start", Duration::ZERO);
    let sess = reg.register_backend(
        "budget_takeover_tab",
        Arc::clone(&fast_backend) as Arc<dyn BrowserSessionBackend>,
    );
    let mut ev_rx = reg.subscribe();

    let ctx_scoped = HermesToolContext::new(dir.path())
        .with_session("budget_takeover_tab", 1)
        .with_browser_sessions(Arc::clone(&reg))
        .with_browser_turn_scope();

    assert!(
        HermesToolExecutor::execute(
            "browser_navigate",
            json!({ "url": "https://example.com/1" }),
            &ctx_scoped,
        )
        .await
        .success
    );
    assert!(
        HermesToolExecutor::execute(
            "browser_scroll",
            json!({ "delta_x": 0.0, "delta_y": 20.0 }),
            &ctx_scoped,
        )
        .await
        .success
    );
    let step3 = HermesToolExecutor::execute(
        "browser_request_takeover",
        json!({ "reason": "Bypass budget?", "wait_for_finish": false }),
        &ctx_scoped,
    )
    .await;
    assert!(!step3.success);
    assert!(step3.output.contains("budget exhausted"));
    drop(ctx_scoped);

    let mut saw_takeover_step_failed = false;
    while let Ok(ev) = ev_rx.try_recv() {
        if matches!(
            ev,
            BrowserAutomationEvent::StepFailed {
                operation,
                error,
                ..
            } if operation == "browser_request_takeover" && error.contains("budget exhausted")
        ) {
            saw_takeover_step_failed = true;
        }
    }
    assert!(saw_takeover_step_failed);

    // 2. replay_script_app emits ControlModeChanged(AiControlling -> Idle) on fast mock
    let fast_app = AutomationScriptApp {
        schema_version: 1,
        app_id: "fast_app".into(),
        slug: "fast-app".into(),
        name: "Fast App".into(),
        version: "1.0.0".into(),
        description: "".into(),
        category: "automation".into(),
        target_domains: vec![],
        parameters: vec![],
        steps: vec![
            ScriptStep::Navigate {
                url_template: "https://example.com/done".into(),
            },
            ScriptStep::Scroll {
                delta_x: 0.0,
                delta_y: 10.0,
            },
        ],
        required_capabilities: vec![],
        author_id: None,
        source_trace_summary: None,
    };
    let rep = reg
        .replay_script("budget_takeover_tab", &fast_app, &BTreeMap::new(), None)
        .await
        .unwrap();
    assert!(rep.success);

    let mut modes = Vec::new();
    while let Ok(ev) = ev_rx.try_recv() {
        if let BrowserAutomationEvent::ControlModeChanged { mode, .. } = ev {
            modes.push(mode);
        }
    }
    assert!(
        modes
            .iter()
            .any(|m| matches!(m, BrowserControlMode::AiControlling { .. })),
        "missing ControlModeChanged(AiControlling): {modes:?}"
    );
    assert!(
        matches!(modes.last(), Some(BrowserControlMode::Idle)),
        "missing final ControlModeChanged(Idle): {modes:?}"
    );

    // 3. replay_script_app aborted by tokio::select! emits ControlModeChanged(Idle) & releases lock
    let slow_backend = RemediationBackend::new("https://example.com/slow", Duration::from_millis(300));
    let slow_sess = reg.register_backend(
        "slow_replay_tab",
        Arc::clone(&slow_backend) as Arc<dyn BrowserSessionBackend>,
    );
    let mut slow_rx = slow_sess.subscribe();
    let empty_params = BTreeMap::new();
    tokio::select! {
        _ = reg.replay_script("slow_replay_tab", &fast_app, &empty_params, None) => {
            panic!("slow replay should have been aborted by select!");
        }
        _ = tokio::time::sleep(Duration::from_millis(40)) => {}
    }
    assert!(!slow_sess.is_ai_locked());
    assert_eq!(slow_sess.control_mode(), BrowserControlMode::Idle);
    let mut slow_modes = Vec::new();
    while let Ok(ev) = slow_rx.try_recv() {
        if let BrowserAutomationEvent::ControlModeChanged { mode, .. } = ev {
            slow_modes.push(mode);
        }
    }
    assert!(
        matches!(slow_modes.last(), Some(BrowserControlMode::Idle)),
        "expected ControlModeChanged(Idle) after select! abort: {slow_modes:?}"
    );

    // 4. Standalone browser_request_takeover(wait_for_finish = true) mid-await abort resets to Idle,
    //    whereas normal wait_for_finish = false preserves WaitingForTakeover.
    let standalone_ctx = HermesToolContext::new(dir.path())
        .with_session("budget_takeover_tab", 2)
        .with_browser_sessions(Arc::clone(&reg));
    sess.reset_step_budget();
    sess.set_max_steps_per_run(10);

    let ctx_abort = standalone_ctx.clone();
    let abort_task = tokio::spawn(async move {
        HermesToolExecutor::execute(
            "browser_request_takeover",
            json!({ "reason": "Standalone abort", "wait_for_finish": true }),
            &ctx_abort,
        )
        .await
    });
    for _ in 0..50 {
        if matches!(
            sess.control_mode(),
            BrowserControlMode::WaitingForTakeover { .. }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(matches!(
        sess.control_mode(),
        BrowserControlMode::WaitingForTakeover { .. }
    ));
    abort_task.abort();
    let _ = abort_task.await;
    assert_eq!(
        sess.control_mode(),
        BrowserControlMode::Idle,
        "aborting standalone suspended takeover must reset control_mode to Idle"
    );
    assert!(!sess.is_ai_locked());

    // Normal wait_for_finish = false preserves WaitingForTakeover across standalone drop
    let nonblocking = HermesToolExecutor::execute(
        "browser_request_takeover",
        json!({ "reason": "Keep waiting", "wait_for_finish": false }),
        &standalone_ctx,
    )
    .await;
    assert!(nonblocking.success, "{}", nonblocking.output);
    assert!(matches!(
        sess.control_mode(),
        BrowserControlMode::WaitingForTakeover { ref reason } if reason == "Keep waiting"
    ));
    sess.reset_control();

    // 5. browser_navigate auto-pause cancellation on high-sensitivity origin emits StepFailed
    while ev_rx.try_recv().is_ok() {}
    let ctx_sens = standalone_ctx.clone();
    let sens_task = tokio::spawn(async move {
        HermesToolExecutor::execute(
            "browser_navigate",
            json!({ "url": "https://checkout.stripe.com/pay" }),
            &ctx_sens,
        )
        .await
    });
    for _ in 0..50 {
        if matches!(
            sess.control_mode(),
            BrowserControlMode::WaitingForTakeover { .. }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    sess.user_cancel_ai();
    let sens_res = sens_task.await.unwrap();
    assert!(!sens_res.success);
    let mut saw_nav_step_failed = false;
    while let Ok(ev) = ev_rx.try_recv() {
        if matches!(
            ev,
            BrowserAutomationEvent::StepFailed { operation, .. } if operation == "browser_navigate"
        ) {
            saw_nav_step_failed = true;
        }
    }
    assert!(saw_nav_step_failed);
}

#[tokio::test]
async fn test_m3_slow_url_and_cookie_abort_guards_and_browser_type_locator_parity() {
    let dir = TempDir::new().unwrap();
    let reg = Arc::new(BrowserSessionRegistry::new());

    // 1. Aborting `browser_request_takeover(wait_for_finish = true)` via `tokio::select!`
    //    while slow `backend.current_url().await` (80ms) is pending resets to Idle & unlocks.
    let slow_url_backend = RemediationBackend::with_delays(
        "https://example.com/2fa",
        Duration::ZERO,
        Duration::from_millis(80),
        Duration::ZERO,
    );
    let sess_takeover = reg.register_backend(
        "slow_url_takeover_tab",
        Arc::clone(&slow_url_backend) as Arc<dyn BrowserSessionBackend>,
    );
    let ctx_takeover = HermesToolContext::new(dir.path())
        .with_session("slow_url_takeover_tab", 1)
        .with_browser_sessions(Arc::clone(&reg));

    tokio::select! {
        _ = HermesToolExecutor::execute(
            "browser_request_takeover",
            json!({ "reason": "Slow URL 2FA", "wait_for_finish": true }),
            &ctx_takeover,
        ) => {
            panic!("browser_request_takeover should have been aborted while current_url().await was pending");
        }
        _ = async {
            while !matches!(sess_takeover.control_mode(), BrowserControlMode::WaitingForTakeover { .. }) {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        } => {}
    }
    assert_eq!(sess_takeover.control_mode(), BrowserControlMode::Idle);
    assert!(!sess_takeover.is_ai_locked());

    // Normal `browser_request_takeover(wait_for_finish = false)` still preserves `WaitingForTakeover`
    // even when `current_url().await` has an 80ms delay.
    let nonblocking_res = HermesToolExecutor::execute(
        "browser_request_takeover",
        json!({ "reason": "Preserve slow URL waiting", "wait_for_finish": false }),
        &ctx_takeover,
    )
    .await;
    assert!(nonblocking_res.success, "{}", nonblocking_res.output);
    assert!(matches!(
        sess_takeover.control_mode(),
        BrowserControlMode::WaitingForTakeover { ref reason } if reason == "Preserve slow URL waiting"
    ));
    assert!(!sess_takeover.is_ai_locked());
    sess_takeover.reset_control();

    // Also verify `prepare_step` abort while `current_url().await` (80ms) is pending under `UserTakeover`.
    sess_takeover.control_gate().start_ai("active");
    sess_takeover.user_take_over();
    tokio::select! {
        _ = HermesToolExecutor::execute(
            "browser_scroll",
            json!({ "delta_x": 0.0, "delta_y": 15.0 }),
            &ctx_takeover,
        ) => {
            panic!("prepare_step should have been aborted during slow current_url().await");
        }
        _ = tokio::time::sleep(Duration::from_millis(20)) => {}
    }
    assert_eq!(sess_takeover.control_mode(), BrowserControlMode::Idle);
    assert!(!sess_takeover.is_ai_locked());

    // 2. Aborting `human_assist_qr` via `tokio::select!` while slow `current_url().await`
    //    is pending inside `request_takeover_and_wait(..., false, true)` resets to Idle & unlocks.
    let slow_qr_url_backend = RemediationBackend::with_delays(
        "https://example.com/qr-login",
        Duration::ZERO,
        Duration::from_millis(80),
        Duration::ZERO,
    );
    let sess_qr_url = reg.register_backend(
        "slow_qr_url_tab",
        Arc::clone(&slow_qr_url_backend) as Arc<dyn BrowserSessionBackend>,
    );
    let ctx_qr_url = HermesToolContext::new(dir.path())
        .with_session("slow_qr_url_tab", 1)
        .with_browser_sessions(Arc::clone(&reg));

    tokio::select! {
        _ = HermesToolExecutor::execute(
            "human_assist_qr",
            json!({ "qr_selector": "#qr" }),
            &ctx_qr_url,
        ) => {
            panic!("human_assist_qr should have been aborted during request_takeover_and_wait current_url().await");
        }
        _ = async {
            while !matches!(sess_qr_url.control_mode(), BrowserControlMode::WaitingForTakeover { .. }) {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        } => {}
    }
    assert_eq!(sess_qr_url.control_mode(), BrowserControlMode::Idle);
    assert!(!sess_qr_url.is_ai_locked());

    // 3. Aborting `human_assist_qr` via `tokio::select!` while slow `export_cookies().await`
    //    is pending inside `wait_for_qr_assist_or_resume` resets to Idle & unlocks.
    let slow_qr_cookie_backend = RemediationBackend::with_delays(
        "https://example.com/qr-cookie",
        Duration::ZERO,
        Duration::ZERO,
        Duration::from_millis(120),
    );
    let sess_qr_cookie = reg.register_backend(
        "slow_qr_cookie_tab",
        Arc::clone(&slow_qr_cookie_backend) as Arc<dyn BrowserSessionBackend>,
    );
    let ctx_qr_cookie = HermesToolContext::new(dir.path())
        .with_session("slow_qr_cookie_tab", 1)
        .with_browser_sessions(Arc::clone(&reg));

    tokio::select! {
        _ = HermesToolExecutor::execute(
            "human_assist_qr",
            json!({ "qr_selector": "#qr" }),
            &ctx_qr_cookie,
        ) => {
            panic!("human_assist_qr should have been aborted during export_cookies().await");
        }
        _ = async {
            while !matches!(sess_qr_cookie.control_mode(), BrowserControlMode::WaitingForTakeover { .. }) {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        } => {}
    }
    assert_eq!(sess_qr_cookie.control_mode(), BrowserControlMode::Idle);
    assert!(!sess_qr_cookie.is_ai_locked());

    // 4. `tool_browser_type` refuses `session_id`/`cookie`/`bearer` across `css_selector`,
    //    `name_attr`, `test_id`, `aria_label`, and `text_anchor` identically to `selector`,
    //    both with `browser_sessions` attached and in fallback mode.
    let fast_backend = RemediationBackend::new("https://example.com/form", Duration::ZERO);
    let _sess_form = reg.register_backend(
        "type_parity_tab",
        Arc::clone(&fast_backend) as Arc<dyn BrowserSessionBackend>,
    );
    let ctx_form = HermesToolContext::new(dir.path())
        .with_session("type_parity_tab", 1)
        .with_browser_sessions(Arc::clone(&reg));

    for marker in ["session_id", "cookie", "bearer"] {
        for field in ["selector", "css_selector", "name_attr", "test_id", "aria_label", "text_anchor"] {
            let arg_val = if matches!(field, "selector" | "css_selector") {
                format!("input[name='{marker}']")
            } else {
                marker.to_string()
            };
            let res = HermesToolExecutor::execute(
                "browser_type",
                json!({ field: arg_val, "text": "leak" }),
                &ctx_form,
            )
            .await;
            assert!(
                !res.success,
                "browser_type should refuse '{marker}' via '{field}', got success: {}",
                res.output
            );
            assert!(
                !res.output.to_ascii_lowercase().contains("chrome"),
                "error output must not contain 'chrome': {}",
                res.output
            );
        }
    }
    assert!(
        fast_backend.fills.lock().is_empty(),
        "refused sensitive fields must never reach backend.fill_locator"
    );

    // Fallback mode (no browser_sessions) also refuses `css_selector` and `name_attr` for `session_id`/`cookie`/`bearer`
    let ctx_fallback = HermesToolContext::new(dir.path()).with_session("fallback_tab", 1);
    for marker in ["session_id", "cookie", "bearer"] {
        for field in ["selector", "css_selector", "name_attr"] {
            let arg_val = if matches!(field, "selector" | "css_selector") {
                format!("input[name='{marker}']")
            } else {
                marker.to_string()
            };
            let res = HermesToolExecutor::execute(
                "browser_type",
                json!({ field: arg_val, "text": "leak" }),
                &ctx_fallback,
            )
            .await;
            assert!(
                !res.success,
                "fallback browser_type should refuse '{marker}' via '{field}'",
            );
            assert!(
                res.output.contains("Refusing to type"),
                "expected refusal message, got: {}",
                res.output
            );
        }
    }

    // Benign field still succeeds
    let benign = HermesToolExecutor::execute(
        "browser_type",
        json!({ "name_attr": "q", "text": "rust browser" }),
        &ctx_form,
    )
    .await;
    assert!(benign.success, "benign field should succeed: {}", benign.output);
    assert_eq!(fast_backend.fills.lock().len(), 1);
}

// ============================================================================
// Iteration 4 Remediation Regression Tests
// ============================================================================

struct RecordingCdpTransport {
    eval_expressions: SyncMutex<Vec<String>>,
    focused_selectors: SyncMutex<Vec<String>>,
    clicked_selectors: SyncMutex<Vec<String>>,
    inserted_texts: SyncMutex<Vec<String>>,
}

impl RecordingCdpTransport {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            eval_expressions: SyncMutex::new(Vec::new()),
            focused_selectors: SyncMutex::new(Vec::new()),
            clicked_selectors: SyncMutex::new(Vec::new()),
            inserted_texts: SyncMutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl CdpTransport for RecordingCdpTransport {
    async fn send_command(&self, method: &str, params: Value) -> Result<Value, BotBrowserError> {
        match method {
            "Runtime.evaluate" => {
                let expr = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.eval_expressions.lock().push(expr.clone());

                // Simulate DOM for click_selector_internal (`el.click()`)
                if expr.contains("el.click()") {
                    if expr.contains("\"#submit_resident\"") {
                        self.clicked_selectors
                            .lock()
                            .push("#submit_resident".to_string());
                        return Ok(json!({ "result": { "value": "CLICKED" } }));
                    }
                    return Ok(json!({ "result": { "value": "NOT_FOUND" } }));
                }

                // Simulate DOM for probe_and_focus_non_sensitive
                if expr.contains("const sel = \"#resident_id\"") {
                    if expr.contains("el.focus()") {
                        self.focused_selectors.lock().push("#resident_id".to_string());
                    }
                    return Ok(json!({
                        "result": { "value": "{\"found\":true,\"sensitive\":false}" }
                    }));
                }
                if expr.contains("const sel = \"#account_entry_field\"") {
                    // Simulated DOM element has `type="password"` even though selector string is benign
                    return Ok(json!({
                        "result": { "value": "{\"found\":true,\"sensitive\":true}" }
                    }));
                }
                if expr.contains("const sel = null") {
                    // Active element fallback — must NOT count as focusing #resident_id
                    return Ok(json!({
                        "result": { "value": "{\"found\":true,\"sensitive\":false}" }
                    }));
                }

                Ok(json!({ "result": { "value": "ok" } }))
            }
            "Input.insertText" => {
                let txt = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.inserted_texts.lock().push(txt);
                Ok(json!({}))
            }
            _ => Ok(json!({})),
        }
    }

    async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
        Ok("http://127.0.0.1:0".to_string())
    }
}

struct RecordingTransportFactory {
    transport: Arc<RecordingCdpTransport>,
}

#[async_trait]
impl SessionTransportFactory for RecordingTransportFactory {
    async fn create_transport(
        &self,
        _session_id: &str,
        _data_dir: &std::path::Path,
        _origin_hint: Option<&NormalizedOrigin>,
        _event_tx: &broadcast::Sender<BotBrowserEvent>,
    ) -> Result<Arc<dyn CdpTransport>, BotBrowserError> {
        Ok(Arc::clone(&self.transport) as Arc<dyn CdpTransport>)
    }
}

#[tokio::test]
async fn test_m3_orchestrator_css_selector_focus_and_dom_password_blocking() {
    let dir = TempDir::new().unwrap();
    let vault = Arc::new(
        CredentialVault::open(dir.path().join("vault_it4.db"), &MemoryMasterKeyStore::new()).unwrap(),
    );
    let transport = RecordingCdpTransport::new();
    let orch = Arc::new(BotBrowserOrchestrator::with_transport_factory(
        OrchestratorConfig::new(dir.path()),
        vault,
        Arc::new(RecordingTransportFactory {
            transport: Arc::clone(&transport),
        }),
    ));

    // 1. Registry-backed Orchestrator path (`browser_sessions` attached)
    let reg = Arc::new(BrowserSessionRegistry::new());
    let ctx_reg = HermesToolContext::new(dir.path())
        .with_session("orch_css_tab", 1)
        .with_bot_browser(Arc::clone(&orch))
        .with_browser_sessions(Arc::clone(&reg))
        .with_browser_turn_scope();

    let ok_type = HermesToolExecutor::execute(
        "browser_type",
        json!({ "css_selector": "#resident_id", "text": "42" }),
        &ctx_reg,
    )
    .await;
    assert!(ok_type.success, "expected css_selector type to succeed: {}", ok_type.output);
    assert_eq!(
        transport.focused_selectors.lock().as_slice(),
        &["#resident_id".to_string()],
        "Orchestrator browser_type must pass css_selector to probe_and_focus_non_sensitive"
    );
    assert_eq!(
        transport.inserted_texts.lock().as_slice(),
        &["42".to_string()]
    );

    // Benign-named selector targeting a DOM `type="password"` element MUST be blocked via css_selector
    let blocked_dom_pw = HermesToolExecutor::execute(
        "browser_type",
        json!({ "css_selector": "#account_entry_field", "text": "hunter2" }),
        &ctx_reg,
    )
    .await;
    assert!(
        !blocked_dom_pw.success,
        "DOM type='password' targeted via css_selector must be blocked"
    );
    assert!(
        blocked_dom_pw.output.contains("Refusing to type"),
        "unexpected error: {}",
        blocked_dom_pw.output
    );
    assert_eq!(
        transport.inserted_texts.lock().as_slice(),
        &["42".to_string()],
        "blocked DOM password input must not receive Input.insertText"
    );

    // Also verify browser_click with css_selector (and no selector) routes selector to Orchestrator
    let ok_click = HermesToolExecutor::execute(
        "browser_click",
        json!({ "css_selector": "#submit_resident" }),
        &ctx_reg,
    )
    .await;
    assert!(ok_click.success, "expected css_selector click to succeed: {}", ok_click.output);
    assert_eq!(
        transport.clicked_selectors.lock().as_slice(),
        &["#submit_resident".to_string()]
    );
    drop(ctx_reg);

    // 2. Fallback Orchestrator path (`browser_sessions` omitted)
    transport.focused_selectors.lock().clear();
    transport.clicked_selectors.lock().clear();
    transport.inserted_texts.lock().clear();

    let ctx_fallback = HermesToolContext::new(dir.path())
        .with_session("orch_css_fallback_tab", 1)
        .with_bot_browser(Arc::clone(&orch));

    let fb_ok = HermesToolExecutor::execute(
        "browser_type",
        json!({ "css_selector": "#resident_id", "text": "99" }),
        &ctx_fallback,
    )
    .await;
    assert!(fb_ok.success, "fallback css_selector type failed: {}", fb_ok.output);
    assert_eq!(
        transport.focused_selectors.lock().as_slice(),
        &["#resident_id".to_string()]
    );
    assert_eq!(
        transport.inserted_texts.lock().as_slice(),
        &["99".to_string()]
    );

    let fb_blocked = HermesToolExecutor::execute(
        "browser_type",
        json!({ "css_selector": "#account_entry_field", "text": "hunter2" }),
        &ctx_fallback,
    )
    .await;
    assert!(!fb_blocked.success);
    assert_eq!(
        transport.inserted_texts.lock().as_slice(),
        &["99".to_string()]
    );

    let fb_click = HermesToolExecutor::execute(
        "browser_click",
        json!({ "css_selector": "#submit_resident" }),
        &ctx_fallback,
    )
    .await;
    assert!(fb_click.success, "fallback css_selector click failed: {}", fb_click.output);
    assert_eq!(
        transport.clicked_selectors.lock().as_slice(),
        &["#submit_resident".to_string()]
    );
}

#[tokio::test]
async fn test_m3_extract_text_sensitivity_parity_and_normalization_hardening() {
    let dir = TempDir::new().unwrap();
    let reg = Arc::new(BrowserSessionRegistry::new());
    let backend = RemediationBackend::new("https://example.com/profile", Duration::ZERO);
    *backend.extracted.lock() = "opaque_session_val_999".to_string();
    let sess = reg.register_backend(
        "extract_parity_tab",
        Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>,
    );
    sess.set_max_steps_per_run(128);

    let ctx = HermesToolContext::new(dir.path())
        .with_session("extract_parity_tab", 1)
        .with_browser_sessions(Arc::clone(&reg));

    // 1. `browser_extract_text` refuses sensitive markers across all SemanticLocator fields
    let sensitive_cases = vec![
        ("name_attr", "session_id"),
        ("name_attr", "cookie"),
        ("test_id", "bearer"),
        ("aria_label", "refresh_token"),
        ("text_anchor", "id_token"),
        ("name_attr", "oauth_token"),
        ("role", "session_id"),
        ("role", "cookie"),
        ("role", "bearer"),
    ];
    for (field, val) in sensitive_cases {
        let res = HermesToolExecutor::execute(
            "browser_extract_text",
            json!({ field: val, "output_key": "out" }),
            &ctx,
        )
        .await;
        assert!(
            !res.success,
            "browser_extract_text must refuse {field}='{val}', got success: {}",
            res.output
        );
        assert!(
            !res.output.contains("opaque_session_val_999"),
            "refused extract_text must never leak opaque value: {}",
            res.output
        );
    }

    // 2. Hyphenated & camelCase & role forms refused by BOTH `browser_type` and `browser_extract_text`
    let normalized_sensitive_cases = vec![
        ("name_attr", "session-id"),
        ("name_attr", "sessionId"),
        ("css_selector", "input[name='authToken']"),
        ("selector", "#auth-token"),
        ("test_id", "access-token"),
        ("aria_label", "accessToken"),
        ("name_attr", "refresh-token"),
        ("test_id", "refreshToken"),
        ("name_attr", "id-token"),
        ("aria_label", "idToken"),
        ("name_attr", "oauth-token"),
        ("test_id", "oauthToken"),
        ("name_attr", "api-key"),
        ("aria_label", "apiKey"),
        ("name_attr", "bearer-token"),
        ("test_id", "bearerToken"),
        ("role", "session_id"),
        ("role", "cookie"),
        ("role", "bearer"),
    ];
    for (field, val) in normalized_sensitive_cases {
        let type_res = HermesToolExecutor::execute(
            "browser_type",
            json!({ field: val, "text": "leak" }),
            &ctx,
        )
        .await;
        assert!(
            !type_res.success,
            "browser_type must refuse {field}='{val}', got: {}",
            type_res.output
        );

        let ext_res = HermesToolExecutor::execute(
            "browser_extract_text",
            json!({ field: val, "output_key": "out" }),
            &ctx,
        )
        .await;
        assert!(
            !ext_res.success,
            "browser_extract_text must refuse {field}='{val}', got: {}",
            ext_res.output
        );
        assert!(!ext_res.output.contains("opaque_session_val_999"));
    }
    assert!(
        backend.fills.lock().is_empty(),
        "no refused browser_type call may invoke backend.fill_locator"
    );

    // 3. Benign identifiers (including *_id / *_code / ARIA roles) succeed on BOTH tools
    *backend.extracted.lock() = "Resident Profile Verified".to_string();
    let benign_cases = vec![
        ("name_attr", "resident_id"),
        ("name_attr", "residentId"),
        ("name_attr", "country_code"),
        ("name_attr", "zipcode"),
        ("name_attr", "promo_code"),
        ("name_attr", "email"),
        ("name_attr", "username"),
        ("name_attr", "search"),
        ("name_attr", "q"),
        ("role", "textbox"),
        ("role", "button"),
    ];
    for &(field, val) in &benign_cases {
        let type_res = HermesToolExecutor::execute(
            "browser_type",
            json!({ field: val, "text": "ok_value" }),
            &ctx,
        )
        .await;
        assert!(
            type_res.success,
            "benign browser_type for {field}='{val}' must succeed: {}",
            type_res.output
        );

        let ext_res = HermesToolExecutor::execute(
            "browser_extract_text",
            json!({ field: val, "output_key": "out" }),
            &ctx,
        )
        .await;
        assert!(
            ext_res.success,
            "benign browser_extract_text for {field}='{val}' must succeed: {}",
            ext_res.output
        );
        assert!(ext_res.output.contains("Resident Profile Verified"));
    }
    assert_eq!(backend.fills.lock().len(), benign_cases.len());
}
