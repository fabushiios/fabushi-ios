use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use image::{ImageFormat, Rgba, RgbaImage};
use parking_lot::Mutex as SyncMutex;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::broadcast;
use zeroize::Zeroizing;

use bot_browser::cdp::CdpTransport;
use bot_browser::orchestrator::{
    BotBrowserOrchestrator, OrchestratorConfig, SessionTransportFactory,
};
use bot_browser::types::{
    AssistCardPayload, BotBrowserError, BotBrowserEvent, LocalLoginSubmission,
    NormalizedClickPoint, NormalizedOrigin,
};
use bot_browser::vault::{CredentialVault, MemoryMasterKeyStore};

use gpui_telegram_agent::fabushi_bot::{FabushiBotHost, FabushiBotProvisionSpec};
use gpui_telegram_agent::hermes::tools_web_media::execute_web_media_tool;
use gpui_telegram_agent::hermes::toolsets::{
    HermesToolContext, HermesToolExecutor, HermesToolResult, ToolsetId, ToolsetPreset,
    all_builtin_tool_specs,
};

struct MockPage {
    url: String,
    cookies: Vec<Value>,
    png: Vec<u8>,
    commands: Vec<(String, Value)>,
}

impl MockPage {
    fn new(url: &str, png: Vec<u8>) -> Self {
        Self {
            url: url.to_string(),
            cookies: Vec::new(),
            png,
            commands: Vec::new(),
        }
    }

    fn origin(&self) -> String {
        NormalizedOrigin::parse(&self.url)
            .map(|o| o.as_str().to_string())
            .unwrap_or_else(|_| self.url.clone())
    }
}

struct MockCdpTransport(Arc<SyncMutex<MockPage>>);

#[async_trait]
impl CdpTransport for MockCdpTransport {
    async fn send_command(&self, method: &str, params: Value) -> Result<Value, BotBrowserError> {
        let mut st = self.0.lock();
        st.commands.push((method.to_string(), params.clone()));
        match method {
            "Page.navigate" => {
                if let Some(u) = params.get("url").and_then(|v| v.as_str()) {
                    st.url = u.to_string();
                }
                Ok(json!({ "frameId": "main" }))
            }
            "DOM.getDocument" => Ok(json!({ "root": { "nodeId": 1 } })),
            "DOM.querySelector" => Ok(json!({ "nodeId": 42 })),
            "DOM.focus"
            | "Input.dispatchKeyEvent"
            | "Input.insertText"
            | "Input.dispatchMouseEvent" => Ok(json!({})),
            "Network.getCookies" | "Storage.getCookies" => {
                Ok(json!({ "cookies": st.cookies.clone() }))
            }
            "Page.captureScreenshot" => Ok(json!({ "data": BASE64_STANDARD.encode(&st.png) })),
            "Runtime.evaluate" => {
                let expr = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if expr.contains("page_origin") && expr.contains("frame_origin") {
                    let origin = st.origin();
                    let val = json!({
                        "page_origin": origin,
                        "frame_origin": origin,
                        "href": st.url,
                    })
                    .to_string();
                    return Ok(json!({ "result": { "value": val } }));
                }
                if expr.contains("redacted_field_count") && expr.contains("interactive_elements") {
                    let origin = st.origin();
                    let val = json!({
                        "url": st.url,
                        "origin": origin,
                        "title": "Login Portal",
                        "text_excerpt": "Sign in",
                        "interactive_elements": [
                            {
                                "ref_id": "el_1",
                                "tag": "input",
                                "input_type": "text",
                                "name": "email",
                                "id": "email",
                                "text": "",
                                "value": "public-hint",
                                "is_password": false,
                                "selector": "#email"
                            },
                            {
                                "ref_id": "el_2",
                                "tag": "input",
                                "input_type": "password",
                                "name": "password",
                                "id": "password",
                                "text": "",
                                "value": "S3cr3tP@ssw0rd!99",
                                "is_password": true,
                                "selector": "#password"
                            }
                        ],
                        "redacted_field_count": 1
                    })
                    .to_string();
                    return Ok(json!({ "result": { "value": val } }));
                }
                if expr.contains("getBoundingClientRect") && expr.contains("viewport_width") {
                    let val = json!({
                        "found": true,
                        "x": 20.0,
                        "y": 40.0,
                        "width": 120.0,
                        "height": 120.0,
                        "device_pixel_ratio": 1.0,
                        "viewport_width": 400.0,
                        "viewport_height": 400.0
                    })
                    .to_string();
                    return Ok(json!({ "result": { "value": val } }));
                }
                if expr.contains("sensitive") && expr.contains("found") {
                    let is_pw = expr.contains("#password");
                    let val = json!({ "found": true, "sensitive": is_pw }).to_string();
                    return Ok(json!({ "result": { "value": val } }));
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
        Ok("http://127.0.0.1:0".to_string())
    }
}

struct MockFactory(Arc<SyncMutex<MockPage>>);

#[async_trait]
impl SessionTransportFactory for MockFactory {
    async fn create_transport(
        &self,
        _session_id: &str,
        _data_dir: &Path,
        _origin_hint: Option<&NormalizedOrigin>,
        _event_tx: &broadcast::Sender<BotBrowserEvent>,
    ) -> Result<Arc<dyn CdpTransport>, BotBrowserError> {
        Ok(Arc::new(MockCdpTransport(Arc::clone(&self.0))))
    }
}

fn make_png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| Rgba([x as u8, y as u8, 200, 255]));
    let mut cur = Cursor::new(Vec::new());
    img.write_to(&mut cur, ImageFormat::Png).unwrap();
    cur.into_inner()
}

fn assert_no_secrets_in_result(res: &HermesToolResult, secrets: &[&str]) {
    let serialized = serde_json::to_string(res).unwrap();
    for secret in secrets {
        assert!(
            !serialized.contains(secret),
            "tool '{}' leaked secret '{}' in result: {}",
            res.tool_name,
            secret,
            serialized
        );
    }
}

#[tokio::test]
async fn test_hermes_browser_vault_and_assist_tools_zero_knowledge() {
    let dir = TempDir::new().unwrap();
    let page = Arc::new(SyncMutex::new(MockPage::new(
        "https://portal.example.com/login",
        make_png(400, 400),
    )));

    let mut cfg = OrchestratorConfig::new(dir.path());
    cfg.qr_refresh_interval = Duration::from_millis(20);
    cfg.qr_max_wait = Duration::from_secs(4);
    let vault = Arc::new(
        CredentialVault::open(dir.path().join("vault.db"), &MemoryMasterKeyStore::new()).unwrap(),
    );
    let orch = Arc::new(BotBrowserOrchestrator::with_transport_factory(
        cfg,
        vault,
        Arc::new(MockFactory(Arc::clone(&page))),
    ));

    // Verify Browser toolset spec registration and zero Chrome references
    let browser_specs: Vec<_> = all_builtin_tool_specs()
        .into_iter()
        .filter(|s| s.toolset == ToolsetId::Browser)
        .collect();
    let names: Vec<_> = browser_specs.iter().map(|s| s.name.as_str()).collect();
    for expected in [
        "browser_navigate",
        "browser_snapshot",
        "browser_click",
        "browser_type",
        "browser_screenshot",
        "vault_list",
        "vault_request_card",
        "vault_fill",
        "human_assist_qr",
        "human_assist_click",
    ] {
        assert!(names.contains(&expected), "missing browser tool '{expected}'");
    }
    assert!(!ToolsetId::Browser.description().to_ascii_lowercase().contains("chrome"));
    for spec in &browser_specs {
        assert!(
            !spec.description.to_ascii_lowercase().contains("chrome"),
            "tool '{}' description still mentions Chrome: {}",
            spec.name,
            spec.description
        );
    }

    let ctx = HermesToolContext::new(dir.path())
        .with_session("hermes_sess_1", 1)
        .with_chat_and_bot(Some(4242), Some(7))
        .with_bot_browser(Arc::clone(&orch));

    let secret_user = "alice@portal.example.com";
    let secret_pw = "S3cr3tP@ssw0rd!99";
    let secret_totp = "JBSWY3DPEHPK3PXP";
    let forbidden = [secret_user, secret_pw, secret_totp];

    // 1. browser_navigate
    let nav = HermesToolExecutor::execute(
        "browser_navigate",
        json!({ "url": "https://portal.example.com/login" }),
        &ctx,
    )
    .await;
    assert!(nav.success, "{}", nav.output);
    assert_no_secrets_in_result(&nav, &forbidden);

    // 2. browser_snapshot must redact password fields
    let snap = HermesToolExecutor::execute("browser_snapshot", json!({}), &ctx).await;
    assert!(snap.success, "{}", snap.output);
    assert_no_secrets_in_result(&snap, &forbidden);
    let snap_struct = snap.structured.as_ref().unwrap();
    assert_eq!(snap_struct["redacted_field_count"], 1);
    assert_eq!(
        snap_struct["interactive_elements"][1]["value"],
        "[REDACTED]"
    );

    // 3. browser_type works for normal input & refuses password selector
    let ok_type = HermesToolExecutor::execute(
        "browser_type",
        json!({ "selector": "#email", "text": "non-secret-note" }),
        &ctx,
    )
    .await;
    assert!(ok_type.success, "{}", ok_type.output);
    let blocked_type = HermesToolExecutor::execute(
        "browser_type",
        json!({ "selector": "#password", "text": "attempt" }),
        &ctx,
    )
    .await;
    assert!(!blocked_type.success);

    // 4. browser_click + browser_screenshot
    let clk = HermesToolExecutor::execute(
        "browser_click",
        json!({ "selector": "#next-btn" }),
        &ctx,
    )
    .await;
    assert!(clk.success, "{}", clk.output);
    let shot = HermesToolExecutor::execute(
        "browser_screenshot",
        json!({ "output_path": "shot.png" }),
        &ctx,
    )
    .await;
    assert!(shot.success, "{}", shot.output);
    assert!(dir.path().join("shot.png").is_file());
    assert_eq!(shot.artifacts.len(), 1);

    // 5. vault_list -> vault_request_card -> local submit -> vault_list -> vault_fill
    let list_empty = execute_web_media_tool("vault_list", json!({}), &ctx).await;
    assert!(list_empty.success, "{}", list_empty.output);
    assert_no_secrets_in_result(&list_empty, &forbidden);

    let mut events_rx = orch.subscribe_events();
    let ctx_req = ctx.clone();
    let req_handle = tokio::spawn(async move {
        HermesToolExecutor::execute(
            "vault_request_card",
            json!({
                "site_title": "Portal Login",
                "username_selector": "#email",
                "password_selector": "#password",
                "totp_selector": "#totp",
                "submit_selector": "#submit"
            }),
            &ctx_req,
        )
        .await
    });

    let login_card_id = loop {
        match events_rx.recv().await.unwrap() {
            BotBrowserEvent::UpsertCard(AssistCardPayload::VaultLogin { card_id, .. }) => {
                break card_id;
            }
            _ => continue,
        }
    };

    orch.submit_login_card(LocalLoginSubmission {
        card_id: login_card_id,
        username: Zeroizing::new(secret_user.to_string()),
        password: Zeroizing::new(secret_pw.to_string()),
        totp_secret: Some(Zeroizing::new(secret_totp.to_string())),
        account_hint: None,
        allow_auto_reuse: true,
    })
    .await
    .unwrap();

    let req_res = req_handle.await.unwrap();
    assert!(req_res.success, "{}", req_res.output);
    assert_no_secrets_in_result(&req_res, &forbidden);
    let cred_id = req_res.structured.as_ref().unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();

    let list_after = HermesToolExecutor::execute("vault_list", json!({}), &ctx).await;
    assert!(list_after.success);
    assert_no_secrets_in_result(&list_after, &forbidden);
    assert_eq!(
        list_after.structured.as_ref().unwrap()["credentials"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let fill_res = HermesToolExecutor::execute(
        "vault_fill",
        json!({
            "credential_id": cred_id,
            "username_selector": "#email",
            "password_selector": "#password",
            "totp_selector": "#totp",
            "submit_selector": "#submit"
        }),
        &ctx,
    )
    .await;
    assert!(fill_res.success, "{}", fill_res.output);
    assert_no_secrets_in_result(&fill_res, &forbidden);
    assert_eq!(
        fill_res.structured.as_ref().unwrap()["status"],
        "filled_and_submitted"
    );
    assert_eq!(fill_res.structured.as_ref().unwrap()["used_totp"], true);

    // 6. human_assist_qr
    let ctx_qr = ctx.clone();
    let qr_handle = tokio::spawn(async move {
        execute_web_media_tool(
            "human_assist_qr",
            json!({
                "qr_selector": "#qr",
                "success_url_contains": "/home",
                "auth_cookie_names": ["sid"]
            }),
            &ctx_qr,
        )
        .await
    });

    loop {
        match events_rx.recv().await.unwrap() {
            BotBrowserEvent::RefreshQrImage { poll_attempt, .. } if poll_attempt >= 1 => {
                let mut st = page.lock();
                st.url = "https://portal.example.com/home".to_string();
                st.cookies.push(json!({ "name": "sid", "value": "ok" }));
                break;
            }
            _ => continue,
        }
    }

    let qr_res = qr_handle.await.unwrap();
    assert!(qr_res.success, "{}", qr_res.output);
    assert_no_secrets_in_result(&qr_res, &forbidden);
    assert_eq!(
        qr_res.structured.as_ref().unwrap()["status"],
        "qr_login_succeeded"
    );

    // 7. human_assist_click
    let ctx_click = ctx.clone();
    let click_handle = tokio::spawn(async move {
        HermesToolExecutor::execute(
            "human_assist_click",
            json!({
                "target_selector": "#captcha",
                "instruction": "Click the bicycle",
                "max_clicks": 1
            }),
            &ctx_click,
        )
        .await
    });

    let click_card_id = loop {
        match events_rx.recv().await.unwrap() {
            BotBrowserEvent::UpsertCard(AssistCardPayload::ClickCaptcha { card_id, .. }) => {
                break card_id;
            }
            _ => continue,
        }
    };
    orch.submit_click_captcha(&click_card_id, vec![NormalizedClickPoint::new(0.5, 0.5)])
        .await
        .unwrap();

    let click_res = click_handle.await.unwrap();
    assert!(click_res.success, "{}", click_res.output);
    assert_no_secrets_in_result(&click_res, &forbidden);
    assert_eq!(click_res.structured.as_ref().unwrap()["count"], 1);

    // 8. Pure Obscura error when neither orchestrator nor endpoint is configured (zero Chrome msg)
    let bare_ctx = HermesToolContext::new(dir.path());
    let bare_res = HermesToolExecutor::execute(
        "browser_navigate",
        json!({ "url": "https://portal.example.com" }),
        &bare_ctx,
    )
    .await;
    assert!(!bare_res.success);
    assert!(!bare_res.output.to_ascii_lowercase().contains("chrome"));
}

#[tokio::test]
async fn test_fabushi_bot_host_set_bot_browser_propagates_to_core_and_provisioned_bots() {
    let dir = TempDir::new().unwrap();
    let page = Arc::new(SyncMutex::new(MockPage::new(
        "https://portal.example.com/login",
        make_png(400, 400),
    )));
    let vault = Arc::new(
        CredentialVault::open(dir.path().join("vault.db"), &MemoryMasterKeyStore::new()).unwrap(),
    );
    let orch = Arc::new(BotBrowserOrchestrator::with_transport_factory(
        OrchestratorConfig::new(dir.path()),
        vault,
        Arc::new(MockFactory(Arc::clone(&page))),
    ));

    let host = FabushiBotHost::new_unified_ephemeral("local-hermes-v1").unwrap();

    // Provision a custom bot BEFORE calling set_bot_browser
    let pre_bot = host
        .provision_or_sync_bot(FabushiBotProvisionSpec {
            bot_user_id: 800_101,
            owner_user_id: 1001,
            username: "pre_browser_bot".to_string(),
            display_name: "Pre Browser Bot".to_string(),
            description: "Tests pre-existing bot propagation".to_string(),
            title: "Agent".to_string(),
            agent_id: None,
            avatar_shape: None,
            avatar_color: None,
            preferred_model: None,
            thinking_level: None,
            approval_mode: None,
            toolset_preset: Some("full".to_string()),
            commands: Vec::new(),
            inline_mode_enabled: false,
        })
        .unwrap();

    // Wire BotBrowserOrchestrator onto FabushiBotHost
    host.set_bot_browser(Arc::clone(&orch));

    // Provision a custom bot AFTER calling set_bot_browser
    let post_bot = host
        .provision_or_sync_bot(FabushiBotProvisionSpec {
            bot_user_id: 800_102,
            owner_user_id: 1001,
            username: "post_browser_bot".to_string(),
            display_name: "Post Browser Bot".to_string(),
            description: "Tests post-wiring bot propagation".to_string(),
            title: "Agent".to_string(),
            agent_id: None,
            avatar_shape: None,
            avatar_color: None,
            preferred_model: None,
            thinking_level: None,
            approval_mode: None,
            toolset_preset: Some("full".to_string()),
            commands: Vec::new(),
            inline_mode_enabled: false,
        })
        .unwrap();

    // 1. Core engine context receives orchestrator and executes vault_list + browser_snapshot
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let core_ctx = host.core_engine().build_tool_context(
        "fabushi_core_sess",
        Some(10),
        Some("777000"),
        1,
        Some(ToolsetPreset::Full),
        0,
        Arc::clone(&cancel),
    );
    assert!(core_ctx.bot_browser.is_some());
    let core_vault = HermesToolExecutor::execute("vault_list", json!({}), &core_ctx).await;
    assert!(core_vault.success, "core vault_list failed: {}", core_vault.output);
    let core_nav = HermesToolExecutor::execute(
        "browser_navigate",
        json!({ "url": "https://portal.example.com/login" }),
        &core_ctx,
    )
    .await;
    assert!(core_nav.success, "core browser_navigate failed: {}", core_nav.output);
    let core_snap = HermesToolExecutor::execute("browser_snapshot", json!({}), &core_ctx).await;
    assert!(core_snap.success, "core browser_snapshot failed: {}", core_snap.output);

    // 2. Pre-provisioned bot instance receives orchestrator
    let pre_ctx = pre_bot.engine().build_tool_context(
        "pre_sess",
        Some(11),
        Some("800101"),
        1,
        Some(ToolsetPreset::Full),
        0,
        Arc::clone(&cancel),
    );
    assert!(pre_ctx.bot_browser.is_some());
    let pre_vault = HermesToolExecutor::execute("vault_list", json!({}), &pre_ctx).await;
    assert!(pre_vault.success, "pre_bot vault_list failed: {}", pre_vault.output);

    // 3. Post-provisioned bot instance automatically receives orchestrator on creation
    let post_ctx = post_bot.engine().build_tool_context(
        "post_sess",
        Some(12),
        Some("800102"),
        1,
        Some(ToolsetPreset::Full),
        0,
        cancel,
    );
    assert!(post_ctx.bot_browser.is_some());
    let post_snap = HermesToolExecutor::execute("browser_snapshot", json!({}), &post_ctx).await;
    assert!(post_snap.success, "post_bot browser_snapshot failed: {}", post_snap.output);
    assert_eq!(
        post_snap.structured.as_ref().unwrap()["redacted_field_count"],
        1
    );
}
