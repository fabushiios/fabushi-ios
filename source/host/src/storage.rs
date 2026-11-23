use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::dedup::{CachedMessage, MessageDedupStore};
use crate::error::{HostError, HostResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSearchHit {
    pub message_id: i64,
    pub chat_id: i64,
    pub sender_id: u64,
    pub timestamp_ms: i64,
    pub body: String,
    pub score: f32,
    pub engine: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageMetrics {
    pub storage_dir: String,
    pub total_bytes: u64,
    pub media_cas_bytes: u64,
    pub media_cas_files: u64,
    pub sqlite_bytes: u64,
    pub tantivy_bytes: u64,
    pub vault_bytes: u64,
    pub hermes_bytes: u64,
}

pub struct HostSqliteStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
    fts5_enabled: bool,
}

impl HostSqliteStore {
    pub fn open(storage_dir: &Path) -> HostResult<Self> {
        std::fs::create_dir_all(storage_dir)
            .map_err(|e| HostError::storage(format!("Failed to create storage dir: {e}")))?;
        let db_path = storage_dir.join("host_state.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| HostError::storage(format!("Failed to open host_state.db: {e}")))?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS host_meta (
                 key TEXT PRIMARY KEY NOT NULL,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS host_kv (
                 namespace TEXT NOT NULL,
                 key TEXT NOT NULL,
                 value TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL,
                 PRIMARY KEY (namespace, key)
             );
             CREATE TABLE IF NOT EXISTS nonce_dedup (
                 sender_id INTEGER NOT NULL,
                 client_nonce INTEGER NOT NULL,
                 message_id INTEGER NOT NULL,
                 chat_id INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 timestamp_ms INTEGER NOT NULL,
                 reply_to INTEGER,
                 delivery_status TEXT NOT NULL DEFAULT 'sent',
                 created_at_ms INTEGER NOT NULL,
                 PRIMARY KEY (sender_id, client_nonce)
             );
             CREATE INDEX IF NOT EXISTS idx_nonce_dedup_created
                 ON nonce_dedup(created_at_ms);
             CREATE TABLE IF NOT EXISTS chat_pts (
                 chat_id INTEGER PRIMARY KEY NOT NULL,
                 pts INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS host_messages_index (
                 message_id INTEGER PRIMARY KEY NOT NULL,
                 chat_id INTEGER NOT NULL,
                 sender_id INTEGER NOT NULL,
                 timestamp_ms INTEGER NOT NULL,
                 body TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_host_messages_chat
                 ON host_messages_index(chat_id, timestamp_ms);",
        )
        .map_err(|e| HostError::storage(format!("Failed to initialize host_state.db schema: {e}")))?;

        // Idempotent migration for pre-existing host_state.db files lacking delivery_status
        let _ = conn.execute(
            "ALTER TABLE nonce_dedup ADD COLUMN delivery_status TEXT NOT NULL DEFAULT 'sent'",
            [],
        );

        let fts5_enabled = conn
            .execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS host_messages_fts USING fts5(
                     message_id UNINDEXED,
                     chat_id UNINDEXED,
                     sender_id UNINDEXED,
                     body
                 );",
            )
            .is_ok();

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
            fts5_enabled,
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn fts5_enabled(&self) -> bool {
        self.fts5_enabled
    }

    pub fn wal_checkpoint_truncate(&self) -> HostResult<(i32, i32, i32)> {
        let conn = self.conn.lock();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| HostError::storage(format!("WAL checkpoint failed: {e}")))
    }

    pub fn get_meta(&self, key: &str) -> HostResult<Option<String>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT value FROM host_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HostError::storage(format!("get_meta({key}) failed: {e}")))
    }

    pub fn set_meta(&self, key: &str, value: &str) -> HostResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO host_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| HostError::storage(format!("set_meta({key}) failed: {e}")))?;
        Ok(())
    }

    pub fn delete_meta(&self, key: &str) -> HostResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM host_meta WHERE key = ?1", params![key])
            .map_err(|e| HostError::storage(format!("delete_meta({key}) failed: {e}")))?;
        Ok(())
    }

    pub fn kv_get(&self, namespace: &str, key: &str) -> HostResult<Option<String>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT value FROM host_kv WHERE namespace = ?1 AND key = ?2",
            params![namespace, key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HostError::storage(format!("kv_get({namespace}/{key}) failed: {e}")))
    }

    pub fn kv_set(&self, namespace: &str, key: &str, value: &str) -> HostResult<()> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO host_kv (namespace, key, value, updated_at_ms) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(namespace, key) DO UPDATE SET value = excluded.value, updated_at_ms = excluded.updated_at_ms",
            params![namespace, key, value, now],
        )
        .map_err(|e| HostError::storage(format!("kv_set({namespace}/{key}) failed: {e}")))?;
        Ok(())
    }

    pub fn kv_delete(&self, namespace: &str, key: &str) -> HostResult<bool> {
        let conn = self.conn.lock();
        let changed = conn
            .execute(
                "DELETE FROM host_kv WHERE namespace = ?1 AND key = ?2",
                params![namespace, key],
            )
            .map_err(|e| HostError::storage(format!("kv_delete({namespace}/{key}) failed: {e}")))?;
        Ok(changed > 0)
    }

    pub fn kv_list(&self, namespace: &str) -> HostResult<HashMap<String, String>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT key, value FROM host_kv WHERE namespace = ?1 ORDER BY key ASC")
            .map_err(|e| HostError::storage(format!("kv_list prepare failed: {e}")))?;
        let rows = stmt
            .query_map(params![namespace], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| HostError::storage(format!("kv_list query failed: {e}")))?;
        let mut out = HashMap::new();
        for row in rows {
            let (k, v) = row.map_err(|e| HostError::storage(format!("kv_list row failed: {e}")))?;
            out.insert(k, v);
        }
        Ok(out)
    }

    pub fn load_generation(&self) -> HostResult<u64> {
        let val = self
            .get_meta("account_generation")?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1)
            .max(1);
        Ok(val)
    }

    pub fn save_generation(&self, generation: u64) -> HostResult<()> {
        self.set_meta("account_generation", &generation.to_string())
    }

    pub fn load_next_message_id(&self) -> HostResult<i64> {
        let conn = self.conn.lock();
        let meta_next: i64 = conn
            .query_row(
                "SELECT value FROM host_meta WHERE key = 'next_message_id'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| HostError::storage(e.to_string()))?
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(1001);
        let max_dedup: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(message_id), 1000) FROM nonce_dedup",
                [],
                |r| r.get(0),
            )
            .unwrap_or(1000);
        let max_idx: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(message_id), 1000) FROM host_messages_index",
                [],
                |r| r.get(0),
            )
            .unwrap_or(1000);
        Ok(meta_next.max(max_dedup + 1).max(max_idx + 1).max(1001))
    }

    pub fn persist_next_message_id(&self, next_id: i64) -> HostResult<()> {
        self.set_meta("next_message_id", &next_id.to_string())
    }

    pub fn load_all_pts(&self) -> HostResult<Vec<(i64, u64)>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT chat_id, pts FROM chat_pts")
            .map_err(|e| HostError::storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? as u64)))
            .map_err(|e| HostError::storage(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| HostError::storage(e.to_string()))?);
        }
        Ok(out)
    }

    pub fn save_pts(&self, chat_id: i64, pts: u64) -> HostResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO chat_pts (chat_id, pts) VALUES (?1, ?2)
             ON CONFLICT(chat_id) DO UPDATE SET pts = excluded.pts",
            params![chat_id, pts as i64],
        )
        .map_err(|e| HostError::storage(e.to_string()))?;
        Ok(())
    }

    pub fn load_valid_nonces(&self, now: DateTime<Utc>) -> HostResult<Vec<CachedMessage>> {
        let cutoff_ms = now.timestamp_millis() - MessageDedupStore::DEFAULT_TTL_MS;
        let conn = self.conn.lock();
        let _ = conn.execute(
            "DELETE FROM nonce_dedup WHERE created_at_ms <= ?1",
            params![cutoff_ms],
        );
        let mut stmt = conn
            .prepare(
                "SELECT sender_id, client_nonce, message_id, chat_id, text, timestamp_ms, reply_to, delivery_status, created_at_ms
                 FROM nonce_dedup
                 WHERE created_at_ms > ?1",
            )
            .map_err(|e| HostError::storage(e.to_string()))?;
        let rows = stmt
            .query_map(params![cutoff_ms], |row| {
                let sender_raw: i64 = row.get(0)?;
                let nonce_raw: i64 = row.get(1)?;
                let created_ms: i64 = row.get(8)?;
                let created_at = Utc
                    .timestamp_millis_opt(created_ms)
                    .single()
                    .unwrap_or_else(Utc::now);
                Ok(CachedMessage {
                    message_id: row.get(2)?,
                    chat_id: row.get(3)?,
                    sender_id: sender_raw as u64,
                    text: row.get(4)?,
                    timestamp: row.get(5)?,
                    reply_to: row.get(6)?,
                    client_nonce: nonce_raw as u64,
                    delivery_status: row
                        .get::<_, Option<String>>(7)?
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "sent".to_string()),
                    created_at,
                })
            })
            .map_err(|e| HostError::storage(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| HostError::storage(e.to_string()))?);
        }
        Ok(out)
    }

    pub fn persist_nonce(&self, cached: &CachedMessage) -> HostResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO nonce_dedup (
                sender_id, client_nonce, message_id, chat_id, text, timestamp_ms, reply_to, delivery_status, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(sender_id, client_nonce) DO UPDATE SET
                message_id = excluded.message_id,
                chat_id = excluded.chat_id,
                text = excluded.text,
                timestamp_ms = excluded.timestamp_ms,
                reply_to = excluded.reply_to,
                delivery_status = excluded.delivery_status,
                created_at_ms = excluded.created_at_ms",
            params![
                cached.sender_id as i64,
                cached.client_nonce as i64,
                cached.message_id,
                cached.chat_id,
                cached.text,
                cached.timestamp,
                cached.reply_to,
                cached.delivery_status,
                cached.created_at.timestamp_millis(),
            ],
        )
        .map_err(|e| HostError::storage(format!("persist_nonce failed: {e}")))?;
        Ok(())
    }

    pub fn index_message_fts(
        &self,
        message_id: i64,
        chat_id: i64,
        sender_id: u64,
        timestamp_ms: i64,
        body: &str,
    ) -> HostResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO host_messages_index (message_id, chat_id, sender_id, timestamp_ms, body)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(message_id) DO UPDATE SET
                chat_id = excluded.chat_id,
                sender_id = excluded.sender_id,
                timestamp_ms = excluded.timestamp_ms,
                body = excluded.body",
            params![message_id, chat_id, sender_id as i64, timestamp_ms, body],
        )
        .map_err(|e| HostError::storage(format!("index_message_fts failed: {e}")))?;

        if self.fts5_enabled {
            let _ = conn.execute(
                "DELETE FROM host_messages_fts WHERE message_id = ?1",
                params![message_id],
            );
            let _ = conn.execute(
                "INSERT INTO host_messages_fts (message_id, chat_id, sender_id, body)
                 VALUES (?1, ?2, ?3, ?4)",
                params![message_id, chat_id, sender_id as i64, body],
            );
        }
        Ok(())
    }

    pub fn list_all_indexed_messages(&self) -> HostResult<Vec<(i64, i64, u64, i64, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT message_id, chat_id, sender_id, timestamp_ms, body
                 FROM host_messages_index
                 ORDER BY timestamp_ms ASC, message_id ASC",
            )
            .map_err(|e| HostError::storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| HostError::storage(e.to_string()))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| HostError::storage(e.to_string()))?);
        }
        Ok(out)
    }

    pub fn search_fts5_or_cjk(
        &self,
        query: &str,
        chat_id: Option<i64>,
        limit: usize,
    ) -> HostResult<Vec<LocalSearchHit>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let limit_i64 = limit.clamp(1, 200) as i64;
        let conn = self.conn.lock();

        // 1. Try bundled SQLite FTS5 match first for tokenized queries
        if self.fts5_enabled {
            let escaped = format!("\"{}\"", trimmed.replace('"', "\"\""));
            let sql = if chat_id.is_some() {
                "SELECT m.message_id, m.chat_id, m.sender_id, m.timestamp_ms, m.body, -bm25(host_messages_fts)
                 FROM host_messages_fts
                 JOIN host_messages_index m ON m.message_id = host_messages_fts.message_id
                 WHERE host_messages_fts MATCH ?1 AND m.chat_id = ?2
                 ORDER BY bm25(host_messages_fts) ASC
                 LIMIT ?3"
            } else {
                "SELECT m.message_id, m.chat_id, m.sender_id, m.timestamp_ms, m.body, -bm25(host_messages_fts)
                 FROM host_messages_fts
                 JOIN host_messages_index m ON m.message_id = host_messages_fts.message_id
                 WHERE host_messages_fts MATCH ?1
                 ORDER BY bm25(host_messages_fts) ASC
                 LIMIT ?2"
            };
            if let Ok(mut stmt) = conn.prepare(sql) {
                fn map_fts_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalSearchHit> {
                    let score: f64 = row.get(5).unwrap_or(1.0);
                    Ok(LocalSearchHit {
                        message_id: row.get(0)?,
                        chat_id: row.get(1)?,
                        sender_id: row.get::<_, i64>(2)? as u64,
                        timestamp_ms: row.get(3)?,
                        body: row.get(4)?,
                        score: (score.abs() as f32).max(0.01),
                        engine: "sqlite_fts5".to_string(),
                    })
                }
                let mapped = if let Some(cid) = chat_id {
                    stmt.query_map(params![escaped, cid, limit_i64], map_fts_row)
                } else {
                    stmt.query_map(params![escaped, limit_i64], map_fts_row)
                };
                if let Ok(rows) = mapped {
                    let mut hits = Vec::new();
                    for r in rows.flatten() {
                        hits.push(r);
                    }
                    if !hits.is_empty() {
                        return Ok(hits);
                    }
                }
            }
        }

        // 2. Substring fallback for unsegmented CJK / partial terms
        let escaped_like = trimmed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let like_pattern = format!("%{escaped_like}%");
        let mut hits = Vec::new();
        if let Some(cid) = chat_id {
            let mut stmt = conn
                .prepare(
                    "SELECT message_id, chat_id, sender_id, timestamp_ms, body
                     FROM host_messages_index
                     WHERE chat_id = ?1 AND body LIKE ?2 ESCAPE '\\'
                     ORDER BY timestamp_ms DESC, message_id DESC
                     LIMIT ?3",
                )
                .map_err(|e| HostError::storage(e.to_string()))?;
            let rows = stmt
                .query_map(params![cid, like_pattern, limit_i64], |row| {
                    Ok(LocalSearchHit {
                        message_id: row.get(0)?,
                        chat_id: row.get(1)?,
                        sender_id: row.get::<_, i64>(2)? as u64,
                        timestamp_ms: row.get(3)?,
                        body: row.get(4)?,
                        score: 1.0,
                        engine: "sqlite_fts5_cjk_fallback".to_string(),
                    })
                })
                .map_err(|e| HostError::storage(e.to_string()))?;
            for r in rows {
                hits.push(r.map_err(|e| HostError::storage(e.to_string()))?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT message_id, chat_id, sender_id, timestamp_ms, body
                     FROM host_messages_index
                     WHERE body LIKE ?1 ESCAPE '\\'
                     ORDER BY timestamp_ms DESC, message_id DESC
                     LIMIT ?2",
                )
                .map_err(|e| HostError::storage(e.to_string()))?;
            let rows = stmt
                .query_map(params![like_pattern, limit_i64], |row| {
                    Ok(LocalSearchHit {
                        message_id: row.get(0)?,
                        chat_id: row.get(1)?,
                        sender_id: row.get::<_, i64>(2)? as u64,
                        timestamp_ms: row.get(3)?,
                        body: row.get(4)?,
                        score: 1.0,
                        engine: "sqlite_fts5_cjk_fallback".to_string(),
                    })
                })
                .map_err(|e| HostError::storage(e.to_string()))?;
            for r in rows {
                hits.push(r.map_err(|e| HostError::storage(e.to_string()))?);
            }
        }
        Ok(hits)
    }
}

fn dir_size_and_count(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, 0);
    };
    let ft = meta.file_type();
    if ft.is_symlink() {
        return (0, 0);
    }
    if ft.is_file() {
        return (meta.len(), 1);
    }
    if !ft.is_dir() {
        return (0, 0);
    }
    let mut bytes = 0u64;
    let mut files = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            let (b, f) = dir_size_and_count(&p);
            bytes = bytes.saturating_add(b);
            files = files.saturating_add(f);
        }
    }
    (bytes, files)
}

pub fn compute_storage_metrics(storage_dir: &Path) -> StorageMetrics {
    let (total_bytes, _) = dir_size_and_count(storage_dir);
    let (media_cas_bytes, media_cas_files) = dir_size_and_count(&storage_dir.join("media"));
    let (tantivy_bytes, _) = dir_size_and_count(&storage_dir.join("tantivy_index"));
    let (hermes_bytes, _) = dir_size_and_count(&storage_dir.join("hermes"));
    let vault_bytes = ["vault.db", "vault.db-wal", "vault.db-shm"]
        .iter()
        .map(|f| dir_size_and_count(&storage_dir.join(f)).0)
        .sum();
    let sqlite_bytes = [
        "client_cache.db",
        "client_cache.db-wal",
        "client_cache.db-shm",
        "host_state.db",
        "host_state.db-wal",
        "host_state.db-shm",
        "coding_store.db",
        "fabushi_miniapp.db",
    ]
    .iter()
    .map(|f| dir_size_and_count(&storage_dir.join(f)).0)
    .sum();

    StorageMetrics {
        storage_dir: storage_dir.to_string_lossy().into_owned(),
        total_bytes,
        media_cas_bytes,
        media_cas_files,
        sqlite_bytes,
        tantivy_bytes,
        vault_bytes,
        hermes_bytes,
    }
}

pub fn purge_media_cas_dir(storage_dir: &Path) -> HostResult<(u64, u64)> {
    let media_dir = storage_dir.join("media");
    if !media_dir.exists() {
        return Ok((0, 0));
    }
    let (bytes_before, files_before) = dir_size_and_count(&media_dir);
    std::fs::remove_dir_all(&media_dir)
        .map_err(|e| HostError::storage(format!("Failed to purge media CAS directory: {e}")))?;
    std::fs::create_dir_all(&media_dir)
        .map_err(|e| HostError::storage(format!("Failed to recreate media CAS directory: {e}")))?;
    Ok((bytes_before, files_before))
}
