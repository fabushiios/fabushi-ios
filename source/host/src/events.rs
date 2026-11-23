use std::collections::VecDeque;
use std::ffi::{c_char, c_void, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use gpui_telegram_client::client_dispatch::ClientDomainEvent;
use gpui_telegram_client::outbox::OutboxStatusEvent;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Function pointer type for legacy asynchronous push event callbacks.
pub type TelegramBridgeEventCallback = unsafe extern "C" fn(
    event_type: *const c_char,
    payload_json: *const c_char,
    user_context: *mut c_void,
);

#[derive(Clone)]
pub struct BridgeEventSink {
    callback: TelegramBridgeEventCallback,
    user_context: *mut c_void,
}

// Safety: C function pointer and caller-owned opaque context are invoked synchronously
// without mutating Rust pointers; caller manages callback lifetime and unregisters via NULL.
unsafe impl Send for BridgeEventSink {}
unsafe impl Sync for BridgeEventSink {}

impl BridgeEventSink {
    pub fn new(callback: TelegramBridgeEventCallback, user_context: *mut c_void) -> Self {
        Self {
            callback,
            user_context,
        }
    }

    pub fn emit(&self, event_type: &str, payload_json: &str) {
        if let (Ok(type_c), Ok(payload_c)) = (
            CString::new(event_type.replace('\0', "")),
            CString::new(payload_json.replace('\0', "")),
        ) {
            unsafe {
                (self.callback)(type_c.as_ptr(), payload_c.as_ptr(), self.user_context);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEvent {
    pub seq: u64,
    pub generation: u64,
    pub timestamp_ms: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: Value,
}

const DEFAULT_EVENT_QUEUE_CAPACITY: usize = 4096;

struct EventQueueInner {
    events: VecDeque<HostEvent>,
    closed: bool,
}

pub struct EventQueue {
    inner: Mutex<EventQueueInner>,
    cvar: Condvar,
    seq: AtomicU64,
    capacity: usize,
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_QUEUE_CAPACITY)
    }
}

impl EventQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(EventQueueInner {
                events: VecDeque::with_capacity(capacity.min(1024)),
                closed: false,
            }),
            cvar: Condvar::new(),
            seq: AtomicU64::new(1),
            capacity: capacity.max(64),
        }
    }

    pub fn push(&self, generation: u64, event_type: impl Into<String>, payload: Value) -> HostEvent {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let event = HostEvent {
            seq,
            generation,
            timestamp_ms: Utc::now().timestamp_millis(),
            event_type: event_type.into(),
            payload,
        };
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.events.len() >= self.capacity {
            guard.events.pop_front();
        }
        guard.events.push_back(event.clone());
        self.cvar.notify_all();
        event
    }

    pub fn poll(&self, max_count: usize, timeout_ms: u32) -> Vec<HostEvent> {
        let limit = if max_count == 0 { 64 } else { max_count.min(4096) };
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        if guard.events.is_empty() && !guard.closed && timeout_ms > 0 {
            let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
            while guard.events.is_empty() && !guard.closed {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let remaining = deadline.saturating_duration_since(now);
                let (next_guard, wait_res) = self
                    .cvar
                    .wait_timeout(guard, remaining)
                    .unwrap_or_else(|e| e.into_inner());
                guard = next_guard;
                if wait_res.timed_out() {
                    break;
                }
            }
        }

        let drain_count = guard.events.len().min(limit);
        let mut drained = Vec::with_capacity(drain_count);
        for _ in 0..drain_count {
            if let Some(ev) = guard.events.pop_front() {
                drained.push(ev);
            }
        }
        drained
    }

    pub fn close(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.closed = true;
        self.cvar.notify_all();
    }
}

/// Lossless exhaustive mapper converting every `ClientDomainEvent` into a typed JSON event pair.
pub fn domain_event_to_host_event(ev: ClientDomainEvent) -> (String, Value) {
    match ev {
        ClientDomainEvent::NewMessage(msg) => (
            "newMessage".to_string(),
            serde_json::to_value(&msg).unwrap_or(Value::Null),
        ),
        ClientDomainEvent::EditMessage(msg) => (
            "editMessage".to_string(),
            serde_json::to_value(&msg).unwrap_or(Value::Null),
        ),
        ClientDomainEvent::DeleteMessages {
            chat_id,
            message_ids,
            for_everyone,
        } => (
            "deleteMessages".to_string(),
            json!({
                "chat_id": chat_id.0,
                "message_ids": message_ids.into_iter().map(|m| m.0).collect::<Vec<_>>(),
                "for_everyone": for_everyone,
            }),
        ),
        ClientDomainEvent::ReactionsUpdated {
            chat_id,
            message_id,
            user_id,
            reactions,
        } => (
            "reactionUpdated".to_string(),
            json!({
                "chat_id": chat_id.0,
                "message_id": message_id.0,
                "user_id": user_id.0,
                "reactions": reactions,
            }),
        ),
        ClientDomainEvent::PollUpdated {
            chat_id,
            message_id,
            poll,
        } => (
            "pollUpdated".to_string(),
            json!({
                "chat_id": chat_id.0,
                "message_id": message_id.0,
                "poll": poll,
            }),
        ),
        ClientDomainEvent::TodoUpdated {
            chat_id,
            message_id,
            todo,
        } => (
            "todoUpdated".to_string(),
            json!({
                "chat_id": chat_id.0,
                "message_id": message_id.0,
                "todo": todo,
            }),
        ),
        ClientDomainEvent::ReadHistory {
            chat_id,
            max_id,
            is_outbox,
            unread_count,
        } => (
            "readHistory".to_string(),
            json!({
                "chat_id": chat_id.0,
                "max_id": max_id.0,
                "is_outbox": is_outbox,
                "unread_count": unread_count,
            }),
        ),
        ClientDomainEvent::Typing(typing) => (
            "userTyping".to_string(),
            serde_json::to_value(&typing).unwrap_or(Value::Null),
        ),
        ClientDomainEvent::UserStatusChanged { user_id, status } => (
            "userStatusChanged".to_string(),
            json!({
                "user_id": user_id.0,
                "status": status,
            }),
        ),
        ClientDomainEvent::PinnedMessagesChanged {
            chat_id,
            pinned_ids,
        } => (
            "pinnedMessagesChanged".to_string(),
            json!({
                "chat_id": chat_id.0,
                "pinned_ids": pinned_ids.into_iter().map(|m| m.0).collect::<Vec<_>>(),
            }),
        ),
        ClientDomainEvent::DraftUpdated {
            chat_id,
            topic_id,
            draft,
        } => (
            "draftUpdated".to_string(),
            json!({
                "chat_id": chat_id.0,
                "topic_id": topic_id,
                "draft": draft,
            }),
        ),
        ClientDomainEvent::ForumTopicChanged { chat_id, topic } => (
            "forumTopicChanged".to_string(),
            json!({
                "chat_id": chat_id.0,
                "topic": topic,
            }),
        ),
        ClientDomainEvent::FolderChanged { folder } => (
            "folderChanged".to_string(),
            json!({ "folder": folder }),
        ),
        ClientDomainEvent::CallUpdated {
            call_id,
            state,
            signal,
        } => (
            "callUpdated".to_string(),
            json!({
                "call_id": call_id,
                "state": state,
                "signal": signal,
            }),
        ),
        ClientDomainEvent::StoryUpdated { story } => (
            "storyUpdated".to_string(),
            json!({ "story": story }),
        ),
        ClientDomainEvent::AgentFloorChanged {
            chat_id,
            current_speaker,
            queued_count,
            loop_hops,
        } => (
            "agentFloorChanged".to_string(),
            json!({
                "chat_id": chat_id.0,
                "current_speaker": current_speaker,
                "queued_count": queued_count,
                "loop_hops": loop_hops,
            }),
        ),
        ClientDomainEvent::AgentWorkspaceUpdated {
            chat_id,
            revision,
            key,
            value,
            updated_by,
        } => (
            "agentWorkspaceUpdated".to_string(),
            json!({
                "chat_id": chat_id.0,
                "revision": revision,
                "key": key,
                "value": value,
                "updated_by": updated_by,
            }),
        ),
        ClientDomainEvent::AgentHandoffEvent {
            chat_id,
            from_agent,
            to_agent,
            hop,
            max_hops,
        } => (
            "agentHandoffEvent".to_string(),
            json!({
                "chat_id": chat_id.0,
                "from_agent": from_agent,
                "to_agent": to_agent,
                "hop": hop,
                "max_hops": max_hops,
            }),
        ),
        ClientDomainEvent::BotActivityChanged {
            chat_id,
            bot_user_id,
            state,
            detail,
        } => (
            "botActivityChanged".to_string(),
            json!({
                "chat_id": chat_id.0,
                "bot_user_id": bot_user_id.0,
                "state": state,
                "detail": detail,
            }),
        ),
        ClientDomainEvent::MiniAppWebMcpCommand(cmd) => (
            "miniAppWebMcpCommand".to_string(),
            serde_json::to_value(&cmd).unwrap_or(Value::Null),
        ),
        ClientDomainEvent::MiniAppControlStatus {
            bot_username,
            short_name,
            chat_id,
            bot_user_id,
            app_title,
            mode,
            status_text,
            active_tool,
        } => (
            "miniAppControlStatus".to_string(),
            json!({
                "bot_username": bot_username,
                "short_name": short_name,
                "chat_id": chat_id.0,
                "bot_user_id": bot_user_id.0,
                "app_title": app_title,
                "mode": mode,
                "status_text": status_text,
                "active_tool": active_tool,
            }),
        ),
        ClientDomainEvent::CodingHostCommand(cmd) => (
            "codingHostCommand".to_string(),
            serde_json::to_value(&cmd).unwrap_or(Value::Null),
        ),
        ClientDomainEvent::ChatAction { chat_id, action } => (
            "chatAction".to_string(),
            json!({
                "chat_id": chat_id.map(|c| c.0),
                "action": action,
            }),
        ),
        ClientDomainEvent::MessageAck {
            correlation_id,
            client_nonce,
            message_id,
            pts,
        } => (
            "messageAck".to_string(),
            json!({
                "correlation_id": correlation_id,
                "client_nonce": client_nonce,
                "message_id": message_id.0,
                "pts": pts,
            }),
        ),
        ClientDomainEvent::SyncGapDetected { chat_id, from_pts } => (
            "syncGapDetected".to_string(),
            json!({
                "chat_id": chat_id.0,
                "from_pts": from_pts,
            }),
        ),
        ClientDomainEvent::ServerError {
            code,
            message,
            details,
        } => (
            "serverError".to_string(),
            json!({
                "code": code,
                "message": message,
                "details": details,
            }),
        ),
        ClientDomainEvent::Pong { nonce } => (
            "pong".to_string(),
            json!({ "nonce": nonce }),
        ),
    }
}

pub fn outbox_event_to_host_event(ev: &OutboxStatusEvent) -> (String, Value) {
    (
        "outboxStatusChanged".to_string(),
        json!({
            "client_nonce": ev.client_nonce,
            "chat_id": ev.chat_id,
            "status": format!("{:?}", ev.status),
            "server_message_id": ev.server_message_id,
            "retry_count": ev.retry_count,
            "last_error": ev.last_error,
        }),
    )
}
