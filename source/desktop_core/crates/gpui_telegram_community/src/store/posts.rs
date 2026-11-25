//! SQLite operations for Posts, Multi-Post Threads, Edit History, Votes, Likes, Reposts, Bookmarks, Polls, Awards, and Schedulers.

use super::actors::{bump_affinity_conn, get_actor_conn, insert_notification_conn};
use super::communities::{enqueue_mod_item_tx, get_community_conn, get_member_role_conn};
use super::{CommunityStore, db_err};
use crate::model::{apply_poll_vote, extract_entities, validate_create_post_request};
use crate::moderation::AutoModEngine;
use crate::recommender::scoring::{reddit_hot, wilson_lower_bound};
use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, AutoModActionKind, BookmarkResponse, CommunityMemberRole,
    CommunityNotificationKind, CommunityPollOptionDto, CommunityPostDto, CreatePostRequest,
    CreatePostResponse, EmbeddedPostSummaryDto, GiveAwardRequest, LinkPreviewDto, PollDto,
    PostAwardDto, PostEditRevisionDto, PostFormat, PostMediaAttachmentDto, PostStatus,
    RepostResponse, ToggleActionResponse, UpdatePostRequest, VerifiedTier, VoteResponse,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashMap;
use uuid::Uuid;

pub(crate) fn load_poll_for_post_conn(
    conn: &Connection,
    post_id: &str,
    poll_json: Option<&str>,
    poll_is_closed: bool,
    viewer_actor_id: Option<&str>,
) -> Result<Option<PollDto>> {
    let Some(raw) = poll_json.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let mut poll: PollDto = serde_json::from_str(raw)?;
    poll.is_closed = poll.is_closed || poll_is_closed;

    // Reset counts and aggregate live from community_poll_votes for exact Human vs Bot accuracy.
    for opt in &mut poll.options {
        opt.all_votes = 0;
        opt.human_votes = 0;
        opt.bot_votes = 0;
    }
    let mut stmt = conn
        .prepare(
            "SELECT option_id, actor_kind, COUNT(*) FROM community_poll_votes
             WHERE post_id = ?1 GROUP BY option_id, actor_kind",
        )
        .map_err(|e| db_err("prepare poll tally", e))?;
    let rows = stmt
        .query_map(params![post_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?.max(0) as u64,
            ))
        })
        .map_err(|e| db_err("query poll tally", e))?;
    for r in rows {
        let (opt_id, kind_str, cnt) = r.map_err(|e| db_err("read poll tally", e))?;
        if let Some(opt) = poll.options.iter_mut().find(|o| o.option_id == opt_id) {
            opt.all_votes = opt.all_votes.saturating_add(cnt);
            if kind_str == "human" {
                opt.human_votes = opt.human_votes.saturating_add(cnt);
            } else {
                opt.bot_votes = opt.bot_votes.saturating_add(cnt);
            }
        }
    }
    poll.total_votes = poll
        .options
        .iter()
        .fold(0u64, |acc, o| acc.saturating_add(o.all_votes));
    poll.total_human_votes = poll
        .options
        .iter()
        .fold(0u64, |acc, o| acc.saturating_add(o.human_votes));
    poll.total_bot_votes = poll
        .options
        .iter()
        .fold(0u64, |acc, o| acc.saturating_add(o.bot_votes));

    poll.viewer_voted_option_ids.clear();
    if let Some(vid) = viewer_actor_id {
        let mut vstmt = conn
            .prepare("SELECT option_id FROM community_poll_votes WHERE post_id = ?1 AND actor_id = ?2")
            .map_err(|e| db_err("prepare viewer poll votes", e))?;
        let vrows = vstmt
            .query_map(params![post_id, vid], |r| r.get::<_, String>(0))
            .map_err(|e| db_err("query viewer poll votes", e))?;
        for vr in vrows {
            poll.viewer_voted_option_ids.push(vr.map_err(|e| db_err("read viewer poll vote", e))?);
        }
    }
    Ok(Some(poll))
}

pub(crate) fn load_awards_conn(
    conn: &Connection,
    target_kind: &str,
    target_id: &str,
) -> Result<Vec<PostAwardDto>> {
    let mut stmt = conn
        .prepare(
            "SELECT aw.award_id, aw.award_kind, aw.icon_emoji, aw.giver_actor_id, a.handle, aw.message, aw.created_at_ms
             FROM community_awards aw
             JOIN community_actors a ON a.actor_id = aw.giver_actor_id
             WHERE aw.target_kind = ?1 AND aw.target_id = ?2
             ORDER BY aw.created_at_ms ASC",
        )
        .map_err(|e| db_err("prepare awards", e))?;
    let rows = stmt
        .query_map(params![target_kind, target_id], |r| {
            let msg: String = r.get(5)?;
            Ok(PostAwardDto {
                award_id: r.get(0)?,
                award_kind: r.get(1)?,
                icon_emoji: r.get(2)?,
                giver_actor_id: r.get(3)?,
                giver_handle: r.get(4)?,
                message: (!msg.is_empty()).then_some(msg),
                created_at_ms: r.get(6)?,
            })
        })
        .map_err(|e| db_err("query awards", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| db_err("read award", e))?);
    }
    Ok(out)
}

fn load_embedded_summary_conn(conn: &Connection, post_id: &str) -> Result<Option<EmbeddedPostSummaryDto>> {
    conn.query_row(
        "SELECT p.post_id, c.slug, p.author_actor_id, a.handle, a.display_name, a.actor_kind,
                a.avatar_shape, a.avatar_color, p.title, p.body_markdown, p.post_format,
                p.score, p.comments_count, p.created_at_ms
         FROM community_posts p
         JOIN community_actors a ON a.actor_id = p.author_actor_id
         LEFT JOIN communities c ON c.community_id = p.community_id
         WHERE p.post_id = ?1",
        params![post_id],
        |r| {
            let kind_str: String = r.get(5)?;
            let body: String = r.get(9)?;
            let fmt_str: String = r.get(10)?;
            Ok(EmbeddedPostSummaryDto {
                post_id: r.get(0)?,
                community_slug: r.get(1)?,
                author_actor_id: r.get(2)?,
                author_handle: r.get(3)?,
                author_display_name: r.get(4)?,
                author_kind: ActorKind::parse(&kind_str).unwrap_or(ActorKind::Human),
                author_avatar_shape: r.get(6)?,
                author_avatar_color: r.get(7)?,
                title: r.get(8)?,
                body_preview: body.chars().take(200).collect(),
                format: PostFormat::parse(&fmt_str).unwrap_or(PostFormat::Micro),
                score: r.get(11)?,
                comments_count: r.get::<_, i64>(12)?.max(0) as u64,
                created_at_ms: r.get(13)?,
            })
        },
    )
    .optional()
    .map_err(|e| db_err("load embedded post summary", e))
}

pub fn list_post_edits_conn(conn: &Connection, post_id: &str) -> Result<Vec<PostEditRevisionDto>> {
    let mut stmt = conn
        .prepare(
            "SELECT revision, previous_title, previous_body_markdown, edited_at_ms
             FROM community_post_edits WHERE post_id = ?1 ORDER BY revision ASC",
        )
        .map_err(|e| db_err("prepare post edits", e))?;
    let rows = stmt
        .query_map(params![post_id], |r| {
            Ok(PostEditRevisionDto {
                revision: r.get::<_, i64>(0)?.max(1) as u32,
                title: r.get(1)?,
                body: r.get(2)?,
                edited_at_ms: r.get(3)?,
            })
        })
        .map_err(|e| db_err("query post edits", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| db_err("read post edit", e))?);
    }
    Ok(out)
}

pub(crate) fn hydrate_post_row_conn(
    conn: &Connection,
    post_id: &str,
    viewer_actor_id: Option<&str>,
) -> Result<Option<CommunityPostDto>> {
    let raw = conn
        .query_row(
            "SELECT
                p.post_id, p.community_id, c.slug, p.author_actor_id,
                a.handle, a.display_name, a.actor_kind, a.verified_tier, a.bot_badge,
                a.avatar_media_id, a.avatar_shape, a.avatar_color,
                p.co_author_actor_id, p.post_format, p.status, p.title, p.body_markdown, p.flair_text,
                p.hashtags_json, p.mentions_json, p.media_json, p.link_preview_json, p.poll_json,
                p.poll_is_closed, p.quote_post_id, p.repost_of_post_id, p.crosspost_of_post_id,
                p.thread_root_id, p.parent_post_id, p.thread_index, p.thread_total,
                p.is_nsfw, p.is_spoiler, p.is_pinned, p.is_locked, p.is_removed, p.is_distinguished_mod,
                p.upvotes, p.downvotes, p.score, p.likes_count, p.reposts_count, p.quotes_count,
                p.comments_count, p.bookmarks_count, p.views_count, p.scheduled_at_ms,
                p.created_at_ms, p.updated_at_ms
             FROM community_posts p
             JOIN community_actors a ON a.actor_id = p.author_actor_id
             LEFT JOIN communities c ON c.community_id = p.community_id
             WHERE p.post_id = ?1",
            params![post_id],
            |r| {
                let kind_str: String = r.get(6)?;
                let tier_str: String = r.get(7)?;
                let fmt_str: String = r.get(13)?;
                let st_str: String = r.get(14)?;
                let tags_json: String = r.get(18)?;
                let mentions_json: String = r.get(19)?;
                let media_json: String = r.get(20)?;
                let lp_json: Option<String> = r.get(21)?;
                let poll_json: Option<String> = r.get(22)?;
                let poll_closed: bool = r.get::<_, i64>(23)? != 0;
                let link_preview: Option<LinkPreviewDto> = lp_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok());
                let media: Vec<PostMediaAttachmentDto> =
                    serde_json::from_str(&media_json).unwrap_or_default();
                let post = CommunityPostDto {
                    post_id: r.get(0)?,
                    community_id: r.get(1)?,
                    community_slug: r.get(2)?,
                    author_actor_id: r.get(3)?,
                    author_handle: r.get(4)?,
                    author_display_name: r.get(5)?,
                    author_kind: ActorKind::parse(&kind_str).unwrap_or(ActorKind::Human),
                    author_verified_tier: VerifiedTier::parse(&tier_str).unwrap_or(VerifiedTier::None),
                    author_bot_badge: r.get(8)?,
                    author_avatar_media_id: r.get(9)?,
                    author_avatar_shape: r.get(10)?,
                    author_avatar_color: r.get(11)?,
                    co_author_actor_id: r.get(12)?,
                    format: PostFormat::parse(&fmt_str).unwrap_or(PostFormat::Micro),
                    status: PostStatus::parse(&st_str).unwrap_or(PostStatus::Published),
                    title: r.get(15)?,
                    body: r.get(16)?,
                    flair: r.get(17)?,
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    mentions: serde_json::from_str(&mentions_json).unwrap_or_default(),
                    media,
                    link_preview,
                    poll: None,
                    quoted_post_id: r.get(24)?,
                    quoted_post: None,
                    reposted_post_id: r.get(25)?,
                    reposted_post: None,
                    crosspost_of_id: r.get(26)?,
                    crosspost_origin: None,
                    thread_root_id: r.get(27)?,
                    parent_post_id: r.get(28)?,
                    thread_index: r.get::<_, i64>(29)?.max(0) as u32,
                    thread_total: r.get::<_, i64>(30)?.max(1) as u32,
                    is_nsfw: r.get::<_, i64>(31)? != 0,
                    is_spoiler: r.get::<_, i64>(32)? != 0,
                    is_pinned: r.get::<_, i64>(33)? != 0,
                    is_locked: r.get::<_, i64>(34)? != 0,
                    is_removed: r.get::<_, i64>(35)? != 0,
                    is_mod_distinguished: r.get::<_, i64>(36)? != 0,
                    upvotes: r.get(37)?,
                    downvotes: r.get(38)?,
                    score: r.get(39)?,
                    likes_count: r.get::<_, i64>(40)?.max(0) as u64,
                    reposts_count: r.get::<_, i64>(41)?.max(0) as u64,
                    quotes_count: r.get::<_, i64>(42)?.max(0) as u64,
                    comments_count: r.get::<_, i64>(43)?.max(0) as u64,
                    bookmarks_count: r.get::<_, i64>(44)?.max(0) as u64,
                    views_count: r.get::<_, i64>(45)?.max(0) as u64,
                    awards: Vec::new(),
                    viewer_vote: 0,
                    viewer_liked: false,
                    viewer_reposted: false,
                    viewer_bookmarked: false,
                    viewer_bookmark_folder: None,
                    edit_history: Vec::new(),
                    scheduled_at_ms: r.get(46)?,
                    created_at_ms: r.get(47)?,
                    updated_at_ms: r.get(48)?,
                };
                Ok((post, poll_json, poll_closed))
            },
        )
        .optional()
        .map_err(|e| db_err("query post", e))?;

    let Some((mut post, poll_json, poll_closed)) = raw else {
        return Ok(None);
    };

    post.poll = load_poll_for_post_conn(conn, &post.post_id, poll_json.as_deref(), poll_closed, viewer_actor_id)?;
    post.awards = load_awards_conn(conn, "post", &post.post_id)?;
    post.edit_history = list_post_edits_conn(conn, &post.post_id)?;

    if let Some(qid) = post.quoted_post_id.as_deref() {
        post.quoted_post = load_embedded_summary_conn(conn, qid)?.map(Box::new);
    }
    if let Some(rid) = post.reposted_post_id.as_deref() {
        post.reposted_post = load_embedded_summary_conn(conn, rid)?.map(Box::new);
    }
    if let Some(xid) = post.crosspost_of_id.as_deref() {
        post.crosspost_origin = load_embedded_summary_conn(conn, xid)?.map(Box::new);
    }

    if let Some(vid) = viewer_actor_id {
        if let Some(dir) = conn
            .query_row(
                "SELECT direction FROM community_votes WHERE target_kind = 'post' AND target_id = ?1 AND actor_id = ?2",
                params![post.post_id, vid],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| db_err("viewer post vote", e))?
        {
            post.viewer_vote = dir.clamp(-1, 1) as i8;
        }
        let liked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM community_likes WHERE target_kind = 'post' AND target_id = ?1 AND actor_id = ?2",
                params![post.post_id, vid],
                |r| r.get(0),
            )
            .map_err(|e| db_err("viewer post like", e))?;
        post.viewer_liked = liked > 0;
        let reposted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM community_reposts WHERE original_post_id = ?1 AND actor_id = ?2",
                params![post.post_id, vid],
                |r| r.get(0),
            )
            .map_err(|e| db_err("viewer post repost", e))?;
        post.viewer_reposted = reposted > 0;
        let folder: Option<String> = conn
            .query_row(
                "SELECT folder_name FROM community_bookmarks WHERE post_id = ?1 AND actor_id = ?2",
                params![post.post_id, vid],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| db_err("viewer post bookmark", e))?;
        if let Some(f) = folder {
            post.viewer_bookmarked = true;
            post.viewer_bookmark_folder = Some(f);
        }
    }

    Ok(Some(post))
}

pub fn get_post_conn(
    conn: &Connection,
    post_id: &str,
    viewer_actor_id: Option<&str>,
) -> Result<Option<CommunityPostDto>> {
    hydrate_post_row_conn(conn, post_id, viewer_actor_id)
}

pub fn create_post_tx(
    tx: &Transaction<'_>,
    author_actor_id: &str,
    req: &CreatePostRequest,
    now_ms: i64,
) -> Result<CreatePostResponse> {
    validate_create_post_request(req, now_ms)?;
    let author = get_actor_conn(tx, author_actor_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("author actor '{author_actor_id}' not found")))?;

    let community = if let Some(slug) = req.community_slug.as_deref().filter(|s| !s.trim().is_empty()) {
        let comm = get_community_conn(tx, slug, Some(author_actor_id))?
            .ok_or_else(|| TelegramError::NotFound(format!("community '{slug}' not found")))?;
        if author.actor_kind.is_bot_like() && !comm.allow_bots {
            return Err(TelegramError::PermissionDenied(format!(
                "community c/{} does not allow bot posts",
                comm.slug
            )));
        }
        if matches!(
            get_member_role_conn(tx, &comm.community_id, author_actor_id)?,
            Some(CommunityMemberRole::Banned | CommunityMemberRole::Muted)
        ) {
            return Err(TelegramError::PermissionDenied(format!(
                "actor '{}' is muted or banned in c/{}",
                author.handle, comm.slug
            )));
        }
        Some(comm)
    } else {
        None
    };

    // Fetch recent bodies for duplicate spam guard
    let recent_bodies: Vec<String> = {
        let mut stmt = tx
            .prepare("SELECT body_markdown FROM community_posts WHERE author_actor_id = ?1 ORDER BY created_at_ms DESC LIMIT 5")
            .map_err(|e| db_err("prepare recent bodies", e))?;
        let rows = stmt
            .query_map(params![author_actor_id], |r| r.get::<_, String>(0))
            .map_err(|e| db_err("query recent bodies", e))?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r.map_err(|e| db_err("read recent body", e))?);
        }
        v
    };
    let recent_refs: Vec<&str> = recent_bodies.iter().map(String::as_str).collect();

    let automod_decision = if let Some(ref comm) = community {
        let mut combined_automod_body = req.body.clone();
        for part in &req.thread_parts {
            if !part.body.trim().is_empty() {
                if !combined_automod_body.is_empty() {
                    combined_automod_body.push_str("\n\n");
                }
                combined_automod_body.push_str(&part.body);
            }
        }
        AutoModEngine::evaluate_with_context(
            &comm.automod_rules,
            &author,
            req.title.as_deref(),
            &combined_automod_body,
            now_ms,
            &recent_refs,
            Some(&comm.slug),
        )
    } else {
        Default::default()
    };

    let root_post_id = format!("post_{}", Uuid::new_v4().simple());
    let thread_total = (1 + req.thread_parts.len()) as u32;
    let thread_root_id = (thread_total > 1).then(|| root_post_id.clone());

    let effective_flair = req
        .flair
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| automod_decision.assigned_flair.clone());
    let is_removed = automod_decision.action == AutoModActionKind::Remove;
    let removed_reason = is_removed.then(|| automod_decision.reasons.join("; "));

    let poll_dto = req.poll.as_ref().map(|pi| {
        let dur = if pi.duration_secs == 0 { 86_400 } else { pi.duration_secs };
        PollDto {
            poll_id: format!("poll_{}", Uuid::new_v4().simple()),
            question: pi.question.trim().to_string(),
            options: pi
                .options
                .iter()
                .enumerate()
                .map(|(i, text)| CommunityPollOptionDto {
                    option_id: format!("opt_{i}"),
                    text: text.trim().to_string(),
                    all_votes: 0,
                    human_votes: 0,
                    bot_votes: 0,
                })
                .collect(),
            multi_select: pi.multi_select,
            expires_at_ms: now_ms.saturating_add(dur.saturating_mul(1000)),
            is_closed: false,
            total_votes: 0,
            total_human_votes: 0,
            total_bot_votes: 0,
            viewer_voted_option_ids: Vec::new(),
        }
    });

    let combined = format!("{} {}", req.title.as_deref().unwrap_or(""), req.body);
    let entities = extract_entities(&combined);
    let mut tags: Vec<String> = req
        .tags
        .iter()
        .map(|t| t.trim().trim_start_matches('#').to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    for htag in entities.hashtags {
        if !tags.contains(&htag) {
            tags.push(htag);
        }
    }

    let hot = reddit_hot(1, 0, now_ms);
    let wilson = wilson_lower_bound(1, 0, 1.96);
    let link_url = req
        .link_preview
        .as_ref()
        .map(|lp| lp.url.clone())
        .or_else(|| entities.urls.first().cloned());

    tx.execute(
        "INSERT INTO community_posts (
            post_id, community_id, author_actor_id, co_author_actor_id, post_format,
            title, body_markdown, link_url, link_preview_json, media_json, flair_text,
            thread_root_id, parent_post_id, thread_index, thread_total,
            quote_post_id, repost_of_post_id, crosspost_of_post_id,
            poll_json, poll_closes_at_ms, poll_is_closed,
            hashtags_json, mentions_json,
            is_spoiler, is_nsfw, is_removed, removed_reason,
            scheduled_at_ms, status,
            upvotes, downvotes, score, hot_score, wilson_score,
            created_at_ms, updated_at_ms
         ) VALUES (
            ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,0,?13,?14,?15,?16,
            ?17,?18,0,?19,?20,?21,?22,?23,?24,?25,?26,1,0,1,?27,?28,?29,?29
         )",
        params![
            root_post_id,
            community.as_ref().map(|c| c.community_id.as_str()),
            author_actor_id,
            req.co_author_actor_id,
            req.format.as_str(),
            req.title.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            req.body,
            link_url,
            req.link_preview.as_ref().map(serde_json::to_string).transpose()?,
            serde_json::to_string(&req.media)?,
            effective_flair,
            thread_root_id,
            i64::from(thread_total),
            req.quoted_post_id,
            req.reposted_post_id,
            req.crosspost_of_id,
            poll_dto.as_ref().map(serde_json::to_string).transpose()?,
            poll_dto.as_ref().map(|p| p.expires_at_ms),
            serde_json::to_string(&tags)?,
            serde_json::to_string(&entities.mentions)?,
            req.is_spoiler as i64,
            req.is_nsfw as i64,
            is_removed as i64,
            removed_reason,
            req.scheduled_at_ms,
            req.status.as_str(),
            hot,
            wilson,
            now_ms,
        ],
    )
    .map_err(|e| db_err("insert post", e))?;

    // Author initial self-upvote (does not grant self-karma)
    tx.execute(
        "INSERT OR IGNORE INTO community_votes (target_kind, target_id, actor_id, direction, updated_at_ms)
         VALUES ('post', ?1, ?2, 1, ?3)",
        params![root_post_id, author_actor_id, now_ms],
    )
    .map_err(|e| db_err("insert initial post vote", e))?;

    if req.status == PostStatus::Published && !is_removed {
        tx.execute(
            "UPDATE community_actors SET posts_count = posts_count + 1, updated_at_ms = ?2 WHERE actor_id = ?1",
            params![author_actor_id, now_ms],
        )
        .map_err(|e| db_err("inc actor posts_count", e))?;
        if let Some(ref comm) = community {
            tx.execute(
                "UPDATE communities SET posts_count = posts_count + 1, updated_at_ms = ?2 WHERE community_id = ?1",
                params![comm.community_id, now_ms],
            )
            .map_err(|e| db_err("inc community posts_count", e))?;
        }
        if let Some(qid) = req.quoted_post_id.as_deref() {
            tx.execute(
                "UPDATE community_posts SET quotes_count = quotes_count + 1, updated_at_ms = ?2 WHERE post_id = ?1",
                params![qid, now_ms],
            )
            .map_err(|e| db_err("inc quotes_count", e))?;
            if let Some(orig_author) = tx
                .query_row("SELECT author_actor_id FROM community_posts WHERE post_id = ?1", params![qid], |r| r.get::<_, String>(0))
                .optional()
                .map_err(|e| db_err("lookup quoted author", e))?
            {
                insert_notification_conn(
                    tx,
                    &orig_author,
                    author_actor_id,
                    CommunityNotificationKind::Quote,
                    Some(&root_post_id),
                    None,
                    &format!("@{} quoted your post.", author.handle),
                    now_ms,
                )?;
            }
        }
        for mention in &entities.mentions {
            if let Some(mentioned) = get_actor_conn(tx, mention, None)? {
                insert_notification_conn(
                    tx,
                    &mentioned.actor_id,
                    author_actor_id,
                    CommunityNotificationKind::Mention,
                    Some(&root_post_id),
                    None,
                    &format!("@{} mentioned you in a post.", author.handle),
                    now_ms,
                )?;
            }
        }
    }

    if let Some(ref comm) = community {
        if matches!(
            automod_decision.action,
            AutoModActionKind::FlagForReview | AutoModActionKind::Remove
        ) {
            enqueue_mod_item_tx(
                tx,
                &comm.community_id,
                "post",
                &root_post_id,
                None,
                "automod",
                &automod_decision.reasons.join("; "),
                automod_decision.matched_rule_names.first().map(String::as_str),
                now_ms,
            )?;
        }
    }

    // Create additional X thread parts (2/N .. N/N)
    let mut thread_posts = Vec::with_capacity(req.thread_parts.len());
    let mut prev_id = root_post_id.clone();
    for (idx, part) in req.thread_parts.iter().enumerate() {
        let part_id = format!("post_{}", Uuid::new_v4().simple());
        let part_ts = now_ms.saturating_add((idx + 1) as i64);
        let p_ent = extract_entities(&part.body);
        tx.execute(
            "INSERT INTO community_posts (
                post_id, community_id, author_actor_id, post_format, body_markdown,
                link_preview_json, media_json, thread_root_id, parent_post_id,
                thread_index, thread_total, hashtags_json, mentions_json,
                is_spoiler, is_nsfw, is_removed, removed_reason, scheduled_at_ms,
                status, upvotes, downvotes, score, hot_score, wilson_score,
                created_at_ms, updated_at_ms
             ) VALUES (?1,?2,?3,'micro',?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,1,0,1,?19,?20,?21,?21)",
            params![
                part_id,
                community.as_ref().map(|c| c.community_id.as_str()),
                author_actor_id,
                part.body,
                part.link_preview.as_ref().map(serde_json::to_string).transpose()?,
                serde_json::to_string(&part.media)?,
                root_post_id,
                prev_id,
                (idx + 1) as i64,
                i64::from(thread_total),
                serde_json::to_string(&p_ent.hashtags)?,
                serde_json::to_string(&p_ent.mentions)?,
                req.is_spoiler as i64,
                req.is_nsfw as i64,
                is_removed as i64,
                removed_reason,
                req.scheduled_at_ms,
                req.status.as_str(),
                reddit_hot(1, 0, part_ts),
                wilson_lower_bound(1, 0, 1.96),
                part_ts,
            ],
        )
        .map_err(|e| db_err("insert thread part", e))?;
        // Author initial self-upvote for each thread part (prevents self double-upvoting 1 -> 2)
        tx.execute(
            "INSERT OR IGNORE INTO community_votes (target_kind, target_id, actor_id, direction, updated_at_ms)
             VALUES ('post', ?1, ?2, 1, ?3)",
            params![part_id, author_actor_id, part_ts],
        )
        .map_err(|e| db_err("insert initial thread part vote", e))?;
        if let Some(tp) = hydrate_post_row_conn(tx, &part_id, Some(author_actor_id))? {
            thread_posts.push(tp);
        }
        prev_id = part_id;
    }

    let post = hydrate_post_row_conn(tx, &root_post_id, Some(author_actor_id))?
        .ok_or_else(|| TelegramError::Internal("created post missing".to_string()))?;

    Ok(CreatePostResponse {
        post,
        thread_posts,
        automod_action: automod_decision.action,
        automod_note: (!automod_decision.reasons.is_empty()).then(|| automod_decision.reasons.join("; ")),
    })
}

pub fn update_post_tx(
    tx: &Transaction<'_>,
    post_id: &str,
    editor_actor_id: &str,
    patch: &UpdatePostRequest,
    now_ms: i64,
) -> Result<CommunityPostDto> {
    let existing = hydrate_post_row_conn(tx, post_id, Some(editor_actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("post '{post_id}' not found")))?;
    let is_author = existing.author_actor_id == editor_actor_id;
    let is_mod = match existing.community_id.as_deref() {
        Some(cid) => get_member_role_conn(tx, cid, editor_actor_id)?
            .is_some_and(CommunityMemberRole::is_moderator),
        None => false,
    };
    if !is_author && !is_mod {
        return Err(TelegramError::PermissionDenied(
            "only the post author or a community moderator may edit this post".to_string(),
        ));
    }
    if existing.status == PostStatus::Deleted {
        return Err(TelegramError::Validation(
            "cannot edit a deleted post".to_string(),
        ));
    }

    let new_title = patch.title.clone().or_else(|| existing.title.clone());
    let new_body = patch.body.clone().unwrap_or_else(|| existing.body.clone());
    let new_flair = patch.flair.clone().or_else(|| existing.flair.clone());
    let new_nsfw = patch.is_nsfw.unwrap_or(existing.is_nsfw);
    let new_spoiler = patch.is_spoiler.unwrap_or(existing.is_spoiler);
    let new_status = patch.status.unwrap_or(existing.status);
    let new_sched = patch.scheduled_at_ms.or(existing.scheduled_at_ms);

    if new_status == PostStatus::Deleted {
        return Err(TelegramError::Validation(
            "use delete_post to delete a post".to_string(),
        ));
    }
    if new_status == PostStatus::Scheduled && new_sched.is_none() {
        return Err(TelegramError::Validation(
            "scheduled posts must specify scheduled_at_ms".to_string(),
        ));
    }
    if new_title.as_deref().is_some_and(|t| t.trim().chars().count() > 300) {
        return Err(TelegramError::Validation(
            "post title must be <= 300 characters".to_string(),
        ));
    }
    if new_body.trim().chars().count() > crate::model::MAX_ARTICLE_BODY_CHARS {
        return Err(TelegramError::Validation(format!(
            "post body exceeds {} characters",
            crate::model::MAX_ARTICLE_BODY_CHARS
        )));
    }

    let next_rev: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM community_post_edits WHERE post_id = ?1",
            params![post_id],
            |r| r.get(0),
        )
        .map_err(|e| db_err("next post revision", e))?;
    tx.execute(
        "INSERT INTO community_post_edits (edit_id, post_id, revision, editor_actor_id, previous_title, previous_body_markdown, edit_reason, edited_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', ?7)",
        params![
            format!("edit_{}", Uuid::new_v4().simple()),
            post_id,
            next_rev,
            editor_actor_id,
            existing.title,
            existing.body,
            now_ms,
        ],
    )
    .map_err(|e| db_err("record post edit snapshot", e))?;

    let ent = extract_entities(&format!("{} {}", new_title.as_deref().unwrap_or(""), new_body));

    tx.execute(
        "UPDATE community_posts SET
            title=?2, body_markdown=?3, flair_text=?4, is_nsfw=?5, is_spoiler=?6,
            status=?7, scheduled_at_ms=?8, hashtags_json=?9, mentions_json=?10,
            edited_at_ms=?11, updated_at_ms=?11
         WHERE post_id=?1",
        params![
            post_id,
            new_title,
            new_body,
            new_flair,
            new_nsfw as i64,
            new_spoiler as i64,
            new_status.as_str(),
            new_sched,
            serde_json::to_string(&ent.hashtags)?,
            serde_json::to_string(&ent.mentions)?,
            now_ms,
        ],
    )
    .map_err(|e| db_err("update post", e))?;

    // Maintain accurate posts_count on Draft/Scheduled <-> Published transitions
    if !existing.is_removed {
        if matches!(existing.status, PostStatus::Draft | PostStatus::Scheduled)
            && new_status == PostStatus::Published
        {
            tx.execute(
                "UPDATE community_actors SET posts_count = posts_count + 1, updated_at_ms = ?2 WHERE actor_id = ?1",
                params![existing.author_actor_id, now_ms],
            )
            .map_err(|e| db_err("inc actor posts_count on publish", e))?;
            if let Some(cid) = existing.community_id.as_deref() {
                tx.execute(
                    "UPDATE communities SET posts_count = posts_count + 1, updated_at_ms = ?2 WHERE community_id = ?1",
                    params![cid, now_ms],
                )
                .map_err(|e| db_err("inc community posts_count on publish", e))?;
            }
        } else if existing.status == PostStatus::Published
            && matches!(new_status, PostStatus::Draft | PostStatus::Scheduled)
        {
            tx.execute(
                "UPDATE community_actors SET posts_count = MAX(0, posts_count - 1), updated_at_ms = ?2 WHERE actor_id = ?1",
                params![existing.author_actor_id, now_ms],
            )
            .map_err(|e| db_err("dec actor posts_count on unpublish", e))?;
            if let Some(cid) = existing.community_id.as_deref() {
                tx.execute(
                    "UPDATE communities SET posts_count = MAX(0, posts_count - 1), updated_at_ms = ?2 WHERE community_id = ?1",
                    params![cid, now_ms],
                )
                .map_err(|e| db_err("dec community posts_count on unpublish", e))?;
            }
        }
    }

    hydrate_post_row_conn(tx, post_id, Some(editor_actor_id))?
        .ok_or_else(|| TelegramError::Internal("updated post missing".to_string()))
}

pub fn delete_post_tx(tx: &Transaction<'_>, post_id: &str, actor_id: &str, now_ms: i64) -> Result<bool> {
    let existing = hydrate_post_row_conn(tx, post_id, Some(actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("post '{post_id}' not found")))?;
    let is_author = existing.author_actor_id == actor_id;
    let is_mod = match existing.community_id.as_deref() {
        Some(cid) => get_member_role_conn(tx, cid, actor_id)?
            .is_some_and(CommunityMemberRole::is_moderator),
        None => false,
    };
    if !is_author && !is_mod {
        return Err(TelegramError::PermissionDenied(
            "only the post author or a moderator may delete this post".to_string(),
        ));
    }
    if existing.status == PostStatus::Deleted {
        return Ok(false);
    }
    tx.execute(
        "UPDATE community_posts SET status = 'deleted', is_removed = 1, updated_at_ms = ?2 WHERE post_id = ?1",
        params![post_id, now_ms],
    )
    .map_err(|e| db_err("soft delete post", e))?;

    if existing.status == PostStatus::Published && !existing.is_removed {
        tx.execute(
            "UPDATE community_actors SET posts_count = MAX(0, posts_count - 1) WHERE actor_id = ?1",
            params![existing.author_actor_id],
        )
        .map_err(|e| db_err("dec actor posts_count", e))?;
        if let Some(cid) = existing.community_id.as_deref() {
            tx.execute(
                "UPDATE communities SET posts_count = MAX(0, posts_count - 1) WHERE community_id = ?1",
                params![cid],
            )
            .map_err(|e| db_err("dec community posts_count", e))?;
        }
    }
    Ok(true)
}

pub fn vote_post_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    post_id: &str,
    direction: i8,
    now_ms: i64,
) -> Result<VoteResponse> {
    if !matches!(direction, -1 | 0 | 1) {
        return Err(TelegramError::Validation("vote direction must be -1, 0, or 1".to_string()));
    }
    let (author_id, comm_slug, mut up, mut down, created_at_ms): (String, Option<String>, i64, i64, i64) = tx
        .query_row(
            "SELECT p.author_actor_id, c.slug, p.upvotes, p.downvotes, p.created_at_ms
             FROM community_posts p
             LEFT JOIN communities c ON c.community_id = p.community_id
             WHERE p.post_id = ?1",
            params![post_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(|e| db_err("lookup post for vote", e))?
        .ok_or_else(|| TelegramError::NotFound(format!("post '{post_id}' not found")))?;

    let prev_dir: i8 = tx
        .query_row(
            "SELECT direction FROM community_votes WHERE target_kind = 'post' AND target_id = ?1 AND actor_id = ?2",
            params![post_id, actor_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| db_err("lookup prior post vote", e))?
        .unwrap_or(0) as i8;

    let d_up = i64::from(direction == 1) - i64::from(prev_dir == 1);
    let d_down = i64::from(direction == -1) - i64::from(prev_dir == -1);
    let d_score = i64::from(direction) - i64::from(prev_dir);

    if direction == 0 {
        tx.execute(
            "DELETE FROM community_votes WHERE target_kind = 'post' AND target_id = ?1 AND actor_id = ?2",
            params![post_id, actor_id],
        )
        .map_err(|e| db_err("delete post vote", e))?;
    } else {
        tx.execute(
            "INSERT INTO community_votes (target_kind, target_id, actor_id, direction, updated_at_ms)
             VALUES ('post', ?1, ?2, ?3, ?4)
             ON CONFLICT(target_kind, target_id, actor_id) DO UPDATE SET direction = excluded.direction, updated_at_ms = excluded.updated_at_ms",
            params![post_id, actor_id, i64::from(direction), now_ms],
        )
        .map_err(|e| db_err("upsert post vote", e))?;
    }

    up = (up + d_up).max(0);
    down = (down + d_down).max(0);
    let score = up - down;
    let hot = reddit_hot(up, down, created_at_ms);
    let wilson = wilson_lower_bound(up, down, 1.96);

    tx.execute(
        "UPDATE community_posts SET upvotes=?2, downvotes=?3, score=?4, hot_score=?5, wilson_score=?6, updated_at_ms=?7 WHERE post_id=?1",
        params![post_id, up, down, score, hot, wilson, now_ms],
    )
    .map_err(|e| db_err("update post vote counts", e))?;

    // Self-votes NEVER grant or remove karma
    if author_id != actor_id && d_score != 0 {
        tx.execute(
            "UPDATE community_actors SET post_karma = post_karma + ?2, updated_at_ms = ?3 WHERE actor_id = ?1",
            params![author_id, d_score, now_ms],
        )
        .map_err(|e| db_err("update post_karma", e))?;
        let aff_delta = if direction == 1 { 1.5 } else if direction == -1 { -1.5 } else { 0.0 };
        bump_affinity_conn(tx, actor_id, "author", &author_id, aff_delta, now_ms)?;
        if let Some(slug) = comm_slug.as_deref() {
            bump_affinity_conn(tx, actor_id, "community", slug, aff_delta * 0.7, now_ms)?;
        }
    }

    Ok(VoteResponse {
        target_id: post_id.to_string(),
        direction,
        upvotes: up,
        downvotes: down,
        score,
    })
}

pub fn toggle_like_post_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    post_id: &str,
    now_ms: i64,
) -> Result<ToggleActionResponse> {
    let author_id: String = tx
        .query_row("SELECT author_actor_id FROM community_posts WHERE post_id = ?1", params![post_id], |r| r.get(0))
        .optional()
        .map_err(|e| db_err("lookup post for like", e))?
        .ok_or_else(|| TelegramError::NotFound(format!("post '{post_id}' not found")))?;

    let deleted = tx
        .execute(
            "DELETE FROM community_likes WHERE target_kind = 'post' AND target_id = ?1 AND actor_id = ?2",
            params![post_id, actor_id],
        )
        .map_err(|e| db_err("delete post like", e))?;
    let active = if deleted == 1 {
        tx.execute("UPDATE community_posts SET likes_count = MAX(0, likes_count - 1) WHERE post_id = ?1", params![post_id])
            .map_err(|e| db_err("dec post likes", e))?;
        false
    } else {
        tx.execute(
            "INSERT INTO community_likes (target_kind, target_id, actor_id, created_at_ms) VALUES ('post', ?1, ?2, ?3)",
            params![post_id, actor_id, now_ms],
        )
        .map_err(|e| db_err("insert post like", e))?;
        tx.execute("UPDATE community_posts SET likes_count = likes_count + 1 WHERE post_id = ?1", params![post_id])
            .map_err(|e| db_err("inc post likes", e))?;
        bump_affinity_conn(tx, actor_id, "author", &author_id, 1.0, now_ms)?;
        true
    };
    let count: i64 = tx
        .query_row("SELECT likes_count FROM community_posts WHERE post_id = ?1", params![post_id], |r| r.get(0))
        .map_err(|e| db_err("read post likes_count", e))?;
    Ok(ToggleActionResponse {
        target_id: post_id.to_string(),
        active,
        count: count.max(0) as u64,
    })
}

pub fn toggle_repost_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    post_id: &str,
    now_ms: i64,
) -> Result<RepostResponse> {
    let author_id: String = tx
        .query_row("SELECT author_actor_id FROM community_posts WHERE post_id = ?1", params![post_id], |r| r.get(0))
        .optional()
        .map_err(|e| db_err("lookup post for repost", e))?
        .ok_or_else(|| TelegramError::NotFound(format!("post '{post_id}' not found")))?;
    let actor = get_actor_conn(tx, actor_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("actor '{actor_id}' not found")))?;

    let deleted = tx
        .execute(
            "DELETE FROM community_reposts WHERE actor_id = ?1 AND original_post_id = ?2",
            params![actor_id, post_id],
        )
        .map_err(|e| db_err("delete repost", e))?;
    let reposted = if deleted == 1 {
        tx.execute("UPDATE community_posts SET reposts_count = MAX(0, reposts_count - 1) WHERE post_id = ?1", params![post_id])
            .map_err(|e| db_err("dec reposts_count", e))?;
        false
    } else {
        tx.execute(
            "INSERT INTO community_reposts (actor_id, original_post_id, repost_post_id, created_at_ms) VALUES (?1, ?2, NULL, ?3)",
            params![actor_id, post_id, now_ms],
        )
        .map_err(|e| db_err("insert repost", e))?;
        tx.execute("UPDATE community_posts SET reposts_count = reposts_count + 1 WHERE post_id = ?1", params![post_id])
            .map_err(|e| db_err("inc reposts_count", e))?;
        bump_affinity_conn(tx, actor_id, "author", &author_id, 2.5, now_ms)?;
        insert_notification_conn(
            tx,
            &author_id,
            actor_id,
            CommunityNotificationKind::Repost,
            Some(post_id),
            None,
            &format!("@{} reposted your post.", actor.handle),
            now_ms,
        )?;
        true
    };
    let count: i64 = tx
        .query_row("SELECT reposts_count FROM community_posts WHERE post_id = ?1", params![post_id], |r| r.get(0))
        .map_err(|e| db_err("read reposts_count", e))?;
    Ok(RepostResponse {
        post_id: post_id.to_string(),
        reposted,
        reposts_count: count.max(0) as u64,
        repost_post_id: None,
    })
}

pub fn toggle_bookmark_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    post_id: &str,
    folder_name: Option<&str>,
    force_remove: Option<bool>,
    now_ms: i64,
) -> Result<BookmarkResponse> {
    if hydrate_post_row_conn(tx, post_id, None)?.is_none() {
        return Err(TelegramError::NotFound(format!("post '{post_id}' not found")));
    }
    let folder = folder_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("All")
        .to_string();

    let exists: Option<String> = tx
        .query_row(
            "SELECT folder_name FROM community_bookmarks WHERE actor_id = ?1 AND post_id = ?2",
            params![actor_id, post_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| db_err("check bookmark", e))?;

    let should_remove = force_remove.unwrap_or_else(|| exists.is_some() && folder_name.is_none());
    let bookmarked = if should_remove {
        let del = tx
            .execute(
                "DELETE FROM community_bookmarks WHERE actor_id = ?1 AND post_id = ?2",
                params![actor_id, post_id],
            )
            .map_err(|e| db_err("delete bookmark", e))?;
        if del == 1 {
            tx.execute("UPDATE community_posts SET bookmarks_count = MAX(0, bookmarks_count - 1) WHERE post_id = ?1", params![post_id])
                .map_err(|e| db_err("dec bookmarks_count", e))?;
        }
        false
    } else {
        tx.execute(
            "INSERT INTO community_bookmarks (actor_id, post_id, folder_name, created_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(actor_id, post_id) DO UPDATE SET folder_name = excluded.folder_name",
            params![actor_id, post_id, folder, now_ms],
        )
        .map_err(|e| db_err("upsert bookmark", e))?;
        if exists.is_none() {
            tx.execute("UPDATE community_posts SET bookmarks_count = bookmarks_count + 1 WHERE post_id = ?1", params![post_id])
                .map_err(|e| db_err("inc bookmarks_count", e))?;
        }
        true
    };
    let count: i64 = tx
        .query_row("SELECT bookmarks_count FROM community_posts WHERE post_id = ?1", params![post_id], |r| r.get(0))
        .map_err(|e| db_err("read bookmarks_count", e))?;
    Ok(BookmarkResponse {
        post_id: post_id.to_string(),
        bookmarked,
        folder,
        bookmarks_count: count.max(0) as u64,
    })
}

pub fn vote_poll_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    post_id: &str,
    option_ids: &[String],
    now_ms: i64,
) -> Result<PollDto> {
    let voter = get_actor_conn(tx, actor_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("actor '{actor_id}' not found")))?;
    let (poll_json, poll_closed): (Option<String>, bool) = tx
        .query_row(
            "SELECT poll_json, poll_is_closed FROM community_posts WHERE post_id = ?1",
            params![post_id],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
        )
        .optional()
        .map_err(|e| db_err("lookup post poll", e))?
        .ok_or_else(|| TelegramError::NotFound(format!("post '{post_id}' not found")))?;

    let mut poll = load_poll_for_post_conn(tx, post_id, poll_json.as_deref(), poll_closed, Some(actor_id))?
        .ok_or_else(|| TelegramError::Validation(format!("post '{post_id}' does not contain a poll")))?;
    let prev = poll.viewer_voted_option_ids.clone();

    apply_poll_vote(&mut poll, voter.actor_kind, option_ids, &prev, now_ms)?;

    tx.execute(
        "DELETE FROM community_poll_votes WHERE post_id = ?1 AND actor_id = ?2",
        params![post_id, actor_id],
    )
    .map_err(|e| db_err("clear prior poll votes", e))?;
    for opt_id in &poll.viewer_voted_option_ids {
        tx.execute(
            "INSERT INTO community_poll_votes (post_id, actor_id, option_id, actor_kind, voted_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![post_id, actor_id, opt_id, voter.actor_kind.as_str(), now_ms],
        )
        .map_err(|e| db_err("insert poll vote", e))?;
    }

    tx.execute(
        "UPDATE community_posts SET poll_json = ?2, updated_at_ms = ?3 WHERE post_id = ?1",
        params![post_id, serde_json::to_string(&poll)?, now_ms],
    )
    .map_err(|e| db_err("persist poll json", e))?;

    Ok(poll)
}

pub fn give_award_tx(
    tx: &Transaction<'_>,
    giver_actor_id: &str,
    target_kind: &str,
    target_id: &str,
    req: &GiveAwardRequest,
    now_ms: i64,
) -> Result<Vec<PostAwardDto>> {
    if !matches!(target_kind, "post" | "comment") {
        return Err(TelegramError::Validation("target_kind must be 'post' or 'comment'".to_string()));
    }
    let giver = get_actor_conn(tx, giver_actor_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("giver '{giver_actor_id}' not found")))?;
    let (recipient_actor_id, post_id_for_notif): (String, String) = if target_kind == "post" {
        let a: String = tx
            .query_row("SELECT author_actor_id FROM community_posts WHERE post_id = ?1", params![target_id], |r| r.get(0))
            .optional()
            .map_err(|e| db_err("lookup post author for award", e))?
            .ok_or_else(|| TelegramError::NotFound(format!("post '{target_id}' not found")))?;
        (a, target_id.to_string())
    } else {
        tx.query_row(
            "SELECT author_actor_id, post_id FROM community_comments WHERE comment_id = ?1",
            params![target_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| db_err("lookup comment author for award", e))?
        .ok_or_else(|| TelegramError::NotFound(format!("comment '{target_id}' not found")))?
    };

    let kind = req.award_kind.trim().to_lowercase();
    let (icon_emoji, karma_bonus) = match kind.as_str() {
        "gold" => ("🏆", 25_i64),
        "silver" => ("🥈", 10_i64),
        "spark" | "insightful" => ("💡", 15_i64),
        "rocket" | "boost" => ("🚀", 20_i64),
        _ => ("🌟", 10_i64),
    };
    let award_id = format!("aw_{}", Uuid::new_v4().simple());
    tx.execute(
        "INSERT INTO community_awards (award_id, target_kind, target_id, giver_actor_id, recipient_actor_id, award_kind, icon_emoji, message, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            award_id,
            target_kind,
            target_id,
            giver_actor_id,
            recipient_actor_id,
            if kind.is_empty() { "gold" } else { kind.as_str() },
            icon_emoji,
            req.message.as_deref().unwrap_or(""),
            now_ms,
        ],
    )
    .map_err(|e| db_err("insert award", e))?;

    if target_kind == "post" {
        tx.execute("UPDATE community_posts SET awards_count = awards_count + 1 WHERE post_id = ?1", params![target_id])
            .map_err(|e| db_err("inc post awards_count", e))?;
    } else {
        tx.execute("UPDATE community_comments SET awards_count = awards_count + 1 WHERE comment_id = ?1", params![target_id])
            .map_err(|e| db_err("inc comment awards_count", e))?;
    }
    if recipient_actor_id != giver_actor_id {
        tx.execute(
            "UPDATE community_actors SET awardee_karma = awardee_karma + ?2, updated_at_ms = ?3 WHERE actor_id = ?1",
            params![recipient_actor_id, karma_bonus, now_ms],
        )
        .map_err(|e| db_err("inc awardee_karma", e))?;
        insert_notification_conn(
            tx,
            &recipient_actor_id,
            giver_actor_id,
            CommunityNotificationKind::Award,
            Some(&post_id_for_notif),
            (target_kind == "comment").then_some(target_id),
            &format!("@{} gave you a {icon_emoji} award!", giver.handle),
            now_ms,
        )?;
    }

    load_awards_conn(tx, target_kind, target_id)
}

pub fn publish_due_scheduled_posts_tx(tx: &Transaction<'_>, now_ms: i64) -> Result<Vec<CommunityPostDto>> {
    let mut stmt = tx
        .prepare(
            "SELECT post_id, author_actor_id, community_id, upvotes, downvotes, is_removed, thread_index
             FROM community_posts
             WHERE status = 'scheduled' AND scheduled_at_ms IS NOT NULL AND scheduled_at_ms <= ?1
             ORDER BY scheduled_at_ms ASC, thread_index ASC",
        )
        .map_err(|e| db_err("prepare due scheduled posts", e))?;
    let due: Vec<(String, String, Option<String>, i64, i64, i64, i64)> = stmt
        .query_map(params![now_ms], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .map_err(|e| db_err("query due scheduled posts", e))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| db_err("collect due scheduled posts", e))?;
    drop(stmt);

    let mut published = Vec::with_capacity(due.len());
    for (post_id, author_id, comm_id, up, down, is_removed, thread_index) in due {
        let publish_ts = now_ms.saturating_add(thread_index.max(0));
        let hot = reddit_hot(up, down, publish_ts);
        tx.execute(
            "UPDATE community_posts SET status = 'published', hot_score = ?2, created_at_ms = ?3, updated_at_ms = ?3 WHERE post_id = ?1",
            params![post_id, hot, publish_ts],
        )
        .map_err(|e| db_err("publish scheduled post", e))?;
        if is_removed == 0 && thread_index == 0 {
            tx.execute(
                "UPDATE community_actors SET posts_count = posts_count + 1, updated_at_ms = ?2 WHERE actor_id = ?1",
                params![author_id, publish_ts],
            )
            .map_err(|e| db_err("inc actor posts on publish", e))?;
            if let Some(cid) = comm_id {
                tx.execute(
                    "UPDATE communities SET posts_count = posts_count + 1, updated_at_ms = ?2 WHERE community_id = ?1",
                    params![cid, publish_ts],
                )
                .map_err(|e| db_err("inc community posts on publish", e))?;
            }
        }
        if let Some(p) = hydrate_post_row_conn(tx, &post_id, None)? {
            published.push(p);
        }
    }
    Ok(published)
}

pub fn close_expired_polls_tx(tx: &Transaction<'_>, now_ms: i64) -> Result<Vec<String>> {
    let mut stmt = tx
        .prepare(
            "SELECT post_id FROM community_posts
             WHERE poll_is_closed = 0 AND poll_closes_at_ms IS NOT NULL AND poll_closes_at_ms <= ?1",
        )
        .map_err(|e| db_err("prepare expired polls", e))?;
    let ids: Vec<String> = stmt
        .query_map(params![now_ms], |r| r.get::<_, String>(0))
        .map_err(|e| db_err("query expired polls", e))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| db_err("collect expired polls", e))?;
    drop(stmt);

    for pid in &ids {
        tx.execute(
            "UPDATE community_posts SET poll_is_closed = 1, updated_at_ms = ?2 WHERE post_id = ?1",
            params![pid, now_ms],
        )
        .map_err(|e| db_err("close expired poll", e))?;
    }
    Ok(ids)
}

/// Checks whether `media_id` is referenced by any public published post, actor avatar/banner, or community icon/banner (M2 public media access).
pub fn is_media_referenced_publicly_conn(conn: &Connection, media_id: &str) -> Result<bool> {
    let trimmed = media_id.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    let actor_hit: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM community_actors WHERE avatar_media_id = ?1 OR banner_media_id = ?1",
            params![trimmed],
            |r| r.get(0),
        )
        .map_err(|e| db_err("check actor media ref", e))?;
    if actor_hit > 0 {
        return Ok(true);
    }
    let comm_hit: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM communities WHERE icon_media_id = ?1 OR banner_media_id = ?1",
            params![trimmed],
            |r| r.get(0),
        )
        .map_err(|e| db_err("check community media ref", e))?;
    if comm_hit > 0 {
        return Ok(true);
    }
    let mut stmt = conn
        .prepare("SELECT media_json FROM community_posts WHERE status = 'published' AND is_removed = 0 AND media_json != '[]'")
        .map_err(|e| db_err("prepare post media ref", e))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| db_err("query post media ref", e))?;
    for r in rows {
        let json = r.map_err(|e| db_err("read media_json", e))?;
        if let Ok(items) = serde_json::from_str::<Vec<PostMediaAttachmentDto>>(&json) {
            if items.iter().any(|m| m.media_id == trimmed) {
                return Ok(true);
            }
        }
    }
    let mut cstmt = conn
        .prepare(
            "SELECT c.media_json
             FROM community_comments c
             JOIN community_posts p ON p.post_id = c.post_id
             WHERE c.is_removed = 0 AND c.is_deleted = 0
               AND p.status = 'published' AND p.is_removed = 0
               AND c.media_json != '[]'",
        )
        .map_err(|e| db_err("prepare comment media ref", e))?;
    let crows = cstmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| db_err("query comment media ref", e))?;
    for r in crows {
        let json = r.map_err(|e| db_err("read comment media_json", e))?;
        if let Ok(items) = serde_json::from_str::<Vec<PostMediaAttachmentDto>>(&json) {
            if items.iter().any(|m| m.media_id == trimmed) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

impl CommunityStore {
    pub fn create_post(&self, author_actor_id: &str, req: &CreatePostRequest, now_ms: i64) -> Result<CreatePostResponse> {
        self.with_transaction(|tx| create_post_tx(tx, author_actor_id, req, now_ms))
    }

    pub fn get_post(&self, post_id: &str, viewer_actor_id: Option<&str>) -> Result<Option<CommunityPostDto>> {
        self.with_conn(|conn| get_post_conn(conn, post_id, viewer_actor_id))
    }

    pub fn update_post(&self, post_id: &str, editor_actor_id: &str, patch: &UpdatePostRequest, now_ms: i64) -> Result<CommunityPostDto> {
        self.with_transaction(|tx| update_post_tx(tx, post_id, editor_actor_id, patch, now_ms))
    }

    pub fn list_post_edits(&self, post_id: &str) -> Result<Vec<PostEditRevisionDto>> {
        self.with_conn(|conn| list_post_edits_conn(conn, post_id))
    }

    pub fn delete_post(&self, post_id: &str, actor_id: &str, now_ms: i64) -> Result<bool> {
        self.with_transaction(|tx| delete_post_tx(tx, post_id, actor_id, now_ms))
    }

    pub fn vote_post(&self, actor_id: &str, post_id: &str, direction: i8, now_ms: i64) -> Result<VoteResponse> {
        self.with_transaction(|tx| vote_post_tx(tx, actor_id, post_id, direction, now_ms))
    }

    pub fn toggle_like_post(&self, actor_id: &str, post_id: &str, now_ms: i64) -> Result<ToggleActionResponse> {
        self.with_transaction(|tx| toggle_like_post_tx(tx, actor_id, post_id, now_ms))
    }

    pub fn toggle_repost(&self, actor_id: &str, post_id: &str, now_ms: i64) -> Result<RepostResponse> {
        self.with_transaction(|tx| toggle_repost_tx(tx, actor_id, post_id, now_ms))
    }

    pub fn toggle_bookmark(
        &self,
        actor_id: &str,
        post_id: &str,
        folder_name: Option<&str>,
        force_remove: Option<bool>,
        now_ms: i64,
    ) -> Result<BookmarkResponse> {
        self.with_transaction(|tx| {
            toggle_bookmark_tx(tx, actor_id, post_id, folder_name, force_remove, now_ms)
        })
    }

    pub fn vote_poll(
        &self,
        actor_id: &str,
        post_id: &str,
        option_ids: &[String],
        now_ms: i64,
    ) -> Result<PollDto> {
        self.with_transaction(|tx| vote_poll_tx(tx, actor_id, post_id, option_ids, now_ms))
    }

    pub fn give_award(
        &self,
        giver_actor_id: &str,
        target_kind: &str,
        target_id: &str,
        req: &GiveAwardRequest,
        now_ms: i64,
    ) -> Result<Vec<PostAwardDto>> {
        self.with_transaction(|tx| give_award_tx(tx, giver_actor_id, target_kind, target_id, req, now_ms))
    }

    pub fn publish_due_scheduled_posts(&self, now_ms: i64) -> Result<Vec<CommunityPostDto>> {
        self.with_transaction(|tx| publish_due_scheduled_posts_tx(tx, now_ms))
    }

    pub fn close_expired_polls(&self, now_ms: i64) -> Result<Vec<String>> {
        self.with_transaction(|tx| close_expired_polls_tx(tx, now_ms))
    }

    pub fn is_media_referenced_publicly(&self, media_id: &str) -> Result<bool> {
        self.with_conn(|conn| is_media_referenced_publicly_conn(conn, media_id))
    }

    /// Loads thread posts sharing the same `thread_root_id` (plus quote chain ancestors) for a post.
    pub fn list_thread_and_quote_pool(&self, post: &CommunityPostDto, viewer_actor_id: Option<&str>) -> Result<Vec<CommunityPostDto>> {
        self.with_conn(|conn| {
            let mut ids = Vec::new();
            let root_id = post.thread_root_id.as_deref().unwrap_or(&post.post_id);
            let mut stmt = conn
                .prepare("SELECT post_id FROM community_posts WHERE post_id = ?1 OR thread_root_id = ?1 ORDER BY thread_index ASC")
                .map_err(|e| db_err("prepare thread pool", e))?;
            let rows = stmt
                .query_map(params![root_id], |r| r.get::<_, String>(0))
                .map_err(|e| db_err("query thread pool", e))?;
            for r in rows {
                ids.push(r.map_err(|e| db_err("read thread id", e))?);
            }
            let mut cur_q = post.quoted_post_id.clone();
            let mut seen = HashMap::<String, bool>::new();
            for _ in 0..8 {
                let Some(qid) = cur_q else { break };
                if seen.insert(qid.clone(), true).is_some() {
                    break;
                }
                ids.push(qid.clone());
                cur_q = conn
                    .query_row("SELECT quote_post_id FROM community_posts WHERE post_id = ?1", params![qid], |r| r.get::<_, Option<String>>(0))
                    .optional()
                    .map_err(|e| db_err("walk quote chain", e))?
                    .flatten();
            }
            let mut out = Vec::new();
            let mut dedup = HashMap::<String, bool>::new();
            for id in ids {
                if dedup.insert(id.clone(), true).is_none() {
                    if let Some(p) = hydrate_post_row_conn(conn, &id, viewer_actor_id)? {
                        out.push(p);
                    }
                }
            }
            Ok(out)
        })
    }
}
