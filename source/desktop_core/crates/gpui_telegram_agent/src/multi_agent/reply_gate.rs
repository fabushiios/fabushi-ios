//! Two-stage lightweight `shouldReply` group gate (Grok Bot Jev Pattern).
//!
//! - **Stage 1 (`evaluate_group_wake_gate`)**: Decides which bot(s), if any, should wake up for an
//!   incoming group message. Explicit `@mention`, `@all`, reply-to-bot, and single-bot rooms wake
//!   deterministically; unmentioned messages in multi-bot rooms are scored on lexical name/role/skill
//!   match + conversation floor continuity + cooldown guard, waking only top relevant candidate(s)
//!   above threshold or returning [`RoomMemberTurnOutcome::Pass`] to prevent noisy multi-bot pile-ons.
//! - **Stage 2 (`evaluate_outbound_turn_review`)**: Reviews a bot's drafted response before posting to
//!   the room transcript, detecting explicit silence sentinels (`[PASS]`, `<PASS/>`, `NO_REPLY`),
//!   empty responses, and near-duplicate echoes of recent room messages.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::floor_control::RoomMemberTurnOutcome;

fn default_true() -> bool {
    true
}

fn default_wake_threshold() -> f32 {
    0.35
}

fn default_max_unmentioned_wakes() -> usize {
    1
}

fn default_cooldown_seconds() -> u64 {
    15
}

fn default_duplicate_threshold() -> f32 {
    0.85
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupWakeBotCandidate {
    pub bot_user_id: i64,
    pub agent_id: String,
    pub name: String,
    pub username: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub skills: Vec<String>,
    /// Seconds since this bot last replied in the room (`None` if it has not spoken recently).
    #[serde(default)]
    pub seconds_since_last_reply: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupWakeGateInput {
    pub candidates: Vec<GroupWakeBotCandidate>,
    pub message_text: String,
    #[serde(default)]
    pub explicit_mentions: Vec<String>,
    #[serde(default)]
    pub reply_to_bot_user_id: Option<i64>,
    #[serde(default)]
    pub recent_speaker_bot_user_id: Option<i64>,
    #[serde(default = "default_true")]
    pub sender_is_human: bool,
    pub remaining_loop_budget: u32,
    #[serde(default = "default_wake_threshold")]
    pub wake_threshold: f32,
    #[serde(default = "default_max_unmentioned_wakes")]
    pub max_unmentioned_wakes: usize,
    #[serde(default = "default_cooldown_seconds")]
    pub cooldown_seconds: u64,
}

impl Default for GroupWakeGateInput {
    fn default() -> Self {
        Self {
            candidates: Vec::new(),
            message_text: String::new(),
            explicit_mentions: Vec::new(),
            reply_to_bot_user_id: None,
            recent_speaker_bot_user_id: None,
            sender_is_human: true,
            remaining_loop_budget: 5,
            wake_threshold: default_wake_threshold(),
            max_unmentioned_wakes: default_max_unmentioned_wakes(),
            cooldown_seconds: default_cooldown_seconds(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupWakeBotScore {
    pub bot_user_id: i64,
    pub agent_id: String,
    pub score: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupWakeReason {
    ExplicitMention,
    MentionAll,
    ReplyToBot,
    SingleBotRoom,
    RelevanceMatch,
    PrimaryFallback,
    BudgetExhausted,
    NoRelevantCandidate,
    EmptyCandidates,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupWakeGateDecision {
    pub should_wake: bool,
    pub outcome: RoomMemberTurnOutcome,
    pub wake_bot_user_ids: Vec<i64>,
    pub reason: GroupWakeReason,
    pub scores: Vec<GroupWakeBotScore>,
    pub is_winding_down: bool,
}

/// Stage 1 lightweight wake classifier deciding which candidate bot(s) should take a turn.
pub fn evaluate_group_wake_gate(input: &GroupWakeGateInput) -> GroupWakeGateDecision {
    let is_winding_down = input.remaining_loop_budget <= 1;

    if input.candidates.is_empty() {
        return GroupWakeGateDecision {
            should_wake: false,
            outcome: RoomMemberTurnOutcome::Skipped,
            wake_bot_user_ids: Vec::new(),
            reason: GroupWakeReason::EmptyCandidates,
            scores: Vec::new(),
            is_winding_down,
        };
    }

    if input.remaining_loop_budget == 0 && !input.sender_is_human {
        return GroupWakeGateDecision {
            should_wake: false,
            outcome: RoomMemberTurnOutcome::Skipped,
            wake_bot_user_ids: Vec::new(),
            reason: GroupWakeReason::BudgetExhausted,
            scores: Vec::new(),
            is_winding_down: true,
        };
    }

    // Collect explicit mentions from structured input + inline `@tokens` in message_text.
    let mentions = collect_normalized_mentions(&input.explicit_mentions, &input.message_text);

    // Rule 1: `@all` / `@everyone` / `@所有人` wakes all candidates deterministically.
    if mentions
        .iter()
        .any(|m| matches!(m.as_str(), "all" | "everyone" | "here" | "所有人"))
    {
        let scores = input
            .candidates
            .iter()
            .map(|c| GroupWakeBotScore {
                bot_user_id: c.bot_user_id,
                agent_id: c.agent_id.clone(),
                score: 1.0,
                reasons: vec!["mention_all".to_string()],
            })
            .collect();
        return GroupWakeGateDecision {
            should_wake: true,
            outcome: RoomMemberTurnOutcome::Sent,
            wake_bot_user_ids: input.candidates.iter().map(|c| c.bot_user_id).collect(),
            reason: GroupWakeReason::MentionAll,
            scores,
            is_winding_down,
        };
    }

    // Rule 2: Explicit `@mention` matching one or more candidate bots.
    if !mentions.is_empty() || input.message_text.contains('@') {
        let mut matched_ids = Vec::new();
        let mut scores = Vec::new();
        for c in &input.candidates {
            if candidate_matches_mentions(c, &mentions, &input.message_text) {
                matched_ids.push(c.bot_user_id);
                scores.push(GroupWakeBotScore {
                    bot_user_id: c.bot_user_id,
                    agent_id: c.agent_id.clone(),
                    score: 1.0,
                    reasons: vec!["explicit_mention".to_string()],
                });
            }
        }
        if !matched_ids.is_empty() {
            return GroupWakeGateDecision {
                should_wake: true,
                outcome: RoomMemberTurnOutcome::Sent,
                wake_bot_user_ids: matched_ids,
                reason: GroupWakeReason::ExplicitMention,
                scores,
                is_winding_down,
            };
        }
    }

    // Rule 3: Direct reply to a candidate bot's message.
    if let Some(reply_bot_id) = input.reply_to_bot_user_id {
        if let Some(target) = input
            .candidates
            .iter()
            .find(|c| c.bot_user_id == reply_bot_id)
        {
            return GroupWakeGateDecision {
                should_wake: true,
                outcome: RoomMemberTurnOutcome::Sent,
                wake_bot_user_ids: vec![target.bot_user_id],
                reason: GroupWakeReason::ReplyToBot,
                scores: vec![GroupWakeBotScore {
                    bot_user_id: target.bot_user_id,
                    agent_id: target.agent_id.clone(),
                    score: 1.0,
                    reasons: vec!["reply_to_bot".to_string()],
                }],
                is_winding_down,
            };
        }
    }

    // Rule 4: Single-bot room wakes deterministically (unless bot-to-bot budget was already exhausted above).
    if input.candidates.len() == 1 {
        let sole = &input.candidates[0];
        return GroupWakeGateDecision {
            should_wake: true,
            outcome: RoomMemberTurnOutcome::Sent,
            wake_bot_user_ids: vec![sole.bot_user_id],
            reason: GroupWakeReason::SingleBotRoom,
            scores: vec![GroupWakeBotScore {
                bot_user_id: sole.bot_user_id,
                agent_id: sole.agent_id.clone(),
                score: 1.0,
                reasons: vec!["single_bot_room".to_string()],
            }],
            is_winding_down,
        };
    }

    // Rule 5: Unmentioned message in a multi-bot room — lexical + skill + continuity + cooldown scoring.
    let trimmed_msg = input.message_text.trim();
    if trimmed_msg.is_empty() || is_low_signal_acknowledgement(trimmed_msg) {
        let scores = input
            .candidates
            .iter()
            .map(|c| GroupWakeBotScore {
                bot_user_id: c.bot_user_id,
                agent_id: c.agent_id.clone(),
                score: 0.0,
                reasons: vec!["low_signal_chatter".to_string()],
            })
            .collect();
        return GroupWakeGateDecision {
            should_wake: false,
            outcome: RoomMemberTurnOutcome::Pass,
            wake_bot_user_ids: Vec::new(),
            reason: GroupWakeReason::NoRelevantCandidate,
            scores,
            is_winding_down,
        };
    }

    let msg_lower = trimmed_msg.to_lowercase();
    let msg_tokens = extract_keywords(&msg_lower);
    let looks_like_followup = is_followup_cue(&msg_lower, trimmed_msg);

    let mut scored: Vec<GroupWakeBotScore> = input
        .candidates
        .iter()
        .map(|cand| {
            let mut score = 0.0f32;
            let mut reasons = Vec::new();

            // (a) Direct un-prefixed name / username / agent_id lexical match
            let name_lower = cand.name.trim().to_lowercase();
            let user_lower = cand.username.trim().trim_start_matches('@').to_lowercase();
            let agent_lower = cand.agent_id.trim().to_lowercase();
            if (!name_lower.is_empty() && name_lower.chars().count() >= 2 && contains_term(&msg_lower, &name_lower))
                || (!user_lower.is_empty() && user_lower.chars().count() >= 2 && contains_term(&msg_lower, &user_lower))
                || (!agent_lower.is_empty() && agent_lower.chars().count() >= 3 && contains_term(&msg_lower, &agent_lower))
            {
                score += 0.55;
                reasons.push("name_lexical_match".to_string());
            }

            // (b) Skill match (exact phrase or keyword overlap)
            let mut skill_bonus = 0.0f32;
            for skill in &cand.skills {
                let sk_lower = skill.trim().to_lowercase();
                if sk_lower.is_empty() {
                    continue;
                }
                if contains_term(&msg_lower, &sk_lower) {
                    skill_bonus += 0.42;
                } else {
                    let sk_tokens = extract_keywords(&sk_lower);
                    let overlap = sk_tokens
                        .iter()
                        .filter(|t| msg_tokens.contains(*t))
                        .count();
                    if overlap > 0 {
                        skill_bonus += (0.20 * overlap as f32).min(0.35);
                    }
                }
            }
            if skill_bonus > 0.0 {
                let capped = skill_bonus.min(0.55);
                score += capped;
                reasons.push(format!("skill_match(+{capped:.2})"));
            }

            // (c) Role & description lexical overlap
            let profile_text = format!("{} {}", cand.role, cand.description).to_lowercase();
            let profile_tokens = extract_keywords(&profile_text);
            if !profile_tokens.is_empty() && !msg_tokens.is_empty() {
                let hits = msg_tokens
                    .iter()
                    .filter(|tok| profile_tokens.contains(*tok))
                    .count();
                if hits > 0 {
                    let ratio = hits as f32 / (msg_tokens.len().min(6) as f32);
                    let role_bonus = (0.18 + ratio * 0.30).min(0.45);
                    score += role_bonus;
                    reasons.push(format!("role_description_match({hits} hits, +{role_bonus:.2})"));
                }
            }

            // (d) Conversation floor continuity
            let is_recent_speaker = input.recent_speaker_bot_user_id == Some(cand.bot_user_id);
            if is_recent_speaker && input.sender_is_human && looks_like_followup {
                score += 0.38;
                reasons.push("floor_continuity".to_string());
            } else if is_recent_speaker && input.sender_is_human {
                score += 0.15;
                reasons.push("recent_speaker_hint".to_string());
            }

            // (e) Cooldown guard to prevent one bot from dominating or rapid self-loops
            if let Some(secs) = cand.seconds_since_last_reply {
                if secs < input.cooldown_seconds {
                    if !input.sender_is_human {
                        score -= 0.35;
                        reasons.push("bot_loop_cooldown_guard".to_string());
                    } else if !is_recent_speaker || !looks_like_followup {
                        score -= 0.20;
                        reasons.push("cooldown_guard".to_string());
                    }
                }
            }

            let clamped = score.clamp(0.0, 1.0);
            GroupWakeBotScore {
                bot_user_id: cand.bot_user_id,
                agent_id: cand.agent_id.clone(),
                score: clamped,
                reasons,
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.bot_user_id.cmp(&b.bot_user_id))
    });

    let threshold = input.wake_threshold.clamp(0.0, 1.0);
    let max_wakes = input.max_unmentioned_wakes.max(1);
    let wake_bot_user_ids: Vec<i64> = scored
        .iter()
        .filter(|s| s.score >= threshold)
        .take(max_wakes)
        .map(|s| s.bot_user_id)
        .collect();

    if wake_bot_user_ids.is_empty() {
        // When a human sends a substantive message (not empty / not low-signal acknowledgement)
        // to a group with AI bots and no specialist matched the lexical threshold, wake ONE
        // deterministic primary responder instead of going completely silent:
        //   1. Top partially-scored candidate (score > 0.0)
        //   2. Recent speaker bot (conversation continuity)
        //   3. Official default generalist bot (777_000) if present
        //   4. First candidate in room order
        // Bot-to-bot unmentioned messages with no relevant match still stay silent (Pass).
        if input.sender_is_human {
            let chosen_id = scored
                .first()
                .filter(|s| s.score > 0.0)
                .map(|s| s.bot_user_id)
                .or_else(|| {
                    input.recent_speaker_bot_user_id.filter(|rid| {
                        input.candidates.iter().any(|c| c.bot_user_id == *rid)
                    })
                })
                .or_else(|| {
                    input
                        .candidates
                        .iter()
                        .find(|c| c.bot_user_id == crate::fabushi_bot::OFFICIAL_DEFAULT_BOT_USER_ID)
                        .map(|c| c.bot_user_id)
                })
                .or_else(|| input.candidates.first().map(|c| c.bot_user_id));

            if let Some(primary_id) = chosen_id {
                return GroupWakeGateDecision {
                    should_wake: true,
                    outcome: RoomMemberTurnOutcome::Sent,
                    wake_bot_user_ids: vec![primary_id],
                    reason: GroupWakeReason::PrimaryFallback,
                    scores: scored,
                    is_winding_down,
                };
            }
        }
        GroupWakeGateDecision {
            should_wake: false,
            outcome: RoomMemberTurnOutcome::Pass,
            wake_bot_user_ids,
            reason: GroupWakeReason::NoRelevantCandidate,
            scores: scored,
            is_winding_down,
        }
    } else {
        GroupWakeGateDecision {
            should_wake: true,
            outcome: RoomMemberTurnOutcome::Sent,
            wake_bot_user_ids,
            reason: GroupWakeReason::RelevanceMatch,
            scores: scored,
            is_winding_down,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboundTurnReviewInput {
    pub agent_id: String,
    pub candidate_text: String,
    #[serde(default)]
    pub recent_room_messages: Vec<String>,
    #[serde(default = "default_duplicate_threshold")]
    pub duplicate_similarity_threshold: f32,
}

impl Default for OutboundTurnReviewInput {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            candidate_text: String::new(),
            recent_room_messages: Vec::new(),
            duplicate_similarity_threshold: default_duplicate_threshold(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundReviewReason {
    Approved,
    EmptyResponse,
    ExplicitPassSentinel,
    DuplicateEcho,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboundTurnVerdict {
    pub outcome: RoomMemberTurnOutcome,
    pub should_send: bool,
    pub reason: OutboundReviewReason,
    pub sanitized_text: String,
    pub max_duplicate_similarity: f32,
}

/// Stage 2 outbound chat review gate checking silence sentinels (`[PASS]`, `<PASS/>`, `NO_REPLY`),
/// empty outputs, and near-duplicate echoes against recent room messages.
pub fn evaluate_outbound_turn_review(input: &OutboundTurnReviewInput) -> OutboundTurnVerdict {
    let trimmed = input.candidate_text.trim();
    if trimmed.is_empty() {
        return OutboundTurnVerdict {
            outcome: RoomMemberTurnOutcome::Pass,
            should_send: false,
            reason: OutboundReviewReason::EmptyResponse,
            sanitized_text: String::new(),
            max_duplicate_similarity: 0.0,
        };
    }

    if is_explicit_pass_sentinel(trimmed) {
        return OutboundTurnVerdict {
            outcome: RoomMemberTurnOutcome::Pass,
            should_send: false,
            reason: OutboundReviewReason::ExplicitPassSentinel,
            sanitized_text: String::new(),
            max_duplicate_similarity: 0.0,
        };
    }

    let threshold = input.duplicate_similarity_threshold.clamp(0.1, 1.0);
    let mut max_sim = 0.0f32;
    for recent in &input.recent_room_messages {
        let sim = text_similarity(trimmed, recent);
        if sim > max_sim {
            max_sim = sim;
        }
    }

    if max_sim >= threshold {
        return OutboundTurnVerdict {
            outcome: RoomMemberTurnOutcome::Pass,
            should_send: false,
            reason: OutboundReviewReason::DuplicateEcho,
            sanitized_text: String::new(),
            max_duplicate_similarity: max_sim,
        };
    }

    OutboundTurnVerdict {
        outcome: RoomMemberTurnOutcome::Sent,
        should_send: true,
        reason: OutboundReviewReason::Approved,
        sanitized_text: trimmed.to_string(),
        max_duplicate_similarity: max_sim,
    }
}

/// Returns `true` when the agent's text is an explicit stay-silent sentinel (`[PASS]`, `<PASS/>`, `NO_REPLY`).
pub fn is_explicit_pass_sentinel(raw: &str) -> bool {
    let trimmed = raw
        .trim()
        .trim_matches('`')
        .trim()
        .trim_matches(|c: char| matches!(c, '*' | '_' | '"'));
    let upper = trimmed.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "[PASS]"
            | "<PASS/>"
            | "<PASS />"
            | "<PASS></PASS>"
            | "NO_REPLY"
            | "[NO_REPLY]"
            | "<NO_REPLY/>"
            | "<NO_REPLY />"
            | "__PASS__"
            | "PASS"
            | "PASS."
    ) {
        return true;
    }

    // Leading sentinel token followed by optional brief internal note (e.g. `[PASS] already answered`).
    for prefix in ["[PASS]", "<PASS/>", "<PASS />", "NO_REPLY", "[NO_REPLY]"] {
        if let Some(rest) = upper.strip_prefix(prefix) {
            if rest.is_empty()
                || rest.starts_with(char::is_whitespace)
                || rest.starts_with(':')
                || rest.starts_with('-')
                || rest.starts_with('—')
            {
                return true;
            }
        }
    }

    false
}

fn collect_normalized_mentions(explicit: &[String], text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in explicit {
        let norm = raw.trim().trim_start_matches('@').to_lowercase();
        if !norm.is_empty() && seen.insert(norm.clone()) {
            out.push(norm);
        }
    }

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i].1 == '@' {
            let prev_is_slash_cmd = i > 0 && {
                let mut k = i;
                while k > 0 && (chars[k - 1].1.is_ascii_alphanumeric() || chars[k - 1].1 == '_') {
                    k -= 1;
                }
                k < i
                    && k > 0
                    && chars[k - 1].1 == '/'
                    && (k == 1 || chars[k - 2].1.is_whitespace())
            };
            let prev_ident = i > 0
                && !prev_is_slash_cmd
                && (chars[i - 1].1.is_ascii_alphanumeric() || chars[i - 1].1 == '_');
            if !prev_ident {
                let start_idx = i + 1;
                if start_idx < chars.len() {
                    let rest = &text[chars[start_idx].0..];
                    if rest.starts_with("所有人") && seen.insert("所有人".to_string()) {
                        out.push("所有人".to_string());
                    }
                    let first_is_ascii_ident = chars[start_idx].1.is_ascii_alphanumeric()
                        || chars[start_idx].1 == '_'
                        || chars[start_idx].1 == '-';
                    let mut end_idx = start_idx;
                    if first_is_ascii_ident {
                        while end_idx < chars.len()
                            && (chars[end_idx].1.is_ascii_alphanumeric()
                                || chars[end_idx].1 == '_'
                                || chars[end_idx].1 == '-')
                        {
                            end_idx += 1;
                        }
                    } else {
                        while end_idx < chars.len()
                            && (chars[end_idx].1.is_alphanumeric()
                                || chars[end_idx].1 == '_'
                                || chars[end_idx].1 == '-')
                        {
                            end_idx += 1;
                        }
                    }
                    if end_idx > start_idx {
                        let byte_start = chars[start_idx].0;
                        let byte_end = if end_idx < chars.len() {
                            chars[end_idx].0
                        } else {
                            text.len()
                        };
                        let norm = text[byte_start..byte_end].to_lowercase();
                        if !norm.is_empty() && seen.insert(norm.clone()) {
                            out.push(norm);
                        }
                    }
                    i = end_idx;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn text_has_at_mention_label(text: &str, label_lower: &str) -> bool {
    if label_lower.is_empty() || !text.contains('@') {
        return false;
    }
    let text_lower = text.to_lowercase();
    let bytes = text_lower.as_bytes();
    let label_ends_ascii = label_lower
        .chars()
        .last()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let mut search_from = 0usize;
    while let Some(rel) = text_lower[search_from..].find('@') {
        let at_idx = search_from + rel;
        let prev_ident = at_idx > 0
            && (bytes[at_idx - 1].is_ascii_alphanumeric() || bytes[at_idx - 1] == b'_');
        if !prev_ident {
            let after = &text_lower[at_idx + 1..];
            if let Some(rem) = after.strip_prefix(label_lower) {
                if !label_ends_ascii
                    || rem
                        .chars()
                        .next()
                        .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
                {
                    return true;
                }
            }
        }
        search_from = at_idx + 1;
    }
    false
}

fn candidate_matches_mentions(
    cand: &GroupWakeBotCandidate,
    mentions: &[String],
    message_text: &str,
) -> bool {
    let username = cand.username.trim().trim_start_matches('@').to_lowercase();
    let agent_id = cand.agent_id.trim().to_lowercase();
    let name = cand.name.trim().to_lowercase();
    let id_str = cand.bot_user_id.to_string();

    mentions.iter().any(|m| {
        (!username.is_empty() && *m == username)
            || (!agent_id.is_empty() && *m == agent_id)
            || (!name.is_empty() && *m == name)
            || *m == id_str
    }) || (!name.is_empty() && text_has_at_mention_label(message_text, &name))
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF | 0xAC00..=0xD7AF
    )
}

fn is_stopword(tok: &str) -> bool {
    matches!(
        tok,
        "the"
            | "and"
            | "for"
            | "with"
            | "that"
            | "this"
            | "from"
            | "have"
            | "are"
            | "was"
            | "were"
            | "you"
            | "your"
            | "can"
            | "could"
            | "would"
            | "should"
            | "please"
            | "help"
            | "about"
            | "into"
            | "just"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "how"
            | "why"
            | "的"
            | "了"
            | "是"
            | "在"
            | "我"
            | "你"
            | "他"
            | "我们"
            | "你们"
            | "一下"
            | "一个"
            | "这个"
            | "那个"
            | "可以"
            | "帮忙"
            | "请问"
    )
}

fn extract_keywords(lower: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    let mut ascii_buf = String::new();
    let mut cjk_buf: Vec<char> = Vec::new();

    let flush_ascii = |buf: &mut String, out: &mut HashSet<String>| {
        if buf.len() >= 2 && !is_stopword(buf.as_str()) {
            out.insert(buf.clone());
        }
        buf.clear();
    };
    let flush_cjk = |buf: &mut Vec<char>, out: &mut HashSet<String>| {
        if buf.len() >= 2 {
            for win in buf.windows(2) {
                let s: String = win.iter().collect();
                if !is_stopword(&s) {
                    out.insert(s);
                }
            }
        }
        buf.clear();
    };

    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            flush_cjk(&mut cjk_buf, &mut set);
            ascii_buf.push(ch);
        } else if is_cjk(ch) {
            flush_ascii(&mut ascii_buf, &mut set);
            cjk_buf.push(ch);
        } else {
            flush_ascii(&mut ascii_buf, &mut set);
            flush_cjk(&mut cjk_buf, &mut set);
        }
    }
    flush_ascii(&mut ascii_buf, &mut set);
    flush_cjk(&mut cjk_buf, &mut set);
    set
}

fn contains_term(haystack_lower: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return false;
    }
    if needle_lower.chars().any(is_cjk) {
        return haystack_lower.contains(needle_lower);
    }
    let bytes = haystack_lower.as_bytes();
    let mut start = 0usize;
    while let Some(rel) = haystack_lower[start..].find(needle_lower) {
        let idx = start + rel;
        let end = idx + needle_lower.len();
        let left_ok = idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        let step = haystack_lower[idx..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
        start = idx + step;
    }
    false
}

fn is_low_signal_acknowledgement(trimmed: &str) -> bool {
    let lower = trimmed
        .to_lowercase()
        .trim_matches(|c: char| matches!(c, '!' | '.' | '。' | '！' | '~' | '～' | ',' | '，' | ' '))
        .to_string();
    if matches!(
        lower.as_str(),
        "ok" | "okay"
            | "k"
            | "kk"
            | "yes"
            | "yep"
            | "yup"
            | "no"
            | "nope"
            | "lol"
            | "haha"
            | "thanks"
            | "thank you"
            | "thx"
            | "ty"
            | "nice"
            | "cool"
            | "got it"
            | "sounds good"
            | "ok thanks"
            | "okay thanks"
            | "好的"
            | "好"
            | "收到"
            | "嗯"
            | "嗯嗯"
            | "谢谢"
            | "感谢"
            | "多谢"
            | "哈哈"
            | "哈哈哈"
            | "了解"
            | "明白了"
            | "知道了"
            | "👍"
            | "👌"
            | "🙏"
    ) {
        return true;
    }

    let parts: Vec<&str> = lower
        .split(|c: char| {
            c.is_whitespace() || matches!(c, ',' | '，' | '!' | '！' | '.' | '。' | '~' | '～')
        })
        .filter(|p| !p.is_empty())
        .collect();
    !parts.is_empty()
        && parts.len() <= 3
        && parts.iter().all(|p| {
            matches!(
                *p,
                "ok" | "okay"
                    | "k"
                    | "kk"
                    | "yes"
                    | "yep"
                    | "yup"
                    | "sure"
                    | "thanks"
                    | "thank"
                    | "you"
                    | "thx"
                    | "ty"
                    | "nice"
                    | "cool"
                    | "lol"
                    | "haha"
                    | "好的"
                    | "好"
                    | "收到"
                    | "嗯"
                    | "嗯嗯"
                    | "谢谢"
                    | "感谢"
                    | "多谢"
                    | "哈哈"
                    | "哈哈哈"
                    | "了解"
                    | "明白"
                    | "明白了"
                    | "知道了"
                    | "👍"
                    | "👌"
                    | "🙏"
            )
        })
}

fn is_followup_cue(lower: &str, raw: &str) -> bool {
    raw.contains('?')
        || raw.contains('？')
        || [
            "why",
            "how",
            "what about",
            "more",
            "continue",
            "go on",
            "explain",
            "details",
            "继续",
            "详细",
            "为什么",
            "怎么",
            "然后",
            "那",
            "还有",
            "展开",
        ]
        .iter()
        .any(|cue| lower.contains(cue))
}

fn normalize_for_similarity(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || is_cjk(*c) || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn char_ngrams(text: &str, n: usize) -> HashSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut set = HashSet::new();
    if chars.len() < n {
        if !text.is_empty() {
            set.insert(text.to_string());
        }
        return set;
    }
    for win in chars.windows(n) {
        set.insert(win.iter().collect());
    }
    set
}

fn text_similarity(a: &str, b: &str) -> f32 {
    let na = normalize_for_similarity(a);
    let nb = normalize_for_similarity(b);
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }
    if na == nb {
        return 1.0;
    }
    let ga = char_ngrams(&na, 3);
    let gb = char_ngrams(&nb, 3);
    let inter = ga.intersection(&gb).count() as f32;
    let union = ga.union(&gb).count() as f32;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}
