use crate::error::{EngineResult, RemoteEngineError};
use gpui_telegram_proto::remote_control_contracts::{
    RemoteControlMode, RemoteControllerKind, RemoteSessionDto,
};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::{broadcast, watch};

/// Global monotonic counter bumped whenever the local user overrides remote/AI control
/// or invokes the "disconnect all" kill switch.
pub static USER_OVERRIDE_EPOCH: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub enum ControlHubEvent {
    SessionStarted(RemoteSessionDto),
    SessionUpdated(RemoteSessionDto),
    SessionDisconnected { session_id: String },
    AllDisconnected { count: usize, override_epoch: u64 },
}

/// Cancellation/preemption token checked by in-flight remote or AI MCP operations.
#[derive(Debug, Clone)]
pub struct PreemptionToken {
    epoch: u64,
    rx: watch::Receiver<u64>,
    global_epoch: Option<&'static AtomicU64>,
}

impl PreemptionToken {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow() != self.epoch
            || self
                .global_epoch
                .is_some_and(|g| g.load(Ordering::SeqCst) != self.epoch)
    }

    pub fn check(&self) -> EngineResult<()> {
        if self.is_cancelled() {
            Err(RemoteEngineError::Preempted(
                "user kill switch or override epoch bumped".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    pub async fn cancelled(&mut self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            if self.rx.changed().await.is_err() || self.is_cancelled() {
                return;
            }
        }
    }
}

/// Coordinates active remote-control / AI MCP / assist sessions and instant kill-switch preemption.
#[derive(Debug)]
pub struct RemoteControlHub {
    sessions: RwLock<HashMap<String, RemoteSessionDto>>,
    override_epoch: AtomicU64,
    global_epoch: Option<&'static AtomicU64>,
    epoch_lock: Mutex<()>,
    epoch_tx: watch::Sender<u64>,
    event_tx: broadcast::Sender<ControlHubEvent>,
}

impl Default for RemoteControlHub {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteControlHub {
    fn with_global_epoch(global_epoch: Option<&'static AtomicU64>) -> Self {
        let initial_epoch = global_epoch.map_or(0, |g| g.load(Ordering::SeqCst));
        let (epoch_tx, _) = watch::channel(initial_epoch);
        let (event_tx, _) = broadcast::channel(128);
        Self {
            sessions: RwLock::new(HashMap::new()),
            override_epoch: AtomicU64::new(initial_epoch),
            global_epoch,
            epoch_lock: Mutex::new(()),
            epoch_tx,
            event_tx,
        }
    }

    pub fn new() -> Self {
        Self::with_global_epoch(None)
    }

    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<RemoteControlHub>> = OnceLock::new();
        Arc::clone(
            SHARED.get_or_init(|| Arc::new(Self::with_global_epoch(Some(&USER_OVERRIDE_EPOCH)))),
        )
    }

    pub fn override_epoch(&self) -> u64 {
        self.override_epoch.load(Ordering::SeqCst)
    }

    pub fn bump_user_override(&self) -> u64 {
        let _guard = self.epoch_lock.lock();
        let local = self.override_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let next = if let Some(global_epoch) = self.global_epoch {
            let global = global_epoch.fetch_add(1, Ordering::SeqCst) + 1;
            let max_epoch = global.max(local);
            self.override_epoch.store(max_epoch, Ordering::SeqCst);
            max_epoch
        } else {
            local
        };
        self.epoch_tx.send_replace(next);
        next
    }

    pub fn preemption_token(&self) -> PreemptionToken {
        let _guard = self.epoch_lock.lock();
        let rx = self.epoch_tx.subscribe();
        let epoch = *rx.borrow();
        PreemptionToken {
            epoch,
            rx,
            global_epoch: self.global_epoch,
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<ControlHubEvent> {
        self.event_tx.subscribe()
    }

    pub fn register_session(&self, mut session: RemoteSessionDto) -> RemoteSessionDto {
        let now = chrono::Utc::now().timestamp_millis();
        if session.started_at_ms == 0 {
            session.started_at_ms = now;
        }
        if session.last_activity_ms == 0 {
            session.last_activity_ms = now;
        }
        let mut guard = self.sessions.write();
        guard.insert(session.session_id.clone(), session.clone());
        let _ = self
            .event_tx
            .send(ControlHubEvent::SessionStarted(session.clone()));
        session
    }

    pub fn acquire_exclusive_lease(
        &self,
        session: RemoteSessionDto,
        allow_preempt_ai: bool,
    ) -> EngineResult<RemoteSessionDto> {
        let mut guard = self.sessions.write();
        if let Some(existing) = guard.values().find(|s| s.session_id != session.session_id) {
            let can_preempt = allow_preempt_ai
                && existing.controller_kind == RemoteControllerKind::AiMcp
                && session.controller_kind != RemoteControllerKind::AiMcp;
            if !can_preempt && existing.mode == RemoteControlMode::Desktop {
                return Err(RemoteEngineError::SessionRejected(format!(
                    "device already controlled by {} ({})",
                    existing.controller_name, existing.session_id
                )));
            }
            if can_preempt {
                let evicted: Vec<String> = guard
                    .iter()
                    .filter(|(_, s)| s.controller_kind == RemoteControllerKind::AiMcp)
                    .map(|(k, _)| k.clone())
                    .collect();
                for id in evicted {
                    guard.remove(&id);
                    let _ = self
                        .event_tx
                        .send(ControlHubEvent::SessionDisconnected { session_id: id });
                }
                self.bump_user_override();
            }
        }
        let mut session = session;
        let now = chrono::Utc::now().timestamp_millis();
        if session.started_at_ms == 0 {
            session.started_at_ms = now;
        }
        if session.last_activity_ms == 0 {
            session.last_activity_ms = now;
        }
        guard.insert(session.session_id.clone(), session.clone());
        let _ = self
            .event_tx
            .send(ControlHubEvent::SessionStarted(session.clone()));
        Ok(session)
    }

    pub fn touch_session(
        &self,
        session_id: &str,
        active_tool: Option<String>,
        rtt_ms: Option<u32>,
        fps: Option<u32>,
    ) -> bool {
        let mut guard = self.sessions.write();
        let Some(sess) = guard.get_mut(session_id) else {
            return false;
        };
        sess.last_activity_ms = chrono::Utc::now().timestamp_millis();
        if active_tool.is_some() {
            sess.active_tool = active_tool;
        }
        if rtt_ms.is_some() {
            sess.rtt_ms = rtt_ms;
        }
        if fps.is_some() {
            sess.fps = fps;
        }
        let _ = self
            .event_tx
            .send(ControlHubEvent::SessionUpdated(sess.clone()));
        true
    }

    pub fn list_sessions(&self) -> Vec<RemoteSessionDto> {
        let mut list: Vec<_> = self.sessions.read().values().cloned().collect();
        list.sort_by_key(|s| s.started_at_ms);
        list
    }

    pub fn get_session(&self, session_id: &str) -> Option<RemoteSessionDto> {
        self.sessions.read().get(session_id).cloned()
    }

    pub fn active_count(&self) -> usize {
        self.sessions.read().len()
    }

    pub fn disconnect_session(&self, session_id: &str) -> bool {
        let removed = self.sessions.write().remove(session_id).is_some();
        if removed {
            self.bump_user_override();
            let _ = self.event_tx.send(ControlHubEvent::SessionDisconnected {
                session_id: session_id.to_string(),
            });
        }
        removed
    }

    /// Removes a short-lived ephemeral local IPC/CLI session without bumping `override_epoch`
    /// or cancelling active preemption tokens.
    pub fn finish_ephemeral_session(&self, session_id: &str) -> bool {
        let removed = self.sessions.write().remove(session_id).is_some();
        if removed {
            let _ = self.event_tx.send(ControlHubEvent::SessionDisconnected {
                session_id: session_id.to_string(),
            });
        }
        removed
    }

    pub fn disconnect_all(&self) -> usize {
        let count = {
            let mut guard = self.sessions.write();
            let n = guard.len();
            guard.clear();
            n
        };
        let epoch = self.bump_user_override();
        let _ = self.event_tx.send(ControlHubEvent::AllDisconnected {
            count,
            override_epoch: epoch,
        });
        count
    }
}
