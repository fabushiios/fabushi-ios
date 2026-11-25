//! SQLite STRICT WAL V42 storage backend (`CommunityStore`) for server & offline client runtimes.

pub mod actors;
pub mod communities;
pub mod comments;
pub mod posts;
pub mod schema;
pub mod social;
#[cfg(test)]
mod tests;

pub use actors::{TelegramBotSyncInput, TelegramHumanSyncInput};
pub use schema::MIGRATION_V42_COMMUNITY_SQL;

use crate::recommender::scoring::{reddit_hot, wilson_lower_bound};
use crate::seed::build_starter_dataset;
use gpui_telegram_core::{Result, TelegramError};
use parking_lot::Mutex;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) fn db_err(context: &str, err: rusqlite::Error) -> TelegramError {
    TelegramError::Database(format!("{context}: {err}"))
}

/// Low-level connection operations callable directly inside `ServerDatabase::pool().with_writer(...)` in M2.
pub mod ops {
    pub use super::actors::{
        follow_actor_tx, get_actor_conn, list_actors_conn, list_identities_for_user_conn,
        list_mutes_blocks_conn, remove_mute_block_tx, set_mute_block_tx,
        sync_telegram_identities_tx, unfollow_actor_tx, update_actor_tx, upsert_actor_conn,
    };
    pub use super::comments::{
        create_comment_tx, delete_comment_tx, list_comment_tree_conn, toggle_like_comment_tx,
        update_comment_tx, vote_comment_tx,
    };
    pub use super::communities::{
        apply_mod_action_tx, create_community_tx, enqueue_mod_item_tx, get_community_conn,
        join_community_tx, leave_community_tx, list_communities_conn, list_mod_logs_conn,
        list_mod_queue_conn, list_wiki_revisions_conn, update_community_tx, update_wiki_tx,
    };
    pub use super::posts::{
        close_expired_polls_tx, create_post_tx, delete_post_tx, get_post_conn, give_award_tx,
        is_media_referenced_publicly_conn, list_post_edits_conn, publish_due_scheduled_posts_tx,
        toggle_bookmark_tx, toggle_like_post_tx, toggle_repost_tx, update_post_tx, vote_poll_tx,
        vote_post_tx,
    };
    pub use super::social::{
        check_and_record_bot_cooldown_tx, create_list_tx, delete_list_tx, get_algo_prefs_conn,
        get_feed_conn, get_trending_conn, list_lists_conn, list_notifications_conn,
        mark_notifications_read_tx, put_algo_prefs_tx, record_signal_tx, update_list_tx,
    };
    pub use super::{apply_migrations_conn, apply_pragmas_conn, seed_if_empty_tx};
}

/// Applies production WAL + foreign_keys + busy_timeout pragmas on a SQLite connection.
pub fn apply_pragmas_conn(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA busy_timeout = 5000;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
        PRAGMA temp_store = MEMORY;
        PRAGMA cache_size = -32000;
        ",
    )
    .map_err(|e| db_err("apply community pragmas", e))
}

/// Runs idempotent `MIGRATION_V42_COMMUNITY_SQL` DDL on a SQLite connection.
pub fn apply_migrations_conn(conn: &Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_V42_COMMUNITY_SQL)
        .map_err(|e| db_err("apply MIGRATION_V42_COMMUNITY_SQL", e))
}

/// Thread-safe SQLite STRICT WAL Community store.
#[derive(Clone)]
pub struct CommunityStore {
    conn: Arc<Mutex<Connection>>,
    db_path: Option<PathBuf>,
    // MUST BE LAST FIELD: Connection drops and closes WAL files before TempDir removes the directory!
    _ephemeral_dir: Option<Arc<tempfile::TempDir>>,
}

impl CommunityStore {
    /// Opens (or creates) a persistent SQLite WAL database at `path` and applies V42 migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref();
        if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(p).map_err(|e| db_err("open community sqlite", e))?;
        apply_pragmas_conn(&conn)?;
        apply_migrations_conn(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: Some(p.to_path_buf()),
            _ephemeral_dir: None,
        })
    }

    /// Opens an ephemeral WAL database inside a RAII `tempfile::TempDir` (`gpui_tg_comm_db_*`).
    pub fn open_in_memory() -> Result<Self> {
        let tmp = tempfile::Builder::new()
            .prefix("gpui_tg_comm_db_")
            .tempdir()?;
        let db_file = tmp.path().join("community_inmem.sqlite");
        let conn = Connection::open(&db_file).map_err(|e| db_err("open ephemeral sqlite", e))?;
        apply_pragmas_conn(&conn)?;
        apply_migrations_conn(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: Some(db_file),
            _ephemeral_dir: Some(Arc::new(tmp)),
        })
    }

    /// Wraps an already-opened `rusqlite::Connection`, applying pragmas and V42 migrations.
    pub fn from_connection(conn: Connection) -> Result<Self> {
        apply_pragmas_conn(&conn)?;
        apply_migrations_conn(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: None,
            _ephemeral_dir: None,
        })
    }

    pub fn apply_pragmas(conn: &Connection) -> Result<()> {
        apply_pragmas_conn(conn)
    }

    pub fn apply_migrations(conn: &Connection) -> Result<()> {
        apply_migrations_conn(conn)
    }

    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    /// Executes a closure holding the connection mutex.
    pub fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        let conn = self.conn.lock();
        f(&conn)
    }

    /// Executes an atomic `IMMEDIATE` transaction with automatic rollback on `Err`.
    pub fn with_transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Transaction<'_>) -> Result<R>,
    {
        let mut conn = self.conn.lock();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| db_err("begin immediate tx", e))?;
        match f(&tx) {
            Ok(val) => {
                tx.commit().map_err(|e| db_err("commit tx", e))?;
                Ok(val)
            }
            Err(err) => {
                let _ = tx.rollback();
                Err(err)
            }
        }
    }

    /// Idempotently populates the starter dataset if `communities` is empty.
    pub fn seed_if_empty(&self, now_ms: i64) -> Result<bool> {
        self.with_transaction(|tx| seed_if_empty_tx(tx, now_ms))
    }
}

/// Seeds the starter dataset atomically inside `tx` if no communities exist yet.
pub fn seed_if_empty_tx(tx: &Transaction<'_>, now_ms: i64) -> Result<bool> {
    let existing: i64 = tx
        .query_row("SELECT COUNT(*) FROM communities", [], |r| r.get(0))
        .map_err(|e| db_err("count communities", e))?;
    if existing > 0 {
        return Ok(false);
    }

    let ds = build_starter_dataset(now_ms);

    // 1. Actors
    for actor in &ds.actors {
        actors::upsert_actor_conn(tx, actor, now_ms)?;
    }

    // 2. Follows
    for (follower, target, ts) in &ds.follows {
        tx.execute(
            "INSERT OR IGNORE INTO community_follows (follower_actor_id, target_actor_id, created_at_ms) VALUES (?1, ?2, ?3)",
            params![follower, target, ts],
        )
        .map_err(|e| db_err("seed follow", e))?;
    }

    // 3. Communities + Wiki v1
    for comm in &ds.communities {
        tx.execute(
            "INSERT OR IGNORE INTO communities (
                community_id, slug, name, description, category, icon_emoji, icon_media_id, banner_media_id,
                accent_color, creator_actor_id, rules_json, post_flairs_json, user_flairs_json,
                automod_rules_json, pinned_post_ids_json, wiki_markdown, wiki_version,
                allow_bots, require_bot_disclosure, is_nsfw, members_count, posts_count, created_at_ms, updated_at_ms
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
            params![
                comm.community_id,
                comm.slug,
                comm.name,
                comm.description,
                comm.category,
                comm.icon_emoji,
                comm.icon_media_id,
                comm.banner_media_id,
                comm.accent_color,
                comm.created_by_actor_id,
                serde_json::to_string(&comm.rules)?,
                serde_json::to_string(&comm.post_flairs)?,
                serde_json::to_string(&comm.user_flairs)?,
                serde_json::to_string(&comm.automod_rules)?,
                serde_json::to_string(&comm.pinned_post_ids)?,
                comm.wiki_markdown,
                i64::from(comm.wiki_revision.max(1)),
                comm.allow_bots as i64,
                comm.require_bot_disclosure as i64,
                comm.is_nsfw as i64,
                comm.member_count as i64,
                comm.posts_count as i64,
                comm.created_at_ms,
                now_ms,
            ],
        )
        .map_err(|e| db_err("seed community", e))?;

        tx.execute(
            "INSERT OR IGNORE INTO community_wiki_revisions (revision_id, community_id, version, editor_actor_id, content_markdown, summary, created_at_ms)
             VALUES (?1, ?2, 1, ?3, ?4, 'Initial community wiki', ?5)",
            params![
                format!("rev_{}_1", comm.community_id),
                comm.community_id,
                comm.created_by_actor_id,
                comm.wiki_markdown,
                comm.created_at_ms,
            ],
        )
        .map_err(|e| db_err("seed wiki rev", e))?;
    }

    // 4. Memberships
    for m in &ds.memberships {
        tx.execute(
            "INSERT OR IGNORE INTO community_memberships (community_id, actor_id, role, notifications_enabled, joined_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, 1, ?4, ?4)",
            params![m.community_id, m.actor_id, m.role.as_str(), m.joined_at_ms],
        )
        .map_err(|e| db_err("seed membership", e))?;
    }

    // 5. Posts + Awards
    for p in &ds.posts {
        let hot = reddit_hot(p.upvotes, p.downvotes, p.created_at_ms);
        let wilson = wilson_lower_bound(p.upvotes, p.downvotes, 1.96);
        let poll_json = p.poll.as_ref().map(serde_json::to_string).transpose()?;
        let poll_closes = p.poll.as_ref().map(|pl| pl.expires_at_ms);
        let poll_closed = p.poll.as_ref().is_some_and(|pl| pl.is_closed) as i64;
        let lp_json = p.link_preview.as_ref().map(serde_json::to_string).transpose()?;
        let link_url = p.link_preview.as_ref().map(|lp| lp.url.clone());

        tx.execute(
            "INSERT OR IGNORE INTO community_posts (
                post_id, community_id, author_actor_id, co_author_actor_id, post_format,
                title, body_markdown, link_url, link_preview_json, media_json, flair_text,
                thread_root_id, parent_post_id, thread_index, thread_total,
                quote_post_id, repost_of_post_id, crosspost_of_post_id,
                poll_json, poll_closes_at_ms, poll_is_closed,
                hashtags_json, mentions_json,
                is_spoiler, is_nsfw, is_pinned, is_locked, is_distinguished_mod, is_removed,
                scheduled_at_ms, status,
                upvotes, downvotes, score, likes_count, comments_count, reposts_count, quotes_count,
                bookmarks_count, views_count, awards_count, hot_score, wilson_score,
                created_at_ms, updated_at_ms
             ) VALUES (
                ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
                ?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,
                ?32,?33,?34,?35,?36,?37,?38,?39,?40,?41,?42,?43,?44,?45
             )",
            params![
                p.post_id, p.community_id, p.author_actor_id, p.co_author_actor_id, p.format.as_str(),
                p.title, p.body, link_url, lp_json, serde_json::to_string(&p.media)?, p.flair,
                p.thread_root_id, p.parent_post_id, i64::from(p.thread_index), i64::from(p.thread_total),
                p.quoted_post_id, p.reposted_post_id, p.crosspost_of_id,
                poll_json, poll_closes, poll_closed,
                serde_json::to_string(&p.tags)?, serde_json::to_string(&p.mentions)?,
                p.is_spoiler as i64, p.is_nsfw as i64, p.is_pinned as i64, p.is_locked as i64,
                p.is_mod_distinguished as i64, p.is_removed as i64,
                p.scheduled_at_ms, p.status.as_str(),
                p.upvotes, p.downvotes, p.score, p.likes_count as i64, p.comments_count as i64,
                p.reposts_count as i64, p.quotes_count as i64, p.bookmarks_count as i64,
                p.views_count as i64, p.awards.len() as i64, hot, wilson,
                p.created_at_ms, p.updated_at_ms,
            ],
        )
        .map_err(|e| db_err("seed post", e))?;

        for aw in &p.awards {
            tx.execute(
                "INSERT OR IGNORE INTO community_awards (award_id, target_kind, target_id, giver_actor_id, recipient_actor_id, award_kind, icon_emoji, message, created_at_ms)
                 VALUES (?1, 'post', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![aw.award_id, p.post_id, aw.giver_actor_id, p.author_actor_id, aw.award_kind, aw.icon_emoji, aw.message.as_deref().unwrap_or(""), aw.created_at_ms],
            )
            .map_err(|e| db_err("seed award", e))?;
        }
    }

    // 6. Poll votes
    for pv in &ds.poll_votes {
        tx.execute(
            "INSERT OR IGNORE INTO community_poll_votes (post_id, actor_id, option_id, actor_kind, voted_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pv.post_id, pv.actor_id, pv.option_id, pv.actor_kind.as_str(), pv.voted_at_ms],
        )
        .map_err(|e| db_err("seed poll vote", e))?;
    }

    // 7. Comments
    for c in &ds.comments {
        let wilson = wilson_lower_bound(c.upvotes, c.downvotes, 1.96);
        let root_id = c.path.split('/').next().unwrap_or(&c.comment_id);
        tx.execute(
            "INSERT OR IGNORE INTO community_comments (
                comment_id, post_id, parent_comment_id, root_comment_id, depth, path, author_actor_id,
                body_markdown, media_json, upvotes, downvotes, score, likes_count, wilson_score,
                is_stickied, is_distinguished_mod, is_removed, is_deleted, created_at_ms, updated_at_ms
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
            params![
                c.comment_id, c.post_id, c.parent_comment_id, root_id, i64::from(c.depth), c.path,
                c.author_actor_id, c.body, serde_json::to_string(&c.media)?,
                c.upvotes, c.downvotes, c.score, c.likes_count as i64, wilson,
                c.is_stickied as i64, c.is_mod_distinguished as i64, c.is_removed as i64, c.is_deleted as i64,
                c.created_at_ms, c.updated_at_ms,
            ],
        )
        .map_err(|e| db_err("seed comment", e))?;
    }

    // 8. Lists, Notifications, Mod Logs
    for list in &ds.lists {
        tx.execute(
            "INSERT OR IGNORE INTO community_lists (list_id, owner_actor_id, name, description, kind, is_private, is_pinned, members_json, communities_json, created_at_ms, updated_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                list.list_id, list.owner_actor_id, list.name, list.description, list.kind.as_str(),
                list.is_private as i64, list.is_pinned as i64,
                serde_json::to_string(&list.member_actor_ids)?,
                serde_json::to_string(&list.community_slugs)?,
                list.created_at_ms, list.updated_at_ms,
            ],
        )
        .map_err(|e| db_err("seed list", e))?;
    }
    for n in &ds.notifications {
        tx.execute(
            "INSERT OR IGNORE INTO community_notifications (notif_id, recipient_actor_id, actor_id, kind, target_post_id, target_comment_id, preview_text, is_read, created_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                n.notif_id, n.recipient_actor_id, n.actor_id, n.kind.as_str(),
                n.target_post_id, n.target_comment_id, n.preview_text, n.is_read as i64, n.created_at_ms,
            ],
        )
        .map_err(|e| db_err("seed notification", e))?;
    }
    for ml in &ds.mod_logs {
        tx.execute(
            "INSERT OR IGNORE INTO community_mod_logs (log_id, community_id, mod_actor_id, action, target_kind, target_id, reason, rule_triggered, created_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                ml.log_id, ml.community_id, ml.moderator_actor_id, ml.action.as_str(),
                ml.target_kind, ml.target_id, ml.reason.as_deref().unwrap_or(""), ml.rule_triggered, ml.created_at_ms,
            ],
        )
        .map_err(|e| db_err("seed mod_log", e))?;
    }

    Ok(true)
}
