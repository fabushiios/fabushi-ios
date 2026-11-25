use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bot_browser::backend::{BrowserSessionBackend, CdpBrowserBackend};
use bot_browser::orchestrator::{BotBrowserOrchestrator, QrAssistResult};
use bot_browser::script_app::{
    AutomationScriptApp, BrowserActionTrace, BrowserControlGate, BrowserControlMode,
    CredentialVaultSecretResolver, DeterministicScriptRunner, RecordedActionKind,
    ScriptAppMetadata, ScriptCompiler, ScriptReplayReport, VaultSecretResolver,
};
use bot_browser::types::{
    BotBrowserError, BrowserBackendKind, BrowserSnapshot, NormalizedOrigin, SemanticLocator,
};
use parking_lot::{Mutex as SyncMutex, RwLock as SyncRwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex, OwnedMutexGuard};

use super::trajectory::SecretRedactor;

pub const DEFAULT_MAX_STEPS_PER_RUN: usize = 32;
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSessionConfig {
    pub max_steps_per_run: usize,
    pub auto_pause_on_sensitive_origin: bool,
    pub event_channel_capacity: usize,
}

impl Default for BrowserSessionConfig {
    fn default() -> Self {
        Self {
            max_steps_per_run: DEFAULT_MAX_STEPS_PER_RUN,
            auto_pause_on_sensitive_origin: false,
            event_channel_capacity: DEFAULT_EVENT_CHANNEL_CAPACITY,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserAutomationEvent {
    StepStarted {
        session_id: String,
        step_index: usize,
        operation: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locator: Option<SemanticLocator>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    StepCompleted {
        session_id: String,
        step_index: usize,
        operation: String,
        duration_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locator: Option<SemanticLocator>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    StepFailed {
        session_id: String,
        step_index: usize,
        operation: String,
        duration_ms: u64,
        error: String,
    },
    TakeoverRequested {
        session_id: String,
        reason: String,
        paused_at_step: usize,
    },
    ControlModeChanged {
        session_id: String,
        mode: BrowserControlMode,
    },
    TraceCompleted {
        session_id: String,
        trace: BrowserActionTrace,
    },
    ScriptDistilled {
        session_id: String,
        app: AutomationScriptApp,
    },
}

/// Forwards events from a `broadcast::Receiver<BrowserAutomationEvent>` into a `tokio::sync::mpsc::Sender`.
pub fn forward_events_to_mpsc(
    mut rx: broadcast::Receiver<BrowserAutomationEvent>,
    tx: mpsc::Sender<BrowserAutomationEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TakeoverOutcome {
    pub session_id: String,
    pub resumed: bool,
    pub reason: String,
    pub paused_at_step: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<BrowserSnapshot>,
}

#[derive(Debug, Clone)]
pub struct StepGateInfo {
    pub step_index: usize,
    pub resumed_from_takeover: bool,
    pub refreshed_url: Option<String>,
    pub refreshed_title: Option<String>,
    pub refreshed_snapshot: Option<BrowserSnapshot>,
}

#[derive(Clone)]
pub enum SessionBackendSource {
    Registered(Arc<dyn BrowserSessionBackend>),
    Orchestrator {
        orchestrator: Arc<BotBrowserOrchestrator>,
        backend: Arc<CdpBrowserBackend>,
    },
}

impl SessionBackendSource {
    pub fn as_backend(&self) -> Arc<dyn BrowserSessionBackend> {
        match self {
            Self::Registered(b) => Arc::clone(b),
            Self::Orchestrator { backend, .. } => Arc::clone(backend) as Arc<dyn BrowserSessionBackend>,
        }
    }

    pub fn orchestrator(&self) -> Option<&Arc<BotBrowserOrchestrator>> {
        match self {
            Self::Registered(_) => None,
            Self::Orchestrator { orchestrator, .. } => Some(orchestrator),
        }
    }

    pub fn is_registered(&self) -> bool {
        matches!(self, Self::Registered(_))
    }
}

/// RAII guard owning the per-session AI concurrency lock for an AI browser run.
/// Dropping the guard completes AI control mode, finalizes any active trace (when configured),
/// and releases the session lock so subsequent AI runs can proceed.
pub struct AiTurnGuard {
    session: Arc<BrowserAutomationSession>,
    _lock_guard: Option<OwnedMutexGuard<()>>,
    finalize_trace_on_drop: bool,
    finished: bool,
}

impl AiTurnGuard {
    pub fn session(&self) -> &Arc<BrowserAutomationSession> {
        &self.session
    }

    pub fn session_id(&self) -> &str {
        self.session.session_id()
    }

    pub fn finish(&mut self) {
        if !self.finished {
            self.finished = true;
            self.session
                .finish_ai_turn_internal(self.finalize_trace_on_drop);
        }
    }
}

impl Drop for AiTurnGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            self.session
                .finish_ai_turn_internal(self.finalize_trace_on_drop);
        }
    }
}

/// Per-turn container tracking active `AiTurnGuard`s acquired within a single `HermesEngine` turn.
/// Allows sequential intra-turn tool calls on the same `session_id` without self-rejection while
/// rejecting overlapping concurrent AI turns on the same session.
pub type TurnBrowserGuardMap = Arc<AsyncMutex<HashMap<String, AiTurnGuard>>>;

pub struct BrowserAutomationSession {
    session_id: String,
    source: SyncRwLock<SessionBackendSource>,
    control_gate: Arc<BrowserControlGate>,
    ai_lock: Arc<AsyncMutex<()>>,
    steps_used: AtomicUsize,
    max_steps_per_run: AtomicUsize,
    auto_pause_on_sensitive_origin: AtomicBool,
    pause_checkpoint_recorded: AtomicBool,
    trace_finalized: AtomicBool,
    trace: SyncMutex<BrowserActionTrace>,
    last_completed_trace: SyncMutex<Option<BrowserActionTrace>>,
    last_distilled_app: SyncMutex<Option<AutomationScriptApp>>,
    last_emitted_mode: Arc<SyncMutex<BrowserControlMode>>,
    event_tx: broadcast::Sender<BrowserAutomationEvent>,
    registry_event_tx: Option<broadcast::Sender<BrowserAutomationEvent>>,
}

struct WaitPauseGuard<'a> {
    session: &'a BrowserAutomationSession,
    armed: bool,
}

impl<'a> WaitPauseGuard<'a> {
    fn new(session: &'a BrowserAutomationSession) -> Self {
        Self {
            session,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for WaitPauseGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.session.control_gate.complete_ai();
            self.session
                .pause_checkpoint_recorded
                .store(false, Ordering::SeqCst);
            self.session.emit_mode_if_changed();
        }
    }
}

fn emit_mode_transition(
    session_id: &str,
    mode: BrowserControlMode,
    last_emitted_mode: &SyncMutex<BrowserControlMode>,
    event_tx: &broadcast::Sender<BrowserAutomationEvent>,
    registry_event_tx: Option<&broadcast::Sender<BrowserAutomationEvent>>,
) {
    let mut last = last_emitted_mode.lock();
    if *last != mode {
        *last = mode.clone();
        drop(last);
        let ev = BrowserAutomationEvent::ControlModeChanged {
            session_id: session_id.to_string(),
            mode,
        };
        let _ = event_tx.send(ev.clone());
        if let Some(reg_tx) = registry_event_tx {
            let _ = reg_tx.send(ev);
        }
    }
}

impl BrowserAutomationSession {
    pub fn new(
        session_id: impl Into<String>,
        source: SessionBackendSource,
        config: BrowserSessionConfig,
    ) -> Arc<Self> {
        Self::new_with_registry_tx(session_id, source, config, None)
    }

    pub(crate) fn new_with_registry_tx(
        session_id: impl Into<String>,
        source: SessionBackendSource,
        config: BrowserSessionConfig,
        registry_event_tx: Option<broadcast::Sender<BrowserAutomationEvent>>,
    ) -> Arc<Self> {
        let sid = session_id.into();
        let cap = config.event_channel_capacity.max(16);
        let (event_tx, _) = broadcast::channel(cap);
        let max_steps = config.max_steps_per_run.max(1);
        Arc::new(Self {
            trace: SyncMutex::new(BrowserActionTrace::new(&sid)),
            session_id: sid,
            source: SyncRwLock::new(source),
            control_gate: Arc::new(BrowserControlGate::new()),
            ai_lock: Arc::new(AsyncMutex::new(())),
            steps_used: AtomicUsize::new(0),
            max_steps_per_run: AtomicUsize::new(max_steps),
            auto_pause_on_sensitive_origin: AtomicBool::new(config.auto_pause_on_sensitive_origin),
            pause_checkpoint_recorded: AtomicBool::new(false),
            trace_finalized: AtomicBool::new(false),
            last_completed_trace: SyncMutex::new(None),
            last_distilled_app: SyncMutex::new(None),
            last_emitted_mode: Arc::new(SyncMutex::new(BrowserControlMode::Idle)),
            event_tx,
            registry_event_tx,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn backend(&self) -> Arc<dyn BrowserSessionBackend> {
        self.source.read().as_backend()
    }

    pub fn source(&self) -> SessionBackendSource {
        self.source.read().clone()
    }

    pub fn set_registered_backend(&self, backend: Arc<dyn BrowserSessionBackend>) {
        *self.source.write() = SessionBackendSource::Registered(backend);
    }

    pub fn set_orchestrator_backend(
        &self,
        orchestrator: Arc<BotBrowserOrchestrator>,
        backend: Arc<CdpBrowserBackend>,
    ) {
        *self.source.write() = SessionBackendSource::Orchestrator {
            orchestrator,
            backend,
        };
    }

    pub fn control_gate(&self) -> &Arc<BrowserControlGate> {
        &self.control_gate
    }

    pub fn control_mode(&self) -> BrowserControlMode {
        self.control_gate.mode()
    }

    pub fn max_steps_per_run(&self) -> usize {
        self.max_steps_per_run.load(Ordering::SeqCst)
    }

    pub fn set_max_steps_per_run(&self, max_steps: usize) {
        self.max_steps_per_run
            .store(max_steps.max(1), Ordering::SeqCst);
    }

    pub fn steps_used(&self) -> usize {
        self.steps_used.load(Ordering::SeqCst)
    }

    pub fn auto_pause_on_sensitive_origin(&self) -> bool {
        self.auto_pause_on_sensitive_origin.load(Ordering::SeqCst)
    }

    pub fn set_auto_pause_on_sensitive_origin(&self, enabled: bool) {
        self.auto_pause_on_sensitive_origin
            .store(enabled, Ordering::SeqCst);
    }

    pub fn is_ai_locked(&self) -> bool {
        self.ai_lock.try_lock().is_err()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BrowserAutomationEvent> {
        self.event_tx.subscribe()
    }

    pub fn subscribe_mpsc(
        &self,
        capacity: usize,
    ) -> (mpsc::Receiver<BrowserAutomationEvent>, tokio::task::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(capacity.max(16));
        let handle = forward_events_to_mpsc(self.subscribe(), tx);
        (rx, handle)
    }

    pub(crate) fn emit_event(&self, ev: BrowserAutomationEvent) {
        let _ = self.event_tx.send(ev.clone());
        if let Some(ref reg_tx) = self.registry_event_tx {
            let _ = reg_tx.send(ev);
        }
    }

    fn emit_mode_if_changed(&self) {
        emit_mode_transition(
            &self.session_id,
            self.control_gate.mode(),
            &self.last_emitted_mode,
            &self.event_tx,
            self.registry_event_tx.as_ref(),
        );
    }

    pub fn user_take_over(&self) {
        self.control_gate.user_take_over();
        self.emit_mode_if_changed();
    }

    pub fn user_resume_ai(&self) {
        self.control_gate.user_resume_ai();
        self.emit_mode_if_changed();
    }

    pub fn user_cancel_ai(&self) {
        self.control_gate.user_cancel_ai();
        self.emit_mode_if_changed();
    }

    pub fn reset_control(&self) {
        self.control_gate.reset();
        self.pause_checkpoint_recorded
            .store(false, Ordering::SeqCst);
        self.emit_mode_if_changed();
    }

    /// Acquires the per-session AI concurrency lock for a full AI turn (`try_lock_owned`).
    /// Rejects overlapping AI runs on the same locked session without blocking.
    pub fn try_begin_ai_turn(self: &Arc<Self>) -> Result<AiTurnGuard, BotBrowserError> {
        let lock_guard = self.ai_lock.clone().try_lock_owned().map_err(|_| {
            BotBrowserError::Cdp(format!(
                "Browser session '{}' is currently locked by another active AI run",
                self.session_id
            ))
        })?;
        self.control_gate.reset();
        self.steps_used.store(0, Ordering::SeqCst);
        self.pause_checkpoint_recorded
            .store(false, Ordering::SeqCst);
        self.trace_finalized.store(false, Ordering::SeqCst);
        *self.trace.lock() = BrowserActionTrace::new(&self.session_id);
        self.control_gate.start_ai("AI browser automation active");
        self.emit_mode_if_changed();
        Ok(AiTurnGuard {
            session: Arc::clone(self),
            _lock_guard: Some(lock_guard),
            finalize_trace_on_drop: true,
            finished: false,
        })
    }

    /// Acquires the per-session AI concurrency lock for a single standalone tool call
    /// (outside `HermesEngine::run_agent_loop`) without wiping the accumulated trace or
    /// clearing an active cancellation flag.
    pub fn try_begin_standalone_step(self: &Arc<Self>) -> Result<AiTurnGuard, BotBrowserError> {
        let lock_guard = self.ai_lock.clone().try_lock_owned().map_err(|_| {
            BotBrowserError::Cdp(format!(
                "Browser session '{}' is currently locked by another active AI run",
                self.session_id
            ))
        })?;
        if !self.control_gate.is_cancelled()
            && matches!(self.control_gate.mode(), BrowserControlMode::Idle)
        {
            self.control_gate.start_ai("AI browser step active");
            self.emit_mode_if_changed();
        }
        Ok(AiTurnGuard {
            session: Arc::clone(self),
            _lock_guard: Some(lock_guard),
            finalize_trace_on_drop: false,
            finished: false,
        })
    }

    fn finish_ai_turn_internal(&self, finalize_trace: bool) {
        if matches!(
            self.control_gate.mode(),
            BrowserControlMode::AiControlling { .. }
        ) {
            self.control_gate.complete_ai();
            self.emit_mode_if_changed();
        }
        if finalize_trace {
            let _ = self.finalize_trace();
        }
    }

    /// Marks the active `BrowserActionTrace` as completed (if non-empty and not yet finalized),
    /// stores it in `last_completed_trace`, and emits `TraceCompleted`.
    pub fn finalize_trace(&self) -> Option<BrowserActionTrace> {
        let mut guard = self.trace.lock();
        if guard.actions.is_empty() {
            return self.last_completed_trace.lock().clone();
        }
        guard.mark_completed();
        let completed = guard.clone();
        *guard = BrowserActionTrace::new(&self.session_id);
        drop(guard);

        *self.last_completed_trace.lock() = Some(completed.clone());
        self.trace_finalized.store(true, Ordering::SeqCst);
        self.emit_event(BrowserAutomationEvent::TraceCompleted {
            session_id: self.session_id.clone(),
            trace: completed.clone(),
        });
        Some(completed)
    }

    pub fn current_trace(&self) -> BrowserActionTrace {
        self.trace.lock().clone()
    }

    pub fn last_completed_trace(&self) -> Option<BrowserActionTrace> {
        self.last_completed_trace.lock().clone()
    }

    pub fn last_distilled_app(&self) -> Option<AutomationScriptApp> {
        self.last_distilled_app.lock().clone()
    }

    pub fn reset_step_budget(&self) {
        self.steps_used.store(0, Ordering::SeqCst);
    }

    /// Pause gate + step-budget check + `StepStarted` telemetry executed before each AI step.
    pub async fn prepare_step(
        &self,
        operation: &str,
        locator: Option<SemanticLocator>,
        url: Option<String>,
    ) -> Result<StepGateInfo, BotBrowserError> {
        let pre_step = self.steps_used.load(Ordering::SeqCst);
        let was_paused = matches!(
            self.control_gate.mode(),
            BrowserControlMode::WaitingForTakeover { .. } | BrowserControlMode::UserTakeover { .. }
        );
        // Wait cleanly on tokio::sync::Notify if paused, or abort immediately if cancelled.
        // Arm `WaitPauseGuard` before `current_url().await` so aborting during URL lookup
        // or while suspended restores `Idle` and clears `pause_checkpoint_recorded`.
        {
            let mut wait_guard = WaitPauseGuard::new(self);
            if was_paused && !self.pause_checkpoint_recorded.swap(true, Ordering::SeqCst) {
                let reason = match self.control_gate.mode() {
                    BrowserControlMode::WaitingForTakeover { reason } => reason,
                    BrowserControlMode::UserTakeover { paused_at_step } => {
                        format!("User took over browser control at step {paused_at_step}")
                    }
                    _ => "Human takeover checkpoint".to_string(),
                };
                let current_url = self
                    .backend()
                    .current_url()
                    .await
                    .ok()
                    .map(|u| redact_url_for_trace(&u));
                self.trace.lock().push_action(
                    current_url,
                    RecordedActionKind::HumanCheckpoint { reason },
                    None,
                );
            }

            self.control_gate
                .wait_if_paused_or_cancelled(pre_step)
                .await?;
            wait_guard.disarm();
        }

        self.pause_checkpoint_recorded
            .store(false, Ordering::SeqCst);

        let mut refreshed_url = None;
        let mut refreshed_title = None;
        let mut refreshed_snapshot = None;
        if was_paused {
            self.emit_mode_if_changed();
            let (u, t, s) = self.capture_live_observation().await;
            refreshed_url = u;
            refreshed_title = t;
            refreshed_snapshot = s;
        }

        let max_steps = self.max_steps_per_run.load(Ordering::SeqCst).max(1);
        let step_num = self.steps_used.fetch_add(1, Ordering::SeqCst) + 1;
        let step_index = step_num - 1;
        if step_num > max_steps {
            let err_msg = format!(
                "Step budget exhausted ({max_steps} steps max) while attempting '{operation}' on session '{}'",
                self.session_id
            );
            self.emit_event(BrowserAutomationEvent::StepFailed {
                session_id: self.session_id.clone(),
                step_index,
                operation: operation.to_string(),
                duration_ms: 0,
                error: err_msg.clone(),
            });
            self.control_gate.complete_ai();
            self.emit_mode_if_changed();
            return Err(BotBrowserError::ScriptReplay(err_msg));
        }

        if matches!(self.control_gate.mode(), BrowserControlMode::Idle) {
            self.control_gate.start_ai(operation);
        }
        self.control_gate.set_active_step(step_index, operation);
        self.emit_mode_if_changed();

        let redacted_url = url.map(|u| redact_url_for_trace(&u));
        self.emit_event(BrowserAutomationEvent::StepStarted {
            session_id: self.session_id.clone(),
            step_index,
            operation: operation.to_string(),
            locator,
            url: redacted_url,
        });

        Ok(StepGateInfo {
            step_index,
            resumed_from_takeover: was_paused,
            refreshed_url,
            refreshed_title,
            refreshed_snapshot,
        })
    }

    pub fn emit_step_completed(
        &self,
        step_index: usize,
        operation: &str,
        duration_ms: u64,
        locator: Option<SemanticLocator>,
        url: Option<String>,
    ) {
        self.emit_event(BrowserAutomationEvent::StepCompleted {
            session_id: self.session_id.clone(),
            step_index,
            operation: operation.to_string(),
            duration_ms,
            locator,
            url: url.map(|u| redact_url_for_trace(&u)),
        });
    }

    pub fn emit_step_failed(
        &self,
        step_index: usize,
        operation: &str,
        duration_ms: u64,
        error: impl Into<String>,
    ) {
        self.emit_event(BrowserAutomationEvent::StepFailed {
            session_id: self.session_id.clone(),
            step_index,
            operation: operation.to_string(),
            duration_ms,
            error: error.into(),
        });
    }

    /// Requests human takeover via `BrowserControlGate`, emits `TakeoverRequested` +
    /// `ControlModeChanged`, records a `HumanCheckpoint` boundary in `BrowserActionTrace`,
    /// optionally suspends on `tokio::sync::Notify` until `user_resume_ai()` / `user_cancel_ai()`,
    /// and refreshes live DOM & URL upon resume.
    pub async fn request_takeover_and_wait(
        &self,
        reason: &str,
        wait_for_finish: bool,
        record_checkpoint: bool,
    ) -> Result<TakeoverOutcome, BotBrowserError> {
        if self.control_gate.is_cancelled() {
            return Err(BotBrowserError::ControlCancelled(format!(
                "cancelled before takeover at step {}",
                self.steps_used.load(Ordering::SeqCst)
            )));
        }

        let paused_at_step = self.steps_used.load(Ordering::SeqCst);
        let clean_reason = if reason.trim().is_empty() {
            "User assistance requested".to_string()
        } else {
            reason.trim().to_string()
        };

        let mut wait_guard = WaitPauseGuard::new(self);
        self.control_gate.request_takeover(clean_reason.clone());
        self.emit_mode_if_changed();
        self.emit_event(BrowserAutomationEvent::TakeoverRequested {
            session_id: self.session_id.clone(),
            reason: clean_reason.clone(),
            paused_at_step,
        });

        if record_checkpoint {
            let current_url = self
                .backend()
                .current_url()
                .await
                .ok()
                .map(|u| redact_url_for_trace(&u));
            self.pause_checkpoint_recorded
                .store(true, Ordering::SeqCst);
            self.trace.lock().push_action(
                current_url,
                RecordedActionKind::HumanCheckpoint {
                    reason: clean_reason.clone(),
                },
                None,
            );
        }

        if !wait_for_finish {
            wait_guard.disarm();
            return Ok(TakeoverOutcome {
                session_id: self.session_id.clone(),
                resumed: false,
                reason: clean_reason,
                paused_at_step,
                current_url: None,
                page_title: None,
                snapshot: None,
            });
        }

        self.control_gate
            .wait_if_paused_or_cancelled(paused_at_step)
            .await?;
        wait_guard.disarm();

        self.pause_checkpoint_recorded
            .store(false, Ordering::SeqCst);
        self.emit_mode_if_changed();

        let (current_url, page_title, snapshot) = self.capture_live_observation().await;
        Ok(TakeoverOutcome {
            session_id: self.session_id.clone(),
            resumed: true,
            reason: clean_reason,
            paused_at_step,
            current_url,
            page_title,
            snapshot,
        })
    }

    pub async fn capture_live_observation(
        &self,
    ) -> (Option<String>, Option<String>, Option<BrowserSnapshot>) {
        let backend = self.backend();
        let url = backend
            .current_url()
            .await
            .ok()
            .map(|u| redact_url_for_trace(&u));
        let title = backend.page_title().await.ok();
        let snapshot = match backend.snapshot_dom().await {
            Ok(mut snap) => {
                redact_snapshot_for_observation(&mut snap);
                Some(snap)
            }
            Err(_) => None,
        };
        (url, title, snapshot)
    }

    pub fn record_navigate(&self, url: &str, duration_ms: u64) {
        let redacted = redact_url_for_trace(url);
        let mut trace = self.trace.lock();
        if trace.initial_url.is_none() {
            trace.initial_url = Some(redacted.clone());
        }
        trace.push_action(
            Some(redacted.clone()),
            RecordedActionKind::Navigate { url: redacted },
            Some(duration_ms),
        );
    }

    pub fn record_click(&self, page_url: Option<String>, locator: SemanticLocator, duration_ms: u64) {
        let redacted_url = page_url.map(|u| redact_url_for_trace(&u));
        self.trace.lock().push_action(
            redacted_url,
            RecordedActionKind::Click { locator },
            Some(duration_ms),
        );
    }

    pub fn record_fill(
        &self,
        page_url: Option<String>,
        locator: SemanticLocator,
        raw_value: &str,
        explicit_is_secret: bool,
        vault_key_hint: Option<String>,
        parameter_name_hint: Option<String>,
        duration_ms: u64,
    ) {
        let redacted_url = page_url.map(|u| redact_url_for_trace(&u));
        let (safe_value, is_secret) =
            redact_value_for_trace(raw_value, &locator, explicit_is_secret);
        let hint = if is_secret && vault_key_hint.is_none() {
            parameter_name_hint
                .as_deref()
                .map(|p| format!("secret.{p}"))
        } else {
            vault_key_hint
        };
        self.trace.lock().push_action(
            redacted_url,
            RecordedActionKind::Fill {
                locator,
                value: safe_value,
                is_secret,
                vault_key_hint: hint,
                parameter_name_hint,
            },
            Some(duration_ms),
        );
    }

    pub fn record_press_key(&self, page_url: Option<String>, key: String, duration_ms: u64) {
        let redacted_url = page_url.map(|u| redact_url_for_trace(&u));
        self.trace.lock().push_action(
            redacted_url,
            RecordedActionKind::PressKey { key },
            Some(duration_ms),
        );
    }

    pub fn record_scroll(
        &self,
        page_url: Option<String>,
        delta_x: f64,
        delta_y: f64,
        duration_ms: u64,
    ) {
        let redacted_url = page_url.map(|u| redact_url_for_trace(&u));
        self.trace.lock().push_action(
            redacted_url,
            RecordedActionKind::Scroll { delta_x, delta_y },
            Some(duration_ms),
        );
    }

    pub fn record_wait_for(
        &self,
        page_url: Option<String>,
        locator: SemanticLocator,
        timeout_ms: u64,
        duration_ms: u64,
    ) {
        let redacted_url = page_url.map(|u| redact_url_for_trace(&u));
        self.trace.lock().push_action(
            redacted_url,
            RecordedActionKind::WaitFor {
                locator,
                timeout_ms,
            },
            Some(duration_ms),
        );
    }

    pub fn record_extract_text(
        &self,
        page_url: Option<String>,
        locator: SemanticLocator,
        output_key: String,
        duration_ms: u64,
    ) {
        let redacted_url = page_url.map(|u| redact_url_for_trace(&u));
        self.trace.lock().push_action(
            redacted_url,
            RecordedActionKind::ExtractText {
                locator,
                output_key,
            },
            Some(duration_ms),
        );
    }

    pub fn record_human_checkpoint(&self, page_url: Option<String>, reason: String) {
        let redacted_url = page_url.map(|u| redact_url_for_trace(&u));
        self.trace.lock().push_action(
            redacted_url,
            RecordedActionKind::HumanCheckpoint { reason },
            None,
        );
    }

    pub fn record_vault_fill(
        &self,
        page_url: Option<String>,
        credential_id: &str,
        username_selector: &str,
        password_selector: &str,
        totp_selector: Option<&str>,
        submit_selector: Option<&str>,
        duration_ms: u64,
    ) {
        let redacted_url = page_url.map(|u| redact_url_for_trace(&u));
        let mut trace = self.trace.lock();
        trace.push_action(
            redacted_url.clone(),
            RecordedActionKind::Fill {
                locator: SemanticLocator::by_css(username_selector),
                value: String::new(),
                is_secret: true,
                vault_key_hint: Some(format!("{credential_id}:username")),
                parameter_name_hint: Some("username".to_string()),
            },
            Some(duration_ms),
        );
        trace.push_action(
            redacted_url.clone(),
            RecordedActionKind::Fill {
                locator: SemanticLocator::by_css(password_selector),
                value: String::new(),
                is_secret: true,
                vault_key_hint: Some(format!("{credential_id}:password")),
                parameter_name_hint: Some("password".to_string()),
            },
            Some(duration_ms),
        );
        if let Some(totp_sel) = totp_selector {
            trace.push_action(
                redacted_url.clone(),
                RecordedActionKind::Fill {
                    locator: SemanticLocator::by_css(totp_sel),
                    value: String::new(),
                    is_secret: true,
                    vault_key_hint: Some(format!("{credential_id}:totp")),
                    parameter_name_hint: Some("totp".to_string()),
                },
                Some(duration_ms),
            );
        }
        if let Some(submit_sel) = submit_selector {
            trace.push_action(
                redacted_url,
                RecordedActionKind::Click {
                    locator: SemanticLocator::by_css(submit_sel),
                },
                Some(duration_ms),
            );
        }
    }

    /// Suspends on `BrowserControlGate` / `tokio::sync::Notify` while polling a registered
    /// `BrowserSessionBackend` for QR login redirect or auth cookie appearance (zero duplicate tab).
    pub async fn wait_for_qr_assist_or_resume(
        &self,
        step_index: usize,
        card_id: String,
        initial_url: &str,
        success_url_contains: Option<&str>,
        auth_cookie_names: &[String],
        refresh_interval: Duration,
        max_wait: Duration,
    ) -> Result<QrAssistResult, BotBrowserError> {
        let mut wait_guard = WaitPauseGuard::new(self);
        let backend = self.backend();
        let initial_cookies: HashMap<String, String> = backend
            .export_cookies(None)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect();

        let refresh_interval = refresh_interval.max(Duration::from_millis(10));
        let max_wait = max_wait.max(refresh_interval);
        let started = tokio::time::Instant::now();
        let mut poll_attempts = 0u32;

        loop {
            tokio::select! {
                gate_res = self.control_gate.wait_if_paused_or_cancelled(step_index) => {
                    gate_res?;
                    wait_guard.disarm();
                    self.pause_checkpoint_recorded.store(false, Ordering::SeqCst);
                    self.emit_mode_if_changed();
                    let current_url = backend.current_url().await.ok().map(|u| redact_url_for_trace(&u));
                    return Ok(QrAssistResult {
                        status: "qr_login_succeeded".to_string(),
                        card_id,
                        redirected_url: current_url,
                        matched_cookie: None,
                        poll_attempts,
                    });
                }
                _ = tokio::time::sleep(refresh_interval) => {}
            }

            if self.control_gate.is_cancelled() {
                return Err(BotBrowserError::ControlCancelled(format!(
                    "QR assist cancelled at step {step_index}"
                )));
            }

            poll_attempts += 1;
            let current_url = backend
                .current_url()
                .await
                .unwrap_or_else(|_| initial_url.to_string());
            let current_cookies: HashMap<String, String> = backend
                .export_cookies(None)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|c| (c.name.clone(), c.value.clone()))
                .collect();

            let url_matched = success_url_contains
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some_and(|needle| current_url.contains(needle));
            let url_changed = current_url != initial_url;
            let matched_cookie = if auth_cookie_names.is_empty() {
                current_cookies
                    .iter()
                    .find(|(k, v)| !v.is_empty() && initial_cookies.get(*k) != Some(*v))
                    .map(|(k, _)| k.clone())
            } else {
                auth_cookie_names
                    .iter()
                    .find(|name| {
                        current_cookies.get(*name).is_some_and(|v| {
                            !v.is_empty() && initial_cookies.get(*name) != Some(v)
                        })
                    })
                    .cloned()
            };

            if url_matched || url_changed || matched_cookie.is_some() {
                wait_guard.disarm();
                self.control_gate.user_resume_ai();
                self.pause_checkpoint_recorded.store(false, Ordering::SeqCst);
                self.emit_mode_if_changed();
                return Ok(QrAssistResult {
                    status: "qr_login_succeeded".to_string(),
                    card_id,
                    redirected_url: Some(redact_url_for_trace(&current_url)),
                    matched_cookie,
                    poll_attempts,
                });
            }

            if started.elapsed() >= max_wait {
                wait_guard.disarm();
                self.control_gate.user_resume_ai();
                self.pause_checkpoint_recorded.store(false, Ordering::SeqCst);
                self.emit_mode_if_changed();
                return Ok(QrAssistResult {
                    status: "expired".to_string(),
                    card_id,
                    redirected_url: None,
                    matched_cookie: None,
                    poll_attempts,
                });
            }
        }
    }

    /// Compiles the active (or last completed) `BrowserActionTrace` into a validated,
    /// secret-free `AutomationScriptApp`, emits `TraceCompleted` (if needed, once) and
    /// `ScriptDistilled`, and caches the resulting app.
    pub fn distill_script_app(
        &self,
        mut meta: ScriptAppMetadata,
    ) -> Result<AutomationScriptApp, BotBrowserError> {
        let (trace, had_active_actions) = {
            let active = self.trace.lock();
            if !active.actions.is_empty() {
                let mut completed = active.clone();
                completed.mark_completed();
                (completed, true)
            } else {
                drop(active);
                let prev = self
                    .last_completed_trace
                    .lock()
                    .clone()
                    .ok_or_else(|| {
                        BotBrowserError::ScriptCompilation(format!(
                            "No browser actions recorded yet for session '{}'",
                            self.session_id
                        ))
                    })?;
                (prev, false)
            }
        };

        if meta.target_domains.is_empty() {
            meta.target_domains = infer_target_domains_from_trace(&trace);
        }

        let app = ScriptCompiler::compile_from_trace(&trace, meta)?;
        if had_active_actions {
            let _ = self.finalize_trace();
        }
        *self.last_distilled_app.lock() = Some(app.clone());
        self.emit_event(BrowserAutomationEvent::ScriptDistilled {
            session_id: self.session_id.clone(),
            app: app.clone(),
        });
        Ok(app)
    }

    /// Deterministically replays a compiled `AutomationScriptApp` against this session's
    /// live `BrowserSessionBackend` with zero LLM inference (`used_llm == false`).
    pub async fn replay_script_app(
        &self,
        app: &AutomationScriptApp,
        params: &BTreeMap<String, String>,
        vault_resolver: Option<&dyn VaultSecretResolver>,
    ) -> Result<ScriptReplayReport, BotBrowserError> {
        let _lock = self.ai_lock.clone().try_lock_owned().map_err(|_| {
            BotBrowserError::Cdp(format!(
                "Browser session '{}' is currently locked by another active AI run",
                self.session_id
            ))
        })?;

        self.control_gate.reset();
        self.steps_used.store(0, Ordering::SeqCst);
        self.pause_checkpoint_recorded
            .store(false, Ordering::SeqCst);

        let backend = self.backend();
        let default_resolver = self
            .source
            .read()
            .orchestrator()
            .map(|orch| CredentialVaultSecretResolver::new(Arc::clone(orch.vault())));
        let resolver_ref: Option<&dyn VaultSecretResolver> = match vault_resolver {
            Some(r) => Some(r),
            None => default_resolver
                .as_ref()
                .map(|r| r as &dyn VaultSecretResolver),
        };

        // Synchronously transition and emit `AiControlling` before starting replay so fast
        // backends never race past an unpolled watcher task.
        self.control_gate
            .start_ai(format!("Replaying {}", app.name));
        self.emit_mode_if_changed();

        struct ReplayModeDropGuard<'a> {
            session: &'a BrowserAutomationSession,
            watcher: Option<tokio::task::JoinHandle<()>>,
            finished: bool,
        }
        impl<'a> ReplayModeDropGuard<'a> {
            fn finish(&mut self) {
                if !self.finished {
                    self.finished = true;
                    if let Some(h) = self.watcher.take() {
                        h.abort();
                    }
                    self.session.control_gate.complete_ai();
                    self.session.emit_mode_if_changed();
                }
            }
        }
        impl Drop for ReplayModeDropGuard<'_> {
            fn drop(&mut self) {
                self.finish();
            }
        }

        let mut mode_rx = self.control_gate.subscribe();
        let sid = self.session_id.clone();
        let last_mode = Arc::clone(&self.last_emitted_mode);
        let local_tx = self.event_tx.clone();
        let reg_tx = self.registry_event_tx.clone();
        let watcher = tokio::spawn(async move {
            while mode_rx.changed().await.is_ok() {
                let mode = mode_rx.borrow().clone();
                emit_mode_transition(&sid, mode, &last_mode, &local_tx, reg_tx.as_ref());
            }
        });
        let mut mode_guard = ReplayModeDropGuard {
            session: self,
            watcher: Some(watcher),
            finished: false,
        };

        let start = Instant::now();
        let report = DeterministicScriptRunner::run(
            backend.as_ref(),
            app,
            params,
            resolver_ref,
            Some(&self.control_gate),
        )
        .await;

        mode_guard.finish();

        match report {
            Ok(rep) => {
                let safe_final_url = rep.final_url.as_deref().map(redact_url_for_trace);
                for (idx, metric) in rep.step_timings.iter().enumerate() {
                    if metric.success {
                        self.emit_event(BrowserAutomationEvent::StepCompleted {
                            session_id: self.session_id.clone(),
                            step_index: metric.step_index.unwrap_or(idx),
                            operation: metric.operation.clone(),
                            duration_ms: metric.duration_ms,
                            locator: None,
                            url: safe_final_url.clone(),
                        });
                    } else {
                        self.emit_event(BrowserAutomationEvent::StepFailed {
                            session_id: self.session_id.clone(),
                            step_index: metric.step_index.unwrap_or(idx),
                            operation: metric.operation.clone(),
                            duration_ms: metric.duration_ms,
                            error: metric
                                .detail
                                .clone()
                                .or_else(|| rep.error.clone())
                                .unwrap_or_else(|| "script step failed".to_string()),
                        });
                    }
                }
                Ok(rep)
            }
            Err(err) => {
                self.emit_event(BrowserAutomationEvent::StepFailed {
                    session_id: self.session_id.clone(),
                    step_index: 0,
                    operation: "script_replay".to_string(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    error: err.to_string(),
                });
                Err(err)
            }
        }
    }
}

/// Shared registry mapping `session_id`s to active `BrowserAutomationSession`s.
/// Supports registered in-app `Arc<dyn BrowserSessionBackend>` tabs (zero duplicate tab
/// spawning) and lazy headless `Obscura` / CDP session creation via `BotBrowserOrchestrator`.
pub struct BrowserSessionRegistry {
    sessions: SyncRwLock<HashMap<String, Arc<BrowserAutomationSession>>>,
    orchestrator: SyncRwLock<Option<Arc<BotBrowserOrchestrator>>>,
    active_session_id: SyncRwLock<Option<String>>,
    default_config: SyncRwLock<BrowserSessionConfig>,
    event_tx: broadcast::Sender<BrowserAutomationEvent>,
}

impl Default for BrowserSessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserSessionRegistry {
    pub fn new() -> Self {
        Self::with_config(BrowserSessionConfig::default())
    }

    pub fn with_config(config: BrowserSessionConfig) -> Self {
        let cap = config.event_channel_capacity.max(16);
        let (event_tx, _) = broadcast::channel(cap);
        Self {
            sessions: SyncRwLock::new(HashMap::new()),
            orchestrator: SyncRwLock::new(None),
            active_session_id: SyncRwLock::new(None),
            default_config: SyncRwLock::new(config),
            event_tx,
        }
    }

    pub fn default_config(&self) -> BrowserSessionConfig {
        self.default_config.read().clone()
    }

    pub fn set_default_config(&self, config: BrowserSessionConfig) {
        *self.default_config.write() = config;
    }

    pub fn set_orchestrator(&self, orchestrator: Arc<BotBrowserOrchestrator>) {
        *self.orchestrator.write() = Some(orchestrator);
    }

    pub fn orchestrator(&self) -> Option<Arc<BotBrowserOrchestrator>> {
        self.orchestrator.read().clone()
    }

    pub fn set_active_session_id(&self, session_id: Option<String>) {
        *self.active_session_id.write() = session_id;
    }

    pub fn active_session_id(&self) -> Option<String> {
        self.active_session_id.read().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BrowserAutomationEvent> {
        self.event_tx.subscribe()
    }

    pub fn subscribe_mpsc(
        &self,
        capacity: usize,
    ) -> (mpsc::Receiver<BrowserAutomationEvent>, tokio::task::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(capacity.max(16));
        let handle = forward_events_to_mpsc(self.subscribe(), tx);
        (rx, handle)
    }

    pub fn list_session_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.sessions.read().keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn get_session(&self, session_id: &str) -> Option<Arc<BrowserAutomationSession>> {
        self.sessions.read().get(session_id).cloned()
    }

    /// Registers an active `Arc<dyn BrowserSessionBackend>` tab under `session_id`.
    /// If `session_id` already exists in the registry, updates its backend in place so
    /// existing subscribers and control-gate watchers remain attached without spawning duplicates.
    pub fn register_backend(
        &self,
        session_id: impl Into<String>,
        backend: Arc<dyn BrowserSessionBackend>,
    ) -> Arc<BrowserAutomationSession> {
        let sid = session_id.into();
        let mut guard = self.sessions.write();
        if let Some(existing) = guard.get(&sid) {
            existing.set_registered_backend(backend);
            return Arc::clone(existing);
        }
        let session = BrowserAutomationSession::new_with_registry_tx(
            sid.clone(),
            SessionBackendSource::Registered(backend),
            self.default_config(),
            Some(self.event_tx.clone()),
        );
        guard.insert(sid, Arc::clone(&session));
        session
    }

    /// Registers an active backend and sets it as the default active tab fallback.
    pub fn register_active_backend(
        &self,
        session_id: impl Into<String>,
        backend: Arc<dyn BrowserSessionBackend>,
    ) -> Arc<BrowserAutomationSession> {
        let sid = session_id.into();
        let session = self.register_backend(sid.clone(), backend);
        self.set_active_session_id(Some(sid));
        session
    }

    pub fn unregister_backend(&self, session_id: &str) -> Option<Arc<BrowserAutomationSession>> {
        let removed = self.sessions.write().remove(session_id);
        let mut active = self.active_session_id.write();
        if active.as_deref() == Some(session_id) {
            *active = None;
        }
        removed
    }

    /// Resolves an existing session (by explicit `session_id`, `ctx_session_id`, or active
    /// default tab fallback when `explicit_session_id` was omitted) or lazily creates one
    /// via `BotBrowserOrchestrator::ensure_session_backend` or `cdp_url` without duplicating tabs.
    pub async fn resolve_session(
        &self,
        explicit_session_id: Option<&str>,
        ctx_session_id: &str,
        orchestrator_override: Option<&Arc<BotBrowserOrchestrator>>,
        cdp_url: Option<&str>,
        origin_hint: Option<&NormalizedOrigin>,
    ) -> Result<Arc<BrowserAutomationSession>, BotBrowserError> {
        let target_id = explicit_session_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(ctx_session_id);

        if let Some(existing) = self.get_session(target_id) {
            return Ok(existing);
        }

        // If caller did not explicitly target a specific tab ID, check active default tab fallback.
        if explicit_session_id
            .map(str::trim)
            .is_none_or(str::is_empty)
        {
            if let Some(active_id) = self.active_session_id() {
                if let Some(active_sess) = self.get_session(&active_id) {
                    return Ok(active_sess);
                }
            }
        }

        let orch = orchestrator_override
            .cloned()
            .or_else(|| self.orchestrator());
        if let Some(orch) = orch {
            let backend = orch.ensure_session_backend(target_id, origin_hint).await?;
            let mut guard = self.sessions.write();
            if let Some(existing) = guard.get(target_id) {
                return Ok(Arc::clone(existing));
            }
            let session = BrowserAutomationSession::new_with_registry_tx(
                target_id.to_string(),
                SessionBackendSource::Orchestrator {
                    orchestrator: orch,
                    backend,
                },
                self.default_config(),
                Some(self.event_tx.clone()),
            );
            guard.insert(target_id.to_string(), Arc::clone(&session));
            return Ok(session);
        }

        if let Some(url) = cdp_url.map(str::trim).filter(|s| !s.is_empty()) {
            let backend: Arc<dyn BrowserSessionBackend> = Arc::new(CdpBrowserBackend::connect_cdp(
                BrowserBackendKind::ObscuraCdp,
                url,
                Duration::from_secs(15),
            ));
            return Ok(self.register_backend(target_id.to_string(), backend));
        }

        Err(BotBrowserError::Cdp(format!(
            "Obscura browser engine is not attached and no browser session or CDP endpoint is registered for '{target_id}'."
        )))
    }

    /// Resolves a session for distillation (`ctx_session_id`, or `active_session_id` fallback,
    /// or the most recently active session with a trace).
    pub fn resolve_session_for_distill(
        &self,
        session_id: &str,
    ) -> Option<Arc<BrowserAutomationSession>> {
        if let Some(sess) = self.get_session(session_id) {
            return Some(sess);
        }
        if let Some(active_id) = self.active_session_id() {
            if let Some(sess) = self.get_session(&active_id) {
                return Some(sess);
            }
        }
        let guard = self.sessions.read();
        guard
            .values()
            .find(|s| !s.current_trace().actions.is_empty() || s.last_completed_trace().is_some())
            .cloned()
    }

    pub fn distill_script(
        &self,
        session_id: &str,
        meta: ScriptAppMetadata,
    ) -> Result<AutomationScriptApp, BotBrowserError> {
        let session = self.resolve_session_for_distill(session_id).ok_or_else(|| {
            BotBrowserError::ScriptCompilation(format!(
                "No browser session found for '{session_id}' to distill"
            ))
        })?;
        session.distill_script_app(meta)
    }

    pub async fn replay_script(
        &self,
        session_id: &str,
        app: &AutomationScriptApp,
        params: &BTreeMap<String, String>,
        vault_resolver: Option<&dyn VaultSecretResolver>,
    ) -> Result<ScriptReplayReport, BotBrowserError> {
        let origin_hint = app
            .target_domains
            .first()
            .and_then(|d| NormalizedOrigin::parse(&format!("https://{d}")).ok());
        let session = self
            .resolve_session(Some(session_id), session_id, None, None, origin_hint.as_ref())
            .await?;
        session.replay_script_app(app, params, vault_resolver).await
    }
}

// ============================================================================
// Secret Redaction & Trace Helpers
// ============================================================================

fn hex_val_u8(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val_u8(bytes[i + 1]), hex_val_u8(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_sensitive_url_param_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "password"
            | "passwd"
            | "passcode"
            | "pwd"
            | "pin"
            | "secret"
            | "token"
            | "auth_token"
            | "ct0"
            | "cookie"
            | "set-cookie"
            | "set_cookie"
            | "bearer"
            | "totp"
            | "otp"
            | "api_key"
            | "apikey"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "oauth_token"
            | "session_id"
            | "sid"
            | "code"
            | "x_api_key"
            | "x-api-key"
            | "authorization"
    ) {
        return true;
    }
    lower.contains("password")
        || lower.contains("passwd")
        || lower.contains("passcode")
        || lower.contains("auth_token")
        || lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("id_token")
        || lower.contains("oauth_token")
        || lower.contains("session_id")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("x_api_key")
        || lower.contains("x-api-key")
        || lower.contains("secret")
        || lower.contains("totp")
        || lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|tok| matches!(tok, "otp" | "ct0" | "pwd" | "token" | "pin" | "sid" | "bearer" | "cookie"))
}

fn is_jwt_token_str(s: &str) -> bool {
    if !s.starts_with("eyJ") || s.len() < 24 {
        return false;
    }
    let mut parts = s.split('.');
    let p1 = parts.next().unwrap_or("");
    let p2 = parts.next().unwrap_or("");
    let p3 = parts.next().unwrap_or("");
    parts.next().is_none() && p1.len() >= 4 && p2.len() >= 4 && p3.len() >= 4
}

fn check_secret_value_raw(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }
    let (_, report) = SecretRedactor::new().redact_text(trimmed);
    if report.total_redactions > 0 {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("auth_token=")
        || lower.contains("access_token=")
        || lower.contains("refresh_token=")
        || lower.contains("id_token=")
        || lower.contains("oauth_token=")
        || lower.contains("api_key=")
        || lower.contains("apikey=")
        || lower.contains("ct0=")
        || lower.contains("bearer ")
        || lower.contains("password=")
        || lower.contains("secret=")
    {
        return true;
    }
    is_jwt_token_str(trimmed)
}

pub fn redact_url_for_trace(url: &str) -> String {
    let trimmed = url.trim();
    // 1. Split #fragment and ?query FIRST before any text redaction so `&q=rust#top` is never eaten.
    let (before_frag, frag_opt) = match trimmed.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (trimmed, None),
    };
    let (base_part, query_opt) = match before_frag.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (before_frag, None),
    };

    // 2. Strip/redact URL userinfo (`user:pass@` -> `[REDACTED]@`) and redact secret path segments.
    let mut out = if let Some((scheme, after_scheme)) = base_part.split_once("://") {
        let auth_end = after_scheme.find('/').unwrap_or(after_scheme.len());
        let authority = &after_scheme[..auth_end];
        let rest_path = &after_scheme[auth_end..];
        let clean_auth = if let Some((_userinfo, host_port)) = authority.rsplit_once('@') {
            format!("[REDACTED]@{host_port}")
        } else {
            authority.to_string()
        };
        let (clean_path, _) = SecretRedactor::new().redact_text(rest_path);
        format!("{scheme}://{clean_auth}{clean_path}")
    } else {
        let (clean_base, _) = SecretRedactor::new().redact_text(base_part);
        clean_base
    };

    // 3. Redact sensitive query parameters while preserving benign params (e.g. country_code, zipcode, resident_id, q).
    if let Some(q) = query_opt {
        let redactor = SecretRedactor::new();
        let redacted_pairs: Vec<String> = q
            .split('&')
            .filter(|kv| !kv.is_empty())
            .map(|kv| match kv.split_once('=') {
                Some((k, v)) => {
                    let decoded_k = percent_decode_lossy(k);
                    if is_sensitive_url_param_name(k) || is_sensitive_url_param_name(&decoded_k) {
                        format!("{k}=[REDACTED]")
                    } else if looks_like_secret_value(v) {
                        format!("{k}=[REDACTED]")
                    } else {
                        let (rv, _) = redactor.redact_text(v);
                        format!("{k}={rv}")
                    }
                }
                None => {
                    if looks_like_secret_value(kv) {
                        "[REDACTED]".to_string()
                    } else {
                        let (rkv, _) = redactor.redact_text(kv);
                        rkv
                    }
                }
            })
            .collect();
        if !redacted_pairs.is_empty() {
            out.push('?');
            out.push_str(&redacted_pairs.join("&"));
        }
    }

    // 4. Redact KV secrets, percent-encoded secrets, and raw JWTs inside #fragment while preserving benign #top.
    if let Some(frag) = frag_opt {
        let redactor = SecretRedactor::new();
        let redacted_parts: Vec<String> = frag
            .split('&')
            .map(|part| {
                let kv_opt = part.split_once('=').map(|(fk, fv)| (fk, fv, false)).or_else(|| {
                    let dec = percent_decode_lossy(part);
                    dec.split_once('=').map(|_| ("", "", true))
                });
                if let Some((fk, fv, is_encoded_kv)) = kv_opt {
                    if is_encoded_kv {
                        let dec = percent_decode_lossy(part);
                        if let Some((dk, dv)) = dec.split_once('=') {
                            if is_sensitive_url_param_name(dk) || looks_like_secret_value(dv) {
                                return "[REDACTED]".to_string();
                            }
                        }
                    } else {
                        let decoded_k = percent_decode_lossy(fk);
                        if is_sensitive_url_param_name(fk)
                            || is_sensitive_url_param_name(&decoded_k)
                            || looks_like_secret_value(fv)
                        {
                            return format!("{fk}=[REDACTED]");
                        }
                        let (rfv, _) = redactor.redact_text(fv);
                        return format!("{fk}={rfv}");
                    }
                }
                if looks_like_secret_value(part) {
                    "[REDACTED]".to_string()
                } else {
                    let (rf, _) = redactor.redact_text(part);
                    rf
                }
            })
            .collect();
        out.push('#');
        out.push_str(&redacted_parts.join("&"));
    }

    out
}

pub fn looks_like_secret_value(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if check_secret_value_raw(trimmed) {
        return true;
    }
    let decoded = percent_decode_lossy(trimmed);
    let decoded_trimmed = decoded.trim();
    if decoded_trimmed != trimmed && check_secret_value_raw(decoded_trimmed) {
        return true;
    }
    false
}

fn is_sensitive_field_hint(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    const MARKERS: &[&str] = &[
        "password",
        "passwd",
        "passcode",
        "pwd",
        "one-time-code",
        "totp",
        "secret",
        "api_key",
        "apikey",
        "auth_token",
        "access_token",
        "refresh_token",
        "id_token",
        "oauth_token",
        "session_id",
        "ct0",
    ];
    if MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| matches!(tok, "otp" | "token" | "pin" | "bearer" | "cookie"))
}

pub fn redact_value_for_trace(
    raw_value: &str,
    locator: &SemanticLocator,
    explicit_is_secret: bool,
) -> (String, bool) {
    if explicit_is_secret || locator.is_sensitive_hint() || looks_like_secret_value(raw_value) {
        (String::new(), true)
    } else {
        (raw_value.to_string(), false)
    }
}

pub fn redact_snapshot_for_observation(snapshot: &mut BrowserSnapshot) {
    let redactor = SecretRedactor::new();
    snapshot.url = redact_url_for_trace(&snapshot.url);
    let (clean_excerpt, _) = redactor.redact_text(&snapshot.text_excerpt);
    snapshot.text_excerpt = clean_excerpt;
    for node in &mut snapshot.interactive_elements {
        let mut loc = SemanticLocator::default();
        loc.name_attr = node.name.clone();
        loc.aria_label = node.aria_label.clone();
        loc.test_id = node.test_id.clone();
        loc.css_selector = Some(node.selector.clone());
        let is_sensitive = node.is_password
            || node
                .input_type
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("password"))
            || loc.is_sensitive_hint()
            || node.id.as_deref().is_some_and(is_sensitive_field_hint)
            || node.placeholder.as_deref().is_some_and(is_sensitive_field_hint)
            || is_sensitive_field_hint(&node.selector);

        if is_sensitive {
            if let Some(ref val) = node.value {
                if !val.is_empty() && val != "[REDACTED]" {
                    node.value = Some("[REDACTED]".to_string());
                    snapshot.redacted_field_count =
                        snapshot.redacted_field_count.saturating_add(1);
                }
            }
        } else if let Some(ref val) = node.value {
            let (clean_val, rep) = redactor.redact_text(val);
            if rep.total_redactions > 0 {
                node.value = Some(clean_val);
                snapshot.redacted_field_count = snapshot.redacted_field_count.saturating_add(1);
            } else if val != "[REDACTED]" && looks_like_secret_value(val) {
                node.value = Some("[REDACTED]".to_string());
                snapshot.redacted_field_count = snapshot.redacted_field_count.saturating_add(1);
            }
        }

        let (clean_text, text_rep) = redactor.redact_text(&node.text);
        if text_rep.total_redactions > 0 {
            node.text = clean_text;
            snapshot.redacted_field_count = snapshot.redacted_field_count.saturating_add(1);
        } else if node.text != "[REDACTED]" && looks_like_secret_value(&node.text) {
            node.text = "[REDACTED]".to_string();
            snapshot.redacted_field_count = snapshot.redacted_field_count.saturating_add(1);
        }
    }
}

fn infer_target_domains_from_trace(trace: &BrowserActionTrace) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut domains = Vec::new();
    let mut push_url = |u: &str| {
        if let Ok(origin) = NormalizedOrigin::parse(u) {
            let host = origin.host().to_string();
            if seen.insert(host.clone()) {
                domains.push(host);
            }
        }
    };
    if let Some(ref init) = trace.initial_url {
        push_url(init);
    }
    for rec in &trace.actions {
        if let RecordedActionKind::Navigate { ref url } = rec.action {
            push_url(url);
        }
        if let Some(ref u) = rec.page_url {
            push_url(u);
        }
    }
    domains
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use bot_browser::backend::{
        ActionOutcome, KeyInputEvent, NavigationResult, PointerInputEvent,
    };
    use bot_browser::script_app::ScriptStep;
    use bot_browser::types::BrowserCookie;
    use serde_json::Value;

    struct UnitMockBackend {
        url: SyncMutex<String>,
        title: SyncMutex<String>,
        snapshot_text: SyncMutex<String>,
        clicks: SyncMutex<Vec<SemanticLocator>>,
        fills: SyncMutex<Vec<(SemanticLocator, String, bool)>>,
    }

    impl UnitMockBackend {
        fn new(initial_url: &str) -> Arc<Self> {
            Arc::new(Self {
                url: SyncMutex::new(initial_url.to_string()),
                title: SyncMutex::new("Mock Page".to_string()),
                snapshot_text: SyncMutex::new("Initial DOM".to_string()),
                clicks: SyncMutex::new(Vec::new()),
                fills: SyncMutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl BrowserSessionBackend for UnitMockBackend {
        fn backend_kind(&self) -> BrowserBackendKind {
            BrowserBackendKind::MockCdp
        }
        async fn navigate(&self, url: &str) -> Result<NavigationResult, BotBrowserError> {
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
            Ok(self.url.lock().clone())
        }
        async fn page_title(&self) -> Result<String, BotBrowserError> {
            Ok(self.title.lock().clone())
        }
        async fn snapshot_dom(&self) -> Result<BrowserSnapshot, BotBrowserError> {
            let u = self.url.lock().clone();
            let origin = NormalizedOrigin::parse(&u)
                .map(|o| o.as_str().to_string())
                .unwrap_or_else(|_| u.clone());
            Ok(BrowserSnapshot {
                url: u,
                origin,
                title: self.title.lock().clone(),
                text_excerpt: self.snapshot_text.lock().clone(),
                interactive_elements: Vec::new(),
                redacted_field_count: 0,
            })
        }
        async fn eval_js(&self, _script: &str) -> Result<Value, BotBrowserError> {
            Ok(Value::Null)
        }
        async fn click_locator(&self, locator: &SemanticLocator) -> Result<ActionOutcome, BotBrowserError> {
            self.clicks.lock().push(locator.clone());
            Ok(ActionOutcome {
                ok: true,
                operation: "click".to_string(),
                duration_ms: 1,
                matched_strategy: Some("test_id".to_string()),
                matched_selector: locator.test_id.clone(),
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
            self.fills.lock().push((locator.clone(), value.to_string(), is_secret));
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
        async fn extract_text_locator(
            &self,
            _locator: &SemanticLocator,
        ) -> Result<String, BotBrowserError> {
            Ok(self.snapshot_text.lock().clone())
        }
        async fn capture_screenshot(&self) -> Result<Vec<u8>, BotBrowserError> {
            Ok(vec![137, 80, 78, 71])
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

    #[tokio::test]
    async fn test_session_concurrency_lock_and_step_budget() {
        let reg = BrowserSessionRegistry::with_config(BrowserSessionConfig {
            max_steps_per_run: 2,
            auto_pause_on_sensitive_origin: false,
            event_channel_capacity: 64,
        });
        let backend = UnitMockBackend::new("https://example.com");
        let sess = reg.register_backend("tab_1", backend);

        let mut guard = sess.try_begin_ai_turn().expect("first lock succeeds");
        let err = sess.try_begin_ai_turn().err().expect("overlapping lock must fail");
        assert!(err.to_string().contains("locked"));

        sess.prepare_step("navigate", None, Some("https://example.com".into()))
            .await
            .unwrap();
        sess.prepare_step("click", Some(SemanticLocator::by_test_id("btn")), None)
            .await
            .unwrap();
        let budget_err = sess
            .prepare_step("click", Some(SemanticLocator::by_test_id("btn2")), None)
            .await
            .unwrap_err();
        assert!(budget_err.to_string().contains("budget exhausted"));
        assert_eq!(sess.control_mode(), BrowserControlMode::Idle);

        guard.finish();
        drop(guard);
        assert!(!sess.is_ai_locked());
        let _guard2 = sess.try_begin_ai_turn().expect("lock released after guard drop");
    }

    #[tokio::test]
    async fn test_trace_redaction_and_distill_zero_llm_replay() {
        let reg = BrowserSessionRegistry::new();
        let backend = UnitMockBackend::new("https://example.com/start");
        let sess = reg.register_backend("tab_redact", Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>);
        let mut rx = reg.subscribe();

        {
            let _guard = sess.try_begin_ai_turn().unwrap();
            sess.record_navigate("https://example.com/login?auth_token=supersecret123&ok=1", 5);
            sess.record_fill(
                Some("https://example.com/login".into()),
                SemanticLocator::by_css("#password"),
                "S3cr3tP@ss!",
                false,
                None,
                Some("password".into()),
                4,
            );
            sess.record_fill(
                Some("https://example.com/compose".into()),
                SemanticLocator::by_test_id("post-input"),
                "Hello world",
                false,
                None,
                None,
                3,
            );
            sess.record_click(
                Some("https://example.com/compose".into()),
                SemanticLocator::by_test_id("submit-btn"),
                2,
            );
        }

        let app = sess
            .distill_script_app(ScriptAppMetadata {
                name: "Test Workflow".into(),
                ..Default::default()
            })
            .expect("distill succeeds");
        let serialized = serde_json::to_string(&app).unwrap();
        assert!(!serialized.contains("supersecret123"));
        assert!(!serialized.contains("S3cr3tP@ss!"));
        assert!(app.steps.iter().any(|s| matches!(s, ScriptStep::Click { .. })));

        // Drain events and verify TraceCompleted + ScriptDistilled emitted
        let mut saw_trace = false;
        let mut saw_distilled = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                BrowserAutomationEvent::TraceCompleted { .. } => saw_trace = true,
                BrowserAutomationEvent::ScriptDistilled { .. } => saw_distilled = true,
                _ => {}
            }
        }
        assert!(saw_trace && saw_distilled);
    }

    #[tokio::test]
    async fn test_takeover_suspend_resume_and_cancel_without_deadlock() {
        let reg = BrowserSessionRegistry::new();
        let backend = UnitMockBackend::new("https://social.example.com/home");
        let sess = reg.register_backend("tab_takeover", Arc::clone(&backend) as Arc<dyn BrowserSessionBackend>);

        let _guard = sess.try_begin_ai_turn().unwrap();
        let sess_clone = Arc::clone(&sess);
        let backend_clone = Arc::clone(&backend);

        let waiter = tokio::spawn(async move {
            sess_clone
                .request_takeover_and_wait("Need manual verification", true, true)
                .await
        });

        // Wait until WaitingForTakeover
        for _ in 0..50 {
            if matches!(sess.control_mode(), BrowserControlMode::WaitingForTakeover { .. }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(matches!(
            sess.control_mode(),
            BrowserControlMode::WaitingForTakeover { .. }
        ));

        // User takes over and modifies the page (must NOT deadlock on ai_lock!)
        sess.user_take_over();
        assert!(matches!(
            sess.control_mode(),
            BrowserControlMode::UserTakeover { .. }
        ));
        *backend_clone.url.lock() = "https://social.example.com/verified".to_string();
        *backend_clone.snapshot_text.lock() = "User verified account".to_string();

        sess.user_resume_ai();
        let outcome = waiter.await.unwrap().expect("resumed cleanly");
        assert!(outcome.resumed);
        assert_eq!(
            outcome.current_url.as_deref(),
            Some("https://social.example.com/verified")
        );
        assert_eq!(
            outcome.snapshot.as_ref().map(|s| s.text_excerpt.as_str()),
            Some("User verified account")
        );

        // Now test cancel mid-takeover
        let sess_cancel = Arc::clone(&sess);
        let cancel_waiter = tokio::spawn(async move {
            sess_cancel
                .request_takeover_and_wait("Another checkpoint", true, true)
                .await
        });
        for _ in 0..50 {
            if matches!(sess.control_mode(), BrowserControlMode::WaitingForTakeover { .. }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        sess.user_cancel_ai();
        let err = cancel_waiter.await.unwrap().unwrap_err();
        assert!(matches!(err, BotBrowserError::ControlCancelled(_)));
        assert_eq!(sess.control_mode(), BrowserControlMode::Idle);
    }

    #[test]
    fn test_redact_url_for_trace_preserves_benign_params_and_strips_secrets() {
        let benign = "https://shop.example.com/search?country_code=US&currency_code=USD&promo_code=SAVE10&zipcode=10001&resident_id=42&q=rust#top";
        assert_eq!(redact_url_for_trace(benign), benign);

        let mixed = "https://alice:hunter2@example.com/search?api_key=my_long_secret&q=rust&ct0=ct0_val_123&auth_token=auth_val_456#access_token=frag_tok_789&section=bio";
        let redacted = redact_url_for_trace(mixed);
        assert_eq!(
            redacted,
            "https://[REDACTED]@example.com/search?api_key=[REDACTED]&q=rust&ct0=[REDACTED]&auth_token=[REDACTED]#access_token=[REDACTED]&section=bio"
        );

        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let frag_jwt = format!("https://example.com/callback#{jwt}");
        assert_eq!(
            redact_url_for_trace(&frag_jwt),
            "https://example.com/callback#[REDACTED]"
        );

        let nested = "https://example.com/callback?next=https%3A%2F%2Fapp.example.com%2F%3Fauth_token%3Dsecret_nested_999&state=ok";
        let redacted_nested = redact_url_for_trace(nested);
        assert!(!redacted_nested.contains("secret_nested_999"), "{redacted_nested}");
        assert!(redacted_nested.contains("next=[REDACTED]&state=ok"), "{redacted_nested}");
    }

    #[test]
    fn test_redact_snapshot_for_observation_inspects_id_placeholder_and_percent_encoded() {
        use bot_browser::types::DomNodeSnapshot;
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let mut snap = BrowserSnapshot {
            url: "https://example.com/form?api_key=abc123456&zipcode=10001#top".into(),
            origin: "https://example.com".into(),
            title: "Form".into(),
            text_excerpt: "Debug: ct0=CT0_EXCERPT_777 auth_token=AUTH_EXCERPT_888".into(),
            interactive_elements: vec![
                DomNodeSnapshot {
                    ref_id: "n1".into(),
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
                    ref_id: "n2".into(),
                    tag: "input".into(),
                    input_type: Some("text".into()),
                    name: Some("note".into()),
                    id: Some("note".into()),
                    aria_label: None,
                    placeholder: Some("Note".into()),
                    test_id: None,
                    role: None,
                    text: format!("JWT: {jwt}"),
                    value: Some("%70%61%73%73%77%6f%72%64=hunter2_secret".into()),
                    is_password: false,
                    selector: "#note".into(),
                    frame_origin: None,
                    rect: None,
                },
            ],
            redacted_field_count: 0,
        };
        redact_snapshot_for_observation(&mut snap);
        assert_eq!(snap.url, "https://example.com/form?api_key=[REDACTED]&zipcode=10001#top");
        assert!(!snap.text_excerpt.contains("CT0_EXCERPT_777"));
        assert!(!snap.text_excerpt.contains("AUTH_EXCERPT_888"));
        assert_eq!(snap.interactive_elements[0].value.as_deref(), Some("[REDACTED]"));
        assert_eq!(snap.interactive_elements[1].value.as_deref(), Some("[REDACTED]"));
        assert!(!snap.interactive_elements[1].text.contains(jwt));
        assert!(snap.redacted_field_count >= 3);
    }
}
