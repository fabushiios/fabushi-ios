//! SQLite WAL persistence for coding threads, turn checkpoints, and idempotent turn receipts.

use gpui_telegram_proto::{
    CodingDiffStat, CodingHostEvent, CodingPermissionMode, CodingProviderKind,
};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;
use std::path::Path;

/// Canonical thread composite key (`topic_id == 0` when `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadKey {
    pub owner_user_id: u64,
    pub bot_user_id: i64,
    pub chat_id: i64,
    pub topic_id: i64,
}

impl ThreadKey {
    pub fn new(bot_user_id: i64, chat_id: i64, topic_id: Option<i64>) -> Self {
        Self::for_owner(0, bot_user_id, chat_id, topic_id)
    }

    pub fn for_owner(
        owner_user_id: u64,
        bot_user_id: i64,
        chat_id: i64,
        topic_id: Option<i64>,
    ) -> Self {
        Self {
            owner_user_id,
            bot_user_id,
            chat_id,
            topic_id: topic_id.unwrap_or(0),
        }
    }
}

/// Persistent thread metadata and provider session binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRecord {
    pub owner_user_id: u64,
    pub bot_user_id: i64,
    pub chat_id: i64,
    pub topic_id: i64,
    pub thread_id: String,
    pub provider_session_id: Option<String>,
    pub workspace_path: String,
    pub worktree_path: Option<String>,
    pub branch_name: Option<String>,
    pub use_worktree: bool,
    pub active_model: Option<String>,
    pub permission_mode: CodingPermissionMode,
    pub turn_count: u64,
    pub last_summary: Option<String>,
    pub updated_at_ms: i64,
    /// Last-seen bot-level config baseline values for detecting `PATCH` / Bot Inspector updates.
    pub bot_provider_kind: Option<CodingProviderKind>,
    pub bot_workspace_path: String,
    pub bot_use_worktree: bool,
    pub bot_active_model: Option<String>,
    pub bot_permission_mode: Option<CodingPermissionMode>,
}

/// Persistent checkpoint record for one completed coding turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRecord {
    pub owner_user_id: u64,
    pub thread_id: String,
    pub turn_idx: u64,
    pub before_ref: String,
    pub after_ref: String,
    pub diff_stat: CodingDiffStat,
    pub prompt_preview: String,
    pub reply_preview: String,
    pub created_at_ms: i64,
}

/// Result of attempting to claim a `turn_id` in `command_receipts`.
#[derive(Debug, Clone)]
pub enum TurnReceiptClaim {
    Claimed,
    AlreadyCompleted(CodingHostEvent),
    InProgress,
    FinishedWithoutEvent { turn_idx: u64, status: String },
}

/// Thread-safe SQLite WAL store backing `LocalCodingHostRuntime`.
pub struct CodingStore {
    conn: Mutex<Connection>,
}

impl CodingStore {
    /// Opens or creates the SQLite WAL database at `db_path`, creating parent dirs automatically.
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "failed to create coding store directory `{}`: {e}",
                        parent.display()
                    )
                })?;
            }
        }

        let conn = Connection::open(db_path).map_err(|e| {
            format!(
                "failed to open coding store database `{}`: {e}",
                db_path.display()
            )
        })?;

        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA busy_timeout = 5000;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS coding_threads (
                owner_user_id       INTEGER NOT NULL DEFAULT 0,
                bot_user_id         INTEGER NOT NULL,
                chat_id             INTEGER NOT NULL,
                topic_id            INTEGER NOT NULL,
                thread_id           TEXT NOT NULL,
                provider_session_id TEXT,
                workspace_path      TEXT NOT NULL,
                worktree_path       TEXT,
                branch_name         TEXT,
                use_worktree        INTEGER NOT NULL DEFAULT 0,
                active_model        TEXT,
                permission_mode     TEXT NOT NULL,
                turn_count          INTEGER NOT NULL DEFAULT 0,
                last_summary        TEXT,
                updated_at_ms       INTEGER NOT NULL,
                bot_provider_kind   TEXT,
                bot_workspace_path  TEXT NOT NULL DEFAULT '',
                bot_use_worktree    INTEGER NOT NULL DEFAULT 0,
                bot_active_model    TEXT,
                bot_permission_mode TEXT,
                PRIMARY KEY (owner_user_id, bot_user_id, chat_id, topic_id)
            );

            CREATE TABLE IF NOT EXISTS coding_checkpoints (
                owner_user_id   INTEGER NOT NULL DEFAULT 0,
                thread_id       TEXT NOT NULL,
                turn_idx        INTEGER NOT NULL,
                before_ref      TEXT NOT NULL,
                after_ref       TEXT NOT NULL,
                diff_stat_json  TEXT NOT NULL,
                prompt_preview  TEXT NOT NULL,
                reply_preview   TEXT NOT NULL,
                created_at_ms   INTEGER NOT NULL,
                PRIMARY KEY (owner_user_id, thread_id, turn_idx)
            );

            CREATE TABLE IF NOT EXISTS command_receipts (
                owner_user_id         INTEGER NOT NULL DEFAULT 0,
                turn_id               TEXT NOT NULL,
                thread_id             TEXT NOT NULL,
                status                TEXT NOT NULL,
                turn_idx              INTEGER NOT NULL DEFAULT 0,
                completed_event_json  TEXT,
                created_at_ms         INTEGER NOT NULL,
                updated_at_ms         INTEGER NOT NULL,
                PRIMARY KEY (owner_user_id, turn_id)
            );
            ",
        )
        .map_err(|e| format!("failed to initialize coding store schema: {e}"))?;

        Self::migrate_coding_threads_baseline_columns(&conn)?;
        Self::migrate_owner_user_id_tables(&conn)?;

        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_coding_threads_thread_id ON coding_threads(owner_user_id, thread_id);",
        )
        .map_err(|e| format!("failed to create coding_threads index: {e}"))?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn table_has_col(conn: &Connection, table: &str, col: &str) -> Result<bool, String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| format!("failed to inspect {table} schema: {e}"))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| format!("failed to read {table} columns: {e}"))?;
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("failed to iterate {table} columns: {e}"))?
        {
            let name: String = row
                .get(1)
                .map_err(|e| format!("failed to decode {table} column name: {e}"))?;
            if name == col {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn table_has_owner_pk(conn: &Connection, table: &str) -> Result<bool, String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| format!("failed to inspect {table} schema: {e}"))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| format!("failed to read {table} columns: {e}"))?;
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("failed to iterate {table} columns: {e}"))?
        {
            let name: String = row
                .get(1)
                .map_err(|e| format!("failed to decode {table} column name: {e}"))?;
            let pk: i64 = row
                .get(5)
                .map_err(|e| format!("failed to decode {table} pk flag: {e}"))?;
            if name == "owner_user_id" && pk > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn migrate_owner_user_id_tables(conn: &Connection) -> Result<(), String> {
        let threads_ok = Self::table_has_owner_pk(conn, "coding_threads")?;
        let checkpoints_ok = Self::table_has_owner_pk(conn, "coding_checkpoints")?;
        let receipts_ok = Self::table_has_owner_pk(conn, "command_receipts")?;
        if threads_ok && checkpoints_ok && receipts_ok {
            return Ok(());
        }

        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| format!("failed to begin owner_user_id migration transaction: {e}"))?;

        let run = || -> Result<(), String> {
            if !threads_ok {
                let owner_expr = if Self::table_has_col(conn, "coding_threads", "owner_user_id")? {
                    "owner_user_id"
                } else {
                    "0"
                };
                conn.execute_batch(&format!(
                    "
                    CREATE TABLE coding_threads_new (
                        owner_user_id       INTEGER NOT NULL DEFAULT 0,
                        bot_user_id         INTEGER NOT NULL,
                        chat_id             INTEGER NOT NULL,
                        topic_id            INTEGER NOT NULL,
                        thread_id           TEXT NOT NULL,
                        provider_session_id TEXT,
                        workspace_path      TEXT NOT NULL,
                        worktree_path       TEXT,
                        branch_name         TEXT,
                        use_worktree        INTEGER NOT NULL DEFAULT 0,
                        active_model        TEXT,
                        permission_mode     TEXT NOT NULL,
                        turn_count          INTEGER NOT NULL DEFAULT 0,
                        last_summary        TEXT,
                        updated_at_ms       INTEGER NOT NULL,
                        bot_provider_kind   TEXT,
                        bot_workspace_path  TEXT NOT NULL DEFAULT '',
                        bot_use_worktree    INTEGER NOT NULL DEFAULT 0,
                        bot_active_model    TEXT,
                        bot_permission_mode TEXT,
                        PRIMARY KEY (owner_user_id, bot_user_id, chat_id, topic_id)
                    );
                    INSERT INTO coding_threads_new (
                        owner_user_id, bot_user_id, chat_id, topic_id, thread_id,
                        provider_session_id, workspace_path, worktree_path, branch_name,
                        use_worktree, active_model, permission_mode, turn_count, last_summary,
                        updated_at_ms, bot_provider_kind, bot_workspace_path, bot_use_worktree,
                        bot_active_model, bot_permission_mode
                    )
                    SELECT
                        {owner_expr}, bot_user_id, chat_id, topic_id, thread_id,
                        provider_session_id, workspace_path, worktree_path, branch_name,
                        use_worktree, active_model, permission_mode, turn_count, last_summary,
                        updated_at_ms, bot_provider_kind, bot_workspace_path, bot_use_worktree,
                        bot_active_model, bot_permission_mode
                    FROM coding_threads;
                    DROP TABLE coding_threads;
                    ALTER TABLE coding_threads_new RENAME TO coding_threads;
                    "
                ))
                .map_err(|e| format!("failed to migrate coding_threads owner_user_id: {e}"))?;
            }

            if !checkpoints_ok {
                let owner_expr = if Self::table_has_col(conn, "coding_checkpoints", "owner_user_id")? {
                    "owner_user_id"
                } else {
                    "0"
                };
                conn.execute_batch(&format!(
                    "
                    CREATE TABLE coding_checkpoints_new (
                        owner_user_id   INTEGER NOT NULL DEFAULT 0,
                        thread_id       TEXT NOT NULL,
                        turn_idx        INTEGER NOT NULL,
                        before_ref      TEXT NOT NULL,
                        after_ref       TEXT NOT NULL,
                        diff_stat_json  TEXT NOT NULL,
                        prompt_preview  TEXT NOT NULL,
                        reply_preview   TEXT NOT NULL,
                        created_at_ms   INTEGER NOT NULL,
                        PRIMARY KEY (owner_user_id, thread_id, turn_idx)
                    );
                    INSERT INTO coding_checkpoints_new (
                        owner_user_id, thread_id, turn_idx, before_ref, after_ref,
                        diff_stat_json, prompt_preview, reply_preview, created_at_ms
                    )
                    SELECT
                        {owner_expr}, thread_id, turn_idx, before_ref, after_ref,
                        diff_stat_json, prompt_preview, reply_preview, created_at_ms
                    FROM coding_checkpoints;
                    DROP TABLE coding_checkpoints;
                    ALTER TABLE coding_checkpoints_new RENAME TO coding_checkpoints;
                    "
                ))
                .map_err(|e| format!("failed to migrate coding_checkpoints owner_user_id: {e}"))?;
            }

            if !receipts_ok {
                let owner_expr = if Self::table_has_col(conn, "command_receipts", "owner_user_id")? {
                    "owner_user_id"
                } else {
                    "0"
                };
                conn.execute_batch(&format!(
                    "
                    CREATE TABLE command_receipts_new (
                        owner_user_id         INTEGER NOT NULL DEFAULT 0,
                        turn_id               TEXT NOT NULL,
                        thread_id             TEXT NOT NULL,
                        status                TEXT NOT NULL,
                        turn_idx              INTEGER NOT NULL DEFAULT 0,
                        completed_event_json  TEXT,
                        created_at_ms         INTEGER NOT NULL,
                        updated_at_ms         INTEGER NOT NULL,
                        PRIMARY KEY (owner_user_id, turn_id)
                    );
                    INSERT INTO command_receipts_new (
                        owner_user_id, turn_id, thread_id, status, turn_idx,
                        completed_event_json, created_at_ms, updated_at_ms
                    )
                    SELECT
                        {owner_expr}, turn_id, thread_id, status, turn_idx,
                        completed_event_json, created_at_ms, updated_at_ms
                    FROM command_receipts;
                    DROP TABLE command_receipts;
                    ALTER TABLE command_receipts_new RENAME TO command_receipts;
                    "
                ))
                .map_err(|e| format!("failed to migrate command_receipts owner_user_id: {e}"))?;
            }

            Ok(())
        };

        if let Err(err) = run() {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err);
        }

        conn.execute_batch("COMMIT;")
            .map_err(|e| format!("failed to commit owner_user_id migration transaction: {e}"))?;
        Ok(())
    }

    fn migrate_coding_threads_baseline_columns(conn: &Connection) -> Result<(), String> {
        let mut stmt = conn
            .prepare("PRAGMA table_info(coding_threads)")
            .map_err(|e| format!("failed to inspect coding_threads schema: {e}"))?;
        let existing_cols: HashSet<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("failed to read coding_threads columns: {e}"))?
            .filter_map(Result::ok)
            .collect();

        let migrations = [
            (
                "bot_provider_kind",
                "ALTER TABLE coding_threads ADD COLUMN bot_provider_kind TEXT",
            ),
            (
                "bot_workspace_path",
                "ALTER TABLE coding_threads ADD COLUMN bot_workspace_path TEXT NOT NULL DEFAULT ''",
            ),
            (
                "bot_use_worktree",
                "ALTER TABLE coding_threads ADD COLUMN bot_use_worktree INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "bot_active_model",
                "ALTER TABLE coding_threads ADD COLUMN bot_active_model TEXT",
            ),
            (
                "bot_permission_mode",
                "ALTER TABLE coding_threads ADD COLUMN bot_permission_mode TEXT",
            ),
        ];

        for (col, sql) in migrations {
            if !existing_cols.contains(col) {
                conn.execute(sql, [])
                    .map_err(|e| format!("failed to migrate coding_threads.{col}: {e}"))?;
            }
        }
        Ok(())
    }

    /// Queries the active SQLite journal mode (expected `"wal"`).
    pub fn journal_mode(&self) -> Result<String, String> {
        let conn = self.conn.lock();
        conn.query_row("PRAGMA journal_mode;", [], |row| row.get::<_, String>(0))
            .map_err(|e| format!("failed to read journal_mode: {e}"))
    }

    pub fn get_thread(&self, key: ThreadKey) -> Result<Option<ThreadRecord>, String> {
        let conn = self.conn.lock();
        conn.query_row(
            "
            SELECT owner_user_id, bot_user_id, chat_id, topic_id, thread_id, provider_session_id,
                   workspace_path, worktree_path, branch_name, use_worktree,
                   active_model, permission_mode, turn_count, last_summary, updated_at_ms,
                   bot_provider_kind, bot_workspace_path, bot_use_worktree,
                   bot_active_model, bot_permission_mode
            FROM coding_threads
            WHERE owner_user_id = ?1 AND bot_user_id = ?2 AND chat_id = ?3 AND topic_id = ?4
            ",
            params![key.owner_user_id as i64, key.bot_user_id, key.chat_id, key.topic_id],
            |row| {
                let owner_i64: i64 = row.get(0)?;
                let use_wt_i64: i64 = row.get(9)?;
                let perm_raw: String = row.get(11)?;
                let turn_count_i64: i64 = row.get(12)?;
                let bot_provider_raw: Option<String> = row.get(15)?;
                let bot_use_wt_i64: i64 = row.get(17)?;
                let bot_perm_raw: Option<String> = row.get(19)?;
                Ok(ThreadRecord {
                    owner_user_id: owner_i64.max(0) as u64,
                    bot_user_id: row.get(1)?,
                    chat_id: row.get(2)?,
                    topic_id: row.get(3)?,
                    thread_id: row.get(4)?,
                    provider_session_id: row.get(5)?,
                    workspace_path: row.get(6)?,
                    worktree_path: row.get(7)?,
                    branch_name: row.get(8)?,
                    use_worktree: use_wt_i64 != 0,
                    active_model: row.get(10)?,
                    permission_mode: CodingPermissionMode::parse(&perm_raw).unwrap_or_default(),
                    turn_count: turn_count_i64.max(0) as u64,
                    last_summary: row.get(13)?,
                    updated_at_ms: row.get(14)?,
                    bot_provider_kind: bot_provider_raw
                        .as_deref()
                        .and_then(CodingProviderKind::parse),
                    bot_workspace_path: row.get(16)?,
                    bot_use_worktree: bot_use_wt_i64 != 0,
                    bot_active_model: row.get(18)?,
                    bot_permission_mode: bot_perm_raw
                        .as_deref()
                        .and_then(CodingPermissionMode::parse),
                })
            },
        )
        .optional()
        .map_err(|e| format!("failed to query coding_threads: {e}"))
    }

    pub fn upsert_thread(&self, rec: &ThreadRecord) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "
            INSERT INTO coding_threads (
                owner_user_id, bot_user_id, chat_id, topic_id, thread_id, provider_session_id,
                workspace_path, worktree_path, branch_name, use_worktree,
                active_model, permission_mode, turn_count, last_summary, updated_at_ms,
                bot_provider_kind, bot_workspace_path, bot_use_worktree,
                bot_active_model, bot_permission_mode
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
            ON CONFLICT(owner_user_id, bot_user_id, chat_id, topic_id) DO UPDATE SET
                thread_id           = excluded.thread_id,
                provider_session_id = excluded.provider_session_id,
                workspace_path      = excluded.workspace_path,
                worktree_path       = excluded.worktree_path,
                branch_name         = excluded.branch_name,
                use_worktree        = excluded.use_worktree,
                active_model        = excluded.active_model,
                permission_mode     = excluded.permission_mode,
                turn_count          = excluded.turn_count,
                last_summary        = excluded.last_summary,
                updated_at_ms       = excluded.updated_at_ms,
                bot_provider_kind   = excluded.bot_provider_kind,
                bot_workspace_path  = excluded.bot_workspace_path,
                bot_use_worktree    = excluded.bot_use_worktree,
                bot_active_model    = excluded.bot_active_model,
                bot_permission_mode = excluded.bot_permission_mode
            ",
            params![
                rec.owner_user_id as i64,
                rec.bot_user_id,
                rec.chat_id,
                rec.topic_id,
                rec.thread_id,
                rec.provider_session_id,
                rec.workspace_path,
                rec.worktree_path,
                rec.branch_name,
                if rec.use_worktree { 1_i64 } else { 0_i64 },
                rec.active_model,
                rec.permission_mode.as_str(),
                rec.turn_count as i64,
                rec.last_summary,
                rec.updated_at_ms,
                rec.bot_provider_kind.map(|k| k.as_str()),
                rec.bot_workspace_path,
                if rec.bot_use_worktree { 1_i64 } else { 0_i64 },
                rec.bot_active_model,
                rec.bot_permission_mode.map(|m| m.as_str()),
            ],
        )
        .map_err(|e| format!("failed to upsert coding_threads: {e}"))?;
        Ok(())
    }

    pub fn save_checkpoint(&self, cp: &CheckpointRecord) -> Result<(), String> {
        let diff_json = serde_json::to_string(&cp.diff_stat)
            .map_err(|e| format!("failed to serialize CodingDiffStat: {e}"))?;
        let conn = self.conn.lock();
        conn.execute(
            "
            INSERT INTO coding_checkpoints (
                owner_user_id, thread_id, turn_idx, before_ref, after_ref, diff_stat_json,
                prompt_preview, reply_preview, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(owner_user_id, thread_id, turn_idx) DO UPDATE SET
                before_ref     = excluded.before_ref,
                after_ref      = excluded.after_ref,
                diff_stat_json = excluded.diff_stat_json,
                prompt_preview = excluded.prompt_preview,
                reply_preview  = excluded.reply_preview,
                created_at_ms  = excluded.created_at_ms
            ",
            params![
                cp.owner_user_id as i64,
                cp.thread_id,
                cp.turn_idx as i64,
                cp.before_ref,
                cp.after_ref,
                diff_json,
                cp.prompt_preview,
                cp.reply_preview,
                cp.created_at_ms,
            ],
        )
        .map_err(|e| format!("failed to save coding_checkpoints: {e}"))?;
        Ok(())
    }

    pub fn get_checkpoint(
        &self,
        owner_user_id: u64,
        thread_id: &str,
        turn_idx: Option<u64>,
    ) -> Result<Option<CheckpointRecord>, String> {
        let conn = self.conn.lock();
        let row_map = |row: &rusqlite::Row<'_>| -> rusqlite::Result<CheckpointRecord> {
            let owner_i64: i64 = row.get(0)?;
            let turn_i64: i64 = row.get(2)?;
            let diff_json: String = row.get(5)?;
            let diff_stat: CodingDiffStat = serde_json::from_str(&diff_json).unwrap_or_default();
            Ok(CheckpointRecord {
                owner_user_id: owner_i64.max(0) as u64,
                thread_id: row.get(1)?,
                turn_idx: turn_i64.max(0) as u64,
                before_ref: row.get(3)?,
                after_ref: row.get(4)?,
                diff_stat,
                prompt_preview: row.get(6)?,
                reply_preview: row.get(7)?,
                created_at_ms: row.get(8)?,
            })
        };

        if let Some(idx) = turn_idx {
            conn.query_row(
                "
                SELECT owner_user_id, thread_id, turn_idx, before_ref, after_ref, diff_stat_json,
                       prompt_preview, reply_preview, created_at_ms
                FROM coding_checkpoints
                WHERE owner_user_id = ?1 AND thread_id = ?2 AND turn_idx = ?3
                ",
                params![owner_user_id as i64, thread_id, idx as i64],
                row_map,
            )
            .optional()
            .map_err(|e| format!("failed to query coding_checkpoints: {e}"))
        } else {
            conn.query_row(
                "
                SELECT owner_user_id, thread_id, turn_idx, before_ref, after_ref, diff_stat_json,
                       prompt_preview, reply_preview, created_at_ms
                FROM coding_checkpoints
                WHERE owner_user_id = ?1 AND thread_id = ?2
                ORDER BY turn_idx DESC
                LIMIT 1
                ",
                params![owner_user_id as i64, thread_id],
                row_map,
            )
            .optional()
            .map_err(|e| format!("failed to query latest coding_checkpoints: {e}"))
        }
    }

    pub fn list_recent_checkpoints(
        &self,
        owner_user_id: u64,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<CheckpointRecord>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "
                SELECT owner_user_id, thread_id, turn_idx, before_ref, after_ref, diff_stat_json,
                       prompt_preview, reply_preview, created_at_ms
                FROM coding_checkpoints
                WHERE owner_user_id = ?1 AND thread_id = ?2
                ORDER BY turn_idx DESC
                LIMIT ?3
                ",
            )
            .map_err(|e| format!("failed to prepare recent checkpoints query: {e}"))?;

        let rows = stmt
            .query_map(params![owner_user_id as i64, thread_id, limit as i64], |row| {
                let owner_i64: i64 = row.get(0)?;
                let turn_i64: i64 = row.get(2)?;
                let diff_json: String = row.get(5)?;
                let diff_stat: CodingDiffStat =
                    serde_json::from_str(&diff_json).unwrap_or_default();
                Ok(CheckpointRecord {
                    owner_user_id: owner_i64.max(0) as u64,
                    thread_id: row.get(1)?,
                    turn_idx: turn_i64.max(0) as u64,
                    before_ref: row.get(3)?,
                    after_ref: row.get(4)?,
                    diff_stat,
                    prompt_preview: row.get(6)?,
                    reply_preview: row.get(7)?,
                    created_at_ms: row.get(8)?,
                })
            })
            .map_err(|e| format!("failed to query recent checkpoints: {e}"))?;

        let mut out = Vec::new();
        for item in rows {
            out.push(item.map_err(|e| format!("failed to decode checkpoint row: {e}"))?);
        }
        out.reverse();
        Ok(out)
    }

    /// Atomically claims `turn_id` in `command_receipts` for idempotent retry protection.
    pub fn claim_turn_receipt(
        &self,
        owner_user_id: u64,
        turn_id: &str,
        thread_id: &str,
    ) -> Result<TurnReceiptClaim, String> {
        let now = now_ms();
        let conn = self.conn.lock();
        let inserted = conn
            .execute(
                "
                INSERT INTO command_receipts (
                    owner_user_id, turn_id, thread_id, status, turn_idx, completed_event_json,
                    created_at_ms, updated_at_ms
                ) VALUES (?1, ?2, ?3, 'running', 0, NULL, ?4, ?4)
                ON CONFLICT(owner_user_id, turn_id) DO NOTHING
                ",
                params![owner_user_id as i64, turn_id, thread_id, now],
            )
            .map_err(|e| format!("failed to claim turn receipt: {e}"))?;

        if inserted == 1 {
            return Ok(TurnReceiptClaim::Claimed);
        }

        let row: Option<(String, i64, Option<String>)> = conn
            .query_row(
                "
                SELECT status, turn_idx, completed_event_json
                FROM command_receipts
                WHERE owner_user_id = ?1 AND turn_id = ?2
                ",
                params![owner_user_id as i64, turn_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| format!("failed to read existing turn receipt: {e}"))?;

        match row {
            None => Ok(TurnReceiptClaim::Claimed),
            Some((status, _idx, _)) if status == "running" => Ok(TurnReceiptClaim::InProgress),
            Some((status, idx, Some(json))) => {
                if let Ok(ev) = serde_json::from_str::<CodingHostEvent>(&json) {
                    Ok(TurnReceiptClaim::AlreadyCompleted(ev))
                } else {
                    Ok(TurnReceiptClaim::FinishedWithoutEvent {
                        turn_idx: idx.max(0) as u64,
                        status,
                    })
                }
            }
            Some((status, idx, None)) => Ok(TurnReceiptClaim::FinishedWithoutEvent {
                turn_idx: idx.max(0) as u64,
                status,
            }),
        }
    }

    /// Updates `command_receipts` once a turn finishes (success, failure, or cancellation).
    pub fn finish_turn_receipt(
        &self,
        owner_user_id: u64,
        turn_id: &str,
        status: &str,
        turn_idx: u64,
        completed_event: Option<&CodingHostEvent>,
    ) -> Result<(), String> {
        let now = now_ms();
        let event_json = completed_event
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| format!("failed to serialize completed receipt event: {e}"))?;
        let conn = self.conn.lock();
        conn.execute(
            "
            UPDATE command_receipts
            SET status = ?3,
                turn_idx = ?4,
                completed_event_json = ?5,
                updated_at_ms = ?6
            WHERE owner_user_id = ?1 AND turn_id = ?2
            ",
            params![owner_user_id as i64, turn_id, status, turn_idx as i64, event_json, now],
        )
        .map_err(|e| format!("failed to finalize turn receipt: {e}"))?;
        Ok(())
    }

    pub fn wal_checkpoint_truncate(&self) -> Result<(i32, i32, i32), String> {
        let conn = self.conn.lock();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| format!("failed to checkpoint coding_store WAL: {e}"))
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn preview_text(raw: &str, max_chars: usize) -> String {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        let mut s: String = normalized.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn store_enables_wal_creates_parents_and_enforces_idempotency() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("nested").join("coding").join("state.db");
        let store = CodingStore::open(&db_path).unwrap();

        assert_eq!(store.journal_mode().unwrap().to_lowercase(), "wal");

        let key = ThreadKey::new(800_001, -100_123, None);
        assert_eq!(key.topic_id, 0);
        assert!(store.get_thread(key).unwrap().is_none());

        let rec = ThreadRecord {
            owner_user_id: key.owner_user_id,
            bot_user_id: key.bot_user_id,
            chat_id: key.chat_id,
            topic_id: key.topic_id,
            thread_id: "thr_1".to_string(),
            provider_session_id: Some("sess_1".to_string()),
            workspace_path: "/repo".to_string(),
            worktree_path: None,
            branch_name: Some("main".to_string()),
            use_worktree: false,
            active_model: Some("sonnet".to_string()),
            permission_mode: CodingPermissionMode::Ask,
            turn_count: 1,
            last_summary: Some("done".to_string()),
            updated_at_ms: now_ms(),
            bot_provider_kind: Some(CodingProviderKind::ClaudeCode),
            bot_workspace_path: "/repo".to_string(),
            bot_use_worktree: false,
            bot_active_model: Some("sonnet".to_string()),
            bot_permission_mode: Some(CodingPermissionMode::Ask),
        };
        store.upsert_thread(&rec).unwrap();
        let loaded = store.get_thread(key).unwrap().unwrap();
        assert_eq!(loaded, rec);

        // Save checkpoint and fetch by explicit turn_idx and None (latest).
        let cp = CheckpointRecord {
            owner_user_id: 0,
            thread_id: "thr_1".to_string(),
            turn_idx: 1,
            before_ref: "refs/fabushi-coding/checkpoints/thr_1/1-before".to_string(),
            after_ref: "refs/fabushi-coding/checkpoints/thr_1/1".to_string(),
            diff_stat: CodingDiffStat {
                files_changed: 1,
                insertions: 5,
                deletions: 2,
                changed_files: vec!["src/lib.rs".to_string()],
            },
            prompt_preview: "fix bug".to_string(),
            reply_preview: "fixed".to_string(),
            created_at_ms: now_ms(),
        };
        store.save_checkpoint(&cp).unwrap();
        assert_eq!(store.get_checkpoint(0, "thr_1", Some(1)).unwrap(), Some(cp.clone()));
        assert_eq!(store.get_checkpoint(0, "thr_1", None).unwrap(), Some(cp));

        // Idempotent command receipts.
        assert!(matches!(
            store.claim_turn_receipt(0, "turn_1", "thr_1").unwrap(),
            TurnReceiptClaim::Claimed
        ));
        assert!(matches!(
            store.claim_turn_receipt(0, "turn_1", "thr_1").unwrap(),
            TurnReceiptClaim::InProgress
        ));

        let completed_ev = CodingHostEvent::TurnCompleted {
            turn_id: "turn_1".to_string(),
            ok: true,
            final_text: "all good".to_string(),
            error: None,
            turn_idx: 1,
            checkpoint_ref: Some("refs/fabushi-coding/checkpoints/thr_1/1".to_string()),
            diff_stat: None,
            token_usage: None,
        };
        store
            .finish_turn_receipt(0, "turn_1", "completed", 1, Some(&completed_ev))
            .unwrap();

        match store.claim_turn_receipt(0, "turn_1", "thr_1").unwrap() {
            TurnReceiptClaim::AlreadyCompleted(ev) => assert_eq!(ev, completed_ev),
            other => panic!("expected AlreadyCompleted, got {other:?}"),
        }
    }

    #[test]
    fn store_migrates_legacy_coding_threads_table_idempotently() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("legacy.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE coding_threads (
                    bot_user_id         INTEGER NOT NULL,
                    chat_id             INTEGER NOT NULL,
                    topic_id            INTEGER NOT NULL,
                    thread_id           TEXT NOT NULL,
                    provider_session_id TEXT,
                    workspace_path      TEXT NOT NULL,
                    worktree_path       TEXT,
                    branch_name         TEXT,
                    use_worktree        INTEGER NOT NULL DEFAULT 0,
                    active_model        TEXT,
                    permission_mode     TEXT NOT NULL,
                    turn_count          INTEGER NOT NULL DEFAULT 0,
                    last_summary        TEXT,
                    updated_at_ms       INTEGER NOT NULL,
                    PRIMARY KEY (bot_user_id, chat_id, topic_id)
                );
                INSERT INTO coding_threads (
                    bot_user_id, chat_id, topic_id, thread_id, workspace_path,
                    use_worktree, permission_mode, turn_count, updated_at_ms
                ) VALUES (10, 20, 0, 'thr_legacy', '/old', 0, 'full_access', 2, 1000);

                CREATE TABLE coding_checkpoints (
                    thread_id       TEXT NOT NULL,
                    turn_idx        INTEGER NOT NULL,
                    before_ref      TEXT NOT NULL,
                    after_ref       TEXT NOT NULL,
                    diff_stat_json  TEXT NOT NULL,
                    prompt_preview  TEXT NOT NULL,
                    reply_preview   TEXT NOT NULL,
                    created_at_ms   INTEGER NOT NULL,
                    PRIMARY KEY (thread_id, turn_idx)
                );
                INSERT INTO coding_checkpoints (
                    thread_id, turn_idx, before_ref, after_ref, diff_stat_json,
                    prompt_preview, reply_preview, created_at_ms
                ) VALUES ('thr_legacy', 1, 'b1', 'a1', '{}', 'p', 'r', 1000);

                CREATE TABLE command_receipts (
                    turn_id               TEXT PRIMARY KEY,
                    thread_id             TEXT NOT NULL,
                    status                TEXT NOT NULL,
                    turn_idx              INTEGER NOT NULL DEFAULT 0,
                    completed_event_json  TEXT,
                    created_at_ms         INTEGER NOT NULL,
                    updated_at_ms         INTEGER NOT NULL
                );
                INSERT INTO command_receipts (
                    turn_id, thread_id, status, turn_idx, completed_event_json,
                    created_at_ms, updated_at_ms
                ) VALUES ('turn_legacy', 'thr_legacy', 'completed', 1, NULL, 1000, 1000);
                ",
            )
            .unwrap();
        }

        let store = CodingStore::open(&db_path).unwrap();
        let rec = store.get_thread(ThreadKey::new(10, 20, None)).unwrap().unwrap();
        assert_eq!(rec.owner_user_id, 0);
        assert_eq!(rec.thread_id, "thr_legacy");
        assert_eq!(rec.bot_workspace_path, "");
        assert_eq!(rec.bot_permission_mode, None);

        // Real authenticated owner (e.g. 1001) must NOT see migrated owner_user_id=0 rows.
        assert!(
            store
                .get_thread(ThreadKey::for_owner(1001, 10, 20, None))
                .unwrap()
                .is_none()
        );
        assert!(store.get_checkpoint(1001, "thr_legacy", Some(1)).unwrap().is_none());
        assert!(store.get_checkpoint(0, "thr_legacy", Some(1)).unwrap().is_some());

        // Upserting for owner 1001 with colliding (10, 20, 0) must succeed without PK conflict.
        let rec_1001 = ThreadRecord {
            owner_user_id: 1001,
            bot_user_id: 10,
            chat_id: 20,
            topic_id: 0,
            thread_id: "thr_1001".to_string(),
            provider_session_id: None,
            workspace_path: "/new".to_string(),
            worktree_path: None,
            branch_name: None,
            use_worktree: false,
            active_model: None,
            permission_mode: CodingPermissionMode::Ask,
            turn_count: 0,
            last_summary: None,
            updated_at_ms: 2000,
            bot_provider_kind: None,
            bot_workspace_path: "/new".to_string(),
            bot_use_worktree: false,
            bot_active_model: None,
            bot_permission_mode: Some(CodingPermissionMode::Ask),
        };
        store.upsert_thread(&rec_1001).unwrap();
        assert_eq!(
            store.get_thread(ThreadKey::for_owner(1001, 10, 20, None)).unwrap().unwrap().thread_id,
            "thr_1001"
        );
        assert_eq!(
            store.get_thread(ThreadKey::new(10, 20, None)).unwrap().unwrap().thread_id,
            "thr_legacy"
        );

        // Re-opening must be a clean idempotent no-op.
        let store2 = CodingStore::open(&db_path).unwrap();
        assert!(store2.get_thread(ThreadKey::new(10, 20, None)).unwrap().is_some());
        assert!(store2.get_thread(ThreadKey::for_owner(1001, 10, 20, None)).unwrap().is_some());
    }

    #[test]
    fn store_isolates_threads_checkpoints_and_receipts_by_owner_user_id() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("shared.db");
        let store = CodingStore::open(&db_path).unwrap();

        let key_alice = ThreadKey::for_owner(1001, 500, 102, None);
        let key_bob = ThreadKey::for_owner(2002, 500, 102, None);

        let rec_alice = ThreadRecord {
            owner_user_id: 1001,
            bot_user_id: 500,
            chat_id: 102,
            topic_id: 0,
            thread_id: "thr_shared".to_string(),
            provider_session_id: Some("secret_claude_sess_alice".to_string()),
            workspace_path: "/Users/alice/private-repo".to_string(),
            worktree_path: Some("/tmp/wt-alice".to_string()),
            branch_name: Some("fabushi/alice".to_string()),
            use_worktree: true,
            active_model: Some("alice-model".to_string()),
            permission_mode: CodingPermissionMode::AutoEdit,
            turn_count: 17,
            last_summary: Some("alice secret summary".to_string()),
            updated_at_ms: now_ms(),
            bot_provider_kind: Some(CodingProviderKind::ClaudeCode),
            bot_workspace_path: "/Users/alice/private-repo".to_string(),
            bot_use_worktree: true,
            bot_active_model: Some("alice-model".to_string()),
            bot_permission_mode: Some(CodingPermissionMode::AutoEdit),
        };
        store.upsert_thread(&rec_alice).unwrap();

        assert!(store.get_thread(key_bob).unwrap().is_none());
        assert_eq!(store.get_thread(key_alice).unwrap(), Some(rec_alice.clone()));

        let rec_bob = ThreadRecord {
            owner_user_id: 2002,
            bot_user_id: 500,
            chat_id: 102,
            topic_id: 0,
            thread_id: "thr_shared".to_string(),
            provider_session_id: None,
            workspace_path: "/Users/bob/clean-repo".to_string(),
            worktree_path: None,
            branch_name: Some("main".to_string()),
            use_worktree: false,
            active_model: None,
            permission_mode: CodingPermissionMode::Ask,
            turn_count: 0,
            last_summary: None,
            updated_at_ms: now_ms(),
            bot_provider_kind: Some(CodingProviderKind::ClaudeCode),
            bot_workspace_path: "/Users/bob/clean-repo".to_string(),
            bot_use_worktree: false,
            bot_active_model: None,
            bot_permission_mode: Some(CodingPermissionMode::Ask),
        };
        store.upsert_thread(&rec_bob).unwrap();
        assert_eq!(store.get_thread(key_alice).unwrap(), Some(rec_alice));
        assert_eq!(store.get_thread(key_bob).unwrap(), Some(rec_bob));

        let cp_alice = CheckpointRecord {
            owner_user_id: 1001,
            thread_id: "thr_shared".to_string(),
            turn_idx: 1,
            before_ref: "refs/a/before".to_string(),
            after_ref: "refs/a/after".to_string(),
            diff_stat: CodingDiffStat::default(),
            prompt_preview: "alice prompt".to_string(),
            reply_preview: "alice reply".to_string(),
            created_at_ms: now_ms(),
        };
        store.save_checkpoint(&cp_alice).unwrap();

        assert!(store.get_checkpoint(2002, "thr_shared", Some(1)).unwrap().is_none());
        assert!(store.get_checkpoint(2002, "thr_shared", None).unwrap().is_none());
        assert!(store.list_recent_checkpoints(2002, "thr_shared", 5).unwrap().is_empty());
        assert_eq!(store.get_checkpoint(1001, "thr_shared", Some(1)).unwrap(), Some(cp_alice));

        assert!(matches!(
            store.claim_turn_receipt(1001, "shared_turn_1", "thr_shared").unwrap(),
            TurnReceiptClaim::Claimed
        ));
        let ev_alice = CodingHostEvent::TurnCompleted {
            turn_id: "shared_turn_1".to_string(),
            ok: true,
            final_text: "alice confidential result".to_string(),
            error: None,
            turn_idx: 1,
            checkpoint_ref: None,
            diff_stat: None,
            token_usage: None,
        };
        store
            .finish_turn_receipt(1001, "shared_turn_1", "completed", 1, Some(&ev_alice))
            .unwrap();

        assert!(matches!(
            store.claim_turn_receipt(2002, "shared_turn_1", "thr_shared").unwrap(),
            TurnReceiptClaim::Claimed
        ));
        store
            .finish_turn_receipt(2002, "shared_turn_1", "completed", 1, None)
            .unwrap();
        assert!(matches!(
            store.claim_turn_receipt(1001, "shared_turn_1", "thr_shared").unwrap(),
            TurnReceiptClaim::AlreadyCompleted(_)
        ));
    }
}
