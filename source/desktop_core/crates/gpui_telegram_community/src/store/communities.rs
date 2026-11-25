//! SQLite operations for Communities (`c/<slug>`), Memberships, Versioned Wiki, Mod Queue, and Mod Logs.

use super::actors::{bump_affinity_conn, get_actor_conn, insert_notification_conn};
use super::{CommunityStore, db_err};
use crate::model::normalize_community_slug;
use crate::moderation::ModPermissionValidator;
use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, CommunityActorDto, CommunityMemberRole, CommunityNotificationKind,
    CommunityWikiRevisionDto, CreateCommunityRequest, ModActionKind, ModActionRequest,
    ModLogEntryDto, ModQueueItemDto, ModQueueStatus, SubCommunityDto, UpdateCommunityRequest,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use uuid::Uuid;

const COMM_SELECT_COLS: &str = "
    community_id, slug, name, description, category, icon_emoji, icon_media_id, banner_media_id,
    accent_color, creator_actor_id, rules_json, post_flairs_json, user_flairs_json,
    automod_rules_json, pinned_post_ids_json, wiki_markdown, wiki_version,
    allow_bots, require_bot_disclosure, is_nsfw, members_count, posts_count, created_at_ms
";

fn row_to_community(row: &Row<'_>) -> rusqlite::Result<SubCommunityDto> {
    let rules_json: String = row.get(10)?;
    let post_flairs_json: String = row.get(11)?;
    let user_flairs_json: String = row.get(12)?;
    let automod_json: String = row.get(13)?;
    let pinned_json: String = row.get(14)?;
    Ok(SubCommunityDto {
        community_id: row.get(0)?,
        slug: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        category: row.get(4)?,
        icon_emoji: row.get(5)?,
        icon_media_id: row.get(6)?,
        banner_media_id: row.get(7)?,
        accent_color: row.get(8)?,
        created_by_actor_id: row.get(9)?,
        rules: serde_json::from_str(&rules_json).unwrap_or_default(),
        post_flairs: serde_json::from_str(&post_flairs_json).unwrap_or_default(),
        user_flairs: serde_json::from_str(&user_flairs_json).unwrap_or_default(),
        automod_rules: serde_json::from_str(&automod_json).unwrap_or_default(),
        pinned_post_ids: serde_json::from_str(&pinned_json).unwrap_or_default(),
        wiki_markdown: row.get(15)?,
        wiki_revision: row.get::<_, i64>(16)?.max(1) as u32,
        allow_bots: row.get::<_, i64>(17)? != 0,
        require_bot_disclosure: row.get::<_, i64>(18)? != 0,
        is_nsfw: row.get::<_, i64>(19)? != 0,
        member_count: row.get::<_, i64>(20)?.max(0) as u64,
        human_member_count: 0,
        bot_member_count: 0,
        posts_count: row.get::<_, i64>(21)?.max(0) as u64,
        viewer_role: None,
        is_joined: false,
        notifications_enabled: false,
        created_at_ms: row.get(22)?,
    })
}

fn hydrate_community_counts_and_viewer(
    conn: &Connection,
    comm: &mut SubCommunityDto,
    viewer_actor_id: Option<&str>,
) -> Result<()> {
    let (humans, bots): (i64, i64) = conn
        .query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN a.actor_kind = 'human' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN a.actor_kind != 'human' THEN 1 ELSE 0 END), 0)
             FROM community_memberships m
             JOIN community_actors a ON a.actor_id = m.actor_id
             WHERE m.community_id = ?1 AND m.role != 'banned'",
            params![comm.community_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| db_err("hydrate community citizen counts", e))?;
    comm.human_member_count = humans.max(0) as u64;
    comm.bot_member_count = bots.max(0) as u64;
    let live_total = comm.human_member_count + comm.bot_member_count;
    if live_total > 0 || comm.member_count == 0 {
        comm.member_count = live_total;
    }

    if let Some(vid) = viewer_actor_id {
        let membership: Option<(String, i64)> = conn
            .query_row(
                "SELECT role, notifications_enabled FROM community_memberships
                 WHERE community_id = ?1 AND actor_id = ?2",
                params![comm.community_id, vid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| db_err("check viewer community role", e))?;
        if let Some((role_str, notif)) = membership {
            let role = CommunityMemberRole::parse(&role_str).unwrap_or(CommunityMemberRole::Member);
            comm.viewer_role = Some(role);
            comm.is_joined = role != CommunityMemberRole::Banned;
            comm.notifications_enabled = notif != 0;
        }
    }
    Ok(())
}

pub(crate) fn get_member_role_conn(
    conn: &Connection,
    community_id: &str,
    actor_id: &str,
) -> Result<Option<CommunityMemberRole>> {
    let role_str: Option<String> = conn
        .query_row(
            "SELECT role FROM community_memberships WHERE community_id = ?1 AND actor_id = ?2",
            params![community_id, actor_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| db_err("get member role", e))?;
    Ok(role_str.as_deref().and_then(CommunityMemberRole::parse))
}

pub fn get_community_conn(
    conn: &Connection,
    slug_or_id: &str,
    viewer_actor_id: Option<&str>,
) -> Result<Option<SubCommunityDto>> {
    let trimmed = slug_or_id.trim();
    let lower = trimmed.to_ascii_lowercase();
    let cleaned = lower
        .strip_prefix("c/")
        .or_else(|| lower.strip_prefix("r/"))
        .unwrap_or(&lower);
    let sql = format!(
        "SELECT {COMM_SELECT_COLS} FROM communities
         WHERE community_id = ?1 OR slug = ?2 COLLATE NOCASE LIMIT 1"
    );
    let mut comm = conn
        .query_row(&sql, params![trimmed, cleaned], row_to_community)
        .optional()
        .map_err(|e| db_err("get community", e))?;
    if let Some(ref mut c) = comm {
        hydrate_community_counts_and_viewer(conn, c, viewer_actor_id)?;
    }
    Ok(comm)
}

pub fn list_communities_conn(
    conn: &Connection,
    viewer_actor_id: Option<&str>,
    category: Option<&str>,
    limit: usize,
) -> Result<Vec<SubCommunityDto>> {
    let limit = limit.clamp(1, 200) as i64;
    let mut out = Vec::new();
    if let Some(cat) = category.map(str::trim).filter(|c| !c.is_empty()) {
        let sql = format!(
            "SELECT {COMM_SELECT_COLS} FROM communities
             WHERE category = ?1 COLLATE NOCASE
             ORDER BY members_count DESC, posts_count DESC, slug ASC LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| db_err("prepare list communities cat", e))?;
        let rows = stmt
            .query_map(params![cat, limit], row_to_community)
            .map_err(|e| db_err("query list communities cat", e))?;
        for r in rows {
            let mut c = r.map_err(|e| db_err("read community", e))?;
            hydrate_community_counts_and_viewer(conn, &mut c, viewer_actor_id)?;
            out.push(c);
        }
    } else {
        let sql = format!(
            "SELECT {COMM_SELECT_COLS} FROM communities
             ORDER BY members_count DESC, posts_count DESC, slug ASC LIMIT ?1"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| db_err("prepare list communities", e))?;
        let rows = stmt
            .query_map(params![limit], row_to_community)
            .map_err(|e| db_err("query list communities", e))?;
        for r in rows {
            let mut c = r.map_err(|e| db_err("read community", e))?;
            hydrate_community_counts_and_viewer(conn, &mut c, viewer_actor_id)?;
            out.push(c);
        }
    }
    Ok(out)
}

pub fn create_community_tx(
    tx: &Transaction<'_>,
    creator_actor_id: &str,
    req: &CreateCommunityRequest,
    now_ms: i64,
) -> Result<SubCommunityDto> {
    let slug = normalize_community_slug(&req.slug)?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(TelegramError::Validation(
            "community name must be 1..=80 characters".to_string(),
        ));
    }
    if get_actor_conn(tx, creator_actor_id, None)?.is_none() {
        return Err(TelegramError::NotFound(format!(
            "creator actor '{creator_actor_id}' not found"
        )));
    }
    if get_community_conn(tx, &slug, None)?.is_some() {
        return Err(TelegramError::Validation(format!(
            "community c/{slug} already exists"
        )));
    }

    let comm_id = format!("comm_{}", Uuid::new_v4().simple());
    let category = if req.category.trim().is_empty() {
        "general".to_string()
    } else {
        req.category.trim().to_ascii_lowercase()
    };
    let icon_emoji = req
        .icon_emoji
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("🌐")
        .to_string();
    let accent = req
        .accent_color
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("#3B82F6")
        .to_string();
    let wiki = if req.wiki_markdown.trim().is_empty() {
        format!("# c/{slug}\n\n{}", req.description.trim())
    } else {
        req.wiki_markdown.clone()
    };

    tx.execute(
        "INSERT INTO communities (
            community_id, slug, name, description, category, icon_emoji, icon_media_id, banner_media_id,
            accent_color, creator_actor_id, rules_json, post_flairs_json, user_flairs_json,
            automod_rules_json, pinned_post_ids_json, wiki_markdown, wiki_version,
            allow_bots, require_bot_disclosure, is_nsfw, members_count, posts_count, created_at_ms, updated_at_ms
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,'[]',?15,1,?16,?17,?18,1,0,?19,?19)",
        params![
            comm_id, slug, name, req.description.trim(), category, icon_emoji,
            req.icon_media_id, req.banner_media_id, accent, creator_actor_id,
            serde_json::to_string(&req.rules)?,
            serde_json::to_string(&req.post_flairs)?,
            serde_json::to_string(&req.user_flairs)?,
            serde_json::to_string(&req.automod_rules)?,
            wiki, req.allow_bots as i64, req.require_bot_disclosure as i64,
            req.is_nsfw as i64, now_ms,
        ],
    )
    .map_err(|e| db_err("insert community", e))?;

    tx.execute(
        "INSERT INTO community_wiki_revisions (revision_id, community_id, version, editor_actor_id, content_markdown, summary, created_at_ms)
         VALUES (?1, ?2, 1, ?3, ?4, 'Initial wiki', ?5)",
        params![format!("rev_{}", Uuid::new_v4().simple()), comm_id, creator_actor_id, wiki, now_ms],
    )
    .map_err(|e| db_err("insert initial wiki rev", e))?;

    tx.execute(
        "INSERT INTO community_memberships (community_id, actor_id, role, notifications_enabled, joined_at_ms, updated_at_ms)
         VALUES (?1, ?2, 'owner', 1, ?3, ?3)",
        params![comm_id, creator_actor_id, now_ms],
    )
    .map_err(|e| db_err("insert creator ownership", e))?;

    get_community_conn(tx, &comm_id, Some(creator_actor_id))?
        .ok_or_else(|| TelegramError::Internal("created community missing".to_string()))
}

pub fn update_community_tx(
    tx: &Transaction<'_>,
    slug_or_id: &str,
    mod_actor_id: &str,
    patch: &UpdateCommunityRequest,
    now_ms: i64,
) -> Result<SubCommunityDto> {
    let mut comm = get_community_conn(tx, slug_or_id, Some(mod_actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
    let role = get_member_role_conn(tx, &comm.community_id, mod_actor_id)?;
    if !ModPermissionValidator::can_moderate(role) {
        return Err(TelegramError::PermissionDenied(
            "only community moderators may update community settings".to_string(),
        ));
    }

    if let Some(name) = patch.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        comm.name = name.to_string();
    }
    if let Some(desc) = patch.description.as_deref() {
        comm.description = desc.trim().to_string();
    }
    if let Some(cat) = patch.category.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        comm.category = cat.to_ascii_lowercase();
    }
    if let Some(bn) = patch.banner_media_id.as_deref() {
        comm.banner_media_id = (!bn.trim().is_empty()).then(|| bn.trim().to_string());
    }
    if let Some(ic) = patch.icon_media_id.as_deref() {
        comm.icon_media_id = (!ic.trim().is_empty()).then(|| ic.trim().to_string());
    }
    if let Some(em) = patch.icon_emoji.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        comm.icon_emoji = em.to_string();
    }
    if let Some(ac) = patch.accent_color.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        comm.accent_color = ac.to_string();
    }
    if let Some(rules) = patch.rules.as_ref() {
        comm.rules = rules.clone();
    }
    if let Some(pf) = patch.post_flairs.as_ref() {
        comm.post_flairs = pf.clone();
    }
    if let Some(uf) = patch.user_flairs.as_ref() {
        comm.user_flairs = uf.clone();
    }
    if let Some(am) = patch.automod_rules.as_ref() {
        comm.automod_rules = am.clone();
    }
    if let Some(ab) = patch.allow_bots {
        comm.allow_bots = ab;
    }
    if let Some(rbd) = patch.require_bot_disclosure {
        comm.require_bot_disclosure = rbd;
    }
    if let Some(nsfw) = patch.is_nsfw {
        comm.is_nsfw = nsfw;
    }

    tx.execute(
        "UPDATE communities SET
            name=?2, description=?3, category=?4, banner_media_id=?5, icon_media_id=?6,
            icon_emoji=?7, accent_color=?8, rules_json=?9, post_flairs_json=?10,
            user_flairs_json=?11, automod_rules_json=?12, allow_bots=?13,
            require_bot_disclosure=?14, is_nsfw=?15, updated_at_ms=?16
         WHERE community_id=?1",
        params![
            comm.community_id,
            comm.name,
            comm.description,
            comm.category,
            comm.banner_media_id,
            comm.icon_media_id,
            comm.icon_emoji,
            comm.accent_color,
            serde_json::to_string(&comm.rules)?,
            serde_json::to_string(&comm.post_flairs)?,
            serde_json::to_string(&comm.user_flairs)?,
            serde_json::to_string(&comm.automod_rules)?,
            comm.allow_bots as i64,
            comm.require_bot_disclosure as i64,
            comm.is_nsfw as i64,
            now_ms,
        ],
    )
    .map_err(|e| db_err("update community", e))?;

    get_community_conn(tx, &comm.community_id, Some(mod_actor_id))?
        .ok_or_else(|| TelegramError::Internal("updated community missing".to_string()))
}

pub fn join_community_tx(
    tx: &Transaction<'_>,
    slug_or_id: &str,
    actor_id: &str,
    now_ms: i64,
) -> Result<SubCommunityDto> {
    let comm = get_community_conn(tx, slug_or_id, Some(actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
    if get_actor_conn(tx, actor_id, None)?.is_none() {
        return Err(TelegramError::NotFound(format!("actor '{actor_id}' not found")));
    }
    if get_member_role_conn(tx, &comm.community_id, actor_id)? == Some(CommunityMemberRole::Banned) {
        return Err(TelegramError::PermissionDenied(format!(
            "actor '{actor_id}' is banned from c/{}",
            comm.slug
        )));
    }
    let changed = tx
        .execute(
            "INSERT OR IGNORE INTO community_memberships (community_id, actor_id, role, notifications_enabled, joined_at_ms, updated_at_ms)
             VALUES (?1, ?2, 'member', 1, ?3, ?3)",
            params![comm.community_id, actor_id, now_ms],
        )
        .map_err(|e| db_err("join community", e))?;
    if changed == 1 {
        tx.execute(
            "UPDATE communities SET members_count = members_count + 1, updated_at_ms = ?2 WHERE community_id = ?1",
            params![comm.community_id, now_ms],
        )
        .map_err(|e| db_err("inc community members_count", e))?;
        bump_affinity_conn(tx, actor_id, "community", &comm.slug, 3.0, now_ms)?;
    }
    get_community_conn(tx, &comm.community_id, Some(actor_id))?
        .ok_or_else(|| TelegramError::Internal("community missing after join".to_string()))
}

pub fn leave_community_tx(
    tx: &Transaction<'_>,
    slug_or_id: &str,
    actor_id: &str,
) -> Result<SubCommunityDto> {
    let comm = get_community_conn(tx, slug_or_id, Some(actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
    let role = get_member_role_conn(tx, &comm.community_id, actor_id)?;
    if role == Some(CommunityMemberRole::Banned) {
        return Err(TelegramError::PermissionDenied(
            "banned members cannot leave to bypass a ban".to_string(),
        ));
    }
    if role == Some(CommunityMemberRole::Owner) {
        return Err(TelegramError::Validation(
            "community owner cannot leave without transferring ownership".to_string(),
        ));
    }
    let changed = tx
        .execute(
            "DELETE FROM community_memberships WHERE community_id = ?1 AND actor_id = ?2 AND role != 'banned'",
            params![comm.community_id, actor_id],
        )
        .map_err(|e| db_err("leave community", e))?;
    if changed == 1 {
        tx.execute(
            "UPDATE communities SET members_count = MAX(0, members_count - 1) WHERE community_id = ?1",
            params![comm.community_id],
        )
        .map_err(|e| db_err("dec community members_count", e))?;
    }
    get_community_conn(tx, &comm.community_id, Some(actor_id))?
        .ok_or_else(|| TelegramError::Internal("community missing after leave".to_string()))
}

pub fn update_wiki_tx(
    tx: &Transaction<'_>,
    slug_or_id: &str,
    editor_actor_id: &str,
    content_markdown: &str,
    summary: &str,
    now_ms: i64,
) -> Result<CommunityWikiRevisionDto> {
    let comm = get_community_conn(tx, slug_or_id, Some(editor_actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
    let editor = get_actor_conn(tx, editor_actor_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("editor '{editor_actor_id}' not found")))?;
    let role = get_member_role_conn(tx, &comm.community_id, editor_actor_id)?;
    if !ModPermissionValidator::can_moderate(role) {
        return Err(TelegramError::PermissionDenied(
            "only community moderators may edit the community wiki".to_string(),
        ));
    }

    let next_ver = comm.wiki_revision.saturating_add(1);
    tx.execute(
        "UPDATE communities SET wiki_markdown = ?2, wiki_version = ?3, updated_at_ms = ?4 WHERE community_id = ?1",
        params![comm.community_id, content_markdown, i64::from(next_ver), now_ms],
    )
    .map_err(|e| db_err("update community wiki", e))?;

    let rev_id = format!("rev_{}", Uuid::new_v4().simple());
    tx.execute(
        "INSERT INTO community_wiki_revisions (revision_id, community_id, version, editor_actor_id, content_markdown, summary, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![rev_id, comm.community_id, i64::from(next_ver), editor_actor_id, content_markdown, summary, now_ms],
    )
    .map_err(|e| db_err("insert wiki revision", e))?;

    let log = ModPermissionValidator::build_mod_log_entry(
        &comm.community_id,
        &editor,
        ModActionKind::WikiEdit,
        "wiki",
        &comm.community_id,
        Some(summary),
        None,
        now_ms,
    );
    insert_mod_log_conn(tx, &log)?;

    Ok(CommunityWikiRevisionDto {
        revision: next_ver,
        markdown: content_markdown.to_string(),
        editor_actor_id: editor.actor_id,
        editor_handle: editor.handle,
        summary: summary.to_string(),
        created_at_ms: now_ms,
    })
}

pub fn list_wiki_revisions_conn(
    conn: &Connection,
    slug_or_id: &str,
    limit: usize,
) -> Result<Vec<CommunityWikiRevisionDto>> {
    let comm = get_community_conn(conn, slug_or_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
    let mut stmt = conn
        .prepare(
            "SELECT r.version, r.content_markdown, r.editor_actor_id, a.handle, r.summary, r.created_at_ms
             FROM community_wiki_revisions r
             JOIN community_actors a ON a.actor_id = r.editor_actor_id
             WHERE r.community_id = ?1
             ORDER BY r.version DESC LIMIT ?2",
        )
        .map_err(|e| db_err("prepare wiki revisions", e))?;
    let rows = stmt
        .query_map(params![comm.community_id, limit.clamp(1, 100) as i64], |r| {
            Ok(CommunityWikiRevisionDto {
                revision: r.get::<_, i64>(0)?.max(1) as u32,
                markdown: r.get(1)?,
                editor_actor_id: r.get(2)?,
                editor_handle: r.get(3)?,
                summary: r.get(4)?,
                created_at_ms: r.get(5)?,
            })
        })
        .map_err(|e| db_err("query wiki revisions", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| db_err("read wiki rev", e))?);
    }
    Ok(out)
}

pub(crate) fn insert_mod_log_conn(conn: &Connection, log: &ModLogEntryDto) -> Result<()> {
    conn.execute(
        "INSERT INTO community_mod_logs (log_id, community_id, mod_actor_id, action, target_kind, target_id, reason, rule_triggered, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            log.log_id,
            log.community_id,
            log.moderator_actor_id,
            log.action.as_str(),
            log.target_kind,
            log.target_id,
            log.reason.as_deref().unwrap_or(""),
            log.rule_triggered,
            log.created_at_ms,
        ],
    )
    .map_err(|e| db_err("insert mod log", e))?;
    Ok(())
}

pub fn enqueue_mod_item_tx(
    tx: &Transaction<'_>,
    community_id: &str,
    target_kind: &str,
    target_id: &str,
    reporter_actor_id: Option<&str>,
    source: &str,
    reason: &str,
    rule_triggered: Option<&str>,
    now_ms: i64,
) -> Result<ModQueueItemDto> {
    let item_id = format!("mq_{}", Uuid::new_v4().simple());
    tx.execute(
        "INSERT INTO community_mod_queue (
            item_id, community_id, target_kind, target_id, reporter_actor_id,
            source, reason, rule_triggered, status, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9)",
        params![
            item_id,
            community_id,
            target_kind,
            target_id,
            reporter_actor_id,
            source,
            reason,
            rule_triggered,
            now_ms,
        ],
    )
    .map_err(|e| db_err("enqueue mod queue item", e))?;
    Ok(ModQueueItemDto {
        item_id,
        community_id: community_id.to_string(),
        target_kind: target_kind.to_string(),
        target_id: target_id.to_string(),
        reporter_actor_id: reporter_actor_id.map(str::to_string),
        source: source.to_string(),
        reason: reason.to_string(),
        rule_triggered: rule_triggered.map(str::to_string),
        status: ModQueueStatus::Pending,
        resolved_by_actor_id: None,
        resolved_at_ms: None,
        created_at_ms: now_ms,
    })
}

pub fn list_mod_queue_conn(
    conn: &Connection,
    slug_or_id: &str,
    status: Option<ModQueueStatus>,
    limit: usize,
) -> Result<Vec<ModQueueItemDto>> {
    let comm = get_community_conn(conn, slug_or_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
    let limit = limit.clamp(1, 200) as i64;
    let map_row = |r: &Row<'_>| -> rusqlite::Result<ModQueueItemDto> {
        let st: String = r.get(8)?;
        Ok(ModQueueItemDto {
            item_id: r.get(0)?,
            community_id: r.get(1)?,
            target_kind: r.get(2)?,
            target_id: r.get(3)?,
            reporter_actor_id: r.get(4)?,
            source: r.get(5)?,
            reason: r.get(6)?,
            rule_triggered: r.get(7)?,
            status: ModQueueStatus::parse(&st).unwrap_or(ModQueueStatus::Pending),
            resolved_by_actor_id: r.get(9)?,
            resolved_at_ms: r.get(10)?,
            created_at_ms: r.get(11)?,
        })
    };
    let mut out = Vec::new();
    if let Some(st) = status {
        let mut stmt = conn
            .prepare(
                "SELECT item_id, community_id, target_kind, target_id, reporter_actor_id, source, reason, rule_triggered, status, resolved_by_actor_id, resolved_at_ms, created_at_ms
                 FROM community_mod_queue WHERE community_id = ?1 AND status = ?2 ORDER BY created_at_ms DESC LIMIT ?3",
            )
            .map_err(|e| db_err("prepare mod queue", e))?;
        let rows = stmt
            .query_map(params![comm.community_id, st.as_str(), limit], map_row)
            .map_err(|e| db_err("query mod queue", e))?;
        for r in rows {
            out.push(r.map_err(|e| db_err("read mod queue", e))?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT item_id, community_id, target_kind, target_id, reporter_actor_id, source, reason, rule_triggered, status, resolved_by_actor_id, resolved_at_ms, created_at_ms
                 FROM community_mod_queue WHERE community_id = ?1 ORDER BY created_at_ms DESC LIMIT ?2",
            )
            .map_err(|e| db_err("prepare mod queue all", e))?;
        let rows = stmt
            .query_map(params![comm.community_id, limit], map_row)
            .map_err(|e| db_err("query mod queue all", e))?;
        for r in rows {
            out.push(r.map_err(|e| db_err("read mod queue all", e))?);
        }
    }
    Ok(out)
}

fn verify_post_in_community(
    tx: &Transaction<'_>,
    post_id: &str,
    comm: &SubCommunityDto,
) -> Result<String> {
    let row: Option<(Option<String>, String)> = tx
        .query_row(
            "SELECT community_id, author_actor_id FROM community_posts WHERE post_id = ?1",
            params![post_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| db_err("verify post community ownership", e))?;
    let (post_comm_id, author_id) = row
        .ok_or_else(|| TelegramError::NotFound(format!("post '{post_id}' not found")))?;
    if post_comm_id.as_deref() != Some(comm.community_id.as_str()) {
        return Err(TelegramError::PermissionDenied(format!(
            "post '{post_id}' does not belong to c/{}",
            comm.slug
        )));
    }
    Ok(author_id)
}

fn verify_comment_in_community(
    tx: &Transaction<'_>,
    comment_id: &str,
    comm: &SubCommunityDto,
) -> Result<String> {
    let row: Option<(Option<String>, String)> = tx
        .query_row(
            "SELECT p.community_id, c.author_actor_id
             FROM community_comments c
             JOIN community_posts p ON p.post_id = c.post_id
             WHERE c.comment_id = ?1",
            params![comment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| db_err("verify comment community ownership", e))?;
    let (post_comm_id, author_id) = row
        .ok_or_else(|| TelegramError::NotFound(format!("comment '{comment_id}' not found")))?;
    if post_comm_id.as_deref() != Some(comm.community_id.as_str()) {
        return Err(TelegramError::PermissionDenied(format!(
            "comment '{comment_id}' does not belong to c/{}",
            comm.slug
        )));
    }
    Ok(author_id)
}

pub fn apply_mod_action_tx(
    tx: &Transaction<'_>,
    slug_or_id: &str,
    mod_actor_id: &str,
    req: &ModActionRequest,
    now_ms: i64,
) -> Result<ModLogEntryDto> {
    let comm = get_community_conn(tx, slug_or_id, Some(mod_actor_id))?
        .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
    let moderator = get_actor_conn(tx, mod_actor_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("moderator '{mod_actor_id}' not found")))?;
    let mod_role = get_member_role_conn(tx, &comm.community_id, mod_actor_id)?;

    let target_kind = req.target_kind.trim().to_ascii_lowercase();
    let is_actor_action = matches!(
        req.action,
        ModActionKind::BanActor
            | ModActionKind::UnbanActor
            | ModActionKind::MuteActor
            | ModActionKind::UnmuteActor
            | ModActionKind::PromoteMod
            | ModActionKind::DemoteMod
            | ModActionKind::SetUserFlair
    );

    // Enforce strict target_kind matching per ModActionKind so role checks can never be bypassed.
    match req.action {
        ModActionKind::BanActor
        | ModActionKind::UnbanActor
        | ModActionKind::MuteActor
        | ModActionKind::UnmuteActor
        | ModActionKind::PromoteMod
        | ModActionKind::DemoteMod
        | ModActionKind::SetUserFlair => {
            if target_kind != "actor" {
                return Err(TelegramError::Validation(format!(
                    "moderation action '{}' requires target_kind 'actor'",
                    req.action.as_str()
                )));
            }
        }
        ModActionKind::Lock | ModActionKind::Unlock | ModActionKind::SetPostFlair => {
            if target_kind != "post" {
                return Err(TelegramError::Validation(format!(
                    "moderation action '{}' requires target_kind 'post'",
                    req.action.as_str()
                )));
            }
        }
        ModActionKind::Approve
        | ModActionKind::Remove
        | ModActionKind::Pin
        | ModActionKind::Unpin
        | ModActionKind::Distinguish
        | ModActionKind::Undistinguish => {
            if target_kind != "post" && target_kind != "comment" {
                return Err(TelegramError::Validation(format!(
                    "moderation action '{}' requires target_kind 'post' or 'comment'",
                    req.action.as_str()
                )));
            }
        }
        ModActionKind::ResolveReport | ModActionKind::WikiEdit => {}
    }

    let target_role = if is_actor_action {
        get_member_role_conn(tx, &comm.community_id, &req.target_id)?
    } else {
        None
    };
    ModPermissionValidator::validate_mod_action(mod_role, target_role, req.action)?;

    // Upfront ownership verification for explicit queue_item_id (prevents cross-community dismissal)
    if let Some(qid) = req.queue_item_id.as_deref() {
        let q_comm: Option<String> = tx
            .query_row(
                "SELECT community_id FROM community_mod_queue WHERE item_id = ?1",
                params![qid],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| db_err("verify mod queue item ownership", e))?;
        match q_comm.as_deref() {
            None => {
                return Err(TelegramError::NotFound(format!(
                    "mod queue item '{qid}' not found"
                )));
            }
            Some(cid) if cid != comm.community_id => {
                return Err(TelegramError::PermissionDenied(format!(
                    "mod queue item '{qid}' does not belong to c/{}",
                    comm.slug
                )));
            }
            Some(_) => {}
        }
    }

    let mut affected_actor: Option<String> = None;
    let mut queue_status: Option<&str> = None;

    match req.action {
        ModActionKind::Approve => {
            if target_kind == "post" {
                verify_post_in_community(tx, &req.target_id, &comm)?;
                tx.execute(
                    "UPDATE community_posts SET is_removed = 0, removed_reason = NULL, updated_at_ms = ?3
                     WHERE post_id = ?1 AND community_id = ?2",
                    params![req.target_id, comm.community_id, now_ms],
                )
                .map_err(|e| db_err("approve post", e))?;
            } else {
                verify_comment_in_community(tx, &req.target_id, &comm)?;
                tx.execute(
                    "UPDATE community_comments SET is_removed = 0, removed_reason = NULL, updated_at_ms = ?2
                     WHERE comment_id = ?1",
                    params![req.target_id, now_ms],
                )
                .map_err(|e| db_err("approve comment", e))?;
            }
            queue_status = Some("approved");
        }
        ModActionKind::Remove => {
            if target_kind == "post" {
                affected_actor = Some(verify_post_in_community(tx, &req.target_id, &comm)?);
                tx.execute(
                    "UPDATE community_posts SET is_removed = 1, removed_reason = ?3, updated_at_ms = ?4
                     WHERE post_id = ?1 AND community_id = ?2",
                    params![req.target_id, comm.community_id, req.reason, now_ms],
                )
                .map_err(|e| db_err("remove post", e))?;
            } else {
                affected_actor = Some(verify_comment_in_community(tx, &req.target_id, &comm)?);
                tx.execute(
                    "UPDATE community_comments SET is_removed = 1, removed_reason = ?2, updated_at_ms = ?3
                     WHERE comment_id = ?1",
                    params![req.target_id, req.reason, now_ms],
                )
                .map_err(|e| db_err("remove comment", e))?;
            }
            queue_status = Some("removed");
        }
        ModActionKind::Lock | ModActionKind::Unlock => {
            verify_post_in_community(tx, &req.target_id, &comm)?;
            let v = i64::from(req.action == ModActionKind::Lock);
            tx.execute(
                "UPDATE community_posts SET is_locked = ?3, updated_at_ms = ?4
                 WHERE post_id = ?1 AND community_id = ?2",
                params![req.target_id, comm.community_id, v, now_ms],
            )
            .map_err(|e| db_err("lock/unlock post", e))?;
        }
        ModActionKind::Pin | ModActionKind::Unpin => {
            let pin = req.action == ModActionKind::Pin;
            if target_kind == "comment" {
                verify_comment_in_community(tx, &req.target_id, &comm)?;
                tx.execute(
                    "UPDATE community_comments SET is_stickied = ?2, updated_at_ms = ?3 WHERE comment_id = ?1",
                    params![req.target_id, pin as i64, now_ms],
                )
                .map_err(|e| db_err("sticky comment", e))?;
            } else {
                verify_post_in_community(tx, &req.target_id, &comm)?;
                tx.execute(
                    "UPDATE community_posts SET is_pinned = ?3, updated_at_ms = ?4
                     WHERE post_id = ?1 AND community_id = ?2",
                    params![req.target_id, comm.community_id, pin as i64, now_ms],
                )
                .map_err(|e| db_err("pin post", e))?;
                let mut pinned = comm.pinned_post_ids.clone();
                if pin {
                    if !pinned.contains(&req.target_id) {
                        pinned.push(req.target_id.clone());
                    }
                } else {
                    pinned.retain(|id| id != &req.target_id);
                }
                tx.execute(
                    "UPDATE communities SET pinned_post_ids_json = ?2, updated_at_ms = ?3 WHERE community_id = ?1",
                    params![comm.community_id, serde_json::to_string(&pinned)?, now_ms],
                )
                .map_err(|e| db_err("update pinned_post_ids", e))?;
            }
        }
        ModActionKind::Distinguish | ModActionKind::Undistinguish => {
            let v = i64::from(req.action == ModActionKind::Distinguish);
            if target_kind == "comment" {
                verify_comment_in_community(tx, &req.target_id, &comm)?;
                tx.execute(
                    "UPDATE community_comments SET is_distinguished_mod = ?2, updated_at_ms = ?3 WHERE comment_id = ?1",
                    params![req.target_id, v, now_ms],
                )
                .map_err(|e| db_err("distinguish comment", e))?;
            } else {
                verify_post_in_community(tx, &req.target_id, &comm)?;
                tx.execute(
                    "UPDATE community_posts SET is_distinguished_mod = ?3, updated_at_ms = ?4
                     WHERE post_id = ?1 AND community_id = ?2",
                    params![req.target_id, comm.community_id, v, now_ms],
                )
                .map_err(|e| db_err("distinguish post", e))?;
            }
        }
        ModActionKind::BanActor | ModActionKind::MuteActor | ModActionKind::PromoteMod => {
            let target_actor = get_actor_conn(tx, &req.target_id, None)?
                .ok_or_else(|| TelegramError::NotFound(format!("target actor '{}' not found", req.target_id)))?;
            let new_role = match req.action {
                ModActionKind::BanActor => "banned",
                ModActionKind::MuteActor => "muted",
                ModActionKind::PromoteMod => {
                    if target_actor.actor_kind.is_bot_like() { "bot_mod" } else { "mod" }
                }
                _ => "member",
            };
            tx.execute(
                "INSERT INTO community_memberships (community_id, actor_id, role, notifications_enabled, joined_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 1, ?4, ?4)
                 ON CONFLICT(community_id, actor_id) DO UPDATE SET role = excluded.role, updated_at_ms = excluded.updated_at_ms",
                params![comm.community_id, req.target_id, new_role, now_ms],
            )
            .map_err(|e| db_err("set membership role", e))?;
            affected_actor = Some(req.target_id.clone());
        }
        ModActionKind::UnbanActor | ModActionKind::UnmuteActor | ModActionKind::DemoteMod => {
            let sql = match req.action {
                ModActionKind::UnbanActor => {
                    "UPDATE community_memberships SET role = 'member', updated_at_ms = ?3
                     WHERE community_id = ?1 AND actor_id = ?2 AND role = 'banned'"
                }
                ModActionKind::UnmuteActor => {
                    "UPDATE community_memberships SET role = 'member', updated_at_ms = ?3
                     WHERE community_id = ?1 AND actor_id = ?2 AND role = 'muted'"
                }
                _ => {
                    "UPDATE community_memberships SET role = 'member', updated_at_ms = ?3
                     WHERE community_id = ?1 AND actor_id = ?2 AND role IN ('mod', 'bot_mod')"
                }
            };
            tx.execute(sql, params![comm.community_id, req.target_id, now_ms])
                .map_err(|e| db_err("restore member role", e))?;
            affected_actor = Some(req.target_id.clone());
        }
        ModActionKind::SetPostFlair => {
            verify_post_in_community(tx, &req.target_id, &comm)?;
            tx.execute(
                "UPDATE community_posts SET flair_text = ?3, updated_at_ms = ?4
                 WHERE post_id = ?1 AND community_id = ?2",
                params![req.target_id, comm.community_id, req.flair, now_ms],
            )
            .map_err(|e| db_err("set post flair", e))?;
        }
        ModActionKind::SetUserFlair => {
            tx.execute(
                "UPDATE community_memberships SET user_flair_text = ?3, updated_at_ms = ?4
                 WHERE community_id = ?1 AND actor_id = ?2",
                params![comm.community_id, req.target_id, req.flair, now_ms],
            )
            .map_err(|e| db_err("set user flair", e))?;
        }
        ModActionKind::ResolveReport => {
            if req.queue_item_id.is_none() {
                if target_kind == "post" {
                    verify_post_in_community(tx, &req.target_id, &comm)?;
                } else if target_kind == "comment" {
                    verify_comment_in_community(tx, &req.target_id, &comm)?;
                }
            }
            queue_status = Some("dismissed");
        }
        ModActionKind::WikiEdit => {}
    }

    if let Some(st) = queue_status {
        if let Some(qid) = req.queue_item_id.as_deref() {
            let changed = tx
                .execute(
                    "UPDATE community_mod_queue SET status = ?3, resolved_by_actor_id = ?4, resolved_at_ms = ?5
                     WHERE item_id = ?1 AND community_id = ?2",
                    params![qid, comm.community_id, st, mod_actor_id, now_ms],
                )
                .map_err(|e| db_err("resolve queue item by id", e))?;
            if changed == 0 {
                return Err(TelegramError::PermissionDenied(format!(
                    "mod queue item '{qid}' does not belong to c/{}",
                    comm.slug
                )));
            }
        } else {
            tx.execute(
                "UPDATE community_mod_queue SET status = ?3, resolved_by_actor_id = ?4, resolved_at_ms = ?5
                 WHERE community_id = ?1 AND target_id = ?2 AND status = 'pending'",
                params![comm.community_id, req.target_id, st, mod_actor_id, now_ms],
            )
            .map_err(|e| db_err("resolve matching queue items", e))?;
        }
    }

    let log = ModPermissionValidator::build_mod_log_entry(
        &comm.community_id,
        &moderator,
        req.action,
        &target_kind,
        &req.target_id,
        req.reason.as_deref(),
        None,
        now_ms,
    );
    insert_mod_log_conn(tx, &log)?;

    if let Some(recip) = affected_actor {
        insert_notification_conn(
            tx,
            &recip,
            mod_actor_id,
            CommunityNotificationKind::ModAction,
            (target_kind == "post").then_some(req.target_id.as_str()),
            (target_kind == "comment").then_some(req.target_id.as_str()),
            &format!("Moderator @{} performed {} in c/{}", moderator.handle, req.action.as_str(), comm.slug),
            now_ms,
        )?;
    }

    Ok(log)
}

pub fn list_mod_logs_conn(conn: &Connection, slug_or_id: &str, limit: usize) -> Result<Vec<ModLogEntryDto>> {
    let comm = get_community_conn(conn, slug_or_id, None)?
        .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
    let mut stmt = conn
        .prepare(
            "SELECT l.log_id, l.community_id, l.mod_actor_id, a.handle, a.actor_kind, l.action, l.target_kind, l.target_id, l.reason, l.rule_triggered, l.created_at_ms
             FROM community_mod_logs l
             JOIN community_actors a ON a.actor_id = l.mod_actor_id
             WHERE l.community_id = ?1 ORDER BY l.created_at_ms DESC LIMIT ?2",
        )
        .map_err(|e| db_err("prepare mod logs", e))?;
    let rows = stmt
        .query_map(params![comm.community_id, limit.clamp(1, 200) as i64], |r| {
            let kind_str: String = r.get(4)?;
            let act_str: String = r.get(5)?;
            let reason_str: String = r.get(8)?;
            Ok(ModLogEntryDto {
                log_id: r.get(0)?,
                community_id: r.get(1)?,
                moderator_actor_id: r.get(2)?,
                moderator_handle: r.get(3)?,
                moderator_kind: ActorKind::parse(&kind_str).unwrap_or(ActorKind::Human),
                action: ModActionKind::parse(&act_str).unwrap_or(ModActionKind::Approve),
                target_kind: r.get(6)?,
                target_id: r.get(7)?,
                reason: (!reason_str.is_empty()).then_some(reason_str),
                rule_triggered: r.get(9)?,
                created_at_ms: r.get(10)?,
            })
        })
        .map_err(|e| db_err("query mod logs", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| db_err("read mod log", e))?);
    }
    Ok(out)
}

impl CommunityStore {
    pub fn create_community(&self, creator_actor_id: &str, req: &CreateCommunityRequest, now_ms: i64) -> Result<SubCommunityDto> {
        self.with_transaction(|tx| create_community_tx(tx, creator_actor_id, req, now_ms))
    }
    pub fn list_communities(&self, viewer_actor_id: Option<&str>, category: Option<&str>, limit: usize) -> Result<Vec<SubCommunityDto>> {
        self.with_conn(|conn| list_communities_conn(conn, viewer_actor_id, category, limit))
    }
    pub fn get_community(&self, slug_or_id: &str, viewer_actor_id: Option<&str>) -> Result<Option<SubCommunityDto>> {
        self.with_conn(|conn| get_community_conn(conn, slug_or_id, viewer_actor_id))
    }
    pub fn update_community(&self, slug_or_id: &str, mod_actor_id: &str, patch: &UpdateCommunityRequest, now_ms: i64) -> Result<SubCommunityDto> {
        self.with_transaction(|tx| update_community_tx(tx, slug_or_id, mod_actor_id, patch, now_ms))
    }
    pub fn join_community(&self, slug_or_id: &str, actor_id: &str, now_ms: i64) -> Result<SubCommunityDto> {
        self.with_transaction(|tx| join_community_tx(tx, slug_or_id, actor_id, now_ms))
    }
    pub fn leave_community(&self, slug_or_id: &str, actor_id: &str) -> Result<SubCommunityDto> {
        self.with_transaction(|tx| leave_community_tx(tx, slug_or_id, actor_id))
    }
    pub fn update_wiki(&self, slug_or_id: &str, editor_actor_id: &str, content_markdown: &str, summary: &str, now_ms: i64) -> Result<CommunityWikiRevisionDto> {
        self.with_transaction(|tx| update_wiki_tx(tx, slug_or_id, editor_actor_id, content_markdown, summary, now_ms))
    }
    pub fn list_wiki_revisions(&self, slug_or_id: &str, limit: usize) -> Result<Vec<CommunityWikiRevisionDto>> {
        self.with_conn(|conn| list_wiki_revisions_conn(conn, slug_or_id, limit))
    }
    pub fn enqueue_mod_item(&self, community_id: &str, target_kind: &str, target_id: &str, reporter_actor_id: Option<&str>, source: &str, reason: &str, rule_triggered: Option<&str>, now_ms: i64) -> Result<ModQueueItemDto> {
        self.with_transaction(|tx| enqueue_mod_item_tx(tx, community_id, target_kind, target_id, reporter_actor_id, source, reason, rule_triggered, now_ms))
    }
    pub fn list_mod_queue(&self, slug_or_id: &str, status: Option<ModQueueStatus>, limit: usize) -> Result<Vec<ModQueueItemDto>> {
        self.with_conn(|conn| list_mod_queue_conn(conn, slug_or_id, status, limit))
    }
    pub fn apply_mod_action(&self, slug_or_id: &str, mod_actor_id: &str, req: &ModActionRequest, now_ms: i64) -> Result<ModLogEntryDto> {
        self.with_transaction(|tx| apply_mod_action_tx(tx, slug_or_id, mod_actor_id, req, now_ms))
    }
    pub fn list_mod_logs(&self, slug_or_id: &str, limit: usize) -> Result<Vec<ModLogEntryDto>> {
        self.with_conn(|conn| list_mod_logs_conn(conn, slug_or_id, limit))
    }
    pub fn list_moderators(&self, slug_or_id: &str) -> Result<Vec<CommunityActorDto>> {
        self.with_conn(|conn| {
            let comm = get_community_conn(conn, slug_or_id, None)?
                .ok_or_else(|| TelegramError::NotFound(format!("community '{slug_or_id}' not found")))?;
            let sql = format!(
                "SELECT {} FROM community_actors a
                 JOIN community_memberships m ON m.actor_id = a.actor_id
                 WHERE m.community_id = ?1 AND m.role IN ('owner', 'mod', 'bot_mod')
                 ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'mod' THEN 1 ELSE 2 END, a.handle ASC",
                super::actors::ACTOR_SELECT_COLS.replace("actor_id,", "a.actor_id,")
                    .replace("created_at_ms", "a.created_at_ms")
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| db_err("prepare moderators", e))?;
            let rows = stmt
                .query_map(params![comm.community_id], super::actors::row_to_actor)
                .map_err(|e| db_err("query moderators", e))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(|e| db_err("read moderator", e))?);
            }
            Ok(out)
        })
    }
}
