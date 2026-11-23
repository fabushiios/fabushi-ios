use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use image::{ImageFormat, Rgba, RgbaImage};
use parking_lot::Mutex as SyncMutex;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::broadcast;

use bot_browser::cdp::{CdpTransport, LoginFormSelectors};
use bot_browser::orchestrator::{
    BotBrowserOrchestrator, OrchestratorConfig, SessionTransportFactory,
};
use bot_browser::totp::generate_totp_now;
use bot_browser::types::{
    AssistCardPayload, AssistCardStatus, BotBrowserError, BotBrowserEvent, LocalLoginSubmission,
    NormalizedClickPoint, NormalizedOrigin, ReusePolicy,
};
use bot_browser::vault::{CredentialVault, MemoryMasterKeyStore};
use zeroize::Zeroizing;

#[derive(Debug, Clone)]
struct RecordedCommand {
    method: String,
    params: Value,
}

struct MockPageState {
    url: String,
    frame_origin: Option<String>,
    title: String,
    cookies: Vec<Value>,
    element_rect: (f64, f64, f64, f64),
    screenshot_png: Vec<u8>,
    password_field_value: String,
    commands: Vec<RecordedCommand>,
    session_counter: usize,
    navigated_back_count: usize,
}

impl MockPageState {
    fn new(initial_url: &str, png: Vec<u8>) -> Self {
        Self {
            url: initial_url.to_string(),
            frame_origin: None,
            title: "Mock Auth Page".to_string(),
            cookies: Vec::new(),
            element_rect: (100.0, 200.0, 300.0, 160.0),
            screenshot_png: png,
            password_field_value: "preexisting-secret-pw".to_string(),
            commands: Vec::new(),
            session_counter: 0,
            navigated_back_count: 0,
        }
    }

    fn origin(&self) -> String {
        NormalizedOrigin::parse(&self.url)
            .map(|o| o.as_str().to_string())
            .unwrap_or_else(|_| self.url.clone())
    }
}

struct MockCdpTransport {
    state: Arc<SyncMutex<MockPageState>>,
    fail_first_close: AtomicBool,
}

impl MockCdpTransport {
    fn new(state: Arc<SyncMutex<MockPageState>>) -> Self {
        Self {
            state,
            fail_first_close: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl CdpTransport for MockCdpTransport {
    async fn send_command(&self, method: &str, params: Value) -> Result<Value, BotBrowserError> {
        let mut st = self.state.lock();
        st.commands.push(RecordedCommand {
            method: method.to_string(),
            params: params.clone(),
        });

        match method {
            "Page.navigate" => {
                if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
                    st.url = url.to_string();
                    st.navigated_back_count += 1;
                }
                Ok(json!({ "frameId": "main-frame" }))
            }
            "DOM.getDocument" => Ok(json!({ "root": { "nodeId": 1 } })),
            "DOM.querySelector" => {
                let sel = params.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                let node_id = match sel {
                    "#user" => 10,
                    "#pass" => 11,
                    "#totp" => 12,
                    _ => 13,
                };
                Ok(json!({ "nodeId": node_id }))
            }
            "DOM.focus" => Ok(json!({})),
            "Input.dispatchKeyEvent" | "Input.insertText" | "Input.dispatchMouseEvent" => {
                Ok(json!({}))
            }
            "Network.getCookies" | "Storage.getCookies" => {
                Ok(json!({ "cookies": st.cookies.clone() }))
            }
            "Page.captureScreenshot" => {
                let b64 = BASE64_STANDARD.encode(&st.screenshot_png);
                Ok(json!({ "data": b64 }))
            }
            "Runtime.evaluate" => {
                let expr = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if expr.contains("page_origin") && expr.contains("frame_origin") {
                    let page_origin = st.origin();
                    let frame_origin = st
                        .frame_origin
                        .clone()
                        .unwrap_or_else(|| page_origin.clone());
                    let payload = json!({
                        "page_origin": page_origin,
                        "frame_origin": frame_origin,
                        "href": st.url,
                    })
                    .to_string();
                    return Ok(json!({ "result": { "value": payload } }));
                }
                if expr.contains("redacted_field_count") && expr.contains("interactive_elements") {
                    let origin = st.origin();
                    let payload = json!({
                        "url": st.url,
                        "origin": origin,
                        "title": st.title,
                        "text_excerpt": "Sign in to continue",
                        "interactive_elements": [
                            {
                                "ref_id": "el_1",
                                "tag": "input",
                                "input_type": "text",
                                "name": "username",
                                "id": "user",
                                "text": "",
                                "value": "visible-hint",
                                "is_password": false,
                                "selector": "#user"
                            },
                            {
                                "ref_id": "el_2",
                                "tag": "input",
                                "input_type": "password",
                                "name": "password",
                                "id": "pass",
                                "text": "",
                                "value": st.password_field_value,
                                "is_password": true,
                                "selector": "#pass"
                            }
                        ],
                        "redacted_field_count": 1
                    })
                    .to_string();
                    return Ok(json!({ "result": { "value": payload } }));
                }
                if expr.contains("getBoundingClientRect") && expr.contains("viewport_width") {
                    let (x, y, w, h) = st.element_rect;
                    let payload = json!({
                        "found": true,
                        "x": x,
                        "y": y,
                        "width": w,
                        "height": h,
                        "device_pixel_ratio": 2.0,
                        "viewport_width": 800.0,
                        "viewport_height": 600.0,
                    })
                    .to_string();
                    return Ok(json!({ "result": { "value": payload } }));
                }
                if expr.contains("sensitive") && expr.contains("found") {
                    let is_pw = expr.contains("#pass");
                    let payload = json!({
                        "found": true,
                        "sensitive": is_pw,
                    })
                    .to_string();
                    return Ok(json!({ "result": { "value": payload } }));
                }
                if expr.contains("'CLICKED'") {
                    return Ok(json!({ "result": { "value": "CLICKED" } }));
                }
                Ok(json!({ "result": { "value": true } }))
            }
            _ => Ok(json!({})),
        }
    }

    async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
        let _ = self.fail_first_close.load(Ordering::Relaxed);
        Ok("http://127.0.0.1:0".to_string())
    }
}

struct MockSessionFactory {
    state: Arc<SyncMutex<MockPageState>>,
}

impl MockSessionFactory {
    fn new(state: Arc<SyncMutex<MockPageState>>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl SessionTransportFactory for MockSessionFactory {
    async fn create_transport(
        &self,
        _session_id: &str,
        _data_dir: &Path,
        _origin_hint: Option<&NormalizedOrigin>,
        _event_tx: &broadcast::Sender<BotBrowserEvent>,
    ) -> Result<Arc<dyn CdpTransport>, BotBrowserError> {
        self.state.lock().session_counter += 1;
        Ok(Arc::new(MockCdpTransport::new(Arc::clone(&self.state))))
    }
}

fn make_test_png(width: u32, height: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 255) as u8, (y % 255) as u8, 128, 255])
    });
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).unwrap();
    buf.into_inner()
}

fn make_orchestrator(
    dir: &TempDir,
    state: Arc<SyncMutex<MockPageState>>,
    config: OrchestratorConfig,
) -> Arc<BotBrowserOrchestrator> {
    let key_store = MemoryMasterKeyStore::new();
    let vault = Arc::new(
        CredentialVault::open(dir.path().join("vault.db"), &key_store).expect("open vault"),
    );
    let factory = Arc::new(MockSessionFactory::new(state));
    Arc::new(BotBrowserOrchestrator::with_transport_factory(
        config, vault, factory,
    ))
}

fn default_selectors() -> LoginFormSelectors {
    LoginFormSelectors {
        username_selector: "#user".to_string(),
        password_selector: "#pass".to_string(),
        totp_selector: Some("#totp".to_string()),
        submit_selector: Some("#submit".to_string()),
    }
}

#[tokio::test]
async fn test_full_end_to_end_vault_card_submit_list_and_atomic_fill_with_totp() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(SyncMutex::new(MockPageState::new(
        "https://app.site-a.com/login",
        make_test_png(800, 600),
    )));
    let orch = make_orchestrator(&dir, Arc::clone(&state), OrchestratorConfig::new(dir.path()));

    // 1. Snapshot redacts password value
    let snap = orch.tool_browser_snapshot("sess_1", None).await.unwrap();
    let serialized_snap = serde_json::to_string(&snap).unwrap();
    assert!(!serialized_snap.contains("preexisting-secret-pw"));
    assert_eq!(snap.redacted_field_count, 1);

    // 2. browser_type refuses password field
    let type_err = orch
        .browser_type("sess_1", Some("#pass"), "nope", false)
        .await
        .unwrap_err();
    assert!(matches!(type_err, BotBrowserError::Cdp(_)));

    // 3. Initial vault_list is empty
    let empty_list = orch.tool_vault_list("sess_1").await.unwrap();
    assert_eq!(empty_list.origin, "https://app.site-a.com");
    assert!(empty_list.credentials.is_empty());

    // 4. Request card & submit from local GPUI thread
    let mut events_rx = orch.subscribe_events();
    let orch_req = Arc::clone(&orch);
    let req_handle = tokio::spawn(async move {
        orch_req
            .tool_vault_request_card("sess_1", "task_1", Some(1001), "Site A", default_selectors())
            .await
    });

    let card_id = loop {
        match events_rx.recv().await.unwrap() {
            BotBrowserEvent::UpsertCard(AssistCardPayload::VaultLogin {
                card_id,
                origin,
                is_high_sensitivity,
                suggested_reuse_policy,
                status,
                ..
            }) => {
                assert_eq!(origin, "https://app.site-a.com");
                assert!(!is_high_sensitivity);
                assert_eq!(suggested_reuse_policy, ReusePolicy::AutoReuse);
                assert_eq!(status, AssistCardStatus::AwaitingUser);
                break card_id;
            }
            _ => continue,
        }
    };

    let totp_secret = "JBSWY3DPEHPK3PXP";
    let secret_pw = "UltraS3cret!Pw#42";
    let secret_user = "alice.wonder@site-a.com";

    let submit_res = orch
        .submit_login_card(LocalLoginSubmission {
            card_id: card_id.clone(),
            username: Zeroizing::new(secret_user.to_string()),
            password: Zeroizing::new(secret_pw.to_string()),
            totp_secret: Some(Zeroizing::new(totp_secret.to_string())),
            account_hint: None,
            allow_auto_reuse: true,
        })
        .await
        .unwrap();
    assert!(!submit_res.resumed_timed_out_task);

    let req_res = req_handle.await.unwrap().unwrap();
    assert_eq!(req_res.status, "saved");
    assert_eq!(req_res.card_id, card_id);
    let cred_id = req_res.credential_id.clone().unwrap();
    let req_json = serde_json::to_string(&req_res).unwrap();
    assert!(!req_json.contains(secret_pw));
    assert!(!req_json.contains(secret_user));
    assert!(!req_json.contains(totp_secret));

    // 5. vault_list now returns non-secret CredentialMeta
    let listed = orch.tool_vault_list("sess_1").await.unwrap();
    assert_eq!(listed.credentials.len(), 1);
    let meta = &listed.credentials[0];
    assert_eq!(meta.credential_id, cred_id);
    assert!(meta.has_totp);
    assert_eq!(meta.reuse_policy, ReusePolicy::AutoReuse);
    let listed_json = serde_json::to_string(&listed).unwrap();
    assert!(!listed_json.contains(secret_pw));
    assert!(!listed_json.contains(secret_user));
    assert!(!listed_json.contains(totp_secret));

    // 6. vault_fill atomically injects username, password, and TOTP via Input.insertText
    let expected_totp = generate_totp_now(totp_secret).unwrap();
    let fill_res = orch
        .tool_vault_fill("sess_1", "task_1", Some(1001), &cred_id, default_selectors())
        .await
        .unwrap();
    assert_eq!(fill_res.status, "filled_and_submitted");
    assert_eq!(fill_res.origin, "https://app.site-a.com");
    assert!(fill_res.used_totp);

    let cmds = state.lock().commands.clone();
    let inserted_texts: Vec<String> = cmds
        .iter()
        .filter(|c| c.method == "Input.insertText")
        .filter_map(|c| c.params.get("text").and_then(|v| v.as_str()).map(ToString::to_string))
        .collect();
    assert_eq!(inserted_texts.len(), 3);
    assert_eq!(inserted_texts[0], secret_user);
    assert_eq!(inserted_texts[1], secret_pw);
    assert_eq!(inserted_texts[2], expected_totp.as_str());

    for cmd in cmds.iter().filter(|c| c.method == "Runtime.evaluate") {
        let expr = cmd.params.get("expression").and_then(|v| v.as_str()).unwrap_or("");
        assert!(!expr.contains(secret_pw));
        assert!(!expr.contains(secret_user));
        assert!(!expr.contains(totp_secret));
    }
}

#[tokio::test]
async fn test_cross_origin_prompt_injection_and_iframe_blocked() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(SyncMutex::new(MockPageState::new(
        "https://app.site-a.com/login",
        make_test_png(800, 600),
    )));
    let orch = make_orchestrator(&dir, Arc::clone(&state), OrchestratorConfig::new(dir.path()));

    let meta = orch
        .vault()
        .store_credential(
            "https://app.site-a.com",
            "user@a.com",
            "TopSecretA!99",
            None,
            None,
            Some(ReusePolicy::AutoReuse),
        )
        .unwrap();

    // Navigate to evil.site-b.com
    orch.browser_navigate("sess_evil", "https://evil.site-b.com/phish")
        .await
        .unwrap();

    let err = orch
        .tool_vault_fill(
            "sess_evil",
            "task_evil",
            Some(99),
            &meta.credential_id,
            default_selectors(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        BotBrowserError::OriginMismatch { ref expected, ref actual }
            if expected == "https://app.site-a.com" && actual == "https://evil.site-b.com"
    ));

    // Also test cross-origin iframe inside site-a.com
    orch.browser_navigate("sess_evil", "https://app.site-a.com/login")
        .await
        .unwrap();
    state.lock().frame_origin = Some("https://evil-iframe.site-c.com".to_string());

    let frame_err = orch
        .tool_vault_fill(
            "sess_evil",
            "task_evil_frame",
            Some(99),
            &meta.credential_id,
            default_selectors(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        frame_err,
        BotBrowserError::FrameOriginMismatch { ref expected, ref actual }
            if expected == "https://app.site-a.com" && actual == "https://evil-iframe.site-c.com"
    ));

    // Zero Input.insertText calls must ever occur
    let cmds = &state.lock().commands;
    assert!(!cmds.iter().any(|c| c.method == "Input.insertText"));
}

#[tokio::test]
async fn test_decoupled_timeout_and_late_submit_auto_wakeup() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(SyncMutex::new(MockPageState::new(
        "https://app.site-a.com/auth/login",
        make_test_png(800, 600),
    )));
    let mut cfg = OrchestratorConfig::new(dir.path());
    cfg.card_wait_timeout = Duration::from_millis(50);
    let orch = make_orchestrator(&dir, Arc::clone(&state), cfg);

    let mut events_rx = orch.subscribe_events();
    let mut wakeups_rx = orch.subscribe_wakeups();

    let req_res = orch
        .tool_vault_request_card(
            "sess_late",
            "task_late_42",
            Some(777),
            "Site A Login",
            default_selectors(),
        )
        .await
        .unwrap();

    assert_eq!(req_res.status, "timed_out_card_still_active");
    let card_id = req_res.card_id.clone();

    let card_after_timeout = orch.get_card(&card_id).expect("card must stay active");
    assert_eq!(card_after_timeout.status(), AssistCardStatus::TimedOutResumable);
    if let AssistCardPayload::VaultLogin { status_note, .. } = &card_after_timeout {
        assert_eq!(
            status_note.as_deref(),
            Some("上轮已超时，补填后将自动保存并续跑")
        );
    } else {
        panic!("expected VaultLogin card");
    }

    // Navigate away and close the session to prove late-fill restores session & target_url
    orch.browser_navigate("sess_late", "https://unrelated.example.org/home")
        .await
        .unwrap();
    orch.close_session("sess_late").await.unwrap();

    // Wait 100ms past timeout before user late-submits the card
    tokio::time::sleep(Duration::from_millis(100)).await;

    let outcome = orch
        .submit_login_card(LocalLoginSubmission {
            card_id: card_id.clone(),
            username: Zeroizing::new("late_user@site-a.com".to_string()),
            password: Zeroizing::new("LateP@ssword!88".to_string()),
            totp_secret: None,
            account_hint: None,
            allow_auto_reuse: true,
        })
        .await
        .unwrap();

    assert!(outcome.resumed_timed_out_task);
    assert!(outcome.auto_filled);
    assert!(!outcome.used_totp);
    assert_eq!(outcome.origin, "https://app.site-a.com");

    // Card is now SavedAndLoggedIn
    let final_card = orch.get_card(&card_id).unwrap();
    assert_eq!(final_card.status(), AssistCardStatus::SavedAndLoggedIn);

    // Wakeup arrived on subscribe_wakeups() and subscribe_events()
    let wakeup = tokio::time::timeout(Duration::from_secs(1), wakeups_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wakeup.task_id, "task_late_42");
    assert_eq!(wakeup.session_id, "sess_late");
    assert_eq!(wakeup.chat_id, Some(777));
    assert_eq!(wakeup.target_url, "https://app.site-a.com/auth/login");
    assert_eq!(wakeup.credential_id, outcome.credential_id);

    let mut saw_wakeup_event = false;
    while let Ok(ev) = events_rx.try_recv() {
        if let BotBrowserEvent::TaskWakeupTriggered(ev_w) = ev {
            assert_eq!(ev_w.task_id, "task_late_42");
            assert_eq!(ev_w.credential_id, outcome.credential_id);
            saw_wakeup_event = true;
        }
    }
    assert!(saw_wakeup_event);

    // Verify browser navigated back to target_url and filled via Input.insertText
    let st = state.lock();
    assert_eq!(st.url, "https://app.site-a.com/auth/login");
    assert!(st.session_counter >= 2);
    let inserted: Vec<String> = st
        .commands
        .iter()
        .filter(|c| c.method == "Input.insertText")
        .filter_map(|c| c.params.get("text").and_then(|v| v.as_str()).map(ToString::to_string))
        .collect();
    assert_eq!(inserted, vec!["late_user@site-a.com", "LateP@ssword!88"]);
}

#[tokio::test]
async fn test_high_sensitivity_domain_requires_reuse_confirm_and_denies_when_rejected() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(SyncMutex::new(MockPageState::new(
        "https://dashboard.stripe.com/login",
        make_test_png(800, 600),
    )));
    let orch = make_orchestrator(&dir, Arc::clone(&state), OrchestratorConfig::new(dir.path()));

    // Default reuse_policy for stripe.com must be ConfirmEveryTime
    let meta = orch
        .vault()
        .store_credential(
            "https://dashboard.stripe.com",
            "finance@acme.io",
            "StripeSuperSecret!1",
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(meta.reuse_policy, ReusePolicy::ConfirmEveryTime);

    // 1. Approve flow
    let mut events_rx = orch.subscribe_events();
    let orch_fill = Arc::clone(&orch);
    let cred_id = meta.credential_id.clone();
    let fill_handle = tokio::spawn(async move {
        orch_fill
            .tool_vault_fill("sess_stripe", "task_s1", Some(55), &cred_id, default_selectors())
            .await
    });

    let confirm_card_id = loop {
        match events_rx.recv().await.unwrap() {
            BotBrowserEvent::UpsertCard(AssistCardPayload::ReuseConfirm {
                card_id,
                origin,
                require_biometric_hint,
                ..
            }) => {
                assert_eq!(origin, "https://dashboard.stripe.com");
                assert!(require_biometric_hint);
                break card_id;
            }
            _ => continue,
        }
    };

    let approval = orch.approve_reuse_card(&confirm_card_id, true).await.unwrap();
    assert!(approval.approved);
    assert!(!approval.resumed_timed_out_task);

    let fill_res = fill_handle.await.unwrap().unwrap();
    assert_eq!(fill_res.status, "filled_and_submitted");
    assert_eq!(
        orch.get_card(&confirm_card_id).unwrap().status(),
        AssistCardStatus::Completed
    );

    // 2. Deny flow must block with ApprovalDenied and emit zero new insertText calls
    let insert_count_before = state
        .lock()
        .commands
        .iter()
        .filter(|c| c.method == "Input.insertText")
        .count();

    let orch_deny = Arc::clone(&orch);
    let cred_id2 = meta.credential_id.clone();
    let deny_handle = tokio::spawn(async move {
        orch_deny
            .tool_vault_fill("sess_stripe", "task_s2", Some(55), &cred_id2, default_selectors())
            .await
    });

    let deny_card_id = loop {
        match events_rx.recv().await.unwrap() {
            BotBrowserEvent::UpsertCard(AssistCardPayload::ReuseConfirm { card_id, .. }) => {
                break card_id;
            }
            _ => continue,
        }
    };

    let denied_outcome = orch.approve_reuse_card(&deny_card_id, false).await.unwrap();
    assert!(!denied_outcome.approved);

    let deny_err = deny_handle.await.unwrap().unwrap_err();
    assert!(matches!(deny_err, BotBrowserError::ApprovalDenied(_)));
    assert_eq!(
        orch.get_card(&deny_card_id).unwrap().status(),
        AssistCardStatus::Denied
    );

    let insert_count_after = state
        .lock()
        .commands
        .iter()
        .filter(|c| c.method == "Input.insertText")
        .count();
    assert_eq!(insert_count_before, insert_count_after);
}

#[tokio::test]
async fn test_qr_polling_refreshes_then_completes_on_redirect_and_cookie() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(SyncMutex::new(MockPageState::new(
        "https://qr.example.com/login",
        make_test_png(800, 600),
    )));
    let mut cfg = OrchestratorConfig::new(dir.path());
    cfg.qr_refresh_interval = Duration::from_millis(30);
    cfg.qr_max_wait = Duration::from_secs(5);
    let orch = make_orchestrator(&dir, Arc::clone(&state), cfg);

    let mut events_rx = orch.subscribe_events();
    let orch_qr = Arc::clone(&orch);
    let qr_handle = tokio::spawn(async move {
        orch_qr
            .tool_human_assist_qr(
                "sess_qr",
                "task_qr",
                Some(88),
                "#qr-img",
                Some("/dashboard"),
                vec!["session_token".to_string()],
            )
            .await
    });

    let mut refresh_count = 0u32;
    let mut card_id_seen = None;

    loop {
        let ev = tokio::time::timeout(Duration::from_secs(3), events_rx.recv())
            .await
            .unwrap()
            .unwrap();
        match ev {
            BotBrowserEvent::UpsertCard(AssistCardPayload::QrLogin {
                card_id,
                png_bytes,
                ..
            }) => {
                assert!(!png_bytes.is_empty());
                // Verify cropped PNG dimensions match element_rect (300x160)
                let decoded = image::load_from_memory(&png_bytes).unwrap();
                assert_eq!(decoded.width(), 300);
                assert_eq!(decoded.height(), 160);
                card_id_seen = Some(card_id);
            }
            BotBrowserEvent::RefreshQrImage { poll_attempt, .. } => {
                refresh_count += 1;
                if poll_attempt >= 2 {
                    let mut st = state.lock();
                    st.url = "https://qr.example.com/dashboard".to_string();
                    st.cookies.push(json!({
                        "name": "session_token",
                        "value": "tok_abc123"
                    }));
                }
            }
            BotBrowserEvent::UpdateCardStatus {
                card_id,
                status: AssistCardStatus::Completed,
                redirected_url,
                ..
            } => {
                assert_eq!(Some(card_id), card_id_seen);
                assert_eq!(
                    redirected_url.as_deref(),
                    Some("https://qr.example.com/dashboard")
                );
                break;
            }
            _ => {}
        }
    }

    assert!(refresh_count >= 2, "expected >=2 refreshes, got {refresh_count}");

    let qr_res = qr_handle.await.unwrap().unwrap();
    assert_eq!(qr_res.status, "qr_login_succeeded");
    assert_eq!(
        qr_res.redirected_url.as_deref(),
        Some("https://qr.example.com/dashboard")
    );
    assert_eq!(qr_res.matched_cookie.as_deref(), Some("session_token"));
}

#[tokio::test]
async fn test_click_captcha_dispatches_scaled_element_relative_coordinates() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(SyncMutex::new(MockPageState::new(
        "https://captcha.example.com/challenge",
        make_test_png(800, 600),
    )));
    let orch = make_orchestrator(&dir, Arc::clone(&state), OrchestratorConfig::new(dir.path()));

    let mut events_rx = orch.subscribe_events();
    let orch_click = Arc::clone(&orch);
    let click_handle = tokio::spawn(async move {
        orch_click
            .tool_human_assist_click(
                "sess_click",
                "task_click",
                Some(333),
                "#captcha-img",
                "Click the traffic light",
                2,
            )
            .await
    });

    let card_id = loop {
        match events_rx.recv().await.unwrap() {
            BotBrowserEvent::UpsertCard(AssistCardPayload::ClickCaptcha {
                card_id,
                element_rect,
                png_bytes,
                ..
            }) => {
                assert_eq!(element_rect.x, 100.0);
                assert_eq!(element_rect.y, 200.0);
                assert_eq!(element_rect.width, 300.0);
                assert_eq!(element_rect.height, 160.0);
                let decoded = image::load_from_memory(&png_bytes).unwrap();
                assert_eq!(decoded.width(), 300);
                assert_eq!(decoded.height(), 160);
                break card_id;
            }
            _ => continue,
        }
    };

    let count = orch
        .submit_click_captcha(&card_id, vec![NormalizedClickPoint::new(0.5, 0.25)])
        .await
        .unwrap();
    assert_eq!(count, 1);

    let click_res = click_handle.await.unwrap().unwrap();
    assert_eq!(click_res.status, "clicks_dispatched");
    assert_eq!(click_res.count, 1);
    assert_eq!(click_res.dispatched_points, vec![(250.0, 240.0)]);
    assert_eq!(
        orch.get_card(&card_id).unwrap().status(),
        AssistCardStatus::Completed
    );

    let mouse_events: Vec<(String, f64, f64)> = state
        .lock()
        .commands
        .iter()
        .filter(|c| c.method == "Input.dispatchMouseEvent")
        .map(|c| {
            (
                c.params["type"].as_str().unwrap_or("").to_string(),
                c.params["x"].as_f64().unwrap_or(-1.0),
                c.params["y"].as_f64().unwrap_or(-1.0),
            )
        })
        .collect();
    assert_eq!(
        mouse_events,
        vec![
            ("mouseMoved".to_string(), 250.0, 240.0),
            ("mousePressed".to_string(), 250.0, 240.0),
            ("mouseReleased".to_string(), 250.0, 240.0),
        ]
    );
}

#[tokio::test]
async fn test_reuse_confirm_late_approval_auto_wakeup_and_cancel_card() {
    let dir = TempDir::new().unwrap();
    let state = Arc::new(SyncMutex::new(MockPageState::new(
        "https://dashboard.stripe.com/login",
        make_test_png(800, 600),
    )));
    let mut cfg = OrchestratorConfig::new(dir.path());
    cfg.card_wait_timeout = Duration::from_millis(40);
    cfg.qr_refresh_interval = Duration::from_millis(50);
    cfg.qr_max_wait = Duration::from_secs(5);
    let orch = make_orchestrator(&dir, Arc::clone(&state), cfg);

    let meta = orch
        .vault()
        .store_credential(
            "https://dashboard.stripe.com",
            "late_stripe@acme.io",
            "LateStripePw!77",
            None,
            None,
            None,
        )
        .unwrap();

    let mut wakeups_rx = orch.subscribe_wakeups();
    let fill_res = orch
        .tool_vault_fill(
            "sess_stripe_late",
            "task_stripe_late",
            Some(909),
            &meta.credential_id,
            default_selectors(),
        )
        .await
        .unwrap();
    assert_eq!(fill_res.status, "approval_timed_out_card_still_active");
    let card_id = fill_res.card_id.unwrap();
    assert_eq!(
        orch.get_card(&card_id).unwrap().status(),
        AssistCardStatus::TimedOutResumable
    );

    tokio::time::sleep(Duration::from_millis(60)).await;
    let outcome = orch.approve_reuse_card(&card_id, true).await.unwrap();
    assert!(outcome.approved);
    assert!(outcome.resumed_timed_out_task);
    assert!(outcome.auto_filled);
    assert_eq!(
        orch.get_card(&card_id).unwrap().status(),
        AssistCardStatus::Completed
    );

    let wakeup = tokio::time::timeout(Duration::from_secs(1), wakeups_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wakeup.task_id, "task_stripe_late");
    assert_eq!(wakeup.credential_id, meta.credential_id);

    // Verify cancel_card stops an active QR polling session promptly
    let mut events_rx = orch.subscribe_events();
    let orch_qr = Arc::clone(&orch);
    let qr_handle = tokio::spawn(async move {
        orch_qr
            .tool_human_assist_qr("sess_stripe_late", "task_qr_cancel", Some(909), "#qr", None, vec![])
            .await
    });
    let qr_card_id = loop {
        match events_rx.recv().await.unwrap() {
            BotBrowserEvent::UpsertCard(AssistCardPayload::QrLogin { card_id, .. }) => {
                break card_id;
            }
            _ => continue,
        }
    };
    orch.cancel_card(&qr_card_id).await.unwrap();
    let qr_err = qr_handle.await.unwrap().unwrap_err();
    assert!(matches!(qr_err, BotBrowserError::ApprovalDenied(_)));
    assert_eq!(
        orch.get_card(&qr_card_id).unwrap().status(),
        AssistCardStatus::Cancelled
    );
}
