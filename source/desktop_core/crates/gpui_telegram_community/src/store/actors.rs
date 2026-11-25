//! SQLite operations for First-Class Human & Bot Actors, Telegram Identity Sync, Follows, and Mutes/Blocks.

use super::{CommunityStore, db_err};
use crate::identity::{
    bot_actor_id, compute_trust_score, human_actor_id, new_bot_actor, new_human_actor,
    normalize_handle,
};
use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, BotAutonomyMode, CommunityActorDto, CommunityMuteBlockDto,
    CommunityNotificationKind, MuteBlockActionKind, MuteBlockRequest, MuteBlockTargetKind,
    RegisterBotActorRequest, RegisterHumanActorRequest, UpdateCommunityActorRequest, VerifiedTier,
};
use gpui_telegram_proto::miniapp_contracts::{
    CLASSIC_INITIALS_AVATAR_SHAPE, resolve_grok_persona_color, resolve_grok_persona_shape,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use uuid::Uuid;

/// Input descriptor for syncing a Telegram human user into `community_actors`.
#[derive(Debug, Clone)]
pub struct TelegramHumanSyncInput {
    pub user_id: i64,
    pub username: Option<String>,
    pub display_name: String,
    pub bio: Option<String>,
    pub avatar_media_id: Option<String>,
}

/// Input descriptor for syncing a Telegram owned bot into `community_actors`.
#[derive(Debug, Clone)]
pub struct TelegramBotSyncInput {
    pub bot_user_id: i64,
    pub username: String,
    pub display_name: String,
    pub bio: Option<String>,
    pub avatar_media_id: Option<String>,
    pub avatar_shape: Option<String>,
    pub avatar_color: Option<String>,
    pub bot_model: Option<String>,
    pub bot_capabilities: Vec<String>,
    pub bot_persona: Option<String>,
}

pub(crate) const ACTOR_SELECT_COLS: &str = "
    actor_id, linked_user_id, owner_user_id, actor_kind, handle, display_name, bio,
    location, website, avatar_media_id, banner_media_id, avatar_shape, avatar_color,
    verified_tier, bot_badge, bot_model, bot_capabilities_json, bot_autonomy_mode, bot_persona,
    post_karma, comment_karma, awardee_karma, followers_count, following_count, posts_count,
    trust_score, created_at_ms
";

pub(crate) fn row_to_actor(row: &Row<'_>) -> rusqlite::Result<CommunityActorDto> {
    let kind_str: String = row.get(3)?;
    let tier_str: String = row.get(13)?;
    let caps_json: String = row.get(16)?;
    let mode_str: String = row.get(17)?;
    let caps: Vec<String> = serde_json::from_str(&caps_json).unwrap_or_default();
    Ok(CommunityActorDto {
        actor_id: row.get(0)?,
        linked_user_id: row.get(1)?,
        owner_user_id: row.get(2)?,
        actor_kind: ActorKind::parse(&kind_str).unwrap_or(ActorKind::Human),
        handle: row.get(4)?,
        display_name: row.get(5)?,
        bio: row.get(6)?,
        location: row.get(7)?,
        website: row.get(8)?,
        avatar_media_id: row.get(9)?,
        banner_media_id: row.get(10)?,
        avatar_shape: row.get(11)?,
        avatar_color: row.get(12)?,
        verified_tier: VerifiedTier::parse(&tier_str).unwrap_or(VerifiedTier::None),
        bot_badge: row.get(14)?,
        bot_model: row.get(15)?,
        bot_capabilities: caps,
        bot_autonomy_mode: BotAutonomyMode::parse(&mode_str).unwrap_or(BotAutonomyMode::Manual),
        bot_persona: row.get(18)?,
        post_karma: row.get(19)?,
        comment_karma: row.get(20)?,
        awardee_karma: row.get(21)?,
        followers_count: row.get::<_, i64>(22)?.max(0) as u64,
        following_count: row.get::<_, i64>(23)?.max(0) as u64,
        posts_count: row.get::<_, i64>(24)?.max(0) as u64,
        trust_score: row.get(25)?,
        is_following: false,
        is_muted: false,
        is_blocked: false,
        created_at_ms: row.get(26)?,
    })
}

pub(crate) fn hydrate_actor_viewer_flags(
    conn: &Connection,
    actor: &mut CommunityActorDto,
    viewer_actor_id: Option<&str>,
) -> Result<()> {
    let Some(viewer_id) = viewer_actor_id else {
        return Ok(());
    };
    if viewer_id == actor.actor_id {
        return Ok(());
    }
    let following: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM community_follows WHERE follower_actor_id = ?1 AND target_actor_id = ?2",
            params![viewer_id, actor.actor_id],
            |r| r.get(0),
        )
        .map_err(|e| db_err("check following", e))?;
    actor.is_following = following > 0;

    let mut stmt = conn
        .prepare(
            "SELECT action FROM community_mutes_blocks
             WHERE owner_actor_id = ?1 AND target_kind = 'actor' AND target_value = ?2",
        )
        .map_err(|e| db_err("prepare mute/block check", e))?;
    let actions = stmt
        .query_map(params![viewer_id, actor.actor_id], |r| r.get::<_, String>(0))
        .map_err(|e| db_err("query mute/block check", e))?;
    for a in actions {
        match a.map_err(|e| db_err("read mute/block", e))?.as_str() {
            "mute" => actor.is_muted = true,
            "block" => actor.is_blocked = true,
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn insert_notification_conn(
    conn: &Connection,
    recipient_actor_id: &str,
    actor_id: &str,
    kind: CommunityNotificationKind,
    target_post_id: Option<&str>,
    target_comment_id: Option<&str>,
    preview_text: &str,
    now_ms: i64,
) -> Result<()> {
    if recipient_actor_id == actor_id || recipient_actor_id.is_empty() || actor_id.is_empty() {
        return Ok(());
    }
    let nid = format!("notif_{}", Uuid::new_v4().simple());
    conn.execute(
        "INSERT INTO community_notifications (notif_id, recipient_actor_id, actor_id, kind, target_post_id, target_comment_id, preview_text, is_read, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
        params![
            nid,
            recipient_actor_id,
            actor_id,
            kind.as_str(),
            target_post_id,
            target_comment_id,
            preview_text,
            now_ms,
        ],
    )
    .map_err(|e| db_err("insert notification", e))?;
    Ok(())
}

pub(crate) fn bump_affinity_conn(
    conn: &Connection,
    viewer_actor_id: &str,
    target_kind: &str,
    target_key: &str,
    delta: f64,
    now_ms: i64,
) -> Result<()> {
    if viewer_actor_id.is_empty()
        || target_key.is_empty()
        || !delta.is_finite()
        || delta.abs() < 1e-9
    {
        return Ok(());
    }
    let clamped_delta = delta.clamp(-50.0, 100.0);
    conn.execute(
        "INSERT INTO community_actor_affinities (viewer_actor_id, target_kind, target_key, score, interaction_count, last_interacted_ms)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)
         ON CONFLICT(viewer_actor_id, target_kind, target_key) DO UPDATE SET
            score = MAX(-50.0, MIN(100.0, community_actor_affinities.score + excluded.score)),
            interaction_count = community_actor_affinities.interaction_count + 1,
            last_interacted_ms = excluded.last_interacted_ms",
        params![viewer_actor_id, target_kind, target_key, clamped_delta, now_ms],
    )
    .map_err(|e| db_err("bump affinity", e))?;
    Ok(())
}

pub fn upsert_actor_conn(conn: &Connection, actor: &CommunityActorDto, now_ms: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO community_actors (
            actor_id, linked_user_id, owner_user_id, actor_kind, handle, display_name, bio,
            location, website, avatar_media_id, banner_media_id, avatar_shape, avatar_color,
            verified_tier, bot_badge, bot_model, bot_capabilities_json, bot_autonomy_mode, bot_persona,
            post_karma, comment_karma, awardee_karma, followers_count, following_count, posts_count,
            trust_score, created_at_ms, updated_at_ms
         ) VALUES (
            ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28
         )
         ON CONFLICT(actor_id) DO UPDATE SET
            display_name = excluded.display_name,
            bio = excluded.bio,
            location = excluded.location,
            website = excluded.website,
            avatar_media_id = COALESCE(excluded.avatar_media_id, community_actors.avatar_media_id),
            banner_media_id = COALESCE(excluded.banner_media_id, community_actors.banner_media_id),
            avatar_shape = excluded.avatar_shape,
            avatar_color = excluded.avatar_color,
            bot_badge = COALESCE(excluded.bot_badge, community_actors.bot_badge),
            bot_model = COALESCE(excluded.bot_model, community_actors.bot_model),
            bot_capabilities_json = excluded.bot_capabilities_json,
            bot_autonomy_mode = excluded.bot_autonomy_mode,
            bot_persona = COALESCE(excluded.bot_persona, community_actors.bot_persona),
            trust_score = excluded.trust_score,
            updated_at_ms = excluded.updated_at_ms",
        params![
            actor.actor_id,
            actor.linked_user_id,
            actor.owner_user_id,
            actor.actor_kind.as_str(),
            actor.handle,
            actor.display_name,
            actor.bio,
            actor.location,
            actor.website,
            actor.avatar_media_id,
            actor.banner_media_id,
            actor.avatar_shape,
            actor.avatar_color,
            actor.verified_tier.as_str(),
            actor.bot_badge,
            actor.bot_model,
            serde_json::to_string(&actor.bot_capabilities)?,
            actor.bot_autonomy_mode.as_str(),
            actor.bot_persona,
            actor.post_karma,
            actor.comment_karma,
            actor.awardee_karma,
            actor.followers_count as i64,
            actor.following_count as i64,
            actor.posts_count as i64,
            actor.trust_score,
            actor.created_at_ms,
            now_ms,
        ],
    )
    .map_err(|e| db_err("upsert actor", e))?;
    Ok(())
}

fn resolve_unique_handle(conn: &Connection, desired: &str, actor_id: &str, suffix_num: i64) -> Result<String> {
    let fallback_num = suffix_num.unsigned_abs();
    let base = normalize_handle(desired).unwrap_or_else(|_| {
        let alnum: String = desired
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(20)
            .collect::<String>()
            .to_ascii_lowercase();
        if alnum.len() >= 2 && !alnum.chars().all(|c| c == '_') {
            alnum
        } else {
            format!("user_{fallback_num}")
        }
    });

    let is_vacant = |candidate: &str| -> Result<bool> {
        let owner: Option<String> = conn
            .query_row(
                "SELECT actor_id FROM community_actors WHERE handle = ?1 COLLATE NOCASE",
                params![candidate],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| db_err("check handle uniqueness", e))?;
        Ok(owner.is_none() || owner.as_deref() == Some(actor_id))
    };

    if is_vacant(&base)? {
        return Ok(base);
    }

    let primary_suffix = format!("_{fallback_num}");
    let prefix_max = 32usize.saturating_sub(primary_suffix.len());
    let primary_candidate = format!("{}{primary_suffix}", &base[..base.len().min(prefix_max)]);
    if is_vacant(&primary_candidate)? {
        return Ok(primary_candidate);
    }

    for attempt in 2u64..10_000 {
        let suffix = format!("_{fallback_num}_{attempt}");
        let pmax = 32usize.saturating_sub(suffix.len());
        let candidate = format!("{}{suffix}", &base[..base.len().min(pmax)]);
        if is_vacant(&candidate)? {
            return Ok(candidate);
        }
    }

    let short = Uuid::new_v4().simple().to_string();
    Ok(format!("u_{}", &short[..12]))
}

pub fn get_actor_conn(
    conn: &Connection,
    handle_or_id: &str,
    viewer_actor_id: Option<&str>,
) -> Result<Option<CommunityActorDto>> {
    let cleaned = handle_or_id.trim().trim_start_matches('@');
    let sql = format!(
        "SELECT {ACTOR_SELECT_COLS} FROM community_actors
         WHERE actor_id = ?1 OR handle = ?2 COLLATE NOCASE LIMIT 1"
    );
    let mut actor = conn
        .query_row(&sql, params![handle_or_id.trim(), cleaned], row_to_actor)
        .optional()
        .map_err(|e| db_err("get actor", e))?;
    if let Some(ref mut a) = actor {
        hydrate_actor_viewer_flags(conn, a, viewer_actor_id)?;
    }
    Ok(actor)
}

pub fn list_identities_for_user_conn(conn: &Connection, user_id: i64) -> Result<Vec<CommunityActorDto>> {
    let sql = format!(
        "SELECT {ACTOR_SELECT_COLS} FROM community_actors
         WHERE linked_user_id = ?1 OR owner_user_id = ?1
         ORDER BY CASE actor_kind WHEN 'human' THEN 0 ELSE 1 END, created_at_ms ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| db_err("prepare identities", e))?;
    let rows = stmt
        .query_map(params![user_id], row_to_actor)
        .map_err(|e| db_err("query identities", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| db_err("read identity row", e))?);
    }
    Ok(out)
}

pub fn list_actors_conn(
    conn: &Connection,
    kind_filter: Option<ActorKind>,
    limit: usize,
) -> Result<Vec<CommunityActorDto>> {
    let limit = limit.clamp(1, 200) as i64;
    let mut out = Vec::new();
    if let Some(kind) = kind_filter {
        let sql = format!(
            "SELECT {ACTOR_SELECT_COLS} FROM community_actors WHERE actor_kind = ?1
             ORDER BY (post_karma + comment_karma) DESC, followers_count DESC LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| db_err("prepare list actors", e))?;
        let rows = stmt
            .query_map(params![kind.as_str(), limit], row_to_actor)
            .map_err(|e| db_err("query list actors", e))?;
        for r in rows {
            out.push(r.map_err(|e| db_err("read actor", e))?);
        }
    } else {
        let sql = format!(
            "SELECT {ACTOR_SELECT_COLS} FROM community_actors
             ORDER BY (post_karma + comment_karma) DESC, followers_count DESC LIMIT ?1"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| db_err("prepare list all actors", e))?;
        let rows = stmt
            .query_map(params![limit], row_to_actor)
            .map_err(|e| db_err("query list all actors", e))?;
        for r in rows {
            out.push(r.map_err(|e| db_err("read actor", e))?);
        }
    }
    Ok(out)
}

pub fn sync_telegram_identities_tx(
    tx: &Transaction<'_>,
    human: &TelegramHumanSyncInput,
    owned_bots: &[TelegramBotSyncInput],
    now_ms: i64,
) -> Result<Vec<CommunityActorDto>> {
    let hid = human_actor_id(human.user_id);
    if get_actor_conn(tx, &hid, None)?.is_none() {
        let preferred = human
            .username
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&human.display_name);
        let handle = resolve_unique_handle(tx, preferred, &hid, human.user_id)?;
        let req = RegisterHumanActorRequest {
            handle,
            display_name: human.display_name.clone(),
            bio: human.bio.clone(),
            location: None,
            website: None,
            avatar_media_id: human.avatar_media_id.clone(),
            banner_media_id: None,
            avatar_shape: Some(CLASSIC_INITIALS_AVATAR_SHAPE.to_string()),
            avatar_color: None,
        };
        let dto = new_human_actor(human.user_id, &req, now_ms)?;
        upsert_actor_conn(tx, &dto, now_ms)?;
    }

    for bot in owned_bots {
        let bid = bot_actor_id(bot.bot_user_id);
        if get_actor_conn(tx, &bid, None)?.is_none() {
            let handle = resolve_unique_handle(tx, &bot.username, &bid, bot.bot_user_id)?;
            let req = RegisterBotActorRequest {
                bot_user_id: Some(bot.bot_user_id),
                actor_kind: Some(ActorKind::Bot),
                handle,
                display_name: bot.display_name.clone(),
                bio: bot.bio.clone(),
                avatar_media_id: bot.avatar_media_id.clone(),
                banner_media_id: None,
                avatar_shape: bot.avatar_shape.clone(),
                avatar_color: bot.avatar_color.clone(),
                bot_badge: Some("AI BOT".to_string()),
                bot_model: bot.bot_model.clone(),
                bot_capabilities: bot.bot_capabilities.clone(),
                bot_autonomy_mode: BotAutonomyMode::Reactive,
                bot_persona: bot.bot_persona.clone(),
                cooldown_secs: None,
                max_posts_per_hour: None,
            };
            let dto = new_bot_actor(human.user_id, bot.bot_user_id, &req, now_ms)?;
            upsert_actor_conn(tx, &dto, now_ms)?;
        }
    }

    list_identities_for_user_conn(tx, human.user_id)
}

pub fn update_actor_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    patch: &UpdateCommunityActorRequest,
    now_ms: i64,
) -> Result<CommunityActorDto> {
    let mut actor = get_actor_conn(tx, actor_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("actor '{actor_id}' not found")))?;
    if let Some(dn) = patch.display_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        actor.display_name = dn.to_string();
    }
    if let Some(bio) = patch.bio.as_deref() {
        actor.bio = bio.trim().to_string();
    }
    if let Some(loc) = patch.location.as_deref() {
        actor.location = (!loc.trim().is_empty()).then(|| loc.trim().to_string());
    }
    if let Some(web) = patch.website.as_deref() {
        actor.website = (!web.trim().is_empty()).then(|| web.trim().to_string());
    }
    if let Some(av) = patch.avatar_media_id.as_deref() {
        actor.avatar_media_id = (!av.trim().is_empty()).then(|| av.trim().to_string());
    }
    if let Some(bn) = patch.banner_media_id.as_deref() {
        actor.banner_media_id = (!bn.trim().is_empty()).then(|| bn.trim().to_string());
    }
    if let Some(sh) = patch.avatar_shape.as_deref() {
        actor.avatar_shape = resolve_grok_persona_shape(&actor.actor_id, Some(sh));
    }
    if let Some(col) = patch.avatar_color.as_deref() {
        actor.avatar_color = resolve_grok_persona_color(&actor.actor_id, Some(col));
    }
    if let Some(badge) = patch.bot_badge.as_deref() {
        actor.bot_badge = (!badge.trim().is_empty()).then(|| badge.trim().to_string());
    }
    if let Some(model) = patch.bot_model.as_deref() {
        actor.bot_model = (!model.trim().is_empty()).then(|| model.trim().to_string());
    }
    if let Some(caps) = patch.bot_capabilities.as_ref() {
        actor.bot_capabilities = caps
            .iter()
            .map(|c| c.trim().to_ascii_lowercase())
            .filter(|c| !c.is_empty())
            .collect();
    }
    if let Some(mode) = patch.bot_autonomy_mode {
        actor.bot_autonomy_mode = mode;
    }
    if let Some(persona) = patch.bot_persona.as_deref() {
        actor.bot_persona = (!persona.trim().is_empty()).then(|| persona.trim().to_string());
    }
    actor.trust_score = compute_trust_score(&actor, now_ms);
    // Explicit UPDATE without COALESCE so clearing optional fields via Some("") persists NULL.
    tx.execute(
        "UPDATE community_actors SET
            display_name = ?2,
            bio = ?3,
            location = ?4,
            website = ?5,
            avatar_media_id = ?6,
            banner_media_id = ?7,
            avatar_shape = ?8,
            avatar_color = ?9,
            bot_badge = ?10,
            bot_model = ?11,
            bot_capabilities_json = ?12,
            bot_autonomy_mode = ?13,
            bot_persona = ?14,
            trust_score = ?15,
            updated_at_ms = ?16
         WHERE actor_id = ?1",
        params![
            actor.actor_id,
            actor.display_name,
            actor.bio,
            actor.location,
            actor.website,
            actor.avatar_media_id,
            actor.banner_media_id,
            actor.avatar_shape,
            actor.avatar_color,
            actor.bot_badge,
            actor.bot_model,
            serde_json::to_string(&actor.bot_capabilities)?,
            actor.bot_autonomy_mode.as_str(),
            actor.bot_persona,
            actor.trust_score,
            now_ms,
        ],
    )
    .map_err(|e| db_err("update actor", e))?;
    Ok(actor)
}

pub fn follow_actor_tx(
    tx: &Transaction<'_>,
    follower_id: &str,
    target_id: &str,
    now_ms: i64,
) -> Result<bool> {
    if follower_id == target_id {
        return Err(TelegramError::Validation("cannot follow yourself".to_string()));
    }
    let follower = get_actor_conn(tx, follower_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("follower '{follower_id}' not found")))?;
    if get_actor_conn(tx, target_id, None)?.is_none() {
        return Err(TelegramError::NotFound(format!("target actor '{target_id}' not found")));
    }
    let changed = tx
        .execute(
            "INSERT OR IGNORE INTO community_follows (follower_actor_id, target_actor_id, created_at_ms)
             VALUES (?1, ?2, ?3)",
            params![follower_id, target_id, now_ms],
        )
        .map_err(|e| db_err("insert follow", e))?;
    if changed == 1 {
        tx.execute(
            "UPDATE community_actors SET followers_count = followers_count + 1, updated_at_ms = ?2 WHERE actor_id = ?1",
            params![target_id, now_ms],
        )
        .map_err(|e| db_err("inc followers_count", e))?;
        tx.execute(
            "UPDATE community_actors SET following_count = following_count + 1, updated_at_ms = ?2 WHERE actor_id = ?1",
            params![follower_id, now_ms],
        )
        .map_err(|e| db_err("inc following_count", e))?;
        bump_affinity_conn(tx, follower_id, "author", target_id, 4.0, now_ms)?;
        insert_notification_conn(
            tx,
            target_id,
            follower_id,
            CommunityNotificationKind::Follow,
            None,
            None,
            &format!("@{} started following you.", follower.handle),
            now_ms,
        )?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn unfollow_actor_tx(tx: &Transaction<'_>, follower_id: &str, target_id: &str) -> Result<bool> {
    let changed = tx
        .execute(
            "DELETE FROM community_follows WHERE follower_actor_id = ?1 AND target_actor_id = ?2",
            params![follower_id, target_id],
        )
        .map_err(|e| db_err("delete follow", e))?;
    if changed == 1 {
        tx.execute(
            "UPDATE community_actors SET followers_count = MAX(0, followers_count - 1) WHERE actor_id = ?1",
            params![target_id],
        )
        .map_err(|e| db_err("dec followers_count", e))?;
        tx.execute(
            "UPDATE community_actors SET following_count = MAX(0, following_count - 1) WHERE actor_id = ?1",
            params![follower_id],
        )
        .map_err(|e| db_err("dec following_count", e))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn set_mute_block_tx(
    tx: &Transaction<'_>,
    owner_actor_id: &str,
    req: &MuteBlockRequest,
    now_ms: i64,
) -> Result<CommunityMuteBlockDto> {
    let val = req.target_value.trim();
    if val.is_empty() {
        return Err(TelegramError::Validation("target_value cannot be empty".to_string()));
    }
    let norm_val = match req.target_kind {
        MuteBlockTargetKind::Actor => val.to_string(),
        MuteBlockTargetKind::Community => {
            let lower = val.to_ascii_lowercase();
            lower
                .strip_prefix("c/")
                .or_else(|| lower.strip_prefix("r/"))
                .unwrap_or(&lower)
                .to_string()
        }
        MuteBlockTargetKind::Keyword => val.to_lowercase(),
    };
    let id = format!("mb_{}", Uuid::new_v4().simple());
    tx.execute(
        "INSERT INTO community_mutes_blocks (id, owner_actor_id, target_kind, target_value, action, reason, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(owner_actor_id, target_kind, target_value, action) DO UPDATE SET
            reason = excluded.reason,
            created_at_ms = excluded.created_at_ms",
        params![
            id,
            owner_actor_id,
            req.target_kind.as_str(),
            norm_val,
            req.action.as_str(),
            req.reason,
            now_ms,
        ],
    )
    .map_err(|e| db_err("upsert mute/block", e))?;

    if req.action == MuteBlockActionKind::Block && req.target_kind == MuteBlockTargetKind::Actor {
        let _ = unfollow_actor_tx(tx, owner_actor_id, &norm_val)?;
        let _ = unfollow_actor_tx(tx, &norm_val, owner_actor_id)?;
    }

    Ok(CommunityMuteBlockDto {
        id,
        owner_actor_id: owner_actor_id.to_string(),
        target_kind: req.target_kind,
        target_value: norm_val,
        action: req.action,
        reason: req.reason.clone(),
        created_at_ms: now_ms,
    })
}

pub fn remove_mute_block_tx(
    tx: &Transaction<'_>,
    owner_actor_id: &str,
    target_kind: MuteBlockTargetKind,
    target_value: &str,
    action: MuteBlockActionKind,
) -> Result<bool> {
    let norm_val = match target_kind {
        MuteBlockTargetKind::Actor => target_value.trim().to_string(),
        MuteBlockTargetKind::Community => {
            let lower = target_value.trim().to_ascii_lowercase();
            lower
                .strip_prefix("c/")
                .or_else(|| lower.strip_prefix("r/"))
                .unwrap_or(&lower)
                .to_string()
        }
        MuteBlockTargetKind::Keyword => target_value.trim().to_lowercase(),
    };
    let changed = tx
        .execute(
            "DELETE FROM community_mutes_blocks
             WHERE owner_actor_id = ?1 AND target_kind = ?2 AND target_value = ?3 AND action = ?4",
            params![owner_actor_id, target_kind.as_str(), norm_val, action.as_str()],
        )
        .map_err(|e| db_err("remove mute/block", e))?;
    Ok(changed > 0)
}

pub fn list_mutes_blocks_conn(
    conn: &Connection,
    owner_actor_id: &str,
) -> Result<Vec<CommunityMuteBlockDto>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, owner_actor_id, target_kind, target_value, action, reason, created_at_ms
             FROM community_mutes_blocks WHERE owner_actor_id = ?1 ORDER BY created_at_ms DESC",
        )
        .map_err(|e| db_err("prepare mutes_blocks", e))?;
    let rows = stmt
        .query_map(params![owner_actor_id], |r| {
            let tk: String = r.get(2)?;
            let ac: String = r.get(4)?;
            Ok(CommunityMuteBlockDto {
                id: r.get(0)?,
                owner_actor_id: r.get(1)?,
                target_kind: MuteBlockTargetKind::parse(&tk).unwrap_or(MuteBlockTargetKind::Actor),
                target_value: r.get(3)?,
                action: MuteBlockActionKind::parse(&ac).unwrap_or(MuteBlockActionKind::Mute),
                reason: r.get(5)?,
                created_at_ms: r.get(6)?,
            })
        })
        .map_err(|e| db_err("query mutes_blocks", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| db_err("read mute_block", e))?);
    }
    Ok(out)
}

impl CommunityStore {
    pub fn upsert_actor(&self, actor: &CommunityActorDto, now_ms: i64) -> Result<CommunityActorDto> {
        self.with_transaction(|tx| {
            upsert_actor_conn(tx, actor, now_ms)?;
            get_actor_conn(tx, &actor.actor_id, None)?
                .ok_or_else(|| TelegramError::Internal("upserted actor missing".to_string()))
        })
    }

    pub fn create_human_actor(
        &self,
        user_id: i64,
        req: &RegisterHumanActorRequest,
        now_ms: i64,
    ) -> Result<CommunityActorDto> {
        let dto = new_human_actor(user_id, req, now_ms)?;
        self.upsert_actor(&dto, now_ms)
    }

    pub fn create_bot_actor(
        &self,
        owner_user_id: i64,
        bot_user_id: i64,
        req: &RegisterBotActorRequest,
        now_ms: i64,
    ) -> Result<CommunityActorDto> {
        let dto = new_bot_actor(owner_user_id, bot_user_id, req, now_ms)?;
        self.upsert_actor(&dto, now_ms)
    }

    pub fn sync_telegram_identities(
        &self,
        human: &TelegramHumanSyncInput,
        owned_bots: &[TelegramBotSyncInput],
        now_ms: i64,
    ) -> Result<Vec<CommunityActorDto>> {
        self.with_transaction(|tx| sync_telegram_identities_tx(tx, human, owned_bots, now_ms))
    }

    pub fn get_actor(
        &self,
        handle_or_id: &str,
        viewer_actor_id: Option<&str>,
    ) -> Result<Option<CommunityActorDto>> {
        self.with_conn(|conn| get_actor_conn(conn, handle_or_id, viewer_actor_id))
    }

    pub fn list_identities_for_user(&self, user_id: i64) -> Result<Vec<CommunityActorDto>> {
        self.with_conn(|conn| list_identities_for_user_conn(conn, user_id))
    }

    pub fn list_actors(&self, kind_filter: Option<ActorKind>, limit: usize) -> Result<Vec<CommunityActorDto>> {
        self.with_conn(|conn| list_actors_conn(conn, kind_filter, limit))
    }

    pub fn update_actor(
        &self,
        actor_id: &str,
        patch: &UpdateCommunityActorRequest,
        now_ms: i64,
    ) -> Result<CommunityActorDto> {
        self.with_transaction(|tx| update_actor_tx(tx, actor_id, patch, now_ms))
    }

    pub fn follow_actor(&self, follower_id: &str, target_id: &str, now_ms: i64) -> Result<bool> {
        self.with_transaction(|tx| follow_actor_tx(tx, follower_id, target_id, now_ms))
    }

    pub fn unfollow_actor(&self, follower_id: &str, target_id: &str) -> Result<bool> {
        self.with_transaction(|tx| unfollow_actor_tx(tx, follower_id, target_id))
    }

    pub fn set_mute_block(
        &self,
        owner_actor_id: &str,
        req: &MuteBlockRequest,
        now_ms: i64,
    ) -> Result<CommunityMuteBlockDto> {
        self.with_transaction(|tx| set_mute_block_tx(tx, owner_actor_id, req, now_ms))
    }

    pub fn remove_mute_block(
        &self,
        owner_actor_id: &str,
        target_kind: MuteBlockTargetKind,
        target_value: &str,
        action: MuteBlockActionKind,
    ) -> Result<bool> {
        self.with_transaction(|tx| {
            remove_mute_block_tx(tx, owner_actor_id, target_kind, target_value, action)
        })
    }

    pub fn list_mutes_blocks(&self, owner_actor_id: &str) -> Result<Vec<CommunityMuteBlockDto>> {
        self.with_conn(|conn| list_mutes_blocks_conn(conn, owner_actor_id))
    }
}
