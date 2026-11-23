use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

const MAX_TRACKED_EXPIRED_NONCES: usize = 256;
const MAX_TRACKED_CANCELLED_TURNS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FloorPriority {
    UserInterruption = 0,
    DirectMention = 1,
    TaskAssignee = 2,
    GeneralContribution = 3,
}

/// Grok Bot-style canonical outcome for a room member's multi-agent turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomMemberTurnOutcome {
    Sent,
    Pass,
    Skipped,
    Timeout,
    Cancelled,
    Error,
}

/// Nonce-fencing validation verdict when a member attempts to deliver a room turn result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnDeliveryIntake {
    Accepted,
    UnknownNonce,
    Expired,
}

/// Fenced floor turn grant returned by `FloorControlQueue::acquire_floor_wait`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorTurnGrant {
    pub agent_id: String,
    pub nonce: String,
    pub acquired_at_ms: i64,
    pub stale_after_ms: u64,
    pub is_winding_down: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct FloorRequest {
    priority: FloorPriority,
    seq: u64,
    agent_id: String,
    enqueued_at_promotion: u64,
}

impl Ord for FloorRequest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Lower numerical priority = higher precedence
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for FloorRequest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone)]
struct ActiveFloorTurn {
    agent_id: String,
    priority: FloorPriority,
    nonce: String,
    acquired_at_ms: i64,
    lease_end_ms: u64,
    stale_after_ms: u64,
    is_winding_down: bool,
}

impl ActiveFloorTurn {
    fn to_grant(&self) -> FloorTurnGrant {
        FloorTurnGrant {
            agent_id: self.agent_id.clone(),
            nonce: self.nonce.clone(),
            acquired_at_ms: self.acquired_at_ms,
            stale_after_ms: self.stale_after_ms,
            is_winding_down: self.is_winding_down,
        }
    }
}

#[derive(Debug)]
struct FloorInner {
    lease_duration_ms: u64,
    active_turn: Option<ActiveFloorTurn>,
    wait_queue: BinaryHeap<FloorRequest>,
    queued_agents: HashSet<String>,
    seq_counter: u64,
    nonce_counter: u64,
    promotion_count: u64,
    last_explicit_clock_ms: Option<u64>,
    expired_nonces: HashSet<String>,
    expired_nonce_order: VecDeque<String>,
    cancelled_turns: HashMap<String, String>,
    cancelled_turn_order: VecDeque<String>,
}

impl FloorInner {
    fn new(lease_duration_ms: u64) -> Self {
        Self {
            lease_duration_ms,
            active_turn: None,
            wait_queue: BinaryHeap::new(),
            queued_agents: HashSet::new(),
            seq_counter: 0,
            nonce_counter: 0,
            promotion_count: 0,
            last_explicit_clock_ms: None,
            expired_nonces: HashSet::new(),
            expired_nonce_order: VecDeque::new(),
            cancelled_turns: HashMap::new(),
            cancelled_turn_order: VecDeque::new(),
        }
    }

    fn next_nonce(&mut self) -> String {
        self.nonce_counter = self.nonce_counter.saturating_add(1);
        format!(
            "trn_{}_{}",
            self.nonce_counter,
            uuid::Uuid::new_v4().simple()
        )
    }

    fn record_expired_nonce(&mut self, nonce: String) {
        if self.expired_nonces.insert(nonce.clone()) {
            self.expired_nonce_order.push_back(nonce);
            while self.expired_nonce_order.len() > MAX_TRACKED_EXPIRED_NONCES {
                if let Some(old) = self.expired_nonce_order.pop_front() {
                    self.expired_nonces.remove(&old);
                }
            }
        }
    }

    fn record_cancelled_turn(&mut self, nonce: String, reason: String) {
        self.cancelled_turns.insert(nonce.clone(), reason);
        self.cancelled_turn_order.push_back(nonce);
        while self.cancelled_turn_order.len() > MAX_TRACKED_CANCELLED_TURNS {
            if let Some(old) = self.cancelled_turn_order.pop_front() {
                self.cancelled_turns.remove(&old);
            }
        }
    }

    fn install_active_turn(
        &mut self,
        agent_id: String,
        priority: FloorPriority,
        now_ms: u64,
        is_winding_down: bool,
    ) -> FloorTurnGrant {
        let nonce = self.next_nonce();
        let active = ActiveFloorTurn {
            agent_id,
            priority,
            nonce,
            acquired_at_ms: i64::try_from(now_ms).unwrap_or(i64::MAX),
            lease_end_ms: now_ms.saturating_add(self.lease_duration_ms),
            stale_after_ms: self.lease_duration_ms,
            is_winding_down,
        };
        let grant = active.to_grant();
        self.active_turn = Some(active);
        grant
    }

    fn remove_from_queue(&mut self, agent_id: &str) -> Option<FloorRequest> {
        if self.queued_agents.remove(agent_id) {
            let mut removed = None;
            let remaining: Vec<FloorRequest> = self
                .wait_queue
                .drain()
                .filter(|r| {
                    if r.agent_id == agent_id && removed.is_none() {
                        removed = Some(r.clone());
                        false
                    } else {
                        true
                    }
                })
                .collect();
            for req in remaining {
                self.wait_queue.push(req);
            }
            removed
        } else {
            None
        }
    }

    fn current_speaker_id(&self) -> Option<String> {
        self.active_turn.as_ref().map(|t| t.agent_id.clone())
    }

    fn drain_expired_leases_internal(&mut self, now_ms: u64) -> bool {
        let is_expired = self
            .active_turn
            .as_ref()
            .is_some_and(|active| now_ms >= active.lease_end_ms);
        if is_expired {
            if let Some(expired) = self.active_turn.take() {
                self.record_expired_nonce(expired.nonce);
            }
            self.promote_next_speaker(now_ms);
            true
        } else {
            false
        }
    }

    fn request_floor_internal(
        &mut self,
        agent_id: &str,
        priority: FloorPriority,
        now_ms: u64,
        is_winding_down: bool,
    ) -> (bool, bool) {
        let mut changed = self.drain_expired_leases_internal(now_ms);

        if let Some(active) = &mut self.active_turn {
            if active.agent_id == agent_id {
                if is_winding_down {
                    active.is_winding_down = true;
                }
                return (true, changed);
            }
        }

        if priority == FloorPriority::UserInterruption {
            self.remove_from_queue(agent_id);
            if let Some(prev) = self.active_turn.take() {
                self.record_cancelled_turn(prev.nonce, "user_interruption_preempted".into());
                if prev.agent_id != agent_id && !self.queued_agents.contains(&prev.agent_id) {
                    self.seq_counter += 1;
                    self.wait_queue.push(FloorRequest {
                        priority: prev.priority,
                        seq: self.seq_counter,
                        agent_id: prev.agent_id.clone(),
                        enqueued_at_promotion: self.promotion_count,
                    });
                    self.queued_agents.insert(prev.agent_id);
                }
            }
            self.install_active_turn(agent_id.to_string(), priority, now_ms, is_winding_down);
            return (true, true);
        }

        if self.active_turn.is_none() {
            self.remove_from_queue(agent_id);
            self.install_active_turn(agent_id.to_string(), priority, now_ms, is_winding_down);
            (true, true)
        } else {
            if self.queued_agents.contains(agent_id) {
                if let Some(mut existing) = self.remove_from_queue(agent_id) {
                    if priority < existing.priority {
                        existing.priority = priority;
                        changed = true;
                    }
                    self.queued_agents.insert(existing.agent_id.clone());
                    self.wait_queue.push(existing);
                }
            } else {
                self.seq_counter += 1;
                self.wait_queue.push(FloorRequest {
                    priority,
                    seq: self.seq_counter,
                    agent_id: agent_id.to_string(),
                    enqueued_at_promotion: self.promotion_count,
                });
                self.queued_agents.insert(agent_id.to_string());
            }
            (false, changed)
        }
    }

    fn promote_next_speaker(&mut self, now_ms: u64) {
        const STARVATION_PROMOTION_THRESHOLD: u64 = 8;
        let starved_agent = self
            .wait_queue
            .iter()
            .filter(|r| {
                self.promotion_count
                    .saturating_sub(r.enqueued_at_promotion)
                    >= STARVATION_PROMOTION_THRESHOLD
            })
            .min_by_key(|r| r.seq)
            .map(|r| r.agent_id.clone());

        if let Some(agent_id) = starved_agent {
            if let Some(req) = self.remove_from_queue(&agent_id) {
                self.promotion_count = self.promotion_count.saturating_add(1);
                self.install_active_turn(req.agent_id, req.priority, now_ms, false);
                return;
            }
        }

        while let Some(req) = self.wait_queue.pop() {
            if self.queued_agents.remove(&req.agent_id) {
                self.promotion_count = self.promotion_count.saturating_add(1);
                self.install_active_turn(req.agent_id, req.priority, now_ms, false);
                break;
            }
        }
    }

    fn validate_turn_nonce_at(
        &mut self,
        agent_id: &str,
        nonce: &str,
        now_ms: u64,
    ) -> (TurnDeliveryIntake, bool) {
        let changed = self.drain_expired_leases_internal(now_ms);
        if let Some(active) = &self.active_turn {
            if active.agent_id == agent_id && active.nonce == nonce {
                return (TurnDeliveryIntake::Accepted, changed);
            }
        }
        if self.expired_nonces.contains(nonce) {
            return (TurnDeliveryIntake::Expired, changed);
        }
        (TurnDeliveryIntake::UnknownNonce, changed)
    }
}

/// Event-driven, nonce-fenced multi-agent floor controller with anti-starvation priority scheduling.
///
/// Cloning `FloorControlQueue` shares the underlying synchronized state and `tokio::sync::Notify`
/// handle so async callers can release outer locks before calling [`FloorControlQueue::acquire_floor_wait`].
#[derive(Debug, Clone)]
pub struct FloorControlQueue {
    inner: Arc<Mutex<FloorInner>>,
    notify: Arc<Notify>,
    cached_speaker: Option<String>,
}

impl Default for FloorControlQueue {
    fn default() -> Self {
        Self::new(10_000)
    }
}

impl FloorControlQueue {
    pub fn new(lease_duration_ms: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FloorInner::new(lease_duration_ms))),
            notify: Arc::new(Notify::new()),
            cached_speaker: None,
        }
    }

    fn wall_clock_ms() -> u64 {
        chrono::Utc::now().timestamp_millis().max(0) as u64
    }

    fn sync_cached_speaker(&mut self) {
        self.cached_speaker = self.inner.lock().current_speaker_id();
    }

    pub fn lease_duration_ms(&self) -> u64 {
        self.inner.lock().lease_duration_ms
    }

    pub fn request_floor(&mut self, agent_id: &str, priority: FloorPriority) -> bool {
        let now_ms = Self::wall_clock_ms();
        let (granted, notify_needed) = {
            let mut inner = self.inner.lock();
            inner.last_explicit_clock_ms = None;
            inner.request_floor_internal(agent_id, priority, now_ms, false)
        };
        self.sync_cached_speaker();
        if notify_needed {
            self.notify.notify_waiters();
        }
        granted
    }

    pub fn request_floor_at(
        &mut self,
        agent_id: &str,
        priority: FloorPriority,
        now_ms: u64,
    ) -> bool {
        let (granted, notify_needed) = {
            let mut inner = self.inner.lock();
            inner.last_explicit_clock_ms = Some(now_ms);
            inner.request_floor_internal(agent_id, priority, now_ms, false)
        };
        self.sync_cached_speaker();
        if notify_needed {
            self.notify.notify_waiters();
        }
        granted
    }

    /// Event-driven async floor acquisition backed by `tokio::sync::Notify` (zero busy-polling).
    ///
    /// Automatically wakes on floor release, turn cancellation, user preemption, or stale-lease
    /// expiration, returning a nonce-fenced [`FloorTurnGrant`] or `None` if `timeout` elapses.
    pub async fn acquire_floor_wait(
        &self,
        agent_id: &str,
        priority: FloorPriority,
        timeout: Duration,
    ) -> Option<FloorTurnGrant> {
        self.acquire_floor_wait_with_options(agent_id, priority, timeout, false)
            .await
    }

    /// Like [`FloorControlQueue::acquire_floor_wait`], allowing callers to tag the granted turn
    /// with `is_winding_down` when the room's multi-agent loop budget is nearly exhausted.
    pub async fn acquire_floor_wait_with_options(
        &self,
        agent_id: &str,
        priority: FloorPriority,
        timeout: Duration,
        is_winding_down: bool,
    ) -> Option<FloorTurnGrant> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            // Register Notify listener BEFORE inspecting or mutating queue state to prevent
            // lost wakeups between lock release and await.
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let (grant_opt, notify_needed, sleep_until_lease_end) = {
                let mut inner = self.inner.lock();
                inner.last_explicit_clock_ms = None;
                let now_ms = Self::wall_clock_ms();
                let (granted, changed) =
                    inner.request_floor_internal(agent_id, priority, now_ms, is_winding_down);
                if granted {
                    let grant = inner.active_turn.as_ref().map(ActiveFloorTurn::to_grant);
                    (grant, changed, None)
                } else {
                    let lease_rem = inner.active_turn.as_ref().map(|t| {
                        Duration::from_millis(t.lease_end_ms.saturating_sub(now_ms).max(1))
                    });
                    (None, changed, lease_rem)
                }
            };

            if notify_needed {
                self.notify.notify_waiters();
            }
            if let Some(grant) = grant_opt {
                return Some(grant);
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                self.cancel_request_shared(agent_id);
                return None;
            }
            let remaining = deadline.saturating_duration_since(now);
            let wait_cap = match sleep_until_lease_end {
                Some(lease_rem) => remaining.min(lease_rem),
                None => remaining,
            };

            let _ = tokio::time::timeout(wait_cap, notified).await;
        }
    }

    /// Validates that `(agent_id, nonce)` is still the active, non-expired floor turn grant.
    ///
    /// Automatically drains stale leases so expired turns yield [`TurnDeliveryIntake::Expired`]
    /// and cancelled/preempted/unknown turns yield [`TurnDeliveryIntake::UnknownNonce`].
    pub fn validate_turn_nonce(&self, agent_id: &str, nonce: &str) -> TurnDeliveryIntake {
        let (verdict, changed) = {
            let mut inner = self.inner.lock();
            let now_ms = inner
                .last_explicit_clock_ms
                .unwrap_or_else(Self::wall_clock_ms);
            inner.validate_turn_nonce_at(agent_id, nonce, now_ms)
        };
        if changed {
            self.notify.notify_waiters();
        }
        verdict
    }

    /// Deterministic clock variant of [`FloorControlQueue::validate_turn_nonce`] for testing or
    /// explicit wall-clock injection.
    pub fn validate_turn_nonce_at(
        &self,
        agent_id: &str,
        nonce: &str,
        now_ms: u64,
    ) -> TurnDeliveryIntake {
        let (verdict, changed) = {
            let mut inner = self.inner.lock();
            if inner.last_explicit_clock_ms.is_some() {
                inner.last_explicit_clock_ms = Some(now_ms);
            }
            inner.validate_turn_nonce_at(agent_id, nonce, now_ms)
        };
        if changed {
            self.notify.notify_waiters();
        }
        verdict
    }

    /// Explicitly cancels an active turn by its fencing `nonce`, records the reason, promotes
    /// the next queued speaker (if any), and wakes waiting async acquirers.
    pub fn cancel_turn(&self, nonce: &str, reason: &str) -> bool {
        let cancelled = {
            let mut inner = self.inner.lock();
            let matches_active = inner
                .active_turn
                .as_ref()
                .is_some_and(|active| active.nonce == nonce);
            if matches_active {
                let now_ms = inner
                    .last_explicit_clock_ms
                    .unwrap_or_else(Self::wall_clock_ms);
                if let Some(active) = inner.active_turn.take() {
                    inner.record_cancelled_turn(active.nonce, reason.to_string());
                }
                inner.promote_next_speaker(now_ms);
                true
            } else {
                false
            }
        };
        if cancelled {
            self.notify.notify_waiters();
        }
        cancelled
    }

    /// Preempts any currently active bot turn due to human user interruption, invalidating its
    /// fencing nonce so cancelled turns can never inject late posts, and wakes all waiters.
    ///
    /// Returns the invalidated active turn's nonce if a turn was preempted.
    pub fn preempt_for_user_interruption(&self) -> Option<String> {
        let invalidated_nonce = {
            let mut inner = self.inner.lock();
            let now_ms = inner
                .last_explicit_clock_ms
                .unwrap_or_else(Self::wall_clock_ms);
            let old = inner.active_turn.take().map(|active| {
                let n = active.nonce.clone();
                inner.record_cancelled_turn(active.nonce, "user_interruption".to_string());
                n
            });
            inner.promote_next_speaker(now_ms);
            old
        };
        self.notify.notify_waiters();
        invalidated_nonce
    }

    /// Fenced floor release: only releases the floor if the active turn matches both `agent_id`
    /// and `nonce`, preventing a stale/cancelled turn guard from releasing the next speaker's floor.
    pub fn release_turn(&self, agent_id: &str, nonce: &str) -> bool {
        let released = {
            let mut inner = self.inner.lock();
            let matches_active = inner
                .active_turn
                .as_ref()
                .is_some_and(|active| active.agent_id == agent_id && active.nonce == nonce);
            if matches_active {
                let now_ms = inner
                    .last_explicit_clock_ms
                    .unwrap_or_else(Self::wall_clock_ms);
                inner.active_turn = None;
                inner.promote_next_speaker(now_ms);
                true
            } else {
                false
            }
        };
        if released {
            self.notify.notify_waiters();
        }
        released
    }

    /// Shared (`&self`) floor release by `agent_id` for async callers holding a cloned queue handle.
    pub fn release_floor_shared(&self, agent_id: &str) -> bool {
        let released = {
            let mut inner = self.inner.lock();
            let matches_active = inner
                .active_turn
                .as_ref()
                .is_some_and(|active| active.agent_id == agent_id);
            if matches_active {
                let now_ms = inner
                    .last_explicit_clock_ms
                    .unwrap_or_else(Self::wall_clock_ms);
                inner.active_turn = None;
                inner.promote_next_speaker(now_ms);
                true
            } else {
                false
            }
        };
        if released {
            self.notify.notify_waiters();
        }
        released
    }

    pub fn cancel_request_shared(&self, agent_id: &str) -> bool {
        let removed = self.inner.lock().remove_from_queue(agent_id).is_some();
        if removed {
            self.notify.notify_waiters();
        }
        removed
    }

    pub fn cancel_request(&mut self, agent_id: &str) -> bool {
        let removed = self.cancel_request_shared(agent_id);
        self.sync_cached_speaker();
        removed
    }

    pub fn release_floor(&mut self, agent_id: &str) {
        let now_ms = {
            let inner = self.inner.lock();
            inner
                .last_explicit_clock_ms
                .unwrap_or_else(Self::wall_clock_ms)
        };
        self.release_floor_at(agent_id, now_ms);
    }

    pub fn release_floor_at(&mut self, agent_id: &str, now_ms: u64) {
        let released = {
            let mut inner = self.inner.lock();
            if inner.last_explicit_clock_ms.is_some() {
                inner.last_explicit_clock_ms = Some(now_ms);
            }
            if inner
                .active_turn
                .as_ref()
                .is_some_and(|active| active.agent_id == agent_id)
            {
                inner.active_turn = None;
                inner.promote_next_speaker(now_ms);
                true
            } else {
                false
            }
        };
        self.sync_cached_speaker();
        if released {
            self.notify.notify_waiters();
        }
    }

    /// Returns the active floor turn grant (including `nonce`, `stale_after_ms`, and `is_winding_down`).
    pub fn active_grant(&self) -> Option<FloorTurnGrant> {
        self.inner
            .lock()
            .active_turn
            .as_ref()
            .map(ActiveFloorTurn::to_grant)
    }

    /// Updates `is_winding_down` on the currently active turn if its nonce matches.
    pub fn set_active_winding_down(&self, nonce: &str, is_winding_down: bool) -> bool {
        let mut inner = self.inner.lock();
        if let Some(active) = &mut inner.active_turn {
            if active.nonce == nonce {
                active.is_winding_down = is_winding_down;
                return true;
            }
        }
        false
    }

    /// Returns the recorded cancellation reason for a previously cancelled turn nonce, if any.
    pub fn last_cancel_reason(&self, nonce: &str) -> Option<String> {
        self.inner.lock().cancelled_turns.get(nonce).cloned()
    }

    /// Returns the live current speaker ID from shared state (preferred for concurrent `&self` callers).
    pub fn current_speaker_id(&self) -> Option<String> {
        self.inner.lock().current_speaker_id()
    }

    /// Backwards-compatible borrowed view of the current speaker (kept synchronized on `&mut self` calls
    /// and validated against live state so preempted/cancelled turns never appear stale).
    pub fn current_speaker(&self) -> Option<&str> {
        let live = self.inner.lock().current_speaker_id();
        match (&self.cached_speaker, live.as_deref()) {
            (Some(cached), Some(cur)) if cached == cur => Some(cached.as_str()),
            _ => None,
        }
    }

    pub fn queued_count(&self) -> usize {
        self.inner.lock().queued_agents.len()
    }

    pub fn drain_expired_leases(&mut self, now_ms: u64) {
        let changed = {
            let mut inner = self.inner.lock();
            if inner.last_explicit_clock_ms.is_some() {
                inner.last_explicit_clock_ms = Some(now_ms);
            }
            inner.drain_expired_leases_internal(now_ms)
        };
        self.sync_cached_speaker();
        if changed {
            self.notify.notify_waiters();
        }
    }

    /// Exposes the underlying `Notify` handle for custom orchestration hooks.
    pub fn notifier(&self) -> Arc<Notify> {
        Arc::clone(&self.notify)
    }
}
