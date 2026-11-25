use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use bot_browser::cdp::CdpTransport;
use bot_browser::orchestrator::{
    BotBrowserOrchestrator, OrchestratorConfig, SessionTransportFactory,
};
use bot_browser::vault::{CredentialVault, MemoryMasterKeyStore};
use bot_browser::{
    ActionOutcome, AutomationScriptApp, BotBrowserError, BotBrowserEvent, BrowserActionTrace,
    BrowserBackendKind, BrowserControlGate, BrowserControlMode, BrowserCookie,
    BrowserSessionBackend, BrowserSnapshot, CdpBrowserBackend, CredentialVaultSecretResolver,
    DeterministicScriptRunner, InMemoryVaultSecretResolver, KeyInputEvent, NavigationResult,
    NormalizedOrigin, PointerInputEvent, RecordedActionKind, ReusePolicy, ScriptAppMetadata,
    ScriptCompiler, ScriptParameter, ScriptStep, SemanticLocator, TraceParameterBinding,
    VaultSecretResolver, validate_script_app_safe_for_publish,
};

#[derive(Default)]
struct SimulatedDomPage {
    current_url: String,
    inputs: BTreeMap<String, String>,
    checkbox_checked: bool,
    visible_texts: BTreeMap<String, String>,
}

struct LiveHttpTestBackend {
    client: reqwest::Client,
    cookies: Mutex<Vec<BrowserCookie>>,
    dom: Mutex<SimulatedDomPage>,
    secret_fills_observed: Mutex<Vec<(String, String, bool)>>,
}

impl LiveHttpTestBackend {
    fn new() -> Self {
        Self {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            cookies: Mutex::new(Vec::new()),
            dom: Mutex::new(SimulatedDomPage::default()),
            secret_fills_observed: Mutex::new(Vec::new()),
        }
    }

    async fn cookie_header_for(&self, url: &str) -> String {
        let origin = NormalizedOrigin::parse(url).unwrap();
        let guard = self.cookies.lock().await;
        guard
            .iter()
            .filter(|c| c.matches_host(origin.host()))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn apply_set_cookies(&self, url: &str, headers: &reqwest::header::HeaderMap) {
        let origin = NormalizedOrigin::parse(url).unwrap();
        let mut guard = self.cookies.lock().await;
        for val in headers.get_all(reqwest::header::SET_COOKIE) {
            if let Ok(s) = val.to_str() {
                let (nv, attrs) = s.split_once(';').unwrap_or((s, ""));
                if let Some((name, value)) = nv.split_once('=') {
                    let mut c = BrowserCookie::new(name.trim(), value.trim(), origin.host());
                    c.http_only = attrs.to_ascii_lowercase().contains("httponly");
                    guard.push(c);
                }
            }
        }
    }

    fn locator_key(locator: &SemanticLocator) -> String {
        locator
            .test_id
            .clone()
            .or_else(|| locator.aria_label.clone())
            .or_else(|| locator.name_attr.clone())
            .or_else(|| locator.css_selector.clone())
            .or_else(|| locator.role.clone())
            .or_else(|| locator.text_anchor.clone())
            .unwrap_or_default()
    }

    async fn sync_dom_from_http(&self, url: &str, body: &str) {
        let mut dom = self.dom.lock().await;
        dom.current_url = url.to_string();
        dom.visible_texts.clear();
        if let Ok(json_body) = serde_json::from_str::<Value>(body) {
            if let Some(obj) = json_body.get("elements").and_then(|v| v.as_object()) {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        dom.visible_texts.insert(k.clone(), s.to_string());
                    }
                }
            }
        } else {
            dom.visible_texts
                .insert("body".to_string(), body.to_string());
        }
    }
}

#[async_trait]
impl BrowserSessionBackend for LiveHttpTestBackend {
    fn backend_kind(&self) -> BrowserBackendKind {
        BrowserBackendKind::MockCdp
    }

    async fn navigate(&self, url: &str) -> Result<NavigationResult, BotBrowserError> {
        let origin = NormalizedOrigin::parse(url)?;
        let cookie_hdr = self.cookie_header_for(url).await;
        let mut req = self.client.get(url);
        if !cookie_hdr.is_empty() {
            req = req.header(reqwest::header::COOKIE, cookie_hdr);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| BotBrowserError::Io(e.to_string()))?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp
            .text()
            .await
            .map_err(|e| BotBrowserError::Io(e.to_string()))?;
        if !status.is_success() {
            return Err(BotBrowserError::ScriptReplay(format!(
                "HTTP {} navigating to {}",
                status.as_u16(),
                url
            )));
        }
        self.apply_set_cookies(url, &headers).await;
        self.sync_dom_from_http(url, &body).await;
        Ok(NavigationResult {
            url: url.to_string(),
            origin: origin.as_str().to_string(),
            title: "Mock App".to_string(),
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
        Ok(self.dom.lock().await.current_url.clone())
    }
    async fn page_title(&self) -> Result<String, BotBrowserError> {
        Ok("Mock App".to_string())
    }
    async fn snapshot_dom(&self) -> Result<BrowserSnapshot, BotBrowserError> {
        let u = self.current_url().await?;
        Ok(BrowserSnapshot {
            url: u.clone(),
            origin: NormalizedOrigin::parse(&u)?.as_str().to_string(),
            title: "Mock App".to_string(),
            text_excerpt: String::new(),
            interactive_elements: Vec::new(),
            redacted_field_count: 0,
        })
    }
    async fn eval_js(&self, _script: &str) -> Result<Value, BotBrowserError> {
        Ok(json!(null))
    }

    async fn click_locator(
        &self,
        locator: &SemanticLocator,
    ) -> Result<ActionOutcome, BotBrowserError> {
        let key = Self::locator_key(locator);
        if locator.role.as_deref() == Some("checkbox") || key == "in_stock_only" {
            let mut dom = self.dom.lock().await;
            dom.checkbox_checked = true;
            return Ok(ActionOutcome {
                ok: true,
                operation: "click_locator".into(),
                duration_ms: 1,
                matched_strategy: Some("role".into()),
                matched_selector: Some(key),
                x: None,
                y: None,
                extracted_text: None,
            });
        }

        let (cur_url, inputs, checked) = {
            let dom = self.dom.lock().await;
            (
                dom.current_url.clone(),
                dom.inputs.clone(),
                dom.checkbox_checked,
            )
        };
        let origin = NormalizedOrigin::parse(&cur_url)?;
        let cookie_hdr = self.cookie_header_for(&cur_url).await;

        let target_endpoint = match key.as_str() {
            "loginSubmit" => format!("{}/login", origin.as_str()),
            "tweetButtonInline" => format!("{}/compose", origin.as_str()),
            "button[type='submit']" => format!("{}/search", origin.as_str()),
            other => {
                return Err(BotBrowserError::LocatorNotFound(format!(
                    "unknown clickable locator '{other}'"
                )));
            }
        };

        let mut payload = json!(inputs);
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("in_stock_only".to_string(), json!(checked));
        }

        let mut req = self.client.post(&target_endpoint).json(&payload);
        if !cookie_hdr.is_empty() {
            req = req.header(reqwest::header::COOKIE, cookie_hdr);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| BotBrowserError::Io(e.to_string()))?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp
            .text()
            .await
            .map_err(|e| BotBrowserError::Io(e.to_string()))?;
        if !status.is_success() {
            return Err(BotBrowserError::ScriptReplay(format!(
                "POST {} failed with HTTP {}: {}",
                target_endpoint,
                status.as_u16(),
                body
            )));
        }
        self.apply_set_cookies(&target_endpoint, &headers).await;
        self.sync_dom_from_http(&target_endpoint, &body).await;

        Ok(ActionOutcome {
            ok: true,
            operation: "click_locator".into(),
            duration_ms: 1,
            matched_strategy: Some("test_id".into()),
            matched_selector: Some(key),
            x: None,
            y: None,
            extracted_text: None,
        })
    }

    async fn fill_locator(
        &self,
        locator: &SemanticLocator,
        value: &str,
        is_secret: bool,
    ) -> Result<ActionOutcome, BotBrowserError> {
        let key = Self::locator_key(locator);
        self.secret_fills_observed
            .lock()
            .await
            .push((key.clone(), value.to_string(), is_secret));
        self.dom.lock().await.inputs.insert(key.clone(), value.to_string());
        Ok(ActionOutcome {
            ok: true,
            operation: "fill_locator".into(),
            duration_ms: 1,
            matched_strategy: Some("test_id".into()),
            matched_selector: Some(key),
            x: None,
            y: None,
            extracted_text: None,
        })
    }

    async fn press_key(&self, _key: &str) -> Result<ActionOutcome, BotBrowserError> {
        Ok(ActionOutcome {
            ok: true,
            operation: "press_key".into(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn scroll(&self, _dx: f64, _dy: f64) -> Result<ActionOutcome, BotBrowserError> {
        Ok(ActionOutcome {
            ok: true,
            operation: "scroll".into(),
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
        let key = Self::locator_key(locator);
        let dom = self.dom.lock().await;
        if dom.visible_texts.contains_key(&key) {
            Ok(ActionOutcome {
                ok: true,
                operation: "wait_for_locator".into(),
                duration_ms: 1,
                matched_strategy: Some("test_id".into()),
                matched_selector: Some(key),
                x: None,
                y: None,
                extracted_text: None,
            })
        } else {
            Err(BotBrowserError::LocatorNotFound(locator.display_label()))
        }
    }

    async fn extract_text_locator(
        &self,
        locator: &SemanticLocator,
    ) -> Result<String, BotBrowserError> {
        if locator.is_sensitive_hint() {
            return Err(BotBrowserError::Cdp("sensitive field".into()));
        }
        let key = Self::locator_key(locator);
        let dom = self.dom.lock().await;
        dom.visible_texts
            .get(&key)
            .cloned()
            .ok_or_else(|| BotBrowserError::LocatorNotFound(locator.display_label()))
    }

    async fn capture_screenshot(&self) -> Result<Vec<u8>, BotBrowserError> {
        Ok(Vec::new())
    }
    async fn export_cookies(
        &self,
        _domain_filter: Option<&str>,
    ) -> Result<Vec<BrowserCookie>, BotBrowserError> {
        Ok(self.cookies.lock().await.clone())
    }
    async fn import_cookies(&self, cookies: &[BrowserCookie]) -> Result<usize, BotBrowserError> {
        let mut g = self.cookies.lock().await;
        g.extend(cookies.iter().cloned());
        Ok(cookies.len())
    }
    async fn dispatch_pointer_event(&self, _event: PointerInputEvent) -> Result<(), BotBrowserError> {
        Ok(())
    }
    async fn dispatch_key_event(&self, _event: KeyInputEvent) -> Result<(), BotBrowserError> {
        Ok(())
    }
}

async fn spawn_mock_x_server() -> (String, Arc<AtomicUsize>, tokio::sync::oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tweet_seq = Arc::new(AtomicUsize::new(100));
    let tweet_seq_srv = Arc::clone(&tweet_seq);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = listener.accept() => {
                    let seq_ref = Arc::clone(&tweet_seq_srv);
                    tokio::spawn(async move {
                        let mut buf = [0u8; 8192];
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        let raw = String::from_utf8_lossy(&buf[..n]);
                        let first_line = raw.lines().next().unwrap_or("");
                        let body_str = raw.split("\r\n\r\n").nth(1).unwrap_or("{}");
                        let has_cookies = raw.lines().any(|l| {
                            let lower = l.to_ascii_lowercase();
                            lower.starts_with("cookie:") && lower.contains("auth_token=x_tok_valid") && lower.contains("ct0=csrf_ok")
                        });

                        let (status_line, extra_hdr, json_resp) = if first_line.starts_with("GET /login") {
                            ("HTTP/1.1 200 OK", "", json!({
                                "elements": {
                                    "loginUser": "ready",
                                    "loginPass": "ready",
                                    "loginSubmit": "Sign in"
                                }
                            }))
                        } else if first_line.starts_with("POST /login") {
                            let parsed: Value = serde_json::from_str(body_str).unwrap_or_default();
                            if parsed["loginPass"] == "hunter2-secret" {
                                (
                                    "HTTP/1.1 200 OK",
                                    "Set-Cookie: auth_token=x_tok_valid; Path=/; HttpOnly\r\nSet-Cookie: ct0=csrf_ok; Path=/\r\n",
                                    json!({ "elements": { "homeTimeline": "Welcome home" } }),
                                )
                            } else {
                                ("HTTP/1.1 401 Unauthorized", "", json!({ "error": "bad password" }))
                            }
                        } else if first_line.starts_with("GET /compose") {
                            if has_cookies {
                                ("HTTP/1.1 200 OK", "", json!({
                                    "elements": {
                                        "tweetTextarea_0": "ready",
                                        "tweetButtonInline": "Post"
                                    }
                                }))
                            } else {
                                ("HTTP/1.1 401 Unauthorized", "", json!({ "error": "missing auth cookies" }))
                            }
                        } else if first_line.starts_with("POST /compose") {
                            if has_cookies {
                                let parsed: Value = serde_json::from_str(body_str).unwrap_or_default();
                                let text = parsed["tweetTextarea_0"].as_str().unwrap_or("");
                                let id = seq_ref.fetch_add(1, Ordering::SeqCst);
                                ("HTTP/1.1 200 OK", "", json!({
                                    "elements": {
                                        "tweet_status": format!("Posted: {text}"),
                                        "tweet_id": format!("tw_{id}")
                                    }
                                }))
                            } else {
                                ("HTTP/1.1 401 Unauthorized", "", json!({ "error": "missing auth cookies" }))
                            }
                        } else {
                            ("HTTP/1.1 404 Not Found", "", json!({ "error": "not found" }))
                        };

                        let body_bytes = json_resp.to_string();
                        let resp = format!(
                            "{status_line}\r\nContent-Type: application/json\r\n{extra_hdr}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body_bytes.len(),
                            body_bytes
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                    });
                }
            }
        }
    });

    (format!("http://{addr}"), tweet_seq, shutdown_tx)
}

#[tokio::test]
async fn x_like_social_workflow_trace_compile_publish_check_and_zero_llm_replay() {
    let (base_url, _tweet_seq, shutdown_tx) = spawn_mock_x_server().await;

    // 1. Record AI-driven X-like workflow into BrowserActionTrace
    let mut trace = BrowserActionTrace::new("x-session-1");
    trace.initial_url = Some(format!("{base_url}/login"));
    trace.parameter_bindings.push(TraceParameterBinding {
        param_name: "post_text".into(),
        literal_value: "Initial AI draft post".into(),
        description: "Text to publish".into(),
        required: true,
        is_secret: false,
    });
    trace.push_action(None, RecordedActionKind::Navigate { url: format!("{base_url}/login") }, Some(5));
    trace.push_action(
        Some(format!("{base_url}/login")),
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_test_id("loginUser"),
            value: "creator_handle".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: None,
        },
        Some(4),
    );
    trace.push_action(
        Some(format!("{base_url}/login")),
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_test_id("loginPass"),
            value: "hunter2-secret".into(),
            is_secret: true,
            vault_key_hint: Some("x_account:password".into()),
            parameter_name_hint: None,
        },
        Some(4),
    );
    trace.push_action(
        Some(format!("{base_url}/login")),
        RecordedActionKind::Click {
            locator: SemanticLocator::by_test_id("loginSubmit"),
        },
        Some(8),
    );
    trace.push_action(
        Some(format!("{base_url}/compose")),
        RecordedActionKind::Navigate {
            url: format!("{base_url}/compose"),
        },
        Some(5),
    );
    trace.push_action(
        Some(format!("{base_url}/compose")),
        RecordedActionKind::WaitFor {
            locator: SemanticLocator::by_test_id("tweetTextarea_0"),
            timeout_ms: 1000,
        },
        Some(2),
    );
    trace.push_action(
        Some(format!("{base_url}/compose")),
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_test_id("tweetTextarea_0"),
            value: "Initial AI draft post".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("post_text".into()),
        },
        Some(4),
    );
    trace.push_action(
        Some(format!("{base_url}/compose")),
        RecordedActionKind::Click {
            locator: SemanticLocator::by_test_id("tweetButtonInline"),
        },
        Some(8),
    );
    trace.push_action(
        Some(format!("{base_url}/compose")),
        RecordedActionKind::AssertTextContains {
            locator: SemanticLocator::by_test_id("tweet_status"),
            expected: "Initial AI draft post".into(),
        },
        Some(2),
    );
    trace.push_action(
        Some(format!("{base_url}/compose")),
        RecordedActionKind::ExtractText {
            locator: SemanticLocator::by_test_id("tweet_id"),
            output_key: "tweet_id".into(),
        },
        Some(2),
    );
    trace.mark_completed();

    // 2. Compile trace into AutomationScriptApp & verify secret stripping
    let app = ScriptCompiler::compile_from_trace(
        &trace,
        ScriptAppMetadata {
            name: "X Social Publisher".into(),
            description: "Logs into X and publishes a parameterized post".into(),
            category: "social_media".into(),
            ..Default::default()
        },
    )
    .expect("compile_from_trace");

    let serialized = serde_json::to_string(&app).unwrap();
    assert!(
        !serialized.contains("hunter2-secret"),
        "compiled app MUST NOT contain plaintext password"
    );
    validate_script_app_safe_for_publish(&app).expect("publish safety check");

    // 3. Replay deterministically with a NEW post_text parameter and VaultSecretResolver (zero LLM)
    let backend = LiveHttpTestBackend::new();
    let resolver = InMemoryVaultSecretResolver::new();
    resolver.insert("x_account:password", "hunter2-secret");

    let mut params = BTreeMap::new();
    params.insert("post_text".to_string(), "Hello from Script App v2!".to_string());

    let report = DeterministicScriptRunner::run(&backend, &app, &params, Some(&resolver), None)
        .await
        .expect("replay succeeds");
    assert!(report.success);
    assert!(!report.used_llm);
    assert_eq!(report.steps_executed, app.steps.len());
    assert_eq!(report.step_timings.len(), app.steps.len());
    assert_eq!(report.extracted_outputs.get("tweet_id").map(String::as_str), Some("tw_100"));

    // Verify CredentialVaultSecretResolver also resolves real encrypted vault credentials
    let vault = Arc::new(CredentialVault::open_in_memory(&MemoryMasterKeyStore::new()).unwrap());
    let meta = vault
        .store_credential(&base_url, "creator_handle", "hunter2-secret", None, Some("x_account"), None)
        .unwrap();
    let vault_resolver = CredentialVaultSecretResolver::new(Arc::clone(&vault));
    let mut app_with_cred_id = app.clone();
    if let ScriptStep::Fill { secret_vault_key, .. } = &mut app_with_cred_id.steps[2] {
        *secret_vault_key = Some(format!("{}:password", meta.credential_id));
    }
    let backend2 = LiveHttpTestBackend::new();
    let mut params2 = BTreeMap::new();
    params2.insert("post_text".to_string(), "Replayed via CredentialVault!".to_string());
    let report2 =
        DeterministicScriptRunner::run(&backend2, &app_with_cred_id, &params2, Some(&vault_resolver), None)
            .await
            .expect("vault replay succeeds");
    assert_eq!(report2.extracted_outputs.get("tweet_id").map(String::as_str), Some("tw_101"));

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn general_web_form_search_and_extract_workflow_replay() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = listener.accept() => {
                    tokio::spawn(async move {
                        let mut buf = [0u8; 4096];
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        let raw = String::from_utf8_lossy(&buf[..n]);
                        let first_line = raw.lines().next().unwrap_or("");
                        let body_str = raw.split("\r\n\r\n").nth(1).unwrap_or("{}");
                        let resp_json = if first_line.starts_with("GET /catalog") {
                            json!({
                                "elements": {
                                    "Search Catalog": "ready",
                                    "results_banner": "Catalog Loaded"
                                }
                            })
                        } else if first_line.starts_with("POST /search") {
                            let parsed: Value = serde_json::from_str(body_str).unwrap_or_default();
                            let q = parsed["Search Catalog"].as_str().unwrap_or("");
                            let in_stock = parsed["in_stock_only"].as_bool().unwrap_or(false);
                            json!({
                                "elements": {
                                    "results_banner": format!("Results for {q} (in_stock={in_stock})"),
                                    "top_result": format!("Item: {q} Pro")
                                }
                            })
                        } else {
                            json!({ "error": "not found" })
                        };
                        let body = resp_json.to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                    });
                }
            }
        }
    });

    let base_url = format!("http://{addr}");
    let mut trace = BrowserActionTrace::new("catalog-session");
    trace.parameter_bindings.push(TraceParameterBinding {
        param_name: "query".into(),
        literal_value: "mechanical keyboard".into(),
        description: "Catalog search query".into(),
        required: true,
        is_secret: false,
    });
    trace.push_action(None, RecordedActionKind::Navigate { url: format!("{base_url}/catalog") }, None);
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_aria_label("Search Catalog"),
            value: "mechanical keyboard".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("query".into()),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::Click {
            locator: SemanticLocator::by_role_and_text("checkbox", "In Stock Only"),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::Click {
            locator: SemanticLocator::by_css("button[type='submit']"),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::AssertVisible {
            locator: SemanticLocator::by_test_id("results_banner"),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::AssertTextContains {
            locator: SemanticLocator::by_test_id("results_banner"),
            expected: "mechanical keyboard".into(),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::ExtractText {
            locator: SemanticLocator::by_test_id("top_result"),
            output_key: "top_result".into(),
        },
        None,
    );

    let app = ScriptCompiler::compile_from_trace(
        &trace,
        ScriptAppMetadata {
            name: "Catalog Search & Extract".into(),
            category: "productivity".into(),
            ..Default::default()
        },
    )
    .expect("compile general web app");
    validate_script_app_safe_for_publish(&app).unwrap();

    let backend = LiveHttpTestBackend::new();
    let mut params = BTreeMap::new();
    params.insert("query".to_string(), "ergonomic mouse".to_string());
    let report = DeterministicScriptRunner::run(&backend, &app, &params, None, None)
        .await
        .expect("replay general web app");
    assert!(report.success);
    assert!(!report.used_llm);
    assert_eq!(
        report.extracted_outputs.get("top_result").map(String::as_str),
        Some("Item: ergonomic mouse Pro")
    );

    let _ = shutdown_tx.send(());
}

fn base_valid_app() -> AutomationScriptApp {
    AutomationScriptApp {
        schema_version: 1,
        app_id: "app_1".into(),
        slug: "safe-app".into(),
        name: "Safe App".into(),
        version: "1.0.0".into(),
        description: "A safe automation script".into(),
        category: "general".into(),
        target_domains: vec!["example.com".into()],
        parameters: vec![],
        steps: vec![ScriptStep::Navigate {
            url_template: "https://example.com".into(),
        }],
        required_capabilities: vec![],
        author_id: None,
        source_trace_summary: None,
    }
}

#[test]
fn validate_script_app_safe_for_publish_rejects_secret_leaks() {
    assert!(validate_script_app_safe_for_publish(&base_valid_app()).is_ok());

    let leak_strings = [
        "auth_token=deadbeef12345",
        "ct0=abcdef987654",
        "Authorization: Bearer abcdefghijklmnop",
        "Cookie: session=xyz",
        "Set-Cookie: sid=xyz",
        "password=my_plain_pw",
        "sk-1234567890abcdef123456",
        "ghp_1234567890abcdefghijklmnop",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
    ];

    for bad in leak_strings {
        let mut app = base_valid_app();
        app.description = format!("Contains {bad}");
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad}`, got {err:?}"
        );
    }

    // Secret parameter with non-empty default_value
    let mut app_def = base_valid_app();
    app_def.parameters.push(ScriptParameter {
        name: "api_secret".into(),
        description: "secret".into(),
        required: true,
        default_value: Some("leaked_default".into()),
        is_secret: true,
    });
    assert!(matches!(
        validate_script_app_safe_for_publish(&app_def),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));

    // Hardcoded password Fill
    let mut app_fill = base_valid_app();
    app_fill.steps.push(ScriptStep::Fill {
        locator: SemanticLocator::by_test_id("password-input"),
        value_template: "plaintext-pw".into(),
        secret_vault_key: None,
    });
    assert!(matches!(
        validate_script_app_safe_for_publish(&app_fill),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));

    // Unsafe URLs: javascript: and user:pass@
    for bad_url in ["javascript:alert(1)", "https://user:pass@example.com/"] {
        let mut app_url = base_valid_app();
        app_url.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        assert!(matches!(
            validate_script_app_safe_for_publish(&app_url),
            Err(BotBrowserError::ScriptValidation(_))
        ));
    }
}

#[tokio::test]
async fn browser_control_gate_pause_takeover_resume_and_cancel_zero_busy_wait() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = listener.accept() => {
                    tokio::spawn(async move {
                        let mut buf = [0u8; 1024];
                        let _ = sock.read(&mut buf).await;
                        let body = json!({ "elements": { "done_label": "Complete" } }).to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                    });
                }
            }
        }
    });

    let base_url = format!("http://{addr}");
    let app = AutomationScriptApp {
        schema_version: 1,
        app_id: "app_gate".into(),
        slug: "gate-test".into(),
        name: "Gate Test".into(),
        version: "1.0.0".into(),
        description: "Test takeover gate".into(),
        category: "general".into(),
        target_domains: vec!["127.0.0.1".into()],
        parameters: vec![],
        steps: vec![
            ScriptStep::Navigate {
                url_template: format!("{base_url}/start"),
            },
            ScriptStep::HumanCheckpoint {
                reason: "Solve captcha manually".into(),
            },
            ScriptStep::ExtractText {
                locator: SemanticLocator::by_test_id("done_label"),
                output_key: "done".into(),
            },
        ],
        required_capabilities: vec![],
        author_id: None,
        source_trace_summary: None,
    };

    // 1. Pause -> UserTakeover -> Resume -> Complete
    let gate = Arc::new(BrowserControlGate::new());
    let mut rx = gate.subscribe();
    let backend = Arc::new(LiveHttpTestBackend::new());

    let gate_runner = Arc::clone(&gate);
    let backend_runner = Arc::clone(&backend);
    let app_runner = app.clone();
    let handle = tokio::spawn(async move {
        DeterministicScriptRunner::run(
            backend_runner.as_ref(),
            &app_runner,
            &BTreeMap::new(),
            None,
            Some(&gate_runner),
        )
        .await
    });

    // Wait reactively on watch::Receiver for WaitingForTakeover
    rx.wait_for(|m| matches!(m, BrowserControlMode::WaitingForTakeover { .. }))
        .await
        .unwrap();
    assert!(!handle.is_finished(), "runner must suspend at HumanCheckpoint");

    gate.user_take_over();
    assert!(matches!(
        gate.mode(),
        BrowserControlMode::UserTakeover { paused_at_step: 1 }
    ));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!handle.is_finished(), "runner must stay suspended during UserTakeover");

    gate.user_resume_ai();
    let report = handle.await.unwrap().expect("resumed replay succeeds");
    assert!(report.success);
    assert_eq!(report.extracted_outputs.get("done").map(String::as_str), Some("Complete"));
    assert_eq!(gate.mode(), BrowserControlMode::Idle);

    // 2. Pause -> UserCancel -> ControlCancelled
    let gate2 = Arc::new(BrowserControlGate::new());
    let mut rx2 = gate2.subscribe();
    let backend2 = Arc::new(LiveHttpTestBackend::new());
    let gate2_runner = Arc::clone(&gate2);
    let handle2 = tokio::spawn(async move {
        DeterministicScriptRunner::run(
            backend2.as_ref(),
            &app,
            &BTreeMap::new(),
            None,
            Some(&gate2_runner),
        )
        .await
    });
    rx2.wait_for(|m| matches!(m, BrowserControlMode::WaitingForTakeover { .. }))
        .await
        .unwrap();
    gate2.user_cancel_ai();
    let err = handle2.await.unwrap().unwrap_err();
    assert!(matches!(err, BotBrowserError::ControlCancelled(_)));

    let _ = shutdown_tx.send(());
}

#[test]
fn script_compiler_rejects_empty_or_invalid_traces() {
    let empty_trace = BrowserActionTrace::new("s1");
    assert!(matches!(
        ScriptCompiler::compile_from_trace(
            &empty_trace,
            ScriptAppMetadata {
                name: "Valid Name".into(),
                ..Default::default()
            }
        ),
        Err(BotBrowserError::ScriptCompilation(_))
    ));

    let mut valid_trace = BrowserActionTrace::new("s2");
    valid_trace.push_action(
        None,
        RecordedActionKind::Navigate {
            url: "https://example.com".into(),
        },
        None,
    );
    assert!(matches!(
        ScriptCompiler::compile_from_trace(
            &valid_trace,
            ScriptAppMetadata {
                name: "   ".into(),
                ..Default::default()
            }
        ),
        Err(BotBrowserError::ScriptCompilation(_))
    ));

    let mut js_trace = BrowserActionTrace::new("s3");
    js_trace.push_action(
        None,
        RecordedActionKind::Navigate {
            url: "javascript:alert(1)".into(),
        },
        None,
    );
    assert!(matches!(
        ScriptCompiler::compile_from_trace(
            &js_trace,
            ScriptAppMetadata {
                name: "Bad URL".into(),
                ..Default::default()
            }
        ),
        Err(BotBrowserError::ScriptCompilation(_))
    ));
}

#[test]
fn validate_script_app_catches_url_encoded_json_query_fragment_slug_role_and_vault_bypasses() {
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";

    // 1. Navigate URL bypasses (percent-encoding, query tokens, fragment tokens, JWT/API keys)
    let bad_urls = [
        format!("https://example.com/?Authorization=Bearer%20{jwt}"),
        "https://example.com/?x=auth_token%3Dsecret123456".to_string(),
        "https://example.com/?k=sk-1234567890abcdef123456".to_string(),
        "https://example.com/?t=ghp_1234567890abcdefghijklmnop".to_string(),
        format!("https://example.com/?jwt={jwt}"),
        "https://example.com/?token=super_secret_token_value".to_string(),
        "https://example.com/#token=super_secret_token_value".to_string(),
        "https://example.com/?secret=super_secret_value".to_string(),
    ];
    for url in bad_urls {
        let mut app = base_valid_app();
        app.steps = vec![ScriptStep::Navigate { url_template: url.clone() }];
        assert!(
            matches!(
                validate_script_app_safe_for_publish(&app),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "expected SecretLeakDetected for Navigate URL `{url}`"
        );
    }

    // 2. Description JSON / Basic Auth / x-api-key bypasses
    for bad_desc in [
        r#"{"auth_token": "deadbeef12345", "ct0": "abcdef987654"}"#,
        r#"{"Cookie": "session=xyz123"}"#,
        "Authorization: Basic dXNlcjpwYXNz",
        "x-api-key=sk-ant-api03-1234567890123456",
    ] {
        let mut app = base_valid_app();
        app.description = bad_desc.into();
        assert!(
            matches!(
                validate_script_app_safe_for_publish(&app),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "expected SecretLeakDetected for description `{bad_desc}`"
        );
    }

    // 3. app.slug & locator.role scanning
    let mut bad_slug = base_valid_app();
    bad_slug.slug = "ghp_1234567890abcdefghijklmnop".into();
    assert!(matches!(
        validate_script_app_safe_for_publish(&bad_slug),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));

    let mut bad_role = base_valid_app();
    bad_role.steps.push(ScriptStep::Click {
        locator: SemanticLocator {
            role: Some("auth_token=deadbeef12345".into()),
            ..Default::default()
        },
    });
    assert!(matches!(
        validate_script_app_safe_for_publish(&bad_role),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));

    // 4. Fill.secret_vault_key allows benign identifiers, rejects raw secret payloads
    let mut benign_vk = base_valid_app();
    for vk in ["x_auth_token", "login.password", "x_account:password"] {
        benign_vk.steps = vec![ScriptStep::Fill {
            locator: SemanticLocator::by_test_id("password-input"),
            value_template: String::new(),
            secret_vault_key: Some(vk.into()),
        }];
        validate_script_app_safe_for_publish(&benign_vk)
            .unwrap_or_else(|e| panic!("benign vault key `{vk}` should be allowed, got {e:?}"));
    }

    let mut leaky_vk = base_valid_app();
    leaky_vk.steps = vec![ScriptStep::Fill {
        locator: SemanticLocator::by_test_id("password-input"),
        value_template: String::new(),
        secret_vault_key: Some("auth_token=deadbeef12345".into()),
    }];
    assert!(matches!(
        validate_script_app_safe_for_publish(&leaky_vk),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));

    // 5. Dummy secret_vault_key + non-secret param default_value MUST be rejected
    let mut dummy_vk = base_valid_app();
    dummy_vk.parameters.push(ScriptParameter {
        name: "pw".into(),
        description: "not marked secret".into(),
        required: true,
        default_value: Some("leaked_plaintext_password_123".into()),
        is_secret: false,
    });
    dummy_vk.steps.push(ScriptStep::Fill {
        locator: SemanticLocator::by_test_id("password-input"),
        value_template: "{{pw}}".into(),
        secret_vault_key: Some("dummy_key".into()),
    });
    assert!(matches!(
        validate_script_app_safe_for_publish(&dummy_vk),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));

    // 6. Navigate ?auth_token={{harmless}} with non-secret default_value MUST be rejected
    let mut nav_harmless = base_valid_app();
    nav_harmless.parameters.push(ScriptParameter {
        name: "harmless".into(),
        description: "non-secret".into(),
        required: true,
        default_value: Some("raw_token_leak".into()),
        is_secret: false,
    });
    nav_harmless.steps = vec![ScriptStep::Navigate {
        url_template: "https://example.com/?auth_token={{harmless}}".into(),
    }];
    assert!(matches!(
        validate_script_app_safe_for_publish(&nav_harmless),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));
}

#[tokio::test]
async fn dummy_vault_fallback_never_replays_non_secret_default_value() {
    let mut app = base_valid_app();
    app.parameters.push(ScriptParameter {
        name: "pw".into(),
        description: "sneaky default".into(),
        required: true,
        default_value: Some("leaked_plaintext_password_123".into()),
        is_secret: false,
    });
    app.steps = vec![ScriptStep::Fill {
        locator: SemanticLocator::by_test_id("password-input"),
        value_template: "{{pw}}".into(),
        secret_vault_key: Some("missing.vault.key".into()),
    }];

    let backend = LiveHttpTestBackend::new();
    let err = DeterministicScriptRunner::run(&backend, &app, &BTreeMap::new(), None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, BotBrowserError::CredentialNotFound(_)),
        "expected CredentialNotFound instead of falling back to default_value, got {err:?}"
    );
    assert!(
        backend.secret_fills_observed.lock().await.is_empty(),
        "default_value must never be filled into the page"
    );
}

#[test]
fn script_compiler_preserves_fragment_encoding_strips_fragment_tokens_and_handles_collisions() {
    let mut trace = BrowserActionTrace::new("nav-frag-session");
    trace.push_action(
        None,
        RecordedActionKind::Navigate {
            url: "https://example.com/cb?auth_token=raw&next=https%3A%2F%2Ffoo.com%2Fp%3Fa%3D1%26b%3D2#section".into(),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::Navigate {
            url: "https://example.com/oauth#token=secret12345&tab=profile".into(),
        },
        None,
    );
    // Colliding parameter_name_hint across non-secret and secret Fill steps
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_css("#user"),
            value: "alice".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("field".into()),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_test_id("password"),
            value: "super-secret-pw".into(),
            is_secret: true,
            vault_key_hint: None,
            parameter_name_hint: Some("field".into()),
        },
        None,
    );
    // Punctuation-only parameter_name_hint must fall back cleanly
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_test_id("pin-input"),
            value: "9999".into(),
            is_secret: true,
            vault_key_hint: None,
            parameter_name_hint: Some("!!!".into()),
        },
        None,
    );
    // Non-secret Fill with sensitive parameter_name_hint ("password") upgrades to secret
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_css("#generic"),
            value: "raw_pw_val".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("password".into()),
        },
        None,
    );
    // Non-secret Fill containing token= or JSON auth_token/Cookie gets redacted
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_css("#notes"),
            value: r#"{"auth_token": "abc123"}"#.into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: None,
        },
        None,
    );

    let app = ScriptCompiler::compile_from_trace(
        &trace,
        ScriptAppMetadata {
            name: "Nav & Collision Test".into(),
            parameter_overrides: vec![ScriptParameter {
                name: "foo".into(),
                description: "non-secret override".into(),
                required: false,
                default_value: Some("harmless".into()),
                is_secret: false,
            }],
            ..Default::default()
        },
    )
    .expect("compile_from_trace must succeed");

    validate_script_app_safe_for_publish(&app).expect("compiled app must be publish-safe");

    // Verify Navigate #1 preserves #section AFTER query AND preserves percent-encoding
    assert_eq!(
        app.steps[0],
        ScriptStep::Navigate {
            url_template: "https://example.com/cb?auth_token={{auth_token}}&next=https%3A%2F%2Ffoo.com%2Fp%3Fa%3D1%26b%3D2#section".into()
        }
    );
    // Verify Navigate #2 strips #token=...
    assert_eq!(
        app.steps[1],
        ScriptStep::Navigate {
            url_template: "https://example.com/oauth#token={{token}}&tab=profile".into()
        }
    );
    // Verify colliding param names keep non-secret "field" and disambiguate secret param
    let field_param = app.parameters.iter().find(|p| p.name == "field").unwrap();
    assert!(!field_param.is_secret);
    assert_eq!(field_param.default_value.as_deref(), Some("alice"));
    if let ScriptStep::Fill {
        value_template,
        secret_vault_key,
        ..
    } = &app.steps[3]
    {
        assert_ne!(value_template, "{{field}}");
        assert!(secret_vault_key.is_some());
    } else {
        panic!("expected Fill step");
    }

    // Verify serialized app contains zero raw secret inputs
    let json = serde_json::to_string(&app).unwrap();
    for leaked in ["raw_pw_val", "super-secret-pw", "secret12345", "abc123"] {
        assert!(!json.contains(leaked), "compiled app leaked `{leaked}`");
    }
}

#[tokio::test]
async fn control_gate_late_subscribe_cancel_race_extract_failure_and_assert_secret_redaction() {
    // 1. Late subscribe after request_takeover immediately sees WaitingForTakeover
    let gate = BrowserControlGate::new();
    gate.start_ai("running");
    gate.request_takeover("Manual 2FA required");
    let mut late_rx = gate.subscribe();
    tokio::time::timeout(
        Duration::from_millis(200),
        late_rx.wait_for(|m| matches!(m, BrowserControlMode::WaitingForTakeover { .. })),
    )
    .await
    .expect("late subscribe must not hang")
    .unwrap();

    // 2. user_cancel_ai before run() starts must not be overwritten by start_ai()
    let cancel_gate = BrowserControlGate::new();
    cancel_gate.user_cancel_ai();
    let simple_app = base_valid_app();
    let backend = LiveHttpTestBackend::new();
    let cancel_err = DeterministicScriptRunner::run(
        &backend,
        &simple_app,
        &BTreeMap::new(),
        None,
        Some(&cancel_gate),
    )
    .await
    .unwrap_err();
    assert!(matches!(cancel_err, BotBrowserError::ControlCancelled(_)));
    cancel_gate.reset();
    assert!(!cancel_gate.is_cancelled());

    // 3. ExtractText failure returns gate.mode() to Idle and records failed StepTimingMetric
    struct FailingExtractTransport;
    #[async_trait]
    impl CdpTransport for FailingExtractTransport {
        async fn send_command(
            &self,
            method: &str,
            params: Value,
        ) -> Result<Value, BotBrowserError> {
            if method == "Runtime.evaluate" {
                let expr = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if expr.contains("missing_el") {
                    return Ok(json!({ "result": { "value": json!({ "found": false }).to_string() } }));
                }
                if expr.contains("banner_el") {
                    return Ok(json!({
                        "result": {
                            "value": json!({
                                "found": true,
                                "strategy": "test_id",
                                "tag": "div",
                                "is_sensitive": false,
                                "selector": "[data-testid=\"banner_el\"]",
                                "rect": { "x": 0.0, "y": 0.0, "width": 50.0, "height": 20.0, "device_pixel_ratio": 1.0 },
                                "center_x": 25.0,
                                "center_y": 10.0,
                                "text": "actual_banner_text"
                            }).to_string()
                        }
                    }));
                }
            }
            Ok(json!({}))
        }
        async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
            Ok("mock://fail-extract".into())
        }
    }

    let cdp_backend = CdpBrowserBackend::new_mock(Arc::new(FailingExtractTransport));
    let fail_gate = BrowserControlGate::new();
    let mut fail_app = base_valid_app();
    fail_app.steps = vec![ScriptStep::ExtractText {
        locator: SemanticLocator::by_test_id("missing_el"),
        output_key: "out".into(),
    }];
    let extract_report = DeterministicScriptRunner::run(
        &cdp_backend,
        &fail_app,
        &BTreeMap::new(),
        None,
        Some(&fail_gate),
    )
    .await
    .expect("mid-script step failure returns Ok(ScriptReplayReport { success: false })");
    assert!(!extract_report.success);
    assert!(!extract_report.used_llm);
    assert_eq!(extract_report.steps_executed, 1);
    assert_eq!(extract_report.step_timings.len(), 1);
    assert!(!extract_report.step_timings[0].success);
    assert_eq!(extract_report.step_timings[0].step_index, Some(0));
    let extract_err_str = extract_report.error.as_deref().unwrap_or("");
    assert!(
        extract_err_str.contains("locator not found") && extract_err_str.contains("missing_el"),
        "expected locator not found error in report.error, got: {extract_err_str}"
    );
    assert_eq!(
        fail_gate.mode(),
        BrowserControlMode::Idle,
        "gate.mode() MUST return to Idle after ExtractText error"
    );
    let metrics = cdp_backend.performance_metrics();
    assert!(
        metrics
            .recent_steps
            .iter()
            .any(|m| m.operation == "extract_text_locator" && !m.success),
        "expected failed StepTimingMetric recorded in backend telemetry"
    );

    // 4. AssertTextContains secret mismatch redacts expected value in report error & step_timings
    let mut assert_app = base_valid_app();
    assert_app.parameters.push(ScriptParameter {
        name: "secret_code".into(),
        description: "2FA code".into(),
        required: true,
        default_value: None,
        is_secret: true,
    });
    assert_app.steps = vec![ScriptStep::AssertTextContains {
        locator: SemanticLocator::by_test_id("banner_el"),
        expected_template: "{{secret_code}}".into(),
    }];
    let mut secret_params = BTreeMap::new();
    secret_params.insert("secret_code".to_string(), "TOP_SECRET_9999".to_string());
    let assert_report = DeterministicScriptRunner::run(
        &cdp_backend,
        &assert_app,
        &secret_params,
        None,
        Some(&fail_gate),
    )
    .await
    .expect("AssertTextContains failure returns Ok(ScriptReplayReport { success: false })");
    assert!(!assert_report.success);
    assert_eq!(assert_report.step_timings.len(), 1);
    assert!(!assert_report.step_timings[0].success);
    let err_str = assert_report.error.as_deref().unwrap_or("");
    let detail_str = assert_report.step_timings[0].detail.as_deref().unwrap_or("");
    assert!(
        !err_str.contains("TOP_SECRET_9999") && !detail_str.contains("TOP_SECRET_9999"),
        "AssertTextContains report leaked secret: error={err_str}, detail={detail_str}"
    );
    assert!(
        err_str.contains("[REDACTED]") && detail_str.contains("[REDACTED]"),
        "expected [REDACTED] in error & step detail: error={err_str}, detail={detail_str}"
    );
    assert_eq!(fail_gate.mode(), BrowserControlMode::Idle);
}

#[tokio::test]
async fn orchestrator_run_script_app_resolves_credential_vault_and_resets_gate() {
    struct OrchestratorMockTransport {
        inserted: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl CdpTransport for OrchestratorMockTransport {
        async fn send_command(
            &self,
            method: &str,
            params: Value,
        ) -> Result<Value, BotBrowserError> {
            match method {
                "Page.navigate" => Ok(json!({ "frameId": "main" })),
                "Input.insertText" => {
                    if let Some(txt) = params.get("text").and_then(|v| v.as_str()) {
                        self.inserted.lock().await.push(txt.to_string());
                    }
                    Ok(json!({}))
                }
                "Runtime.evaluate" => {
                    let expr = params
                        .get("expression")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if expr.contains("document.readyState") {
                        return Ok(json!({ "result": { "value": "complete" } }));
                    }
                    if expr.contains("window.location.href") {
                        return Ok(json!({
                            "result": { "value": "https://example.com/login" }
                        }));
                    }
                    if expr.contains("shouldFocus") && expr.contains("loc") {
                        return Ok(json!({
                            "result": {
                                "value": json!({
                                    "found": true,
                                    "strategy": "test_id",
                                    "tag": "input",
                                    "is_sensitive": true,
                                    "selector": "[data-testid=\"login-pass\"]",
                                    "rect": { "x": 0.0, "y": 0.0, "width": 100.0, "height": 20.0, "device_pixel_ratio": 1.0 },
                                    "center_x": 50.0,
                                    "center_y": 10.0,
                                    "text": ""
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
            Ok("mock://orch".into())
        }
    }

    struct OrchFactory(Arc<OrchestratorMockTransport>);
    #[async_trait]
    impl SessionTransportFactory for OrchFactory {
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

    let tmp = TempDir::new().unwrap();
    let vault = Arc::new(CredentialVault::open_in_memory(&MemoryMasterKeyStore::new()).unwrap());
    let cred_meta = vault
        .store_credential(
            "https://example.com",
            "alice",
            "vault-secret-pw-42",
            None,
            Some("example_login"),
            None,
        )
        .unwrap();

    let inserted = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(OrchestratorMockTransport {
        inserted: Arc::clone(&inserted),
    });
    let orch = BotBrowserOrchestrator::with_transport_factory(
        OrchestratorConfig::new(tmp.path()),
        vault,
        Arc::new(OrchFactory(transport)),
    );

    let app = AutomationScriptApp {
        schema_version: 1,
        app_id: "app_orch_vault".into(),
        slug: "orch-vault".into(),
        name: "Orch Vault Test".into(),
        version: "1.0.0".into(),
        description: "Tests BotBrowserOrchestrator::run_script_app".into(),
        category: "general".into(),
        target_domains: vec!["example.com".into()],
        parameters: vec![],
        steps: vec![
            ScriptStep::Navigate {
                url_template: "https://example.com/login".into(),
            },
            ScriptStep::Fill {
                locator: SemanticLocator::by_test_id("login-pass"),
                value_template: String::new(),
                secret_vault_key: Some(format!("{}:password", cred_meta.credential_id)),
            },
        ],
        required_capabilities: vec![],
        author_id: None,
        source_trace_summary: None,
    };

    let gate = BrowserControlGate::new();
    let report = orch
        .run_script_app("sess-orch-1", &app, &BTreeMap::new(), Some(&gate))
        .await
        .expect("run_script_app must succeed");
    assert!(report.success);
    assert!(!report.used_llm);
    assert_eq!(gate.mode(), BrowserControlMode::Idle);
    assert_eq!(
        inserted.lock().await.as_slice(),
        &["vault-secret-pw-42".to_string()]
    );
}

#[tokio::test]
async fn validator_catches_double_encoding_multiline_json_prefixed_keys_pin_passcode_and_allows_vault_hint_suffix()
{
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";

    // 1. Double-encoded URL and JSON-escaped payloads
    let double_encoded_urls = [
        format!("https://example.com/?next=https%3A%2F%2Fevil.com%2F%3Fq%3DBearer%2520{jwt}"),
        "https://example.com/?next=https%3A%2F%2Fevil.com%2F%3Fq%3DBearer%2520opaque_secret_1234".to_string(),
        "https://example.com/?q=auth_token%253Dsecret123456".to_string(),
        "https://example.com/?q=sk%252d1234567890abcdef123456".to_string(),
        "https://example.com/?q=%2561uth_token%253Dsecret123456".to_string(),
    ];
    for url in double_encoded_urls {
        let mut app = base_valid_app();
        app.steps = vec![ScriptStep::Navigate { url_template: url.clone() }];
        assert!(
            matches!(
                validate_script_app_safe_for_publish(&app),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "expected SecretLeakDetected for double-encoded URL `{url}`"
        );
    }

    for bad_desc in [
        r#"{\\\"auth_token\\\": \\\"deadbeef12345\\\"}"#,
        r#"auth_token\\u003ddeadbeef12345"#,
        // 2. Multiline pretty-printed JSON with \n around ':'
        "{\n  \"auth_token\":\n  \"deadbeef12345\"\n}",
        "{\n  \"auth_token\"\n  : \"deadbeef12345\"\n}",
        // 3. Prefixed compound keys and HTTP headers
        "https://example.com/?x_auth_token=deadbeef12345",
        "https://example.com/?oauth_token=deadbeef12345",
        "https://example.com/?user_password=hunter2_secret",
        "https://example.com/?my_secret=deadbeef12345",
        "https://example.com/?custom_secret=deadbeef12345",
        "X-Auth-Token: deadbeef12345",
        "Proxy-Authorization: Basic dXNlcjpwYXNz",
    ] {
        let mut app = base_valid_app();
        app.description = bad_desc.to_string();
        assert!(
            matches!(
                validate_script_app_safe_for_publish(&app),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "expected SecretLeakDetected for description `{bad_desc}`"
        );
    }

    // Benign prose without KV assignment must still pass without false positives
    let mut benign_app = base_valid_app();
    benign_app.description =
        "Search uses the Basic plan flow, checks no token exists, and resets password-reset docs."
            .into();
    validate_script_app_safe_for_publish(&benign_app)
        .expect("benign natural prose must not false-positive");

    // 4. pin and passcode parameter defaults rejected + AssertTextContains redaction
    for pname in ["pin", "user_pin", "passcode", "unlock_passcode"] {
        let mut pin_app = base_valid_app();
        pin_app.parameters.push(ScriptParameter {
            name: pname.to_string(),
            description: "PIN".into(),
            required: true,
            default_value: Some("8421".into()),
            is_secret: false,
        });
        assert!(
            matches!(
                validate_script_app_safe_for_publish(&pin_app),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "parameter '{pname}' with is_secret=false/default_value must be rejected"
        );
    }

    let backend = LiveHttpTestBackend::new();
    backend
        .dom
        .lock()
        .await
        .visible_texts
        .insert("banner_el".to_string(), "actual_text".to_string());
    let mut assert_pin_app = base_valid_app();
    // Even without is_secret=true declared on the parameter, is_sensitive_param_name protects pin/passcode
    assert_pin_app.parameters.push(ScriptParameter {
        name: "pin".into(),
        description: "PIN".into(),
        required: false,
        default_value: None,
        is_secret: false,
    });
    assert_pin_app.parameters.push(ScriptParameter {
        name: "passcode".into(),
        description: "Passcode".into(),
        required: true,
        default_value: None,
        is_secret: true,
    });
    for (param_key, secret_val) in [("pin", "SECRET_PIN_8421"), ("passcode", "SECRET_PASS_9102")] {
        assert_pin_app.steps = vec![ScriptStep::AssertTextContains {
            locator: SemanticLocator::by_test_id("banner_el"),
            expected_template: format!("{{{{{param_key}}}}}"),
        }];
        let mut pmap = BTreeMap::new();
        pmap.insert(param_key.to_string(), secret_val.to_string());
        let report = DeterministicScriptRunner::run(&backend, &assert_pin_app, &pmap, None, None)
            .await
            .expect("AssertTextContains mismatch returns Ok(ScriptReplayReport { success: false })");
        let err_str = report.error.as_deref().unwrap_or("");
        assert!(
            !report.success
                && !err_str.contains(secret_val)
                && err_str.contains("[REDACTED]"),
            "AssertTextContains report leaked {param_key}: {err_str}"
        );
    }

    // 5. Fill.secret_vault_key allowlists account_hint:field suffixes, while rejecting raw tokens
    for allowed_vk in [
        "auth_token:password",
        "api_key:password",
        "token:password",
        "secret:totp",
        "x_auth_token:username",
    ] {
        let mut ok_app = base_valid_app();
        ok_app.steps = vec![ScriptStep::Fill {
            locator: SemanticLocator::by_test_id("password-input"),
            value_template: String::new(),
            secret_vault_key: Some(allowed_vk.to_string()),
        }];
        validate_script_app_safe_for_publish(&ok_app)
            .unwrap_or_else(|e| panic!("legit secret_vault_key `{allowed_vk}` failed: {e:?}"));
    }

    for bad_vk in [
        "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
        "Basic dXNlcjpwYXNz",
        "sk-1234567890abcdef123456",
        "ghp_1234567890abcdefghijklmnop",
        "AKIAIOSFODNN7EXAMPLE",
        "auth_token=deadbeef12345:password",
        "custom_key=deadbeef12345",
    ] {
        let mut bad_app = base_valid_app();
        bad_app.steps = vec![ScriptStep::Fill {
            locator: SemanticLocator::by_test_id("password-input"),
            value_template: String::new(),
            secret_vault_key: Some(bad_vk.to_string()),
        }];
        assert!(
            matches!(
                validate_script_app_safe_for_publish(&bad_app),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "raw secret_vault_key `{bad_vk}` must be rejected"
        );
    }
}

#[test]
fn script_compiler_preserves_path_placeholders_redacts_encoded_fragments_and_disambiguates_non_secret_hints()
{
    let mut trace = BrowserActionTrace::new("compiler-edge-session");
    trace.parameter_bindings.push(TraceParameterBinding {
        param_name: "user".into(),
        literal_value: "alice".into(),
        description: "Username".into(),
        required: true,
        is_secret: false,
    });
    // 1. Path placeholder + query secret + fragment placeholder
    trace.push_action(
        None,
        RecordedActionKind::Navigate {
            url: "https://example.com/u/alice?auth_token=raw_secret_tok&tab=1#sec-alice".into(),
        },
        None,
    );
    // 2. Percent-encoded KV secret inside fragment
    trace.push_action(
        None,
        RecordedActionKind::Navigate {
            url: "https://example.com/oauth#auth_token%3Dsecret12345&tab=1".into(),
        },
        None,
    );
    // 3. Unkeyed API secret inside multi-part fragment
    trace.push_action(
        None,
        RecordedActionKind::Navigate {
            url: "https://example.com/oauth#x=1&sk-1234567890abcdef123456".into(),
        },
        None,
    );
    // 4. Duplicate non-secret parameter_name_hint with distinct literal values
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_css("#q1"),
            value: "cats".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("query".into()),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_css("#q2"),
            value: "dogs".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("query".into()),
        },
        None,
    );
    // 5. Same non-secret hint and identical literal value reuses the same parameter
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_css("#q3"),
            value: "cats".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("query".into()),
        },
        None,
    );
    // 6. pin / passcode Fill hints are compiled as secret parameters
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_css("#field_a"),
            value: "4321".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("pin".into()),
        },
        None,
    );
    trace.push_action(
        None,
        RecordedActionKind::Fill {
            locator: SemanticLocator::by_css("#field_b"),
            value: "9876".into(),
            is_secret: false,
            vault_key_hint: None,
            parameter_name_hint: Some("passcode".into()),
        },
        None,
    );

    let app = ScriptCompiler::compile_from_trace(
        &trace,
        ScriptAppMetadata {
            name: "Compiler Edge Cases".into(),
            ..Default::default()
        },
    )
    .expect("compile_from_trace must succeed");

    assert_eq!(
        app.steps[0],
        ScriptStep::Navigate {
            url_template: "https://example.com/u/{{user}}?auth_token={{auth_token}}&tab=1#sec-{{user}}".into()
        }
    );
    assert_eq!(
        app.steps[1],
        ScriptStep::Navigate {
            url_template: "https://example.com/oauth#auth_token={{auth_token}}&tab=1".into()
        }
    );
    assert_eq!(
        app.steps[2],
        ScriptStep::Navigate {
            url_template: "https://example.com/oauth#x=1&{{secret_fragment}}".into()
        }
    );

    let q1 = match &app.steps[3] {
        ScriptStep::Fill { value_template, .. } => value_template.clone(),
        other => panic!("unexpected step[3]: {other:?}"),
    };
    let q2 = match &app.steps[4] {
        ScriptStep::Fill { value_template, .. } => value_template.clone(),
        other => panic!("unexpected step[4]: {other:?}"),
    };
    let q3 = match &app.steps[5] {
        ScriptStep::Fill { value_template, .. } => value_template.clone(),
        other => panic!("unexpected step[5]: {other:?}"),
    };
    assert_eq!(q1, "{{query}}");
    assert_ne!(q2, "{{query}}", "distinct literal must disambiguate");
    assert_eq!(q3, "{{query}}", "matching literal reuses existing param");

    let p_query = app.parameters.iter().find(|p| p.name == "query").unwrap();
    assert_eq!(p_query.default_value.as_deref(), Some("cats"));
    let p_query_2 = app
        .parameters
        .iter()
        .find(|p| p.name == q2.trim_matches(|c| c == '{' || c == '}'))
        .unwrap();
    assert_eq!(p_query_2.default_value.as_deref(), Some("dogs"));

    for pname in ["pin", "passcode"] {
        let p = app
            .parameters
            .iter()
            .find(|p| p.name == pname)
            .unwrap_or_else(|| panic!("missing parameter {pname}"));
        assert!(p.is_secret, "{pname} must be is_secret=true");
        assert_eq!(p.default_value, None);
    }
}

#[tokio::test]
async fn credential_vault_resolver_supports_dot_totp_and_enforces_confirm_every_time_policy() {
    let vault = Arc::new(CredentialVault::open_in_memory(&MemoryMasterKeyStore::new()).unwrap());
    // Standard AutoReuse credential with TOTP secret
    let _example_meta = vault
        .store_credential(
            "https://example.com",
            "alice",
            "pw_alice_123",
            Some("JBSWY3DPEHPK3PXP"),
            Some("alice_acct"),
            Some(ReusePolicy::AutoReuse),
        )
        .unwrap();
    // High-sensitivity origin defaults to ConfirmEveryTime
    let stripe_meta = vault
        .store_credential(
            "https://stripe.com",
            "billing@acme.test",
            "stripe_pw_999",
            Some("JBSWY3DPEHPK3PXP"),
            Some("stripe_acct"),
            None,
        )
        .unwrap();
    assert_eq!(stripe_meta.reuse_policy, ReusePolicy::ConfirmEveryTime);

    let resolver = CredentialVaultSecretResolver::new(Arc::clone(&vault));

    // "secret.totp" must resolve a 6-digit TOTP code (not the password!)
    let totp = resolver
        .resolve_secret("secret.totp", Some("https://example.com"))
        .await
        .expect("secret.totp must resolve");
    assert_ne!(totp.as_str(), "pw_alice_123");
    assert_eq!(totp.len(), 6);
    assert!(totp.chars().all(|c| c.is_ascii_digit()));

    let uname = resolver
        .resolve_secret("secret.username", Some("https://example.com"))
        .await
        .expect("secret.username must resolve");
    assert_eq!(uname.as_str(), "alice");

    // ConfirmEveryTime credential MUST fail with ApprovalRequired by default
    let err = resolver
        .resolve_secret(&format!("{}:password", stripe_meta.credential_id), Some("https://stripe.com"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BotBrowserError::ApprovalRequired { ref credential_id, .. } if credential_id == &stripe_meta.credential_id),
        "expected ApprovalRequired for ConfirmEveryTime, got {err:?}"
    );

    // Explicitly approved resolver succeeds
    let approved_resolver =
        CredentialVaultSecretResolver::new(Arc::clone(&vault)).with_confirm_every_time_approved(true);
    let stripe_pw = approved_resolver
        .resolve_secret(&format!("{}:password", stripe_meta.credential_id), Some("https://stripe.com"))
        .await
        .expect("approved resolver must succeed");
    assert_eq!(stripe_pw.as_str(), "stripe_pw_999");
}

#[tokio::test]
async fn runner_drop_abort_while_paused_resets_control_gate_to_idle() {
    let gate = Arc::new(BrowserControlGate::new());
    let mut rx = gate.subscribe();
    let backend = Arc::new(LiveHttpTestBackend::new());
    let mut app = base_valid_app();
    app.steps = vec![ScriptStep::HumanCheckpoint {
        reason: "2FA approval".into(),
    }];

    let gate_runner = Arc::clone(&gate);
    let app_runner = app.clone();
    let handle = tokio::spawn(async move {
        DeterministicScriptRunner::run(
            backend.as_ref(),
            &app_runner,
            &BTreeMap::new(),
            None,
            Some(&gate_runner),
        )
        .await
    });

    rx.wait_for(|m| matches!(m, BrowserControlMode::WaitingForTakeover { .. }))
        .await
        .unwrap();
    assert!(matches!(gate.mode(), BrowserControlMode::WaitingForTakeover { .. }));

    handle.abort();
    let join_err = handle.await.unwrap_err();
    assert!(join_err.is_cancelled());

    assert_eq!(
        gate.mode(),
        BrowserControlMode::Idle,
        "aborting/dropping runner mid-pause must reset gate.mode() to Idle"
    );
}

#[test]
fn validate_script_app_allows_standalone_bearer_english_nouns_but_rejects_leaks() {
    for ok_desc in [
        "Bearer authentication helper for OAuth apps",
        "Bearer authorization workflow guide",
        "Bearer documentation and specification helper",
        "Uses Bearer authentication.",
        "Bearer credential helper",
        "Bearer auth helper",
    ] {
        let mut app = base_valid_app();
        app.description = ok_desc.to_string();
        assert!(
            validate_script_app_safe_for_publish(&app).is_ok(),
            "ordinary English prose `{ok_desc}` should be allowed"
        );
    }

    // Protected secret placeholder inside parentheses/brackets or after kv colon must remain valid
    for ok_ph_desc in [
        "Bearer ({{token}})",
        "Bearer [{{token}}]",
        "Authorization: Bearer ({{token}})",
        "Bearer auth: {{token}}",
    ] {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "token".into(),
            description: "Secret token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.description = ok_ph_desc.to_string();
        assert!(
            validate_script_app_safe_for_publish(&app).is_ok(),
            "protected placeholder `{ok_ph_desc}` should be allowed"
        );
    }

    for bad_desc in [
        "Authorization: Bearer authentication",
        "Bearer abcdefghijklmnop",
        "Bearer {{query}}",
        "Bearer ({{query}})",
        "Bearer aUtHeNtIcAtIoN",
        "Bearer authentication123",
        "Bearer authentication: secret_1234567890",
        "Bearer authentication:secret_1234567890",
        "Bearer credential: tok_live_9876543210",
        "Bearer authentication = secret_1234567890",
        "Bearer (secret_1234567890)",
        "Bearer [secret_1234567890]",
        "Bearer auth: secret_1234567890",
        "Bearer cred: tok_live_9876543210",
        "Bearer bearer_tok: secret_1234567890",
    ] {
        let mut app = base_valid_app();
        app.description = bad_desc.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad_desc}`, got {err:?}"
        );
    }
}

#[tokio::test]
async fn deterministic_script_runner_preserves_step_timings_and_extracted_outputs_on_mid_script_failure()
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = listener.accept() => {
                    let mut buf = [0u8; 1024];
                    let _ = sock.read(&mut buf).await;
                    let body = r#"{"elements":{"summary_badge":"Order #4242 Ready"}}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                }
            }
        }
    });

    let start_url = format!("http://127.0.0.1:{}/start", addr.port());
    let backend = LiveHttpTestBackend::new();
    let gate = BrowserControlGate::new();

    let mut app = base_valid_app();
    app.steps = vec![
        ScriptStep::Navigate {
            url_template: start_url.clone(),
        },
        ScriptStep::ExtractText {
            locator: SemanticLocator::by_test_id("summary_badge"),
            output_key: "order_badge".into(),
        },
        ScriptStep::Click {
            locator: SemanticLocator::by_test_id("missing_step_3_btn"),
        },
        ScriptStep::Click {
            locator: SemanticLocator::by_test_id("trailing_step_4_btn"),
        },
    ];

    let report = DeterministicScriptRunner::run(&backend, &app, &BTreeMap::new(), None, Some(&gate))
        .await
        .expect("mid-script step failure must return ScriptReplayReport");

    assert!(!report.success);
    assert!(!report.used_llm);
    assert_eq!(report.steps_executed, 3);
    assert_eq!(report.step_timings.len(), 3);
    assert!(report.step_timings[0].success);
    assert!(report.step_timings[1].success);
    assert!(!report.step_timings[2].success);
    assert_eq!(report.step_timings[2].step_index, Some(2));
    assert!(
        report.step_timings[2]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("missing_step_3_btn"))
    );
    assert_eq!(
        report.extracted_outputs.get("order_badge").map(String::as_str),
        Some("Order #4242 Ready"),
        "pre-failure extracted_outputs must be preserved"
    );
    assert!(
        report
            .error
            .as_deref()
            .is_some_and(|e| e.contains("missing_step_3_btn"))
    );
    assert_eq!(
        report.final_url.as_deref(),
        Some(start_url.as_str())
    );
    assert_eq!(gate.mode(), BrowserControlMode::Idle);

    let _ = shutdown_tx.send(());
}

#[test]
fn validate_script_app_allows_query_placeholders_and_rejects_trailing_payload_and_unclosed_quote_bypasses()
{
    for ok_url in [
        "https://example.com/auth?token={{auth_token}}",
        "https://example.com/auth?token={{auth_token}}&next=/home",
        "https://example.com/auth?token={{auth_token}}#frag",
    ] {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.steps = vec![ScriptStep::Navigate {
            url_template: ok_url.to_string(),
        }];
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("valid query placeholder URL `{ok_url}` failed: {e:?}"));
    }

    for ok_desc in [
        "(token={{auth_token}})",
        "{token: {{auth_token}}}",
        "Bearer {{auth_token}}",
        "Authorization: Bearer {{auth_token}}",
        "Bearer {{api_token}}",
        "Basic {{api_token}}",
        "Basic ({{auth_token}})",
        "Basic auth guide",
        r#"{"token":""}"#,
        r#"{"token": ""}"#,
        r#"{"url_template":"https://example.com/auth?token={{auth_token}}"}"#,
        r#"{"description":"Bearer {{api_token}}"}"#,
        r#"{"url_template":"https://example.com/auth?token={{auth_token}}","next":"home"}"#,
    ] {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("valid placeholder desc `{ok_desc}` failed: {e:?}"));
    }

    for bad_text in [
        "https://example.com/auth?token=\"raw_secret",
        "https://example.com/auth?token='raw_secret",
        "https://example.com/auth?token=\"\"raw_secret_123456",
        "https://example.com/auth?token=''raw_secret_123456",
        "https://example.com/auth?token={{auth_token}}real_secret",
        "https://example.com/auth?token={{auth_token}}\"raw_secret_123456",
        "https://example.com/auth?token={{auth_token}}'raw_secret_123456",
        "https://example.com/auth?token=\"{{auth_token}}\"raw_secret_123456",
        "https://example.com/auth?token='{{auth_token}}'raw_secret_123456",
        "https://example.com/auth?token={{auth_token}}\\\"raw_secret_123456",
        "https://example.com/auth?token=real_secret{{auth_token}}",
        "Bearer {{auth_token}}raw_secret",
        "Authorization: Bearer {{auth_token}}raw_secret",
        "Authorization: Bearer {{api_token}}\"raw_secret_123456",
        "Authorization: Bearer {{api_token}}\\\"raw_secret_123456",
        "Authorization: Bearer {{api_token}}#raw_secret_123456",
        "Bearer {{api_token}}#raw_secret_123456",
        "Bearer \"{{api_token}}\"raw_secret_123456",
        "Bearer \"raw_secret\"",
        "Bearer 'raw_secret'",
        "Basic {{api_token}}dXNlcjpwYXNz",
        "Basic dXNlcjpwYXNz",
        r#"{"token":"raw_secret"}"#,
    ] {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.description = bad_text.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad_text}`, got {err:?}"
        );
    }
}

#[test]
fn validate_script_app_structural_redesign_unified_matrix_and_utf8_safety() {
    use bot_browser::{scan_json_value_for_secret_leaks, scan_text_for_secret_leaks};

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app
    };

    // 1. All positive controls (MUST PASS)
    for ok_desc in [
        r#"{"url_template":"https://example.com/auth?token={{auth_token}}"}"#,
        r#"{"token":""}"#,
        r#"{"description":"Bearer {{api_token}}"}"#,
        "(token={{auth_token}})",
        "{token: {{auth_token}}}",
        "Bearer ({{api_token}})",
        "Basic ({{auth_token}})",
        "Basic \"你好\"",
        "Bearer 你好",
        "Bearer 认证授权流程的详细文档说明",
        "使用 Basic 认证与 Bearer {{api_token}} 流程说明 🚀",
        "https://example.com/auth?token={{auth_token}}&label=你好世界🚀",
        "Basic auth guide",
        "Bearer auth helper",
        "Uses Bearer authentication.",
    ] {
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("positive control `{ok_desc}` failed: {e:?}"));
    }

    // Navigate positive control with UTF-8/emoji query param
    {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: "https://example.com/auth?token={{auth_token}}&label=你好世界🚀".into(),
        }];
        validate_script_app_safe_for_publish(&app)
            .expect("UTF-8/emoji Navigate URL with protected placeholder must pass");
    }

    // 2. All Section 4.2 rejection vectors (MUST FAIL with SecretLeakDetected)
    let bad_vectors = [
        // Quoted Basic (Challenger 1-2, Reviewer Finding 1)
        "Basic \"dXNlcjpwYXNz\"",
        "Basic 'dXNlcjpwYXNz'",
        "Basic: \"dXNlcjpwYXNz\"",
        "Basic \"x\"dXNlcjpwYXNz",
        // JSON-structural abuse in non-JSON strings (Challenger 3-9)
        "https://example.com/auth?token={{auth_token}}\":raw_secret_123456",
        "https://example.com/auth?token=\"\":raw_secret_123456",
        "https://example.com/auth?token='':raw_secret_123456",
        "Authorization: Bearer {{api_token}}\":raw_secret_123456",
        "Bearer {{api_token}}\",raw_secret_123456",
        "Basic {{api_token}}\":dXNlcjpwYXNz",
        // Bearer non-alnum / short-prefix / quote-tail / UTF-8 (Challenger 10-16)
        "Bearer \"x\"raw_secret_123456",
        "Bearer x\"raw_secret_123456",
        "Bearer abc$raw_secret_123456",
        "Bearer \"s$cr3t!key123\"",
        "Bearer 你好secret_123456",
        "Bearer \"你好\"raw_secret_123456",
        "Bearer 🚀{{api_token}}",
        "Bearer \"my secret token 123\"",
        // Basic non-ASCII prefix (Challenger 17)
        "Basic 你好dXNlcjpwYXNz",
        // Unbalanced brackets / braces
        "Bearer ({{api_token}})raw_secret_123456",
        "Basic ({{api_token}})raw_secret_123456",
        "https://example.com/auth?token={{auth_token}}}raw_secret_123456",
        "(token={{auth_token}})raw_secret_123456",
        // Embedded JSON & percent-encoded controls
        r#"{"token":"raw_secret"}"#,
        r#"Debug payload: {"authToken":"raw_secret"}"#,
        "https://example.com/auth?token=%7B%7Bauth_token%7D%7D",
        r#"{\\\"auth_token\\\": \\\"deadbeef12345\\\"}"#,
    ];

    for bad in bad_vectors {
        let mut app = make_app_with_params();
        app.description = bad.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad}`, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad, "direct", &app.parameters),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "scan_text_for_secret_leaks must reject `{bad}`"
        );
    }

    // 3. MAX_JSON_DEPTH fail-closed check (depth > 16)
    let mut deep = json!("leaf");
    for _ in 0..20 {
        deep = json!({ "nested": deep });
    }
    let err = scan_json_value_for_secret_leaks(&deep, "deep", &[]).unwrap_err();
    assert!(
        matches!(err, BotBrowserError::SecretLeakDetected(_)),
        "expected fail-closed SecretLeakDetected on excessive JSON depth, got {err:?}"
    );

    // 4. Phase 1 It4 Adversarial Classes 1-6 + Reviewer p1_5 Minor Finding (plus positive controls)
    for ok_desc in [
        r#"{"Basic":"auth guide"}"#,
        r#"{"Bearer":"{{api_token}}"}"#,
        r#"{"Bearer":"authentication"}"#,
        r#"{"Basic":null,"Bearer":""}"#,
        "Bearer ( {{api_token}} )",
        "Bearer [ {{api_token}} ]",
        "Basic ( {{auth_token}} )",
        "Basic auth: value: {{api_token}}",
        "https://example.com/auth?authToken={{auth_token}}",
        "Bearer auth:",
        "Bearer authentication:",
    ] {
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("p1_4 positive control `{ok_desc}` failed: {e:?}"));
    }

    for ok_url in [
        "https://example.com/auth?authToken={{auth_token}}",
        "https://example.com/auth?api_key_prod={{api_token}}",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: ok_url.to_string(),
        }];
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("p1_4 Navigate positive control `{ok_url}` failed: {e:?}"));
    }

    let p14_bad_vectors = [
        // Class 1: JSON keys "Bearer" / "Basic"
        r#"{"msg":"\uD83D\uDE80","Bearer":"secret_1234567890"}"#,
        r#"{"Bearer":"secret_1234567890"}"#,
        r#"{"Basic":"dXNlcjpwYXNz"}"#,
        r#"{"Basic":"user dXNlcjpwYXNz"}"#,
        r#"{"Bearer":"raw_secret"}"#,
        // Class 2: Whitespace inside (...) / [...] after Bearer / Basic
        "Bearer ( raw_secret_123456)",
        "Bearer (my raw_secret_123456)",
        "Bearer [ raw_secret_123456]",
        "Bearer ( {{api_token}} )raw_secret_123456",
        "Basic ( dXNlcjpwYXNz)",
        "Basic (user dXNlcjpwYXNz)",
        "Basic [ dXNlcjpwYXNz]",
        // Class 3: camelCase & suffixed sensitive keys in URLs, KV strings, truncated JSON
        "https://example.com/auth?authToken=raw_secret_123456",
        "https://example.com/auth?accessToken=raw_secret_123456",
        "https://example.com/auth?refreshToken=raw_secret_123456",
        "https://example.com/auth?userPassword=raw_secret_123456",
        "https://example.com/auth?mySecret=raw_secret_123456",
        "https://example.com/auth?auth_token_1=raw_secret_123456",
        "https://example.com/auth?api_key_prod=raw_secret_123456",
        "https://example.com/auth?password_v2=raw_secret_123456",
        r#"Debug: {"authToken": "raw_secret_123456""#,
        "authToken: raw_secret_123456",
        // Class 4: Authorization header trailing secret after space-separated placeholder
        r#"{"Authorization": "Bearer {{api_token}} raw_secret_123456"}"#,
        r#"{"Authorization": "Basic {{api_token}} dXNlcjpwYXNz"}"#,
        "Authorization: Bearer {{api_token}} raw_secret_123456",
        "Authorization: Basic {{api_token}} dXNlcjpwYXNz",
        // Class 5: Chained Basic KV labels
        "Basic auth: value: dXNlcjpwYXNz",
        "Basic authentication = header: dXNlcjpwYXNz",
        "Basic step: 1 = dXNlcjpwYXNz",
        // Class 6: Markdown backtick & angle-bracket delimiters
        "`sk-ant-api03-1234567890abcdef1234`",
        "<sk-proj-1234567890abcdef1234>",
        "`eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U`",
        // Reviewer p1_5 Minor Finding: Bearer <lhs>: with empty RHS
        "Bearer raw_secret_123456:",
        "Bearer secret_1234567890=",
    ];

    for bad in p14_bad_vectors {
        let mut app = make_app_with_params();
        app.description = bad.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad}`, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad, "p1_4_direct", &app.parameters),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "scan_text_for_secret_leaks must reject `{bad}`"
        );
    }

    // Class 3 inside Navigate.url_template
    {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: "https://example.com/auth?authToken=raw_secret_123456".into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(matches!(err, BotBrowserError::SecretLeakDetected(_)));
    }

    // 5. Phase 1 It5 Challenger p1_8 Classes A-F (38 vectors) + Reviewer p1_7 Minor Notes 1-2 + Positive Controls
    for ok_desc in [
        r#"{"Bearer": "Use {{api_token}}"}"#,
        r#"{"Basic": "Use {{api_token}}"}"#,
        r#"{"basicAuth": "auth guide"}"#,
        r#"{"bearerAuth": "{{api_token}}"}"#,
        r#"{"bearerAuth": "Use {{api_token}}"}"#,
        r#"{"basic_auth": "{{auth_token}}"}"#,
        "Bearer (\n {{api_token}} \n)",
        "Bearer [\r\n {{api_token}} \r\n]",
        "Basic (\n {{auth_token}} \n)",
        "Bearer { {{api_token}} }",
        "Bearer < {{api_token}} >",
        "Basic { {{auth_token}} }",
        "Basic < {{auth_token}} >",
        r#"Authorization: "Bearer {{api_token}}""#,
        r#"X-Authorization: "Bearer {{api_token}}""#,
        "https://example.com/auth?bearerAuth={{api_token}}",
        "https://example.com/auth?basicAuth={{api_token}}",
        "Bearer auth:",
        "Bearer authentication:",
        "Search uses the Basic plan flow, checks no token exists, and resets docs.",
    ] {
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("p1_5 positive control `{ok_desc}` failed: {e:?}"));
        scan_text_for_secret_leaks(ok_desc, "p1_5_ok_direct", &app.parameters)
            .unwrap_or_else(|e| panic!("p1_5 direct positive control `{ok_desc}` failed: {e:?}"));
    }

    let p15_bad_vectors = [
        // Class A: Bearer/Basic camelCase, snake_case & suffixed keys (JSON, KV, URL)
        r#"{"basicAuth": "dXNlcjpwYXNz"}"#,
        r#"{"bearerAuth": "raw_secret_123456"}"#,
        r#"{"bearer_auth": "raw_secret_123456"}"#,
        r#"{"basic_auth": "dXNlcjpwYXNz"}"#,
        r#"{"myBearer": "raw_secret_123456"}"#,
        r#"{"basic_1": "dXNlcjpwYXNz"}"#,
        r#"{"bearer_1": "raw_secret_123456"}"#,
        "https://example.com/auth?bearerAuth=raw_secret_123456",
        "https://example.com/auth?basicAuth=dXNlcjpwYXNz",
        "bearerAuth: raw_secret_123456",
        // Class B: Multiline & unclosed (...) / [...] after Bearer / Basic
        "Bearer (\nraw_secret_123456\n)",
        "Bearer [\nraw_secret_123456\n]",
        "Basic (\ndXNlcjpwYXNz\n)",
        "Basic [\ndXNlcjpwYXNz\n]",
        "Bearer (\r\nraw_secret_123456\r\n)",
        "Basic (\r\ndXNlcjpwYXNz\r\n)",
        "Bearer ( raw_secret_123456",
        "Bearer [ raw_secret_123456",
        "Basic ( dXNlcjpwYXNz",
        "Basic [ dXNlcjpwYXNz",
        "Bearer ( a raw_secret_123456",
        "Bearer (\nraw_secret_123456",
        "Bearer (my raw_secret_123456)",
        "Basic (user dXNlcjpwYXNz)",
        // Class C: { ... } & < ... > enclosures with leading space after Bearer / Basic
        "Bearer { raw_secret_123456 }",
        "Bearer < raw_secret_123456 >",
        "Basic { dXNlcjpwYXNz }",
        "Basic < dXNlcjpwYXNz >",
        // Class D: Quoted Authorization / X-Authorization with trailing space before closing quote + trailing secret
        r#"Authorization: "Bearer {{api_token}} " raw_secret_123456"#,
        "Authorization: 'Basic {{api_token}} ' dXNlcjpwYXNz",
        r#"X-Authorization: "Bearer {{api_token}} " raw_secret_123456"#,
        r#"Debug: {"Authorization": "Bearer {{api_token}} " raw_secret_123456"#,
        // Class E: Multi-word short secrets (6-11 chars) under JSON {"Bearer": "..."} + unprotected placeholder
        r#"{"Bearer": "key sec_12345"}"#,
        r#"{"Bearer": "my s3cr3t!"}"#,
        "{\"Bearer\": \"(\nsec_12345\n)\"}",
        r#"{"Bearer": "Use {{query}}"}"#,
        // Class F: UTF-8 adjacent to extract_kv_key_before_sep
        "authToken你好=raw_secret_123456",
        r#"Debug: {"你好authToken": "raw_secret_123456""#,
        // Reviewer p1_7 Minor Note 2: 6-11 char non-prose <lhs>: with empty RHS
        "Bearer sec_123:",
    ];

    for bad in p15_bad_vectors {
        let mut app = make_app_with_params();
        app.description = bad.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad}`, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad, "p1_5_direct", &app.parameters),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "scan_text_for_secret_leaks must reject `{bad}`"
        );
    }

    // Class A extra_headers & Class E extra_headers via scan_json_value_for_secret_leaks
    for bad_json in [
        json!({ "extra_headers": { "basicAuth": "dXNlcjpwYXNz" } }),
        json!({ "extra_headers": { "bearerAuth": "raw_secret_123456" } }),
        json!({ "extra_headers": { "Bearer": "key sec_12345" } }),
    ] {
        let err = scan_json_value_for_secret_leaks(&bad_json, "extra_headers", &[]).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad_json}`, got {err:?}"
        );
    }

    // 6. Phase 1 It6 Challenger p1_10 Classes 1-5 (37 vectors) + Reviewer p1_10 Findings 1-3 (16 vectors + 2 minor FPs)
    let padded_ok_bearer = format!("Bearer (\n{}\n{{{{api_token}}}}\n)", " ".repeat(1030));
    let mut p16_ok_list: Vec<&str> = vec![
        "bearerAuth: auth guide",
        "basicAuth: auth guide",
        r#"{"bearerAuth": "Use {{api_token}}"}"#,
        r#"{"Bearer": "Use <{{api_token}}>"}"#,
        r#"Use header {"Authorization": "Bearer {{api_token}}", "Accept": "application/json"}"#,
        r#"Use header {"Authorization": "Bearer {{api_token}}"}"#,
        "https://example.com/auth?bearerAuth={{api_token}}&tab=1",
        r#"Use "Bearer {{api_token}}"."#,
        r#"Use "Bearer {{api_token}}" for authenticating."#,
        r#"Use "Bearer ({{api_token}})" for authenticating."#,
        "Bearer (auth) guide",
        "Basic (auth) guide",
        "Basic auth: value: {{api_token}}",
        &padded_ok_bearer,
    ];
    for ok_desc in p16_ok_list.drain(..) {
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("p1_6 positive control `{ok_desc}` failed: {e:?}"));
        scan_text_for_secret_leaks(ok_desc, "p1_6_ok_direct", &app.parameters)
            .unwrap_or_else(|e| panic!("p1_6 direct positive control `{ok_desc}` failed: {e:?}"));
    }

    // Benign digit-containing parameter names (oauth2_url, api_v2_endpoint) must remain valid
    for ok_param in ["oauth2_url", "api_v2_endpoint"] {
        let mut app = make_app_with_params();
        app.parameters.push(ScriptParameter {
            name: ok_param.into(),
            description: "Endpoint URL".into(),
            required: false,
            default_value: Some("https://example.com/v2".into()),
            is_secret: false,
        });
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("benign parameter `{ok_param}` failed: {e:?}"));
    }

    let padded_bearer_sp = format!("Bearer (\n{}\nraw_secret_123456\n)", " ".repeat(1030));
    let padded_basic_sp = format!("Basic (\n{}\ndXNlcjpwYXNz\n)", " ".repeat(1030));
    let padded_bracket_nl = format!("Bearer [\n{}\nraw_secret_123456\n]", "\n".repeat(1030));
    let padded_unclosed_a_sp = format!("Bearer ( a\n{}\nraw_secret_123456", " ".repeat(1030));
    let padded_unclosed_utf8 = format!("Bearer (\n{}\n你好raw_secret_123456", " ".repeat(1030));

    let p16_bad_vectors: Vec<&str> = vec![
        // Class 1 + Reviewer Finding 1: is_bearer_or_basic_key KV & truncated-JSON
        "bearerAuth: key sec_12345",
        "bearerAuth: my raw_secret_123456",
        "bearerAuth: auth: raw_secret_123456",
        "basicAuth: auth: dXNlcjpwYXNz",
        r#"bearerAuth: "{{api_token}}" raw_secret_123456"#,
        r#"basicAuth: "{{api_token}}" dXNlcjpwYXNz"#,
        "basicAuth: user dXNlcjpwYXNz",
        "bearerAuth={{api_token}} raw_secret_123456",
        r#"Debug: {"bearerAuth": "auth" raw_secret_123456"#,
        r#"Debug: {"basicAuth": "auth" dXNlcjpwYXNz"#,
        "bearerAuth: ( raw_secret_123456 )",
        "bearerAuth: (\nraw_secret_123456\n)",
        "basicAuth: ( dXNlcjpwYXNz )",
        "basicAuth: (\ndXNlcjpwYXNz\n)",
        "Debug: {\"bearerAuth\": \"(\nraw_secret_123456\n)\"",
        "Debug: {\"basicAuth\": \"(\ndXNlcjpwYXNz\n)\"",
        // Class 2: Trailing same-line secrets after enclosed/quoted Bearer/Basic
        "Bearer ({{api_token}}) raw_secret_123456",
        r#"Bearer "{{api_token}}" raw_secret_123456"#,
        "Bearer (auth) raw_secret_123456",
        "Bearer {auth} raw_secret_123456",
        "Bearer <auth> raw_secret_123456",
        "Bearer (\n{{api_token}}\n) raw_secret_123456",
        "Basic ({{api_token}}) dXNlcjpwYXNz",
        r#"Basic "{{api_token}}" dXNlcjpwYXNz"#,
        "Basic (auth) dXNlcjpwYXNz",
        "Basic <auth> dXNlcjpwYXNz",
        // Class 3 + Reviewer Finding 2: Padding, unclosed multiline, mismatched inner brackets
        &padded_bearer_sp,
        &padded_basic_sp,
        &padded_bracket_nl,
        &padded_unclosed_a_sp,
        &padded_unclosed_utf8,
        "Bearer ( [ ) raw_secret_123456",
        "Basic ( [ ) dXNlcjpwYXNz",
        "Bearer ( a\nraw_secret_123456",
        "Bearer [\n a\nraw_secret_123456",
        "Bearer { a\nraw_secret_123456",
        "Bearer < a\nraw_secret_123456",
        "Basic ( a\ndXNlcjpwYXNz",
        "Basic [ a\ndXNlcjpwYXNz",
        "Basic (\n a\ndXNlcjpwYXNz",
        "Bearer \"(\nraw_secret_123456\n)\"",
        "Basic \"(\ndXNlcjpwYXNz\n)\"",
        // Class 4: Digit-suffixed sensitive keys without _ / - + ct0 preservation
        r#"{"basic1": "dXNlcjpwYXNz"}"#,
        r#"{"bearer1": "raw_secret_123456"}"#,
        r#"{"authToken1": "raw_secret_123456"}"#,
        r#"{"password1": "raw_secret_123456"}"#,
        r#"{"apiKey2": "raw_secret_123456"}"#,
        r#"{"ct0": "raw_secret_123456"}"#,
        r#"{"myCt0": "raw_secret_123456"}"#,
        "https://example.com/auth?bearer1=raw_secret_123456",
        "https://example.com/auth?authToken1=raw_secret_123456",
        "https://example.com/auth?ct0=raw_secret_123456",
        // Truncated/invalid JSON & duplicate-key JSON must never be skipped
        r#"Debug: {"Authorization": "Bearer {{api_token}} " raw_secret_123456"#,
        r#"Use header {"Authorization": "Bearer raw_secret_123456", "Authorization": "Bearer {{api_token}}"}"#,
        r#"{"Authorization": "Bearer raw_secret_123456", "Authorization": "Bearer {{api_token}}"}"#,
    ];

    for bad in p16_bad_vectors {
        let mut app = make_app_with_params();
        app.description = bad.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad}`, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad, "p1_6_direct", &app.parameters),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "scan_text_for_secret_leaks must reject `{bad}`"
        );
    }

    // Class 4 extra_headers via scan_json_value_for_secret_leaks
    for bad_json in [
        json!({ "extra_headers": { "basic1": "dXNlcjpwYXNz" } }),
        json!({ "extra_headers": { "authToken1": "raw_secret_123456" } }),
        json!({ "extra_headers": { "ct0": "raw_secret_123456" } }),
        json!({ "extra_headers": { "myCt0": "raw_secret_123456" } }),
    ] {
        let err = scan_json_value_for_secret_leaks(&bad_json, "extra_headers_p16", &[]).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "expected SecretLeakDetected for `{bad_json}`, got {err:?}"
        );
    }

    // Class 5: ScriptParameter camelCase, bearerAuth, basicAuth, digit-suffixed, and ct0 with is_secret=false
    for (pname, default_val) in [
        ("authToken", "raw_secret_123456"),
        ("bearerAuth", "raw_secret_123456"),
        ("basicAuth", "dXNlcjpwYXNz"),
        ("authToken1", "raw_secret_123456"),
        ("bearer1", "raw_secret_123456"),
        ("basic1", "dXNlcjpwYXNz"),
        ("ct0", "raw_secret_123456"),
        ("myCt0", "raw_secret_123456"),
    ] {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: pname.to_string(),
            description: "Bypass probe".into(),
            required: false,
            default_value: Some(default_val.to_string()),
            is_secret: false,
        });
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "parameter '{pname}' with is_secret=false/default_value must be rejected, got {err:?}"
        );
    }

    // 7. Phase 1 It7 Challenger p1_12 New Classes 1-6 (48 vectors) + Reviewer p1_12 >64KiB/>128KiB padding + Positive Controls
    let padded_ok_131k_bearer = format!("Bearer (\n{}\n{{{{api_token}}}}\n)", " ".repeat(131_080));
    let padded_ok_65k_quote_bearer = format!("Bearer \"\n{}\n{{{{api_token}}}}\n\"", " ".repeat(65_540));
    for ok_desc in [
        r#"Use "Bearer {{api_token}}" for authenticating."#,
        r#"Use "Bearer ({{api_token}})" for authenticating."#,
        r#"Use "Basic {{api_token}}" for authenticating."#,
        "Search uses the Basic plan flow, checks no token exists, and resets docs.",
        "Search uses the Basic plan flow, checks no token exists, and resets password-reset docs.",
        "https://example.com/auth?bearerAuth={{api_token}}&tab=1",
        "https://example.com/auth?oauth2url=https://example.com/v2",
        padded_ok_131k_bearer.as_str(),
        padded_ok_65k_quote_bearer.as_str(),
    ] {
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("p1_7 positive control `{ok_desc}` failed: {e:?}"));
        scan_text_for_secret_leaks(ok_desc, "p1_7_ok_direct", &app.parameters)
            .unwrap_or_else(|e| panic!("p1_7 direct positive control `{ok_desc}` failed: {e:?}"));
    }

    for ok_param in ["oauth2_url", "api_v2_endpoint", "oauth2url"] {
        let mut app = make_app_with_params();
        app.parameters.push(ScriptParameter {
            name: ok_param.into(),
            description: "Endpoint URL".into(),
            required: false,
            default_value: Some("https://example.com/v2".into()),
            is_secret: false,
        });
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("p1_7 benign parameter `{ok_param}` failed: {e:?}"));
    }

    let pad_65k_a_sp = format!("Bearer ( a {}\nraw_secret_123456 )", " ".repeat(65_540));
    let pad_65k_q_sp = format!("Bearer \"{}\nraw_secret_123456\"", " ".repeat(65_540));
    let pad_131k_b_sp = format!("Bearer (\n{}\nraw_secret_123456\n)", " ".repeat(131_080));
    let pad_65k_q_nl_sp = format!("Bearer \"\n{}\nraw_secret_123456\n\"", " ".repeat(65_540));
    let pad_350_utf8_paren = format!("Bearer (\n{}\nraw_secret_123456\n)", "你".repeat(350));
    let pad_350_utf8_quote = format!("Bearer \"\n{}\nraw_secret_123456\n\"", "你".repeat(350));
    let pad_260_words_paren = format!("Basic ( {}\ndXNlcjpwYXNz )", "word ".repeat(260));
    let pad_260_words_quote = format!("Basic \" {}\ndXNlcjpwYXNz\"", "word ".repeat(260));
    let pad_350_utf8_kv = format!("bearerAuth: ( {}\nraw_secret_123456 )", "你".repeat(350));

    let p17_bad_vectors: Vec<&str> = vec![
        // New Class 1: Unquoted-placeholder & Basic multi-word same-line trailing secrets
        "Bearer {{api_token}} my raw_secret_123456",
        "Bearer {{api_token}} key sec_12345",
        "Bearer {{api_token}} auth: raw_secret_123456",
        "Basic {{api_token}} user dXNlcjpwYXNz",
        "Basic {{api_token}} auth: dXNlcjpwYXNz",
        "Basic user dXNlcjpwYXNz",
        "Basic my dXNlcjpwYXNz",
        "Bearer my raw_secret_123456",
        r#""Bearer {{api_token}}" my raw_secret_123456"#,
        r#""Basic {{api_token}}" user dXNlcjpwYXNz"#,
        r#""Bearer ({{api_token}})" my raw_secret_123456"#,
        // New Class 2: Attached suffix on Basic (and Bearer) enclosure / quote
        "Basic (auth)x: dXNlcjpwYXNz",
        "Basic (auth)x dXNlcjpwYXNz",
        r#"Basic "auth"x: dXNlcjpwYXNz"#,
        r#"Basic "auth"x dXNlcjpwYXNz"#,
        "Basic <auth>x dXNlcjpwYXNz",
        "Bearer (auth)x: raw_secret_123456",
        "Bearer (auth)x: secret",
        // New Class 3 + Reviewer p1_12: UTF-8, alpha-word & >64KiB / >128KiB whitespace padding
        &pad_65k_a_sp,
        &pad_65k_q_sp,
        &pad_131k_b_sp,
        &pad_65k_q_nl_sp,
        &pad_350_utf8_paren,
        &pad_350_utf8_quote,
        &pad_260_words_paren,
        &pad_260_words_quote,
        &pad_350_utf8_kv,
        // New Class 4: Digit -> lowercase transitions in sensitive keys
        r#"{"v2token": "raw_secret_123456"}"#,
        r#"{"v2apiKey": "raw_secret_123456"}"#,
        r#"{"v2api_key": "raw_secret_123456"}"#,
        r#"{"v2bearerAuth": "raw_secret_123456"}"#,
        r#"{"v2basicAuth": "dXNlcjpwYXNz"}"#,
        r#"{"v2password": "raw_secret_123456"}"#,
        r#"{"v2secret": "raw_secret_123456"}"#,
        r#"{"v2ct0": "raw_secret_123456"}"#,
        "https://example.com/auth?v2token=raw_secret_123456",
        "https://example.com/auth?v2api_key=raw_secret_123456",
        // New Class 5: JSON object keys carrying plaintext secrets (standalone & embedded)
        r#"{"Basic dXNlcjpwYXNz": ""}"#,
        r#"{"Bearer raw_secret_123456": ""}"#,
        r#"{"auth_token=raw_secret_123456": ""}"#,
        r#"Use {"Basic dXNlcjpwYXNz": ""} here"#,
        r#"Use {"Bearer raw_secret_123456": "{{api_token}}"} here"#,
        r#"Use {"auth_token=raw_secret_123456": ""} here"#,
        // New Class 6: & / ; outside URLs in bearerAuth / basicAuth KV tails
        "bearerAuth: a&raw_secret_123456",
        "basicAuth: user;dXNlcjpwYXNz",
        "basicAuth: user&dXNlcjpwYXNz",
    ];

    for bad in p17_bad_vectors {
        let mut app = make_app_with_params();
        app.description = bad.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "p1_7 expected SecretLeakDetected for `{bad}`, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad, "p1_7_direct", &app.parameters),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "p1_7 scan_text_for_secret_leaks must reject `{bad}`"
        );
    }

    // New Class 4 Navigate url_template & extra_headers
    for bad_url in [
        "https://example.com/auth?v2token=raw_secret_123456",
        "https://example.com/auth?v2api_key=raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(matches!(err, BotBrowserError::SecretLeakDetected(_)));
    }

    for bad_json in [
        json!({ "extra_headers": { "v2token": "raw_secret_123456" } }),
        json!({ "extra_headers": { "Basic dXNlcjpwYXNz": "" } }),
        json!({ "extra_headers": { "Bearer raw_secret_123456": "{{api_token}}" } }),
    ] {
        let err = scan_json_value_for_secret_leaks(&bad_json, "p1_7_json", &[]).unwrap_err();
        assert!(matches!(err, BotBrowserError::SecretLeakDetected(_)));
    }

    // Exact JSON depth boundary (depth 16 ok, depth 17 err) + overflow-followed-by-tail + unclosed O(N) guard
    {
        let mut d16 = json!("leaf");
        for _ in 0..16 {
            d16 = json!({ "nested": d16 });
        }
        scan_json_value_for_secret_leaks(&d16, "depth_16", &[])
            .expect("JSON depth 16 must pass");
        let d17 = json!({ "nested": d16 });
        let err = scan_json_value_for_secret_leaks(&d17, "depth_17", &[]).unwrap_err();
        assert!(matches!(err, BotBrowserError::SecretLeakDetected(_)));

        let overflow_bearer_tail = format!("Bearer ( {} ) raw_secret_123456", "word ".repeat(260));
        assert!(matches!(
            scan_text_for_secret_leaks(&overflow_bearer_tail, "ov_b_tail", &[]),
            Err(BotBrowserError::SecretLeakDetected(_))
        ));
        let overflow_basic_tail = format!("Basic ( {} ) dXNlcjpwYXNz", "word ".repeat(260));
        assert!(matches!(
            scan_text_for_secret_leaks(&overflow_basic_tail, "ov_ba_tail", &[]),
            Err(BotBrowserError::SecretLeakDetected(_))
        ));
        let many_unclosed_basic = "Basic ( ".repeat(5000);
        scan_text_for_secret_leaks(&many_unclosed_basic, "many_unclosed_basic", &[])
            .expect("many unclosed benign Basic enclosures must finish cleanly in O(N)");
        let many_unclosed_bearer = "Bearer ( ".repeat(5000);
        assert!(matches!(
            scan_text_for_secret_leaks(&many_unclosed_bearer, "many_unclosed_bearer", &[]),
            Err(BotBrowserError::SecretLeakDetected(_))
        ));
    }

    // New Class 4 ScriptParameter variants with is_secret=false
    for (pname, default_val) in [
        ("v2token", "raw_secret_123456"),
        ("v2bearerAuth", "raw_secret_123456"),
        ("v2basicAuth", "dXNlcjpwYXNz"),
        ("v2ct0", "raw_secret_123456"),
    ] {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: pname.to_string(),
            description: "New Class 4 param probe".into(),
            required: false,
            default_value: Some(default_val.to_string()),
            is_secret: false,
        });
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "p1_7 parameter '{pname}' with is_secret=false/default_value must be rejected, got {err:?}"
        );
    }
}

#[test]
fn validate_script_app_remediates_it7_n1_to_n5_token_split_and_linear_scaling() {
    use bot_browser::{scan_json_value_for_secret_leaks, scan_text_for_secret_leaks};
    use std::time::Instant;

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app
    };

    // 1. Positive controls (N1 prose, Minor FP bracketed quotes, N4 valid JSON/URL/multi-KV, >1024-byte placeholder)
    let ov_ph_after = format!("Bearer ( {} {{{{api_token}}}} )", "word ".repeat(300));
    let ov_ph_before = format!("Bearer ( {{{{api_token}}}} {} )", "word ".repeat(300));
    let ov_ph_split = format!("Bearer ( {} {{{{api_token}}}} )", "a ".repeat(1018));
    for ok_desc in [
        r#"Use "Bearer {{api_token}}", then proceed."#,
        r#"Use "Bearer {{api_token}}". Next, open dashboard."#,
        r#"Use "Bearer {{api_token}}" for authenticating."#,
        r#"("Bearer {{api_token}}") is used for auth."#,
        r#"("Basic {{api_token}}") is used for auth."#,
        r#"{"auth_token": ""}"#,
        r#"{"auth_token": "{{auth_token}}"}"#,
        "https://example.com/auth?token={{auth_token}}&next=/home",
        "Set auth_token: {{auth_token}} and api_key: {{api_token}}",
        ov_ph_after.as_str(),
        ov_ph_before.as_str(),
        ov_ph_split.as_str(),
    ] {
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("p1_8 positive control `{ok_desc}` failed: {e:?}"));
        scan_text_for_secret_leaks(ok_desc, "p1_8_ok", &app.parameters)
            .unwrap_or_else(|e| panic!("p1_8 direct positive control `{ok_desc}` failed: {e:?}"));
    }

    // 2. Class N1 (12 bypasses), N2 (7 text + extra_headers), 1024-byte Basic split (6 bypasses), N3 (7 text + Navigate), N4 (9 bypasses)
    let ov_mismatch_b = format!("Bearer (\n{}\n]\nraw_secret_123456", "你".repeat(350));
    let ov_mismatch_b2 = format!("Bearer {{\n{}\n)\nraw_secret_123456", "你".repeat(350));
    let ov_mismatch_b3 = format!("Bearer <\n{}\n)\nraw_secret_123456", "你".repeat(350));
    let ov_mismatch_ba = format!("Basic (\n{}\n]\ndXNlcjpwYXNz", "word ".repeat(260));
    let ov_mismatch_kv = format!("bearerAuth: (\n{}\n]\nraw_secret_123456", "你".repeat(350));
    let ov_quote_paren_ba = format!("Basic (\n{}\n\")\"\ndXNlcjpwYXNz", "word ".repeat(260));
    let ov_quote_paren_b = format!("Bearer (\n{}\n\")\"\nraw_secret_123456", "你".repeat(350));

    let split_basic_1 = format!("Basic ( {}dXNlcjpwYXNz )", "a ".repeat(1017));
    let split_basic_2 = format!("Basic ( {} dXNlcjpwYXNz )", "你".repeat(339));
    let split_basic_3 = format!("Basic ( {}a dXNlcjpwYXNz )", "word ".repeat(254));
    let split_edge_13 = format!("Basic ({}dXNlcjpwYXNz)", "你".repeat(341));
    let split_edge_14 = format!("Basic ({}dXNlcjpwYXNz)", "a ".repeat(1018));
    let split_edge_15 = format!("Basic ( {}dXNlcjoxMjM0NQ )", "a ".repeat(1017));

    let bad_vectors: Vec<&str> = vec![
        // Class N1
        r#"Use "Bearer {{api_token}}" , raw_secret_123456"#,
        r#"Use "Bearer {{api_token}}" . raw_secret_123456"#,
        r#"Use "Bearer {{api_token}}" ; raw_secret_123456"#,
        r#"("Bearer {{api_token}}" ) raw_secret_123456"#,
        r#"["Bearer {{api_token}}" ] raw_secret_123456"#,
        r#"{"Bearer {{api_token}}" } raw_secret_123456"#,
        r#"Use "Basic {{api_token}}" , dXNlcjpwYXNz"#,
        r#"("Basic {{api_token}}" ) dXNlcjpwYXNz"#,
        r#"Use "Bearer ({{api_token}})" , raw_secret_123456"#,
        r#"Use "Basic auth" , dXNlcjpwYXNz"#,
        r#"Use 'Bearer {{api_token}}' , raw_secret_123456"#,
        "Use%20%22Bearer%20%7B%7Bapi_token%7D%7D%22%20%2C%20raw_secret_123456",
        // Class N2
        &ov_mismatch_b,
        &ov_mismatch_b2,
        &ov_mismatch_b3,
        &ov_mismatch_ba,
        &ov_mismatch_kv,
        &ov_quote_paren_ba,
        &ov_quote_paren_b,
        // 1024-byte Basic token-splitting
        &split_basic_1,
        &split_basic_2,
        &split_basic_3,
        &split_edge_13,
        &split_edge_14,
        &split_edge_15,
        // Class N3
        "https://example.com/auth?auth_token[0]=raw_secret_123456",
        "https://example.com/auth?token[]=raw_secret_123456",
        "https://example.com/auth?password[prod]=raw_secret_123456",
        "auth_token[0]=raw_secret_123456",
        "auth_token%5B0%5D=raw_secret_123456",
        "auth_token[0][1]=raw_secret_123456",
        "user[auth_token]=raw_secret_123456",
        // Class N4
        r#"auth_token: "{{auth_token}}" raw_secret_123456"#,
        "auth_token={{auth_token}} raw_secret_123456",
        "auth_token={{auth_token}}#raw_secret_123456",
        "auth_token={{auth_token}};raw_secret_123456",
        "auth_token={{auth_token}}&raw_secret_123456",
        r#"auth_token: "" raw_secret_123456"#,
        r#"password: "" raw_secret_123456"#,
        r#"Authorization: "" raw_secret_123456"#,
        r#"Debug: {"auth_token": "" raw_secret_123456"#,
    ];

    for bad in bad_vectors {
        let mut app = make_app_with_params();
        app.description = bad.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "p1_8 expected SecretLeakDetected for `{bad}`, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad, "p1_8_direct", &app.parameters),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "p1_8 scan_text_for_secret_leaks must reject `{bad}`"
        );
    }

    // Class N2 extra_headers & Class N3 Navigate.url_template
    let err_extra = scan_json_value_for_secret_leaks(
        &json!({ "extra_headers": { "note": ov_mismatch_b } }),
        "p1_8_extra_ov",
        &[],
    )
    .unwrap_err();
    assert!(matches!(err_extra, BotBrowserError::SecretLeakDetected(_)));

    let mut nav_app = make_app_with_params();
    nav_app.steps = vec![ScriptStep::Navigate {
        url_template: "https://example.com/auth?auth_token[0]=raw_secret_123456".into(),
    }];
    let err_nav = validate_script_app_safe_for_publish(&nav_app).unwrap_err();
    assert!(matches!(err_nav, BotBrowserError::SecretLeakDetected(_)));

    // 3. True O(N) scaling across all previously quadratic paths (debug build <= 500ms each)
    let params = make_app_with_params().parameters;
    let perf_cases = [
        ("bearer_ph_5k", "Bearer {{api_token}} ".repeat(5_000)),
        ("quoted_bearer_ph_2k", r#""Bearer {{api_token}}" "#.repeat(2_000)),
        ("quoted_bearer_auth_5k", "\"Bearer auth ".repeat(5_000)),
        ("singleline_bearer_auth_10k", "Bearer auth ".repeat(10_000)),
        ("basic_ph_5k", "Basic {{api_token}} ".repeat(5_000)),
        ("unclosed_basic_10k", "Basic ( ".repeat(10_000)),
        (
            "nested_basic_5k",
            format!("{}{}", "Basic ( ".repeat(5_000), ")".repeat(5_000)),
        ),
        (
            "nested_bearer_5k",
            format!("Bearer {}{}{}", "( ".repeat(5_000), "a", ")".repeat(5_000)),
        ),
        ("kv_ph_5k", "auth_token: {{auth_token}} ".repeat(5_000)),
        ("d1_kv_paren_5k", format!("( {}", "auth_token: {{auth_token}} ".repeat(5_000))),
        ("d2_bearer_auth_kv_5k", "bearerAuth: {{api_token}} ".repeat(5_000)),
        ("d2_basic_auth_kv_5k", "basicAuth: {{api_token}} ".repeat(5_000)),
        ("d3_bearer_colon_kv_4k", "Bearer auth: {{api_token}} ".repeat(4_000)),
        ("d3_basic_colon_kv_4k", "Basic auth: {{api_token}} ".repeat(4_000)),
        (
            "d4_kv_url_5k",
            format!("https://example.com/auth?{}x=1", "auth_token={{auth_token}}&".repeat(5_000)),
        ),
        (
            "d4_bearer_url_5k",
            format!("https://example.com/auth?{}x=1", "bearerAuth={{api_token}}&".repeat(5_000)),
        ),
        ("d5_unclosed_json_20k", format!("{}1", "{\"a\": ".repeat(20_000))),
        ("d5_unclosed_json_single_close_20k", format!("{}}}", "{\"a\": 1, \"b\": ".repeat(20_000))),
        ("d5_unclosed_json_quote_20k", format!("{}\"}} ", "{\"a\": ".repeat(20_000))),
        ("d6_nested_angle_basic_5k", format!("{}{}", "Basic < ".repeat(5_000), ">".repeat(5_000))),
        ("d6_nested_angle_bearer_5k", format!("Bearer {}{}{}", "< ".repeat(5_000), "a", ">".repeat(5_000))),
        ("d7_unclosed_quote_eof_basic_5k", format!("{} \" )", "Basic ( ".repeat(5_000))),
        ("d7_unclosed_quote_eof_bearer_5k", format!("Bearer {} \" )", "( ".repeat(5_000))),
        ("d_bearer_nested_kv_5k", format!("Bearer ( {} )", "a: b ".repeat(5_000))),
    ];
    for (label, input) in perf_cases {
        let t0 = Instant::now();
        scan_text_for_secret_leaks(&input, label, &params)
            .unwrap_or_else(|e| panic!("O(N) perf case `{label}` unexpectedly failed: {e:?}"));
        let elapsed_ms = t0.elapsed().as_millis();
        assert!(
            elapsed_ms < 500,
            "O(N) perf case `{label}` took {elapsed_ms} ms (>= 500 ms)"
        );
    }

    let t_kv_ov = Instant::now();
    let _ = scan_text_for_secret_leaks(
        &format!("Bearer {}", "a: ( ".repeat(5_000)),
        "d_bearer_nested_kv_ov_5k",
        &params,
    );
    assert!(
        t_kv_ov.elapsed().as_millis() < 500,
        "d_bearer_nested_kv_ov_5k took {} ms",
        t_kv_ov.elapsed().as_millis()
    );

    // D5 deeply nested valid JSON (>128 levels) must fail fast with SecretLeakDetected in O(N)
    let deep_20k = format!("{}1{}", "{\"a\": ".repeat(20_000), "}".repeat(20_000));
    let t_deep = Instant::now();
    assert!(matches!(
        scan_text_for_secret_leaks(&deep_20k, "d5_deep_20k", &params),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));
    assert!(
        t_deep.elapsed().as_millis() < 500,
        "d5_deep_20k took {} ms",
        t_deep.elapsed().as_millis()
    );
}

#[test]
fn validate_script_app_remediates_it8_url_b1_to_b4_and_minor_fps() {
    use bot_browser::scan_text_for_secret_leaks;

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app
    };

    for ok_desc in [
        "(Bearer {{api_token}}) is used for auth.",
        "(Basic {{api_token}}) is used for auth.",
        "https://example.com/auth?x={{ auth_token }}&auth_token={{auth_token}}&next=/home",
        "https://example.com/auth?auth_token={{auth_token}}#top",
        "https://example.com/auth?auth_token={{auth_token}}#section",
        "https://example.com/auth?auth_token={{auth_token}}#access_token={{api_token}}",
        "?auth_token={{auth_token}}&next=/home",
        "https://password-reset.example.com:8080/login",
        "Use auth_token. Note: {{auth_token}}",
        r#"{"Bearer": "Use <{{api_token}}>"}"#,
        "Basic <{{api_token}}>",
        "Bearer < {{api_token}} > is used",
    ] {
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("it9 positive control `{ok_desc}` failed: {e:?}"));
        scan_text_for_secret_leaks(ok_desc, "it9_ok", &app.parameters)
            .unwrap_or_else(|e| panic!("it9 direct positive control `{ok_desc}` failed: {e:?}"));
    }

    let angle_bearer = format!("Bearer < {}a > b\nraw_secret_123456", "word ".repeat(260));
    let angle_basic = format!("Basic < {}a > b\ndXNlcjpwYXNz", "word ".repeat(260));
    let ov_broken_q = format!("Bearer ( {} \"{{{{api_token}}}} )", "word ".repeat(300));

    for bad_desc in [
        // URL query/fragment non-KV secret bypasses
        "https://example.com/auth?auth_token={{auth_token}}#raw_secret_123456",
        "https://example.com/auth?auth_token={{auth_token}}&raw_secret_123456",
        "https://example.com/auth?auth_token={{auth_token}};raw_secret_123456",
        "https://example.com/auth?auth_token[0]={{auth_token}}#raw_secret_123456",
        "https://example.com/auth?bearerAuth={{api_token}}&raw_secret_123456",
        "https://example.com/auth?bearerAuth={{api_token}};raw_secret_123456",
        // Mechanism B3: non-URL Use?
        "Use?auth_token={{auth_token}}#raw_secret_123456",
        "Use?auth_token={{auth_token}}&raw_secret_123456",
        "Use?auth_token={{auth_token}};raw_secret_123456",
        // Mechanism B1: extract_kv_key_before_sep subscript & dot variants
        "https://example.com/auth?auth_token[a:b]=raw_secret_123456",
        "https://example.com/auth?auth_token[a=b]=raw_secret_123456",
        "auth_token[a:b]=raw_secret_123456",
        "https://example.com/auth?auth_token%5Ba%3Ab%5D=raw_secret_123456",
        "auth_token [0]=raw_secret_123456",
        "auth_token.0=raw_secret_123456",
        "auth_token.prod=raw_secret_123456",
        // Mechanism B2: punctuated / numeric KV labels in trailing scan
        r#"auth_token: "{{auth_token}}" my.secret.123456: {{api_token}}"#,
        r#"auth_token: "{{auth_token}}" my_secret_123456: {{api_token}}"#,
        // Mechanism B4: angle comparison op ` > ` inside < ... >
        angle_bearer.as_str(),
        angle_basic.as_str(),
        // >1024-byte unclosed quote before placeholder
        ov_broken_q.as_str(),
    ] {
        let mut app = make_app_with_params();
        app.description = bad_desc.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it9 expected SecretLeakDetected for `{bad_desc}`, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad_desc, "it9_direct", &app.parameters),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "it9 scan_text_for_secret_leaks must reject `{bad_desc}`"
        );
    }

    for bad_url in [
        "https://example.com/auth?auth_token={{auth_token}}#raw_secret_123456",
        "https://example.com/auth?auth_token[a:b]=raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(matches!(err, BotBrowserError::SecretLeakDetected(_)));
    }
}

#[test]
fn validate_script_app_remediates_it10_edge_c_bypasses_json_depth_dos_and_fps() {
    use bot_browser::scan_text_for_secret_leaks;
    use std::time::Instant;

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "user".into(),
            description: "Username".into(),
            required: false,
            default_value: Some("alice".into()),
            is_secret: false,
        });
        app
    };
    let params = make_app_with_params().parameters;

    // 1 & 5. O(N) scaling for EDGE_C_BEARER_URL_TRAILING & depth-125/depth-18 450KB invalid JSON
    let edge_c_bearer = format!(
        "https://example.com/auth?{}x=1 foo",
        "bearerAuth={{api_token}}&".repeat(5_000)
    );
    let edge_c_basic = format!(
        "https://example.com/auth?{}x=1 foo",
        "basicAuth={{api_token}}&".repeat(5_000)
    );
    let dos_depth125_450kb = format!(
        "{}{{\"b\": [{}bad]}}{}",
        "{\"a\": ".repeat(125),
        "1,".repeat(220_000),
        "}".repeat(125)
    );
    let dos_depth125_450kb_enc = format!(
        "{}{{\"b\": [{}bad]}}{}",
        "{\"a\": ".repeat(125),
        "1%2C".repeat(110_000),
        "}".repeat(125)
    );
    let dos_depth18_450kb = format!(
        "{}{{\"b\": [{}bad]}}{}",
        "{\"a\": ".repeat(18),
        "1,".repeat(220_000),
        "}".repeat(18)
    );
    for (label, input) in [
        ("edge_c_bearer_url_trailing", edge_c_bearer.as_str()),
        ("edge_c_basic_url_trailing", edge_c_basic.as_str()),
        ("h3_dos_json_depth125_450kb", dos_depth125_450kb.as_str()),
        ("h3_dos_json_depth125_450kb_enc", dos_depth125_450kb_enc.as_str()),
        ("h3_dos_json_depth18_450kb", dos_depth18_450kb.as_str()),
    ] {
        let t0 = Instant::now();
        scan_text_for_secret_leaks(input, label, &params)
            .unwrap_or_else(|e| panic!("it10 perf case `{label}` unexpectedly failed: {e:?}"));
        let elapsed = t0.elapsed().as_millis();
        assert!(
            elapsed < 500,
            "it10 perf case `{label}` took {elapsed} ms (>= 500 ms)"
        );
    }

    // JSON nesting depth boundary: 16 ok, 17 err
    let mk_nested = |levels: usize| {
        format!(
            "{}\"ok\"{}",
            "{\"a\": ".repeat(levels),
            "}".repeat(levels)
        )
    };
    scan_text_for_secret_leaks(&mk_nested(15), "depth16_ok", &params).unwrap();
    assert!(matches!(
        scan_text_for_secret_leaks(&mk_nested(16), "depth17_err", &params),
        Err(BotBrowserError::SecretLeakDetected(_))
    ));

    // 2 & 6. Positive controls (URL trailing prose, #top/#sec-{{user}}, Bearer/Basic ("{{api_token}}"))
    for ok_desc in [
        "https://example.com/auth?bearerAuth={{api_token}} is used",
        "(https://example.com/auth?bearerAuth={{api_token}}) is used",
        "https://example.com/auth?bearerAuth={{api_token}}\n&x=1",
        "https://example.com/auth?bearerAuth={{api_token}}#top",
        "https://example.com/auth?bearerAuth={{api_token}}#section",
        "https://example.com/auth?bearerAuth={{api_token}}#sec-{{user}}",
        "https://example.com/auth?auth_token={{auth_token}}#sec-{{user}}",
        r#"Bearer ("{{api_token}}")"#,
        r#"Basic ("{{api_token}}")"#,
        r#"Bearer ('{{api_token}}')"#,
        r#"{"Authorization": "Bearer (\"{{api_token}}\")"}"#,
        "Use auth_token [note: ok]",
        "Use auth_token. Note: {{auth_token}}",
    ] {
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("it10 positive control `{ok_desc}` failed: {e:?}"));
        scan_text_for_secret_leaks(ok_desc, "it10_ok", &app.parameters)
            .unwrap_or_else(|e| panic!("it10 direct positive control `{ok_desc}` failed: {e:?}"));
    }

    // 2, 3, 4, 6. All 12 extract_kv_key_before_sep bypasses, URL+space+secret, URL #/& {{x=1}}, and broken-quote traps
    for bad_desc in [
        // URL + space + trailing secret
        "https://example.com/auth?bearerAuth={{api_token}} raw_secret_123456",
        "https://example.com/auth?basicAuth={{api_token}} dXNlcjpwYXNz",
        "(https://example.com/auth?bearerAuth={{api_token}}) raw_secret_123456",
        "https://example.com/auth?bearerAuth={{api_token}}&basicAuth={{api_token}}&x=1 dXNlcjox",
        // 12 extract_kv_key_before_sep bypasses + .0
        "auth_token[0].prod=raw_secret_123456",
        "auth_token[0].value=raw_secret_123456",
        "auth_token[0].0=raw_secret_123456",
        "https://example.com/auth?auth_token[0].prod=raw_secret_123456",
        "auth_token [0].prod=raw_secret_123456",
        "https://example.com/auth?auth_token%5B0%5D.prod=raw_secret_123456",
        "auth_token[a[b]=raw_secret_123456",
        "auth_token[0] [1]=raw_secret_123456",
        r#""auth_token"[0]=raw_secret_123456"#,
        r#""auth_token" [0]=raw_secret_123456"#,
        "auth_token[0][1].prod=raw_secret_123456",
        "bearerAuth[0].prod=raw_secret_123456",
        "auth_token.0[1].prod=raw_secret_123456",
        // URL placeholder-`=` bypasses
        "https://example.com/auth?auth_token={{auth_token}}#{{x=1}}raw_secret_123456",
        "https://example.com/auth?auth_token={{auth_token}}&{{x=1}}raw_secret_123456",
        "https://example.com/auth?bearerAuth={{api_token}}#{{x=1}}raw_secret_123456",
        // Fragment secret & broken-quote / trailing-secret traps
        "https://example.com/auth?bearerAuth={{api_token}}#raw_secret_123456",
        r#"Bearer ("{{api_token}}""#,
        r#"Bearer ("{{api_token}}" raw_secret_123456)"#,
        r#"Bearer ("{{api_token}}") raw_secret_123456"#,
        r#"Basic ("{{api_token}}" dXNlcjpwYXNz)"#,
    ] {
        let mut app = make_app_with_params();
        app.description = bad_desc.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it10 expected SecretLeakDetected for `{bad_desc}`, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad_desc, "it10_direct", &app.parameters),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "it10 scan_text_for_secret_leaks must reject `{bad_desc}`"
        );
    }

    for bad_url in [
        "https://example.com/auth?auth_token[0].prod=raw_secret_123456",
        "https://example.com/auth?auth_token={{auth_token}}#{{x=1}}raw_secret_123456",
        "https://example.com/auth?bearerAuth={{api_token}}#raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(matches!(err, BotBrowserError::SecretLeakDetected(_)));
    }
}

#[test]
fn validate_script_app_remediates_it11_bomb_budget_exhaustion_and_kv_bypasses() {
    use bot_browser::scan_text_for_secret_leaks;
    use std::time::Instant;

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app
    };
    let params = make_app_with_params().parameters;

    // 1. Positive controls & worst-case UTF-8 / trailing-comma / late-error 18-layer O(N) performance
    let utf8_d18_400kb = format!(
        "{}{{\"b\": \"{}\", \"x\": bad}}{}",
        "{\"a\": ".repeat(17),
        "🚀".repeat(50_000),
        "}".repeat(17)
    );
    let d18_trailing_comma = format!(
        "{}{{\"b\": [{}],}}{}",
        "{\"a\": ".repeat(17),
        "1,".repeat(200_000),
        "}".repeat(17)
    );
    let late_error_single_stack = format!(
        "{}{{\"p\": \"{}\", \"x\": bad}}{}",
        "{\"a\": ".repeat(17),
        "1".repeat(400_000),
        "}".repeat(17)
    );
    let mut late_error_disjoint_layers = format!("{{\"p\":\"{}\",\"x\":bad}}", "1".repeat(24_000));
    for _ in 0..17 {
        late_error_disjoint_layers = format!(
            "{{\"p\":\"{}\",\"y\":{late_error_disjoint_layers},\"x\":bad}}",
            "1".repeat(24_000)
        );
    }

    for (label, ok_desc) in [
        ("utf8_d18_400kb", utf8_d18_400kb.as_str()),
        ("d18_trailing_comma", d18_trailing_comma.as_str()),
        ("late_error_single_stack", late_error_single_stack.as_str()),
        ("late_error_disjoint_layers", late_error_disjoint_layers.as_str()),
        ("prose_dot_note_ph", "Use auth_token. Note: {{auth_token}}"),
        ("prose_dot_note_ok", "Use auth_token. Note: ok"),
        ("prose_bracket_note_ok", "Use auth_token [note: ok]"),
        ("url_port_ok", "https://password-reset.example.com:8080/login"),
    ] {
        let t0 = Instant::now();
        let mut app = make_app_with_params();
        app.description = ok_desc.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("it11 positive/perf `{label}` failed: {e:?}"));
        scan_text_for_secret_leaks(ok_desc, label, &params)
            .unwrap_or_else(|e| panic!("it11 direct positive/perf `{label}` failed: {e:?}"));
        let elapsed = t0.elapsed().as_millis();
        assert!(
            elapsed < 500,
            "it11 positive/perf `{label}` took {elapsed} ms (>= 500 ms)"
        );
    }

    // 2. Defect 1: 16KB (18-layer) & 70KB (5-layer) failed_parse_bytes bombs + late-error bomb + trailing embedded JSON secrets / depth-17
    let mut bomb_16k_18l = format!("{{\"p\":\"{}\"}}", "1".repeat(16_000));
    for _ in 0..17 {
        bomb_16k_18l = format!("{{\"x\":bad,\"y\":{bomb_16k_18l}}}");
    }
    let mut bomb_70k_5l = format!("{{\"p\":\"{}\"}}", "1".repeat(70_000));
    for _ in 0..5 {
        bomb_70k_5l = format!("{{\"x\":bad,\"y\":{bomb_70k_5l}}}");
    }
    let mk_nested = |levels: usize| {
        format!(
            "{}\"ok\"{}",
            "{\"a\": ".repeat(levels),
            "}".repeat(levels)
        )
    };
    let d17_tail = mk_nested(16);
    let nl_tail = r#"{"auth\ntoken": "raw_secret_123456"}"#;
    let q_tail = r#"{"api\"key": "raw_secret_123456"}"#;

    let bomb_cases = [
        ("b16_nl", format!("{bomb_16k_18l} {nl_tail}")),
        ("b16_q", format!("{bomb_16k_18l} {q_tail}")),
        ("b16_d17", format!("{bomb_16k_18l} {d17_tail}")),
        ("b70_nl", format!("{bomb_70k_5l} {nl_tail}")),
        ("b70_q", format!("{bomb_70k_5l} {q_tail}")),
        ("b70_d17", format!("{bomb_70k_5l} {d17_tail}")),
        ("late_disjoint_nl", format!("{late_error_disjoint_layers} {nl_tail}")),
        ("late_single_nl", format!("{late_error_single_stack} {nl_tail}")),
    ];
    for (label, payload) in &bomb_cases {
        let t0 = Instant::now();
        let mut app = make_app_with_params();
        app.description = payload.clone();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it11 bomb `{label}` must reject with SecretLeakDetected, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(payload, label, &params),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "it11 direct bomb `{label}` must reject"
        );
        let elapsed = t0.elapsed().as_millis();
        assert!(
            elapsed < 500,
            "it11 bomb `{label}` took {elapsed} ms (>= 500 ms)"
        );
    }

    // 3. Defect 2: All 11+ extract_kv_key_before_sep bypass vectors
    for bad_kv in [
        "auth_token[[0]=raw_secret_123456",
        "auth_token[[a]=raw_secret_123456",
        "auth_token[0][[1]=raw_secret_123456",
        "https://example.com/auth?auth_token[[0]=raw_secret_123456",
        "bearerAuth[[0]=raw_secret_123456",
        "https://example.com/auth?auth_token%5B%5B0%5D=raw_secret_123456",
        "auth_token[0] [a[b]=raw_secret_123456",
        r#"auth_token["]"]=raw_secret_123456"#,
        "auth_token[']']=raw_secret_123456",
        r#"auth_token["["]=raw_secret_123456"#,
        r#"auth_token["a]b"]=raw_secret_123456"#,
        "auth_token[0] .prod=raw_secret_123456",
        r#"auth_token[0] "prod"=raw_secret_123456"#,
        "auth_token..prod=raw_secret_123456",
        "auth_token .prod=raw_secret_123456",
        "auth_token[0]a=raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.description = bad_kv.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it11 KV vector `{bad_kv}` must reject in app.description, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(bad_kv, "it11_kv", &params),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "it11 KV vector `{bad_kv}` must reject in scan_text_for_secret_leaks"
        );
    }

    for bad_url in [
        "https://example.com/auth?auth_token[[0]=raw_secret_123456",
        "https://example.com/auth?auth_token%5B%5B0%5D=raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(matches!(err, BotBrowserError::SecretLeakDetected(_)));
    }
}

#[test]
fn validate_script_app_remediates_it12_fallback_arr_depth_and_13_kv_bypasses() {
    use bot_browser::scan_text_for_secret_leaks;
    use std::time::Instant;

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app
    };
    let params = make_app_with_params().parameters;

    // 1. Positive prose controls + depth-16 boundary + O(N) scaling (including >256-byte subscripts & unmatched/nested brackets)
    let depth16_ok = format!("prefix {{\"a\": {}1{}}}", "[".repeat(14), "]".repeat(14));
    let depth17_err = format!("prefix {{\"a\": {}1{}}}", "[".repeat(15), "]".repeat(15));
    let h3_dos_d200_450kb = format!(
        "{}{{\"b\": [{}bad]}}{}",
        "{\"a\": ".repeat(200),
        "1,".repeat(220_000),
        "}".repeat(200)
    );
    let long_sub_300_x1000 = format!(
        "{}x=1",
        format!("a[{}]=1 ", "a".repeat(300)).repeat(1_000)
    );
    let unmatched_rbrack_50k_5k = format!("{}{}", "x".repeat(50_000), "]=1".repeat(5_000));
    let nested_rbrack_kv_5k = format!("{}[a=1]{}", "[".repeat(5_000), "]=1".repeat(5_000));
    let d6_nested_angle_basic_10k = format!("{}{}", "Basic < ".repeat(10_000), ">".repeat(10_000));

    for (label, ok_text) in [
        ("prose_dot_note_ph", "Use auth_token. Note: {{auth_token}}"),
        ("prose_bracket_note_ok", "Use auth_token [note: ok]"),
        ("prose_dot_quoted_note_ok", r#"Use auth_token. "Note": ok"#),
        (
            "prose_dot_quoted_note_ph",
            r#"Use auth_token. "Note": {{auth_token}}"#,
        ),
        ("prose_two_quotes_note_ok", r#"Use "auth_token" "Note": ok"#),
        ("depth16_ok", depth16_ok.as_str()),
        ("h3_dos_d200_450kb", h3_dos_d200_450kb.as_str()),
        ("long_sub_300_x1000", long_sub_300_x1000.as_str()),
        ("unmatched_rbrack_50k_5k", unmatched_rbrack_50k_5k.as_str()),
        ("nested_rbrack_kv_5k", nested_rbrack_kv_5k.as_str()),
        ("d6_nested_angle_basic_10k", d6_nested_angle_basic_10k.as_str()),
    ] {
        let t0 = Instant::now();
        scan_text_for_secret_leaks(ok_text, label, &params)
            .unwrap_or_else(|e| panic!("it12 positive/perf `{label}` direct failed: {e:?}"));
        if label != "depth16_ok" {
            let mut app = make_app_with_params();
            app.description = ok_text.to_string();
            validate_script_app_safe_for_publish(&app)
                .unwrap_or_else(|e| panic!("it12 positive/perf `{label}` app failed: {e:?}"));
        }
        let elapsed = t0.elapsed().as_millis();
        assert!(
            elapsed < 500,
            "it12 positive/perf `{label}` took {elapsed} ms (>= 500 ms)"
        );
    }

    // 2. Defect 1: fallback_attempts >= 8 unclosed-quote exhaustion bypasses
    let mk_nested = |levels: usize| {
        format!(
            "{}\"ok\"{}",
            "{\"a\": ".repeat(levels),
            "}".repeat(levels)
        )
    };
    let nl_tail = r#"{"auth\ntoken": "raw_secret_123456"}"#;
    let pass_tail = r#"{"pass\nword": "raw_secret_123456"}"#;
    let d17_tail = mk_nested(16);
    let prefix_8_ok = format!("{{\"p\": \"{}", r#"{"a":"1"}"#.repeat(8));
    let prefix_20_bad = format!("{{\"p\": \"{}", r#"{"x":bad,"y":1}"#.repeat(20));
    let late_error_single_stack = format!(
        "{}{{\"p\": \"{}\", \"x\": bad}}{}",
        "{\"a\": ".repeat(17),
        "1".repeat(400_000),
        "}".repeat(17)
    );
    let prefix_unclosed_late_400k = format!("{{\"p\": \"{late_error_single_stack}");

    // 3. Defect 2: max_depth > MAX_JSON_DEPTH + 2 array-depth evasion
    let arr19 = format!("{}1{}", "[".repeat(19), "]".repeat(19));
    let arr19_nl = format!(
        "prefix {{\"auth\\ntoken\": \"raw_secret_123456\", \"pad\": {arr19}}}"
    );
    let arr19_depth20 = format!("prefix {{\"a\": {arr19}}}");

    // 4. Defect 3: All 13 Challenger KV bypass vectors + 3 trailing-sep corner cases
    let long_sub_260 = format!("auth_token[{}]=raw_secret_123456", "a".repeat(260));
    let long_qsub_260 = format!("auth_token[\"{}\"]=raw_secret_123456", "a".repeat(260));
    let long_dot_q_260 = format!("auth_token.\"{}\"=raw_secret_123456", "a".repeat(260));

    let defect_cases = [
        ("d1_unclosed_8_nl", format!("{prefix_8_ok}{nl_tail}")),
        ("d1_unclosed_8_pass", format!("{prefix_8_ok}{pass_tail}")),
        ("d1_unclosed_8_d17", format!("{prefix_8_ok}{d17_tail}")),
        ("d1_unclosed_20_bad_nl", format!("{prefix_20_bad}{nl_tail}")),
        (
            "d1_unclosed_late_400k_nl",
            format!("{prefix_unclosed_late_400k} {nl_tail}"),
        ),
        ("d2_arr19_nl", arr19_nl),
        ("d2_arr19_depth20", arr19_depth20),
        ("d2_arr15_depth17", depth17_err),
        (
            "d3_01_dot_sp_quoted",
            r#"auth_token. "prod"=raw_secret_123456"#.to_string(),
        ),
        (
            "d3_02_sub_dot_sp_quoted",
            r#"auth_token[0]. "prod"=raw_secret_123456"#.to_string(),
        ),
        (
            "d3_03_sub_dot_sp_quoted_sub",
            r#"auth_token[0]. "prod"[1]=raw_secret_123456"#.to_string(),
        ),
        (
            "d3_04_sp_dot_sp_quoted",
            r#"auth_token . "prod"=raw_secret_123456"#.to_string(),
        ),
        (
            "d3_05_dot_q_sp_q",
            r#"auth_token."a" "b"=raw_secret_123456"#.to_string(),
        ),
        (
            "d3_06_ident_quoted",
            r#"auth_token"prod"=raw_secret_123456"#.to_string(),
        ),
        (
            "d3_07_ident_under_quoted",
            r#"auth_token_"prod"=raw_secret_123456"#.to_string(),
        ),
        (
            "d3_08_sub_ident_quoted",
            r#"auth_token[0]a"prod"=raw_secret_123456"#.to_string(),
        ),
        (
            "d3_09_extra_rbrack",
            "auth_token[0]]=raw_secret_123456".to_string(),
        ),
        (
            "d3_09b_url_extra_rbrack",
            "https://example.com/auth?auth_token[0]]=raw_secret_123456".to_string(),
        ),
        (
            "d3_10_sp_extra_rbrack",
            "auth_token[0] ]=raw_secret_123456".to_string(),
        ),
        (
            "d3_11_sub_sp_ident",
            "auth_token[0] a=raw_secret_123456".to_string(),
        ),
        ("d3_12_long_sub_260", long_sub_260),
        ("d3_13_long_qsub_260", long_qsub_260),
        ("d3_13b_long_dot_q_260", long_dot_q_260),
        (
            "d3_14_trailing_dot",
            "auth_token.=raw_secret_123456".to_string(),
        ),
        (
            "d3_15_trailing_lbrack",
            "auth_token[=raw_secret_123456".to_string(),
        ),
        (
            "d3_16_sub_trailing_dot",
            "auth_token[0].=raw_secret_123456".to_string(),
        ),
    ];

    for (label, payload) in &defect_cases {
        let t0 = Instant::now();
        let mut app = make_app_with_params();
        app.description = payload.clone();
        let err = match validate_script_app_safe_for_publish(&app) {
            Err(e) => e,
            Ok(()) => panic!("it12 vector `{label}` unexpectedly returned Ok(()) in app.description"),
        };
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it12 vector `{label}` must reject in app.description, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(payload, label, &params),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "it12 vector `{label}` must reject in scan_text_for_secret_leaks"
        );
        let elapsed = t0.elapsed().as_millis();
        assert!(
            elapsed < 500,
            "it12 vector `{label}` took {elapsed} ms (>= 500 ms)"
        );
    }

    for bad_url in [
        "https://example.com/auth?auth_token[0]]=raw_secret_123456",
        "https://example.com/auth?auth_token[0]a=raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(matches!(err, BotBrowserError::SecretLeakDetected(_)));
    }
}

#[test]
fn validate_script_app_remediates_it13_dos_depth128_dup_nl_and_kv_findings() {
    use bot_browser::scan_text_for_secret_leaks;
    use std::time::Instant;

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app
    };
    let params = make_app_with_params().parameters;

    // 1. Prose positive controls (6/6) + O(N) scaling across Defect A paths + h3_dos_d200_450kb (< 500 ms direct)
    let dos_path_a = format!(
        "{}\" {}{}",
        "[".repeat(5_000),
        "a".repeat(20_000),
        "]=1,".repeat(5_000)
    );
    let dos_path_a_prefix = format!(
        "a{}\" {}{}",
        "[".repeat(5_000),
        "a".repeat(20_000),
        "]=1,".repeat(5_000)
    );
    let dos_path_b_sp = format!("a{}{}", "[ ".repeat(5_000), "]=1,".repeat(5_000));
    let dos_path_b_nosp = format!("a{}{}", "[".repeat(5_000), "]=1,".repeat(5_000));
    let dos_path_b_sub_chain = format!(
        "a{}{}{}",
        "[ ]".repeat(5_000),
        "[".repeat(5_000),
        "]=1,".repeat(5_000)
    );
    let h3_dos_d200_450kb = format!(
        "{}{{\"b\": [{}bad]}}{}",
        "{\"a\": ".repeat(200),
        "1,".repeat(220_000),
        "}".repeat(200)
    );

    for (label, ok_text) in [
        ("prose_dot_note_ph", "Use auth_token. Note: {{auth_token}}"),
        ("prose_bracket_note_ok", "Use auth_token [note: ok]"),
        ("prose_dot_quoted_note_ok", r#"Use auth_token. "Note": ok"#),
        (
            "prose_dot_quoted_note_ph",
            r#"Use auth_token. "Note": {{auth_token}}"#,
        ),
        ("prose_two_quotes_note_ok", r#"Use "auth_token" "Note": ok"#),
        (
            "prose_bracket_note_ph",
            "Use auth_token [note: {{auth_token}}]",
        ),
        ("dos_path_a_unclosed_q_5k", dos_path_a.as_str()),
        ("dos_path_a_prefix_5k", dos_path_a_prefix.as_str()),
        ("dos_path_b_lbrack_sp_5k", dos_path_b_sp.as_str()),
        ("dos_path_b_lbrack_nosp_5k", dos_path_b_nosp.as_str()),
        ("dos_path_b_sub_chain_5k", dos_path_b_sub_chain.as_str()),
        ("h3_dos_d200_450kb", h3_dos_d200_450kb.as_str()),
    ] {
        let t0 = Instant::now();
        scan_text_for_secret_leaks(ok_text, label, &params)
            .unwrap_or_else(|e| panic!("it13 positive/perf `{label}` direct failed: {e:?}"));
        let mut app = make_app_with_params();
        app.description = ok_text.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("it13 positive/perf `{label}` app failed: {e:?}"));
        let elapsed = t0.elapsed().as_millis();
        assert!(
            elapsed < 500,
            "it13 positive/perf `{label}` took {elapsed} ms (>= 500 ms)"
        );
    }

    // 2. Defect B (serde_json 128-recursion-limit bypasses)
    let arr130 = format!("{}1{}", "[".repeat(130), "]".repeat(130));
    let arr200 = format!("{}1{}", "[".repeat(200), "]".repeat(200));
    let d_b1 = format!("prefix {{\"auth\\ntoken\": \"raw_secret_123456\", \"pad\": {arr130}}}");
    let d_b2 = format!("prefix {{\"pad\": {arr130}, \"auth\\ntoken\": \"raw_secret_123456\"}}");
    let d_b3 = format!("prefix {{\"a\": {arr200}}}");
    let d_b4 = format!("{{\"auth\\ntoken\": \"raw_secret_123456\", \"pad\": {arr130}}}");
    let d_b5 = format!("[{arr200}]");
    let d_b6 = format!("prefix {{\"auth\\ntoken\": \"raw_secret_123456\", \"pad\": {arr130}, bad}}");

    let mut defect_cases: Vec<(&str, String)> = vec![
        ("defect_b1_130_secret_first", d_b1),
        ("defect_b2_130_secret_last", d_b2),
        ("defect_b3_200_depth", d_b3),
        ("defect_b4_standalone_130", d_b4),
        ("defect_b5_top_arr_200", d_b5),
        ("defect_b6_130_malformed_tail", d_b6),
        // 3. Defect D (Duplicate-key \n-escaped JSON bypasses)
        (
            "defect_d1_dup_api_nl_prefix",
            r#"prefix {"api\nkey": "raw_secret_123456", "api\nkey": "{{auth_token}}"}"#.to_string(),
        ),
        (
            "defect_d2_dup_api_nl_standalone",
            r#"{"api\nkey": "raw_secret_123456", "api\nkey": "{{auth_token}}"}"#.to_string(),
        ),
        (
            "defect_d3_dup_unicode_nl_standalone",
            r#"{"api\u000akey": "raw_secret_123456", "api\nkey": "{{auth_token}}"}"#.to_string(),
        ),
        (
            "defect_d4_dup_nested_arr",
            r#"prefix {"items": [{"auth\ntoken": "raw_secret_123456", "auth\ntoken": "{{auth_token}}"}]}"#.to_string(),
        ),
    ];

    // 4. Defect C (9 Challenger vectors) + Reviewer p1_24 Findings 1-3
    for (label, vec_str) in [
        ("defect_c1_colon_sp_rbrack", "auth_token ]: raw_secret_123456"),
        ("defect_c2_colon_sp_rbrack_nosp", "auth_token ]:raw_secret_123456"),
        ("defect_c3_colon_quoted_nosp", r#"auth_token "prod":raw_secret_123456"#),
        ("defect_c4_colon_quoted_sp", r#"auth_token "prod": raw_secret_123456"#),
        ("defect_c5_colon_two_q_nosp", r#"auth_token "a" "b":raw_secret_123456"#),
        ("defect_c6_sub_q_sp_ident_eq", r#"auth_token[0] "a" b=raw_secret_123456"#),
        ("defect_c7_dot_q_sp_ident_eq", r#"auth_token."a" b=raw_secret_123456"#),
        ("defect_c8_sub_ident_sp_ident_eq", "auth_token[0]a b=raw_secret_123456"),
        ("defect_c9_sub_sp_ident_sp_ident_eq", "auth_token[0] a b=raw_secret_123456"),
        ("finding_1_empty_q_dot", "\"\".auth_token=raw_secret_123456"),
        ("finding_1_unclosed_q_dot", "\".auth_token=raw_secret_123456"),
        ("finding_1_empty_q_dot_q", "\"\".\"auth_token\"=raw_secret_123456"),
        ("finding_1_empty_q_sub_dot", "\"\"[0].auth_token=raw_secret_123456"),
        ("finding_1_url_empty_q_dot", "https://example.com/auth?\"\".auth_token=raw_secret_123456"),
        ("finding_2_lbrack_sp_q_colon", r#"auth_token[ "prod":raw_secret_123456"#),
        ("finding_2_lbrack_sp_ident_colon", "auth_token[ prod:raw_secret_123456"),
        ("finding_2_sub_sp_lbrack_sp_q_colon", r#"auth_token[0] [ "prod":raw_secret_123456"#),
        ("finding_2_sub_sp_lbrack_sp_ident_colon", "auth_token[0] [ prod:raw_secret_123456"),
        ("finding_2_sub_sp_lbrack_sp_q_colon_sp", r#"auth_token[0] [ "prod": raw_secret_123456"#),
        ("finding_2_sub_sp_lbrack_sp_ident_colon_sp", "auth_token[0] [ prod: raw_secret_123456"),
        ("finding_2_sp_lbrack_sp_q_eq", r#"auth_token [ "prod"=raw_secret_123456"#),
        ("finding_2_sp_lbrack_sp_ident_eq", "auth_token [ prod=raw_secret_123456"),
        ("finding_2_multi_lbrack_sp_q_colon", r#"auth_token [[[[[[[[ "prod":raw_secret_123456"#),
        ("finding_2_url_pct_lbrack_sp_ident", "https://example.com/auth?auth_token%5B%20prod:raw_secret_123456"),
        ("finding_2_url_pct_lbrack_sp_q", "https://example.com/auth?auth_token%5B%20%22prod%22:raw_secret_123456"),
        ("finding_2_url_pct_sp_lbrack_sp_eq", "https://example.com/auth?auth_token%20%5B%20prod=raw_secret_123456"),
    ] {
        defect_cases.push((label, vec_str.to_string()));
    }

    for (label, payload) in &defect_cases {
        let t0 = Instant::now();
        let mut app = make_app_with_params();
        app.description = payload.clone();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err_or_panic(label);
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it13 vector `{label}` must reject in app.description, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(payload, label, &params),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "it13 vector `{label}` must reject in scan_text_for_secret_leaks"
        );
        assert!(
            t0.elapsed().as_millis() < 500,
            "it13 vector `{label}` took {} ms (>= 500 ms)",
            t0.elapsed().as_millis()
        );
    }

    for bad_url in [
        r#"https://example.com/auth?"".auth_token=raw_secret_123456"#,
        "https://example.com/auth?auth_token%5B%20prod:raw_secret_123456",
        "https://example.com/auth?auth_token%5B%20%22prod%22:raw_secret_123456",
        "https://example.com/auth?auth_token%20%5B%20prod=raw_secret_123456",
        "https://example.com/auth?auth_token[0] a b=raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "Navigate URL `{bad_url}` must reject, got {err:?}"
        );
    }
}

#[test]
fn validate_script_app_remediates_it14_decreasing_dos_dup_array_and_h2_to_h5() {
    use bot_browser::scan_text_for_secret_leaks;
    use std::time::Instant;

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app
    };
    let params = make_app_with_params().parameters;

    // 1. Defect 1 (O(N^2) DoS on decreasing key_start offsets) + all 7 prose controls + URL path control
    let dec_dos_5k = format!(
        "{}{}",
        ",auth_token[".repeat(5_000),
        "]= {{auth_token}}, ".repeat(5_000)
    );
    let dec_dos_200k_5k = format!(
        "{}{}{}",
        "a".repeat(200_000),
        ",auth_token[".repeat(5_000),
        "]= {{auth_token}}, ".repeat(5_000)
    );
    let dec_dos_url_5k = format!(
        "https://example.com/auth?{}{}",
        "auth_token[".repeat(5_000),
        "]={{auth_token}}&".repeat(5_000)
    );
    let d6_nested_angle_basic_10k = format!("{}{}", "Basic < ".repeat(10_000), ">".repeat(10_000));
    let d5_unclosed_json_single_close_20k =
        format!("{}}}", "{\"a\": 1, \"b\": ".repeat(20_000));

    for (label, ok_text) in [
        ("prose_dot_note_ph", "Use auth_token. Note: {{auth_token}}"),
        ("prose_dot_note_ok", "Use auth_token. Note: ok"),
        ("prose_bracket_note_ok", "Use auth_token [note: ok]"),
        ("prose_dot_quoted_note_ok", r#"Use auth_token. "Note": ok"#),
        (
            "prose_dot_quoted_note_ph",
            r#"Use auth_token. "Note": {{auth_token}}"#,
        ),
        ("prose_two_quotes_note_ok", r#"Use "auth_token" "Note": ok"#),
        (
            "prose_bracket_note_ph",
            "Use auth_token [note: {{auth_token}}]",
        ),
        (
            "prose_sub_dot_note_ph",
            "Use auth_token[0]. Note: {{auth_token}}",
        ),
        (
            "url_path_dot_before_slash_ok",
            "https://example.com/auth_token.html?user=alice",
        ),
        ("dec_dos_5k", dec_dos_5k.as_str()),
        ("dec_dos_200k_5k", dec_dos_200k_5k.as_str()),
        ("dec_dos_url_5k", dec_dos_url_5k.as_str()),
        ("d6_nested_angle_basic_10k", d6_nested_angle_basic_10k.as_str()),
        (
            "d5_unclosed_json_single_close_20k",
            d5_unclosed_json_single_close_20k.as_str(),
        ),
    ] {
        let t0 = Instant::now();
        scan_text_for_secret_leaks(ok_text, label, &params)
            .unwrap_or_else(|e| panic!("it14 positive/perf `{label}` direct failed: {e:?}"));
        let mut app = make_app_with_params();
        app.description = ok_text.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("it14 positive/perf `{label}` app failed: {e:?}"));
        let elapsed = t0.elapsed().as_millis();
        assert!(
            elapsed < 500,
            "it14 positive/perf `{label}` took {elapsed} ms (>= 500 ms)"
        );
    }

    // 2. Defect 2 / H1 (Top-level & array-wrapped duplicate-key JSON + malformed \n-escaped JSON)
    let malformed_130_nl = format!(
        "prefix {{\"pad\": {}bad{}, \"api\\nkey\": \"raw_secret_123456\"}}",
        "[".repeat(130),
        "]".repeat(130)
    );
    let mut defect_cases: Vec<(&str, String)> = vec![
        (
            "h1_arr_auth_token",
            r#"[{"auth_token": "raw_secret_123456", "auth_token": "{{auth_token}}"}]"#.into(),
        ),
        (
            "h1_arr_api_nl",
            r#"[{"api\nkey": "raw_secret_123456", "api\nkey": "{{auth_token}}"}]"#.into(),
        ),
        (
            "h1_arr_auth_nl",
            r#"[{"auth\ntoken": "raw_secret_123456", "auth\ntoken": "{{auth_token}}"}]"#.into(),
        ),
        (
            "h1_arr_deep_wrap",
            r#"[[[{"auth_token": "raw_secret_123456", "auth_token": "{{auth_token}}"}]]]"#.into(),
        ),
        (
            "h1_arr_nested_obj",
            r#"[{"outer": {"api\nkey": "raw_secret_123456", "api\nkey": "{{auth_token}}"}}]"#
                .into(),
        ),
        (
            "h1_arr_nested_items",
            r#"[{"items": [{"auth\ntoken": "raw_secret_123456", "auth\ntoken": "{{auth_token}}"}]}]"#
                .into(),
        ),
        (
            "h1_arr_step_b_escaped",
            r#"[{\"api\\nkey\": \"raw_secret_123456\", \"api\\nkey\": \"{{auth_token}}\"}]"#
                .into(),
        ),
        (
            "h1_malformed_bad_nl_key",
            r#"prefix {"pad": [bad], "api\nkey": "raw_secret_123456"}"#.into(),
        ),
        ("h1_malformed_130_nl_key", malformed_130_nl),
    ];

    // 3. Defect 3 / H2-H5 + Challenger 3.1-3.10
    for (label, vec_str) in [
        ("h2_dot_ident_sp_ident_eq", "auth_token.a b=raw_secret_123456"),
        ("h2_dot_ident_sp_ident_colon", "auth_token.a b:raw_secret_123456"),
        ("h2_dot_ident_sp_ident_colon_sp", "auth_token.a b: raw_secret_123456"),
        ("h2_dot_q_sp_ident_colon_sp", r#"auth_token."a" b: raw_secret_123456"#),
        ("h2_dot_qident_sp_ident_eq", r#"auth_token."a"c b=raw_secret_123456"#),
        ("h2_dot_q_two_ident_eq", r#"auth_token."a" b c=raw_secret_123456"#),
        ("h2_sub_dot_qeq_ident_sp_eq", r#"auth_token[0]."a=b"d c=raw_secret_123456"#),
        ("h2_dot_qident2_sp_ident_eq", r#"auth_token."a"b c=raw_secret_123456"#),
        ("h2_sub_two_ident_colon_sp", "auth_token[0] a b: raw_secret_123456"),
        (
            "h2_url_dot_ident_sp_eq",
            "https://example.com/auth?auth_token.a%20b=raw_secret_123456",
        ),
        ("h3_sub_ident_sp_lbrack_colon_sp", "auth_token[0]a [ prod: raw_secret_123456"),
        ("h3_dot_ident_sp_lbrack_colon_sp", "auth_token.a [ prod: raw_secret_123456"),
        ("h3_sub_ident_sp_lbrack_q_colon_sp", r#"auth_token[0]a [ "prod": raw_secret_123456"#),
        ("h3_dot_ident_sp_lbrack_q_colon_sp", r#"auth_token.a [ "prod": raw_secret_123456"#),
        ("h4_q_root_two_q_colon_sp", r#""auth_token" "prod": raw_secret_123456"#),
        ("h4_ident_two_q_colon_sp", r#"auth_token "a" "b": raw_secret_123456"#),
        ("h4_ident_three_q_colon_sp", r#"auth_token "a" "b" "c": raw_secret_123456"#),
        ("h4_q_root_three_q_colon_sp", r#""auth_token" "a" "b": raw_secret_123456"#),
        (
            "h4_url_two_q_colon_sp",
            "https://example.com/auth?auth_token%20%22a%22%20%22b%22:%20raw_secret_123456",
        ),
        ("h5_root_bracketed_eq", "[auth_token]=raw_secret_123456"),
        ("h5_url_delim_bracketed_eq", "https://example.com?[auth_token]=raw_secret_123456"),
    ] {
        defect_cases.push((label, vec_str.to_string()));
    }

    for (label, payload) in &defect_cases {
        let mut app = make_app_with_params();
        app.description = payload.clone();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err_or_panic(label);
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it14 vector `{label}` must reject in app.description, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(payload, label, &params),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "it14 vector `{label}` must reject in scan_text_for_secret_leaks"
        );
    }

    for bad_url in [
        "https://example.com/auth?auth_token.a%20b=raw_secret_123456",
        "https://example.com/auth?auth_token%20%22a%22%20%22b%22:%20raw_secret_123456",
        "https://example.com?[auth_token]=raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it14 Navigate URL `{bad_url}` must reject, got {err:?}"
        );
    }
}

#[test]
fn validate_script_app_remediates_it15_quote_desync_tight_infix_dot_sp_and_h4_prefix() {
    use bot_browser::scan_text_for_secret_leaks;
    use std::time::Instant;

    let make_app_with_params = || {
        let mut app = base_valid_app();
        app.parameters.push(ScriptParameter {
            name: "auth_token".into(),
            description: "Auth token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app.parameters.push(ScriptParameter {
            name: "api_token".into(),
            description: "API token".into(),
            required: true,
            default_value: None,
            is_secret: true,
        });
        app
    };
    let params = make_app_with_params().parameters;

    // 1. All 14 prose / URL / UTF-8 positive controls + O(N) quote-chain scaling
    let quote_chain_dos = format!(
        "{}{}",
        "\"auth_token\" ".repeat(5_000),
        "b: {{auth_token}}, ".repeat(5_000)
    );
    for (label, ok_text) in [
        ("prose_dot_note_ph", "Use auth_token. Note: {{auth_token}}"),
        ("prose_dot_note_ok", "Use auth_token. Note: ok"),
        ("prose_sp_note_ok", "Use auth_token Note: ok"),
        ("prose_q_note_ok", r#"Use "auth_token" Note: ok"#),
        ("prose_bracket_note_ok", "Use auth_token [note: ok]"),
        ("prose_dot_quoted_note_ok", r#"Use auth_token. "Note": ok"#),
        (
            "prose_dot_quoted_note_ph",
            r#"Use auth_token. "Note": {{auth_token}}"#,
        ),
        ("prose_two_quotes_note_ok", r#"Use "auth_token" "Note": ok"#),
        (
            "prose_bracket_note_ph",
            "Use auth_token [note: {{auth_token}}]",
        ),
        (
            "prose_sub_dot_note_ph",
            "Use auth_token[0]. Note: {{auth_token}}",
        ),
        (
            "prose_utf8_dot_note_ph",
            "Use 中文 auth_token. Note: {{auth_token}}",
        ),
        (
            "url_path_dot_before_slash_ok",
            "https://example.com/auth_token.html?user=alice",
        ),
        (
            "url_utf8_path_dot_before_slash_ok",
            "https://example.com/中文/auth_token.html?user=alice",
        ),
        ("quote_chain_dos_5k", quote_chain_dos.as_str()),
    ] {
        let t0 = Instant::now();
        scan_text_for_secret_leaks(ok_text, label, &params)
            .unwrap_or_else(|e| panic!("it15 positive/perf `{label}` direct failed: {e:?}"));
        let mut app = make_app_with_params();
        app.description = ok_text.to_string();
        validate_script_app_safe_for_publish(&app)
            .unwrap_or_else(|e| panic!("it15 positive/perf `{label}` app failed: {e:?}"));
        let elapsed = t0.elapsed().as_millis();
        assert!(
            elapsed < 500,
            "it15 positive/perf `{label}` took {elapsed} ms (>= 500 ms)"
        );
    }

    // 2. Reviewer p1_28 Findings 1-3 & Challenger p1_28 Subclasses A-D
    let defect_cases = [
        // Finding 1: single-quote / double-quote desync
        (
            "f1_unclosed_sq_q_eq_colon_sp",
            r#"' auth_token."a=b" c: raw_secret_123456"#,
        ),
        (
            "f1_unclosed_dq_sq_eq_colon_sp",
            r#"" auth_token.'a=b' c: raw_secret_123456"#,
        ),
        (
            "f1_unclosed_sq_sub_q_eq_colon_sp",
            r#"' auth_token[0]."a=b" c: raw_secret_123456"#,
        ),
        (
            "f1_unclosed_sq_q_prose_colon_sp",
            r#"' auth_token."a. b" c: raw_secret_123456"#,
        ),
        (
            "f1_unclosed_sq_q_slash_colon_sp",
            r#"' auth_token."a/b" c: raw_secret_123456"#,
        ),
        (
            "f1_url_unclosed_sq_q_eq",
            r#"https://example.com/auth?'auth_token."a=b"%20c:%20raw_secret_123456"#,
        ),
        // Finding 3: tight quoted-infix chain
        (
            "f3_ident_q_ident_sp_lbrack_colon_sp",
            r#"auth_token"a"b [ prod: raw_secret_123456"#,
        ),
        (
            "f3_ident_q_ident_sp_ident_colon_sp",
            r#"auth_token"a"b c: raw_secret_123456"#,
        ),
        (
            "f3_ident_q_sp_ident_colon_sp",
            r#"auth_token"a" b: raw_secret_123456"#,
        ),
        // Finding 2 & Subclasses A/C: quote + : + unquoted ident & [ b prod:
        (
            "f2_q_root_ident_colon_sp",
            r#""auth_token" b: raw_secret_123456"#,
        ),
        (
            "sub_a_ident_q_ident_colon_sp",
            r#"auth_token "a" b: raw_secret_123456"#,
        ),
        (
            "sub_a_q_root_q_ident_colon_sp",
            r#""auth_token" "a" b: raw_secret_123456"#,
        ),
        (
            "sub_a_two_q_ident_colon_sp",
            r#"auth_token "a" "b" c: raw_secret_123456"#,
        ),
        (
            "sub_a_q_two_ident_colon_sp",
            r#"auth_token "a" b c: raw_secret_123456"#,
        ),
        (
            "sub_c_q_lbrack_sp_two_ident_colon",
            r#"auth_token "a" [ b prod: raw_secret_123456"#,
        ),
        (
            "sub_a_url_q_root_ident_colon_sp",
            "https://example.com/auth?%22auth_token%22%20b:%20raw_secret_123456",
        ),
        (
            "sub_a_url_ident_q_ident_colon_sp",
            "https://example.com/auth?auth_token%20%22a%22%20b:%20raw_secret_123456",
        ),
        // Subclass B: . + space before multi-token / quoted tail
        (
            "sub_b_sub_dot_sp_two_ident",
            "auth_token[0]. a b: raw_secret_123456",
        ),
        ("sub_b_dot_sp_two_ident", "auth_token. a b: raw_secret_123456"),
        (
            "sub_b_dot_sp_q_ident",
            r#"auth_token. "a" b: raw_secret_123456"#,
        ),
        (
            "sub_b_sub_dot_sp_q_ident",
            r#"auth_token[0]. "a" b: raw_secret_123456"#,
        ),
        (
            "sub_b_q_dot_sp_ident",
            r#"auth_token "a". b: raw_secret_123456"#,
        ),
        (
            "sub_b_q_dot_sp_q",
            r#"auth_token "a". "b": raw_secret_123456"#,
        ),
        (
            "sub_b_url_sub_dot_sp_two_ident",
            "https://example.com/auth?auth_token[0].%20a%20b:%20raw_secret_123456",
        ),
        // Subclass D: H4 non-sensitive prefix word
        (
            "sub_d_non_sens_word_two_q_colon",
            r#"foo "auth_token" "prod": raw_secret_123456"#,
        ),
        (
            "sub_d_url_non_sens_two_q_colon",
            "https://example.com/auth?foo%20%22auth_token%22%20%22prod%22:%20raw_secret_123456",
        ),
        (
            "sub_d_cap_prefix_secret_rhs",
            r#"Use "auth_token" "prod": raw_secret_123456"#,
        ),
    ];

    for (label, payload) in defect_cases {
        let mut app = make_app_with_params();
        app.description = payload.to_string();
        let err = validate_script_app_safe_for_publish(&app).unwrap_err_or_panic(label);
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it15 vector `{label}` must reject in app.description, got {err:?}"
        );
        assert!(
            matches!(
                scan_text_for_secret_leaks(payload, label, &params),
                Err(BotBrowserError::SecretLeakDetected(_))
            ),
            "it15 vector `{label}` must reject in scan_text_for_secret_leaks"
        );
    }

    for bad_url in [
        r#"https://example.com/auth?'auth_token."a=b"%20c:%20raw_secret_123456"#,
        "https://example.com/auth?%22auth_token%22%20b:%20raw_secret_123456",
        "https://example.com/auth?auth_token%20%22a%22%20b:%20raw_secret_123456",
        "https://example.com/auth?auth_token[0].%20a%20b:%20raw_secret_123456",
        "https://example.com/auth?foo%20%22auth_token%22%20%22prod%22:%20raw_secret_123456",
    ] {
        let mut app = make_app_with_params();
        app.steps = vec![ScriptStep::Navigate {
            url_template: bad_url.into(),
        }];
        let err = validate_script_app_safe_for_publish(&app).unwrap_err();
        assert!(
            matches!(err, BotBrowserError::SecretLeakDetected(_)),
            "it15 Navigate URL `{bad_url}` must reject, got {err:?}"
        );
    }
}

trait UnwrapErrOrPanic<E> {
    fn unwrap_err_or_panic(self, label: &str) -> E;
}

impl<E: std::fmt::Debug> UnwrapErrOrPanic<E> for Result<(), E> {
    fn unwrap_err_or_panic(self, label: &str) -> E {
        match self {
            Err(e) => e,
            Ok(()) => panic!("expected Err for `{label}`, got Ok(())"),
        }
    }
}
