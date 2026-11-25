//! Unified Human + Bot first-class actor identity, authorization, karma, trust, and follow graph.

use gpui_telegram_core::types::UserId;
use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, BotAutonomyMode, CommunityActorDto, RegisterBotActorRequest,
    RegisterHumanActorRequest, VerifiedTier,
};
use gpui_telegram_proto::miniapp_contracts::{
    CLASSIC_INITIALS_AVATAR_SHAPE, resolve_grok_persona_color, resolve_grok_persona_shape,
};
use std::collections::{HashMap, HashSet};

pub const MIN_HANDLE_LEN: usize = 2;
pub const MAX_HANDLE_LEN: usize = 32;
pub const MIN_TRUST_SCORE: f64 = 0.05;
pub const MAX_TRUST_SCORE: f64 = 1.00;

/// Canonical actor ID for a Telegram human account.
pub fn human_actor_id(user_id: i64) -> String {
    format!("u:{user_id}")
}

/// Canonical actor ID for a Telegram bot account.
pub fn bot_actor_id(bot_user_id: i64) -> String {
    format!("b:{bot_user_id}")
}

/// Canonical actor ID derived from core `UserId`.
pub fn human_actor_id_from_uid(uid: UserId) -> Result<String> {
    let signed = i64::try_from(uid.raw())
        .map_err(|_| TelegramError::Validation(format!("user_id {} exceeds i64::MAX", uid.raw())))?;
    Ok(human_actor_id(signed))
}

/// Converts a linked `i64` user ID back into core `UserId`.
pub fn user_id_from_i64(user_id: i64) -> Result<UserId> {
    let raw = u64::try_from(user_id)
        .map_err(|_| TelegramError::Validation(format!("negative user_id {user_id}")))?;
    Ok(UserId::new(raw))
}

/// Normalizes and validates a community `@handle` (`2..=32` chars, `[a-z0-9_]`).
pub fn normalize_handle(raw: &str) -> Result<String> {
    let cleaned = raw.trim().trim_start_matches('@').to_ascii_lowercase();
    if cleaned.len() < MIN_HANDLE_LEN || cleaned.len() > MAX_HANDLE_LEN {
        return Err(TelegramError::Validation(format!(
            "handle must be between {MIN_HANDLE_LEN} and {MAX_HANDLE_LEN} characters"
        )));
    }
    if !cleaned
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(TelegramError::Validation(
            "handle may only contain ASCII letters, digits, and underscores".to_string(),
        ));
    }
    if cleaned.chars().all(|c| c == '_') {
        return Err(TelegramError::Validation(
            "handle cannot consist solely of underscores".to_string(),
        ));
    }
    Ok(cleaned)
}

/// Builds a new first-class Human `CommunityActorDto`.
pub fn new_human_actor(
    user_id: i64,
    req: &RegisterHumanActorRequest,
    now_ms: i64,
) -> Result<CommunityActorDto> {
    let handle = normalize_handle(&req.handle)?;
    let display_name = req.display_name.trim().to_string();
    if display_name.is_empty() || display_name.chars().count() > 64 {
        return Err(TelegramError::Validation(
            "display_name must be 1..=64 characters".to_string(),
        ));
    }
    let actor_id = human_actor_id(user_id);
    let avatar_shape = match req.avatar_shape.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => resolve_grok_persona_shape(&actor_id, Some(s)),
        _ => CLASSIC_INITIALS_AVATAR_SHAPE.to_string(),
    };
    let avatar_color = resolve_grok_persona_color(&actor_id, req.avatar_color.as_deref());

    let mut actor = CommunityActorDto {
        actor_id,
        linked_user_id: Some(user_id),
        owner_user_id: Some(user_id),
        actor_kind: ActorKind::Human,
        handle,
        display_name,
        bio: req.bio.clone().unwrap_or_default().trim().to_string(),
        location: req.location.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        website: req.website.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        avatar_media_id: req.avatar_media_id.clone().filter(|s| !s.trim().is_empty()),
        banner_media_id: req.banner_media_id.clone().filter(|s| !s.trim().is_empty()),
        avatar_shape,
        avatar_color,
        verified_tier: VerifiedTier::None,
        bot_badge: None,
        bot_model: None,
        bot_capabilities: Vec::new(),
        bot_autonomy_mode: BotAutonomyMode::Manual,
        bot_persona: None,
        post_karma: 0,
        comment_karma: 0,
        awardee_karma: 0,
        followers_count: 0,
        following_count: 0,
        posts_count: 0,
        trust_score: 0.50,
        is_following: false,
        is_muted: false,
        is_blocked: false,
        created_at_ms: now_ms,
    };
    actor.trust_score = compute_trust_score(&actor, now_ms);
    Ok(actor)
}

/// Builds a new first-class Bot or HybridAgent `CommunityActorDto`.
pub fn new_bot_actor(
    owner_user_id: i64,
    bot_user_id: i64,
    req: &RegisterBotActorRequest,
    now_ms: i64,
) -> Result<CommunityActorDto> {
    let actor_kind = req.actor_kind.unwrap_or(ActorKind::Bot);
    if actor_kind == ActorKind::Human {
        return Err(TelegramError::Validation(
            "bot actor registration requires actor_kind Bot or HybridAgent".to_string(),
        ));
    }
    let handle = normalize_handle(&req.handle)?;
    let display_name = req.display_name.trim().to_string();
    if display_name.is_empty() || display_name.chars().count() > 64 {
        return Err(TelegramError::Validation(
            "display_name must be 1..=64 characters".to_string(),
        ));
    }
    let actor_id = bot_actor_id(bot_user_id);
    let avatar_shape = resolve_grok_persona_shape(&actor_id, req.avatar_shape.as_deref());
    let avatar_color = resolve_grok_persona_color(&actor_id, req.avatar_color.as_deref());
    let bot_badge = req
        .bot_badge
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            Some(match actor_kind {
                ActorKind::HybridAgent => "HYBRID AI".to_string(),
                _ => "AI BOT".to_string(),
            })
        });

    let mut actor = CommunityActorDto {
        actor_id,
        linked_user_id: Some(bot_user_id),
        owner_user_id: Some(owner_user_id),
        actor_kind,
        handle,
        display_name,
        bio: req.bio.clone().unwrap_or_default().trim().to_string(),
        location: None,
        website: None,
        avatar_media_id: req.avatar_media_id.clone().filter(|s| !s.trim().is_empty()),
        banner_media_id: req.banner_media_id.clone().filter(|s| !s.trim().is_empty()),
        avatar_shape,
        avatar_color,
        verified_tier: VerifiedTier::OfficialBot,
        bot_badge,
        bot_model: req.bot_model.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        bot_capabilities: req
            .bot_capabilities
            .iter()
            .map(|c| c.trim().to_ascii_lowercase())
            .filter(|c| !c.is_empty())
            .collect(),
        bot_autonomy_mode: req.bot_autonomy_mode,
        bot_persona: req
            .bot_persona
            .clone()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        post_karma: 0,
        comment_karma: 0,
        awardee_karma: 0,
        followers_count: 0,
        following_count: 0,
        posts_count: 0,
        trust_score: 0.55,
        is_following: false,
        is_muted: false,
        is_blocked: false,
        created_at_ms: now_ms,
    };
    actor.trust_score = compute_trust_score(&actor, now_ms);
    Ok(actor)
}

/// Checks whether `caller_user_id` is authorized to act as `actor` (own Human identity or owned Bot/HybridAgent).
pub fn can_user_act_as(caller_user_id: i64, actor: &CommunityActorDto) -> bool {
    (actor.actor_kind == ActorKind::Human && actor.linked_user_id == Some(caller_user_id))
        || actor.owner_user_id == Some(caller_user_id)
}

/// Returns `Ok(())` if `caller_user_id` may act as `actor`, else `TelegramError::PermissionDenied`.
pub fn validate_act_as(caller_user_id: i64, actor: &CommunityActorDto) -> Result<()> {
    if can_user_act_as(caller_user_id, actor) {
        Ok(())
    } else {
        Err(TelegramError::PermissionDenied(format!(
            "user {caller_user_id} is not authorized to act as actor {}",
            actor.actor_id
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KarmaKind {
    Post,
    Comment,
    Awardee,
}

/// Applies a saturating karma delta to the requested karma bucket.
pub fn apply_karma_delta(actor: &mut CommunityActorDto, kind: KarmaKind, delta: i64) {
    match kind {
        KarmaKind::Post => actor.post_karma = actor.post_karma.saturating_add(delta),
        KarmaKind::Comment => actor.comment_karma = actor.comment_karma.saturating_add(delta),
        KarmaKind::Awardee => actor.awardee_karma = actor.awardee_karma.saturating_add(delta),
    }
}

/// Computes a bounded trust score in `[0.05, 1.0]` based on karma, followers, account age, and verification tier.
pub fn compute_trust_score(actor: &CommunityActorDto, now_ms: i64) -> f64 {
    let total_karma = actor
        .post_karma
        .saturating_add(actor.comment_karma)
        .saturating_add(actor.awardee_karma);
    let karma_signal = if total_karma >= 0 {
        0.25 * (1.0 + total_karma as f64).ln() / 10.0
    } else {
        -(0.25 * (1.0 + total_karma.unsigned_abs() as f64).ln() / 6.0).min(0.25)
    };
    let follower_signal = 0.15 * (1.0 + actor.followers_count as f64).ln() / 8.0;
    let age_days = (now_ms.saturating_sub(actor.created_at_ms).max(0) as f64) / 86_400_000.0;
    let age_signal = 0.10 * (age_days / 30.0).min(1.0);
    let verified_bonus = match actor.verified_tier {
        VerifiedTier::None => 0.0,
        VerifiedTier::HumanVerified | VerifiedTier::OfficialBot | VerifiedTier::VerifiedOrg => 0.15,
        VerifiedTier::CommunityMod => 0.12,
    };
    let raw = 0.35 + karma_signal + follower_signal + age_signal + verified_bonus;
    if raw.is_finite() {
        raw.clamp(MIN_TRUST_SCORE, MAX_TRUST_SCORE)
    } else {
        0.50
    }
}

/// Pure-Rust in-memory follow graph supporting 1-hop and normalized 2-hop social signals.
#[derive(Debug, Clone, Default)]
pub struct ActorGraph {
    following: HashMap<String, HashSet<String>>,
    followers: HashMap<String, HashSet<String>>,
}

impl ActorGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn follow(&mut self, follower: &str, target: &str) -> bool {
        if follower == target || follower.is_empty() || target.is_empty() {
            return false;
        }
        let inserted = self
            .following
            .entry(follower.to_string())
            .or_default()
            .insert(target.to_string());
        if inserted {
            self.followers
                .entry(target.to_string())
                .or_default()
                .insert(follower.to_string());
        }
        inserted
    }

    pub fn unfollow(&mut self, follower: &str, target: &str) -> bool {
        let removed = self
            .following
            .get_mut(follower)
            .is_some_and(|set| set.remove(target));
        if removed {
            if let Some(set) = self.followers.get_mut(target) {
                set.remove(follower);
            }
        }
        removed
    }

    pub fn is_following(&self, follower: &str, target: &str) -> bool {
        self.following
            .get(follower)
            .is_some_and(|set| set.contains(target))
    }

    pub fn following_of(&self, actor_id: &str) -> HashSet<String> {
        self.following.get(actor_id).cloned().unwrap_or_default()
    }

    pub fn followers_of(&self, actor_id: &str) -> HashSet<String> {
        self.followers.get(actor_id).cloned().unwrap_or_default()
    }

    /// Computes normalized 2-hop follow affinity weights in `(0.0, 1.0]` for accounts followed by `actor_id`'s followees.
    pub fn second_hop_weights(&self, actor_id: &str) -> HashMap<String, f64> {
        let direct = self.following_of(actor_id);
        if direct.is_empty() {
            return HashMap::new();
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        for friend in &direct {
            if let Some(fof) = self.following.get(friend) {
                for candidate in fof {
                    if candidate != actor_id && !direct.contains(candidate) {
                        *counts.entry(candidate.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
        let denom = direct.len() as f64;
        counts
            .into_iter()
            .map(|(k, c)| (k, (c as f64 / denom).clamp(0.0, 1.0)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_telegram_proto::miniapp_contracts::GROK_AVATAR_SHAPES;

    #[test]
    fn handle_validation_and_normalization() {
        assert_eq!(normalize_handle("  @Alex_Chen42 ").unwrap(), "alex_chen42");
        assert!(normalize_handle("a").is_err());
        assert!(normalize_handle("___").is_err());
        assert!(normalize_handle("bad-handle!").is_err());
        assert!(normalize_handle(&"a".repeat(33)).is_err());
    }

    #[test]
    fn act_as_ownership_and_grok_avatars() {
        let now = 1_760_000_000_000;
        let human = new_human_actor(
            101,
            &RegisterHumanActorRequest {
                handle: "@Alice".into(),
                display_name: "Alice".into(),
                bio: Some("Builder".into()),
                location: None,
                website: None,
                avatar_media_id: None,
                banner_media_id: None,
                avatar_shape: None,
                avatar_color: None,
            },
            now,
        )
        .unwrap();
        assert_eq!(human.actor_id, "u:101");
        assert_eq!(human.avatar_shape, CLASSIC_INITIALS_AVATAR_SHAPE);
        assert!(can_user_act_as(101, &human));
        assert!(validate_act_as(202, &human).is_err());

        let bot = new_bot_actor(
            101,
            9001,
            &RegisterBotActorRequest {
                bot_user_id: Some(9001),
                actor_kind: Some(ActorKind::Bot),
                handle: "ferris_bot".into(),
                display_name: "Ferris Bot".into(),
                bio: Some("Rust assistant".into()),
                avatar_media_id: None,
                banner_media_id: None,
                avatar_shape: None,
                avatar_color: Some("orange".into()),
                bot_badge: None,
                bot_model: Some("hermes-4".into()),
                bot_capabilities: vec!["Rust".into(), "Systems".into()],
                bot_autonomy_mode: BotAutonomyMode::Reactive,
                bot_persona: Some("Systems mentor".into()),
                cooldown_secs: None,
                max_posts_per_hour: None,
            },
            now,
        )
        .unwrap();
        assert_eq!(bot.actor_id, "b:9001");
        assert!(GROK_AVATAR_SHAPES.contains(&bot.avatar_shape.as_str()));
        assert_eq!(bot.avatar_color, "orange");
        assert!(validate_act_as(101, &bot).is_ok());
        assert!(validate_act_as(202, &bot).is_err());
    }

    #[test]
    fn trust_score_bounds_and_two_hop_graph() {
        let now = 1_760_000_000_000;
        let mut actor = new_human_actor(
            7,
            &RegisterHumanActorRequest {
                handle: "tester".into(),
                display_name: "Tester".into(),
                bio: None,
                location: None,
                website: None,
                avatar_media_id: None,
                banner_media_id: None,
                avatar_shape: None,
                avatar_color: None,
            },
            now - 60 * 86_400_000,
        )
        .unwrap();
        apply_karma_delta(&mut actor, KarmaKind::Post, -50_000);
        let low = compute_trust_score(&actor, now);
        assert!((MIN_TRUST_SCORE..=MAX_TRUST_SCORE).contains(&low));

        apply_karma_delta(&mut actor, KarmaKind::Post, 1_000_000);
        actor.followers_count = 500_000;
        actor.verified_tier = VerifiedTier::HumanVerified;
        let high = compute_trust_score(&actor, now);
        assert!((MIN_TRUST_SCORE..=MAX_TRUST_SCORE).contains(&high));
        assert!(high > low);

        let mut g = ActorGraph::new();
        assert!(!g.follow("u:1", "u:1"));
        assert!(g.follow("u:1", "u:2"));
        assert!(g.follow("u:1", "b:3"));
        assert!(g.follow("u:2", "b:4"));
        assert!(g.follow("b:3", "b:4"));
        assert!(g.follow("b:3", "u:5"));
        let w = g.second_hop_weights("u:1");
        assert!((w["b:4"] - 1.0).abs() < 1e-9);
        assert!((w["u:5"] - 0.5).abs() < 1e-9);
        assert!(!w.contains_key("u:2"));
    }

    #[test]
    fn trust_score_handles_i64_min_karma_and_timestamps_without_overflow() {
        let now = 1_760_100_000_000_i64;
        let mut actor = new_human_actor(
            99,
            &RegisterHumanActorRequest {
                handle: "extreme_user".into(),
                display_name: "Extreme".into(),
                bio: None,
                location: None,
                website: None,
                avatar_media_id: None,
                banner_media_id: None,
                avatar_shape: None,
                avatar_color: None,
            },
            i64::MIN,
        )
        .unwrap();
        actor.post_karma = i64::MIN;
        actor.comment_karma = 0;
        actor.awardee_karma = 0;
        let score = compute_trust_score(&actor, now);
        assert!(score.is_finite());
        assert!((MIN_TRUST_SCORE..=MAX_TRUST_SCORE).contains(&score));

        actor.created_at_ms = i64::MAX;
        let future_score = compute_trust_score(&actor, i64::MIN);
        assert!(future_score.is_finite());
        assert!((MIN_TRUST_SCORE..=MAX_TRUST_SCORE).contains(&future_score));
    }
}
