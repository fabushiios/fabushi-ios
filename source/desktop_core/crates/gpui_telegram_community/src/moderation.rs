//! Declarative AutoMod evaluator (keywords, CJK, ReDoS-safe bounded regex, karma/age/link/spam guards) and Mod permission checks.

use crate::model::extract_entities;
use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, AutoModActionKind, AutoModRuleDto, CommunityActorDto, CommunityMemberRole,
    ModActionKind, ModLogEntryDto,
};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

pub const MAX_REGEX_SIZE_BYTES: usize = 64 * 1024;
pub const DUPLICATE_SPAM_JACCARD_THRESHOLD: f32 = 0.88;

/// Outcome produced by `AutoModEngine::evaluate` / `evaluate_with_context`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoModDecision {
    pub action: AutoModActionKind,
    pub matched_rule_ids: Vec<String>,
    pub matched_rule_names: Vec<String>,
    pub reasons: Vec<String>,
    pub assigned_flair: Option<String>,
    pub auto_reply_body: Option<String>,
}

impl Default for AutoModDecision {
    fn default() -> Self {
        Self {
            action: AutoModActionKind::Approve,
            matched_rule_ids: Vec::new(),
            matched_rule_names: Vec::new(),
            reasons: Vec::new(),
            assigned_flair: None,
            auto_reply_body: None,
        }
    }
}

fn action_severity(action: AutoModActionKind) -> u8 {
    match action {
        AutoModActionKind::Approve => 0,
        AutoModActionKind::AssignFlair => 1,
        AutoModActionKind::AutoReply => 2,
        AutoModActionKind::FlagForReview => 3,
        AutoModActionKind::Remove => 4,
    }
}

fn is_ascii_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn contains_keyword(haystack_lower: &str, keyword: &str) -> bool {
    let kw = keyword.trim().to_lowercase();
    if kw.is_empty() {
        return false;
    }
    if !kw.is_ascii() {
        return haystack_lower.contains(&kw);
    }
    let hay_bytes = haystack_lower.as_bytes();
    let kw_bytes = kw.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack_lower[start..].find(&kw) {
        let abs = start + pos;
        let left_ok = abs == 0 || !is_ascii_word_char(hay_bytes[abs - 1]);
        let end = abs + kw_bytes.len();
        let right_ok = end >= hay_bytes.len() || !is_ascii_word_char(hay_bytes[end]);
        if left_ok && right_ok {
            return true;
        }
        start = abs + 1;
        if start >= haystack_lower.len() {
            break;
        }
    }
    false
}

/// Computes normalized 3-gram Jaccard similarity in `[0.0, 1.0]` for spam / echo detection.
pub fn trigram_jaccard_similarity(a: &str, b: &str) -> f32 {
    let norm = |s: &str| -> Vec<char> {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect()
    };
    let ca = norm(a);
    let cb = norm(b);
    if ca.is_empty() || cb.is_empty() {
        return 0.0;
    }
    if ca == cb {
        return 1.0;
    }
    if ca.len() < 3 || cb.len() < 3 {
        return 0.0;
    }
    let sa: HashSet<[char; 3]> = ca.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
    let sb: HashSet<[char; 3]> = cb.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    if union == 0 {
        0.0
    } else {
        (inter as f32) / (union as f32)
    }
}

/// Declarative per-community AutoMod evaluator.
pub struct AutoModEngine;

impl AutoModEngine {
    /// Evaluates AutoMod rules against an actor's submission using current UTC timestamp.
    pub fn evaluate(
        rules: &[AutoModRuleDto],
        actor: &CommunityActorDto,
        title: Option<&str>,
        body: &str,
    ) -> AutoModDecision {
        Self::evaluate_with_context(
            rules,
            actor,
            title,
            body,
            chrono::Utc::now().timestamp_millis(),
            &[],
            None,
        )
    }

    /// Evaluates AutoMod rules with explicit `now_ms`, recent actor bodies (duplicate spam guard), and optional community slug.
    pub fn evaluate_with_context(
        rules: &[AutoModRuleDto],
        actor: &CommunityActorDto,
        title: Option<&str>,
        body: &str,
        now_ms: i64,
        recent_actor_bodies: &[&str],
        community_slug: Option<&str>,
    ) -> AutoModDecision {
        let mut decision = AutoModDecision::default();
        if rules.is_empty() {
            return decision;
        }

        let is_bot = actor.actor_kind != ActorKind::Human;
        let mut active: Vec<&AutoModRuleDto> = rules
            .iter()
            .filter(|r| {
                r.enabled
                    && ((is_bot && r.apply_to_bots) || (!is_bot && r.apply_to_humans))
            })
            .collect();
        active.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.rule_id.cmp(&b.rule_id)));

        let combined = match title.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => format!("{t}\n{body}"),
            None => body.to_string(),
        };
        let combined_lower = combined.to_lowercase();
        let entities = extract_entities(&combined);
        let total_karma = actor.post_karma.saturating_add(actor.comment_karma);
        let account_age_secs = (now_ms.saturating_sub(actor.created_at_ms).max(0)) / 1000;

        for rule in active {
            let mut rule_reasons = Vec::new();

            // 1. Keyword match (word-boundary for ASCII, substring for CJK)
            for kw in &rule.keywords {
                if contains_keyword(&combined_lower, kw) {
                    rule_reasons.push(format!("matched keyword '{kw}'"));
                    break;
                }
            }

            // 2. Bounded ReDoS-safe regex match
            if let Some(pat) = rule.regex_pattern.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                match RegexBuilder::new(pat)
                    .case_insensitive(true)
                    .size_limit(MAX_REGEX_SIZE_BYTES)
                    .dfa_size_limit(MAX_REGEX_SIZE_BYTES)
                    .build()
                {
                    Ok(re) => {
                        if re.is_match(&combined) {
                            rule_reasons.push(format!("matched regex '{}'", rule.name));
                        }
                    }
                    Err(err) => {
                        tracing::warn!(rule_id = %rule.rule_id, error = %err, "skipping invalid/oversized automod regex");
                    }
                }
            }

            // 3. Minimum karma floor
            if let Some(min_k) = rule.min_author_karma {
                if total_karma < min_k {
                    rule_reasons.push(format!("author karma {total_karma} below minimum {min_k}"));
                }
            }

            // 4. Minimum account age floor
            if let Some(min_age) = rule.min_account_age_secs {
                if account_age_secs < min_age {
                    rule_reasons.push(format!(
                        "account age {account_age_secs}s below minimum {min_age}s"
                    ));
                }
            }

            // 5. Link cap
            if let Some(max_links) = rule.max_links {
                if entities.urls.len() > max_links {
                    rule_reasons.push(format!(
                        "contains {} links exceeding limit {max_links}",
                        entities.urls.len()
                    ));
                }
            }

            // 6. Duplicate spam guard
            if rule.block_duplicate_spam && !body.trim().is_empty() {
                if recent_actor_bodies
                    .iter()
                    .any(|prev| trigram_jaccard_similarity(body, prev) >= DUPLICATE_SPAM_JACCARD_THRESHOLD)
                {
                    rule_reasons.push("detected near-duplicate spam submission".to_string());
                }
            }

            if !rule_reasons.is_empty() {
                decision.matched_rule_ids.push(rule.rule_id.clone());
                decision.matched_rule_names.push(rule.name.clone());
                decision.reasons.extend(rule_reasons);

                if action_severity(rule.action) > action_severity(decision.action) {
                    decision.action = rule.action;
                }
                if decision.assigned_flair.is_none() {
                    if let Some(f) = rule.assign_flair.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        decision.assigned_flair = Some(f.to_string());
                    }
                }
                if decision.auto_reply_body.is_none() {
                    if let Some(tmpl) = rule.auto_reply_template.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        let rendered = tmpl
                            .replace("{handle}", &actor.handle)
                            .replace("{community}", community_slug.unwrap_or("community"));
                        decision.auto_reply_body = Some(rendered);
                    }
                }
            }
        }

        if decision.action == AutoModActionKind::Remove {
            decision.auto_reply_body = None;
        }
        decision
    }
}

/// Validates moderator permissions across community roles and builds audit log entries.
pub struct ModPermissionValidator;

impl ModPermissionValidator {
    pub fn can_moderate(role: Option<CommunityMemberRole>) -> bool {
        role.is_some_and(CommunityMemberRole::is_moderator)
    }

    pub fn validate_mod_action(
        moderator_role: Option<CommunityMemberRole>,
        target_role: Option<CommunityMemberRole>,
        action: ModActionKind,
    ) -> Result<()> {
        let mod_role = moderator_role.ok_or_else(|| {
            TelegramError::PermissionDenied("actor is not a community moderator".to_string())
        })?;
        if !mod_role.is_moderator() {
            return Err(TelegramError::PermissionDenied(format!(
                "role '{}' cannot perform moderation actions",
                mod_role.as_str()
            )));
        }
        if matches!(action, ModActionKind::PromoteMod | ModActionKind::DemoteMod)
            && mod_role != CommunityMemberRole::Owner
        {
            return Err(TelegramError::PermissionDenied(
                "only community owners may promote or demote moderators".to_string(),
            ));
        }
        if target_role == Some(CommunityMemberRole::Owner)
            && matches!(
                action,
                ModActionKind::BanActor
                    | ModActionKind::UnbanActor
                    | ModActionKind::MuteActor
                    | ModActionKind::UnmuteActor
                    | ModActionKind::PromoteMod
                    | ModActionKind::DemoteMod
            )
        {
            return Err(TelegramError::PermissionDenied(
                "cannot ban, mute, or change the role of a community owner".to_string(),
            ));
        }
        if matches!(
            target_role,
            Some(CommunityMemberRole::Mod | CommunityMemberRole::BotMod)
        ) && matches!(
            action,
            ModActionKind::BanActor
                | ModActionKind::UnbanActor
                | ModActionKind::MuteActor
                | ModActionKind::UnmuteActor
        ) && mod_role != CommunityMemberRole::Owner
        {
            return Err(TelegramError::PermissionDenied(
                "only community owners may ban or mute a moderator".to_string(),
            ));
        }
        if action == ModActionKind::SetUserFlair
            && mod_role != CommunityMemberRole::Owner
            && matches!(
                target_role,
                Some(
                    CommunityMemberRole::Owner
                        | CommunityMemberRole::Mod
                        | CommunityMemberRole::BotMod
                )
            )
        {
            return Err(TelegramError::PermissionDenied(
                "only community owners may modify the user flair of owners or moderators"
                    .to_string(),
            ));
        }
        Ok(())
    }

    pub fn build_mod_log_entry(
        community_id: &str,
        moderator: &CommunityActorDto,
        action: ModActionKind,
        target_kind: &str,
        target_id: &str,
        reason: Option<&str>,
        rule_triggered: Option<&str>,
        now_ms: i64,
    ) -> ModLogEntryDto {
        ModLogEntryDto {
            log_id: format!("modlog_{}", Uuid::new_v4().simple()),
            community_id: community_id.to_string(),
            moderator_actor_id: moderator.actor_id.clone(),
            moderator_handle: moderator.handle.clone(),
            moderator_kind: moderator.actor_kind,
            action,
            target_kind: target_kind.to_string(),
            target_id: target_id.to_string(),
            reason: reason.map(str::to_string),
            rule_triggered: rule_triggered.map(str::to_string),
            created_at_ms: now_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_telegram_proto::community_contracts::{BotAutonomyMode, VerifiedTier};

    fn actor(kind: ActorKind, karma: i64, created_at_ms: i64) -> CommunityActorDto {
        CommunityActorDto {
            actor_id: if kind.is_bot_like() { "b:1".into() } else { "u:1".into() },
            linked_user_id: Some(1),
            owner_user_id: Some(1),
            actor_kind: kind,
            handle: "tester".into(),
            display_name: "Tester".into(),
            bio: String::new(),
            location: None,
            website: None,
            avatar_media_id: None,
            banner_media_id: None,
            avatar_shape: "classic".into(),
            avatar_color: "blue".into(),
            verified_tier: VerifiedTier::None,
            bot_badge: None,
            bot_model: None,
            bot_capabilities: vec![],
            bot_autonomy_mode: BotAutonomyMode::Manual,
            bot_persona: None,
            post_karma: karma,
            comment_karma: 0,
            awardee_karma: 0,
            followers_count: 0,
            following_count: 0,
            posts_count: 0,
            trust_score: 0.5,
            is_following: false,
            is_muted: false,
            is_blocked: false,
            created_at_ms,
        }
    }

    fn base_rule(id: &str, action: AutoModActionKind) -> AutoModRuleDto {
        AutoModRuleDto {
            rule_id: id.into(),
            community_id: "comm_1".into(),
            name: id.into(),
            enabled: true,
            priority: 10,
            keywords: vec![],
            regex_pattern: None,
            min_author_karma: None,
            min_account_age_secs: None,
            max_links: None,
            block_duplicate_spam: false,
            apply_to_bots: true,
            apply_to_humans: true,
            action,
            assign_flair: None,
            auto_reply_template: None,
            created_at_ms: 0,
        }
    }

    #[test]
    fn automod_evaluates_keywords_cjk_regex_karma_age_links_spam_and_precedence() {
        let now = 1_760_000_000_000_i64;
        let fresh_human = actor(ActorKind::Human, 2, now - 60_000);
        let trusted_bot = actor(ActorKind::Bot, 500, now - 90 * 86_400_000);

        let mut r_flair = base_rule("r_flair", AutoModActionKind::AssignFlair);
        r_flair.keywords = vec!["benchmark".into(), "性能测试".into()];
        r_flair.assign_flair = Some("Benchmarks".into());
        r_flair.auto_reply_template = Some("Thanks @{handle} for posting in c/{community}!".into());

        let mut r_karma = base_rule("r_karma", AutoModActionKind::FlagForReview);
        r_karma.min_author_karma = Some(10);
        r_karma.min_account_age_secs = Some(3600);
        r_karma.apply_to_bots = false; // Humans only

        let mut r_links = base_rule("r_links", AutoModActionKind::Remove);
        r_links.max_links = Some(2);
        r_links.regex_pattern = Some(r"(free\s+nitro|guaranteed\s+profit)".into());
        r_links.block_duplicate_spam = true;

        let mut r_bad_regex = base_rule("r_bad", AutoModActionKind::Remove);
        r_bad_regex.regex_pattern = Some("[unclosed(".into());

        let rules = vec![r_flair, r_karma, r_links, r_bad_regex];

        // 1. Trusted bot posting CJK benchmark gets AssignFlair & template reply, not flagged by human-only karma rule
        let d_bot = AutoModEngine::evaluate_with_context(
            &rules,
            &trusted_bot,
            Some("Rust 性能测试"),
            "Comparing allocator throughput.",
            now,
            &[],
            Some("rust"),
        );
        assert_eq!(d_bot.action, AutoModActionKind::AssignFlair);
        assert_eq!(d_bot.assigned_flair.as_deref(), Some("Benchmarks"));
        assert!(d_bot.auto_reply_body.as_deref().unwrap().contains("@tester"));

        // 2. Fresh human with low karma gets FlagForReview
        let d_human = AutoModEngine::evaluate_with_context(
            &rules,
            &fresh_human,
            Some("Hello"),
            "First post here!",
            now,
            &[],
            Some("rust"),
        );
        assert_eq!(d_human.action, AutoModActionKind::FlagForReview);

        // 3. Duplicate spam or excess links escalates to Remove
        let spam_body = "Check https://a.com https://b.com https://c.com right now!";
        let d_remove = AutoModEngine::evaluate_with_context(
            &rules,
            &trusted_bot,
            None,
            spam_body,
            now,
            &[spam_body],
            Some("rust"),
        );
        assert_eq!(d_remove.action, AutoModActionKind::Remove);
        assert!(d_remove.auto_reply_body.is_none());
    }

    #[test]
    fn mod_permission_validator_enforces_hierarchy() {
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::Member),
            None,
            ModActionKind::Remove
        )
        .is_err());
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::BotMod),
            Some(CommunityMemberRole::Member),
            ModActionKind::Remove
        )
        .is_ok());
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::Mod),
            Some(CommunityMemberRole::Owner),
            ModActionKind::BanActor
        )
        .is_err());
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::Mod),
            Some(CommunityMemberRole::Member),
            ModActionKind::PromoteMod
        )
        .is_err());
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::Owner),
            Some(CommunityMemberRole::Member),
            ModActionKind::PromoteMod
        )
        .is_ok());

        // Peer-mod escalation blocked: Mod and BotMod cannot ban or mute peer Mod/BotMod
        for actor_role in [CommunityMemberRole::Mod, CommunityMemberRole::BotMod] {
            for target_role in [CommunityMemberRole::Mod, CommunityMemberRole::BotMod] {
                for action in [ModActionKind::BanActor, ModActionKind::MuteActor] {
                    assert!(
                        ModPermissionValidator::validate_mod_action(
                            Some(actor_role),
                            Some(target_role),
                            action
                        )
                        .is_err(),
                        "{actor_role:?} must not {action:?} peer {target_role:?}"
                    );
                }
            }
        }
        // Owner CAN ban/mute a Mod or BotMod, but CANNOT ban/mute/demote an Owner
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::Owner),
            Some(CommunityMemberRole::Mod),
            ModActionKind::BanActor
        )
        .is_ok());
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::Owner),
            Some(CommunityMemberRole::BotMod),
            ModActionKind::MuteActor
        )
        .is_ok());
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::Owner),
            Some(CommunityMemberRole::Owner),
            ModActionKind::BanActor
        )
        .is_err());
        assert!(ModPermissionValidator::validate_mod_action(
            Some(CommunityMemberRole::Mod),
            Some(CommunityMemberRole::Owner),
            ModActionKind::UnbanActor
        )
        .is_err());

        // SetUserFlair privilege escalation guard: Mod/BotMod blocked against Owner/Mod/BotMod
        for actor_role in [CommunityMemberRole::Mod, CommunityMemberRole::BotMod] {
            for protected in [
                CommunityMemberRole::Owner,
                CommunityMemberRole::Mod,
                CommunityMemberRole::BotMod,
            ] {
                assert!(
                    ModPermissionValidator::validate_mod_action(
                        Some(actor_role),
                        Some(protected),
                        ModActionKind::SetUserFlair
                    )
                    .is_err(),
                    "{actor_role:?} must not SetUserFlair on {protected:?}"
                );
            }
            for allowed in [
                CommunityMemberRole::Member,
                CommunityMemberRole::Muted,
                CommunityMemberRole::Banned,
            ] {
                assert!(
                    ModPermissionValidator::validate_mod_action(
                        Some(actor_role),
                        Some(allowed),
                        ModActionKind::SetUserFlair
                    )
                    .is_ok(),
                    "{actor_role:?} should be allowed to SetUserFlair on {allowed:?}"
                );
            }
        }
        // Owner remains allowed to SetUserFlair on Owner, Mod, BotMod, and Member
        for target in [
            CommunityMemberRole::Owner,
            CommunityMemberRole::Mod,
            CommunityMemberRole::BotMod,
            CommunityMemberRole::Member,
        ] {
            assert!(ModPermissionValidator::validate_mod_action(
                Some(CommunityMemberRole::Owner),
                Some(target),
                ModActionKind::SetUserFlair
            )
            .is_ok());
        }
    }
}
