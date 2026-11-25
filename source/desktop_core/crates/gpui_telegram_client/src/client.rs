use crate::cache::LocalStore;
use crate::client_dispatch::{apply_ws_frame, ClientDomainEvent};
use crate::config::ClientConfig;
use crate::e2ee::{E2eeManager, E2eeStore};
use crate::media_transfer::{ChunkedTransferManager, FileTransferStore};
use crate::net::{
    AuthResponse, BotComputerProfileDto, ClientProxyConfig, ComputerListResponse, ComputerNodeDto,
    ConnectionStatus,
    HermesApprovalDecisionRequest, HermesApprovalDto, HermesCallInviteRequest,
    HermesCallInviteResponse, HermesClient, HermesCommandDescriptor, HermesCommandRequest,
    HermesCommandResponse, HermesCreateKanbanBoardRequest, HermesCreateKanbanCardRequest,
    HermesCronJobDto, HermesHeartbeatConfigDto, HermesHonchoRequest, HermesHonchoStateDto,
    HermesKanbanBoardDto, HermesKanbanCardDto, HermesMemoryMutationRequest, HermesMemoryStateDto,
    HermesMoveKanbanCardRequest, HermesProviderConfigDto, HermesRichChatDto, HermesRichMessageDto,
    HermesRollbackRequest, HermesRollbackResponse, HermesSaveCronRequest,
    HermesSaveProviderRequest, HermesSaveSkillRequest, HermesSessionBranchRequest,
    HermesSessionDetailDto, HermesSessionDto, HermesSessionExportResponse,
    HermesSessionSearchRequest, HermesSkillDto, HermesSoulDto, HermesStatusDto,
    HermesTalkTurnRequest, HermesTalkTurnResponse, HermesTerminalProcessDto, HermesToolsetDto,
    HermesTurnRequest, HermesTurnResponse, HermesUpdateToolsetsRequest, HermesUsageSummaryDto,
    HttpClient, HttpClientError, MagpieDeleteProviderResponse, MagpieGroupsRequest,
    MagpieGroupsResponse, MagpieImportRequest, MagpieImportResponse, MagpieMaskingConfigDto,
    MagpieMaskingPreviewRequest, MagpieMaskingPreviewResponse, MagpieMaskingUpdateResponse,
    MagpieProbeRequest, MagpieProbeResponse, MagpieSaveProviderRequest,
    MagpieSaveProviderResponse, MagpieStateDto, MagpieSubscriptionsScanResponse,
    OpenAiChatCompletionsRequest, PendingComputerCommandDto,
    RegisterLocalComputerRequest, RemoteComputerCommandRequest, RemoteComputerCommandResult,
    RemoteComputerOp, TailnetEngine, TailnetIdentity, TailnetNetMap, TailnetNodeStatus,
    UpdateBotComputerRequest, UpdateComputerPolicyRequest, UpsertCloudComputerRequest,
    ApproveDeviceSyncRequest, ArchiveLocalBotReplyRequest, ArchiveLocalBotReplyResponse,
    AssistStartRequest, AssistStartResponse,
    DeviceSyncActionResponse, DeviceSyncRequestBody, DeviceSyncRequestResponse,
    DeviceSyncStatusResponse, McpOAuthGrantsResponse, PendingDeviceSyncListResponse,
    RemoteAssistConfigDto, RemoteSessionsResponse, UpsertUserVaultItemRequest,
    UpsertUserVaultItemResponse, UserVaultEnvelopeResponse, UserVaultKeyEnvelopeDto,
    UserVaultSyncPullResponse, WsChannel, WsTransport,
};
use crate::outbox::{NetworkState, NetworkStatusListener, OutboxStore, OutboxWorker, TokenProvider};
use crate::sync::ClientSyncCoordinator;
use gpui_telegram_core::error::Result;
use gpui_telegram_core::types::{ChatId, MessageId, UserId};
use gpui_telegram_proto::message::{MessageContent, MessageItem};
use gpui_telegram_proto::ws::{WsClientFrame, WsServerFrame};
use parking_lot::{Mutex, RwLock};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

pub struct TelegramClient {
    pub config: ClientConfig,
    pub local_store: LocalStore,
    pub sync_coordinator: ClientSyncCoordinator,
    pub http_client: HttpClient,
    pub hermes: HermesClient,
    pub ws_channel: WsChannel,
    pub ws_transport: WsTransport,
    pub tailnet: Arc<TailnetEngine>,
    pub outbox: Arc<OutboxWorker>,
    pub network_listener: Arc<NetworkStatusListener>,
    pub chunked_transfers: Arc<ChunkedTransferManager>,
    pub e2ee: Arc<E2eeManager>,
    runtime: Arc<Runtime>,
    auth_session: Arc<RwLock<Option<AuthResponse>>>,
    auth_token: Arc<RwLock<Option<String>>>,
    pending_events: Arc<Mutex<Vec<ClientDomainEvent>>>,
    gap_chats: Arc<Mutex<HashSet<ChatId>>>,
    next_nonce: Arc<AtomicU64>,
    next_client_msg_offset: Arc<AtomicU32>,
    ws_listener_attached: Arc<AtomicBool>,
    session_epoch: Arc<AtomicU64>,
}

impl TelegramClient {
    pub fn new(config: ClientConfig) -> Self {
        Self::with_store(config, LocalStore::new())
    }

    pub fn open_with_dir(config: ClientConfig, cache_dir: impl AsRef<Path>) -> Result<Self> {
        let store = LocalStore::open_with_dir(cache_dir)?;
        Ok(Self::with_store(config, store))
    }

    fn with_store(config: ClientConfig, local_store: LocalStore) -> Self {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("gpui-tokio-client")
                .enable_all()
                .build()
                .expect("Failed to initialize client Tokio runtime"),
        );

        let tailnet_identity = if let Some(dir) = local_store.cache_dir() {
            TailnetIdentity::load_or_create(&dir)
        } else if std::env::var_os("GPUI_TELEGRAM_CONFIG_DIR")
            .and_then(|v| (!v.to_string_lossy().trim().is_empty()).then_some(v))
            .is_some()
        {
            TailnetIdentity::load_or_create(&ClientConfig::get_app_dir())
        } else {
            TailnetIdentity::ephemeral()
        };

        let handle = runtime.handle().clone();
        let http_client = HttpClient::new_with_proxy(
            config.server_url.clone(),
            Duration::from_secs(config.http_timeout_secs),
            Some(handle.clone()),
            config.proxy.clone(),
        );
        http_client.set_device_id(tailnet_identity.device_id.clone());
        let tailnet = TailnetEngine::new(tailnet_identity);
        let hermes = HermesClient::new(http_client.clone());
        let ws_channel = WsChannel::new();
        let ws_transport = WsTransport::new_with_runtime(
            config.server_url.clone(),
            ws_channel.clone(),
            Some(handle),
        )
        .with_proxy(config.proxy.clone());

        let auth_session: Arc<RwLock<Option<AuthResponse>>> = Arc::new(RwLock::new(None));
        let auth_token: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let token_provider: TokenProvider = {
            let tok = Arc::clone(&auth_token);
            let sess = Arc::clone(&auth_session);
            Arc::new(move || {
                tok.read()
                    .clone()
                    .or_else(|| sess.read().as_ref().map(|s| s.token.clone()))
            })
        };
        let device_id_provider = {
            let hc = http_client.clone();
            Arc::new(move || hc.device_id())
        };
        let user_id_provider = {
            let sess = Arc::clone(&auth_session);
            Arc::new(move || sess.read().as_ref().map(|s| s.user_id))
        };

        let sync_coordinator = ClientSyncCoordinator::new();
        let network_listener = NetworkStatusListener::new(NetworkState::Online);
        let session_epoch = Arc::new(AtomicU64::new(0));

        let sqlite_opt = local_store.sqlite_cache();
        let outbox_store = match sqlite_opt.clone() {
            Some(db) => OutboxStore::from_cache(db),
            None => OutboxStore::in_memory().expect("Failed to open in-memory outbox store"),
        };
        let outbox = OutboxWorker::new_with_session_epoch(
            outbox_store,
            Arc::clone(&network_listener),
            http_client.clone(),
            Arc::clone(&token_provider),
            local_store.clone(),
            sync_coordinator.clone(),
            Arc::clone(&session_epoch),
        );

        let transfer_store = match sqlite_opt.clone() {
            Some(db) => FileTransferStore::from_cache(db),
            None => FileTransferStore::in_memory().expect("Failed to open in-memory transfer store"),
        };
        let chunked_transfers = ChunkedTransferManager::new_with_session_epoch(
            transfer_store,
            http_client.clone(),
            Arc::clone(&token_provider),
            Arc::clone(&session_epoch),
        );

        let e2ee_store = match sqlite_opt {
            Some(db) => E2eeStore::from_cache(db),
            None => E2eeStore::in_memory(),
        };
        let e2ee = E2eeManager::new_with_session_epoch(
            e2ee_store,
            http_client.clone(),
            token_provider,
            device_id_provider,
            user_id_provider,
            Arc::clone(&session_epoch),
        );

        {
            let outbox_ref = Arc::clone(&outbox);
            let transfers_ref = Arc::clone(&chunked_transfers);
            let e2ee_ref = Arc::clone(&e2ee);
            let sync_ref = sync_coordinator.clone();
            local_store.register_clear_hook(Arc::new(move || {
                outbox_ref.clear_session_state();
                transfers_ref.clear_session_state();
                e2ee_ref.store().clear();
                sync_ref.clear();
            }));
        }

        let now_ms = chrono::Utc::now().timestamp_millis().max(1) as u64;
        let persisted_max = local_store.max_outbox_nonce();
        let safe_max = (i64::MAX as u64).saturating_sub(1_000_000);
        let seed_nonce = now_ms
            .min(safe_max)
            .max(persisted_max.saturating_add(1))
            .min(i64::MAX as u64);

        Self {
            config,
            local_store,
            sync_coordinator,
            http_client,
            hermes,
            ws_channel,
            ws_transport,
            tailnet,
            outbox,
            network_listener,
            chunked_transfers,
            e2ee,
            runtime,
            auth_session,
            auth_token,
            pending_events: Arc::new(Mutex::new(Vec::new())),
            gap_chats: Arc::new(Mutex::new(HashSet::new())),
            next_nonce: Arc::new(AtomicU64::new(seed_nonce)),
            next_client_msg_offset: Arc::new(AtomicU32::new(1)),
            ws_listener_attached: Arc::new(AtomicBool::new(false)),
            session_epoch,
        }
    }

    pub fn new_with_url(server_url: String) -> Self {
        let config = ClientConfig {
            server_url,
            ..Default::default()
        };
        Self::new(config)
    }

    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }

    pub fn outbox_worker(&self) -> &Arc<OutboxWorker> {
        &self.outbox
    }

    pub fn chunked_transfer_manager(&self) -> &Arc<ChunkedTransferManager> {
        &self.chunked_transfers
    }

    pub fn e2ee_manager(&self) -> &Arc<E2eeManager> {
        &self.e2ee
    }

    pub fn apply_proxy(
        &self,
        proxy: Option<ClientProxyConfig>,
    ) -> std::result::Result<(), HttpClientError> {
        self.http_client.set_proxy(proxy.clone())?;
        self.ws_transport.set_proxy(proxy);
        Ok(())
    }

    pub fn device_id(&self) -> String {
        self.http_client.device_id()
    }

    pub fn set_device_id(&self, id: impl Into<String>) {
        let trimmed = id.into().trim().to_string();
        if !trimmed.is_empty() {
            self.http_client.set_device_id(&trimmed);
            self.tailnet.set_device_id_override(Some(trimmed));
        }
    }

    pub fn bind_persistent_identity_dir(&self, dir: impl AsRef<Path>) {
        let prev_device_id = self.device_id();
        let was_running = self.tailnet.is_running();
        let id = TailnetIdentity::load_or_create(dir.as_ref());
        let next_device_id = id.device_id.clone();
        self.http_client.set_device_id(next_device_id.clone());
        self.tailnet.bind_identity(id);
        if was_running && prev_device_id != next_device_id {
            self.tailnet.stop();
            self.start_tailnet_mesh();
        }
    }

    pub fn start_tailnet_mesh(&self) {
        let Some((expected_epoch, token, user_id)) = self.snapshot_pinned_tailnet_session() else {
            return;
        };
        self.tailnet.start_if(
            self.config.server_url.clone(),
            token.clone(),
            user_id,
            self.http_client.clone(),
            self.runtime.handle().clone(),
            || self.ensure_pinned_session(expected_epoch, &token).is_ok(),
        );
    }

    pub fn stop_tailnet_mesh(&self) {
        self.tailnet.stop();
    }

    pub fn tailnet_status(&self) -> TailnetNodeStatus {
        self.tailnet.status()
    }

    pub fn tailnet_pending_approvals(&self) -> Vec<PendingComputerCommandDto> {
        self.tailnet.pending_approvals()
    }

    pub fn decide_tailnet_approval_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        command_id: &str,
        approved: bool,
    ) -> Option<std::result::Result<(), String>> {
        if let Err(e) = self.ensure_pinned_session(expected_epoch, token) {
            return Some(Err(e.to_string()));
        }
        self.tailnet.decide_approval(command_id, approved)
    }

    pub fn decide_tailnet_approval_async<'a>(
        &'a self,
        command_id: &'a str,
        approved: bool,
    ) -> impl std::future::Future<Output = Option<std::result::Result<(), String>>> + Send + 'a {
        let start_epoch = self.session_epoch();
        let start_token = self.token();
        async move {
            if let Some(ref tok) = start_token {
                if self.ensure_pinned_session(start_epoch, tok).is_err() {
                    return Some(Err("Session changed before tailnet approval".to_string()));
                }
            } else if !self.is_same_session_epoch(start_epoch) {
                return Some(Err("Session changed before tailnet approval".to_string()));
            }
            self.tailnet.decide_approval(command_id, approved)
        }
    }

    pub fn set_tailnet_force_derp_only(&self, force: bool) {
        self.tailnet.set_force_derp_only(force);
    }

    fn apply_tailnet_netmap_if_pinned(
        &self,
        expected_epoch: u64,
        token: &str,
        netmap: TailnetNetMap,
    ) -> bool {
        self.tailnet.apply_netmap_if(netmap, || {
            self.ensure_pinned_session(expected_epoch, token).is_ok()
        })
    }

    pub async fn register_tailnet_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<TailnetNetMap> {
        self.register_tailnet_inner(token, expected_epoch, self.current_user_id())
            .await
    }

    async fn register_tailnet_inner(
        &self,
        token: &str,
        expected_epoch: u64,
        captured_user_id: Option<u64>,
    ) -> Result<TailnetNetMap> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let user_id = match captured_user_id {
            Some(uid) if uid > 0 => uid,
            _ => {
                let uid = self.http_client.get_me(token).await?.user_id;
                self.ensure_pinned_session(expected_epoch, token)?;
                uid
            }
        };
        let req = self.tailnet.build_register_request(user_id);
        let netmap = self.http_client.register_tailnet(token, &req).await?;
        let _ = self.apply_tailnet_netmap_if_pinned(expected_epoch, token, netmap.clone());
        Ok(netmap)
    }

    pub fn register_tailnet_async(
        &self,
    ) -> impl std::future::Future<Output = Result<TailnetNetMap>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        let captured_uid = self.current_user_id();
        async move {
            let (expected_epoch, token) = pinned?;
            self.register_tailnet_inner(&token, expected_epoch, captured_uid)
                .await
        }
    }

    pub async fn get_tailnet_netmap_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<TailnetNetMap> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let netmap = self.http_client.get_tailnet_netmap(token).await?;
        let _ = self.apply_tailnet_netmap_if_pinned(expected_epoch, token, netmap.clone());
        Ok(netmap)
    }

    pub fn get_tailnet_netmap_async(
        &self,
    ) -> impl std::future::Future<Output = Result<TailnetNetMap>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_tailnet_netmap_with_token(&token, expected_epoch)
                .await
        }
    }

    fn block_on_http<F, T>(&self, fut: F) -> Result<T>
    where
        F: std::future::Future<Output = std::result::Result<T, HttpClientError>> + Send,
        T: Send,
    {
        let res = if tokio::runtime::Handle::try_current().is_ok() {
            let runtime = Arc::clone(&self.runtime);
            std::thread::scope(|scope| scope.spawn(move || runtime.block_on(fut)).join()).map_err(
                |_| {
                    gpui_telegram_core::error::TelegramError::Internal(
                        "Client runtime scope thread panicked".to_string(),
                    )
                },
            )?
        } else {
            self.runtime.block_on(fut)
        };
        res.map_err(Into::into)
    }

    pub fn status(&self) -> ConnectionStatus {
        self.ws_transport.status()
    }

    pub fn is_connected(&self) -> bool {
        self.ws_transport.is_connected()
    }

    pub fn session_epoch(&self) -> u64 {
        self.session_epoch.load(Ordering::SeqCst)
    }

    pub fn bump_session_epoch(&self) -> u64 {
        self.session_epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn is_same_session_epoch(&self, expected_epoch: u64) -> bool {
        self.session_epoch.load(Ordering::SeqCst) == expected_epoch
    }

    pub fn snapshot_pinned_session(&self) -> std::result::Result<(u64, String), HttpClientError> {
        let tok = self.auth_token.read();
        let sess = self.auth_session.read();
        let epoch = self.session_epoch.load(Ordering::SeqCst);
        let token = tok
            .clone()
            .or_else(|| sess.as_ref().map(|s| s.token.clone()))
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| HttpClientError::Unauthorized("No active session token".to_string()))?;
        Ok((epoch, token))
    }

    fn snapshot_pinned_tailnet_session(&self) -> Option<(u64, String, Option<u64>)> {
        let tok = self.auth_token.read();
        let sess = self.auth_session.read();
        let epoch = self.session_epoch.load(Ordering::SeqCst);
        let token = tok
            .clone()
            .or_else(|| sess.as_ref().map(|s| s.token.clone()))
            .filter(|t| !t.trim().is_empty())?;
        let user_id = sess.as_ref().map(|s| s.user_id);
        Some((epoch, token, user_id))
    }

    pub fn ensure_pinned_session(
        &self,
        expected_epoch: u64,
        expected_token: &str,
    ) -> std::result::Result<(), HttpClientError> {
        if expected_token.trim().is_empty() {
            return Err(HttpClientError::Unauthorized(
                "Session changed before pinned request".to_string(),
            ));
        }
        let tok = self.auth_token.read();
        let sess = self.auth_session.read();
        if self.session_epoch.load(Ordering::SeqCst) != expected_epoch {
            return Err(HttpClientError::Unauthorized(
                "Session changed before pinned request".to_string(),
            ));
        }
        let cur = tok
            .as_deref()
            .or_else(|| sess.as_ref().map(|s| s.token.as_str()));
        if cur.is_some_and(|t| !t.trim().is_empty() && t != expected_token) {
            return Err(HttpClientError::Unauthorized(
                "Session token changed before pinned request".to_string(),
            ));
        }
        Ok(())
    }

    pub fn set_auth_session(&self, session: AuthResponse) {
        let prev_user_id = self.current_user_id();
        let is_user_switch =
            prev_user_id.is_some_and(|prev| prev != session.user_id && session.user_id > 0);
        if is_user_switch {
            self.clear_session();
        }
        self.local_store
            .bind_account_scope(&self.config.server_url, session.user_id);
        {
            let mut tok_guard = self.auth_token.write();
            let mut sess_guard = self.auth_session.write();
            let cur_uid = sess_guard.as_ref().map(|s| s.user_id);
            let cur_tok = tok_guard
                .as_deref()
                .or_else(|| sess_guard.as_ref().map(|s| s.token.as_str()));
            if cur_uid != Some(session.user_id) || cur_tok != Some(session.token.as_str()) {
                self.bump_session_epoch();
            }
            *tok_guard = Some(session.token.clone());
            *sess_guard = Some(session.clone());
            self.ws_transport.set_token(Some(session.token));
        }
        self.outbox.start(self.runtime.handle());
    }

    pub fn set_token(&self, token: Option<String>) {
        let mut tok_guard = self.auth_token.write();
        let sess_guard = self.auth_session.read();
        let prev = tok_guard
            .as_deref()
            .or_else(|| sess_guard.as_ref().map(|s| s.token.as_str()));
        if prev != token.as_deref() {
            self.bump_session_epoch();
        }
        *tok_guard = token;
        let effective_token = tok_guard
            .clone()
            .or_else(|| sess_guard.as_ref().map(|s| s.token.clone()));
        self.ws_transport.set_token(effective_token);
    }

    pub fn token(&self) -> Option<String> {
        self.auth_token
            .read()
            .clone()
            .or_else(|| self.auth_session.read().as_ref().map(|s| s.token.clone()))
    }

    pub fn current_auth_session(&self) -> Option<AuthResponse> {
        self.auth_session.read().clone()
    }

    pub fn current_user_id(&self) -> Option<u64> {
        self.auth_session.read().as_ref().map(|s| s.user_id)
    }

    pub fn clear_session(&self) {
        {
            let mut tok_guard = self.auth_token.write();
            let mut sess_guard = self.auth_session.write();
            self.bump_session_epoch();
            *tok_guard = None;
            *sess_guard = None;
            self.ws_transport.set_token(None);
        }
        self.outbox.clear_session_state();
        self.chunked_transfers.clear_session_state();
        self.e2ee.store().clear();
        self.sync_coordinator.clear();
        self.pending_events.lock().clear();
        self.gap_chats.lock().clear();
        self.ws_transport.stop();
        self.stop_tailnet_mesh();
    }

    /// Stops background workers and clears in-memory auth credentials without wiping
    /// persistent SQLite `outbox_messages`, `file_transfers`, or `e2ee_*` tables.
    /// Used on graceful app shutdown and same-account re-authentication so offline
    /// queued outbox messages and E2EE identities survive restarts (B6-01/B6-02).
    pub fn suspend_session_preserving_stores(&self) {
        {
            let mut tok_guard = self.auth_token.write();
            let mut sess_guard = self.auth_session.write();
            self.bump_session_epoch();
            *tok_guard = None;
            *sess_guard = None;
            self.ws_transport.set_token(None);
        }
        self.outbox.stop();
        self.pending_events.lock().clear();
        self.gap_chats.lock().clear();
        self.ws_transport.stop();
        self.stop_tailnet_mesh();
    }

    pub(crate) fn require_token(&self) -> std::result::Result<String, HttpClientError> {
        self.token()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| HttpClientError::Unauthorized("No active session token".to_string()))
    }

    pub fn connect_ws(&self) {
        if !self.ws_listener_attached.swap(true, Ordering::SeqCst) {
            self.network_listener
                .attach_ws_status(self.ws_transport.subscribe_status(), self.runtime.handle());
        }
        self.outbox.start(self.runtime.handle());
        self.ws_transport.start();
    }

    pub fn disconnect_ws(&self) {
        self.ws_transport.stop();
    }

    pub fn send_ws_frame(&self, frame: WsClientFrame) -> bool {
        self.ws_channel.send(frame)
    }

    pub fn next_client_nonce(&self) -> u64 {
        self.next_nonce.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn next_provisional_msg_id(&self) -> MessageId {
        let offset = self.next_client_msg_offset.fetch_add(1, Ordering::Relaxed);
        gpui_telegram_core::types::MsgId::generate_client(offset)
    }

    pub fn get_messages(&self, chat_id: ChatId) -> Result<Vec<MessageItem>> {
        self.local_store.get_messages(chat_id)
    }

    fn handle_e2ee_side_effects(&self, events: &[ClientDomainEvent]) {
        for ev in events {
            if let ClientDomainEvent::NewMessage(msg) = ev {
                if let MessageContent::Encrypted(ref env) = msg.content {
                    if matches!(
                        env.kind,
                        gpui_telegram_proto::e2ee::EncryptedPayloadKind::SenderKeyDistribution { .. }
                    ) {
                        let _ = self.e2ee.decrypt_envelope(env);
                    }
                }
            }
        }
    }

    pub fn process_ws_frame(&self, frame: WsServerFrame) -> Vec<ClientDomainEvent> {
        let mut events = Vec::new();
        let viewer = self.current_user_id().map(UserId);
        let gaps = apply_ws_frame(
            &self.local_store,
            &self.sync_coordinator,
            viewer,
            frame,
            &mut events,
        );
        self.handle_e2ee_side_effects(&events);
        if !gaps.is_empty() {
            self.gap_chats.lock().extend(gaps);
        }
        if !events.is_empty() {
            self.pending_events.lock().extend(events.iter().cloned());
        }
        events
    }

    pub fn poll_events(&self) -> Vec<ClientDomainEvent> {
        let viewer = self.current_user_id().map(UserId);
        while let Some(frame) = self.ws_channel.try_recv() {
            let mut batch = Vec::new();
            let gaps = apply_ws_frame(
                &self.local_store,
                &self.sync_coordinator,
                viewer,
                frame,
                &mut batch,
            );
            self.handle_e2ee_side_effects(&batch);
            if !gaps.is_empty() {
                self.gap_chats.lock().extend(gaps);
            }
            if !batch.is_empty() {
                self.pending_events.lock().extend(batch);
            }
        }
        std::mem::take(&mut *self.pending_events.lock())
    }

    pub fn pending_gap_chats(&self) -> Vec<ChatId> {
        self.gap_chats.lock().iter().copied().collect()
    }

    pub(crate) fn clear_gap_chat(&self, chat_id: ChatId) {
        self.gap_chats.lock().remove(&chat_id);
    }

    pub fn hermes(&self) -> &HermesClient {
        &self.hermes
    }

    pub fn hermes_status(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<HermesStatusDto, HttpClientError>>
           + Send
           + '_ {
        self.hermes_status_for_bot(None)
    }

    pub fn hermes_status_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<Output = std::result::Result<HermesStatusDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.status_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_run_turn<'a>(
        &'a self,
        req: &'a HermesTurnRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesTurnResponse, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.run_turn(&token, req).await
        }
    }

    pub fn hermes_run_command<'a>(
        &'a self,
        req: &'a HermesCommandRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesCommandResponse, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.run_command(&token, req).await
        }
    }

    pub fn hermes_list_commands<'a>(
        &'a self,
        prefix: Option<&'a str>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesCommandDescriptor>, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.list_commands(&token, prefix).await
        }
    }

    pub fn hermes_list_sessions(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesSessionDto>, HttpClientError>,
    > + Send
           + '_ {
        self.hermes_list_sessions_for_bot(None)
    }

    pub fn hermes_list_sessions_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesSessionDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.list_sessions_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_get_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesSessionDetailDto, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.get_session(&token, session_id).await
        }
    }

    pub fn hermes_search_sessions<'a>(
        &'a self,
        req: &'a HermesSessionSearchRequest,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<serde_json::Value>, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.search_sessions(&token, req).await
        }
    }

    pub fn hermes_branch_session<'a>(
        &'a self,
        session_id: &'a str,
        req: &'a HermesSessionBranchRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesSessionDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.branch_session(&token, session_id, req).await
        }
    }

    pub fn hermes_export_session<'a>(
        &'a self,
        session_id: &'a str,
        format: Option<&'a str>,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesSessionExportResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.export_session(&token, session_id, format).await
        }
    }

    pub fn hermes_get_memory(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<HermesMemoryStateDto, HttpClientError>>
           + Send
           + '_ {
        self.hermes_get_memory_for_bot(None)
    }

    pub fn hermes_get_memory_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<Output = std::result::Result<HermesMemoryStateDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.get_memory_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_save_memory_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        memory_markdown: &'a str,
        user_markdown: Option<&'a str>,
    ) -> impl std::future::Future<Output = std::result::Result<HermesMemoryStateDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .save_memory_for_bot(&token, bot_id, memory_markdown, user_markdown)
                .await
        }
    }

    pub fn hermes_mutate_memory<'a>(
        &'a self,
        req: &'a HermesMemoryMutationRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesMemoryStateDto, HttpClientError>>
           + Send
           + 'a {
        self.hermes_mutate_memory_for_bot(None, req)
    }

    pub fn hermes_mutate_memory_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        req: &'a HermesMemoryMutationRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesMemoryStateDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.mutate_memory_for_bot(&token, bot_id, req).await
        }
    }

    pub fn hermes_get_soul(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<Output = std::result::Result<HermesSoulDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.get_soul(&token, bot_id).await
        }
    }

    pub fn hermes_save_soul<'a>(
        &'a self,
        soul: &'a HermesSoulDto,
    ) -> impl std::future::Future<Output = std::result::Result<HermesSoulDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.save_soul(&token, soul).await
        }
    }

    pub fn hermes_get_honcho(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<HermesHonchoStateDto, HttpClientError>>
           + Send
           + '_ {
        self.hermes_get_honcho_for_bot(None)
    }

    pub fn hermes_get_honcho_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<Output = std::result::Result<HermesHonchoStateDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.get_honcho_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_post_honcho<'a>(
        &'a self,
        req: &'a HermesHonchoRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesHonchoStateDto, HttpClientError>>
           + Send
           + 'a {
        self.hermes_post_honcho_for_bot(None, req)
    }

    pub fn hermes_post_honcho_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        req: &'a HermesHonchoRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesHonchoStateDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.post_honcho_for_bot(&token, bot_id, req).await
        }
    }

    pub fn hermes_list_skills(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<HermesSkillDto>, HttpClientError>>
           + Send
           + '_ {
        self.hermes_list_skills_for_bot(None)
    }

    pub fn hermes_list_skills_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<HermesSkillDto>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.list_skills_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_save_skill<'a>(
        &'a self,
        req: &'a HermesSaveSkillRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesSkillDto, HttpClientError>>
           + Send
           + 'a {
        self.hermes_save_skill_for_bot(None, req)
    }

    pub fn hermes_save_skill_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        req: &'a HermesSaveSkillRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesSkillDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.save_skill_for_bot(&token, bot_id, req).await
        }
    }

    pub fn hermes_delete_skill<'a>(
        &'a self,
        name: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        self.hermes_delete_skill_for_bot(None, name)
    }

    pub fn hermes_delete_skill_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        name: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.delete_skill_for_bot(&token, bot_id, name).await
        }
    }

    pub fn hermes_patch_skill_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        name: &'a str,
        patch: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<HermesSkillDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .patch_skill_for_bot(&token, bot_id, name, patch)
                .await
        }
    }

    pub fn hermes_approve_skill_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        name: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<HermesSkillDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .approve_skill_for_bot(&token, bot_id, name)
                .await
        }
    }

    pub fn hermes_list_toolsets(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesToolsetDto>, HttpClientError>,
    > + Send
           + '_ {
        self.hermes_list_toolsets_for_bot(None)
    }

    pub fn hermes_list_toolsets_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesToolsetDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.list_toolsets_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_update_toolsets<'a>(
        &'a self,
        req: &'a HermesUpdateToolsetsRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesToolsetDto>, HttpClientError>,
    > + Send
           + 'a {
        self.hermes_update_toolsets_for_bot(None, req)
    }

    pub fn hermes_update_toolsets_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        req: &'a HermesUpdateToolsetsRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesToolsetDto>, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .update_toolsets_for_bot(&token, bot_id, req)
                .await
        }
    }

    pub fn hermes_list_approvals(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesApprovalDto>, HttpClientError>,
    > + Send
           + '_ {
        self.hermes_list_approvals_for_bot(None)
    }

    pub fn hermes_list_approvals_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesApprovalDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.list_approvals_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_decide_approval<'a>(
        &'a self,
        approval_id: &'a str,
        req: &'a HermesApprovalDecisionRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesApprovalDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.decide_approval(&token, approval_id, req).await
        }
    }

    pub fn hermes_list_cron(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesCronJobDto>, HttpClientError>,
    > + Send
           + '_ {
        self.hermes_list_cron_for_bot(None)
    }

    pub fn hermes_list_cron_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesCronJobDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.list_cron_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_save_cron<'a>(
        &'a self,
        req: &'a HermesSaveCronRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesCronJobDto, HttpClientError>>
           + Send
           + 'a {
        self.hermes_save_cron_for_bot(None, req)
    }

    pub fn hermes_save_cron_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        req: &'a HermesSaveCronRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesCronJobDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.save_cron_for_bot(&token, bot_id, req).await
        }
    }

    pub fn hermes_update_cron_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        job_id: &'a str,
        patch: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<HermesCronJobDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .update_cron_for_bot(&token, bot_id, job_id, patch)
                .await
        }
    }

    pub fn hermes_delete_cron<'a>(
        &'a self,
        job_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        self.hermes_delete_cron_for_bot(None, job_id)
    }

    pub fn hermes_delete_cron_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        job_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .delete_cron_for_bot(&token, bot_id, job_id)
                .await
        }
    }

    pub fn hermes_run_cron<'a>(
        &'a self,
        job_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        self.hermes_run_cron_for_bot(None, job_id)
    }

    pub fn hermes_run_cron_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        job_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.run_cron_for_bot(&token, bot_id, job_id).await
        }
    }

    pub fn hermes_get_heartbeat(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesHeartbeatConfigDto, HttpClientError>,
    > + Send
           + '_ {
        self.hermes_get_heartbeat_for_bot(None)
    }

    pub fn hermes_get_heartbeat_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesHeartbeatConfigDto, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.get_heartbeat_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_save_heartbeat<'a>(
        &'a self,
        cfg: &'a HermesHeartbeatConfigDto,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesHeartbeatConfigDto, HttpClientError>,
    > + Send
           + 'a {
        self.hermes_save_heartbeat_for_bot(None, cfg)
    }

    pub fn hermes_save_heartbeat_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        cfg: &'a HermesHeartbeatConfigDto,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesHeartbeatConfigDto, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .save_heartbeat_for_bot(&token, bot_id, cfg)
                .await
        }
    }

    pub fn hermes_kanban_boards(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesKanbanBoardDto>, HttpClientError>,
    > + Send
           + '_ {
        self.hermes_kanban_boards_for_bot(None)
    }

    pub fn hermes_kanban_boards_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesKanbanBoardDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.kanban_boards_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_kanban_create_board<'a>(
        &'a self,
        req: &'a HermesCreateKanbanBoardRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesKanbanBoardDto, HttpClientError>>
           + Send
           + 'a {
        self.hermes_kanban_create_board_for_bot(None, req)
    }

    pub fn hermes_kanban_create_board_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        req: &'a HermesCreateKanbanBoardRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesKanbanBoardDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .kanban_create_board_for_bot(&token, bot_id, req)
                .await
        }
    }

    pub fn hermes_kanban_create_card<'a>(
        &'a self,
        req: &'a HermesCreateKanbanCardRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesKanbanCardDto, HttpClientError>>
           + Send
           + 'a {
        self.hermes_kanban_create_card_for_bot(None, req)
    }

    pub fn hermes_kanban_create_card_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        req: &'a HermesCreateKanbanCardRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesKanbanCardDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .kanban_create_card_for_bot(&token, bot_id, req)
                .await
        }
    }

    pub fn hermes_kanban_move_card<'a>(
        &'a self,
        card_id: &'a str,
        req: &'a HermesMoveKanbanCardRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesKanbanCardDto, HttpClientError>>
           + Send
           + 'a {
        self.hermes_kanban_move_card_for_bot(None, card_id, req)
    }

    pub fn hermes_kanban_move_card_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        card_id: &'a str,
        req: &'a HermesMoveKanbanCardRequest,
    ) -> impl std::future::Future<Output = std::result::Result<HermesKanbanCardDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .kanban_move_card_for_bot(&token, bot_id, card_id, req)
                .await
        }
    }

    pub fn hermes_kanban_delete_card_for_bot<'a>(
        &'a self,
        bot_id: Option<u64>,
        card_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .kanban_delete_card_for_bot(&token, bot_id, card_id)
                .await
        }
    }

    pub fn hermes_usage_stats(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesUsageSummaryDto, HttpClientError>,
    > + Send
           + '_ {
        self.hermes_usage_stats_for_bot(None)
    }

    pub fn hermes_usage_stats_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesUsageSummaryDto, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.usage_stats_for_bot(&token, bot_id).await
        }
    }

    pub fn hermes_list_providers(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesProviderConfigDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.list_providers(&token).await
        }
    }

    pub fn hermes_save_provider<'a>(
        &'a self,
        req: &'a HermesSaveProviderRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesProviderConfigDto, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.save_provider(&token, req).await
        }
    }

    pub fn hermes_rollback_turn<'a>(
        &'a self,
        req: &'a HermesRollbackRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesRollbackResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.rollback_turn(&token, req).await
        }
    }

    pub fn hermes_terminal_processes(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesTerminalProcessDto>, HttpClientError>,
    > + Send
           + '_ {
        self.hermes_terminal_processes_for_bot(None)
    }

    pub fn hermes_terminal_processes_for_bot(
        &self,
        bot_id: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesTerminalProcessDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes
                .terminal_processes_for_bot(&token, bot_id)
                .await
        }
    }

    pub fn hermes_chat_completions<'a>(
        &'a self,
        req: &'a OpenAiChatCompletionsRequest,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.chat_completions(&token, req).await
        }
    }

    pub fn hermes_talk_turn<'a>(
        &'a self,
        req: &'a HermesTalkTurnRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesTalkTurnResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.hermes_talk_turn_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub async fn hermes_talk_turn_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &HermesTalkTurnRequest,
    ) -> std::result::Result<HermesTalkTurnResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.talk_turn(token, req).await
    }

    pub fn hermes_call_invite<'a>(
        &'a self,
        req: &'a HermesCallInviteRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<HermesCallInviteResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.hermes_call_invite_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub async fn hermes_call_invite_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &HermesCallInviteRequest,
    ) -> std::result::Result<HermesCallInviteResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.call_invite(token, req).await
    }

    pub fn hermes_list_chats_rich(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesRichChatDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.list_chats_rich(&token).await
        }
    }

    pub fn hermes_get_chat_rich(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<HermesRichChatDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.hermes.get_chat_rich(&token, chat_id).await
        }
    }

    pub fn hermes_messages_history_rich(
        &self,
        chat_id: i64,
        limit: Option<usize>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HermesRichMessageDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.hermes_messages_history_rich_with_token(&token, expected_epoch, chat_id, limit)
                .await
        }
    }

    pub async fn hermes_messages_history_rich_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        limit: Option<usize>,
    ) -> std::result::Result<Vec<HermesRichMessageDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.messages_history_rich(token, chat_id, limit).await
    }

    pub async fn hermes_magpie_state_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<MagpieStateDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_state(token).await
    }

    pub async fn hermes_magpie_save_provider_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &MagpieSaveProviderRequest,
    ) -> std::result::Result<MagpieSaveProviderResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_save_provider(token, req).await
    }

    pub async fn hermes_magpie_delete_provider_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        provider_id: &str,
    ) -> std::result::Result<MagpieDeleteProviderResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_delete_provider(token, provider_id).await
    }

    pub async fn hermes_magpie_probe_provider_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        provider_id: &str,
        req: Option<&MagpieProbeRequest>,
    ) -> std::result::Result<MagpieProbeResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes
            .magpie_probe_provider(token, provider_id, req)
            .await
    }

    pub async fn hermes_magpie_scan_subscriptions_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<MagpieSubscriptionsScanResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_scan_subscriptions(token).await
    }

    pub async fn hermes_magpie_upsert_group_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &MagpieGroupsRequest,
    ) -> std::result::Result<MagpieGroupsResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_upsert_group(token, req).await
    }

    pub async fn hermes_magpie_delete_group_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        group_id: &str,
    ) -> std::result::Result<MagpieGroupsResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_delete_group(token, group_id).await
    }

    pub async fn hermes_magpie_update_masking_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        cfg: &MagpieMaskingConfigDto,
    ) -> std::result::Result<MagpieMaskingUpdateResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_update_masking(token, cfg).await
    }

    pub async fn hermes_magpie_preview_masking_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &MagpieMaskingPreviewRequest,
    ) -> std::result::Result<MagpieMaskingPreviewResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_preview_masking(token, req).await
    }

    pub async fn hermes_magpie_import_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &MagpieImportRequest,
    ) -> std::result::Result<MagpieImportResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.hermes.magpie_import(token, req).await
    }

    pub fn list_computers(&self) -> Result<ComputerListResponse> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.list_computers(&token).await })
    }

    pub fn list_computers_async(
        &self,
    ) -> impl std::future::Future<Output = Result<ComputerListResponse>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_computers_with_token(&token, expected_epoch).await
        }
    }

    pub fn register_local_computer(
        &self,
        req: &RegisterLocalComputerRequest,
    ) -> Result<ComputerNodeDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.register_local_computer(&token, req).await })
    }

    pub async fn register_local_computer_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &RegisterLocalComputerRequest,
    ) -> Result<ComputerNodeDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.register_local_computer(token, req).await?)
    }

    pub fn register_local_computer_async<'a>(
        &'a self,
        req: &'a RegisterLocalComputerRequest,
    ) -> impl std::future::Future<Output = Result<ComputerNodeDto>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.register_local_computer_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub fn register_current_host_computer(&self) -> Result<ComputerNodeDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.register_current_host_computer(&token).await })
    }

    pub fn register_current_host_computer_async(
        &self,
    ) -> impl std::future::Future<Output = Result<ComputerNodeDto>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.register_current_host_computer_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn poll_local_computer_commands(&self) -> Result<Vec<PendingComputerCommandDto>> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.poll_local_computer_commands(&token).await })
    }

    pub async fn poll_local_computer_commands_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<Vec<PendingComputerCommandDto>> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .poll_local_computer_commands(token)
            .await?)
    }

    pub fn poll_local_computer_commands_async(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<PendingComputerCommandDto>>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.poll_local_computer_commands_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn process_ready_local_computer_commands(&self) -> Result<Vec<PendingComputerCommandDto>> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.process_ready_local_computer_commands(&token).await })
    }

    pub fn process_ready_local_computer_commands_async(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<PendingComputerCommandDto>>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.process_ready_local_computer_commands_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn approve_computer_command(
        &self,
        command_id: &str,
        approved: bool,
    ) -> Result<PendingComputerCommandDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move {
            http.approve_computer_command(&token, command_id, approved)
                .await
        })
    }

    pub async fn approve_computer_command_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        command_id: &str,
        approved: bool,
    ) -> Result<PendingComputerCommandDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .approve_computer_command(token, command_id, approved)
            .await?)
    }

    pub fn approve_computer_command_async<'a>(
        &'a self,
        command_id: &'a str,
        approved: bool,
    ) -> impl std::future::Future<Output = Result<PendingComputerCommandDto>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.approve_computer_command_with_token(&token, expected_epoch, command_id, approved)
                .await
        }
    }

    pub fn complete_computer_command(
        &self,
        command_id: &str,
        result: &RemoteComputerCommandResult,
    ) -> Result<()> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move {
            http.complete_computer_command(&token, command_id, result)
                .await
        })
    }

    pub fn complete_computer_command_async<'a>(
        &'a self,
        command_id: &'a str,
        result: &'a RemoteComputerCommandResult,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.complete_computer_command_with_token(&token, expected_epoch, command_id, result)
                .await
        }
    }

    pub fn upsert_cloud_computer(
        &self,
        req: &UpsertCloudComputerRequest,
    ) -> Result<ComputerNodeDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.upsert_cloud_computer(&token, req).await })
    }

    pub async fn upsert_cloud_computer_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &UpsertCloudComputerRequest,
    ) -> Result<ComputerNodeDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.upsert_cloud_computer(token, req).await?)
    }

    pub fn upsert_cloud_computer_async<'a>(
        &'a self,
        req: &'a UpsertCloudComputerRequest,
    ) -> impl std::future::Future<Output = Result<ComputerNodeDto>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.upsert_cloud_computer_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub fn update_computer_policy(
        &self,
        node_id: &str,
        req: &UpdateComputerPolicyRequest,
    ) -> Result<ComputerNodeDto> {
        let (start_epoch, token) = self.snapshot_pinned_session()?;
        let http = self.http_client.clone();
        let updated = self
            .block_on_http(async move { http.update_computer_policy(&token, node_id, req).await })?;
        if self.is_same_session_epoch(start_epoch) {
            self.tailnet.sync_node_policy_from_dto(&updated);
        }
        Ok(updated)
    }

    pub async fn update_computer_policy_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        node_id: &str,
        req: &UpdateComputerPolicyRequest,
    ) -> Result<ComputerNodeDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let updated = self
            .http_client
            .update_computer_policy(token, node_id, req)
            .await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.tailnet.sync_node_policy_from_dto(&updated);
        }
        Ok(updated)
    }

    pub fn update_computer_policy_async<'a>(
        &'a self,
        node_id: &'a str,
        req: &'a UpdateComputerPolicyRequest,
    ) -> impl std::future::Future<Output = Result<ComputerNodeDto>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.update_computer_policy_with_token(&token, expected_epoch, node_id, req)
                .await
        }
    }

    pub async fn register_current_host_computer_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<ComputerNodeDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.register_current_host_computer(token).await?)
    }

    pub async fn process_ready_local_computer_commands_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<Vec<PendingComputerCommandDto>> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let pending = self.http_client.poll_local_computer_commands(token).await?;
        self.ensure_pinned_session(expected_epoch, token)?;
        let mut processed = Vec::new();
        for cmd in pending {
            if cmd.requires_user_approval && !cmd.approved {
                continue;
            }
            self.ensure_pinned_session(expected_epoch, token)?;
            let exec_res = crate::net::api_computers::execute_local_computer_op(
                &cmd.node_id,
                &cmd.command_id,
                &cmd.operation,
            )
            .await;
            self.ensure_pinned_session(expected_epoch, token)?;
            self.http_client
                .complete_computer_command(token, &cmd.command_id, &exec_res)
                .await?;
            processed.push(cmd);
        }
        Ok(processed)
    }

    pub async fn complete_computer_command_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        command_id: &str,
        result: &RemoteComputerCommandResult,
    ) -> Result<()> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .complete_computer_command(token, command_id, result)
            .await?)
    }

    pub async fn execute_remote_computer_op_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &RemoteComputerCommandRequest,
    ) -> Result<RemoteComputerCommandResult> {
        self.ensure_pinned_session(expected_epoch, token)?;
        if let Some(res) = self.tailnet.execute_peer_op(req).await {
            self.ensure_pinned_session(expected_epoch, token)?;
            return Ok(res);
        }
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .execute_remote_computer_op(token, req)
            .await?)
    }

    pub async fn list_computers_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<ComputerListResponse> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.list_computers(token).await?)
    }

    pub fn delete_computer(&self, node_id: &str) -> Result<()> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.delete_computer(&token, node_id).await })
    }

    pub async fn delete_computer_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        node_id: &str,
    ) -> Result<()> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.delete_computer(token, node_id).await?)
    }

    pub fn delete_computer_async<'a>(
        &'a self,
        node_id: &'a str,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.delete_computer_with_token(&token, expected_epoch, node_id)
                .await
        }
    }

    pub fn test_computer_connection(
        &self,
        node_id: &str,
    ) -> Result<RemoteComputerCommandResult> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.test_computer_connection(&token, node_id).await })
    }

    pub async fn test_computer_connection_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        node_id: &str,
    ) -> Result<RemoteComputerCommandResult> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let p2p_req = RemoteComputerCommandRequest {
            node_id: node_id.to_string(),
            caller_device_id: Some(self.device_id()),
            caller_bot_id: None,
            caller_label: format!("Device {}", self.device_id()),
            operation: RemoteComputerOp::TestConnection,
        };
        if let Some(res) = self.tailnet.execute_peer_op(&p2p_req).await {
            self.ensure_pinned_session(expected_epoch, token)?;
            return Ok(res);
        }
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .test_computer_connection(token, node_id)
            .await?)
    }

    pub fn test_computer_connection_async<'a>(
        &'a self,
        node_id: &'a str,
    ) -> impl std::future::Future<Output = Result<RemoteComputerCommandResult>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.test_computer_connection_with_token(&token, expected_epoch, node_id)
                .await
        }
    }

    pub fn execute_remote_computer_op(
        &self,
        req: &RemoteComputerCommandRequest,
    ) -> Result<RemoteComputerCommandResult> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.execute_remote_computer_op(&token, req).await })
    }

    pub fn execute_remote_computer_op_async<'a>(
        &'a self,
        req: &'a RemoteComputerCommandRequest,
    ) -> impl std::future::Future<Output = Result<RemoteComputerCommandResult>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.execute_remote_computer_op_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub fn get_bot_computer_profile(&self, bot_id: u64) -> Result<BotComputerProfileDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.get_bot_computer_profile(&token, bot_id).await })
    }

    pub async fn get_bot_computer_profile_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        bot_id: u64,
    ) -> Result<BotComputerProfileDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .get_bot_computer_profile(token, bot_id)
            .await?)
    }

    pub fn get_bot_computer_profile_async(
        &self,
        bot_id: u64,
    ) -> impl std::future::Future<Output = Result<BotComputerProfileDto>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_bot_computer_profile_with_token(&token, expected_epoch, bot_id)
                .await
        }
    }

    pub fn update_bot_computer_profile(
        &self,
        bot_id: u64,
        req: &UpdateBotComputerRequest,
    ) -> Result<BotComputerProfileDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.update_bot_computer_profile(&token, bot_id, req).await })
    }

    pub async fn update_bot_computer_profile_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        bot_id: u64,
        req: &UpdateBotComputerRequest,
    ) -> Result<BotComputerProfileDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .update_bot_computer_profile(token, bot_id, req)
            .await?)
    }

    pub fn update_bot_computer_profile_async<'a>(
        &'a self,
        bot_id: u64,
        req: &'a UpdateBotComputerRequest,
    ) -> impl std::future::Future<Output = Result<BotComputerProfileDto>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.update_bot_computer_profile_with_token(&token, expected_epoch, bot_id, req)
                .await
        }
    }

    pub fn get_remote_assist_info(&self) -> Result<RemoteAssistConfigDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.get_remote_assist_info(&token).await })
    }

    pub async fn get_remote_assist_info_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<RemoteAssistConfigDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.get_remote_assist_info(token).await?)
    }

    pub fn get_remote_assist_info_async(
        &self,
    ) -> impl std::future::Future<Output = Result<RemoteAssistConfigDto>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_remote_assist_info_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn reset_remote_assist_code(&self) -> Result<RemoteAssistConfigDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.reset_remote_assist_code(&token).await })
    }

    pub async fn reset_remote_assist_code_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<RemoteAssistConfigDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.reset_remote_assist_code(token).await?)
    }

    pub fn reset_remote_assist_code_async(
        &self,
    ) -> impl std::future::Future<Output = Result<RemoteAssistConfigDto>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.reset_remote_assist_code_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn set_remote_assist_code(&self, code: &str) -> Result<RemoteAssistConfigDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.set_remote_assist_code(&token, code).await })
    }

    pub async fn set_remote_assist_code_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        code: &str,
    ) -> Result<RemoteAssistConfigDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.set_remote_assist_code(token, code).await?)
    }

    pub fn set_remote_assist_code_async<'a>(
        &'a self,
        code: &'a str,
    ) -> impl std::future::Future<Output = Result<RemoteAssistConfigDto>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.set_remote_assist_code_with_token(&token, expected_epoch, code)
                .await
        }
    }

    pub fn set_remote_assist_allow(&self, allow_assist: bool) -> Result<RemoteAssistConfigDto> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.set_remote_assist_allow(&token, allow_assist).await })
    }

    pub async fn set_remote_assist_allow_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        allow_assist: bool,
    ) -> Result<RemoteAssistConfigDto> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .set_remote_assist_allow(token, allow_assist)
            .await?)
    }

    pub fn set_remote_assist_allow_async(
        &self,
        allow_assist: bool,
    ) -> impl std::future::Future<Output = Result<RemoteAssistConfigDto>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.set_remote_assist_allow_with_token(&token, expected_epoch, allow_assist)
                .await
        }
    }

    pub fn start_remote_assist(&self, req: &AssistStartRequest) -> Result<AssistStartResponse> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.start_remote_assist(&token, req).await })
    }

    pub async fn start_remote_assist_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &AssistStartRequest,
    ) -> Result<AssistStartResponse> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.start_remote_assist(token, req).await?)
    }

    pub fn start_remote_assist_async<'a>(
        &'a self,
        req: &'a AssistStartRequest,
    ) -> impl std::future::Future<Output = Result<AssistStartResponse>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.start_remote_assist_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub fn list_remote_sessions(&self) -> Result<RemoteSessionsResponse> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.list_remote_sessions(&token).await })
    }

    pub async fn list_remote_sessions_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<RemoteSessionsResponse> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.list_remote_sessions(token).await?)
    }

    pub fn list_remote_sessions_async(
        &self,
    ) -> impl std::future::Future<Output = Result<RemoteSessionsResponse>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_remote_sessions_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn disconnect_remote_session(&self, session_id: &str) -> Result<()> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.disconnect_remote_session(&token, session_id).await })
    }

    pub async fn disconnect_remote_session_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        session_id: &str,
    ) -> Result<()> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .disconnect_remote_session(token, session_id)
            .await?)
    }

    pub fn disconnect_remote_session_async<'a>(
        &'a self,
        session_id: &'a str,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.disconnect_remote_session_with_token(&token, expected_epoch, session_id)
                .await
        }
    }

    pub fn disconnect_all_remote_sessions(&self) -> Result<serde_json::Value> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.disconnect_all_remote_sessions(&token).await })
    }

    pub async fn disconnect_all_remote_sessions_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<serde_json::Value> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self
            .http_client
            .disconnect_all_remote_sessions(token)
            .await?)
    }

    pub fn disconnect_all_remote_sessions_async(
        &self,
    ) -> impl std::future::Future<Output = Result<serde_json::Value>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.disconnect_all_remote_sessions_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn list_mcp_grants(&self) -> Result<McpOAuthGrantsResponse> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.list_mcp_grants(&token).await })
    }

    pub async fn list_mcp_grants_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> Result<McpOAuthGrantsResponse> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.list_mcp_grants(token).await?)
    }

    pub fn list_mcp_grants_async(
        &self,
    ) -> impl std::future::Future<Output = Result<McpOAuthGrantsResponse>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_mcp_grants_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn revoke_mcp_grant(&self, family_id: &str) -> Result<()> {
        let token = self.require_token()?;
        let http = self.http_client.clone();
        self.block_on_http(async move { http.revoke_mcp_grant(&token, family_id).await })
    }

    pub async fn revoke_mcp_grant_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        family_id: &str,
    ) -> Result<()> {
        self.ensure_pinned_session(expected_epoch, token)?;
        Ok(self.http_client.revoke_mcp_grant(token, family_id).await?)
    }

    pub fn revoke_mcp_grant_async<'a>(
        &'a self,
        family_id: &'a str,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.revoke_mcp_grant_with_token(&token, expected_epoch, family_id)
                .await
        }
    }

    pub fn get_user_vault_envelope(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<UserVaultEnvelopeResponse, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_user_vault_envelope_with_token(&token, expected_epoch)
                .await
        }
    }

    pub async fn get_user_vault_envelope_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<UserVaultEnvelopeResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_user_vault_envelope(token).await
    }

    pub fn put_user_vault_envelope<'a>(
        &'a self,
        envelope: &'a UserVaultKeyEnvelopeDto,
    ) -> impl std::future::Future<
        Output = std::result::Result<UserVaultEnvelopeResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.put_user_vault_envelope_with_token(&token, expected_epoch, envelope)
                .await
        }
    }

    pub async fn put_user_vault_envelope_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        envelope: &UserVaultKeyEnvelopeDto,
    ) -> std::result::Result<UserVaultEnvelopeResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.put_user_vault_envelope(token, envelope).await
    }

    pub fn pull_user_vault_items<'a>(
        &'a self,
        since_revision: Option<i64>,
        category: Option<&'a str>,
    ) -> impl std::future::Future<
        Output = std::result::Result<UserVaultSyncPullResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.pull_user_vault_items_with_token(&token, expected_epoch, since_revision, category)
                .await
        }
    }

    pub async fn pull_user_vault_items_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        since_revision: Option<i64>,
        category: Option<&str>,
    ) -> std::result::Result<UserVaultSyncPullResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .pull_user_vault_items(token, since_revision, category)
            .await
    }

    pub fn put_user_vault_item<'a>(
        &'a self,
        category: &'a str,
        item_key: &'a str,
        req: &'a UpsertUserVaultItemRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<UpsertUserVaultItemResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.put_user_vault_item_with_token(&token, expected_epoch, category, item_key, req)
                .await
        }
    }

    pub async fn put_user_vault_item_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        category: &str,
        item_key: &str,
        req: &UpsertUserVaultItemRequest,
    ) -> std::result::Result<UpsertUserVaultItemResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .put_user_vault_item(token, category, item_key, req)
            .await
    }

    pub fn delete_user_vault_item<'a>(
        &'a self,
        category: &'a str,
        item_key: &'a str,
        expected_revision: Option<i64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<UpsertUserVaultItemResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.delete_user_vault_item_with_token(
                &token,
                expected_epoch,
                category,
                item_key,
                expected_revision,
            )
            .await
        }
    }

    pub async fn delete_user_vault_item_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        category: &str,
        item_key: &str,
        expected_revision: Option<i64>,
    ) -> std::result::Result<UpsertUserVaultItemResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .delete_user_vault_item(token, category, item_key, expected_revision)
            .await
    }

    pub fn archive_local_bot_reply<'a>(
        &'a self,
        chat_id: i64,
        req: &'a ArchiveLocalBotReplyRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<ArchiveLocalBotReplyResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.archive_local_bot_reply_with_token(&token, expected_epoch, chat_id, req)
                .await
        }
    }

    pub async fn archive_local_bot_reply_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        req: &ArchiveLocalBotReplyRequest,
    ) -> std::result::Result<ArchiveLocalBotReplyResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .archive_local_bot_reply(token, chat_id, req)
            .await
    }

    pub fn device_sync_request<'a>(
        &'a self,
        req: &'a DeviceSyncRequestBody,
    ) -> impl std::future::Future<
        Output = std::result::Result<DeviceSyncRequestResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.device_sync_request_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub async fn device_sync_request_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &DeviceSyncRequestBody,
    ) -> std::result::Result<DeviceSyncRequestResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.device_sync_request(token, req).await
    }

    pub fn device_sync_pending(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<PendingDeviceSyncListResponse, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.device_sync_pending_with_token(&token, expected_epoch)
                .await
        }
    }

    pub async fn device_sync_pending_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<PendingDeviceSyncListResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.device_sync_pending(token).await
    }

    pub fn device_sync_approve<'a>(
        &'a self,
        request_id: &'a str,
        req: &'a ApproveDeviceSyncRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<DeviceSyncActionResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.device_sync_approve_with_token(&token, expected_epoch, request_id, req)
                .await
        }
    }

    pub async fn device_sync_approve_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        request_id: &str,
        req: &ApproveDeviceSyncRequest,
    ) -> std::result::Result<DeviceSyncActionResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .device_sync_approve(token, request_id, req)
            .await
    }

    pub fn device_sync_deny<'a>(
        &'a self,
        request_id: &'a str,
    ) -> impl std::future::Future<
        Output = std::result::Result<DeviceSyncActionResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.device_sync_deny_with_token(&token, expected_epoch, request_id)
                .await
        }
    }

    pub async fn device_sync_deny_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        request_id: &str,
    ) -> std::result::Result<DeviceSyncActionResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.device_sync_deny(token, request_id).await
    }

    pub fn device_sync_status<'a>(
        &'a self,
        request_id: &'a str,
    ) -> impl std::future::Future<
        Output = std::result::Result<DeviceSyncStatusResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.device_sync_status_with_token(&token, expected_epoch, request_id)
                .await
        }
    }

    pub async fn device_sync_status_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        request_id: &str,
    ) -> std::result::Result<DeviceSyncStatusResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.device_sync_status(token, request_id).await
    }
}
