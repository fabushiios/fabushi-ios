use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use image::ImageFormat;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use crate::cdp::{CdpBridge, CdpTransport, ObscuraSidecarTransport, WsCdpTransport, check_eval_exception};
use crate::sidecar::{
    ObscuraBinaryManifest, ObscuraDownloader, ObscuraSidecar, ProfileNamespace, SidecarConfig,
};
use crate::types::{
    BotBrowserError, BrowserSnapshot, ElementRect, NormalizedOrigin,
};

pub use crate::types::{
    BrowserBackendKind, BrowserCookie, SameSitePolicy, SemanticLocator, StepTimingMetric,
};

const MAX_RECENT_STEP_METRICS: usize = 256;
const DEFAULT_NAVIGATE_SETTLE_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_CDP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationResult {
    pub url: String,
    pub origin: String,
    pub title: String,
    pub duration_ms: u64,
    pub backend: BrowserBackendKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionOutcome {
    pub ok: bool,
    pub operation: String,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_text: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OffscreenFrameBuffer {
    pub width: u32,
    pub height: u32,
    pub rgba_bytes: Vec<u8>,
    pub captured_at_ms: i64,
}

impl std::fmt::Debug for OffscreenFrameBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OffscreenFrameBuffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_byte_len", &self.rgba_bytes.len())
            .field("captured_at_ms", &self.captured_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerEventKind {
    Move,
    Down,
    Up,
    Wheel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    None,
    Left,
    Middle,
    Right,
}

impl PointerButton {
    fn as_cdp_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Left => "left",
            Self::Middle => "middle",
            Self::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerInputEvent {
    pub kind: PointerEventKind,
    pub x: f64,
    pub y: f64,
    pub button: PointerButton,
    pub click_count: u8,
    pub delta_x: f64,
    pub delta_y: f64,
    /// Bitmask matching CDP Input modifiers: 1=Alt, 2=Ctrl, 4=Meta, 8=Shift.
    pub modifiers: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyEventKind {
    KeyDown,
    KeyUp,
    Char,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyInputEvent {
    pub kind: KeyEventKind,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_virtual_key_code: Option<u32>,
    #[serde(default)]
    pub modifiers: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserPerformanceMetrics {
    pub backend: Option<BrowserBackendKind>,
    pub total_operations: u64,
    pub failed_operations: u64,
    pub total_duration_ms: u64,
    pub avg_duration_ms: u64,
    pub max_duration_ms: u64,
    pub p95_duration_ms: u64,
    pub recent_steps: Vec<StepTimingMetric>,
}

#[derive(Debug, Clone)]
pub struct ChromeLaunchConfig {
    pub binary_path: Option<PathBuf>,
    pub user_data_dir: PathBuf,
    pub headless: bool,
    pub startup_timeout: Duration,
    pub command_timeout: Duration,
    pub extra_args: Vec<String>,
}

impl ChromeLaunchConfig {
    pub fn new(user_data_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary_path: None,
            user_data_dir: user_data_dir.into(),
            headless: true,
            startup_timeout: Duration::from_secs(15),
            command_timeout: DEFAULT_CDP_TIMEOUT,
            extra_args: Vec::new(),
        }
    }
}

#[async_trait]
pub trait BrowserSessionBackend: Send + Sync {
    fn backend_kind(&self) -> BrowserBackendKind;

    async fn navigate(&self, url: &str) -> Result<NavigationResult, BotBrowserError>;
    async fn go_back(&self) -> Result<(), BotBrowserError>;
    async fn go_forward(&self) -> Result<(), BotBrowserError>;
    async fn reload(&self) -> Result<(), BotBrowserError>;

    async fn current_url(&self) -> Result<String, BotBrowserError>;
    async fn page_title(&self) -> Result<String, BotBrowserError>;
    async fn snapshot_dom(&self) -> Result<BrowserSnapshot, BotBrowserError>;
    async fn eval_js(&self, script: &str) -> Result<Value, BotBrowserError>;

    async fn click_locator(&self, locator: &SemanticLocator)
    -> Result<ActionOutcome, BotBrowserError>;
    async fn fill_locator(
        &self,
        locator: &SemanticLocator,
        value: &str,
        is_secret: bool,
    ) -> Result<ActionOutcome, BotBrowserError>;
    async fn press_key(&self, key: &str) -> Result<ActionOutcome, BotBrowserError>;
    async fn scroll(&self, delta_x: f64, delta_y: f64) -> Result<ActionOutcome, BotBrowserError>;
    async fn wait_for_locator(
        &self,
        locator: &SemanticLocator,
        timeout_ms: u64,
    ) -> Result<ActionOutcome, BotBrowserError>;

    async fn extract_text_locator(
        &self,
        locator: &SemanticLocator,
    ) -> Result<String, BotBrowserError> {
        if locator.is_empty() {
            return Err(BotBrowserError::LocatorNotFound(
                "empty semantic locator".to_string(),
            ));
        }
        if locator.is_sensitive_hint() {
            return Err(BotBrowserError::Cdp(format!(
                "refusing to extract value from sensitive locator ({})",
                locator.display_label()
            )));
        }
        let script = build_resolve_locator_js(locator, false, false)?;
        let val = self.eval_js(&script).await?;
        let resolved: ResolvedLocatorPayload = parse_resolved_value(val, "extract_text_locator")?;
        if !resolved.found {
            return Err(BotBrowserError::LocatorNotFound(locator.display_label()));
        }
        if resolved.is_sensitive {
            return Err(BotBrowserError::Cdp(format!(
                "refusing to extract value from sensitive field ({})",
                locator.display_label()
            )));
        }
        Ok(resolved.text.unwrap_or_default())
    }

    async fn capture_screenshot(&self) -> Result<Vec<u8>, BotBrowserError>;

    /// Decodes PNG screenshot into RGBA8 offscreen frame buffer for GPUI `RenderImage`
    /// on a blocking Tokio worker (`spawn_blocking`) so async executors never stall.
    async fn capture_frame_rgba(&self) -> Result<OffscreenFrameBuffer, BotBrowserError> {
        let png = self.capture_screenshot().await?;
        tokio::task::spawn_blocking(move || decode_png_to_rgba(&png))
            .await
            .map_err(|e| BotBrowserError::Io(format!("frame decode task failed: {e}")))?
    }

    async fn export_cookies(
        &self,
        domain_filter: Option<&str>,
    ) -> Result<Vec<BrowserCookie>, BotBrowserError>;
    async fn import_cookies(&self, cookies: &[BrowserCookie]) -> Result<usize, BotBrowserError>;

    async fn dispatch_pointer_event(&self, event: PointerInputEvent)
    -> Result<(), BotBrowserError>;
    async fn dispatch_key_event(&self, event: KeyInputEvent) -> Result<(), BotBrowserError>;

    fn performance_metrics(&self) -> BrowserPerformanceMetrics {
        BrowserPerformanceMetrics {
            backend: Some(self.backend_kind()),
            ..Default::default()
        }
    }

    async fn close(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }
}

struct TelemetryRecorder {
    backend: BrowserBackendKind,
    total_operations: u64,
    failed_operations: u64,
    total_duration_ms: u64,
    max_duration_ms: u64,
    recent_steps: VecDeque<StepTimingMetric>,
}

impl TelemetryRecorder {
    fn new(backend: BrowserBackendKind) -> Self {
        Self {
            backend,
            total_operations: 0,
            failed_operations: 0,
            total_duration_ms: 0,
            max_duration_ms: 0,
            recent_steps: VecDeque::with_capacity(MAX_RECENT_STEP_METRICS),
        }
    }

    fn record(&mut self, mut metric: StepTimingMetric) {
        if metric.backend.is_none() {
            metric.backend = Some(self.backend);
        }
        if metric.timestamp_ms == 0 {
            metric.timestamp_ms = Utc::now().timestamp_millis();
        }
        self.total_operations = self.total_operations.saturating_add(1);
        if !metric.success {
            self.failed_operations = self.failed_operations.saturating_add(1);
        }
        self.total_duration_ms = self.total_duration_ms.saturating_add(metric.duration_ms);
        self.max_duration_ms = self.max_duration_ms.max(metric.duration_ms);
        if self.recent_steps.len() >= MAX_RECENT_STEP_METRICS {
            self.recent_steps.pop_front();
        }
        self.recent_steps.push_back(metric);
    }

    fn snapshot(&self) -> BrowserPerformanceMetrics {
        let avg_duration_ms = if self.total_operations > 0 {
            self.total_duration_ms / self.total_operations
        } else {
            0
        };
        let p95_duration_ms = if self.recent_steps.is_empty() {
            0
        } else {
            let mut sorted: Vec<u64> = self.recent_steps.iter().map(|s| s.duration_ms).collect();
            sorted.sort_unstable();
            let idx = ((sorted.len() as f64) * 0.95).ceil() as usize;
            let clamped = idx.saturating_sub(1).min(sorted.len() - 1);
            sorted[clamped]
        };
        BrowserPerformanceMetrics {
            backend: Some(self.backend),
            total_operations: self.total_operations,
            failed_operations: self.failed_operations,
            total_duration_ms: self.total_duration_ms,
            avg_duration_ms,
            max_duration_ms: self.max_duration_ms,
            p95_duration_ms,
            recent_steps: self.recent_steps.iter().cloned().collect(),
        }
    }

    fn reset(&mut self) {
        self.total_operations = 0;
        self.failed_operations = 0;
        self.total_duration_ms = 0;
        self.max_duration_ms = 0;
        self.recent_steps.clear();
    }
}

enum BackendLifecycle {
    Obscura {
        sidecar: Arc<ObscuraSidecar>,
        heartbeat: JoinHandle<()>,
    },
    Chrome {
        child: Option<Child>,
    },
}

impl Drop for BackendLifecycle {
    fn drop(&mut self) {
        match self {
            Self::Obscura { heartbeat, .. } => {
                heartbeat.abort();
            }
            Self::Chrome { child } => {
                if let Some(mut proc_child) = child.take() {
                    let _ = proc_child.start_kill();
                }
            }
        }
    }
}

pub struct CdpBrowserBackend {
    kind: BrowserBackendKind,
    bridge: Arc<CdpBridge>,
    telemetry: Arc<Mutex<TelemetryRecorder>>,
    lifecycle: tokio::sync::Mutex<Option<BackendLifecycle>>,
    navigate_settle_timeout: Duration,
}

impl CdpBrowserBackend {
    pub fn from_bridge(kind: BrowserBackendKind, bridge: Arc<CdpBridge>) -> Self {
        Self {
            kind,
            bridge,
            telemetry: Arc::new(Mutex::new(TelemetryRecorder::new(kind))),
            lifecycle: tokio::sync::Mutex::new(None),
            navigate_settle_timeout: DEFAULT_NAVIGATE_SETTLE_TIMEOUT,
        }
    }

    pub fn from_transport(kind: BrowserBackendKind, transport: Arc<dyn CdpTransport>) -> Self {
        Self::from_bridge(kind, Arc::new(CdpBridge::new(transport)))
    }

    pub fn new_mock(transport: Arc<dyn CdpTransport>) -> Self {
        Self::from_transport(BrowserBackendKind::MockCdp, transport)
    }

    pub fn connect_cdp(
        kind: BrowserBackendKind,
        endpoint: impl Into<String>,
        command_timeout: Duration,
    ) -> Self {
        let transport = Arc::new(WsCdpTransport::with_timeout(endpoint, command_timeout));
        Self::from_transport(kind, transport)
    }

    pub async fn launch_obscura(
        session_id: impl Into<String>,
        data_dir: &Path,
        origin_hint: Option<&NormalizedOrigin>,
    ) -> Result<Self, BotBrowserError> {
        if cfg!(target_os = "ios") {
            Err(BotBrowserError::SidecarStartFailed(
                "launching Obscura sidecar subprocess is unavailable inside the iOS App Sandbox"
                    .to_string(),
            ))
        } else {
            let session_id = session_id.into();
            let http_client = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let manifest = ObscuraBinaryManifest::default_obscura();
            let binary_path =
                ObscuraDownloader::ensure_binary(data_dir, &manifest, &http_client).await?;
            let storage_dir = ProfileNamespace::storage_dir_for(data_dir, &session_id, origin_hint);
            let sidecar_cfg = SidecarConfig::new(binary_path, storage_dir);
            let sidecar = Arc::new(ObscuraSidecar::start(sidecar_cfg, &session_id, None).await?);
            let heartbeat = sidecar.spawn_heartbeat();
            let transport: Arc<dyn CdpTransport> =
                Arc::new(ObscuraSidecarTransport::new(Arc::clone(&sidecar)));
            let backend = Self::from_transport(BrowserBackendKind::ObscuraCdp, transport);
            *backend.lifecycle.lock().await =
                Some(BackendLifecycle::Obscura { sidecar, heartbeat });
            Ok(backend)
        }
    }

    pub async fn launch_chrome(config: ChromeLaunchConfig) -> Result<Self, BotBrowserError> {
        if cfg!(target_os = "ios") {
            Err(BotBrowserError::SidecarStartFailed(
                "launching desktop Chrome subprocess is unavailable inside the iOS App Sandbox; use Apple-native WKWebView or a user-authorized remote CDP host"
                    .to_string(),
            ))
        } else {
            let binary = match config.binary_path {
                Some(path) if path.is_file() => path,
                Some(path) => {
                    return Err(BotBrowserError::SidecarStartFailed(format!(
                        "Chrome binary does not exist at {}",
                        path.display()
                    )));
                }
                None => discover_chrome_binary().ok_or_else(|| {
                    BotBrowserError::SidecarStartFailed(
                        "could not locate system Chrome/Chromium binary".to_string(),
                    )
                })?,
            };

            std::fs::create_dir_all(&config.user_data_dir)?;
            let active_port_file = config.user_data_dir.join("DevToolsActivePort");
            let _ = std::fs::remove_file(&active_port_file);

            let mut cmd = Command::new(&binary);
            cmd.arg("--remote-debugging-port=0")
                .arg(format!("--user-data-dir={}", config.user_data_dir.display()))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-background-networking")
                .arg("--disable-sync");
            if config.headless {
                cmd.arg("--headless=new");
            }
            for arg in &config.extra_args {
                cmd.arg(arg);
            }
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true);

            let mut child = cmd.spawn().map_err(|e| {
                BotBrowserError::SidecarStartFailed(format!(
                    "failed to spawn Chrome '{}': {e}",
                    binary.display()
                ))
            })?;

            let stderr = child.stderr.take();
            let (ws_tx, mut ws_rx) = tokio::sync::oneshot::channel::<String>();
            if let Some(err_stream) = stderr {
                tokio::spawn(async move {
                    let mut reader = BufReader::new(err_stream).lines();
                    let mut tx_opt = Some(ws_tx);
                    while let Ok(Some(line)) = reader.next_line().await {
                        if let Some(idx) = line.find("ws://") {
                            let url = line[idx..].trim().to_string();
                            if let Some(tx) = tx_opt.take() {
                                let _ = tx.send(url);
                            }
                        }
                    }
                });
            }

            let deadline = tokio::time::Instant::now() + config.startup_timeout;
            let ws_endpoint = loop {
                if let Ok(url) = ws_rx.try_recv() {
                    break url;
                }
                if let Ok(contents) = std::fs::read_to_string(&active_port_file) {
                    let mut lines = contents.lines();
                    if let (Some(port_str), Some(path_str)) = (lines.next(), lines.next()) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            let ws_path = path_str.trim();
                            if !ws_path.is_empty() {
                                break format!("ws://127.0.0.1:{port}{ws_path}");
                            }
                        }
                    }
                }
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(BotBrowserError::SidecarStartFailed(format!(
                        "Chrome exited prematurely with status {status}"
                    )));
                }
                if tokio::time::Instant::now() >= deadline {
                    let _ = child.start_kill();
                    return Err(BotBrowserError::SidecarTimeout {
                        method: "launch_chrome".to_string(),
                        timeout_ms: config.startup_timeout.as_millis() as u64,
                    });
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            };

            let backend = Self::connect_cdp(
                BrowserBackendKind::ChromeCdp,
                ws_endpoint,
                config.command_timeout,
            );
            *backend.lifecycle.lock().await = Some(BackendLifecycle::Chrome { child: Some(child) });
            Ok(backend)
        }
    }

    pub fn with_navigate_settle_timeout(mut self, timeout: Duration) -> Self {
        self.navigate_settle_timeout = timeout;
        self
    }

    pub fn bridge(&self) -> &Arc<CdpBridge> {
        &self.bridge
    }

    pub fn record_step_metric(&self, metric: StepTimingMetric) {
        self.telemetry.lock().record(metric);
    }

    pub fn reset_metrics(&self) {
        self.telemetry.lock().reset();
    }

    pub async fn read_locator_text(
        &self,
        locator: &SemanticLocator,
    ) -> Result<Option<String>, BotBrowserError> {
        match self.extract_text_locator(locator).await {
            Ok(txt) => Ok(Some(txt)),
            Err(BotBrowserError::LocatorNotFound(_)) => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn record_op(
        &self,
        operation: &str,
        label: impl Into<String>,
        started: Instant,
        success: bool,
        detail: Option<String>,
    ) -> u64 {
        let duration_ms = started.elapsed().as_millis() as u64;
        self.telemetry.lock().record(StepTimingMetric {
            step_index: None,
            operation: operation.to_string(),
            label: label.into(),
            backend: Some(self.kind),
            duration_ms,
            success,
            timestamp_ms: Utc::now().timestamp_millis(),
            detail,
        });
        duration_ms
    }

    async fn resolve_locator_internal(
        &self,
        locator: &SemanticLocator,
        scroll_into_view: bool,
        focus: bool,
    ) -> Result<ResolvedLocatorPayload, BotBrowserError> {
        if locator.is_empty() {
            return Err(BotBrowserError::LocatorNotFound(
                "empty semantic locator".to_string(),
            ));
        }
        let script = build_resolve_locator_js(locator, scroll_into_view, focus)?;
        let res = self.bridge.evaluate_script(&script).await?;
        let resolved: ResolvedLocatorPayload = parse_eval_json(&res, "resolve_locator")?;
        if !resolved.found {
            return Err(BotBrowserError::LocatorNotFound(locator.display_label()));
        }
        Ok(resolved)
    }

    async fn clear_focused_input_internal(&self) -> Result<(), BotBrowserError> {
        let clear_expr = r#"(() => {
  const el = document.activeElement;
  if (el && typeof el.value === "string" && el.value.length > 0) {
    el.value = "";
    try { el.dispatchEvent(new Event("input", { bubbles: true })); } catch (_) {}
    try { el.dispatchEvent(new Event("change", { bubbles: true })); } catch (_) {}
  }
  return true;
})()"#;
        let _ = self.bridge.evaluate_script(clear_expr).await?;
        Ok(())
    }

    async fn wait_for_document_settled(&self) {
        let deadline = tokio::time::Instant::now() + self.navigate_settle_timeout;
        loop {
            let Ok(res) = self
                .bridge
                .evaluate_script("String(document.readyState || '')")
                .await
            else {
                break;
            };
            let state = extract_eval_value(&res).and_then(|v| v.as_str()).unwrap_or("");
            if state != "loading" {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

const WAIT_FOR_POLL_INTERVAL: Duration = Duration::from_millis(50);
const WAIT_FOR_TRANSIENT_CDP_BACKOFF: Duration = Duration::from_millis(25);
const MAX_CONSECUTIVE_TRANSIENT_CDP_RETRIES: u32 = 20;

fn is_transient_navigation_cdp_error(err: &BotBrowserError) -> bool {
    let BotBrowserError::Cdp(msg) = err else {
        return false;
    };
    let lower = msg.to_ascii_lowercase();
    [
        "inspected target navigated or closed",
        "execution context was destroyed",
        "execution context destroyed",
        "cannot find context with specified id",
        "cannot find default execution context",
        "document was destroyed",
        "no frame for given id found",
        "frame with the given id was not found",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
}

#[async_trait]
impl BrowserSessionBackend for CdpBrowserBackend {
    fn backend_kind(&self) -> BrowserBackendKind {
        self.kind
    }

    async fn navigate(&self, url: &str) -> Result<NavigationResult, BotBrowserError> {
        let started = Instant::now();
        let origin = match validate_navigation_url(url) {
            Ok(o) => o,
            Err(e) => {
                self.record_op("navigate", url, started, false, Some(e.to_string()));
                return Err(e);
            }
        };

        if let Err(e) = self.bridge.navigate(url).await {
            self.record_op("navigate", url, started, false, Some(e.to_string()));
            return Err(e);
        }

        self.wait_for_document_settled().await;

        let info_res = self
            .bridge
            .evaluate_script(
                "JSON.stringify({ url: String(window.location.href || ''), title: String(document.title || '') })",
            )
            .await;

        let (final_url, title) = match info_res {
            Ok(val) => match parse_eval_json::<PageUrlTitle>(&val, "navigate_info") {
                Ok(parsed) => {
                    let final_u = if parsed.url.trim().is_empty() {
                        url.to_string()
                    } else {
                        parsed.url
                    };
                    (final_u, parsed.title)
                }
                Err(_) => (url.to_string(), String::new()),
            },
            Err(_) => (url.to_string(), String::new()),
        };

        if final_url.starts_with("chrome-error://") {
            let err = BotBrowserError::Cdp(format!(
                "navigation failed: landed on browser error page ({final_url})"
            ));
            self.record_op("navigate", &final_url, started, false, Some(err.to_string()));
            return Err(err);
        }

        let final_origin = NormalizedOrigin::parse(&final_url)
            .map(|o| o.as_str().to_string())
            .unwrap_or_else(|_| origin.as_str().to_string());
        let duration_ms = self.record_op("navigate", &final_url, started, true, None);
        Ok(NavigationResult {
            url: final_url,
            origin: final_origin,
            title,
            duration_ms,
            backend: self.kind,
        })
    }

    async fn go_back(&self) -> Result<(), BotBrowserError> {
        let started = Instant::now();
        let res = match self
            .bridge
            .send_guarded_command("Page.getNavigationHistory", json!({}))
            .await
        {
            Ok(hist) => {
                let current_idx = hist
                    .get("currentIndex")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1);
                let entries = hist.get("entries").and_then(|v| v.as_array());
                if let (true, Some(arr)) = (current_idx > 0, entries) {
                    let target_idx = (current_idx - 1) as usize;
                    if let Some(entry_id) = arr
                        .get(target_idx)
                        .and_then(|e| e.get("id"))
                        .and_then(|v| v.as_i64())
                    {
                        self.bridge
                            .send_guarded_command(
                                "Page.navigateToHistoryEntry",
                                json!({ "entryId": entry_id }),
                            )
                            .await
                            .map(|_| ())
                    } else {
                        self.bridge
                            .evaluate_script("window.history.back(); true")
                            .await
                            .map(|_| ())
                    }
                } else if entries.is_some_and(|a| !a.is_empty()) && current_idx == 0 {
                    Err(BotBrowserError::Cdp(
                        "no previous navigation history entry".to_string(),
                    ))
                } else {
                    self.bridge
                        .evaluate_script("window.history.back(); true")
                        .await
                        .map(|_| ())
                }
            }
            Err(BotBrowserError::Cdp(_)) => self
                .bridge
                .evaluate_script("window.history.back(); true")
                .await
                .map(|_| ()),
            Err(e) => Err(e),
        };
        self.record_op(
            "go_back",
            "history.back",
            started,
            res.is_ok(),
            res.as_ref().err().map(ToString::to_string),
        );
        res
    }

    async fn go_forward(&self) -> Result<(), BotBrowserError> {
        let started = Instant::now();
        let res = match self
            .bridge
            .send_guarded_command("Page.getNavigationHistory", json!({}))
            .await
        {
            Ok(hist) => {
                let current_idx = hist
                    .get("currentIndex")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1);
                let entries = hist.get("entries").and_then(|v| v.as_array());
                if let (true, Some(arr)) = (current_idx >= 0, entries) {
                    let target_idx = (current_idx + 1) as usize;
                    if let Some(entry_id) = arr
                        .get(target_idx)
                        .and_then(|e| e.get("id"))
                        .and_then(|v| v.as_i64())
                    {
                        self.bridge
                            .send_guarded_command(
                                "Page.navigateToHistoryEntry",
                                json!({ "entryId": entry_id }),
                            )
                            .await
                            .map(|_| ())
                    } else {
                        Err(BotBrowserError::Cdp(
                            "no forward navigation history entry".to_string(),
                        ))
                    }
                } else {
                    self.bridge
                        .evaluate_script("window.history.forward(); true")
                        .await
                        .map(|_| ())
                }
            }
            Err(BotBrowserError::Cdp(_)) => self
                .bridge
                .evaluate_script("window.history.forward(); true")
                .await
                .map(|_| ()),
            Err(e) => Err(e),
        };
        self.record_op(
            "go_forward",
            "history.forward",
            started,
            res.is_ok(),
            res.as_ref().err().map(ToString::to_string),
        );
        res
    }

    async fn reload(&self) -> Result<(), BotBrowserError> {
        let started = Instant::now();
        let res = self
            .bridge
            .send_guarded_command("Page.reload", json!({ "ignoreCache": false }))
            .await
            .map(|_| ());
        if res.is_ok() {
            self.wait_for_document_settled().await;
        }
        self.record_op(
            "reload",
            "reload",
            started,
            res.is_ok(),
            res.as_ref().err().map(ToString::to_string),
        );
        res
    }

    async fn current_url(&self) -> Result<String, BotBrowserError> {
        let res = self
            .bridge
            .evaluate_script("String(window.location.href || '')")
            .await?;
        check_eval_exception(&res, "current_url")?;
        Ok(extract_eval_value(&res)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string())
    }

    async fn page_title(&self) -> Result<String, BotBrowserError> {
        let res = self
            .bridge
            .evaluate_script("String(document.title || '')")
            .await?;
        check_eval_exception(&res, "page_title")?;
        Ok(extract_eval_value(&res)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string())
    }

    async fn snapshot_dom(&self) -> Result<BrowserSnapshot, BotBrowserError> {
        let started = Instant::now();
        let res = self.bridge.capture_redacted_snapshot(None).await;
        self.record_op(
            "snapshot_dom",
            "snapshot_dom",
            started,
            res.is_ok(),
            res.as_ref().err().map(ToString::to_string),
        );
        res
    }

    async fn eval_js(&self, script: &str) -> Result<Value, BotBrowserError> {
        let started = Instant::now();
        let raw = self.bridge.evaluate_script(script).await;
        match raw {
            Ok(val) => {
                if let Err(err) = check_eval_exception(&val, "eval_js") {
                    self.record_op("eval_js", "eval_js", started, false, Some(err.to_string()));
                    return Err(err);
                }
                self.record_op("eval_js", "eval_js", started, true, None);
                Ok(extract_eval_value(&val).cloned().unwrap_or(Value::Null))
            }
            Err(err) => {
                self.record_op("eval_js", "eval_js", started, false, Some(err.to_string()));
                Err(err)
            }
        }
    }

    async fn click_locator(
        &self,
        locator: &SemanticLocator,
    ) -> Result<ActionOutcome, BotBrowserError> {
        let started = Instant::now();
        let label = locator.display_label();
        let resolved = match self.resolve_locator_internal(locator, true, false).await {
            Ok(r) => r,
            Err(e) => {
                self.record_op("click_locator", &label, started, false, Some(e.to_string()));
                return Err(e);
            }
        };

        let mut clicked_x = None;
        let mut clicked_y = None;
        let rect = resolved.rect.as_ref();
        let has_box = rect.is_some_and(|r| r.width > 0.0 && r.height > 0.0);
        let click_res = if has_box {
            let cx = resolved
                .center_x
                .unwrap_or_else(|| rect.map_or(0.0, |r| r.x + r.width / 2.0));
            let cy = resolved
                .center_y
                .unwrap_or_else(|| rect.map_or(0.0, |r| r.y + r.height / 2.0));
            clicked_x = Some(cx);
            clicked_y = Some(cy);
            self.bridge.click_coords(cx, cy).await.map(|_| ())
        } else if let Some(sel) = resolved.selector.as_deref() {
            self.bridge.click_selector(sel).await.map(|_| ())
        } else {
            Err(BotBrowserError::LocatorNotFound(label.clone()))
        };

        if let Err(e) = click_res {
            self.record_op("click_locator", &label, started, false, Some(e.to_string()));
            return Err(e);
        }

        let duration_ms = self.record_op("click_locator", &label, started, true, None);
        Ok(ActionOutcome {
            ok: true,
            operation: "click_locator".to_string(),
            duration_ms,
            matched_strategy: resolved.strategy,
            matched_selector: resolved.selector,
            x: clicked_x,
            y: clicked_y,
            extracted_text: None,
        })
    }

    async fn fill_locator(
        &self,
        locator: &SemanticLocator,
        value: &str,
        is_secret: bool,
    ) -> Result<ActionOutcome, BotBrowserError> {
        let started = Instant::now();
        let label = locator.display_label();

        if !is_secret && locator.is_sensitive_hint() {
            let err = BotBrowserError::Cdp(format!(
                "refusing non-secret fill into sensitive locator ({label})"
            ));
            self.record_op("fill_locator", &label, started, false, Some(err.to_string()));
            return Err(err);
        }

        let resolved = match self.resolve_locator_internal(locator, true, true).await {
            Ok(r) => r,
            Err(e) => {
                self.record_op("fill_locator", &label, started, false, Some(e.to_string()));
                return Err(e);
            }
        };

        if !is_secret && resolved.is_sensitive {
            let err = BotBrowserError::Cdp(format!(
                "refusing non-secret fill into sensitive/password element ({label})"
            ));
            self.record_op("fill_locator", &label, started, false, Some(err.to_string()));
            return Err(err);
        }

        if let Err(e) = self.clear_focused_input_internal().await {
            self.record_op("fill_locator", &label, started, false, Some(e.to_string()));
            return Err(e);
        }

        // Secrets must ONLY travel via Input.insertText, never Runtime.evaluate.
        if let Err(e) = self.bridge.insert_text(value).await {
            self.record_op("fill_locator", &label, started, false, Some(e.to_string()));
            return Err(e);
        }

        let detail = if is_secret {
            Some("secret_filled".to_string())
        } else {
            None
        };
        let duration_ms = self.record_op("fill_locator", &label, started, true, detail);
        Ok(ActionOutcome {
            ok: true,
            operation: "fill_locator".to_string(),
            duration_ms,
            matched_strategy: resolved.strategy,
            matched_selector: resolved.selector,
            x: resolved.center_x,
            y: resolved.center_y,
            extracted_text: None,
        })
    }

    async fn press_key(&self, key: &str) -> Result<ActionOutcome, BotBrowserError> {
        let started = Instant::now();
        let (key_name, code, text, vk) = map_named_key(key);

        let down_res = self
            .dispatch_key_event(KeyInputEvent {
                kind: KeyEventKind::KeyDown,
                key: key_name.clone(),
                code: Some(code.clone()),
                text: text.clone(),
                windows_virtual_key_code: vk,
                modifiers: 0,
            })
            .await;

        let res = match down_res {
            Ok(()) => {
                self.dispatch_key_event(KeyInputEvent {
                    kind: KeyEventKind::KeyUp,
                    key: key_name,
                    code: Some(code),
                    text: None,
                    windows_virtual_key_code: vk,
                    modifiers: 0,
                })
                .await
            }
            Err(e) => Err(e),
        };

        if let Err(e) = res {
            self.record_op("press_key", key, started, false, Some(e.to_string()));
            return Err(e);
        }
        let duration_ms = self.record_op("press_key", key, started, true, None);
        Ok(ActionOutcome {
            ok: true,
            operation: "press_key".to_string(),
            duration_ms,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }

    async fn scroll(&self, delta_x: f64, delta_y: f64) -> Result<ActionOutcome, BotBrowserError> {
        let started = Instant::now();
        if !delta_x.is_finite() || !delta_y.is_finite() {
            let err = BotBrowserError::Cdp(format!(
                "scroll requires finite delta_x and delta_y, got ({delta_x}, {delta_y})"
            ));
            self.record_op(
                "scroll",
                format!("({delta_x}, {delta_y})"),
                started,
                false,
                Some(err.to_string()),
            );
            return Err(err);
        }
        let wheel_res = self
            .dispatch_pointer_event(PointerInputEvent {
                kind: PointerEventKind::Wheel,
                x: 100.0,
                y: 100.0,
                button: PointerButton::None,
                click_count: 0,
                delta_x,
                delta_y,
                modifiers: 0,
            })
            .await;

        let res = match wheel_res {
            Ok(()) => Ok(()),
            Err(BotBrowserError::Cdp(_)) => {
                match self
                    .bridge
                    .evaluate_script(&format!(
                        "window.scrollBy({delta_x}, {delta_y}); true"
                    ))
                    .await
                {
                    Ok(eval_res) => check_eval_exception(&eval_res, "scroll"),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        };

        if let Err(e) = res {
            self.record_op(
                "scroll",
                format!("({delta_x}, {delta_y})"),
                started,
                false,
                Some(e.to_string()),
            );
            return Err(e);
        }
        let duration_ms =
            self.record_op("scroll", format!("({delta_x}, {delta_y})"), started, true, None);
        Ok(ActionOutcome {
            ok: true,
            operation: "scroll".to_string(),
            duration_ms,
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
        timeout_ms: u64,
    ) -> Result<ActionOutcome, BotBrowserError> {
        let started = Instant::now();
        let label = locator.display_label();
        if locator.is_empty() {
            let err = BotBrowserError::LocatorNotFound("empty semantic locator".to_string());
            self.record_op(
                "wait_for_locator",
                &label,
                started,
                false,
                Some(err.to_string()),
            );
            return Err(err);
        }

        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_millis(timeout_ms.max(1)));
        let mut consecutive_transient_cdp: u32 = 0;
        loop {
            match self.resolve_locator_internal(locator, false, false).await {
                Ok(resolved) => {
                    let duration_ms =
                        self.record_op("wait_for_locator", &label, started, true, None);
                    return Ok(ActionOutcome {
                        ok: true,
                        operation: "wait_for_locator".to_string(),
                        duration_ms,
                        matched_strategy: resolved.strategy,
                        matched_selector: resolved.selector,
                        x: resolved.center_x,
                        y: resolved.center_y,
                        extracted_text: None,
                    });
                }
                Err(BotBrowserError::LocatorNotFound(_)) => {
                    consecutive_transient_cdp = 0;
                    if deadline.is_some_and(|d| tokio::time::Instant::now() >= d) {
                        let err = BotBrowserError::LocatorNotFound(format!(
                            "timed out after {timeout_ms}ms waiting for {label}"
                        ));
                        self.record_op(
                            "wait_for_locator",
                            &label,
                            started,
                            false,
                            Some(err.to_string()),
                        );
                        return Err(err);
                    }
                    tokio::time::sleep(WAIT_FOR_POLL_INTERVAL).await;
                }
                Err(err)
                    if is_transient_navigation_cdp_error(&err)
                        && consecutive_transient_cdp < MAX_CONSECUTIVE_TRANSIENT_CDP_RETRIES =>
                {
                    let now = tokio::time::Instant::now();
                    if deadline.is_some_and(|d| now >= d) {
                        self.record_op(
                            "wait_for_locator",
                            &label,
                            started,
                            false,
                            Some(err.to_string()),
                        );
                        return Err(err);
                    }
                    consecutive_transient_cdp += 1;
                    let sleep_for = match deadline {
                        Some(d) => d
                            .saturating_duration_since(now)
                            .min(WAIT_FOR_TRANSIENT_CDP_BACKOFF),
                        None => WAIT_FOR_TRANSIENT_CDP_BACKOFF,
                    };
                    if !sleep_for.is_zero() {
                        tokio::time::sleep(sleep_for).await;
                    }
                }
                Err(err) => {
                    self.record_op(
                        "wait_for_locator",
                        &label,
                        started,
                        false,
                        Some(err.to_string()),
                    );
                    return Err(err);
                }
            }
        }
    }

    async fn extract_text_locator(
        &self,
        locator: &SemanticLocator,
    ) -> Result<String, BotBrowserError> {
        let started = Instant::now();
        let label = locator.display_label();
        if locator.is_sensitive_hint() {
            let err = BotBrowserError::Cdp(format!(
                "refusing to extract value from sensitive locator ({label})"
            ));
            self.record_op(
                "extract_text_locator",
                &label,
                started,
                false,
                Some(err.to_string()),
            );
            return Err(err);
        }
        let resolved = match self.resolve_locator_internal(locator, false, false).await {
            Ok(r) => r,
            Err(e) => {
                self.record_op(
                    "extract_text_locator",
                    &label,
                    started,
                    false,
                    Some(e.to_string()),
                );
                return Err(e);
            }
        };

        if resolved.is_sensitive {
            let err = BotBrowserError::Cdp(format!(
                "refusing to extract value from sensitive field ({label})"
            ));
            self.record_op(
                "extract_text_locator",
                &label,
                started,
                false,
                Some(err.to_string()),
            );
            return Err(err);
        }

        self.record_op("extract_text_locator", &label, started, true, None);
        Ok(resolved.text.unwrap_or_default())
    }

    async fn capture_screenshot(&self) -> Result<Vec<u8>, BotBrowserError> {
        let started = Instant::now();
        let res = self.bridge.capture_screenshot_png(false).await;
        self.record_op(
            "capture_screenshot",
            "viewport",
            started,
            res.is_ok(),
            res.as_ref().err().map(ToString::to_string),
        );
        res
    }

    async fn export_cookies(
        &self,
        domain_filter: Option<&str>,
    ) -> Result<Vec<BrowserCookie>, BotBrowserError> {
        let started = Instant::now();
        let res = self.bridge.get_browser_cookies(domain_filter).await;
        self.record_op(
            "export_cookies",
            domain_filter.unwrap_or("*"),
            started,
            res.is_ok(),
            res.as_ref().err().map(ToString::to_string),
        );
        res
    }

    async fn import_cookies(&self, cookies: &[BrowserCookie]) -> Result<usize, BotBrowserError> {
        let started = Instant::now();
        let mut valid: Vec<BrowserCookie> = Vec::with_capacity(cookies.len());
        for cookie in cookies {
            let has_disallowed = |s: &str| {
                s.chars()
                    .any(|c| c.is_ascii_control() || matches!(c, '\r' | '\n' | '\0' | ';'))
            };
            if has_disallowed(&cookie.name)
                || has_disallowed(&cookie.value)
                || has_disallowed(&cookie.domain)
                || has_disallowed(&cookie.path)
            {
                let err = BotBrowserError::CookieImport(
                    "cookie contains disallowed control characters or ';'".to_string(),
                );
                self.record_op(
                    "import_cookies",
                    format!("{} cookies", cookies.len()),
                    started,
                    false,
                    Some(err.to_string()),
                );
                return Err(err);
            }
            if cookie.name.trim().is_empty() {
                continue;
            }
            let mut normalized = cookie.clone();
            if let Err(err) = crate::session_import::validate_and_normalize_cookie(&mut normalized) {
                self.record_op(
                    "import_cookies",
                    format!("{} cookies", cookies.len()),
                    started,
                    false,
                    Some(err.to_string()),
                );
                return Err(err);
            }
            valid.push(normalized);
        }
        let res = self.bridge.set_browser_cookies(&valid).await;
        self.record_op(
            "import_cookies",
            format!("{} cookies", valid.len()),
            started,
            res.is_ok(),
            res.as_ref().err().map(ToString::to_string),
        );
        res
    }

    async fn dispatch_pointer_event(&self, event: PointerInputEvent) -> Result<(), BotBrowserError> {
        let cdp_type = match event.kind {
            PointerEventKind::Move => "mouseMoved",
            PointerEventKind::Down => "mousePressed",
            PointerEventKind::Up => "mouseReleased",
            PointerEventKind::Wheel => "mouseWheel",
        };
        let mut params = json!({
            "type": cdp_type,
            "x": event.x,
            "y": event.y,
            "button": event.button.as_cdp_str(),
            "modifiers": event.modifiers,
        });
        if matches!(event.kind, PointerEventKind::Down | PointerEventKind::Up) {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("clickCount".to_string(), json!(event.click_count.max(1)));
            }
        }
        if event.kind == PointerEventKind::Wheel {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("deltaX".to_string(), json!(event.delta_x));
                obj.insert("deltaY".to_string(), json!(event.delta_y));
            }
        }
        self.bridge
            .send_guarded_command("Input.dispatchMouseEvent", params)
            .await
            .map(|_| ())
    }

    async fn dispatch_key_event(&self, event: KeyInputEvent) -> Result<(), BotBrowserError> {
        if event.kind == KeyEventKind::Char {
            if let Some(text) = event.text.as_deref().filter(|t| !t.is_empty()) {
                let use_insert_text =
                    event.key == "Unidentified" || text.chars().count() > 1 || !text.is_ascii();
                if use_insert_text {
                    return self.bridge.insert_text(text).await.map(|_| ());
                }
            }
        }

        let cdp_type = match event.kind {
            KeyEventKind::KeyDown => "keyDown",
            KeyEventKind::KeyUp => "keyUp",
            KeyEventKind::Char => "char",
        };
        let mut params = json!({
            "type": cdp_type,
            "key": event.key,
            "modifiers": event.modifiers,
        });
        if let Some(obj) = params.as_object_mut() {
            if let Some(code) = event.code {
                obj.insert("code".to_string(), Value::String(code));
            }
            if let Some(text) = event.text {
                obj.insert("text".to_string(), Value::String(text));
            }
            if let Some(vk) = event.windows_virtual_key_code {
                obj.insert("windowsVirtualKeyCode".to_string(), json!(vk));
            }
        }
        self.bridge
            .send_guarded_command("Input.dispatchKeyEvent", params)
            .await
            .map(|_| ())
    }

    fn performance_metrics(&self) -> BrowserPerformanceMetrics {
        self.telemetry.lock().snapshot()
    }

    async fn close(&self) -> Result<(), BotBrowserError> {
        let mut lifecycle = self.lifecycle.lock().await.take();
        if let Some(ref mut lc) = lifecycle {
            match lc {
                BackendLifecycle::Obscura { sidecar, heartbeat } => {
                    heartbeat.abort();
                    sidecar.shutdown().await?;
                }
                BackendLifecycle::Chrome { child } => {
                    if let Some(mut proc_child) = child.take() {
                        let _ = proc_child.start_kill();
                        let _ =
                            tokio::time::timeout(Duration::from_secs(3), proc_child.wait()).await;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct PageUrlTitle {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct ResolvedLocatorPayload {
    #[serde(default)]
    found: bool,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default)]
    is_sensitive: bool,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    rect: Option<ElementRect>,
    #[serde(default)]
    center_x: Option<f64>,
    #[serde(default)]
    center_y: Option<f64>,
    #[serde(default)]
    text: Option<String>,
}

fn parse_resolved_value<T: for<'de> Deserialize<'de>>(
    val: Value,
    op_name: &str,
) -> Result<T, BotBrowserError> {
    if let Some(json_str) = val.as_str() {
        serde_json::from_str::<T>(json_str)
            .map_err(|e| BotBrowserError::Cdp(format!("{op_name}: invalid JSON payload: {e}")))
    } else {
        serde_json::from_value::<T>(val)
            .map_err(|e| BotBrowserError::Cdp(format!("{op_name}: unexpected result shape: {e}")))
    }
}

fn extract_eval_value(res: &Value) -> Option<&Value> {
    res.get("result")
        .and_then(|r| r.get("value"))
        .or_else(|| res.get("value"))
}

fn parse_eval_json<T: for<'de> Deserialize<'de>>(
    res: &Value,
    op_name: &str,
) -> Result<T, BotBrowserError> {
    check_eval_exception(res, op_name)?;
    let val = extract_eval_value(res)
        .ok_or_else(|| BotBrowserError::Cdp(format!("{op_name}: missing Runtime.evaluate result")))?;
    parse_resolved_value(val.clone(), op_name)
}

fn build_resolve_locator_js(
    locator: &SemanticLocator,
    scroll_into_view: bool,
    focus: bool,
) -> Result<String, BotBrowserError> {
    let loc_json = serde_json::to_string(locator)
        .map_err(|e| BotBrowserError::Cdp(format!("failed serializing SemanticLocator: {e}")))?
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let scroll_flag = if scroll_into_view { "true" } else { "false" };
    let focus_flag = if focus { "true" } else { "false" };

    let template = r####"(() => {
  const loc = __LOC_JSON__;
  const shouldScroll = __SCROLL_FLAG__;
  const shouldFocus = __FOCUS_FLAG__;

  function isVisible(el) {
    if (!el || el.nodeType !== 1) return false;
    try {
      const style = window.getComputedStyle(el);
      if (style && (style.display === "none" || style.visibility === "hidden" || style.opacity === "0")) {
        return false;
      }
    } catch (_) {}
    if (typeof el.getBoundingClientRect === "function") {
      const r = el.getBoundingClientRect();
      if (r && (r.width > 0 || r.height > 0)) return true;
    }
    return (el.offsetWidth || 0) > 0 || (el.offsetHeight || 0) > 0 || (typeof el.getClientRects === "function" && el.getClientRects().length > 0);
  }

  function pickPreferred(candidates) {
    if (!candidates || candidates.length === 0) return null;
    for (const el of candidates) {
      if (isVisible(el)) return el;
    }
    return candidates[0];
  }

  function isSensitive(el) {
    const t = String((typeof el.getAttribute === "function" ? el.getAttribute("type") : "") || el.type || "").toLowerCase();
    const ac = String((typeof el.getAttribute === "function" ? el.getAttribute("autocomplete") : "") || "").toLowerCase();
    const nm = String((typeof el.getAttribute === "function" ? el.getAttribute("name") : "") || "").toLowerCase();
    const eid = String((typeof el.getAttribute === "function" ? el.getAttribute("id") : "") || "").toLowerCase();
    const aria = String((typeof el.getAttribute === "function" ? el.getAttribute("aria-label") : "") || "").toLowerCase();
    const ph = String((typeof el.getAttribute === "function" ? el.getAttribute("placeholder") : "") || "").toLowerCase();
    const tid = String((typeof el.getAttribute === "function" ? el.getAttribute("data-testid") : "") || "").toLowerCase();
    const hay = [ac, nm, eid, aria, ph, tid].join(" ");
    return (
      t === "password" ||
      ac.includes("password") ||
      ac === "one-time-code" ||
      hay.includes("password") ||
      hay.includes("passwd") ||
      hay.includes("passcode") ||
      hay.includes("pwd") ||
      hay.includes("totp") ||
      hay.includes("one-time-code") ||
      hay.includes("secret") ||
      hay.includes("api_key") ||
      hay.includes("apikey") ||
      hay.includes("auth_token") ||
      hay.includes("access_token") ||
      hay.includes("ct0") ||
      /(?:^|[^a-z0-9])(otp|token|pin)(?:[^a-z0-9]|$)/.test(hay)
    );
  }

  function cssPath(el) {
    if (el.id) {
      try { return "#" + CSS.escape(el.id); } catch (_) { return "#" + el.id; }
    }
    const tid = el.getAttribute("data-testid");
    if (tid) {
      return '[data-testid="' + String(tid).replace(/"/g, '\\"') + '"]';
    }
    const nm = el.getAttribute("name");
    const tag = (el.tagName || "div").toLowerCase();
    if (nm) {
      return tag + '[name="' + String(nm).replace(/"/g, '\\"') + '"]';
    }
    return tag;
  }

  function implicitRole(el) {
    const explicit = el.getAttribute("role");
    if (explicit) return String(explicit).toLowerCase();
    const tag = String(el.tagName || "").toLowerCase();
    if (tag === "a" && el.hasAttribute("href")) return "link";
    if (tag === "button") return "button";
    if (tag === "textarea") return "textbox";
    if (tag === "select") return "combobox";
    if (tag === "input") {
      const tp = String(el.getAttribute("type") || "text").toLowerCase();
      if (tp === "submit" || tp === "button" || tp === "reset") return "button";
      if (tp === "checkbox") return "checkbox";
      if (tp === "radio") return "radio";
      if (tp !== "hidden") return "textbox";
    }
    if (/^h[1-6]$/.test(tag)) return "heading";
    return "";
  }

  function elementText(el) {
    const raw = (el.innerText || el.textContent || el.getAttribute("aria-label") || el.getAttribute("placeholder") || "");
    return String(raw).replace(/\s+/g, " ").trim();
  }

  let matched = null;
  let strategy = null;
  const allEls = Array.from(document.querySelectorAll("*"));

  if (loc.test_id) {
    const cands = allEls.filter(el => el.getAttribute && el.getAttribute("data-testid") === loc.test_id);
    matched = pickPreferred(cands);
    if (matched) strategy = "test_id";
  }

  if (!matched && loc.aria_label) {
    const roleWant = loc.role ? String(loc.role).toLowerCase() : null;
    const cands = allEls.filter(el => el.getAttribute && el.getAttribute("aria-label") === loc.aria_label);
    if (roleWant) {
      matched = pickPreferred(cands.filter(el => implicitRole(el) === roleWant));
    }
    if (!matched) matched = pickPreferred(cands);
    if (matched) strategy = "aria_label";
  }

  if (!matched && loc.name_attr) {
    const cands = allEls.filter(el => el.getAttribute && el.getAttribute("name") === loc.name_attr);
    matched = pickPreferred(cands);
    if (matched) strategy = "name_attr";
  }

  if (!matched && loc.role && loc.text_anchor) {
    const roleWant = String(loc.role).toLowerCase();
    const anchor = String(loc.text_anchor).replace(/\s+/g, " ").trim().toLowerCase();
    const roleCands = allEls.filter(el => implicitRole(el) === roleWant);
    const exact = roleCands.filter(el => elementText(el).toLowerCase() === anchor);
    const partial = exact.length > 0 ? exact : roleCands.filter(el => elementText(el).toLowerCase().includes(anchor));
    matched = pickPreferred(partial);
    if (matched) strategy = "role_and_text";
  }

  if (!matched && loc.text_anchor) {
    const anchor = String(loc.text_anchor).replace(/\s+/g, " ").trim().toLowerCase();
    const interactive = Array.from(document.querySelectorAll("a, button, input, select, textarea, [role], label"));
    const iExact = interactive.filter(el => elementText(el).toLowerCase() === anchor);
    const iPart = iExact.length > 0 ? iExact : interactive.filter(el => elementText(el).toLowerCase().includes(anchor));
    matched = pickPreferred(iPart);
    if (!matched) {
      const leafCands = allEls.filter(el => {
        const tag = String(el.tagName || "").toLowerCase();
        if (tag === "html" || tag === "body" || tag === "script" || tag === "style") return false;
        return elementText(el).toLowerCase().includes(anchor);
      });
      leafCands.sort((a, b) => elementText(a).length - elementText(b).length);
      matched = pickPreferred(leafCands);
    }
    if (matched) strategy = "text_anchor";
  }

  if (!matched && loc.css_selector) {
    try {
      const cands = Array.from(document.querySelectorAll(loc.css_selector));
      matched = pickPreferred(cands);
      if (matched) strategy = "css_selector";
    } catch (_) {}
  }

  if (!matched && loc.role) {
    const roleWant = String(loc.role).toLowerCase();
    const cands = allEls.filter(el => implicitRole(el) === roleWant);
    matched = pickPreferred(cands);
    if (matched) strategy = "role";
  }

  if (!matched) {
    return JSON.stringify({ found: false });
  }

  if (shouldScroll && typeof matched.scrollIntoView === "function") {
    try { matched.scrollIntoView({ block: "center", inline: "center" }); } catch (_) {}
  }
  if (shouldFocus && typeof matched.focus === "function") {
    try { matched.focus(); } catch (_) {}
  }

  const r = typeof matched.getBoundingClientRect === "function" ? matched.getBoundingClientRect() : null;
  const dpr = Number(window.devicePixelRatio || 1);
  const rect = r ? {
    x: Number(r.x || 0),
    y: Number(r.y || 0),
    width: Number(r.width || 0),
    height: Number(r.height || 0),
    device_pixel_ratio: dpr
  } : null;
  const cx = rect && rect.width > 0 ? rect.x + rect.width / 2 : null;
  const cy = rect && rect.height > 0 ? rect.y + rect.height / 2 : null;
  const sensitive = isSensitive(matched);
  const tag = String(matched.tagName || "").toLowerCase();
  let safeText = "";
  if (!sensitive) {
    if ((tag === "input" || tag === "textarea" || tag === "select") && typeof matched.value === "string" && matched.value.length > 0) {
      safeText = matched.value;
    } else {
      safeText = String(matched.innerText || matched.textContent || "").trim();
    }
  }

  return JSON.stringify({
    found: true,
    strategy,
    tag,
    is_sensitive: sensitive,
    selector: cssPath(matched),
    rect,
    center_x: cx,
    center_y: cy,
    text: safeText
  });
})()"####;

    Ok(template
        .replace("__LOC_JSON__", &loc_json)
        .replace("__SCROLL_FLAG__", scroll_flag)
        .replace("__FOCUS_FLAG__", focus_flag))
}

pub fn decode_png_to_rgba(png_bytes: &[u8]) -> Result<OffscreenFrameBuffer, BotBrowserError> {
    let img = image::load_from_memory_with_format(png_bytes, ImageFormat::Png)
        .map_err(|e| BotBrowserError::Cdp(format!("failed to decode PNG frame: {e}")))?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(OffscreenFrameBuffer {
        width,
        height,
        rgba_bytes: rgba.into_raw(),
        captured_at_ms: Utc::now().timestamp_millis(),
    })
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

fn discover_chrome_binary() -> Option<PathBuf> {
    if cfg!(target_os = "ios") {
        None
    } else {
        if let Ok(env_bin) = std::env::var("CHROME_BIN") {
            let p = PathBuf::from(env_bin.trim());
            if p.is_file() {
                return Some(p);
            }
        }
        let candidates: &[&str] = if cfg!(target_os = "macos") {
            &[
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            ]
        } else if cfg!(target_os = "windows") {
            &[
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            ]
        } else {
            &[
                "/usr/bin/google-chrome",
                "/usr/bin/google-chrome-stable",
                "/usr/bin/chromium",
                "/usr/bin/chromium-browser",
                "/snap/bin/chromium",
            ]
        };

        for cand in candidates {
            let p = PathBuf::from(cand);
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
}

fn map_named_key(raw: &str) -> (String, String, Option<String>, Option<u32>) {
    let trimmed = raw.trim();
    match trimmed {
        "Enter" | "Return" => ("Enter".into(), "Enter".into(), Some("\r".into()), Some(13)),
        "Tab" => ("Tab".into(), "Tab".into(), None, Some(9)),
        "Escape" | "Esc" => ("Escape".into(), "Escape".into(), None, Some(27)),
        "Backspace" => ("Backspace".into(), "Backspace".into(), None, Some(8)),
        "Delete" => ("Delete".into(), "Delete".into(), None, Some(46)),
        "ArrowUp" => ("ArrowUp".into(), "ArrowUp".into(), None, Some(38)),
        "ArrowDown" => ("ArrowDown".into(), "ArrowDown".into(), None, Some(40)),
        "ArrowLeft" => ("ArrowLeft".into(), "ArrowLeft".into(), None, Some(37)),
        "ArrowRight" => ("ArrowRight".into(), "ArrowRight".into(), None, Some(39)),
        "Space" | " " => (" ".into(), "Space".into(), Some(" ".into()), Some(32)),
        other => {
            if other.chars().count() == 1 {
                let ch = other.chars().next().expect("checked");
                let code = if ch.is_ascii_alphabetic() {
                    format!("Key{}", ch.to_ascii_uppercase())
                } else if ch.is_ascii_digit() {
                    format!("Digit{ch}")
                } else {
                    other.to_string()
                };
                let vk = if ch.is_ascii() {
                    Some(ch.to_ascii_uppercase() as u32)
                } else {
                    None
                };
                (other.to_string(), code, Some(other.to_string()), vk)
            } else {
                (other.to_string(), other.to_string(), None, None)
            }
        }
    }
}
