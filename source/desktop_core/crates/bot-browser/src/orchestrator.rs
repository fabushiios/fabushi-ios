use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, broadcast, oneshot};
use tokio::task::JoinHandle;
use url::Url;
use uuid::Uuid;

use crate::backend::{BrowserBackendKind, BrowserSessionBackend, CdpBrowserBackend};
use crate::cdp::{
    CdpBridge, CdpTransport, FillSubmitResult, LoginFormSelectors, ObscuraSidecarTransport,
};
use crate::script_app::{
    AutomationScriptApp, BrowserControlGate, CredentialVaultSecretResolver,
    DeterministicScriptRunner, ScriptReplayReport,
};
use crate::session_import::BrowserProfileStore;
use crate::sidecar::{
    ObscuraBinaryManifest, ObscuraDownloader, ObscuraSidecar, ProfileNamespace, SidecarConfig,
};
use crate::totp::generate_totp_now;
use crate::types::{
    AssistCardPayload, AssistCardStatus, BotBrowserError, BotBrowserEvent, BrowserSnapshot,
    CredentialMeta, LocalLoginSubmission, NormalizedClickPoint, NormalizedOrigin, ReusePolicy,
    TaskWakeup,
};
use crate::vault::CredentialVault;

const TIMED_OUT_RESUMABLE_NOTE: &str = "上轮已超时，补填后将自动保存并续跑";
const REUSE_TIMED_OUT_NOTE: &str = "上轮已超时，确认后将自动登录并续跑";

/// Configuration governing `BotBrowserOrchestrator` timeouts and storage paths.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub data_dir: PathBuf,
    pub card_wait_timeout: Duration,
    pub qr_refresh_interval: Duration,
    pub qr_max_wait: Duration,
    pub click_wait_timeout: Duration,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(".bot-browser"),
            card_wait_timeout: Duration::from_secs(300),
            qr_refresh_interval: Duration::from_secs(3),
            qr_max_wait: Duration::from_secs(180),
            click_wait_timeout: Duration::from_secs(180),
        }
    }
}

impl OrchestratorConfig {
    /// Creates an `OrchestratorConfig` rooted at `data_dir` with production default timeouts.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            ..Self::default()
        }
    }
}

/// Pluggable factory creating per-session CDP transports (Obscura sidecar in production, mock in tests).
#[async_trait]
pub trait SessionTransportFactory: Send + Sync {
    async fn create_transport(
        &self,
        session_id: &str,
        data_dir: &Path,
        origin_hint: Option<&NormalizedOrigin>,
        event_tx: &broadcast::Sender<BotBrowserEvent>,
    ) -> Result<Arc<dyn CdpTransport>, BotBrowserError>;

    async fn close_transport(
        &self,
        _session_id: &str,
        _transport: Arc<dyn CdpTransport>,
    ) -> Result<(), BotBrowserError> {
        Ok(())
    }
}

/// Production `SessionTransportFactory` launching isolated pure-Obscura sidecar processes.
pub struct ObscuraSessionFactory {
    manifest: ObscuraBinaryManifest,
    http_client: reqwest::Client,
    command_timeout: Duration,
    startup_timeout: Duration,
    heartbeat_interval: Duration,
    max_auto_restarts: u32,
    sidecars: Mutex<HashMap<String, (Arc<ObscuraSidecar>, JoinHandle<()>)>>,
}

impl Default for ObscuraSessionFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ObscuraSessionFactory {
    pub fn new() -> Self {
        let http_client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            manifest: ObscuraBinaryManifest::default_obscura(),
            http_client,
            command_timeout: Duration::from_secs(15),
            startup_timeout: Duration::from_secs(10),
            heartbeat_interval: Duration::from_secs(5),
            max_auto_restarts: 5,
            sidecars: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_manifest(mut self, manifest: ObscuraBinaryManifest) -> Self {
        self.manifest = manifest;
        self
    }

    pub fn with_timeouts(
        mut self,
        command_timeout: Duration,
        startup_timeout: Duration,
        heartbeat_interval: Duration,
    ) -> Self {
        self.command_timeout = command_timeout;
        self.startup_timeout = startup_timeout;
        self.heartbeat_interval = heartbeat_interval;
        self
    }
}

#[async_trait]
impl SessionTransportFactory for ObscuraSessionFactory {
    async fn create_transport(
        &self,
        session_id: &str,
        data_dir: &Path,
        origin_hint: Option<&NormalizedOrigin>,
        event_tx: &broadcast::Sender<BotBrowserEvent>,
    ) -> Result<Arc<dyn CdpTransport>, BotBrowserError> {
        let binary_path =
            ObscuraDownloader::ensure_binary(data_dir, &self.manifest, &self.http_client).await?;
        let storage_dir = ProfileNamespace::storage_dir_for(data_dir, session_id, origin_hint);
        let sidecar_cfg = SidecarConfig::new(binary_path, storage_dir)
            .with_command_timeout(self.command_timeout)
            .with_startup_timeout(self.startup_timeout)
            .with_heartbeat_interval(self.heartbeat_interval)
            .with_max_auto_restarts(self.max_auto_restarts);

        let sidecar = Arc::new(
            ObscuraSidecar::start(sidecar_cfg, session_id, Some(event_tx.clone())).await?,
        );
        let heartbeat = sidecar.spawn_heartbeat();

        let prior = {
            let mut guard = self.sidecars.lock().await;
            guard.insert(session_id.to_string(), (Arc::clone(&sidecar), heartbeat))
        };
        if let Some((old_sidecar, old_hb)) = prior {
            old_hb.abort();
            let _ = old_sidecar.shutdown().await;
        }

        Ok(Arc::new(ObscuraSidecarTransport::new(sidecar)))
    }

    async fn close_transport(
        &self,
        session_id: &str,
        _transport: Arc<dyn CdpTransport>,
    ) -> Result<(), BotBrowserError> {
        let removed = {
            let mut guard = self.sidecars.lock().await;
            guard.remove(session_id)
        };
        if let Some((sidecar, hb)) = removed {
            hb.abort();
            sidecar.shutdown().await?;
        }
        Ok(())
    }
}

/// AI-safe navigation outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserNavigateResult {
    pub session_id: String,
    pub url: String,
    pub origin: String,
}

/// AI-safe click outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserClickResult {
    pub session_id: String,
    pub clicked_selector: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
}

/// AI-safe non-secret typing outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTypeResult {
    pub session_id: String,
    pub chars_typed: usize,
    pub submitted: bool,
}

/// AI-safe credential metadata listing for the active page origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultListResult {
    pub origin: String,
    pub credentials: Vec<CredentialMeta>,
}

/// AI-safe outcome from `tool_vault_request_card` (never contains plaintext secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultRequestCardResult {
    pub status: String,
    pub card_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_hint: Option<String>,
    pub origin: String,
}

/// Local UI submission outcome returned to GPUI after saving credentials (and optionally resuming).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginSubmissionOutcome {
    pub card_id: String,
    pub credential_id: String,
    pub account_hint: String,
    pub origin: String,
    pub resumed_timed_out_task: bool,
    pub auto_filled: bool,
    pub used_totp: bool,
}

/// AI-safe outcome from `tool_vault_fill`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultFillResult {
    pub status: String,
    pub origin: String,
    pub credential_id: String,
    pub used_totp: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
}

/// Local UI approval outcome returned to GPUI for `ReuseConfirm` cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReuseApprovalOutcome {
    pub card_id: String,
    pub credential_id: String,
    pub origin: String,
    pub approved: bool,
    pub resumed_timed_out_task: bool,
    pub auto_filled: bool,
    pub used_totp: bool,
}

/// AI-safe outcome from `tool_human_assist_qr`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrAssistResult {
    pub status: String,
    pub card_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirected_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_cookie: Option<String>,
    pub poll_attempts: u32,
}

/// AI-safe outcome from `tool_human_assist_click`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClickAssistResult {
    pub status: String,
    pub card_id: String,
    pub count: usize,
    pub dispatched_points: Vec<(f64, f64)>,
}

struct SessionState {
    bridge: Arc<CdpBridge>,
    backend: Arc<CdpBrowserBackend>,
}

#[derive(Clone)]
struct LoginWaitResolution {
    credential_id: String,
    account_hint: String,
}

enum CardWaiter {
    Login(oneshot::Sender<Result<LoginWaitResolution, BotBrowserError>>),
    Reuse(oneshot::Sender<Result<bool, BotBrowserError>>),
    Click(oneshot::Sender<Result<Vec<NormalizedClickPoint>, BotBrowserError>>),
}

enum CardResumeContext {
    Login {
        selectors: LoginFormSelectors,
    },
    Reuse {
        selectors: LoginFormSelectors,
    },
    Qr {
        cancel_notify: Arc<Notify>,
    },
    Click,
}

struct ActiveCardEntry {
    payload: AssistCardPayload,
    waiter: Option<CardWaiter>,
    context: CardResumeContext,
}

/// Coordinates pure-Obscura browser sessions, zero-knowledge Vault operations,
/// decoupled human-assist cards, 3s QR polling, and automatic late-submit wakeups.
pub struct BotBrowserOrchestrator {
    config: OrchestratorConfig,
    vault: Arc<CredentialVault>,
    transport_factory: Arc<dyn SessionTransportFactory>,
    sessions: Mutex<HashMap<String, SessionState>>,
    cards: RwLock<HashMap<String, ActiveCardEntry>>,
    event_tx: broadcast::Sender<BotBrowserEvent>,
    wakeup_tx: broadcast::Sender<TaskWakeup>,
}

impl BotBrowserOrchestrator {
    /// Creates an orchestrator backed by the production `ObscuraSessionFactory`.
    pub fn new(config: OrchestratorConfig, vault: Arc<CredentialVault>) -> Self {
        Self::with_transport_factory(config, vault, Arc::new(ObscuraSessionFactory::new()))
    }

    /// Creates an orchestrator with a custom `SessionTransportFactory` (e.g. mock CDP in tests).
    pub fn with_transport_factory(
        config: OrchestratorConfig,
        vault: Arc<CredentialVault>,
        transport_factory: Arc<dyn SessionTransportFactory>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        let (wakeup_tx, _) = broadcast::channel(128);
        Self {
            config,
            vault,
            transport_factory,
            sessions: Mutex::new(HashMap::new()),
            cards: RwLock::new(HashMap::new()),
            event_tx,
            wakeup_tx,
        }
    }

    pub fn config(&self) -> &OrchestratorConfig {
        &self.config
    }

    pub fn vault(&self) -> &Arc<CredentialVault> {
        &self.vault
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<BotBrowserEvent> {
        self.event_tx.subscribe()
    }

    pub fn subscribe_wakeups(&self) -> broadcast::Receiver<TaskWakeup> {
        self.wakeup_tx.subscribe()
    }

    pub fn get_card(&self, card_id: &str) -> Option<AssistCardPayload> {
        self.cards
            .read()
            .get(card_id)
            .map(|entry| entry.payload.clone())
    }

    /// Returns all active or historical assist cards, optionally filtered by `chat_id`.
    pub fn list_cards_for_chat(&self, chat_id: Option<i64>) -> Vec<AssistCardPayload> {
        let guard = self.cards.read();
        let mut out: Vec<AssistCardPayload> = guard
            .values()
            .filter(|entry| match chat_id {
                Some(cid) => entry.payload.chat_id() == Some(cid),
                None => true,
            })
            .map(|entry| entry.payload.clone())
            .collect();
        out.sort_by_key(card_created_at_ms);
        out
    }

    /// Ensures a live `CdpBridge` exists for `session_id`, spawning/connecting via the factory if absent.
    pub async fn ensure_session_bridge(
        &self,
        session_id: &str,
        origin_hint: Option<&NormalizedOrigin>,
    ) -> Result<Arc<CdpBridge>, BotBrowserError> {
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(session_id) {
            return Ok(Arc::clone(&existing.bridge));
        }

        let transport = self
            .transport_factory
            .create_transport(session_id, &self.config.data_dir, origin_hint, &self.event_tx)
            .await?;
        let bridge = Arc::new(CdpBridge::new(transport));
        let backend = Arc::new(CdpBrowserBackend::from_bridge(
            BrowserBackendKind::ObscuraCdp,
            Arc::clone(&bridge),
        ));
        sessions.insert(
            session_id.to_string(),
            SessionState {
                bridge: Arc::clone(&bridge),
                backend,
            },
        );
        Ok(bridge)
    }

    /// Ensures a live `CdpBrowserBackend` exists for `session_id`, spawning/connecting via the factory if absent.
    pub async fn ensure_session_backend(
        &self,
        session_id: &str,
        origin_hint: Option<&NormalizedOrigin>,
    ) -> Result<Arc<CdpBrowserBackend>, BotBrowserError> {
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(session_id) {
            return Ok(Arc::clone(&existing.backend));
        }

        let transport = self
            .transport_factory
            .create_transport(session_id, &self.config.data_dir, origin_hint, &self.event_tx)
            .await?;
        let bridge = Arc::new(CdpBridge::new(transport));
        let backend = Arc::new(CdpBrowserBackend::from_bridge(
            BrowserBackendKind::ObscuraCdp,
            Arc::clone(&bridge),
        ));
        sessions.insert(
            session_id.to_string(),
            SessionState {
                bridge,
                backend: Arc::clone(&backend),
            },
        );
        Ok(backend)
    }

    pub fn profile_store(&self) -> BrowserProfileStore {
        BrowserProfileStore::new(&self.config.data_dir)
    }

    pub async fn persist_session_cookies(
        &self,
        session_id: &str,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> Result<usize, BotBrowserError> {
        let backend = self.ensure_session_backend(session_id, origin).await?;
        let domain_filter = origin.map(|o| o.host());
        let cookies = backend.export_cookies(domain_filter).await?;
        let count = cookies.len();
        self.profile_store()
            .save_cookies(namespace, origin, &cookies)?;
        Ok(count)
    }

    pub async fn restore_session_cookies(
        &self,
        session_id: &str,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> Result<usize, BotBrowserError> {
        let cookies = self.profile_store().load_cookies(namespace, origin)?;
        if cookies.is_empty() {
            return Ok(0);
        }
        let backend = self.ensure_session_backend(session_id, origin).await?;
        backend.import_cookies(&cookies).await
    }

    pub async fn run_script_app(
        &self,
        session_id: &str,
        app: &AutomationScriptApp,
        params: &BTreeMap<String, String>,
        control: Option<&BrowserControlGate>,
    ) -> Result<ScriptReplayReport, BotBrowserError> {
        let origin_hint = app
            .target_domains
            .first()
            .and_then(|d| NormalizedOrigin::parse(&format!("https://{d}")).ok());
        let backend = self
            .ensure_session_backend(session_id, origin_hint.as_ref())
            .await?;
        let resolver = CredentialVaultSecretResolver::new(Arc::clone(&self.vault));
        DeterministicScriptRunner::run(
            backend.as_ref(),
            app,
            params,
            Some(&resolver),
            control,
        )
        .await
    }

    /// Closes and removes a browser session, invoking `close_transport` on the factory.
    pub async fn close_session(&self, session_id: &str) -> Result<(), BotBrowserError> {
        let removed = {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(session_id)
        };
        if let Some(state) = removed {
            self.transport_factory
                .close_transport(session_id, Arc::clone(state.bridge.transport()))
                .await?;
        }
        Ok(())
    }

    /// Navigates the session's Obscura tab to an `http://` or `https://` URL.
    pub async fn browser_navigate(
        &self,
        session_id: &str,
        url: &str,
    ) -> Result<BrowserNavigateResult, BotBrowserError> {
        let origin = validate_navigation_url(url)?;
        let bridge = self.ensure_session_bridge(session_id, Some(&origin)).await?;
        bridge.navigate(url).await?;
        Ok(BrowserNavigateResult {
            session_id: session_id.to_string(),
            url: url.to_string(),
            origin: origin.as_str().to_string(),
        })
    }

    /// Clicks an element by CSS selector or page coordinates `(x, y)`.
    pub async fn browser_click(
        &self,
        session_id: &str,
        selector: Option<&str>,
        x: Option<f64>,
        y: Option<f64>,
    ) -> Result<BrowserClickResult, BotBrowserError> {
        let bridge = self.ensure_session_bridge(session_id, None).await?;
        if let Some(sel) = selector.map(str::trim).filter(|s| !s.is_empty()) {
            bridge.click_selector(sel).await?;
            return Ok(BrowserClickResult {
                session_id: session_id.to_string(),
                clicked_selector: Some(sel.to_string()),
                x: None,
                y: None,
            });
        }
        if let (Some(px), Some(py)) = (x, y) {
            bridge.click_coords(px, py).await?;
            return Ok(BrowserClickResult {
                session_id: session_id.to_string(),
                clicked_selector: None,
                x: Some(px),
                y: Some(py),
            });
        }
        Err(BotBrowserError::Cdp(
            "Provide either 'selector' or ('x', 'y')".to_string(),
        ))
    }

    /// Types non-secret text into a focused or selected element; blocks password/OTP inputs and atomic secret fill.
    pub async fn browser_type(
        &self,
        session_id: &str,
        selector: Option<&str>,
        text: &str,
        submit: bool,
    ) -> Result<BrowserTypeResult, BotBrowserError> {
        let bridge = self.ensure_session_bridge(session_id, None).await?;
        if bridge.is_atomic_fill_active() {
            return Err(BotBrowserError::SessionLockedForSecretFill);
        }

        let cleaned_selector = selector.map(str::trim).filter(|s| !s.is_empty());
        if let Some(sel) = cleaned_selector {
            if is_sensitive_selector_hint(sel) {
                return Err(BotBrowserError::Cdp(
                    "Refusing to type into password/OTP selector via browser_type; use vault_request_card or vault_fill so credentials stay zero-knowledge."
                        .to_string(),
                ));
            }
        }

        probe_and_focus_non_sensitive(&bridge, cleaned_selector).await?;
        bridge.insert_text(text).await?;

        if submit {
            if bridge.is_atomic_fill_active() {
                return Err(BotBrowserError::SessionLockedForSecretFill);
            }
            dispatch_enter_on_bridge(&bridge).await?;
        }

        Ok(BrowserTypeResult {
            session_id: session_id.to_string(),
            chars_typed: text.chars().count(),
            submitted: submit,
        })
    }

    /// Captures a PNG screenshot of the session page (rejected during `AtomicFillGuard`).
    pub async fn browser_screenshot(
        &self,
        session_id: &str,
        full_page: bool,
    ) -> Result<Vec<u8>, BotBrowserError> {
        let bridge = self.ensure_session_bridge(session_id, None).await?;
        bridge.capture_screenshot_png(full_page).await
    }

    /// Captures a redacted DOM snapshot (all password/OTP fields forced to `"[REDACTED]"`).
    pub async fn tool_browser_snapshot(
        &self,
        session_id: &str,
        selector: Option<&str>,
    ) -> Result<BrowserSnapshot, BotBrowserError> {
        let bridge = self.ensure_session_bridge(session_id, None).await?;
        bridge.capture_redacted_snapshot(selector).await
    }

    /// Lists non-secret `CredentialMeta` records bound to the active page's true `origin`.
    pub async fn tool_vault_list(
        &self,
        session_id: &str,
    ) -> Result<VaultListResult, BotBrowserError> {
        let bridge = self.ensure_session_bridge(session_id, None).await?;
        let (origin, _, _) = bridge.inspect_true_origin(None).await?;
        let credentials = self.vault.list_for_origin(origin.as_str())?;
        Ok(VaultListResult {
            origin: origin.as_str().to_string(),
            credentials,
        })
    }

    /// Emits a local-only GPUI `VaultLogin` assist card and awaits user submission up to `card_wait_timeout`.
    /// On timeout, drops ONLY the waiter while leaving the card `TimedOutResumable` for late-fill wakeup.
    pub async fn tool_vault_request_card(
        &self,
        session_id: &str,
        task_id: &str,
        chat_id: Option<i64>,
        site_title: &str,
        selectors: LoginFormSelectors,
    ) -> Result<VaultRequestCardResult, BotBrowserError> {
        let bridge = self.ensure_session_bridge(session_id, None).await?;
        let (origin, frame_origin, target_url) = bridge
            .inspect_true_origin(Some(&selectors.password_selector))
            .await?;
        enforce_same_origin_frame(&origin, frame_origin.as_ref())?;

        let card_id = format!("card_{}", Uuid::new_v4().simple());
        let is_high_sensitivity = origin.is_high_sensitivity();
        let suggested_reuse_policy = ReusePolicy::default_for_origin(&origin);
        let created_at_ms = Utc::now().timestamp_millis();

        let payload = AssistCardPayload::VaultLogin {
            card_id: card_id.clone(),
            task_id: task_id.to_string(),
            session_id: session_id.to_string(),
            chat_id,
            origin: origin.as_str().to_string(),
            target_url,
            site_title: site_title.to_string(),
            suggested_reuse_policy,
            is_high_sensitivity,
            status: AssistCardStatus::AwaitingUser,
            status_note: None,
            saved_credential_id: None,
            saved_account_hint: None,
            created_at_ms,
        };

        let (tx, rx) = oneshot::channel();
        {
            let mut cards = self.cards.write();
            cards.insert(
                card_id.clone(),
                ActiveCardEntry {
                    payload: payload.clone(),
                    waiter: Some(CardWaiter::Login(tx)),
                    context: CardResumeContext::Login { selectors },
                },
            );
        }
        let _ = self.event_tx.send(BotBrowserEvent::UpsertCard(payload));

        match tokio::time::timeout(self.config.card_wait_timeout, rx).await {
            Ok(Ok(Ok(resolution))) => Ok(VaultRequestCardResult {
                status: "saved".to_string(),
                card_id,
                credential_id: Some(resolution.credential_id),
                account_hint: Some(resolution.account_hint),
                origin: origin.as_str().to_string(),
            }),
            Ok(Ok(Err(err))) => Err(err),
            Ok(Err(_closed)) => Err(BotBrowserError::ApprovalDenied(
                "login assist card waiter closed".to_string(),
            )),
            Err(_elapsed) => {
                self.transition_card_timed_out_resumable(
                    &card_id,
                    chat_id,
                    TIMED_OUT_RESUMABLE_NOTE,
                );
                Ok(VaultRequestCardResult {
                    status: "timed_out_card_still_active".to_string(),
                    card_id,
                    credential_id: None,
                    account_hint: None,
                    origin: origin.as_str().to_string(),
                })
            }
        }
    }

    /// Local-only GPUI callback saving user-entered credentials into the encrypted Vault.
    /// If the originating AI turn already timed out (`TimedOutResumable`), automatically ensures
    /// the browser session is live, navigates back to `target_url`, fills atomically, and fires `TaskWakeupTriggered`.
    pub async fn submit_login_card(
        &self,
        submission: LocalLoginSubmission,
    ) -> Result<LoginSubmissionOutcome, BotBrowserError> {
        let card_id = submission.card_id.clone();
        let (session_id, task_id, chat_id, origin, target_url, selectors) = {
            let cards = self.cards.read();
            let entry = cards
                .get(&card_id)
                .ok_or_else(|| BotBrowserError::CardNotFound(card_id.clone()))?;
            let (session_id, task_id, chat_id, origin, target_url, status) = match &entry.payload {
                AssistCardPayload::VaultLogin {
                    session_id,
                    task_id,
                    chat_id,
                    origin,
                    target_url,
                    status,
                    ..
                } => (
                    session_id.clone(),
                    task_id.clone(),
                    *chat_id,
                    origin.clone(),
                    target_url.clone(),
                    *status,
                ),
                _ => {
                    return Err(BotBrowserError::Cdp(format!(
                        "assist card '{card_id}' is not a VaultLogin card"
                    )));
                }
            };
            if !matches!(
                status,
                AssistCardStatus::AwaitingUser | AssistCardStatus::TimedOutResumable
            ) {
                return Err(BotBrowserError::Cdp(format!(
                    "assist card '{card_id}' cannot be submitted in status {status:?}"
                )));
            }
            let selectors = match &entry.context {
                CardResumeContext::Login { selectors } => selectors.clone(),
                _ => {
                    return Err(BotBrowserError::Cdp(format!(
                        "assist card '{card_id}' missing login selectors context"
                    )));
                }
            };
            (session_id, task_id, chat_id, origin, target_url, selectors)
        };

        if submission.password.is_empty() {
            return Err(BotBrowserError::Crypto(
                "password cannot be empty".to_string(),
            ));
        }

        let reuse_policy = if submission.allow_auto_reuse {
            ReusePolicy::AutoReuse
        } else {
            ReusePolicy::ConfirmEveryTime
        };

        let meta = self.vault.store_credential(
            &origin,
            submission.username.as_str(),
            submission.password.as_str(),
            submission.totp_secret.as_deref().map(|s| s.as_str()),
            submission.account_hint.as_deref(),
            Some(reuse_policy),
        )?;
        // Drop Zeroizing submission immediately after Vault encryption.
        drop(submission);

        let active_waiter = {
            let mut cards = self.cards.write();
            let entry = cards
                .get_mut(&card_id)
                .ok_or_else(|| BotBrowserError::CardNotFound(card_id.clone()))?;
            let waiter = entry.waiter.take();
            if waiter.is_some() {
                set_vault_login_saved(
                    &mut entry.payload,
                    AssistCardStatus::SavedAndLoggedIn,
                    None,
                    &meta.credential_id,
                    &meta.account_hint,
                );
            } else {
                set_vault_login_saved(
                    &mut entry.payload,
                    AssistCardStatus::ApprovedAndFilling,
                    Some("正在补填并自动登录…".to_string()),
                    &meta.credential_id,
                    &meta.account_hint,
                );
            }
            waiter
        };

        if let Some(CardWaiter::Login(tx)) = active_waiter {
            self.broadcast_card_status(
                &card_id,
                chat_id,
                AssistCardStatus::SavedAndLoggedIn,
                None,
                Some(meta.credential_id.clone()),
                Some(meta.account_hint.clone()),
                None,
            );
            let _ = tx.send(Ok(LoginWaitResolution {
                credential_id: meta.credential_id.clone(),
                account_hint: meta.account_hint.clone(),
            }));
            return Ok(LoginSubmissionOutcome {
                card_id,
                credential_id: meta.credential_id,
                account_hint: meta.account_hint,
                origin,
                resumed_timed_out_task: false,
                auto_filled: false,
                used_totp: false,
            });
        }

        // Decoupled late-fill auto-wakeup path (original waiter already timed out).
        self.broadcast_card_status(
            &card_id,
            chat_id,
            AssistCardStatus::ApprovedAndFilling,
            Some("正在补填并自动登录…".to_string()),
            Some(meta.credential_id.clone()),
            Some(meta.account_hint.clone()),
            None,
        );

        let parsed_origin = NormalizedOrigin::parse(&origin)?;
        let bridge = self
            .ensure_session_bridge(&session_id, Some(&parsed_origin))
            .await?;

        let fill_attempt: Result<FillSubmitResult, BotBrowserError> = async {
            bridge.navigate(&target_url).await?;
            self.execute_vault_fill_internal(&bridge, &meta.credential_id, &selectors)
                .await
        }
        .await;

        let fill_res = match fill_attempt {
            Ok(res) => res,
            Err(err) => {
                let err_note = format!("凭证已安全保存，但自动登录失败: {err}");
                {
                    let mut cards = self.cards.write();
                    if let Some(entry) = cards.get_mut(&card_id) {
                        set_vault_login_saved(
                            &mut entry.payload,
                            AssistCardStatus::TimedOutResumable,
                            Some(err_note.clone()),
                            &meta.credential_id,
                            &meta.account_hint,
                        );
                    }
                }
                self.broadcast_card_status(
                    &card_id,
                    chat_id,
                    AssistCardStatus::TimedOutResumable,
                    Some(err_note),
                    Some(meta.credential_id.clone()),
                    Some(meta.account_hint.clone()),
                    None,
                );
                return Err(err);
            }
        };

        {
            let mut cards = self.cards.write();
            if let Some(entry) = cards.get_mut(&card_id) {
                set_vault_login_saved(
                    &mut entry.payload,
                    AssistCardStatus::SavedAndLoggedIn,
                    None,
                    &meta.credential_id,
                    &meta.account_hint,
                );
            }
        }
        self.broadcast_card_status(
            &card_id,
            chat_id,
            AssistCardStatus::SavedAndLoggedIn,
            None,
            Some(meta.credential_id.clone()),
            Some(meta.account_hint.clone()),
            None,
        );

        let wakeup = TaskWakeup {
            task_id,
            session_id,
            chat_id,
            target_url,
            origin: origin.clone(),
            credential_id: meta.credential_id.clone(),
            triggered_at_ms: Utc::now().timestamp_millis(),
        };
        let _ = self.wakeup_tx.send(wakeup.clone());
        let _ = self
            .event_tx
            .send(BotBrowserEvent::TaskWakeupTriggered(wakeup));

        Ok(LoginSubmissionOutcome {
            card_id,
            credential_id: meta.credential_id,
            account_hint: meta.account_hint,
            origin,
            resumed_timed_out_task: true,
            auto_filled: true,
            used_totp: fill_res.used_totp,
        })
    }

    /// Atomically decrypts and fills Vault credentials (+ TOTP) after strict origin checks and
    /// `ReusePolicy` enforcement. Never exposes plaintext credentials in its return value.
    pub async fn tool_vault_fill(
        &self,
        session_id: &str,
        task_id: &str,
        chat_id: Option<i64>,
        credential_id: &str,
        selectors: LoginFormSelectors,
    ) -> Result<VaultFillResult, BotBrowserError> {
        let meta = self.vault.get_meta(credential_id)?;
        let bridge = self.ensure_session_bridge(session_id, None).await?;

        // Fail fast on cross-origin prompt injection before emitting any confirmation UI.
        let (page_origin, frame_origin, target_url) = bridge
            .inspect_true_origin(Some(&selectors.password_selector))
            .await?;
        verify_origin_against_meta(&meta.origin, &page_origin, frame_origin.as_ref())?;

        let mut reuse_card_id: Option<String> = None;
        if meta.reuse_policy == ReusePolicy::ConfirmEveryTime {
            let card_id = format!("card_{}", Uuid::new_v4().simple());
            reuse_card_id = Some(card_id.clone());
            let created_at_ms = Utc::now().timestamp_millis();

            let payload = AssistCardPayload::ReuseConfirm {
                card_id: card_id.clone(),
                task_id: task_id.to_string(),
                session_id: session_id.to_string(),
                chat_id,
                credential_id: meta.credential_id.clone(),
                origin: meta.origin.clone(),
                target_url,
                account_hint: meta.account_hint.clone(),
                has_totp: meta.has_totp,
                require_biometric_hint: true,
                status: AssistCardStatus::AwaitingUser,
                status_note: None,
                created_at_ms,
            };

            let (tx, rx) = oneshot::channel();
            {
                let mut cards = self.cards.write();
                cards.insert(
                    card_id.clone(),
                    ActiveCardEntry {
                        payload: payload.clone(),
                        waiter: Some(CardWaiter::Reuse(tx)),
                        context: CardResumeContext::Reuse {
                            selectors: selectors.clone(),
                        },
                    },
                );
            }
            let _ = self.event_tx.send(BotBrowserEvent::UpsertCard(payload));

            match tokio::time::timeout(self.config.card_wait_timeout, rx).await {
                Ok(Ok(Ok(true))) => {}
                Ok(Ok(Ok(false))) => {
                    return Err(BotBrowserError::ApprovalDenied(format!(
                        "user denied credential reuse on {}",
                        meta.origin
                    )));
                }
                Ok(Ok(Err(err))) => return Err(err),
                Ok(Err(_closed)) => {
                    return Err(BotBrowserError::ApprovalDenied(
                        "reuse confirmation waiter closed".to_string(),
                    ));
                }
                Err(_elapsed) => {
                    self.transition_card_timed_out_resumable(
                        &card_id,
                        chat_id,
                        REUSE_TIMED_OUT_NOTE,
                    );
                    return Ok(VaultFillResult {
                        status: "approval_timed_out_card_still_active".to_string(),
                        origin: meta.origin,
                        credential_id: meta.credential_id,
                        used_totp: false,
                        card_id: Some(card_id),
                    });
                }
            }
        }

        let fill_res = self
            .execute_vault_fill_internal(&bridge, credential_id, &selectors)
            .await?;

        if let Some(ref cid) = reuse_card_id {
            self.update_card_status(cid, chat_id, AssistCardStatus::Completed, None, None);
        }

        Ok(VaultFillResult {
            status: "filled_and_submitted".to_string(),
            origin: fill_res.origin,
            credential_id: credential_id.to_string(),
            used_totp: fill_res.used_totp,
            card_id: reuse_card_id,
        })
    }

    /// Local-only GPUI callback approving or denying a `ReuseConfirm` card.
    /// Supports both active waiter wakeup and `TimedOutResumable` late-approval auto-fill wakeup.
    pub async fn approve_reuse_card(
        &self,
        card_id: &str,
        approved: bool,
    ) -> Result<ReuseApprovalOutcome, BotBrowserError> {
        let (
            session_id,
            task_id,
            chat_id,
            credential_id,
            origin,
            target_url,
            selectors,
            waiter,
        ) = {
            let mut cards = self.cards.write();
            let entry = cards
                .get_mut(card_id)
                .ok_or_else(|| BotBrowserError::CardNotFound(card_id.to_string()))?;

            let (session_id, task_id, chat_id, credential_id, origin, target_url, status) =
                match &entry.payload {
                    AssistCardPayload::ReuseConfirm {
                        session_id,
                        task_id,
                        chat_id,
                        credential_id,
                        origin,
                        target_url,
                        status,
                        ..
                    } => (
                        session_id.clone(),
                        task_id.clone(),
                        *chat_id,
                        credential_id.clone(),
                        origin.clone(),
                        target_url.clone(),
                        *status,
                    ),
                    _ => {
                        return Err(BotBrowserError::Cdp(format!(
                            "assist card '{card_id}' is not a ReuseConfirm card"
                        )));
                    }
                };

            if !matches!(
                status,
                AssistCardStatus::AwaitingUser | AssistCardStatus::TimedOutResumable
            ) {
                return Err(BotBrowserError::Cdp(format!(
                    "assist card '{card_id}' cannot be decided in status {status:?}"
                )));
            }

            let selectors = match &entry.context {
                CardResumeContext::Reuse { selectors, .. } => selectors.clone(),
                _ => {
                    return Err(BotBrowserError::Cdp(format!(
                        "assist card '{card_id}' missing reuse selectors context"
                    )));
                }
            };

            let waiter = entry.waiter.take();
            if approved {
                entry
                    .payload
                    .set_status(AssistCardStatus::ApprovedAndFilling, None);
            } else {
                entry.payload.set_status(
                    AssistCardStatus::Denied,
                    Some("用户已拒绝本次登录".to_string()),
                );
            }

            (
                session_id,
                task_id,
                chat_id,
                credential_id,
                origin,
                target_url,
                selectors,
                waiter,
            )
        };

        if !approved {
            self.broadcast_card_status(
                card_id,
                chat_id,
                AssistCardStatus::Denied,
                Some("用户已拒绝本次登录".to_string()),
                None,
                None,
                None,
            );
            if let Some(CardWaiter::Reuse(tx)) = waiter {
                let _ = tx.send(Ok(false));
            }
            return Ok(ReuseApprovalOutcome {
                card_id: card_id.to_string(),
                credential_id,
                origin,
                approved: false,
                resumed_timed_out_task: false,
                auto_filled: false,
                used_totp: false,
            });
        }

        self.broadcast_card_status(
            card_id,
            chat_id,
            AssistCardStatus::ApprovedAndFilling,
            None,
            None,
            None,
            None,
        );

        if let Some(CardWaiter::Reuse(tx)) = waiter {
            let _ = tx.send(Ok(true));
            return Ok(ReuseApprovalOutcome {
                card_id: card_id.to_string(),
                credential_id,
                origin,
                approved: true,
                resumed_timed_out_task: false,
                auto_filled: false,
                used_totp: false,
            });
        }

        // Timed-out late approval wakeup path.
        let parsed_origin = NormalizedOrigin::parse(&origin)?;
        let bridge = self
            .ensure_session_bridge(&session_id, Some(&parsed_origin))
            .await?;
        let fill_attempt: Result<FillSubmitResult, BotBrowserError> = async {
            bridge.navigate(&target_url).await?;
            self.execute_vault_fill_internal(&bridge, &credential_id, &selectors)
                .await
        }
        .await;

        let fill_res = match fill_attempt {
            Ok(res) => res,
            Err(err) => {
                self.update_card_status(
                    card_id,
                    chat_id,
                    AssistCardStatus::TimedOutResumable,
                    Some(format!("自动登录失败: {err}")),
                    None,
                );
                return Err(err);
            }
        };

        self.update_card_status(card_id, chat_id, AssistCardStatus::Completed, None, None);

        let wakeup = TaskWakeup {
            task_id,
            session_id,
            chat_id,
            target_url,
            origin: origin.clone(),
            credential_id: credential_id.clone(),
            triggered_at_ms: Utc::now().timestamp_millis(),
        };
        let _ = self.wakeup_tx.send(wakeup.clone());
        let _ = self
            .event_tx
            .send(BotBrowserEvent::TaskWakeupTriggered(wakeup));

        Ok(ReuseApprovalOutcome {
            card_id: card_id.to_string(),
            credential_id,
            origin,
            approved: true,
            resumed_timed_out_task: true,
            auto_filled: true,
            used_totp: fill_res.used_totp,
        })
    }

    /// Crops a login QR code element, emits a GPUI `QrLogin` card, silently refreshes it at
    /// `qr_refresh_interval` (default 3s), and resolves immediately when URL redirects or auth cookies appear.
    pub async fn tool_human_assist_qr(
        &self,
        session_id: &str,
        task_id: &str,
        chat_id: Option<i64>,
        qr_selector: &str,
        success_url_contains: Option<&str>,
        auth_cookie_names: Vec<String>,
    ) -> Result<QrAssistResult, BotBrowserError> {
        let bridge = self.ensure_session_bridge(session_id, None).await?;
        let (origin, _, initial_url) = bridge.inspect_true_origin(None).await?;
        let initial_cookies = extract_cookie_map(&bridge.get_cookies().await.unwrap_or_default());
        let (initial_png, _) = bridge.capture_element_png(qr_selector).await?;

        let refresh_interval = self.config.qr_refresh_interval.max(Duration::from_millis(5));
        let max_wait = self.config.qr_max_wait.max(refresh_interval);
        let total_secs = max_wait.as_secs().max(1) as u32;
        let refresh_secs = refresh_interval.as_secs().max(1) as u32;

        let card_id = format!("card_{}", Uuid::new_v4().simple());
        let cancel_notify = Arc::new(Notify::new());
        let created_at_ms = Utc::now().timestamp_millis();

        let payload = AssistCardPayload::QrLogin {
            card_id: card_id.clone(),
            task_id: task_id.to_string(),
            session_id: session_id.to_string(),
            chat_id,
            origin: origin.as_str().to_string(),
            target_url: initial_url.clone(),
            png_bytes: initial_png,
            countdown_secs: total_secs,
            refresh_interval_secs: refresh_secs,
            poll_attempt: 0,
            status: AssistCardStatus::AwaitingUser,
            status_note: None,
            redirected_url: None,
            created_at_ms,
        };

        {
            let mut cards = self.cards.write();
            cards.insert(
                card_id.clone(),
                ActiveCardEntry {
                    payload: payload.clone(),
                    waiter: None,
                    context: CardResumeContext::Qr {
                        cancel_notify: Arc::clone(&cancel_notify),
                    },
                },
            );
        }
        let _ = self.event_tx.send(BotBrowserEvent::UpsertCard(payload));

        let started = tokio::time::Instant::now();
        let mut poll_attempt = 0u32;

        loop {
            tokio::select! {
                _ = tokio::time::sleep(refresh_interval) => {}
                _ = cancel_notify.notified() => {
                    return Err(BotBrowserError::ApprovalDenied(
                        "QR login cancelled by user".to_string(),
                    ));
                }
            }

            if self.get_card(&card_id).is_some_and(|c| c.status() == AssistCardStatus::Cancelled) {
                return Err(BotBrowserError::ApprovalDenied(
                    "QR login cancelled by user".to_string(),
                ));
            }

            let elapsed = started.elapsed();
            if elapsed >= max_wait {
                self.update_card_status(
                    &card_id,
                    chat_id,
                    AssistCardStatus::Expired,
                    Some("二维码已过期，请重新发起扫码登录".to_string()),
                    None,
                );
                return Ok(QrAssistResult {
                    status: "expired".to_string(),
                    card_id,
                    redirected_url: None,
                    matched_cookie: None,
                    poll_attempts: poll_attempt,
                });
            }

            poll_attempt += 1;

            let current_url = match bridge.inspect_true_origin(None).await {
                Ok((_, _, url)) => url,
                Err(_) => initial_url.clone(),
            };
            let current_cookies =
                extract_cookie_map(&bridge.get_cookies().await.unwrap_or_default());

            let url_matched = success_url_contains
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some_and(|needle| current_url.contains(needle));
            let url_redirected =
                url_matched || url_host_or_path_changed(&initial_url, &current_url);
            let matched_cookie =
                detect_new_auth_cookie(&initial_cookies, &current_cookies, &auth_cookie_names);

            if url_redirected || matched_cookie.is_some() {
                self.update_card_status(
                    &card_id,
                    chat_id,
                    AssistCardStatus::Completed,
                    None,
                    Some(current_url.clone()),
                );
                return Ok(QrAssistResult {
                    status: "qr_login_succeeded".to_string(),
                    card_id,
                    redirected_url: Some(current_url),
                    matched_cookie,
                    poll_attempts: poll_attempt,
                });
            }

            let (refreshed_png, _) = bridge.capture_element_png(qr_selector).await?;
            let countdown_secs = max_wait.saturating_sub(elapsed).as_secs() as u32;
            {
                let mut cards = self.cards.write();
                if let Some(entry) = cards.get_mut(&card_id) {
                    if let AssistCardPayload::QrLogin {
                        ref mut png_bytes,
                        countdown_secs: ref mut cd,
                        poll_attempt: ref mut pa,
                        ..
                    } = entry.payload
                    {
                        *png_bytes = refreshed_png.clone();
                        *cd = countdown_secs;
                        *pa = poll_attempt;
                    }
                }
            }
            let _ = self.event_tx.send(BotBrowserEvent::RefreshQrImage {
                card_id: card_id.clone(),
                chat_id,
                png_bytes: refreshed_png,
                countdown_secs,
                poll_attempt,
            });
        }
    }

    /// Crops a verification/captcha target element, displays a GPUI `ClickCaptcha` card, awaits
    /// normalized `[0.0, 1.0]` clicks from `submit_click_captcha`, and dispatches scaled CDP clicks.
    pub async fn tool_human_assist_click(
        &self,
        session_id: &str,
        task_id: &str,
        chat_id: Option<i64>,
        target_selector: &str,
        instruction: &str,
        max_clicks: u8,
    ) -> Result<ClickAssistResult, BotBrowserError> {
        let bridge = self.ensure_session_bridge(session_id, None).await?;
        let (origin, _, target_url) = bridge.inspect_true_origin(None).await?;
        let (png_bytes, element_rect) = bridge.capture_element_png(target_selector).await?;
        let max_clicks = max_clicks.max(1);

        let card_id = format!("card_{}", Uuid::new_v4().simple());
        let created_at_ms = Utc::now().timestamp_millis();
        let payload = AssistCardPayload::ClickCaptcha {
            card_id: card_id.clone(),
            task_id: task_id.to_string(),
            session_id: session_id.to_string(),
            chat_id,
            origin: origin.as_str().to_string(),
            target_url,
            png_bytes,
            element_rect: element_rect.clone(),
            instruction: instruction.to_string(),
            max_clicks,
            status: AssistCardStatus::AwaitingUser,
            status_note: None,
            created_at_ms,
        };

        let (tx, rx) = oneshot::channel();
        {
            let mut cards = self.cards.write();
            cards.insert(
                card_id.clone(),
                ActiveCardEntry {
                    payload: payload.clone(),
                    waiter: Some(CardWaiter::Click(tx)),
                    context: CardResumeContext::Click,
                },
            );
        }
        let _ = self.event_tx.send(BotBrowserEvent::UpsertCard(payload));

        let points = match tokio::time::timeout(self.config.click_wait_timeout, rx).await {
            Ok(Ok(Ok(pts))) => pts,
            Ok(Ok(Err(err))) => return Err(err),
            Ok(Err(_closed)) => {
                return Err(BotBrowserError::ApprovalDenied(
                    "click captcha waiter closed".to_string(),
                ));
            }
            Err(_elapsed) => {
                self.update_card_status(
                    &card_id,
                    chat_id,
                    AssistCardStatus::Expired,
                    Some("点选验证已超时".to_string()),
                    None,
                );
                return Ok(ClickAssistResult {
                    status: "expired".to_string(),
                    card_id,
                    count: 0,
                    dispatched_points: Vec::new(),
                });
            }
        };

        let mut dispatched = Vec::with_capacity(points.len().min(max_clicks as usize));
        for pt in points.into_iter().take(max_clicks as usize) {
            let coords = bridge
                .dispatch_element_relative_click(&element_rect, pt)
                .await?;
            dispatched.push(coords);
        }

        self.update_card_status(&card_id, chat_id, AssistCardStatus::Completed, None, None);

        Ok(ClickAssistResult {
            status: "clicks_dispatched".to_string(),
            card_id,
            count: dispatched.len(),
            dispatched_points: dispatched,
        })
    }

    /// Local-only GPUI callback submitting normalized `[0.0, 1.0]` click coordinates for `ClickCaptcha`.
    pub async fn submit_click_captcha(
        &self,
        card_id: &str,
        points: Vec<NormalizedClickPoint>,
    ) -> Result<usize, BotBrowserError> {
        if points.is_empty() {
            return Err(BotBrowserError::Cdp(
                "at least one click coordinate is required".to_string(),
            ));
        }

        let clamped: Vec<NormalizedClickPoint> = points.into_iter().map(|p| p.clamped()).collect();
        let count = clamped.len();

        let (waiter, chat_id) = {
            let mut cards = self.cards.write();
            let entry = cards
                .get_mut(card_id)
                .ok_or_else(|| BotBrowserError::CardNotFound(card_id.to_string()))?;

            let (chat_id, status) = match &entry.payload {
                AssistCardPayload::ClickCaptcha {
                    chat_id, status, ..
                } => (*chat_id, *status),
                _ => {
                    return Err(BotBrowserError::Cdp(format!(
                        "assist card '{card_id}' is not a ClickCaptcha card"
                    )));
                }
            };
            if status != AssistCardStatus::AwaitingUser {
                return Err(BotBrowserError::Cdp(format!(
                    "assist card '{card_id}' is not awaiting clicks (status {status:?})"
                )));
            }
            entry
                .payload
                .set_status(AssistCardStatus::ApprovedAndFilling, None);
            (entry.waiter.take(), chat_id)
        };

        self.broadcast_card_status(
            card_id,
            chat_id,
            AssistCardStatus::ApprovedAndFilling,
            None,
            None,
            None,
            None,
        );

        if let Some(CardWaiter::Click(tx)) = waiter {
            let _ = tx.send(Ok(clamped));
            Ok(count)
        } else {
            Err(BotBrowserError::Cdp(format!(
                "assist card '{card_id}' has no active click waiter"
            )))
        }
    }

    /// Cancels an active or timed-out assist card and wakes any pending waiter with `ApprovalDenied`.
    pub async fn cancel_card(&self, card_id: &str) -> Result<(), BotBrowserError> {
        let (chat_id, waiter, qr_notify) = {
            let mut cards = self.cards.write();
            let entry = cards
                .get_mut(card_id)
                .ok_or_else(|| BotBrowserError::CardNotFound(card_id.to_string()))?;
            let chat_id = entry.payload.chat_id();
            entry
                .payload
                .set_status(AssistCardStatus::Cancelled, Some("已取消".to_string()));
            let waiter = entry.waiter.take();
            let qr_notify = match &entry.context {
                CardResumeContext::Qr { cancel_notify } => Some(Arc::clone(cancel_notify)),
                _ => None,
            };
            (chat_id, waiter, qr_notify)
        };

        if let Some(waiter) = waiter {
            match waiter {
                CardWaiter::Login(tx) => {
                    let _ = tx.send(Err(BotBrowserError::ApprovalDenied(
                        "assist card cancelled by user".to_string(),
                    )));
                }
                CardWaiter::Reuse(tx) => {
                    let _ = tx.send(Err(BotBrowserError::ApprovalDenied(
                        "reuse confirmation cancelled by user".to_string(),
                    )));
                }
                CardWaiter::Click(tx) => {
                    let _ = tx.send(Err(BotBrowserError::ApprovalDenied(
                        "click captcha cancelled by user".to_string(),
                    )));
                }
            }
        }
        if let Some(notify) = qr_notify {
            notify.notify_waiters();
        }

        self.broadcast_card_status(
            card_id,
            chat_id,
            AssistCardStatus::Cancelled,
            Some("已取消".to_string()),
            None,
            None,
            None,
        );
        Ok(())
    }

    async fn execute_vault_fill_internal(
        &self,
        bridge: &Arc<CdpBridge>,
        credential_id: &str,
        selectors: &LoginFormSelectors,
    ) -> Result<FillSubmitResult, BotBrowserError> {
        let (page_origin, frame_origin, _href) = bridge
            .inspect_true_origin(Some(&selectors.password_selector))
            .await?;
        let secret = self.vault.decrypt_for_origin(
            credential_id,
            page_origin.as_str(),
            frame_origin.as_ref().map(|o| o.as_str()),
        )?;

        let totp_code = match secret.totp_secret.as_deref() {
            Some(raw) => Some(generate_totp_now(raw)?),
            None => None,
        };

        let res = bridge
            .atomic_fill_and_submit(&page_origin, &secret, totp_code.as_ref(), selectors)
            .await;

        drop(totp_code);
        drop(secret);
        res
    }

    fn transition_card_timed_out_resumable(
        &self,
        card_id: &str,
        chat_id: Option<i64>,
        note: &str,
    ) {
        {
            let mut cards = self.cards.write();
            if let Some(entry) = cards.get_mut(card_id) {
                entry.waiter = None;
                entry
                    .payload
                    .set_status(AssistCardStatus::TimedOutResumable, Some(note.to_string()));
            }
        }
        self.broadcast_card_status(
            card_id,
            chat_id,
            AssistCardStatus::TimedOutResumable,
            Some(note.to_string()),
            None,
            None,
            None,
        );
    }

    fn update_card_status(
        &self,
        card_id: &str,
        chat_id: Option<i64>,
        status: AssistCardStatus,
        note: Option<String>,
        redirected_url: Option<String>,
    ) {
        {
            let mut cards = self.cards.write();
            if let Some(entry) = cards.get_mut(card_id) {
                entry.payload.set_status(status, note.clone());
                if let AssistCardPayload::QrLogin {
                    redirected_url: ref mut redir,
                    ..
                } = entry.payload
                {
                    if redirected_url.is_some() {
                        *redir = redirected_url.clone();
                    }
                }
            }
        }
        self.broadcast_card_status(card_id, chat_id, status, note, None, None, redirected_url);
    }

    #[allow(clippy::too_many_arguments)]
    fn broadcast_card_status(
        &self,
        card_id: &str,
        chat_id: Option<i64>,
        status: AssistCardStatus,
        status_note: Option<String>,
        saved_credential_id: Option<String>,
        saved_account_hint: Option<String>,
        redirected_url: Option<String>,
    ) {
        let _ = self.event_tx.send(BotBrowserEvent::UpdateCardStatus {
            card_id: card_id.to_string(),
            chat_id,
            status,
            status_note,
            saved_credential_id,
            saved_account_hint,
            redirected_url,
        });
    }
}

fn validate_navigation_url(url: &str) -> Result<NormalizedOrigin, BotBrowserError> {
    let origin = NormalizedOrigin::parse(url)?;
    let host = origin.host();
    if host == "169.254.169.254" || host == "metadata.google.internal" {
        return Err(BotBrowserError::InvalidOrigin(
            "blocked cloud metadata endpoint URL".to_string(),
        ));
    }
    Ok(origin)
}

fn enforce_same_origin_frame(
    page_origin: &NormalizedOrigin,
    frame_origin: Option<&NormalizedOrigin>,
) -> Result<(), BotBrowserError> {
    if let Some(frame) = frame_origin {
        if frame != page_origin {
            return Err(BotBrowserError::FrameOriginMismatch {
                expected: page_origin.as_str().to_string(),
                actual: frame.as_str().to_string(),
            });
        }
    }
    Ok(())
}

fn verify_origin_against_meta(
    expected_origin: &str,
    actual_page: &NormalizedOrigin,
    actual_frame: Option<&NormalizedOrigin>,
) -> Result<(), BotBrowserError> {
    if actual_page.as_str() != expected_origin {
        return Err(BotBrowserError::OriginMismatch {
            expected: expected_origin.to_string(),
            actual: actual_page.as_str().to_string(),
        });
    }
    if let Some(frame) = actual_frame {
        if frame.as_str() != expected_origin {
            return Err(BotBrowserError::FrameOriginMismatch {
                expected: expected_origin.to_string(),
                actual: frame.as_str().to_string(),
            });
        }
    }
    Ok(())
}

fn set_vault_login_saved(
    payload: &mut AssistCardPayload,
    new_status: AssistCardStatus,
    note: Option<String>,
    credential_id: &str,
    account_hint: &str,
) {
    if let AssistCardPayload::VaultLogin {
        status,
        status_note,
        saved_credential_id,
        saved_account_hint,
        ..
    } = payload
    {
        *status = new_status;
        *status_note = note;
        *saved_credential_id = Some(credential_id.to_string());
        *saved_account_hint = Some(account_hint.to_string());
    }
}

fn card_created_at_ms(payload: &AssistCardPayload) -> i64 {
    match payload {
        AssistCardPayload::VaultLogin { created_at_ms, .. }
        | AssistCardPayload::ReuseConfirm { created_at_ms, .. }
        | AssistCardPayload::QrLogin { created_at_ms, .. }
        | AssistCardPayload::ClickCaptcha { created_at_ms, .. } => *created_at_ms,
    }
}

fn is_sensitive_selector_hint(selector: &str) -> bool {
    let lower = selector.to_ascii_lowercase();
    for marker in [
        "password",
        "passwd",
        "passcode",
        "pwd",
        "one-time-code",
        "totp",
    ] {
        if lower.contains(marker) {
            return true;
        }
    }
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| tok == "otp")
}

async fn probe_and_focus_non_sensitive(
    bridge: &Arc<CdpBridge>,
    selector: Option<&str>,
) -> Result<(), BotBrowserError> {
    let sel_json = match selector {
        Some(sel) => serde_json::to_string(sel)
            .map_err(|e| BotBrowserError::Cdp(format!("invalid selector: {e}")))?,
        None => "null".to_string(),
    };
    let script = format!(
        r#"(() => {{
  const sel = {sel_json};
  const el = sel ? document.querySelector(sel) : document.activeElement;
  if (sel && !el) return JSON.stringify({{ found: false, sensitive: false }});
  if (!el) return JSON.stringify({{ found: true, sensitive: false }});
  const t = String((typeof el.getAttribute === "function" ? el.getAttribute("type") : "") || el.type || "").toLowerCase();
  const ac = String((typeof el.getAttribute === "function" ? el.getAttribute("autocomplete") : "") || "").toLowerCase();
  const nm = String((typeof el.getAttribute === "function" ? el.getAttribute("name") : "") || "").toLowerCase();
  const eid = String((typeof el.getAttribute === "function" ? el.getAttribute("id") : "") || "").toLowerCase();
  const aria = String((typeof el.getAttribute === "function" ? el.getAttribute("aria-label") : "") || "").toLowerCase();
  const ph = String((typeof el.getAttribute === "function" ? el.getAttribute("placeholder") : "") || "").toLowerCase();
  const hay = [ac, nm, eid, aria, ph].join(" ");
  const sensitive = t === "password" || ac.includes("password") || ac === "one-time-code" || /\b(password|passwd|passcode|pwd|totp|otp)\b/.test(hay);
  if (!sensitive && sel && typeof el.focus === "function") el.focus();
  return JSON.stringify({{ found: true, sensitive }});
}})()"#
    );

    let eval_res = bridge.evaluate_script(&script).await?;
    let val = eval_res
        .get("result")
        .and_then(|r| r.get("value"))
        .or_else(|| eval_res.get("value"));
    let parsed_obj: Option<Value> = match val {
        Some(Value::String(s)) => serde_json::from_str(s).ok(),
        Some(v @ Value::Object(_)) => Some(v.clone()),
        _ => None,
    };

    if let Some(obj) = parsed_obj {
        if obj.get("sensitive").and_then(|v| v.as_bool()) == Some(true) {
            return Err(BotBrowserError::Cdp(
                "Refusing to type into password/OTP field via browser_type; use vault_request_card or vault_fill."
                    .to_string(),
            ));
        }
        if selector.is_some() && obj.get("found").and_then(|v| v.as_bool()) == Some(false) {
            return Err(BotBrowserError::Cdp(format!(
                "element not found for selector '{}'",
                selector.unwrap_or_default()
            )));
        }
    }

    Ok(())
}

async fn dispatch_enter_on_bridge(bridge: &Arc<CdpBridge>) -> Result<(), BotBrowserError> {
    bridge
        .transport()
        .send_command(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyDown",
                "key": "Enter",
                "code": "Enter",
                "text": "\r",
                "windowsVirtualKeyCode": 13
            }),
        )
        .await?;
    bridge
        .transport()
        .send_command(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "key": "Enter",
                "code": "Enter",
                "windowsVirtualKeyCode": 13
            }),
        )
        .await?;
    Ok(())
}

fn extract_cookie_map(cookies: &[Value]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for cookie in cookies {
        if let Some(name) = cookie.get("name").and_then(|v| v.as_str()) {
            let val = cookie
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            map.insert(name.to_string(), val);
        }
    }
    map
}

fn detect_new_auth_cookie(
    initial: &HashMap<String, String>,
    current: &HashMap<String, String>,
    auth_cookie_names: &[String],
) -> Option<String> {
    if !auth_cookie_names.is_empty() {
        for name in auth_cookie_names {
            if let Some(cur_val) = current.get(name) {
                if cur_val.is_empty() {
                    continue;
                }
                let changed = match initial.get(name) {
                    None => true,
                    Some(prev) => prev != cur_val,
                };
                if changed {
                    return Some(name.clone());
                }
            }
        }
        return None;
    }

    for (name, cur_val) in current {
        if !cur_val.is_empty() && !initial.contains_key(name) {
            return Some(name.clone());
        }
    }
    None
}

fn url_host_or_path_changed(initial_url: &str, current_url: &str) -> bool {
    if initial_url == current_url {
        return false;
    }
    match (Url::parse(initial_url), Url::parse(current_url)) {
        (Ok(a), Ok(b)) => {
            (
                a.scheme(),
                a.host_str(),
                a.port_or_known_default(),
                a.path(),
            ) != (
                b.scheme(),
                b.host_str(),
                b.port_or_known_default(),
                b.path(),
            )
        }
        _ => initial_url != current_url,
    }
}
