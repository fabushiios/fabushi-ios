use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::ThreadId;
use std::time::Duration;

use bot_browser::e2ee_sync::{DeviceSyncKeypair, UnlockedUserVaultKey, UserVaultKeyEnvelope};
use chrono::Utc;
use fabushi_messaging_core::engine::MessagingEngine;
use fabushi_miniapp::state::FabushiClientState;
use fabushi_remote_engine::control_lease::RemoteControlHub;
use fabushi_remote_engine::terminal::Osc52StreamScanner;
use gpui_telegram_agent::computer::ComputerLeaseBroker;
use gpui_telegram_agent::hermes::security::ValueBoundApprovalLedger;
use gpui_telegram_agent::hermes::toolsets::{ToolsetId, ToolsetPreset};
use gpui_telegram_agent::hermes::HermesEngineConfig;
use gpui_telegram_agent::mcp::ToolPermissionDock;
use gpui_telegram_agent::multi_agent::{FloorControlQueue, LoopBudget, SharedGroupWorkspace};
use gpui_telegram_agent::talk::{TalkCoordinator, VadBargeInDetector};
use gpui_telegram_agent::tasks::TaskFsm;
use gpui_telegram_agent::HermesEngine;
use gpui_telegram_client::client::TelegramClient;
use gpui_telegram_client::config::{ClientConfig, DEFAULT_HTTP_TIMEOUT_SECS, DEFAULT_SERVER_URL};
use gpui_telegram_client::e2ee::{DoubleRatchetSession, SenderKeyState};
use gpui_telegram_client::outbox::OutboxStatusEvent;
use gpui_telegram_core::action_bus::{register_full_app_actions, ActionRegistry};
use gpui_telegram_core::auth::GenerationFence;
use gpui_telegram_core::types::ChatId;
use gpui_telegram_coding::store::CodingStore;
use gpui_telegram_media::chunked_upload::ChunkedUploadSession;
use gpui_telegram_proto::miniapp_contracts::MiniAppControlMode;
use gpui_telegram_webrtc::calls::WebRtcCallEngine;
use gpui_telegram_webrtc::media::DefaultAiMediaTap;
use gpui_telegram_webrtc::sfu::SfuBridge;
use magpie_gateway::{MaskingConfig, MaskingEngine, RouteRuntimeMetrics, RoutingGroup, UsageLedger};
use parking_lot::{Condvar, Mutex, RwLock};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::compat::default_app_data_dir;
use crate::dedup::MessageDedupStore;
use crate::error::{HostError, HostResult};
use crate::events::{
    domain_event_to_host_event, outbox_event_to_host_event, BridgeEventSink, EventQueue, HostEvent,
    TelegramBridgeEventCallback,
};
use crate::search::HostSearchEngine;
use crate::storage::HostSqliteStore;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HostConfig {
    pub app_data_dir: Option<String>,
    pub storage_dir: Option<String>,
    pub storage_passphrase: Option<String>,
    pub server_url: Option<String>,
    pub http_timeout_secs: Option<u64>,
    #[serde(default)]
    pub test_mode: bool,
    /// Enforces fail-closed iOS sandbox capability checks. Always `true` on `target_os = "ios"`.
    pub ios_sandbox_strict: Option<bool>,
}

/// iOS Jetsam-safe Argon2id upper bounds to prevent OOM DoS from untrusted server envelopes
/// or corrupted persisted parameters.
pub const MAX_ARGON2_M_COST_KIB: u32 = 262_144; // 256 MiB
pub const MAX_ARGON2_T_COST: u32 = 32;
pub const MAX_ARGON2_P_COST: u32 = 16;

pub fn validate_argon2_ios_ceiling(m_cost_kib: u32, t_cost: u32, p_cost: u32) -> HostResult<()> {
    if m_cost_kib > MAX_ARGON2_M_COST_KIB
        || t_cost > MAX_ARGON2_T_COST
        || p_cost > MAX_ARGON2_P_COST
    {
        return Err(HostError::invalid_arguments(format!(
            "Argon2id parameters exceed iOS Jetsam safety ceiling (m_cost_kib={m_cost_kib} <= {MAX_ARGON2_M_COST_KIB}, t_cost={t_cost} <= {MAX_ARGON2_T_COST}, p_cost={p_cost} <= {MAX_ARGON2_P_COST})"
        )));
    }
    Ok(())
}

/// RAM-only cached bearer token strictly bound to the `server_url` it was issued for.
#[derive(Debug, Clone)]
pub struct RamAccountToken {
    pub server_url: String,
    pub token: Zeroizing<String>,
}

struct InFlightGuard<'a> {
    host: &'a FabushiAppHost,
    request_id: String,
    call_seq: u64,
}

impl<'a> Drop for InFlightGuard<'a> {
    fn drop(&mut self) {
        let mut map = self.host.in_flight_cancels.lock();
        if let Some(entries) = map.get_mut(&self.request_id) {
            entries.retain(|(seq, _)| *seq != self.call_seq);
            if entries.is_empty() {
                map.remove(&self.request_id);
            }
        }
    }
}

struct CallbackGateState {
    sink: Option<BridgeEventSink>,
    shutdown: bool,
    active_threads: HashMap<ThreadId, usize>,
}

struct ActiveCallbackGuard {
    gate: Arc<(Mutex<CallbackGateState>, Condvar)>,
    tid: ThreadId,
}

impl Drop for ActiveCallbackGuard {
    fn drop(&mut self) {
        let mut state = self.gate.0.lock();
        if let Some(count) = state.active_threads.get_mut(&self.tid) {
            if *count <= 1 {
                state.active_threads.remove(&self.tid);
            } else {
                *count -= 1;
            }
        }
        self.gate.1.notify_all();
    }
}

fn emit_to_sink_guarded(
    gate: &Arc<(Mutex<CallbackGateState>, Condvar)>,
    event_type: &str,
    payload_json: &str,
) {
    let tid = std::thread::current().id();
    let sink = {
        let mut state = gate.0.lock();
        if state.shutdown {
            return;
        }
        let Some(sink) = state.sink.clone() else {
            return;
        };
        *state.active_threads.entry(tid).or_insert(0) += 1;
        sink
    };
    let _active_guard = ActiveCallbackGuard {
        gate: Arc::clone(gate),
        tid,
    };
    sink.emit(event_type, payload_json);
}

struct PumpControl {
    stop: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
}

struct PumpLifecycleState {
    control: Option<PumpControl>,
    pump_tid: Option<ThreadId>,
    stopping: bool,
    swapping: bool,
    shutdown: bool,
}

struct SwappingGuard<'a> {
    lifecycle: &'a (Mutex<PumpLifecycleState>, Condvar),
}

impl<'a> Drop for SwappingGuard<'a> {
    fn drop(&mut self) {
        let mut state = self.lifecycle.0.lock();
        state.swapping = false;
        self.lifecycle.1.notify_all();
    }
}

fn validate_server_url_input(raw_url: &str) -> HostResult<String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(HostError::invalid_arguments("server_url cannot be empty"));
    }
    if trimmed
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(HostError::invalid_arguments(
            "server_url must not contain whitespace or control characters",
        ));
    }
    if let Some(scheme_pos) = trimmed.find("://") {
        let scheme = &trimmed[..scheme_pos];
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return Err(HostError::invalid_arguments(format!(
                "Unsupported server_url scheme '{scheme}': only http and https are allowed"
            )));
        }
    } else if trimmed.starts_with("javascript:") || trimmed.starts_with("file:") || trimmed.starts_with(':') {
        return Err(HostError::invalid_arguments(
            "Invalid server_url format; expected http:// or https:// host",
        ));
    }
    let normalized = ClientConfig::normalize_url(trimmed);
    let after_scheme = normalized
        .strip_prefix("https://")
        .or_else(|| normalized.strip_prefix("http://"))
        .unwrap_or("");
    let authority = after_scheme.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.starts_with(':') || authority.contains("://") {
        return Err(HostError::invalid_arguments(format!(
            "Invalid server_url authority in '{raw_url}'"
        )));
    }
    if authority.starts_with('[') {
        let Some(close_idx) = authority.find(']') else {
            return Err(HostError::invalid_arguments(format!(
                "Invalid bracketed IPv6 authority in '{raw_url}'"
            )));
        };
        let ipv6_inner = &authority[1..close_idx];
        let rest = &authority[close_idx + 1..];
        if ipv6_inner.is_empty()
            || ipv6_inner.contains('[')
            || ipv6_inner.contains(']')
            || rest.contains('[')
            || rest.contains(']')
        {
            return Err(HostError::invalid_arguments(format!(
                "Invalid bracketed IPv6 authority in '{raw_url}'"
            )));
        }
        if !rest.is_empty() {
            let Some(port_part) = rest.strip_prefix(':') else {
                return Err(HostError::invalid_arguments(format!(
                    "Invalid bracketed IPv6 host/port in '{raw_url}'"
                )));
            };
            if port_part.is_empty() || port_part.parse::<u16>().is_err() {
                return Err(HostError::invalid_arguments(format!(
                    "Invalid server_url host/port in '{raw_url}'"
                )));
            }
        }
    } else {
        if authority.contains('[') || authority.contains(']') {
            return Err(HostError::invalid_arguments(format!(
                "Invalid bracketed IPv6 authority in '{raw_url}'"
            )));
        }
        if let Some((host_part, port_part)) = authority.rsplit_once(':') {
            if host_part.is_empty() || port_part.is_empty() || port_part.parse::<u16>().is_err() {
                return Err(HostError::invalid_arguments(format!(
                    "Invalid server_url host/port in '{raw_url}'"
                )));
            }
        }
    }
    Ok(normalized)
}

pub struct FabushiAppHost {
    pub storage_dir: PathBuf,
    pub storage_passphrase: Option<Zeroizing<String>>,
    pub test_mode: bool,
    pub ios_sandbox_strict: bool,
    pub http_timeout_secs: u64,
    pub server_url: RwLock<String>,
    pub client: RwLock<Arc<TelegramClient>>,
    pub host_db: Arc<HostSqliteStore>,
    pub search_engine: Arc<HostSearchEngine>,
    pub hermes: Arc<HermesEngine>,
    pub coding_store: Arc<Mutex<CodingStore>>,
    pub miniapp_db: Arc<Mutex<Connection>>,
    pub dharma_state: Arc<Mutex<FabushiClientState>>,
    pub action_registry: Arc<ActionRegistry>,
    pub messaging_engine: Arc<Mutex<MessagingEngine>>,
    pub event_queue: Arc<EventQueue>,
    callback_gate: Arc<(Mutex<CallbackGateState>, Condvar)>,
    pub generation_fence: Arc<GenerationFence>,
    gen_lock: Mutex<()>,
    pub(crate) session_lock: Mutex<()>,
    reconfig_lock: Mutex<()>,
    is_shutdown: AtomicBool,
    pub dedup_store: Arc<Mutex<MessageDedupStore>>,
    pub next_message_id: AtomicI64,
    next_call_seq: AtomicU64,
    pub in_flight_cancels: Mutex<HashMap<String, Vec<(u64, CancellationToken)>>>,
    pump_lifecycle: (Mutex<PumpLifecycleState>, Condvar),
    /// In-memory bearer token cache keyed by account_id and bound to server_url
    /// (used when storage_passphrase is absent; bearer tokens are NEVER written in plaintext to host_state.db).
    pub account_tokens: Mutex<HashMap<String, RamAccountToken>>,
    // Domain engine state
    pub vault_unlocked_key: Mutex<Option<UnlockedUserVaultKey>>,
    pub vault_envelope: Mutex<Option<UserVaultKeyEnvelope>>,
    pub vault_device_keypairs: Mutex<HashMap<String, DeviceSyncKeypair>>,
    pub task_registry: Mutex<HashMap<String, TaskFsm>>,
    pub tool_dock: Mutex<ToolPermissionDock>,
    pub approval_ledger: Mutex<ValueBoundApprovalLedger>,
    pub cua_broker: Mutex<ComputerLeaseBroker>,
    remote_hub: RwLock<Arc<RemoteControlHub>>,
    pub osc52_scanner: Mutex<Osc52StreamScanner>,
    pub floor_queues: Mutex<HashMap<i64, FloorControlQueue>>,
    pub loop_budgets: Mutex<HashMap<i64, LoopBudget>>,
    pub group_workspaces: Mutex<HashMap<i64, SharedGroupWorkspace>>,
    pub magpie_groups: Mutex<HashMap<String, RoutingGroup>>,
    pub magpie_metrics: Mutex<RouteRuntimeMetrics>,
    pub magpie_masking: Mutex<MaskingEngine>,
    pub magpie_usage: Mutex<UsageLedger>,
    pub call_engine: Mutex<WebRtcCallEngine>,
    pub sfu_bridge: Mutex<SfuBridge>,
    pub ai_media_tap: Arc<DefaultAiMediaTap>,
    pub talk_coordinator: Mutex<TalkCoordinator>,
    pub vad_detector: Mutex<VadBargeInDetector>,
    pub webmcp_control_modes: Mutex<HashMap<String, MiniAppControlMode>>,
    pub chunked_sessions: Mutex<HashMap<String, ChunkedUploadSession>>,
    pub local_dr_sessions: Mutex<HashMap<String, DoubleRatchetSession>>,
    pub local_sender_keys: Mutex<HashMap<String, SenderKeyState>>,
}

/// Type alias preserving legacy naming while routing strictly to the single canonical engine.
pub type TelegramAppHost = FabushiAppHost;

impl FabushiAppHost {
    pub fn from_config(config: HostConfig) -> Result<Self, String> {
        let test_mode = {
            #[cfg(all(target_os = "ios", not(debug_assertions)))]
            {
                false
            }
            #[cfg(not(all(target_os = "ios", not(debug_assertions))))]
            {
                config.test_mode
            }
        };

        let ios_sandbox_strict = {
            #[cfg(target_os = "ios")]
            {
                true
            }
            #[cfg(not(target_os = "ios"))]
            {
                config.ios_sandbox_strict.unwrap_or(true)
            }
        };

        let storage_dir = config
            .app_data_dir
            .or(config.storage_dir)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if test_mode {
                    default_app_data_dir().join("test_host")
                } else {
                    default_app_data_dir()
                }
            });

        std::fs::create_dir_all(&storage_dir).map_err(|e| {
            format!(
                "Failed to create storage directory {:?}: {e}",
                storage_dir
            )
        })?;

        let host_db = Arc::new(HostSqliteStore::open(&storage_dir).map_err(|e| e.to_string())?);

        let initial_url = if let Some(ref explicit) = config.server_url {
            let trimmed = explicit.trim();
            if trimmed.is_empty() {
                DEFAULT_SERVER_URL.to_string()
            } else {
                validate_server_url_input(trimmed).map_err(|e| e.to_string())?
            }
        } else if let Ok(Some(saved)) = host_db.get_meta("server_url") {
            let trimmed = saved.trim();
            if trimmed.is_empty() {
                DEFAULT_SERVER_URL.to_string()
            } else {
                ClientConfig::normalize_url(trimmed)
            }
        } else {
            DEFAULT_SERVER_URL.to_string()
        };
        let _ = host_db.set_meta("server_url", &initial_url);

        let http_timeout_secs = config
            .http_timeout_secs
            .unwrap_or(DEFAULT_HTTP_TIMEOUT_SECS)
            .max(1);

        let client_config = ClientConfig {
            server_url: initial_url.clone(),
            http_timeout_secs,
            ..Default::default()
        };

        let client = Arc::new(
            TelegramClient::open_with_dir(client_config, &storage_dir)
                .map_err(|e| format!("Failed to open TelegramClient store: {e}"))?,
        );

        // Restore persisted PTS cursors into ClientSyncCoordinator
        if let Ok(pts_rows) = host_db.load_all_pts() {
            for (cid, pts) in pts_rows {
                client.sync_coordinator.set_pts(ChatId(cid), pts);
            }
        }

        // Open persistent Tantivy 0.22 local search index (lock-free IndexReader, inner writer lock)
        let tantivy_dir = storage_dir.join("tantivy_index");
        let search_engine =
            Arc::new(HostSearchEngine::open(&tantivy_dir).map_err(|e| e.to_string())?);

        // Open embedded Hermes/Fabushi agent runtime (SQLite WAL).
        // Under iOS sandbox policy, skip local CLI dotfile / loopback discovery and default to Safe preset.
        let hermes_dir = storage_dir.join("hermes");
        let mut hermes_cfg = HermesEngineConfig {
            workspace_root: storage_dir.clone(),
            data_dir: Some(hermes_dir),
            ..Default::default()
        };
        if ios_sandbox_strict {
            hermes_cfg.skip_local_discovery = true;
            hermes_cfg.default_preset = ToolsetPreset::Safe;
        }
        let hermes = Arc::new(
            HermesEngine::new(hermes_cfg)
                .map_err(|e| format!("Failed to open HermesEngine: {e}"))?,
        );
        if ios_sandbox_strict {
            let mut ts = hermes.toolsets().write().unwrap_or_else(|e| e.into_inner());
            for forbidden in [
                ToolsetId::Terminal,
                ToolsetId::CodeExecution,
                ToolsetId::DesktopUi,
                ToolsetId::Browser,
            ] {
                ts.disable(forbidden);
            }
        }

        // Open SQLite CodingStore
        let coding_store = Arc::new(Mutex::new(
            CodingStore::open(&storage_dir.join("coding_store.db"))
                .map_err(|e| format!("Failed to open CodingStore: {e}"))?,
        ));

        // Open SQLite MiniApp (全球法布施) storage
        let miniapp_conn = Connection::open(storage_dir.join("fabushi_miniapp.db"))
            .map_err(|e| format!("Failed to open fabushi_miniapp.db: {e}"))?;
        let _ = miniapp_conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
        fabushi_miniapp::storage::init_fabushi_tables(&miniapp_conn)
            .map_err(|e| format!("Failed to initialize fabushi_miniapp tables: {e}"))?;

        let dharma_state = Arc::new(Mutex::new(FabushiClientState::new(
            Utc::now().timestamp_millis() as f64,
        )));

        // Initialize canonical ActionRegistry with all 37+ actions
        let action_registry = Arc::new(ActionRegistry::new());
        register_full_app_actions(&action_registry)
            .map_err(|e| format!("Failed to register full app actions: {e}"))?;

        // Hydrate generation fence, next_message_id, 24h client_nonce dedup, and Magpie groups from SQLite WAL
        let initial_gen = host_db.load_generation().unwrap_or(1);
        let generation_fence = Arc::new(GenerationFence::new(initial_gen));
        let next_msg_id = host_db.load_next_message_id().unwrap_or(1001);

        let mut dedup_store = MessageDedupStore::new();
        let now = Utc::now();
        if let Ok(persisted_nonces) = host_db.load_valid_nonces(now) {
            for item in persisted_nonces {
                dedup_store.insert(item.sender_id, item.client_nonce, item, now);
            }
        }

        let mut magpie_groups = HashMap::new();
        if let Ok(persisted_groups) = host_db.kv_list("magpie_groups") {
            for (gid, raw) in persisted_groups {
                if let Ok(group) = serde_json::from_str::<RoutingGroup>(&raw) {
                    magpie_groups.insert(gid, group);
                }
            }
        }

        let mut group_workspaces = HashMap::new();
        if let Ok(persisted_ws) = host_db.kv_list("multi_agent_workspace") {
            for (cid_str, raw) in persisted_ws {
                if let (Ok(cid), Ok(ws)) = (
                    cid_str.parse::<i64>(),
                    serde_json::from_str::<SharedGroupWorkspace>(&raw),
                ) {
                    group_workspaces.insert(cid, ws);
                }
            }
        }

        let event_queue = Arc::new(EventQueue::default());
        let callback_gate = Arc::new((
            Mutex::new(CallbackGateState {
                sink: None,
                shutdown: false,
                active_threads: HashMap::new(),
            }),
            Condvar::new(),
        ));

        let (pump_control, pump_tid) = match Self::spawn_event_pump(
            Arc::downgrade(&client),
            client.outbox.subscribe_events(),
            Arc::clone(&event_queue),
            Arc::clone(&callback_gate),
            Arc::clone(&generation_fence),
            Arc::clone(&host_db),
        ) {
            Some((ctrl, tid)) => (Some(ctrl), Some(tid)),
            None => (None, None),
        };

        let storage_passphrase = config
            .storage_passphrase
            .filter(|s| !s.is_empty())
            .map(Zeroizing::new);

        Ok(Self {
            storage_dir,
            storage_passphrase,
            test_mode,
            ios_sandbox_strict,
            http_timeout_secs,
            server_url: RwLock::new(initial_url),
            client: RwLock::new(client),
            host_db,
            search_engine,
            hermes,
            coding_store,
            miniapp_db: Arc::new(Mutex::new(miniapp_conn)),
            dharma_state,
            action_registry,
            messaging_engine: Arc::new(Mutex::new(MessagingEngine::new())),
            event_queue,
            callback_gate,
            generation_fence,
            gen_lock: Mutex::new(()),
            session_lock: Mutex::new(()),
            reconfig_lock: Mutex::new(()),
            is_shutdown: AtomicBool::new(false),
            dedup_store: Arc::new(Mutex::new(dedup_store)),
            next_message_id: AtomicI64::new(next_msg_id),
            next_call_seq: AtomicU64::new(1),
            in_flight_cancels: Mutex::new(HashMap::new()),
            pump_lifecycle: (
                Mutex::new(PumpLifecycleState {
                    control: pump_control,
                    pump_tid,
                    stopping: false,
                    swapping: false,
                    shutdown: false,
                }),
                Condvar::new(),
            ),
            account_tokens: Mutex::new(HashMap::new()),
            vault_unlocked_key: Mutex::new(None),
            vault_envelope: Mutex::new(None),
            vault_device_keypairs: Mutex::new(HashMap::new()),
            task_registry: Mutex::new(HashMap::new()),
            tool_dock: Mutex::new(ToolPermissionDock::new()),
            approval_ledger: Mutex::new(ValueBoundApprovalLedger::new()),
            cua_broker: Mutex::new(ComputerLeaseBroker::new()),
            remote_hub: RwLock::new(Arc::new(RemoteControlHub::new())),
            osc52_scanner: Mutex::new(Osc52StreamScanner::default()),
            floor_queues: Mutex::new(HashMap::new()),
            loop_budgets: Mutex::new(HashMap::new()),
            group_workspaces: Mutex::new(group_workspaces),
            magpie_groups: Mutex::new(magpie_groups),
            magpie_metrics: Mutex::new(RouteRuntimeMetrics::default()),
            magpie_masking: Mutex::new(MaskingEngine::new(MaskingConfig {
                enabled: true,
                mask_secrets: true,
                mask_personal: true,
                ..Default::default()
            })),
            magpie_usage: Mutex::new(UsageLedger::default()),
            call_engine: Mutex::new(WebRtcCallEngine::new(Vec::new())),
            sfu_bridge: Mutex::new(SfuBridge::new()),
            ai_media_tap: Arc::new(DefaultAiMediaTap::new(48_000, 1, 300.0)),
            talk_coordinator: Mutex::new(TalkCoordinator::new()),
            vad_detector: Mutex::new(VadBargeInDetector::default()),
            webmcp_control_modes: Mutex::new(HashMap::new()),
            chunked_sessions: Mutex::new(HashMap::new()),
            local_dr_sessions: Mutex::new(HashMap::new()),
            local_sender_keys: Mutex::new(HashMap::new()),
        })
    }

    pub fn new(storage_dir: PathBuf, storage_passphrase: Option<String>) -> Result<Self, String> {
        Self::from_config(HostConfig {
            app_data_dir: Some(storage_dir.to_string_lossy().into_owned()),
            storage_passphrase,
            test_mode: false,
            ..Default::default()
        })
    }

    pub fn new_test(storage_dir: PathBuf) -> Result<Self, String> {
        Self::from_config(HostConfig {
            app_data_dir: Some(storage_dir.to_string_lossy().into_owned()),
            test_mode: true,
            ..Default::default()
        })
    }

    /// Spawns a lightweight dedicated non-Tokio OS thread holding `Weak<TelegramClient>` so:
    /// 1. It NEVER creates a strong `Runtime -> Task -> Arc<TelegramClient> -> Arc<Runtime>` cycle
    ///    and NEVER triggers a Tokio `Runtime::drop` panic inside a worker thread.
    /// 2. Sync SQLite PTS writes & foreign C callbacks NEVER block the 2-thread Tokio worker pool.
    /// 3. `callback_gate` mutex is ALWAYS dropped before invoking `sink.emit(...)`, while tracking
    ///    active callback thread IDs so `shutdown()` drains all in-flight callbacks before returning.
    /// 4. `stop_event_pump()` coordinates via `pump_lifecycle` Condvar and joins synchronously.
    fn spawn_event_pump(
        client_weak: Weak<TelegramClient>,
        mut outbox_rx: tokio::sync::broadcast::Receiver<OutboxStatusEvent>,
        event_queue: Arc<EventQueue>,
        callback_gate: Arc<(Mutex<CallbackGateState>, Condvar)>,
        generation_fence: Arc<GenerationFence>,
        host_db: Arc<HostSqliteStore>,
    ) -> Option<(PumpControl, ThreadId)> {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let stop_bg = Arc::clone(&stop);
        let builder = std::thread::Builder::new().name("fabushi-event-pump".to_string());
        let handle = builder
            .spawn(move || {
                loop {
                    if *stop_bg.0.lock() {
                        break;
                    }

                    // 1. Drain outbox events synchronously via broadcast::Receiver::try_recv()
                    loop {
                        if *stop_bg.0.lock() {
                            break;
                        }
                        match outbox_rx.try_recv() {
                            Ok(ob_ev) => {
                                let (ev_type, payload) = outbox_event_to_host_event(&ob_ev);
                                let generation = generation_fence.current();
                                event_queue.push(generation, &ev_type, payload.clone());
                                if *stop_bg.0.lock() {
                                    break;
                                }
                                emit_to_sink_guarded(&callback_gate, &ev_type, &payload.to_string());
                            }
                            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
                            | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                        }
                    }

                    if *stop_bg.0.lock() {
                        break;
                    }

                    // 2. Upgrade Weak<TelegramClient> inside a minimal inner scope and drop the
                    //    strong Arc BEFORE SQLite persistence, C callbacks, or Condvar wait.
                    let drained_events = {
                        let Some(client) = client_weak.upgrade() else {
                            break;
                        };
                        let events = client.poll_events();
                        let mut mapped = Vec::with_capacity(events.len());
                        for ev in events {
                            let (ev_type, payload) = domain_event_to_host_event(ev);
                            let pts_to_save = if ev_type == "newMessage" {
                                payload
                                    .get("chat_id")
                                    .and_then(|v| v.as_i64())
                                    .and_then(|cid| {
                                        let pts = client.sync_coordinator.last_pts(ChatId(cid));
                                        (pts > 0).then_some((cid, pts))
                                    })
                            } else {
                                None
                            };
                            mapped.push((ev_type, payload, pts_to_save));
                        }
                        mapped
                    };

                    if !drained_events.is_empty() {
                        let generation = generation_fence.current();
                        for (ev_type, payload, pts_to_save) in drained_events {
                            if *stop_bg.0.lock() {
                                break;
                            }
                            if let Some((cid, pts)) = pts_to_save {
                                let _ = host_db.save_pts(cid, pts);
                            }
                            event_queue.push(generation, &ev_type, payload.clone());
                            if *stop_bg.0.lock() {
                                break;
                            }
                            emit_to_sink_guarded(&callback_gate, &ev_type, &payload.to_string());
                        }
                    }

                    // 3. Sleep up to 20ms, waking immediately when shutdown signals Condvar
                    let mut stopped = stop_bg.0.lock();
                    if *stopped {
                        break;
                    }
                    let _ = stop_bg.1.wait_for(&mut stopped, Duration::from_millis(20));
                    if *stopped {
                        break;
                    }
                }
            })
            .ok()?;

        let tid = handle.thread().id();
        Some((
            PumpControl {
                stop,
                handle: Some(handle),
            },
            tid,
        ))
    }

    fn stop_event_pump(&self, mark_shutdown: bool) {
        let my_tid = std::thread::current().id();
        loop {
            let control_to_stop = {
                let mut state = self.pump_lifecycle.0.lock();
                if mark_shutdown {
                    state.shutdown = true;
                }
                if let Some(ctrl) = state.control.take() {
                    state.stopping = true;
                    Some(ctrl)
                } else if state.stopping {
                    if state.pump_tid == Some(my_tid) {
                        // Called re-entrantly from the pump thread itself while another thread is
                        // joining us; never self-deadlock on Condvar.
                        break;
                    }
                    self.pump_lifecycle.1.wait(&mut state);
                    continue;
                } else {
                    break;
                }
            };

            if let Some(mut control) = control_to_stop {
                {
                    let mut stopped = control.stop.0.lock();
                    *stopped = true;
                    control.stop.1.notify_all();
                }
                let joined = if let Some(handle) = control.handle.take() {
                    if handle.thread().id() != my_tid {
                        let _ = handle.join();
                        true
                    } else {
                        false
                    }
                } else {
                    true
                };
                let mut state = self.pump_lifecycle.0.lock();
                state.stopping = false;
                if joined {
                    state.pump_tid = None;
                }
                self.pump_lifecycle.1.notify_all();
                if !joined {
                    break;
                }
            }
        }
    }

    pub fn client(&self) -> Arc<TelegramClient> {
        Arc::clone(&self.client.read())
    }

    pub fn remote_hub(&self) -> Arc<RemoteControlHub> {
        Arc::clone(&self.remote_hub.read())
    }

    pub fn is_shutdown(&self) -> bool {
        self.is_shutdown.load(Ordering::Acquire)
    }

    pub fn current_server_url(&self) -> String {
        self.server_url.read().clone()
    }

    pub fn is_ios_sandbox(&self) -> bool {
        #[cfg(target_os = "ios")]
        {
            true
        }
        #[cfg(not(target_os = "ios"))]
        {
            self.ios_sandbox_strict
        }
    }

    pub fn require_non_ios_host_capability(&self, capability: &str) -> HostResult<()> {
        if self.is_ios_sandbox() {
            return Err(HostError::capability_unavailable(format!(
                "{capability} is unavailable in the iOS sandbox; dispatch to a remote desktop or cloud host instead"
            )));
        }
        Ok(())
    }

    pub fn current_generation(&self) -> u64 {
        self.generation_fence.current()
    }

    pub fn bump_generation(&self) -> u64 {
        let _guard = self.gen_lock.lock();
        let next = self.generation_fence.bump();
        let _ = self.host_db.save_generation(next);
        next
    }

    /// Monotonically advances the generation counter; rejects any downward regression.
    pub fn set_generation(&self, val: u64) -> HostResult<u64> {
        let _guard = self.gen_lock.lock();
        let current = self.generation_fence.current();
        if val < current {
            return Err(HostError::invalid_arguments(format!(
                "generation cannot be decreased below current generation ({val} < {current})"
            )));
        }
        let target = val.max(1);
        self.generation_fence.set(target);
        let _ = self.host_db.save_generation(target);
        Ok(target)
    }

    pub fn next_message_id(&self) -> i64 {
        let id = self.next_message_id.fetch_add(1, Ordering::SeqCst);
        let _ = self.host_db.persist_next_message_id(id + 1);
        id
    }

    /// Cancels all in-flight RPCs and wipes/zeroizes all account-scoped cryptographic keys
    /// and runtime state to prevent cross-account secret leakage across logout/clear/switch/reconfig.
    pub fn reset_account_scope(&self) {
        {
            let mut map = self.in_flight_cancels.lock();
            for (_, entries) in map.drain() {
                for (_, tok) in entries {
                    tok.cancel();
                }
            }
        }
        *self.vault_unlocked_key.lock() = None;
        *self.vault_envelope.lock() = None;
        self.vault_device_keypairs.lock().clear();
        self.local_dr_sessions.lock().clear();
        self.local_sender_keys.lock().clear();
        self.task_registry.lock().clear();
        *self.approval_ledger.lock() = ValueBoundApprovalLedger::new();
        *self.tool_dock.lock() = ToolPermissionDock::new();
        self.floor_queues.lock().clear();
        self.loop_budgets.lock().clear();
        self.group_workspaces.lock().clear();
        self.chunked_sessions.lock().clear();
        self.webmcp_control_modes.lock().clear();
        {
            let mut guard = self.magpie_masking.lock();
            let cfg = guard.config().clone();
            *guard = MaskingEngine::new(cfg);
        }
        *self.call_engine.lock() = WebRtcCallEngine::new(Vec::new());
        *self.sfu_bridge.lock() = SfuBridge::new();
        *self.talk_coordinator.lock() = TalkCoordinator::new();
        *self.cua_broker.lock() = ComputerLeaseBroker::new();
        {
            let old_hub = {
                let mut guard = self.remote_hub.write();
                std::mem::replace(&mut *guard, Arc::new(RemoteControlHub::new()))
            };
            let _ = old_hub.disconnect_all();
        }
        *self.dharma_state.lock() = FabushiClientState::new(Utc::now().timestamp_millis() as f64);
    }

    /// Checkpoints all 5 SQLite WAL databases cleanly for iOS `0xdead10cc` background & shutdown safety.
    pub fn checkpoint_all_wals(&self) -> HostResult<(i32, i32, i32)> {
        let host_ckpt = self.host_db.wal_checkpoint_truncate()?;
        let _ = self.client().local_store.wal_checkpoint_truncate();
        let _ = self.hermes.store().wal_checkpoint_truncate();
        let _ = self.coding_store.lock().wal_checkpoint_truncate();
        let _ = self
            .miniapp_db
            .lock()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        Ok(host_ckpt)
    }

    /// Atomically and serially reconfigures the backend server URL:
    /// fails closed immediately if shutting down or called re-entrantly from the event pump thread,
    /// opens `new_client` FIRST before touching live state, coordinates safely with `shutdown()`,
    /// persists `server_url` before swapping, wipes account scope, swaps cleanly, and bumps generation.
    pub fn set_server_url(&self, raw_url: &str) -> HostResult<String> {
        if self.is_shutdown.load(Ordering::Acquire) {
            return Err(HostError::host_closed());
        }
        {
            let state = self.pump_lifecycle.0.lock();
            if state.shutdown {
                return Err(HostError::host_closed());
            }
            if state.pump_tid == Some(std::thread::current().id()) {
                return Err(HostError::conflict(
                    "session.setServerUrl cannot be invoked synchronously from an event callback thread",
                ));
            }
        }
        let normalized = validate_server_url_input(raw_url)?;
        let _reconfig = self.reconfig_lock.lock();
        if self.is_shutdown.load(Ordering::Acquire) {
            return Err(HostError::host_closed());
        }
        if *self.server_url.read() == normalized {
            return Ok(normalized);
        }

        // 1. Construct and validate the new TelegramClient BEFORE mutating live host state
        let new_cfg = ClientConfig {
            server_url: normalized.clone(),
            http_timeout_secs: self.http_timeout_secs,
            ..Default::default()
        };
        let new_client = Arc::new(
            TelegramClient::open_with_dir(new_cfg, &self.storage_dir)
                .map_err(|e| HostError::storage(format!("Failed to reopen client: {e}")))?,
        );
        if let Ok(pts_rows) = self.host_db.load_all_pts() {
            for (cid, pts) in pts_rows {
                new_client.sync_coordinator.set_pts(ChatId(cid), pts);
            }
        }

        // 2. Stop & join old pump synchronously so no old callbacks/client handles remain
        self.stop_event_pump(false);

        // 3. Synchronize with shutdown before any live mutation or WAL write
        {
            let mut state = self.pump_lifecycle.0.lock();
            if state.shutdown || self.is_shutdown.load(Ordering::Acquire) {
                drop(state);
                drop(new_client);
                return Err(HostError::host_closed());
            }
            state.swapping = true;
        }
        let swapping_guard = SwappingGuard {
            lifecycle: &self.pump_lifecycle,
        };

        // 4. Persist server_url and atomically swap client/account scope under session_lock
        let old_client = self.client();
        let swap_res: HostResult<()> = (|| {
            let _sess = self.session_lock.lock();
            if self.is_shutdown.load(Ordering::Acquire) {
                return Err(HostError::host_closed());
            }
            self.host_db.set_meta("server_url", &normalized)?;
            old_client.clear_session();
            let _ = self.host_db.delete_meta("active_account_id");
            self.reset_account_scope();
            *self.client.write() = Arc::clone(&new_client);
            *self.server_url.write() = normalized.clone();
            self.bump_generation();
            Ok(())
        })();

        if let Err(err) = swap_res {
            {
                let mut state = self.pump_lifecycle.0.lock();
                if !state.shutdown && !self.is_shutdown.load(Ordering::Acquire) {
                    if let Some((restored_pump, tid)) = Self::spawn_event_pump(
                        Arc::downgrade(&old_client),
                        old_client.outbox.subscribe_events(),
                        Arc::clone(&self.event_queue),
                        Arc::clone(&self.callback_gate),
                        Arc::clone(&self.generation_fence),
                        Arc::clone(&self.host_db),
                    ) {
                        state.control = Some(restored_pump);
                        state.pump_tid = Some(tid);
                    }
                }
            }
            drop(swapping_guard);
            drop(new_client);
            return Err(err);
        }

        // 5. Launch dedicated pump for new_client ONLY if shutdown has not occurred
        {
            let mut state = self.pump_lifecycle.0.lock();
            if !state.shutdown && !self.is_shutdown.load(Ordering::Acquire) {
                if let Some((new_pump, tid)) = Self::spawn_event_pump(
                    Arc::downgrade(&new_client),
                    new_client.outbox.subscribe_events(),
                    Arc::clone(&self.event_queue),
                    Arc::clone(&self.callback_gate),
                    Arc::clone(&self.generation_fence),
                    Arc::clone(&self.host_db),
                ) {
                    state.control = Some(new_pump);
                    state.pump_tid = Some(tid);
                }
            }
        }
        drop(swapping_guard);
        drop(old_client);

        Ok(normalized)
    }

    pub fn register_event_callback(
        &self,
        callback: Option<TelegramBridgeEventCallback>,
        user_context: *mut std::ffi::c_void,
    ) {
        let my_tid = std::thread::current().id();
        let mut gate = self.callback_gate.0.lock();
        loop {
            // Clear sink before draining so no other thread can capture a stale user_context.
            gate.sink = None;
            let other_active = gate
                .active_threads
                .iter()
                .any(|(&tid, &count)| count > 0 && tid != my_tid);
            if !other_active {
                break;
            }
            self.callback_gate.1.wait(&mut gate);
        }
        if !gate.shutdown && !self.is_shutdown.load(Ordering::Acquire) {
            gate.sink = callback.map(|cb| BridgeEventSink::new(cb, user_context));
        }
    }

    pub fn unregister_event_callback(&self) {
        self.register_event_callback(None, std::ptr::null_mut());
    }

    pub fn emit_event(&self, event_type: &str, payload: Value) {
        let generation = self.current_generation();
        self.event_queue.push(generation, event_type, payload.clone());
        emit_to_sink_guarded(&self.callback_gate, event_type, &payload.to_string());
    }

    pub fn poll_events(&self, max_count: u32, timeout_ms: u32) -> Vec<HostEvent> {
        if self.is_shutdown() {
            return self.event_queue.poll(max_count as usize, 0);
        }
        // Opportunistically drain any pending WS events immediately before polling,
        // persisting PTS cursors consistently with the background event pump.
        let client = self.client();
        let drained = client.poll_events();
        if !drained.is_empty() {
            for ev in drained {
                if self.is_shutdown() {
                    break;
                }
                let (ev_type, payload) = domain_event_to_host_event(ev);
                if ev_type == "newMessage" && !self.is_shutdown() {
                    if let Some(cid) = payload.get("chat_id").and_then(|v| v.as_i64()) {
                        let pts = client.sync_coordinator.last_pts(ChatId(cid));
                        if pts > 0 && !self.is_shutdown() {
                            let _ = self.host_db.save_pts(cid, pts);
                        }
                    }
                }
                if self.is_shutdown() {
                    break;
                }
                self.emit_event(&ev_type, payload);
            }
        }
        self.event_queue.poll(max_count as usize, timeout_ms)
    }

    pub fn cancel_request(&self, request_id: &str) -> bool {
        let trimmed = request_id.trim();
        if trimmed.is_empty() || trimmed.starts_with("__anon_") {
            return false;
        }
        let tokens: Vec<CancellationToken> = self
            .in_flight_cancels
            .lock()
            .get(trimmed)
            .map(|list| list.iter().map(|(_, tok)| tok.clone()).collect())
            .unwrap_or_default();
        if tokens.is_empty() {
            false
        } else {
            for token in tokens {
                token.cancel();
            }
            true
        }
    }

    /// Runs an async operation on the single canonical `TelegramClient` Tokio runtime
    /// with cooperative `CancellationToken` support for `fabushi_host_cancel`.
    pub fn run_cancellable<F, T>(&self, request_id: &str, fut: F) -> HostResult<T>
    where
        F: Future<Output = HostResult<T>> + Send,
        T: Send,
    {
        if self.is_shutdown.load(Ordering::Acquire) {
            return Err(HostError::cancelled(request_id));
        }

        let token = CancellationToken::new();
        let call_seq = self.next_call_seq.fetch_add(1, Ordering::Relaxed);
        let tracked_id = {
            let trimmed = request_id.trim();
            let id = if trimmed.is_empty() {
                format!("__anon_{call_seq}")
            } else {
                trimmed.to_string()
            };
            self.in_flight_cancels
                .lock()
                .entry(id.clone())
                .or_default()
                .push((call_seq, token.clone()));
            id
        };
        if self.is_shutdown.load(Ordering::Acquire) {
            token.cancel();
        }
        let _guard = InFlightGuard {
            host: self,
            request_id: tracked_id,
            call_seq,
        };

        let req_id_owned = request_id.to_string();
        let client = self.client();
        let runtime = Arc::clone(client.runtime());

        let raced = async move {
            tokio::select! {
                biased;
                _ = token.cancelled() => Err(HostError::cancelled(&req_id_owned)),
                res = fut => res,
            }
        };

        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::scope(|s| {
                s.spawn(move || {
                    let _keep_client = client;
                    runtime.block_on(raced)
                })
                .join()
            })
            .map_err(|_| HostError::host_panic("Scoped async worker panicked"))?
        } else {
            runtime.block_on(raced)
        }
    }

    pub fn dispatch(&self, request_json: &str) -> String {
        crate::dispatch::route_command(self, request_json)
    }

    /// Idempotent synchronous host shutdown invoked by `fabushi_host_destroy` and `Drop`.
    /// Atomically closes the callback gate, joins the event pump, drains ALL in-flight C callbacks
    /// on other threads, waits for any in-flight `set_server_url` state swap, wipes secrets,
    /// and checkpoints all 5 SQLite WAL databases cleanly so ZERO callbacks or background writes
    /// can ever survive after `fabushi_host_destroy` returns.
    pub fn shutdown(&self) {
        // 1. Atomically close the callback gate & mark shutdown FIRST
        {
            let mut gate = self.callback_gate.0.lock();
            gate.shutdown = true;
            gate.sink = None;
            self.is_shutdown.store(true, Ordering::SeqCst);
            self.callback_gate.1.notify_all();
        }
        // 2. Stop and synchronously join the dedicated event pump thread
        self.stop_event_pump(true);
        // 3. Drain all in-flight C callbacks running on OTHER threads before returning
        {
            let my_tid = std::thread::current().id();
            let mut gate = self.callback_gate.0.lock();
            while gate
                .active_threads
                .iter()
                .any(|(&tid, &count)| count > 0 && tid != my_tid)
            {
                self.callback_gate.1.wait(&mut gate);
            }
        }
        // 4. Wait for any in-progress non-blocking state-swap in set_server_url to finish
        {
            let mut state = self.pump_lifecycle.0.lock();
            while state.swapping {
                self.pump_lifecycle.1.wait(&mut state);
            }
        }
        // 5. Wake any blocked poll_events callers cleanly
        self.event_queue.close();
        // 6. Hold session_lock across scope reset, token wipe, client teardown, and WAL checkpoint
        //    so no concurrent session mutator (auth.logout, clearSession, setToken, switchAccount,
        //    finalize_*) can write SQLite WAL after checkpoint_all_wals() returns.
        //    Lock DAG: reconfig_lock -> swapping == false -> session_lock -> leaf locks (acyclic).
        {
            let _sess_guard = self.session_lock.lock();
            self.reset_account_scope();
            self.account_tokens.lock().clear();
            let client = self.client();
            client.clear_session();
            let _ = self.checkpoint_all_wals();
        }
    }
}

impl Drop for FabushiAppHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}
