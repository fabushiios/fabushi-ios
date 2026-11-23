use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedWorkspaceEntry {
    pub key: String,
    pub value: Value,
    pub updated_by: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharedGroupWorkspace {
    pub chat_id: i64,
    pub revision: u64,
    pub entries: HashMap<String, SharedWorkspaceEntry>,
}

impl SharedGroupWorkspace {
    pub fn new(chat_id: i64) -> Self {
        Self {
            chat_id,
            revision: 0,
            entries: HashMap::new(),
        }
    }

    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: Value,
        updated_by: impl Into<String>,
    ) -> u64 {
        let key = key.into();
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.revision = self.revision.saturating_add(1);
        self.entries.insert(
            key.clone(),
            SharedWorkspaceEntry {
                key,
                value,
                updated_by: updated_by.into(),
                updated_at_ms: now_ms,
            },
        );
        self.revision
    }

    pub fn set_with_expected_revision(
        &mut self,
        expected_revision: u64,
        key: impl Into<String>,
        value: Value,
        updated_by: impl Into<String>,
    ) -> Result<u64, String> {
        if self.revision != expected_revision {
            return Err(format!(
                "Workspace revision conflict: expected {expected_revision}, actual {}",
                self.revision
            ));
        }
        Ok(self.set(key, value, updated_by))
    }

    pub fn get(&self, key: &str) -> Option<&SharedWorkspaceEntry> {
        self.entries.get(key)
    }

    pub fn list_entries(&self) -> Vec<SharedWorkspaceEntry> {
        let mut items: Vec<SharedWorkspaceEntry> = self.entries.values().cloned().collect();
        items.sort_by(|a, b| a.key.cmp(&b.key));
        items
    }

    pub fn remove(&mut self, key: &str) -> Option<SharedWorkspaceEntry> {
        let removed = self.entries.remove(key);
        if removed.is_some() {
            self.revision = self.revision.saturating_add(1);
        }
        removed
    }

    pub fn snapshot(&self) -> Value {
        serde_json::json!({
            "chat_id": self.chat_id,
            "revision": self.revision,
            "entries": self.entries,
        })
    }
}

/// Async bridge allowing `HermesEngine` tools (`group_workspace_list`, `group_workspace_read`,
/// `group_workspace_write`) to read and write a group room's [`SharedGroupWorkspace`].
#[async_trait]
pub trait GroupWorkspaceBridge: Send + Sync {
    async fn list_keys(&self, chat_id: i64) -> Result<Vec<SharedWorkspaceEntry>, String>;

    async fn read_key(
        &self,
        chat_id: i64,
        key: &str,
    ) -> Result<Option<SharedWorkspaceEntry>, String>;

    async fn write_key(
        &self,
        chat_id: i64,
        key: &str,
        value: Value,
        updated_by: &str,
        expected_revision: Option<u64>,
    ) -> Result<(u64, SharedWorkspaceEntry), String>;
}

/// Multi-chat in-memory [`GroupWorkspaceBridge`] backed by `SharedGroupWorkspace` instances.
#[derive(Debug, Default, Clone)]
pub struct InMemoryGroupWorkspaceBridge {
    workspaces: Arc<RwLock<HashMap<i64, SharedGroupWorkspace>>>,
}

impl InMemoryGroupWorkspaceBridge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_shared(workspace: SharedGroupWorkspace) -> Self {
        let mut map = HashMap::new();
        map.insert(workspace.chat_id, workspace);
        Self {
            workspaces: Arc::new(RwLock::new(map)),
        }
    }

    pub fn with_shared_map(workspaces: Arc<RwLock<HashMap<i64, SharedGroupWorkspace>>>) -> Self {
        Self { workspaces }
    }

    pub fn revision(&self, chat_id: i64) -> u64 {
        self.workspaces
            .read()
            .get(&chat_id)
            .map(|w| w.revision)
            .unwrap_or(0)
    }

    pub fn snapshot(&self, chat_id: i64) -> Option<Value> {
        self.workspaces
            .read()
            .get(&chat_id)
            .map(SharedGroupWorkspace::snapshot)
    }
}

#[async_trait]
impl GroupWorkspaceBridge for InMemoryGroupWorkspaceBridge {
    async fn list_keys(&self, chat_id: i64) -> Result<Vec<SharedWorkspaceEntry>, String> {
        let guard = self.workspaces.read();
        Ok(guard
            .get(&chat_id)
            .map(SharedGroupWorkspace::list_entries)
            .unwrap_or_default())
    }

    async fn read_key(
        &self,
        chat_id: i64,
        key: &str,
    ) -> Result<Option<SharedWorkspaceEntry>, String> {
        let key_trimmed = key.trim();
        if key_trimmed.is_empty() {
            return Err("Workspace key cannot be empty".into());
        }
        let guard = self.workspaces.read();
        Ok(guard
            .get(&chat_id)
            .and_then(|ws| ws.get(key_trimmed).cloned()))
    }

    async fn write_key(
        &self,
        chat_id: i64,
        key: &str,
        value: Value,
        updated_by: &str,
        expected_revision: Option<u64>,
    ) -> Result<(u64, SharedWorkspaceEntry), String> {
        let key_trimmed = key.trim();
        if key_trimmed.is_empty() {
            return Err("Workspace key cannot be empty".into());
        }
        let mut guard = self.workspaces.write();
        let ws = guard
            .entry(chat_id)
            .or_insert_with(|| SharedGroupWorkspace::new(chat_id));
        let rev = match expected_revision {
            Some(expected) => {
                ws.set_with_expected_revision(expected, key_trimmed, value, updated_by)?
            }
            None => ws.set(key_trimmed, value, updated_by),
        };
        let entry = ws
            .get(key_trimmed)
            .cloned()
            .ok_or_else(|| "Workspace write succeeded but entry lookup failed".to_string())?;
        Ok((rev, entry))
    }
}

/// Single-room adapter wrapping an `Arc<RwLock<SharedGroupWorkspace>>` as a [`GroupWorkspaceBridge`].
#[derive(Debug, Clone)]
pub struct SharedGroupWorkspaceAdapter {
    workspace: Arc<RwLock<SharedGroupWorkspace>>,
}

impl SharedGroupWorkspaceAdapter {
    pub fn new(workspace: Arc<RwLock<SharedGroupWorkspace>>) -> Self {
        Self { workspace }
    }

    pub fn workspace(&self) -> Arc<RwLock<SharedGroupWorkspace>> {
        Arc::clone(&self.workspace)
    }
}

#[async_trait]
impl GroupWorkspaceBridge for SharedGroupWorkspaceAdapter {
    async fn list_keys(&self, chat_id: i64) -> Result<Vec<SharedWorkspaceEntry>, String> {
        let guard = self.workspace.read();
        if guard.chat_id != 0 && guard.chat_id != chat_id {
            return Err(format!(
                "Workspace adapter pinned to chat_id {}, got {}",
                guard.chat_id, chat_id
            ));
        }
        Ok(guard.list_entries())
    }

    async fn read_key(
        &self,
        chat_id: i64,
        key: &str,
    ) -> Result<Option<SharedWorkspaceEntry>, String> {
        let key_trimmed = key.trim();
        if key_trimmed.is_empty() {
            return Err("Workspace key cannot be empty".into());
        }
        let guard = self.workspace.read();
        if guard.chat_id != 0 && guard.chat_id != chat_id {
            return Err(format!(
                "Workspace adapter pinned to chat_id {}, got {}",
                guard.chat_id, chat_id
            ));
        }
        Ok(guard.get(key_trimmed).cloned())
    }

    async fn write_key(
        &self,
        chat_id: i64,
        key: &str,
        value: Value,
        updated_by: &str,
        expected_revision: Option<u64>,
    ) -> Result<(u64, SharedWorkspaceEntry), String> {
        let key_trimmed = key.trim();
        if key_trimmed.is_empty() {
            return Err("Workspace key cannot be empty".into());
        }
        let mut guard = self.workspace.write();
        if guard.chat_id != 0 && guard.chat_id != chat_id {
            return Err(format!(
                "Workspace adapter pinned to chat_id {}, got {}",
                guard.chat_id, chat_id
            ));
        }
        let rev = match expected_revision {
            Some(expected) => {
                guard.set_with_expected_revision(expected, key_trimmed, value, updated_by)?
            }
            None => guard.set(key_trimmed, value, updated_by),
        };
        let entry = guard
            .get(key_trimmed)
            .cloned()
            .ok_or_else(|| "Workspace write succeeded but entry lookup failed".to_string())?;
        Ok((rev, entry))
    }
}
