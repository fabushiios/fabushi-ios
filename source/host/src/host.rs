use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bot_browser::e2ee_sync::{DeviceSyncKeypair, UnlockedUserVaultKey, UserVaultKeyEnvelope};
use chrono::Utc;
use fabushi_messaging_core::engine::MessagingEngine;
use fabushi_miniapp::state::FabushiClientState;
use fabushi_remote_engine::control_lease::RemoteControlHub;
use fabushi_remote_engine::terminal::Osc52StreamScanner;
use gpui_telegram_agent::computer::ComputerLeaseBroker;
use gpui_telegram_agent::hermes::security::ValueBoundApprovalLedger;
use gpui_telegram_agent::mcp::ToolPermissionDock;
use gpui_telegram_agent::multi_agent::{FloorControlQueue, LoopBudget, SharedGroupWorkspace};
use gpui_telegram_agent::talk::{TalkCoordinator, VadBargeInDetector};
use gpui_telegram_agent::tasks::TaskFsm;
use gpui_telegram_agent::HermesEngine;
use gpui_telegram_client::client::TelegramClient;
use gpui_telegram_client::config::{ClientConfig, DEFAULT_HTTP_TIMEOUT_SECS, DEFAULT_SERVER_URL};
use gpui_telegram_client::e2ee::{DoubleRatchetSession, SenderKeyState};
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
use parking_lot::{Mutex, RwLock};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

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

struct InFlightGuard<'a> {
    host: &'a FabushiAppHost,
    request_id: Option<String>,
}

impl<'a> Drop for InFlightGuard<'a> {
    fn drop(&mut self) {
        if let Some(ref id) = self.request_id {
            self.host.in_flight_cancels.lock().remove(id);
        }
    }
}

pub struct FabushiAppHost {
    pub storage_dir: PathBuf,
    pub storage_passphrase: Option<String>,
    pub test_mode: bool,
    pub ios_sandbox_strict: bool,
    pub http_timeout_secs: u64,
    pub server_url: RwLock<String>,
    pub client: RwLock<Arc<TelegramClient>>,
    pub host_db: Arc<HostSqliteStore>,
    pub search_engine: Arc<Mutex<HostSearchEngine>>,
    pub hermes: Arc<HermesEngine>,
    pub coding_store: Arc<Mutex<CodingStore>>,
    pub miniapp_db: Arc<Mutex<Connection>>,
    pub dharma_state: Arc<Mutex<FabushiClientState>>,
    pub action_registry: Arc<ActionRegistry>,
    pub messaging_engine: Arc<Mutex<MessagingEngine>>,
    pub event_queue: Arc<EventQueue>,
    pub event_sink: Arc<Mutex<Option<BridgeEventSink>>>,
    pub generation_fence: Arc<GenerationFence>,
    pub dedup_store: Arc<Mutex<MessageDedupStore>>,
    pub next_message_id: AtomicI64,
    pub in_flight_cancels: Mutex<HashMap<String, CancellationToken>>,
    pump_shutdown: Mutex<CancellationToken>,
    // Domain engine state
    pub vault_unlocked_key: Mutex<Option<UnlockedUserVaultKey>>,
    pub vault_envelope: Mutex<Option<UserVaultKeyEnvelope>>,
    pub vault_device_keypairs: Mutex<HashMap<String, DeviceSyncKeypair>>,
    pub task_registry: Mutex<HashMap<String, TaskFsm>>,
    pub tool_dock: Mutex<ToolPermissionDock>,
    pub approval_ledger: Mutex<ValueBoundApprovalLedger>,
    pub cua_broker: Mutex<ComputerLeaseBroker>,
    pub remote_hub: Arc<RemoteControlHub>,
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
        let storage_dir = config
            .app_data_dir
            .or(config.storage_dir)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if config.test_mode {
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
                ClientConfig::normalize_url(trimmed)
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

        // Open persistent Tantivy 0.22 local search index
        let tantivy_dir = storage_dir.join("tantivy_index");
        let search_engine =
            Arc::new(Mutex::new(HostSearchEngine::open(&tantivy_dir).map_err(|e| e.to_string())?));

        // Open embedded Hermes/Fabushi agent runtime (SQLite WAL)
        let hermes_dir = storage_dir.join("hermes");
        let hermes = Arc::new(
            HermesEngine::open(&hermes_dir, &storage_dir)
                .map_err(|e| format!("Failed to open HermesEngine: {e}"))?,
        );

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

        // Hydrate generation fence, next_message_id, and 24h client_nonce dedup from SQLite WAL
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

        let event_queue = Arc::new(EventQueue::default());
        let event_sink: Arc<Mutex<Option<BridgeEventSink>>> = Arc::new(Mutex::new(None));

        let pump_shutdown = CancellationToken::new();
        Self::spawn_event_pump(
            Arc::clone(&client),
            Arc::clone(&event_queue),
            Arc::clone(&event_sink),
            Arc::clone(&generation_fence),
            Arc::clone(&host_db),
            pump_shutdown.clone(),
        );

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

        Ok(Self {
            storage_dir,
            storage_passphrase: config.storage_passphrase,
            test_mode: config.test_mode,
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
            event_sink,
            generation_fence,
            dedup_store: Arc::new(Mutex::new(dedup_store)),
            next_message_id: AtomicI64::new(next_msg_id),
            in_flight_cancels: Mutex::new(HashMap::new()),
            pump_shutdown: Mutex::new(pump_shutdown),
            vault_unlocked_key: Mutex::new(None),
            vault_envelope: Mutex::new(None),
            vault_device_keypairs: Mutex::new(HashMap::new()),
            task_registry: Mutex::new(HashMap::new()),
            tool_dock: Mutex::new(ToolPermissionDock::new()),
            approval_ledger: Mutex::new(ValueBoundApprovalLedger::new()),
            cua_broker: Mutex::new(ComputerLeaseBroker::new()),
            remote_hub: Arc::new(RemoteControlHub::new()),
            osc52_scanner: Mutex::new(Osc52StreamScanner::default()),
            floor_queues: Mutex::new(HashMap::new()),
            loop_budgets: Mutex::new(HashMap::new()),
            group_workspaces: Mutex::new(HashMap::new()),
            magpie_groups: Mutex::new(HashMap::new()),
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

    fn spawn_event_pump(
        client: Arc<TelegramClient>,
        event_queue: Arc<EventQueue>,
        event_sink: Arc<Mutex<Option<BridgeEventSink>>>,
        generation_fence: Arc<GenerationFence>,
        host_db: Arc<HostSqliteStore>,
        shutdown: CancellationToken,
    ) {
        let rt_handle = client.runtime().handle().clone();
        let mut outbox_rx = client.outbox.subscribe_events();
        rt_handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(15));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => {
                        break;
                    }
                    Ok(ob_ev) = outbox_rx.recv() => {
                        let (ev_type, payload) = outbox_event_to_host_event(&ob_ev);
                        let generation = generation_fence.current();
                        event_queue.push(generation, &ev_type, payload.clone());
                        if let Some(sink) = event_sink.lock().as_ref() {
                            sink.emit(&ev_type, &payload.to_string());
                        }
                    }
                    _ = interval.tick() => {
                        let events = client.poll_events();
                        if !events.is_empty() {
                            let generation = generation_fence.current();
                            for ev in events {
                                if let gpui_telegram_client::client_dispatch::ClientDomainEvent::MessageAck { pts, .. } = &ev {
                                    let _ = pts;
                                }
                                let (ev_type, payload) = domain_event_to_host_event(ev);
                                if ev_type == "newMessage" {
                                    if let Some(cid) = payload.get("chat_id").and_then(|v| v.as_i64()) {
                                        let pts = client.sync_coordinator.last_pts(ChatId(cid));
                                        if pts > 0 {
                                            let _ = host_db.save_pts(cid, pts);
                                        }
                                    }
                                }
                                event_queue.push(generation, &ev_type, payload.clone());
                                if let Some(sink) = event_sink.lock().as_ref() {
                                    sink.emit(&ev_type, &payload.to_string());
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn client(&self) -> Arc<TelegramClient> {
        Arc::clone(&self.client.read())
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
        let next = self.generation_fence.bump();
        let _ = self.host_db.save_generation(next);
        next
    }

    pub fn set_generation(&self, val: u64) -> u64 {
        let clamped = val.max(1);
        self.generation_fence.set(clamped);
        let _ = self.host_db.save_generation(clamped);
        clamped
    }

    pub fn next_message_id(&self) -> i64 {
        let id = self.next_message_id.fetch_add(1, Ordering::SeqCst);
        let _ = self.host_db.persist_next_message_id(id + 1);
        id
    }

    /// Reconfigures the backend server URL, stops the prior client/pump cleanly,
    /// opens a fresh `TelegramClient` against the same `storage_dir`, and bumps generation.
    pub fn set_server_url(&self, raw_url: &str) -> HostResult<String> {
        let trimmed = raw_url.trim();
        if trimmed.is_empty() {
            return Err(HostError::invalid_arguments("server_url cannot be empty"));
        }
        let normalized = ClientConfig::normalize_url(trimmed);
        if *self.server_url.read() == normalized {
            return Ok(normalized);
        }

        // Stop old pump & client cleanly
        {
            let old_shutdown = self.pump_shutdown.lock();
            old_shutdown.cancel();
        }
        let old_client = self.client();
        old_client.clear_session();

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

        let new_shutdown = CancellationToken::new();
        Self::spawn_event_pump(
            Arc::clone(&new_client),
            Arc::clone(&self.event_queue),
            Arc::clone(&self.event_sink),
            Arc::clone(&self.generation_fence),
            Arc::clone(&self.host_db),
            new_shutdown.clone(),
        );

        *self.pump_shutdown.lock() = new_shutdown;
        *self.client.write() = new_client;
        *self.server_url.write() = normalized.clone();
        self.host_db.set_meta("server_url", &normalized)?;
        self.bump_generation();

        Ok(normalized)
    }

    pub fn register_event_callback(
        &self,
        callback: Option<TelegramBridgeEventCallback>,
        user_context: *mut std::ffi::c_void,
    ) {
        let mut guard = self.event_sink.lock();
        *guard = callback.map(|cb| BridgeEventSink::new(cb, user_context));
    }

    pub fn unregister_event_callback(&self) {
        let mut guard = self.event_sink.lock();
        *guard = None;
    }

    pub fn emit_event(&self, event_type: &str, payload: Value) {
        let generation = self.current_generation();
        self.event_queue.push(generation, event_type, payload.clone());
        let sink_opt = self.event_sink.lock().clone();
        if let Some(sink) = sink_opt {
            sink.emit(event_type, &payload.to_string());
        }
    }

    pub fn poll_events(&self, max_count: u32, timeout_ms: u32) -> Vec<HostEvent> {
        // Opportunistically drain any pending WS events immediately before polling
        let client = self.client();
        let drained = client.poll_events();
        if !drained.is_empty() {
            for ev in drained {
                let (ev_type, payload) = domain_event_to_host_event(ev);
                self.emit_event(&ev_type, payload);
            }
        }
        self.event_queue.poll(max_count as usize, timeout_ms)
    }

    pub fn cancel_request(&self, request_id: &str) -> bool {
        let trimmed = request_id.trim();
        if trimmed.is_empty() {
            return false;
        }
        let token_opt = self.in_flight_cancels.lock().get(trimmed).cloned();
        if let Some(token) = token_opt {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Runs an async operation on the single canonical `TelegramClient` Tokio runtime
    /// with cooperative `CancellationToken` support for `fabushi_host_cancel`.
    pub fn run_cancellable<F, T>(&self, request_id: &str, fut: F) -> HostResult<T>
    where
        F: Future<Output = HostResult<T>> + Send,
        T: Send,
    {
        let token = CancellationToken::new();
        let tracked_id = if request_id.trim().is_empty() {
            None
        } else {
            let id = request_id.trim().to_string();
            self.in_flight_cancels
                .lock()
                .insert(id.clone(), token.clone());
            Some(id)
        };
        let _guard = InFlightGuard {
            host: self,
            request_id: tracked_id,
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
            std::thread::scope(|s| s.spawn(move || runtime.block_on(raced)).join())
                .map_err(|_| HostError::host_panic("Scoped async worker panicked"))?
        } else {
            runtime.block_on(raced)
        }
    }

    pub fn dispatch(&self, request_json: &str) -> String {
        crate::dispatch::route_command(self, request_json)
    }
}

impl Drop for FabushiAppHost {
    fn drop(&mut self) {
        self.pump_shutdown.lock().cancel();
        self.event_queue.close();
        for (_, tok) in self.in_flight_cancels.lock().drain() {
            tok.cancel();
        }
        let client = self.client();
        client.clear_session();
        let _ = self.host_db.wal_checkpoint_truncate();
    }
}
