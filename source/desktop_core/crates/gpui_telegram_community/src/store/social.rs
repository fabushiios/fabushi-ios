//! SQLite operations for Lists, Signals/Affinities, Algo Prefs, Notifications, Trending, Feed Execution, and Bot Cooldowns.

use super::actors::{bump_affinity_conn, get_actor_conn};
use super::communities::enqueue_mod_item_tx;
use super::posts::hydrate_post_row_conn;
use super::{CommunityStore, db_err};
use crate::identity::ActorGraph;
use crate::model::{extract_entities, parse_search_operators};
use crate::recommender::scoring::default_prefs_for_preset;
use crate::recommender::signals::{
    PostCandidate, ViewerContext, blend_interest_vector, embed_text_64, signal_affinity_delta,
};
use crate::recommender::{CommunityRecommender, TrendingEngine};
use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, AlgoPreferencesDto, AlgoPreset, CommunityListDto, CommunityListKind,
    CommunityNotificationDto, CommunityNotificationKind, CommunityPostDto,
    CreateCommunityListRequest, EngagementSignalKind, FeedRequestDto, FeedResponseDto, FeedTab,
    NotificationsListResponse, PostFormat, RecordSignalRequest, RecordSignalResponse,
    TrendingTopicDto, UpdateCommunityListRequest,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use std::collections::HashMap;
use uuid::Uuid;

fn normalize_slug_key(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    lower
        .strip_prefix("c/")
        .or_else(|| lower.strip_prefix("r/"))
        .unwrap_or(&lower)
        .to_string()
}

fn row_to_list(r: &Row<'_>) -> rusqlite::Result<CommunityListDto> {
    let kind_str: String = r.get(4)?;
    let members_json: String = r.get(7)?;
    let comms_json: String = r.get(8)?;
    Ok(CommunityListDto {
        list_id: r.get(0)?,
        owner_actor_id: r.get(1)?,
        name: r.get(2)?,
        description: r.get(3)?,
        kind: CommunityListKind::parse(&kind_str).unwrap_or(CommunityListKind::Mixed),
        is_private: r.get::<_, i64>(5)? != 0,
        is_pinned: r.get::<_, i64>(6)? != 0,
        member_actor_ids: serde_json::from_str(&members_json).unwrap_or_default(),
        community_slugs: serde_json::from_str(&comms_json).unwrap_or_default(),
        created_at_ms: r.get(9)?,
        updated_at_ms: r.get(10)?,
    })
}

pub fn list_lists_conn(conn: &Connection, owner_actor_id: &str) -> Result<Vec<CommunityListDto>> {
    let mut stmt = conn
        .prepare(
            "SELECT list_id, owner_actor_id, name, description, kind, is_private, is_pinned, members_json, communities_json, created_at_ms, updated_at_ms
             FROM community_lists WHERE owner_actor_id = ?1 OR is_private = 0
             ORDER BY is_pinned DESC, updated_at_ms DESC",
        )
        .map_err(|e| db_err("prepare lists", e))?;
    let rows = stmt
        .query_map(params![owner_actor_id], row_to_list)
        .map_err(|e| db_err("query lists", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| db_err("read list", e))?);
    }
    Ok(out)
}

pub fn create_list_tx(
    tx: &Transaction<'_>,
    owner_actor_id: &str,
    req: &CreateCommunityListRequest,
    now_ms: i64,
) -> Result<CommunityListDto> {
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(TelegramError::Validation("list name must be 1..=80 characters".to_string()));
    }
    let list_id = format!("list_{}", Uuid::new_v4().simple());
    let slugs: Vec<String> = req
        .community_slugs
        .iter()
        .map(|s| normalize_slug_key(s))
        .filter(|s| !s.is_empty())
        .collect();
    tx.execute(
        "INSERT INTO community_lists (list_id, owner_actor_id, name, description, kind, is_private, is_pinned, members_json, communities_json, created_at_ms, updated_at_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
        params![
            list_id,
            owner_actor_id,
            name,
            req.description.trim(),
            req.kind.as_str(),
            req.is_private as i64,
            req.is_pinned as i64,
            serde_json::to_string(&req.member_actor_ids)?,
            serde_json::to_string(&slugs)?,
            now_ms,
        ],
    )
    .map_err(|e| db_err("insert list", e))?;
    Ok(CommunityListDto {
        list_id,
        owner_actor_id: owner_actor_id.to_string(),
        name: name.to_string(),
        description: req.description.trim().to_string(),
        kind: req.kind,
        is_private: req.is_private,
        is_pinned: req.is_pinned,
        member_actor_ids: req.member_actor_ids.clone(),
        community_slugs: slugs,
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
    })
}

pub fn update_list_tx(
    tx: &Transaction<'_>,
    list_id: &str,
    actor_id: &str,
    patch: &UpdateCommunityListRequest,
    now_ms: i64,
) -> Result<CommunityListDto> {
    let mut list = tx
        .query_row(
            "SELECT list_id, owner_actor_id, name, description, kind, is_private, is_pinned, members_json, communities_json, created_at_ms, updated_at_ms
             FROM community_lists WHERE list_id = ?1",
            params![list_id],
            row_to_list,
        )
        .optional()
        .map_err(|e| db_err("lookup list", e))?
        .ok_or_else(|| TelegramError::NotFound(format!("list '{list_id}' not found")))?;
    if list.owner_actor_id != actor_id {
        return Err(TelegramError::PermissionDenied("only the list owner may update this list".to_string()));
    }
    if let Some(name) = patch.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        list.name = name.to_string();
    }
    if let Some(desc) = patch.description.as_deref() {
        list.description = desc.trim().to_string();
    }
    if let Some(k) = patch.kind {
        list.kind = k;
    }
    if let Some(priv_) = patch.is_private {
        list.is_private = priv_;
    }
    if let Some(pin) = patch.is_pinned {
        list.is_pinned = pin;
    }
    if let Some(members) = patch.member_actor_ids.as_ref() {
        list.member_actor_ids = members.clone();
    }
    if let Some(slugs) = patch.community_slugs.as_ref() {
        list.community_slugs = slugs
            .iter()
            .map(|s| normalize_slug_key(s))
            .filter(|s| !s.is_empty())
            .collect();
    }
    list.updated_at_ms = now_ms;
    tx.execute(
        "UPDATE community_lists SET name=?2, description=?3, kind=?4, is_private=?5, is_pinned=?6, members_json=?7, communities_json=?8, updated_at_ms=?9 WHERE list_id=?1",
        params![
            list.list_id,
            list.name,
            list.description,
            list.kind.as_str(),
            list.is_private as i64,
            list.is_pinned as i64,
            serde_json::to_string(&list.member_actor_ids)?,
            serde_json::to_string(&list.community_slugs)?,
            now_ms,
        ],
    )
    .map_err(|e| db_err("update list", e))?;
    Ok(list)
}

pub fn delete_list_tx(tx: &Transaction<'_>, list_id: &str, actor_id: &str) -> Result<bool> {
    let owner: Option<String> = tx
        .query_row("SELECT owner_actor_id FROM community_lists WHERE list_id = ?1", params![list_id], |r| r.get(0))
        .optional()
        .map_err(|e| db_err("lookup list owner", e))?;
    let Some(owner_id) = owner else {
        return Ok(false);
    };
    if owner_id != actor_id {
        return Err(TelegramError::PermissionDenied("only the list owner may delete this list".to_string()));
    }
    let changed = tx
        .execute("DELETE FROM community_lists WHERE list_id = ?1", params![list_id])
        .map_err(|e| db_err("delete list", e))?;
    Ok(changed > 0)
}

pub fn record_signal_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    req: &RecordSignalRequest,
    now_ms: i64,
) -> Result<RecordSignalResponse> {
    let post_meta: Option<(String, Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT p.author_actor_id, p.community_id, c.slug
             FROM community_posts p
             LEFT JOIN communities c ON c.community_id = p.community_id
             WHERE p.post_id = ?1",
            params![req.post_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| db_err("lookup post for signal", e))?;

    let target_actor_id = req
        .target_actor_id
        .clone()
        .or_else(|| post_meta.as_ref().map(|(a, _, _)| a.clone()));
    let community_slug = req
        .community_slug
        .as_deref()
        .map(normalize_slug_key)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            post_meta
                .as_ref()
                .and_then(|(_, _, slug)| slug.as_deref().map(normalize_slug_key))
                .filter(|s| !s.is_empty())
        });
    let community_id = match post_meta.as_ref().and_then(|(_, cid, _)| cid.clone()) {
        Some(cid) => Some(cid),
        None => match community_slug.as_deref() {
            Some(slug) => tx
                .query_row(
                    "SELECT community_id FROM communities WHERE slug = ?1",
                    params![slug],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| db_err("lookup community_id by slug for signal", e))?,
            None => None,
        },
    };

    let value = req.dwell_ms.map(|ms| ms as f64).unwrap_or(1.0);
    tx.execute(
        "INSERT INTO community_user_signals (actor_id, signal_kind, post_id, community_id, target_actor_id, value, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            actor_id,
            req.kind.as_str(),
            req.post_id,
            community_id,
            target_actor_id,
            value,
            now_ms,
        ],
    )
    .map_err(|e| db_err("insert user signal", e))?;
    let signal_id = tx.last_insert_rowid().to_string();

    if req.kind == EngagementSignalKind::Impression {
        tx.execute(
            "UPDATE community_posts SET views_count = views_count + 1 WHERE post_id = ?1",
            params![req.post_id],
        )
        .map_err(|e| db_err("inc views_count", e))?;
    }

    let delta = signal_affinity_delta(req.kind, req.dwell_ms);
    if delta.abs() > 1e-9 {
        if let Some(ref ta) = target_actor_id {
            if ta != actor_id {
                bump_affinity_conn(tx, actor_id, "author", ta, delta, now_ms)?;
            }
        }
        if let Some(ref slug) = community_slug {
            bump_affinity_conn(tx, actor_id, "community", slug, delta * 0.75, now_ms)?;
        }
    }

    if req.kind == EngagementSignalKind::Report {
        if let Some(ref cid) = community_id {
            enqueue_mod_item_tx(
                tx,
                cid,
                "post",
                &req.post_id,
                Some(actor_id),
                "report",
                req.reason.as_deref().unwrap_or("Reported by user"),
                None,
                now_ms,
            )?;
        }
    }

    Ok(RecordSignalResponse {
        recorded: true,
        signal_id,
    })
}

pub fn get_algo_prefs_conn(conn: &Connection, actor_id: &str) -> Result<AlgoPreferencesDto> {
    let row = conn
        .query_row(
            "SELECT preset, recency_weight, affinity_weight, quality_weight, discovery_ratio, bot_content_ratio, show_explanations, allow_nsfw
             FROM community_algo_prefs WHERE actor_id = ?1",
            params![actor_id],
            |r| {
                let preset_str: String = r.get(0)?;
                Ok(AlgoPreferencesDto {
                    preset: AlgoPreset::parse(&preset_str).unwrap_or(AlgoPreset::Balanced),
                    recency_weight: r.get(1)?,
                    affinity_weight: r.get(2)?,
                    quality_weight: r.get(3)?,
                    discovery_ratio: r.get(4)?,
                    bot_content_ratio: r.get(5)?,
                    show_explanations: r.get::<_, i64>(6)? != 0,
                    allow_nsfw: r.get::<_, i64>(7)? != 0,
                })
            },
        )
        .optional()
        .map_err(|e| db_err("get algo prefs", e))?;
    Ok(row.unwrap_or_else(|| default_prefs_for_preset(AlgoPreset::Balanced)))
}

pub fn put_algo_prefs_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    prefs: &AlgoPreferencesDto,
    now_ms: i64,
) -> Result<AlgoPreferencesDto> {
    let defaults = default_prefs_for_preset(prefs.preset);
    let sanitize_f64 = |v: f64, lo: f64, hi: f64, fallback: f64| -> f64 {
        if v.is_finite() { v.clamp(lo, hi) } else { fallback }
    };
    let sanitized = AlgoPreferencesDto {
        preset: prefs.preset,
        recency_weight: sanitize_f64(prefs.recency_weight, 0.0, 1.0, defaults.recency_weight),
        affinity_weight: sanitize_f64(prefs.affinity_weight, 0.0, 1.0, defaults.affinity_weight),
        quality_weight: sanitize_f64(prefs.quality_weight, 0.0, 1.0, defaults.quality_weight),
        discovery_ratio: sanitize_f64(prefs.discovery_ratio, 0.0, 0.5, defaults.discovery_ratio),
        bot_content_ratio: sanitize_f64(prefs.bot_content_ratio, 0.0, 1.0, defaults.bot_content_ratio),
        show_explanations: prefs.show_explanations,
        allow_nsfw: prefs.allow_nsfw,
    };
    tx.execute(
        "INSERT INTO community_algo_prefs (
            actor_id, preset, recency_weight, affinity_weight, quality_weight,
            discovery_ratio, bot_content_ratio, show_explanations, allow_nsfw, updated_at_ms
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(actor_id) DO UPDATE SET
            preset=excluded.preset,
            recency_weight=excluded.recency_weight,
            affinity_weight=excluded.affinity_weight,
            quality_weight=excluded.quality_weight,
            discovery_ratio=excluded.discovery_ratio,
            bot_content_ratio=excluded.bot_content_ratio,
            show_explanations=excluded.show_explanations,
            allow_nsfw=excluded.allow_nsfw,
            updated_at_ms=excluded.updated_at_ms",
        params![
            actor_id,
            sanitized.preset.as_str(),
            sanitized.recency_weight,
            sanitized.affinity_weight,
            sanitized.quality_weight,
            sanitized.discovery_ratio,
            sanitized.bot_content_ratio,
            sanitized.show_explanations as i64,
            sanitized.allow_nsfw as i64,
            now_ms,
        ],
    )
    .map_err(|e| db_err("upsert algo prefs", e))?;
    Ok(sanitized)
}

pub fn list_notifications_conn(
    conn: &Connection,
    recipient_actor_id: &str,
    kind_filter: Option<CommunityNotificationKind>,
    limit: usize,
) -> Result<NotificationsListResponse> {
    let unread_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM community_notifications WHERE recipient_actor_id = ?1 AND is_read = 0",
            params![recipient_actor_id],
            |r| r.get(0),
        )
        .map_err(|e| db_err("count unread notifications", e))?;

    let limit = limit.clamp(1, 200) as i64;
    let map_row = |r: &Row<'_>| -> rusqlite::Result<CommunityNotificationDto> {
        let ak: String = r.get(5)?;
        let nk: String = r.get(6)?;
        Ok(CommunityNotificationDto {
            notif_id: r.get(0)?,
            recipient_actor_id: r.get(1)?,
            actor_id: r.get(2)?,
            actor_handle: r.get(3)?,
            actor_display_name: r.get(4)?,
            actor_kind: ActorKind::parse(&ak).unwrap_or(ActorKind::Human),
            kind: CommunityNotificationKind::parse(&nk).unwrap_or(CommunityNotificationKind::System),
            target_post_id: r.get(7)?,
            target_comment_id: r.get(8)?,
            preview_text: r.get(9)?,
            is_read: r.get::<_, i64>(10)? != 0,
            created_at_ms: r.get(11)?,
        })
    };

    let mut notifications = Vec::new();
    if let Some(kind) = kind_filter {
        let mut stmt = conn
            .prepare(
                "SELECT n.notif_id, n.recipient_actor_id, n.actor_id, a.handle, a.display_name, a.actor_kind,
                        n.kind, n.target_post_id, n.target_comment_id, n.preview_text, n.is_read, n.created_at_ms
                 FROM community_notifications n
                 JOIN community_actors a ON a.actor_id = n.actor_id
                 WHERE n.recipient_actor_id = ?1 AND n.kind = ?2
                 ORDER BY n.created_at_ms DESC LIMIT ?3",
            )
            .map_err(|e| db_err("prepare filtered notifications", e))?;
        let rows = stmt
            .query_map(params![recipient_actor_id, kind.as_str(), limit], map_row)
            .map_err(|e| db_err("query filtered notifications", e))?;
        for r in rows {
            notifications.push(r.map_err(|e| db_err("read notification", e))?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT n.notif_id, n.recipient_actor_id, n.actor_id, a.handle, a.display_name, a.actor_kind,
                        n.kind, n.target_post_id, n.target_comment_id, n.preview_text, n.is_read, n.created_at_ms
                 FROM community_notifications n
                 JOIN community_actors a ON a.actor_id = n.actor_id
                 WHERE n.recipient_actor_id = ?1
                 ORDER BY n.created_at_ms DESC LIMIT ?2",
            )
            .map_err(|e| db_err("prepare notifications", e))?;
        let rows = stmt
            .query_map(params![recipient_actor_id, limit], map_row)
            .map_err(|e| db_err("query notifications", e))?;
        for r in rows {
            notifications.push(r.map_err(|e| db_err("read notification", e))?);
        }
    }

    Ok(NotificationsListResponse {
        notifications,
        unread_count: unread_count.max(0) as u64,
    })
}

pub fn mark_notifications_read_tx(
    tx: &Transaction<'_>,
    recipient_actor_id: &str,
    notif_ids: Option<&[String]>,
) -> Result<u64> {
    let mut marked: u64 = 0;
    if let Some(ids) = notif_ids {
        for nid in ids {
            let changed = tx
                .execute(
                    "UPDATE community_notifications SET is_read = 1 WHERE recipient_actor_id = ?1 AND notif_id = ?2 AND is_read = 0",
                    params![recipient_actor_id, nid],
                )
                .map_err(|e| db_err("mark notif read by id", e))?;
            marked += changed as u64;
        }
    } else {
        let changed = tx
            .execute(
                "UPDATE community_notifications SET is_read = 1 WHERE recipient_actor_id = ?1 AND is_read = 0",
                params![recipient_actor_id],
            )
            .map_err(|e| db_err("mark all notifs read", e))?;
        marked = changed as u64;
    }
    Ok(marked)
}

pub fn get_trending_conn(conn: &Connection, now_ms: i64, limit: usize) -> Result<Vec<TrendingTopicDto>> {
    let cutoff = now_ms.saturating_sub(72 * 3_600_000_i64);
    let mut stmt = conn
        .prepare(
            "SELECT post_id FROM community_posts
             WHERE status = 'published' AND is_removed = 0 AND created_at_ms >= ?1
             ORDER BY created_at_ms DESC LIMIT 500",
        )
        .map_err(|e| db_err("prepare trending posts", e))?;
    let ids: Vec<String> = stmt
        .query_map(params![cutoff], |r| r.get::<_, String>(0))
        .map_err(|e| db_err("query trending posts", e))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| db_err("collect trending posts", e))?;
    let mut posts = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(p) = hydrate_post_row_conn(conn, &id, None)? {
            posts.push(p);
        }
    }
    Ok(TrendingEngine::compute(&posts, now_ms, limit))
}

pub fn check_and_record_bot_cooldown_tx(
    tx: &Transaction<'_>,
    bot_actor_id: &str,
    scope_key: &str,
    min_cooldown_ms: i64,
    max_per_hour: i64,
    now_ms: i64,
) -> Result<bool> {
    if max_per_hour <= 0 {
        return Ok(false);
    }
    let min_cooldown_ms = min_cooldown_ms.max(0);
    let existing: Option<(i64, i64, i64)> = tx
        .query_row(
            "SELECT last_replied_at_ms, hourly_window_start_ms, hourly_count
             FROM community_bot_cooldowns WHERE bot_actor_id = ?1 AND scope_key = ?2",
            params![bot_actor_id, scope_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| db_err("check bot cooldown", e))?;

    let (window_start, next_count) = match existing {
        Some((last_ts, win_start, count)) => {
            if min_cooldown_ms > 0 && (now_ms < last_ts || now_ms.saturating_sub(last_ts) < min_cooldown_ms) {
                return Ok(false);
            }
            if now_ms >= win_start && now_ms.saturating_sub(win_start) < 3_600_000 {
                if count >= max_per_hour {
                    return Ok(false);
                }
                (win_start, count.saturating_add(1))
            } else {
                (now_ms, 1)
            }
        }
        None => (now_ms, 1),
    };

    tx.execute(
        "INSERT INTO community_bot_cooldowns (bot_actor_id, scope_key, last_replied_at_ms, hourly_window_start_ms, hourly_count)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(bot_actor_id, scope_key) DO UPDATE SET
            last_replied_at_ms = excluded.last_replied_at_ms,
            hourly_window_start_ms = excluded.hourly_window_start_ms,
            hourly_count = excluded.hourly_count",
        params![bot_actor_id, scope_key, now_ms, window_start, next_count],
    )
    .map_err(|e| db_err("record bot cooldown", e))?;

    Ok(true)
}

pub fn get_feed_conn(
    conn: &Connection,
    viewer_actor_id: &str,
    req: &FeedRequestDto,
    now_ms: i64,
) -> Result<FeedResponseDto> {
    let effective_now = req.now_ms.unwrap_or(now_ms);
    let viewer_actor = get_actor_conn(conn, viewer_actor_id, None)?;
    let algo_prefs = get_algo_prefs_conn(conn, viewer_actor_id)?;

    // Build follow graph for 1-hop & 2-hop social signals
    let mut graph = ActorGraph::new();
    {
        let mut stmt = conn
            .prepare("SELECT follower_actor_id, target_actor_id FROM community_follows")
            .map_err(|e| db_err("load follows for feed", e))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| db_err("query follows", e))?;
        for r in rows {
            let (f, t) = r.map_err(|e| db_err("read follow", e))?;
            graph.follow(&f, &t);
        }
    }

    let mut viewer = ViewerContext {
        actor_id: viewer_actor_id.to_string(),
        actor_kind: viewer_actor.as_ref().map(|a| a.actor_kind).unwrap_or(ActorKind::Human),
        following_actor_ids: graph.following_of(viewer_actor_id),
        second_hop_weights: graph.second_hop_weights(viewer_actor_id),
        algo_prefs,
        now_ms: effective_now,
        ..Default::default()
    };

    // Joined communities
    {
        let mut stmt = conn
            .prepare(
                "SELECT c.slug FROM community_memberships m
                 JOIN communities c ON c.community_id = m.community_id
                 WHERE m.actor_id = ?1 AND m.role != 'banned'",
            )
            .map_err(|e| db_err("load joined communities", e))?;
        let rows = stmt
            .query_map(params![viewer_actor_id], |r| r.get::<_, String>(0))
            .map_err(|e| db_err("query joined communities", e))?;
        for r in rows {
            viewer.joined_communities.insert(r.map_err(|e| db_err("read slug", e))?.to_ascii_lowercase());
        }
    }

    // Mutes & blocks
    {
        let mut stmt = conn
            .prepare("SELECT target_kind, target_value, action FROM community_mutes_blocks WHERE owner_actor_id = ?1")
            .map_err(|e| db_err("load mutes/blocks", e))?;
        let rows = stmt
            .query_map(params![viewer_actor_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })
            .map_err(|e| db_err("query mutes/blocks", e))?;
        for r in rows {
            let (tk, val, act) = r.map_err(|e| db_err("read mute/block", e))?;
            match (tk.as_str(), act.as_str()) {
                ("actor", "block") => { viewer.blocked_actor_ids.insert(val); }
                ("actor", "mute") => { viewer.muted_actor_ids.insert(val); }
                ("community", _) => { viewer.muted_communities.insert(normalize_slug_key(&val)); }
                ("keyword", _) => { viewer.muted_keywords.push(val); }
                _ => {}
            }
        }
    }

    // Affinities
    {
        let mut stmt = conn
            .prepare("SELECT target_kind, target_key, score FROM community_actor_affinities WHERE viewer_actor_id = ?1")
            .map_err(|e| db_err("load affinities", e))?;
        let rows = stmt
            .query_map(params![viewer_actor_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?))
            })
            .map_err(|e| db_err("query affinities", e))?;
        for r in rows {
            let (tk, key, score) = r.map_err(|e| db_err("read affinity", e))?;
            match tk.as_str() {
                "author" => { viewer.author_affinities.insert(key, score); }
                "community" => {
                    let norm = normalize_slug_key(&key);
                    if !norm.is_empty() {
                        *viewer.community_affinities.entry(norm).or_insert(0.0) += score;
                    }
                }
                _ => {}
            }
        }
    }

    // Saved bookmarks & hidden posts & impression counts
    {
        let mut stmt = conn
            .prepare("SELECT post_id FROM community_bookmarks WHERE actor_id = ?1")
            .map_err(|e| db_err("load saved posts", e))?;
        for r in stmt.query_map(params![viewer_actor_id], |r| r.get::<_, String>(0)).map_err(|e| db_err("query saved", e))? {
            viewer.saved_post_ids.insert(r.map_err(|e| db_err("read saved", e))?);
        }
        let mut hstmt = conn
            .prepare(
                "SELECT post_id FROM community_user_signals
                 WHERE actor_id = ?1 AND signal_kind IN ('hide_post', 'not_interested') AND post_id IS NOT NULL",
            )
            .map_err(|e| db_err("load hidden posts", e))?;
        for r in hstmt.query_map(params![viewer_actor_id], |r| r.get::<_, String>(0)).map_err(|e| db_err("query hidden", e))? {
            viewer.hidden_post_ids.insert(r.map_err(|e| db_err("read hidden", e))?);
        }
        let mut istmt = conn
            .prepare(
                "SELECT post_id, COUNT(*) FROM community_user_signals
                 WHERE actor_id = ?1 AND signal_kind = 'impression' AND post_id IS NOT NULL
                 GROUP BY post_id",
            )
            .map_err(|e| db_err("load impressions", e))?;
        for r in istmt.query_map(params![viewer_actor_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).map_err(|e| db_err("query impressions", e))? {
            let (pid, cnt) = r.map_err(|e| db_err("read impression", e))?;
            viewer.seen_impressions.insert(pid, cnt.max(0) as u32);
        }
    }

    // Hydrate list definition when FeedTab::List(list_id) is requested (strict privacy enforcement!)
    if let FeedTab::List(ref list_id) = req.tab {
        if let Some(list) = conn
            .query_row(
                "SELECT list_id, owner_actor_id, name, description, kind, is_private, is_pinned, members_json, communities_json, created_at_ms, updated_at_ms
                 FROM community_lists
                 WHERE list_id = ?1 AND (is_private = 0 OR owner_actor_id = ?2)",
                params![list_id, viewer_actor_id],
                row_to_list,
            )
            .optional()
            .map_err(|e| db_err("load list for feed", e))?
        {
            viewer.list_member_actors = list.member_actor_ids.into_iter().collect();
            viewer.list_community_slugs = list
                .community_slugs
                .into_iter()
                .map(|s| normalize_slug_key(&s))
                .collect();
        }
    }

    // Build viewer interest embedding from recent upvoted / liked / bookmarked posts
    let mut interest_vec = [0.0_f32; 64];
    {
        let mut stmt = conn
            .prepare(
                "SELECT COALESCE(p.title, '') || ' ' || p.body_markdown
                 FROM community_posts p
                 WHERE p.post_id IN (
                     SELECT target_id FROM community_votes WHERE target_kind = 'post' AND actor_id = ?1 AND direction = 1
                     UNION
                     SELECT post_id FROM community_bookmarks WHERE actor_id = ?1
                 )
                 ORDER BY p.created_at_ms DESC LIMIT 12",
            )
            .map_err(|e| db_err("prepare interest texts", e))?;
        let rows = stmt
            .query_map(params![viewer_actor_id], |r| r.get::<_, String>(0))
            .map_err(|e| db_err("query interest texts", e))?;
        for r in rows {
            let txt = r.map_err(|e| db_err("read interest text", e))?;
            let emb = embed_text_64(&txt);
            interest_vec = blend_interest_vector(&interest_vec, &emb, 0.25);
        }
    }
    viewer.interest_embedding = interest_vec;

    // Fetch participant kinds per post so ActorFilter::HumanBotDebates is 100% accurate
    let mut participants: HashMap<String, (bool, bool)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT c.post_id, a.actor_kind
                 FROM community_comments c
                 JOIN community_actors a ON a.actor_id = c.author_actor_id
                 WHERE c.is_removed = 0 AND c.is_deleted = 0
                 GROUP BY c.post_id, a.actor_kind",
            )
            .map_err(|e| db_err("load comment participant kinds", e))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| db_err("query participant kinds", e))?;
        for r in rows {
            let (pid, k) = r.map_err(|e| db_err("read participant kind", e))?;
            let entry = participants.entry(pid).or_insert((false, false));
            if k == "human" {
                entry.0 = true;
            } else {
                entry.1 = true;
            }
        }
    }

    // Load trust scores map
    let mut trust_map: HashMap<String, f64> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT actor_id, trust_score FROM community_actors")
            .map_err(|e| db_err("load actor trust", e))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
            .map_err(|e| db_err("query actor trust", e))?;
        for r in rows {
            let (aid, t) = r.map_err(|e| db_err("read actor trust", e))?;
            trust_map.insert(aid, t);
        }
    }

    // Gather candidate posts
    let mut stmt = conn
        .prepare(
            "SELECT post_id FROM community_posts
             WHERE status != 'deleted' AND is_removed = 0
             ORDER BY created_at_ms DESC LIMIT 400",
        )
        .map_err(|e| db_err("load candidate post ids", e))?;
    let post_ids: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| db_err("query candidate post ids", e))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| db_err("collect candidate post ids", e))?;

    let mut candidates = Vec::with_capacity(post_ids.len());
    for pid in post_ids {
        if let Some(post) = hydrate_post_row_conn(conn, &pid, Some(viewer_actor_id))? {
            let mut cand = PostCandidate::from_post(post);
            if let Some(&trust) = trust_map.get(&cand.post.author_actor_id) {
                cand.author_trust_score = trust;
            }
            if let Some(&(has_h, has_b)) = participants.get(&cand.post.post_id) {
                cand.has_human_participants = cand.has_human_participants || has_h;
                cand.has_bot_participants = cand.has_bot_participants || has_b;
            }
            candidates.push(cand);
        }
    }

    let mut req_with_now = req.clone();
    if req_with_now.now_ms.is_none() {
        req_with_now.now_ms = Some(effective_now);
    }
    Ok(CommunityRecommender::rank_feed(&viewer, candidates, &req_with_now))
}

impl CommunityStore {
    pub fn list_lists(&self, owner_actor_id: &str) -> Result<Vec<CommunityListDto>> {
        self.with_conn(|conn| list_lists_conn(conn, owner_actor_id))
    }
    pub fn create_list(&self, owner_actor_id: &str, req: &CreateCommunityListRequest, now_ms: i64) -> Result<CommunityListDto> {
        self.with_transaction(|tx| create_list_tx(tx, owner_actor_id, req, now_ms))
    }
    pub fn update_list(&self, list_id: &str, actor_id: &str, patch: &UpdateCommunityListRequest, now_ms: i64) -> Result<CommunityListDto> {
        self.with_transaction(|tx| update_list_tx(tx, list_id, actor_id, patch, now_ms))
    }
    pub fn delete_list(&self, list_id: &str, actor_id: &str) -> Result<bool> {
        self.with_transaction(|tx| delete_list_tx(tx, list_id, actor_id))
    }
    pub fn record_signal(&self, actor_id: &str, req: &RecordSignalRequest, now_ms: i64) -> Result<RecordSignalResponse> {
        self.with_transaction(|tx| record_signal_tx(tx, actor_id, req, now_ms))
    }
    pub fn get_algo_prefs(&self, actor_id: &str) -> Result<AlgoPreferencesDto> {
        self.with_conn(|conn| get_algo_prefs_conn(conn, actor_id))
    }
    pub fn put_algo_prefs(&self, actor_id: &str, prefs: &AlgoPreferencesDto, now_ms: i64) -> Result<AlgoPreferencesDto> {
        self.with_transaction(|tx| put_algo_prefs_tx(tx, actor_id, prefs, now_ms))
    }
    pub fn list_notifications(&self, recipient_actor_id: &str, kind_filter: Option<CommunityNotificationKind>, limit: usize) -> Result<NotificationsListResponse> {
        self.with_conn(|conn| list_notifications_conn(conn, recipient_actor_id, kind_filter, limit))
    }
    pub fn mark_notifications_read(&self, recipient_actor_id: &str, notif_ids: Option<&[String]>) -> Result<u64> {
        self.with_transaction(|tx| mark_notifications_read_tx(tx, recipient_actor_id, notif_ids))
    }
    pub fn get_trending(&self, now_ms: i64, limit: usize) -> Result<Vec<TrendingTopicDto>> {
        self.with_conn(|conn| get_trending_conn(conn, now_ms, limit))
    }
    pub fn get_feed(&self, viewer_actor_id: &str, req: &FeedRequestDto, now_ms: i64) -> Result<FeedResponseDto> {
        self.with_conn(|conn| get_feed_conn(conn, viewer_actor_id, req, now_ms))
    }
    pub fn check_and_record_bot_cooldown(&self, bot_actor_id: &str, scope_key: &str, min_cooldown_ms: i64, max_per_hour: i64, now_ms: i64) -> Result<bool> {
        self.with_transaction(|tx| check_and_record_bot_cooldown_tx(tx, bot_actor_id, scope_key, min_cooldown_ms, max_per_hour, now_ms))
    }
    /// Pure-SQL search fallback (used by offline client builds where Tantivy `search` feature is not enabled).
    pub fn search_posts_sql(&self, query: &str, viewer_actor_id: Option<&str>, limit: usize) -> Result<Vec<CommunityPostDto>> {
        let parsed = parse_search_operators(query);
        let free_lower = parsed.free_text.to_lowercase();
        let terms: Vec<&str> = free_lower.split_whitespace().collect();
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT post_id FROM community_posts WHERE status = 'published' AND is_removed = 0 ORDER BY hot_score DESC LIMIT 400")
                .map_err(|e| db_err("prepare sql search", e))?;
            let ids: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| db_err("query sql search", e))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| db_err("collect sql search", e))?;
            let mut out = Vec::new();
            let seen_empty_query = terms.is_empty()
                && parsed.community.is_none()
                && parsed.author.is_none()
                && parsed.flair.is_none()
                && parsed.actor_kind.is_none()
                && parsed.has_media.is_none()
                && parsed.has_poll.is_none()
                && parsed.has_link.is_none();
            if seen_empty_query {
                return Ok(out);
            }
            for id in ids {
                let Some(p) = hydrate_post_row_conn(conn, &id, viewer_actor_id)? else { continue };
                if let Some(ref c) = parsed.community {
                    if !p.community_slug.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(c)) { continue; }
                }
                if let Some(ref a) = parsed.author {
                    if !p.author_handle.eq_ignore_ascii_case(a) { continue; }
                }
                if let Some(ref f) = parsed.flair {
                    if !p.flair.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(f)) { continue; }
                }
                if let Some(k) = parsed.actor_kind {
                    let ok = if k == ActorKind::Bot { p.author_kind.is_bot_like() } else { p.author_kind == k };
                    if !ok { continue; }
                }
                if parsed.has_media == Some(true) && p.media.is_empty() { continue; }
                if parsed.has_poll == Some(true) && p.poll.is_none() { continue; }
                if parsed.has_link == Some(true) {
                    let has_link = p.format == PostFormat::Link
                        || p.link_preview.is_some()
                        || !extract_entities(&p.body).urls.is_empty();
                    if !has_link {
                        continue;
                    }
                }
                if !terms.is_empty() {
                    let hay = format!("{} {} {} {}", p.title.as_deref().unwrap_or(""), p.body, p.tags.join(" "), p.author_handle).to_lowercase();
                    if !terms.iter().all(|t| hay.contains(t)) { continue; }
                }
                out.push(p);
                if out.len() >= limit.clamp(1, 100) {
                    break;
                }
            }
            Ok(out)
        })
    }
}
