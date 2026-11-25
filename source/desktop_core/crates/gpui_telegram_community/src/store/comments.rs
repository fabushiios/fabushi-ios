//! SQLite operations for Arbitrary-Depth Nested Comments, Votes, Likes, and Tree Hydration.

use super::actors::{bump_affinity_conn, get_actor_conn, insert_notification_conn};
use super::communities::{enqueue_mod_item_tx, get_community_conn, get_member_role_conn};
use super::posts::load_awards_conn;
use super::{CommunityStore, db_err};
use crate::model::{MAX_COMMENT_BODY_CHARS, extract_entities, validate_create_comment_request};
use crate::moderation::{AutoModEngine, ModPermissionValidator};
use crate::recommender::scoring::wilson_lower_bound;
use crate::thread::{CommentTreeBuilder, CommentTreeOptions, make_comment_path};
use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, AutoModActionKind, CommentSort, CommunityCommentDto, CommunityMemberRole,
    CommunityNotificationKind, CreateCommentRequest, CreateCommentResponse, PostMediaAttachmentDto,
    ToggleActionResponse, VerifiedTier, VoteResponse,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

fn hydrate_comment_conn(
    conn: &Connection,
    comment_id: &str,
    viewer_actor_id: Option<&str>,
) -> Result<Option<CommunityCommentDto>> {
    let raw = conn
        .query_row(
            "SELECT
                c.comment_id, c.post_id, c.parent_comment_id, c.depth, c.path,
                c.author_actor_id, a.handle, a.display_name, a.actor_kind, a.verified_tier,
                a.bot_badge, a.avatar_shape, a.avatar_color, p.author_actor_id,
                c.body_markdown, c.media_json, c.upvotes, c.downvotes, c.score, c.likes_count,
                c.is_stickied, c.is_distinguished_mod, c.is_removed, c.is_deleted,
                c.created_at_ms, c.updated_at_ms
             FROM community_comments c
             JOIN community_actors a ON a.actor_id = c.author_actor_id
             JOIN community_posts p ON p.post_id = c.post_id
             WHERE c.comment_id = ?1",
            params![comment_id],
            |r| {
                let kind_str: String = r.get(8)?;
                let tier_str: String = r.get(9)?;
                let author_id: String = r.get(5)?;
                let post_author_id: String = r.get(13)?;
                let media_json: String = r.get(15)?;
                let media: Vec<PostMediaAttachmentDto> =
                    serde_json::from_str(&media_json).unwrap_or_default();
                Ok(CommunityCommentDto {
                    comment_id: r.get(0)?,
                    post_id: r.get(1)?,
                    parent_comment_id: r.get(2)?,
                    depth: r.get::<_, i64>(3)?.max(0) as u32,
                    path: r.get(4)?,
                    author_actor_id: author_id.clone(),
                    author_handle: r.get(6)?,
                    author_display_name: r.get(7)?,
                    author_kind: ActorKind::parse(&kind_str).unwrap_or(ActorKind::Human),
                    author_verified_tier: VerifiedTier::parse(&tier_str).unwrap_or(VerifiedTier::None),
                    author_bot_badge: r.get(10)?,
                    author_avatar_shape: r.get(11)?,
                    author_avatar_color: r.get(12)?,
                    is_op: author_id == post_author_id,
                    body: r.get(14)?,
                    media,
                    upvotes: r.get(16)?,
                    downvotes: r.get(17)?,
                    score: r.get(18)?,
                    likes_count: r.get::<_, i64>(19)?.max(0) as u64,
                    awards: Vec::new(),
                    viewer_vote: 0,
                    viewer_liked: false,
                    is_stickied: r.get::<_, i64>(20)? != 0,
                    is_mod_distinguished: r.get::<_, i64>(21)? != 0,
                    is_removed: r.get::<_, i64>(22)? != 0,
                    is_deleted: r.get::<_, i64>(23)? != 0,
                    children: Vec::new(),
                    more_children_count: 0,
                    created_at_ms: r.get(24)?,
                    updated_at_ms: r.get(25)?,
                })
            },
        )
        .optional()
        .map_err(|e| db_err("hydrate comment", e))?;

    let Some(mut c) = raw else {
        return Ok(None);
    };
    c.awards = load_awards_conn(conn, "comment", &c.comment_id)?;
    if let Some(vid) = viewer_actor_id {
        if let Some(dir) = conn
            .query_row(
                "SELECT direction FROM community_votes WHERE target_kind = 'comment' AND target_id = ?1 AND actor_id = ?2",
                params![c.comment_id, vid],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| db_err("viewer comment vote", e))?
        {
            c.viewer_vote = dir.clamp(-1, 1) as i8;
        }
        let liked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM community_likes WHERE target_kind = 'comment' AND target_id = ?1 AND actor_id = ?2",
                params![c.comment_id, vid],
                |r| r.get(0),
            )
            .map_err(|e| db_err("viewer comment like", e))?;
        c.viewer_liked = liked > 0;
    }
    Ok(Some(c))
}

pub fn create_comment_tx(
    tx: &Transaction<'_>,
    author_actor_id: &str,
    post_id: &str,
    req: &CreateCommentRequest,
    now_ms: i64,
) -> Result<CreateCommentResponse> {
    validate_create_comment_request(req)?;
    let author = get_actor_conn(tx, author_actor_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("author '{author_actor_id}' not found")))?;

    let (post_author_id, community_id, is_locked, is_removed): (String, Option<String>, bool, bool) = tx
        .query_row(
            "SELECT author_actor_id, community_id, is_locked, is_removed FROM community_posts WHERE post_id = ?1",
            params![post_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get::<_, i64>(3)? != 0)),
        )
        .optional()
        .map_err(|e| db_err("lookup post for comment", e))?
        .ok_or_else(|| TelegramError::NotFound(format!("post '{post_id}' not found")))?;
    if is_locked || is_removed {
        return Err(TelegramError::PermissionDenied(
            "cannot comment on a locked or removed post".to_string(),
        ));
    }

    let mut is_mod_distinguished = false;
    let community = if let Some(cid) = community_id.as_deref() {
        let comm = get_community_conn(tx, cid, Some(author_actor_id))?;
        if let Some(ref c) = comm {
            if author.actor_kind.is_bot_like() && !c.allow_bots {
                return Err(TelegramError::PermissionDenied(format!(
                    "bots are disabled in c/{}",
                    c.slug
                )));
            }
            let role = get_member_role_conn(tx, &c.community_id, author_actor_id)?;
            if matches!(role, Some(CommunityMemberRole::Banned | CommunityMemberRole::Muted)) {
                return Err(TelegramError::PermissionDenied(format!(
                    "actor is banned or muted in c/{}",
                    c.slug
                )));
            }
            if req.is_mod_distinguished && ModPermissionValidator::can_moderate(role) {
                is_mod_distinguished = true;
            }
        }
        comm
    } else {
        None
    };

    let comment_id = format!("cmt_{}", Uuid::new_v4().simple());
    let (depth, path, root_comment_id, parent_author_id, bot_hop_count) =
        if let Some(pid) = req.parent_comment_id.as_deref().filter(|s| !s.trim().is_empty()) {
            let (p_post_id, p_depth, p_path, p_root, p_author, p_hops): (String, i64, String, Option<String>, String, i64) = tx
                .query_row(
                    "SELECT post_id, depth, path, root_comment_id, author_actor_id, bot_hop_count
                     FROM community_comments WHERE comment_id = ?1",
                    params![pid],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .optional()
                .map_err(|e| db_err("lookup parent comment", e))?
                .ok_or_else(|| TelegramError::NotFound(format!("parent comment '{pid}' not found")))?;
            if p_post_id != post_id {
                return Err(TelegramError::Validation(
                    "parent comment belongs to a different post".to_string(),
                ));
            }
            let next_hops = if author.actor_kind.is_bot_like() { p_hops + 1 } else { 0 };
            (
                p_depth + 1,
                make_comment_path(Some(&p_path), &comment_id),
                p_root.unwrap_or_else(|| pid.to_string()),
                Some(p_author),
                next_hops,
            )
        } else {
            (
                0,
                make_comment_path(None, &comment_id),
                comment_id.clone(),
                None,
                i64::from(author.actor_kind.is_bot_like()),
            )
        };

    let automod = if let Some(ref c) = community {
        AutoModEngine::evaluate_with_context(&c.automod_rules, &author, None, &req.body, now_ms, &[], Some(&c.slug))
    } else {
        Default::default()
    };
    let auto_removed = automod.action == AutoModActionKind::Remove;
    let entities = extract_entities(&req.body);

    tx.execute(
        "INSERT INTO community_comments (
            comment_id, post_id, parent_comment_id, root_comment_id, depth, path,
            author_actor_id, body_markdown, media_json, mentions_json,
            upvotes, downvotes, score, wilson_score, bot_hop_count,
            is_distinguished_mod, is_removed, removed_reason, created_at_ms, updated_at_ms
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,0,1,?11,?12,?13,?14,?15,?16,?16)",
        params![
            comment_id,
            post_id,
            req.parent_comment_id.as_deref().filter(|s| !s.trim().is_empty()),
            root_comment_id,
            depth,
            path,
            author_actor_id,
            req.body,
            serde_json::to_string(&req.media)?,
            serde_json::to_string(&entities.mentions)?,
            wilson_lower_bound(1, 0, 1.96),
            bot_hop_count,
            is_mod_distinguished as i64,
            auto_removed as i64,
            auto_removed.then(|| automod.reasons.join("; ")),
            now_ms,
        ],
    )
    .map_err(|e| db_err("insert comment", e))?;

    tx.execute(
        "INSERT OR IGNORE INTO community_votes (target_kind, target_id, actor_id, direction, updated_at_ms)
         VALUES ('comment', ?1, ?2, 1, ?3)",
        params![comment_id, author_actor_id, now_ms],
    )
    .map_err(|e| db_err("insert initial comment vote", e))?;

    if !auto_removed {
        tx.execute(
            "UPDATE community_posts SET comments_count = comments_count + 1, updated_at_ms = ?2 WHERE post_id = ?1",
            params![post_id, now_ms],
        )
        .map_err(|e| db_err("inc post comments_count", e))?;
        if let Some(pid) = req.parent_comment_id.as_deref().filter(|s| !s.trim().is_empty()) {
            tx.execute(
                "UPDATE community_comments SET replies_count = replies_count + 1 WHERE comment_id = ?1",
                params![pid],
            )
            .map_err(|e| db_err("inc parent replies_count", e))?;
        }

        let notif_kind = if author.actor_kind.is_bot_like() {
            CommunityNotificationKind::BotReply
        } else {
            CommunityNotificationKind::Reply
        };
        let reply_target = parent_author_id.as_deref().unwrap_or(&post_author_id);
        insert_notification_conn(
            tx,
            reply_target,
            author_actor_id,
            notif_kind,
            Some(post_id),
            Some(&comment_id),
            &format!("@{} replied: {}", author.handle, req.body.chars().take(100).collect::<String>()),
            now_ms,
        )?;
        for mention in &entities.mentions {
            if let Some(m) = get_actor_conn(tx, mention, None)? {
                insert_notification_conn(
                    tx,
                    &m.actor_id,
                    author_actor_id,
                    CommunityNotificationKind::Mention,
                    Some(post_id),
                    Some(&comment_id),
                    &format!("@{} mentioned you in a comment.", author.handle),
                    now_ms,
                )?;
            }
        }
        bump_affinity_conn(tx, author_actor_id, "author", &post_author_id, 2.5, now_ms)?;
    }

    if let Some(ref c) = community {
        if matches!(automod.action, AutoModActionKind::FlagForReview | AutoModActionKind::Remove) {
            enqueue_mod_item_tx(
                tx,
                &c.community_id,
                "comment",
                &comment_id,
                None,
                "automod",
                &automod.reasons.join("; "),
                automod.matched_rule_names.first().map(String::as_str),
                now_ms,
            )?;
        }
    }

    let comment = hydrate_comment_conn(tx, &comment_id, Some(author_actor_id))?
        .ok_or_else(|| TelegramError::Internal("created comment missing".to_string()))?;
    Ok(CreateCommentResponse {
        comment,
        automod_action: automod.action,
        automod_note: (!automod.reasons.is_empty()).then(|| automod.reasons.join("; ")),
    })
}

pub fn update_comment_tx(
    tx: &Transaction<'_>,
    comment_id: &str,
    actor_id: &str,
    body_markdown: &str,
    now_ms: i64,
) -> Result<CommunityCommentDto> {
    let existing = hydrate_comment_conn(tx, comment_id, Some(actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("comment '{comment_id}' not found")))?;
    if existing.author_actor_id != actor_id {
        return Err(TelegramError::PermissionDenied(
            "only the comment author may edit this comment".to_string(),
        ));
    }
    if existing.is_deleted || existing.is_removed {
        return Err(TelegramError::Validation(
            "cannot edit a deleted or removed comment".to_string(),
        ));
    }
    let trimmed = body_markdown.trim();
    if trimmed.is_empty() {
        return Err(TelegramError::Validation("comment body cannot be empty".to_string()));
    }
    if trimmed.chars().count() > MAX_COMMENT_BODY_CHARS {
        return Err(TelegramError::Validation(format!(
            "comment body exceeds {MAX_COMMENT_BODY_CHARS} characters"
        )));
    }
    tx.execute(
        "UPDATE community_comments SET body_markdown = ?2, edited_at_ms = ?3, updated_at_ms = ?3 WHERE comment_id = ?1",
        params![comment_id, trimmed, now_ms],
    )
    .map_err(|e| db_err("update comment", e))?;
    hydrate_comment_conn(tx, comment_id, Some(actor_id))?
        .ok_or_else(|| TelegramError::Internal("updated comment missing".to_string()))
}

pub fn delete_comment_tx(
    tx: &Transaction<'_>,
    comment_id: &str,
    actor_id: &str,
    now_ms: i64,
) -> Result<bool> {
    let existing = hydrate_comment_conn(tx, comment_id, Some(actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("comment '{comment_id}' not found")))?;
    let community_id: Option<String> = tx
        .query_row("SELECT community_id FROM community_posts WHERE post_id = ?1", params![existing.post_id], |r| r.get(0))
        .optional()
        .map_err(|e| db_err("lookup post comm for comment del", e))?
        .flatten();
    let is_mod = match community_id.as_deref() {
        Some(cid) => get_member_role_conn(tx, cid, actor_id)?
            .is_some_and(CommunityMemberRole::is_moderator),
        None => false,
    };
    if existing.author_actor_id != actor_id && !is_mod {
        return Err(TelegramError::PermissionDenied(
            "only the author or a moderator may delete this comment".to_string(),
        ));
    }

    let child_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM community_comments WHERE parent_comment_id = ?1",
            params![comment_id],
            |r| r.get(0),
        )
        .map_err(|e| db_err("count comment children", e))?;

    if child_count > 0 {
        // Tombstone in place so nested children remain connected cleanly!
        tx.execute(
            "UPDATE community_comments SET body_markdown = '[deleted]', is_deleted = 1, is_removed = 1, updated_at_ms = ?2 WHERE comment_id = ?1",
            params![comment_id, now_ms],
        )
        .map_err(|e| db_err("tombstone comment", e))?;
    } else {
        tx.execute("DELETE FROM community_comments WHERE comment_id = ?1", params![comment_id])
            .map_err(|e| db_err("delete leaf comment", e))?;
    }

    if !existing.is_deleted && !existing.is_removed {
        tx.execute(
            "UPDATE community_posts SET comments_count = MAX(0, comments_count - 1) WHERE post_id = ?1",
            params![existing.post_id],
        )
        .map_err(|e| db_err("dec post comments_count", e))?;
    }
    Ok(true)
}

pub fn vote_comment_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    comment_id: &str,
    direction: i8,
    now_ms: i64,
) -> Result<VoteResponse> {
    if !matches!(direction, -1 | 0 | 1) {
        return Err(TelegramError::Validation("vote direction must be -1, 0, or 1".to_string()));
    }
    let (author_id, mut up, mut down): (String, i64, i64) = tx
        .query_row(
            "SELECT author_actor_id, upvotes, downvotes FROM community_comments WHERE comment_id = ?1",
            params![comment_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| db_err("lookup comment for vote", e))?
        .ok_or_else(|| TelegramError::NotFound(format!("comment '{comment_id}' not found")))?;

    let prev_dir: i8 = tx
        .query_row(
            "SELECT direction FROM community_votes WHERE target_kind = 'comment' AND target_id = ?1 AND actor_id = ?2",
            params![comment_id, actor_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| db_err("lookup prior comment vote", e))?
        .unwrap_or(0) as i8;

    let d_up = i64::from(direction == 1) - i64::from(prev_dir == 1);
    let d_down = i64::from(direction == -1) - i64::from(prev_dir == -1);
    let d_score = i64::from(direction) - i64::from(prev_dir);

    if direction == 0 {
        tx.execute(
            "DELETE FROM community_votes WHERE target_kind = 'comment' AND target_id = ?1 AND actor_id = ?2",
            params![comment_id, actor_id],
        )
        .map_err(|e| db_err("delete comment vote", e))?;
    } else {
        tx.execute(
            "INSERT INTO community_votes (target_kind, target_id, actor_id, direction, updated_at_ms)
             VALUES ('comment', ?1, ?2, ?3, ?4)
             ON CONFLICT(target_kind, target_id, actor_id) DO UPDATE SET direction = excluded.direction, updated_at_ms = excluded.updated_at_ms",
            params![comment_id, actor_id, i64::from(direction), now_ms],
        )
        .map_err(|e| db_err("upsert comment vote", e))?;
    }

    up = (up + d_up).max(0);
    down = (down + d_down).max(0);
    let score = up - down;
    let wilson = wilson_lower_bound(up, down, 1.96);

    tx.execute(
        "UPDATE community_comments SET upvotes=?2, downvotes=?3, score=?4, wilson_score=?5, updated_at_ms=?6 WHERE comment_id=?1",
        params![comment_id, up, down, score, wilson, now_ms],
    )
    .map_err(|e| db_err("update comment vote counts", e))?;

    if author_id != actor_id && d_score != 0 {
        tx.execute(
            "UPDATE community_actors SET comment_karma = comment_karma + ?2, updated_at_ms = ?3 WHERE actor_id = ?1",
            params![author_id, d_score, now_ms],
        )
        .map_err(|e| db_err("update comment_karma", e))?;
    }

    Ok(VoteResponse {
        target_id: comment_id.to_string(),
        direction,
        upvotes: up,
        downvotes: down,
        score,
    })
}

pub fn toggle_like_comment_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    comment_id: &str,
    now_ms: i64,
) -> Result<ToggleActionResponse> {
    if hydrate_comment_conn(tx, comment_id, None)?.is_none() {
        return Err(TelegramError::NotFound(format!("comment '{comment_id}' not found")));
    }
    let deleted = tx
        .execute(
            "DELETE FROM community_likes WHERE target_kind = 'comment' AND target_id = ?1 AND actor_id = ?2",
            params![comment_id, actor_id],
        )
        .map_err(|e| db_err("delete comment like", e))?;
    let active = if deleted == 1 {
        tx.execute("UPDATE community_comments SET likes_count = MAX(0, likes_count - 1) WHERE comment_id = ?1", params![comment_id])
            .map_err(|e| db_err("dec comment likes", e))?;
        false
    } else {
        tx.execute(
            "INSERT INTO community_likes (target_kind, target_id, actor_id, created_at_ms) VALUES ('comment', ?1, ?2, ?3)",
            params![comment_id, actor_id, now_ms],
        )
        .map_err(|e| db_err("insert comment like", e))?;
        tx.execute("UPDATE community_comments SET likes_count = likes_count + 1 WHERE comment_id = ?1", params![comment_id])
            .map_err(|e| db_err("inc comment likes", e))?;
        true
    };
    let count: i64 = tx
        .query_row("SELECT likes_count FROM community_comments WHERE comment_id = ?1", params![comment_id], |r| r.get(0))
        .map_err(|e| db_err("read comment likes_count", e))?;
    Ok(ToggleActionResponse {
        target_id: comment_id.to_string(),
        active,
        count: count.max(0) as u64,
    })
}

pub fn list_comment_tree_conn(
    conn: &Connection,
    post_id: &str,
    viewer_actor_id: Option<&str>,
    sort: CommentSort,
    max_depth: Option<u32>,
    limit: usize,
) -> Result<Vec<CommunityCommentDto>> {
    let mut stmt = conn
        .prepare("SELECT comment_id FROM community_comments WHERE post_id = ?1 ORDER BY created_at_ms ASC LIMIT ?2")
        .map_err(|e| db_err("prepare comment ids", e))?;
    let rows = stmt
        .query_map(params![post_id, limit.clamp(1, 2000) as i64], |r| r.get::<_, String>(0))
        .map_err(|e| db_err("query comment ids", e))?;
    let mut flat = Vec::new();
    for r in rows {
        let cid = r.map_err(|e| db_err("read comment id", e))?;
        if let Some(c) = hydrate_comment_conn(conn, &cid, viewer_actor_id)? {
            flat.push(c);
        }
    }
    let opts = CommentTreeOptions {
        sort,
        max_depth,
        max_children_per_node: None,
        collapsed_ids: Default::default(),
    };
    Ok(CommentTreeBuilder::build_with_options(flat, &opts))
}

impl CommunityStore {
    pub fn create_comment(&self, author_actor_id: &str, post_id: &str, req: &CreateCommentRequest, now_ms: i64) -> Result<CreateCommentResponse> {
        self.with_transaction(|tx| create_comment_tx(tx, author_actor_id, post_id, req, now_ms))
    }
    pub fn update_comment(&self, comment_id: &str, actor_id: &str, body_markdown: &str, now_ms: i64) -> Result<CommunityCommentDto> {
        self.with_transaction(|tx| update_comment_tx(tx, comment_id, actor_id, body_markdown, now_ms))
    }
    pub fn delete_comment(&self, comment_id: &str, actor_id: &str, now_ms: i64) -> Result<bool> {
        self.with_transaction(|tx| delete_comment_tx(tx, comment_id, actor_id, now_ms))
    }
    pub fn vote_comment(&self, actor_id: &str, comment_id: &str, direction: i8, now_ms: i64) -> Result<VoteResponse> {
        self.with_transaction(|tx| vote_comment_tx(tx, actor_id, comment_id, direction, now_ms))
    }
    pub fn toggle_like_comment(&self, actor_id: &str, comment_id: &str, now_ms: i64) -> Result<ToggleActionResponse> {
        self.with_transaction(|tx| toggle_like_comment_tx(tx, actor_id, comment_id, now_ms))
    }
    pub fn list_comment_tree(&self, post_id: &str, viewer_actor_id: Option<&str>, sort: CommentSort, max_depth: Option<u32>, limit: usize) -> Result<Vec<CommunityCommentDto>> {
        self.with_conn(|conn| list_comment_tree_conn(conn, post_id, viewer_actor_id, sort, max_depth, limit))
    }
}
