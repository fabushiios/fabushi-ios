use std::collections::HashMap;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};

fn default_delivery_status() -> String {
    "sent".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedMessage {
    pub message_id: i64,
    pub chat_id: i64,
    pub sender_id: u64,
    pub text: String,
    pub timestamp: i64,
    pub reply_to: Option<i64>,
    pub client_nonce: u64,
    #[serde(default = "default_delivery_status")]
    pub delivery_status: String,
    #[serde(skip)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum InFlightState {
    InProgress,
    Completed(CachedMessage),
    Aborted,
}

#[derive(Debug)]
pub struct InFlightSlot {
    state: Mutex<InFlightState>,
    cvar: Condvar,
    pub created_at: DateTime<Utc>,
}

impl InFlightSlot {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            state: Mutex::new(InFlightState::InProgress),
            cvar: Condvar::new(),
            created_at: now,
        }
    }

    pub fn complete(&self, msg: CachedMessage) {
        let mut guard = self.state.lock();
        *guard = InFlightState::Completed(msg);
        self.cvar.notify_all();
    }

    pub fn abort(&self) {
        let mut guard = self.state.lock();
        *guard = InFlightState::Aborted;
        self.cvar.notify_all();
    }

    pub fn wait(&self) -> Result<CachedMessage, DedupWaitError> {
        let mut guard = self.state.lock();
        while let InFlightState::InProgress = *guard {
            self.cvar.wait(&mut guard);
        }
        match &*guard {
            InFlightState::Completed(msg) => Ok(msg.clone()),
            InFlightState::Aborted => Err(DedupWaitError::Aborted),
            InFlightState::InProgress => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupWaitError {
    Aborted,
}

#[derive(Debug, Clone)]
pub enum DedupEntry {
    InFlight(Arc<InFlightSlot>),
    Completed(CachedMessage),
}

impl DedupEntry {
    pub fn created_at(&self) -> DateTime<Utc> {
        match self {
            DedupEntry::InFlight(slot) => slot.created_at,
            DedupEntry::Completed(msg) => msg.created_at,
        }
    }
}

pub enum ReservationOutcome {
    Leader(Arc<InFlightSlot>),
    Follower(Arc<InFlightSlot>),
    AlreadyExists(CachedMessage),
}

#[derive(Debug)]
pub struct MessageDedupStore {
    // Key: (sender_id, client_nonce)
    entries: HashMap<(u64, u64), DedupEntry>,
    ttl_ms: i64,
    capacity: usize,
}

impl Default for MessageDedupStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageDedupStore {
    pub const DEFAULT_TTL_MS: i64 = 24 * 3600 * 1000; // 24 hours (86,400,000 ms)
    pub const DEFAULT_CAPACITY: usize = 100_000;

    pub fn new() -> Self {
        Self::with_config(Self::DEFAULT_TTL_MS, Self::DEFAULT_CAPACITY)
    }

    pub fn with_config(ttl_ms: i64, capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            ttl_ms,
            capacity,
        }
    }

    fn evict_oldest_completed_if_needed(&mut self, incoming_key: &(u64, u64)) {
        if self.entries.len() >= self.capacity && !self.entries.contains_key(incoming_key) {
            // NEVER evict InFlight entries — evicting an InFlight slot would orphan waiting followers!
            if let Some(oldest_completed_key) = self
                .entries
                .iter()
                .filter(|(_, v)| matches!(v, DedupEntry::Completed(_)))
                .min_by_key(|(_, v)| v.created_at())
                .map(|(k, _)| *k)
            {
                self.entries.remove(&oldest_completed_key);
            }
        }
    }

    pub fn reserve(&mut self, sender_id: u64, nonce: u64, now: DateTime<Utc>) -> ReservationOutcome {
        self.cleanup_expired(now);

        let key = (sender_id, nonce);
        if let Some(entry) = self.entries.get(&key) {
            match entry {
                DedupEntry::InFlight(slot) => {
                    return ReservationOutcome::Follower(Arc::clone(slot));
                }
                DedupEntry::Completed(cached) => {
                    let age_ms = now.signed_duration_since(cached.created_at).num_milliseconds();
                    if age_ms < self.ttl_ms {
                        return ReservationOutcome::AlreadyExists(cached.clone());
                    }
                }
            }
        }

        self.evict_oldest_completed_if_needed(&key);

        let slot = Arc::new(InFlightSlot::new(now));
        self.entries.insert(key, DedupEntry::InFlight(Arc::clone(&slot)));
        ReservationOutcome::Leader(slot)
    }

    pub fn complete(&mut self, sender_id: u64, nonce: u64, msg: CachedMessage) {
        let key = (sender_id, nonce);
        if let Some(DedupEntry::InFlight(slot)) = self.entries.get(&key) {
            slot.complete(msg.clone());
        }
        self.entries.insert(key, DedupEntry::Completed(msg));
    }

    /// Completes a specific leader `slot` directly (guaranteeing waiting followers always wake
    /// even if the entry was replaced/expired in the map) and stores the completed record.
    pub fn complete_slot(
        &mut self,
        sender_id: u64,
        nonce: u64,
        slot: &Arc<InFlightSlot>,
        msg: CachedMessage,
    ) {
        slot.complete(msg.clone());
        let key = (sender_id, nonce);
        if let Some(DedupEntry::InFlight(existing)) = self.entries.get(&key) {
            if !Arc::ptr_eq(existing, slot) {
                existing.complete(msg.clone());
            }
        }
        self.entries.insert(key, DedupEntry::Completed(msg));
    }

    pub fn abort(&mut self, sender_id: u64, nonce: u64) {
        let key = (sender_id, nonce);
        if matches!(self.entries.get(&key), Some(DedupEntry::InFlight(_))) {
            if let Some(DedupEntry::InFlight(slot)) = self.entries.remove(&key) {
                slot.abort();
            }
        }
    }

    /// Aborts a specific leader `slot` directly (guaranteeing waiting followers always wake)
    /// and removes the map entry only if it still points to this exact `slot`.
    pub fn abort_slot(&mut self, sender_id: u64, nonce: u64, slot: &Arc<InFlightSlot>) {
        slot.abort();
        let key = (sender_id, nonce);
        if let Some(DedupEntry::InFlight(existing)) = self.entries.get(&key) {
            if Arc::ptr_eq(existing, slot) {
                self.entries.remove(&key);
            }
        }
    }

    pub fn get(&mut self, sender_id: u64, nonce: u64, now: DateTime<Utc>) -> Option<CachedMessage> {
        self.cleanup_expired(now);
        if let Some(DedupEntry::Completed(entry)) = self.entries.get(&(sender_id, nonce)) {
            let age_ms = now.signed_duration_since(entry.created_at).num_milliseconds();
            if age_ms < self.ttl_ms {
                return Some(entry.clone());
            }
        }
        None
    }

    pub fn insert(&mut self, sender_id: u64, nonce: u64, msg: CachedMessage, now: DateTime<Utc>) {
        self.cleanup_expired(now);
        let key = (sender_id, nonce);
        if let Some(DedupEntry::InFlight(slot)) = self.entries.get(&key) {
            slot.complete(msg.clone());
        }
        self.evict_oldest_completed_if_needed(&key);
        self.entries.insert(key, DedupEntry::Completed(msg));
    }

    pub fn cleanup_expired(&mut self, now: DateTime<Utc>) {
        let ttl_ms = self.ttl_ms;
        self.entries.retain(|_, v| {
            let keep = now.signed_duration_since(v.created_at()).num_milliseconds() < ttl_ms;
            if !keep {
                if let DedupEntry::InFlight(slot) = v {
                    slot.abort();
                }
            }
            keep
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample_cached(
        message_id: i64,
        chat_id: i64,
        sender_id: u64,
        text: &str,
        nonce: u64,
        created_at: DateTime<Utc>,
    ) -> CachedMessage {
        CachedMessage {
            message_id,
            chat_id,
            sender_id,
            text: text.to_string(),
            timestamp: created_at.timestamp_millis(),
            reply_to: None,
            client_nonce: nonce,
            delivery_status: "sent".to_string(),
            created_at,
        }
    }

    #[test]
    fn test_dedup_unique_and_duplicate() {
        let mut store = MessageDedupStore::new();
        let now = Utc::now();

        let cached = sample_cached(1, 10, 42, "Hello", 9999, now);

        assert!(store.get(42, 9999, now).is_none());
        store.insert(42, 9999, cached.clone(), now);

        let retrieved = store.get(42, 9999, now).expect("Should find cached message");
        assert_eq!(retrieved.message_id, 1);
        assert_eq!(retrieved.client_nonce, 9999);

        // Different sender, same nonce -> None
        assert!(store.get(43, 9999, now).is_none());
    }

    #[test]
    fn test_dedup_24h_ttl_expiry() {
        let mut store = MessageDedupStore::new();
        let start = Utc::now();

        let cached = sample_cached(2, 10, 42, "Test TTL", 12345, start);

        store.insert(42, 12345, cached, start);

        // 23 hours later: still valid
        let within_ttl = start + Duration::hours(23);
        assert!(store.get(42, 12345, within_ttl).is_some());

        // 24 hours + 1 minute later: expired
        let after_ttl = start + Duration::hours(24) + Duration::minutes(1);
        assert!(store.get(42, 12345, after_ttl).is_none());
    }

    #[test]
    fn test_dedup_capacity_eviction_never_evicts_in_flight() {
        let mut store = MessageDedupStore::with_config(MessageDedupStore::DEFAULT_TTL_MS, 2);
        let t0 = Utc::now();

        // Reserve an oldest InFlight slot at t0
        let leader_slot = match store.reserve(1, 1, t0) {
            ReservationOutcome::Leader(s) => s,
            _ => panic!("Expected Leader"),
        };
        let follower_slot = match store.reserve(1, 1, t0) {
            ReservationOutcome::Follower(s) => s,
            _ => panic!("Expected Follower"),
        };

        // Insert a newer Completed entry at t0 + 1s (fills capacity = 2)
        store.insert(1, 2, sample_cached(2, 10, 1, "Msg 2", 2, t0 + Duration::seconds(1)), t0 + Duration::seconds(1));

        // Insert a 3rd Completed entry at t0 + 2s: MUST evict Completed nonce 2, NOT InFlight nonce 1!
        let t2 = t0 + Duration::seconds(2);
        store.insert(1, 3, sample_cached(3, 10, 1, "Msg 3", 3, t2), t2);
        assert!(store.get(1, 2, t2).is_none());
        assert!(store.get(1, 3, t2).is_some());

        let waiter = std::thread::spawn(move || follower_slot.wait());
        store.complete_slot(1, 1, &leader_slot, sample_cached(1, 10, 1, "Msg 1", 1, t0));
        let done = waiter.join().unwrap().expect("Follower must wake with completed msg");
        assert_eq!(done.message_id, 1);
    }

    #[test]
    fn test_cleanup_expired_aborts_in_flight_followers() {
        let mut store = MessageDedupStore::new();
        let t0 = Utc::now();

        let _leader = match store.reserve(7, 700, t0) {
            ReservationOutcome::Leader(s) => s,
            _ => panic!("Expected Leader"),
        };
        let follower = match store.reserve(7, 700, t0) {
            ReservationOutcome::Follower(s) => s,
            _ => panic!("Expected Follower"),
        };

        let waiter = std::thread::spawn(move || follower.wait());
        store.cleanup_expired(t0 + Duration::hours(25));
        assert_eq!(waiter.join().unwrap(), Err(DedupWaitError::Aborted));
    }

    #[test]
    fn test_boundary_nonces() {
        let mut store = MessageDedupStore::new();
        let now = Utc::now();

        store.insert(1, 0, sample_cached(100, 1, 1, "Zero", 0, now), now);
        assert_eq!(store.get(1, 0, now).unwrap().message_id, 100);

        store.insert(1, u64::MAX, sample_cached(101, 1, 1, "Max", u64::MAX, now), now);
        assert_eq!(store.get(1, u64::MAX, now).unwrap().message_id, 101);
    }

    #[test]
    fn test_in_flight_leader_follower_completion() {
        let mut store = MessageDedupStore::new();
        let now = Utc::now();

        let leader_slot = match store.reserve(1, 1001, now) {
            ReservationOutcome::Leader(slot) => slot,
            _ => panic!("Expected Leader"),
        };

        let follower_slot = match store.reserve(1, 1001, now) {
            ReservationOutcome::Follower(slot) => slot,
            _ => panic!("Expected Follower"),
        };

        let handle = std::thread::spawn(move || {
            follower_slot.wait().expect("Follower should succeed")
        });

        let cached = sample_cached(555, 10, 1, "Hello", 1001, now);
        store.complete_slot(1, 1001, &leader_slot, cached);

        let result = handle.join().unwrap();
        assert_eq!(result.message_id, 555);

        let outcome3 = store.reserve(1, 1001, now);
        match outcome3 {
            ReservationOutcome::AlreadyExists(msg) => assert_eq!(msg.message_id, 555),
            _ => panic!("Expected AlreadyExists"),
        }
    }

    #[test]
    fn test_in_flight_leader_abort_follower_recovery() {
        let mut store = MessageDedupStore::new();
        let now = Utc::now();

        let leader_slot = match store.reserve(2, 2002, now) {
            ReservationOutcome::Leader(slot) => slot,
            _ => panic!("Expected Leader"),
        };

        let follower_slot = match store.reserve(2, 2002, now) {
            ReservationOutcome::Follower(slot) => slot,
            _ => panic!("Expected Follower"),
        };

        let handle = std::thread::spawn(move || follower_slot.wait());

        store.abort_slot(2, 2002, &leader_slot);

        let err = handle.join().unwrap();
        assert_eq!(err, Err(DedupWaitError::Aborted));

        let outcome3 = store.reserve(2, 2002, now);
        match outcome3 {
            ReservationOutcome::Leader(_) => {}
            _ => panic!("Expected new Leader after abort"),
        }
    }
}
