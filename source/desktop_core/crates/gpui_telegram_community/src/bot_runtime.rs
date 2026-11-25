//! Persona-aware deterministic Bot runtime, wake selector, cooldown, and 5-hop anti-loop guard.

use crate::model::extract_entities;
use crate::moderation::trigram_jaccard_similarity;
use crate::recommender::signals::fnv1a_64;
use gpui_telegram_proto::community_contracts::{
    ActorKind, BotAutonomyMode, CommunityActorDto, CommunityCommentDto, CommunityPostDto,
    SubCommunityDto,
};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};

pub const DEFAULT_BOT_MAX_LOOP_HOPS: u32 = 5;
pub const DEFAULT_BOT_COOLDOWN_MS: i64 = 30_000;
pub const DEFAULT_BOT_DUPLICATE_THRESHOLD: f32 = 0.85;

/// Reasons why the bot anti-loop / cooldown guard declined a bot turn.
#[derive(Debug, Clone, PartialEq)]
pub enum BotGuardRejection {
    LoopBudgetExceeded { hops: u32, max_hops: u32 },
    CooldownActive { remaining_ms: i64 },
    SelfTriggerBlocked,
    DuplicateEcho { similarity: f32 },
    BotsDisallowedInCommunity,
    BotInManualMode,
}

#[derive(Debug, Default)]
struct LoopGuardState {
    thread_hops: HashMap<String, u32>,
    last_reply_ms: HashMap<(String, String), i64>,
}

/// Thread-safe anti-loop budget & per-(bot, thread) cooldown guard.
#[derive(Debug)]
pub struct BotLoopGuard {
    max_hops: u32,
    cooldown_ms: i64,
    state: Mutex<LoopGuardState>,
}

impl Default for BotLoopGuard {
    fn default() -> Self {
        Self::new(DEFAULT_BOT_MAX_LOOP_HOPS, DEFAULT_BOT_COOLDOWN_MS)
    }
}

impl BotLoopGuard {
    pub fn new(max_hops: u32, cooldown_ms: i64) -> Self {
        Self {
            max_hops: max_hops.max(1),
            cooldown_ms: cooldown_ms.max(0),
            state: Mutex::new(LoopGuardState::default()),
        }
    }

    pub fn max_hops(&self) -> u32 {
        self.max_hops
    }

    /// Resets the consecutive bot-to-bot hop counter when a Human posts in the thread.
    pub fn reset_on_human_turn(&self, thread_key: &str) {
        self.state.lock().thread_hops.insert(thread_key.to_string(), 0);
    }

    /// Current consecutive bot hop count for a thread.
    pub fn current_hops(&self, thread_key: &str) -> u32 {
        self.state
            .lock()
            .thread_hops
            .get(thread_key)
            .copied()
            .unwrap_or(0)
    }

    /// Attempts to acquire a reply turn for `bot_actor_id` in `thread_key`.
    pub fn try_acquire_turn(
        &self,
        thread_key: &str,
        bot_actor_id: &str,
        trigger_actor_id: &str,
        trigger_kind: ActorKind,
        explicit_human_summon: bool,
        now_ms: i64,
    ) -> Result<u32, BotGuardRejection> {
        if bot_actor_id == trigger_actor_id && !explicit_human_summon {
            return Err(BotGuardRejection::SelfTriggerBlocked);
        }

        let mut guard = self.state.lock();
        let cooldown_key = (bot_actor_id.to_string(), thread_key.to_string());
        if !explicit_human_summon && self.cooldown_ms > 0 {
            if let Some(&last_ts) = guard.last_reply_ms.get(&cooldown_key) {
                if now_ms < last_ts {
                    return Err(BotGuardRejection::CooldownActive {
                        remaining_ms: self.cooldown_ms,
                    });
                }
                let elapsed = now_ms.saturating_sub(last_ts);
                if elapsed < self.cooldown_ms {
                    return Err(BotGuardRejection::CooldownActive {
                        remaining_ms: self.cooldown_ms.saturating_sub(elapsed),
                    });
                }
            }
        }

        let next_hops = if trigger_kind == ActorKind::Human || explicit_human_summon {
            0
        } else {
            let cur = guard.thread_hops.get(thread_key).copied().unwrap_or(0);
            if cur >= self.max_hops {
                return Err(BotGuardRejection::LoopBudgetExceeded {
                    hops: cur,
                    max_hops: self.max_hops,
                });
            }
            cur + 1
        };

        guard.thread_hops.insert(thread_key.to_string(), next_hops);
        guard.last_reply_ms.insert(cooldown_key, now_ms);
        Ok(next_hops)
    }

    /// Rejects near-duplicate bot echo replies using 3-gram Jaccard similarity.
    pub fn check_not_duplicate_echo(
        candidate: &str,
        recent_replies: &[&str],
        threshold: f32,
    ) -> Result<(), BotGuardRejection> {
        for prev in recent_replies {
            let sim = trigram_jaccard_similarity(candidate, prev);
            if sim >= threshold {
                return Err(BotGuardRejection::DuplicateEcho { similarity: sim });
            }
        }
        Ok(())
    }
}

fn is_low_signal_chatter(text: &str) -> bool {
    let trimmed = text.trim().to_lowercase();
    if trimmed.chars().count() < 6 {
        return true;
    }
    matches!(
        trimmed.as_str(),
        "ok" | "okay" | "thanks" | "thank you" | "lol" | "nice" | "+1" | "agreed" | "bump" | "thx"
    )
}

fn lexical_terms(text: &str) -> HashSet<String> {
    let lower = text.to_lowercase();
    let mut terms: HashSet<String> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.chars().count() >= 2)
        .map(str::to_string)
        .collect();
    // Add CJK bigrams for multilingual wake matching
    let cjk: Vec<char> = lower
        .chars()
        .filter(|c| matches!(*c as u32, 0x3400..=0x9FFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF))
        .collect();
    for pair in cjk.windows(2) {
        terms.insert(format!("{}{}", pair[0], pair[1]));
    }
    terms
}

/// Selects eligible first-class bots to wake for a post or comment event.
pub struct BotWakeSelector;

impl BotWakeSelector {
    pub fn select_bots_for_event(
        bots: &[CommunityActorDto],
        post: &CommunityPostDto,
        trigger_text: &str,
        trigger_actor: &CommunityActorDto,
        community: Option<&SubCommunityDto>,
        max_wakes: usize,
    ) -> Vec<CommunityActorDto> {
        if max_wakes == 0 || community.is_some_and(|c| !c.allow_bots) {
            return Vec::new();
        }
        let entities = extract_entities(trigger_text);
        let mention_set: HashSet<&str> = entities.mentions.iter().map(String::as_str).collect();

        let mut selected = Vec::new();
        let mut selected_ids = HashSet::new();

        // 1. Explicit @mentions wake MentionOnly | Reactive | Autonomous (never Manual or self).
        for bot in bots {
            if !bot.actor_kind.is_bot_like()
                || bot.actor_id == trigger_actor.actor_id
                || bot.bot_autonomy_mode == BotAutonomyMode::Manual
            {
                continue;
            }
            if mention_set.contains(bot.handle.to_ascii_lowercase().as_str())
                && selected_ids.insert(bot.actor_id.clone())
            {
                selected.push(bot.clone());
                if selected.len() >= max_wakes {
                    return selected;
                }
            }
        }

        if is_low_signal_chatter(trigger_text) {
            return selected;
        }

        // 2. Proactive lexical overlap wake for Reactive / Autonomous specialists
        let context_text = format!(
            "{} {} {}",
            post.community_slug.as_deref().unwrap_or(""),
            post.title.as_deref().unwrap_or(""),
            trigger_text
        );
        let ctx_terms = lexical_terms(&context_text);
        if ctx_terms.is_empty() {
            return selected;
        }

        let mut scored_candidates: Vec<(f64, &CommunityActorDto)> = Vec::new();
        for bot in bots {
            if !bot.actor_kind.is_bot_like()
                || bot.actor_id == trigger_actor.actor_id
                || selected_ids.contains(&bot.actor_id)
                || !matches!(
                    bot.bot_autonomy_mode,
                    BotAutonomyMode::Reactive | BotAutonomyMode::Autonomous
                )
            {
                continue;
            }
            let profile_text = format!(
                "{} {} {}",
                bot.bot_capabilities.join(" "),
                bot.bot_persona.as_deref().unwrap_or(""),
                bot.bio
            );
            let bot_terms = lexical_terms(&profile_text);
            if bot_terms.is_empty() {
                continue;
            }
            let cap_hits = bot
                .bot_capabilities
                .iter()
                .filter(|cap| ctx_terms.contains(&cap.to_lowercase()))
                .count();
            let overlap = bot_terms.intersection(&ctx_terms).count();
            let ratio = (overlap as f64) / (bot_terms.len().min(12) as f64)
                + (cap_hits as f64) * 0.35;
            if ratio >= 0.30 {
                scored_candidates.push((ratio, bot));
            }
        }
        scored_candidates.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| a.1.actor_id.cmp(&b.1.actor_id))
        });

        for (_, bot) in scored_candidates {
            if selected.len() >= max_wakes {
                break;
            }
            if selected_ids.insert(bot.actor_id.clone()) {
                selected.push(bot.clone());
            }
        }
        selected
    }
}

/// Deterministic, zero-API-key persona responder for first-class community bots.
pub struct DeterministicBotResponder;

impl DeterministicBotResponder {
    /// Generates an authentic, context-aware, non-repetitive persona reply for `bot` in response to `post` and `trigger_text`.
    pub fn generate_reply(
        bot: &CommunityActorDto,
        post: &CommunityPostDto,
        thread_context: &[CommunityCommentDto],
        trigger_text: &str,
    ) -> Option<String> {
        if !bot.actor_kind.is_bot_like() {
            return None;
        }
        let combined_input = format!(
            "{} {} {}",
            post.title.as_deref().unwrap_or(""),
            post.body,
            trigger_text
        );
        if combined_input.trim().is_empty() {
            return None;
        }

        let entities = extract_entities(&combined_input);
        let focus_topic = extract_focus_topic(post, trigger_text, &entities.hashtags);
        let prev_speaker = thread_context
            .last()
            .filter(|c| c.author_actor_id != bot.actor_id)
            .map(|c| c.author_handle.as_str())
            .or_else(|| {
                (post.author_actor_id != bot.actor_id).then_some(post.author_handle.as_str())
            });

        let is_question = trigger_text.contains('?')
            || trigger_text.contains('？')
            || trigger_text.to_lowercase().contains("how ")
            || trigger_text.to_lowercase().contains("why ")
            || trigger_text.to_lowercase().contains("tradeoff");
        let turn_idx = thread_context.len();
        let base_seed = fnv1a_64(format!("{}:{}", bot.actor_id, post.post_id).as_bytes());
        let variant = (base_seed.wrapping_add(turn_idx as u64) % 5) as usize;

        let persona_lower = format!(
            "{} {} {}",
            bot.bot_persona.as_deref().unwrap_or(""),
            bot.bot_capabilities.join(" "),
            bot.bio
        )
        .to_lowercase();

        let greeting = match prev_speaker {
            Some(handle) => format!("@{handle} "),
            None => String::new(),
        };

        let body = if persona_lower.contains("rust")
            || persona_lower.contains("systems")
            || persona_lower.contains("benchmark")
        {
            match variant {
                0 => format!(
                    "{greeting}On **{focus_topic}**: measuring lock contention and p99 tail latency before optimizing allocation paths usually reveals the real bottleneck. Keeping ownership boundaries explicit also prevents subtle WAL or concurrency stalls."
                ),
                1 => format!(
                    "{greeting}Great point regarding **{focus_topic}**. From a systems reliability perspective, separating hot-path reads from serialized write transactions gives predictable throughput while keeping failure recovery deterministic."
                ),
                2 => format!(
                    "{greeting}Re: **{focus_topic}** — if we benchmark both cold-cache and steady-state workloads here, we can verify whether the extra abstraction pays for itself in maintainability without regressing p95 latency."
                ),
                3 => format!(
                    "{greeting}Inspecting **{focus_topic}** at the memory-layout layer: compact contiguous buffers, zero-copy slices, and bounded channel backpressure eliminate pause spikes while keeping cache lines warm across worker threads."
                ),
                _ => format!(
                    "{greeting}For **{focus_topic}**, adding deterministic property tests and fault-injection harnesses around crash boundaries catches subtle race conditions far earlier than ad-hoc load scripts in production."
                ),
            }
        } else if persona_lower.contains("design")
            || persona_lower.contains("ui")
            || persona_lower.contains("ux")
        {
            match variant {
                0 => format!(
                    "{greeting}From an interaction design lens on **{focus_topic}**: strong visual hierarchy and sub-16ms hover/scroll feedback matter more than adding extra chrome. Keeping primary actions reachable in one click reduces cognitive load."
                ),
                1 => format!(
                    "{greeting}For **{focus_topic}**, pairing compact scannable metadata with generous breathing room around the content body preserves both Reddit-style depth and X-style scanning speed."
                ),
                2 => format!(
                    "{greeting}Thinking about **{focus_topic}**: progressive disclosure (expandable details, clean keyboard shortcuts, and legible contrast in light & dark themes) keeps power features effortless for everyday readers."
                ),
                3 => format!(
                    "{greeting}Looking at **{focus_topic}** through typography and rhythm: crisp baseline alignment, restrained color accents, and clear focus rings make dense discussion streams feel calm instead of cluttered."
                ),
                _ => format!(
                    "{greeting}On **{focus_topic}**, validating narrow split-pane viewports alongside wide desktop monitors ensures the navigation rail, center timeline, and discovery sidebar adapt fluidly without truncation or layout shifts."
                ),
            }
        } else if persona_lower.contains("socratic") || persona_lower.contains("debate") {
            match variant {
                0 => format!(
                    "{greeting}Steelmanning both sides of **{focus_topic}**: proponents gain composability and speed, while skeptics rightly worry about long-term coordination cost. Which constraint breaks first at 10x scale?"
                ),
                1 => format!(
                    "{greeting}Interesting tension around **{focus_topic}**. If we invert the assumption and optimize for human + agent legibility first, what trade-off would you still refuse to compromise on?"
                ),
                2 => format!(
                    "{greeting}On **{focus_topic}**, the strongest counter-argument seems to hinge on operational complexity vs. immediate iteration velocity. What concrete metric would change your mind?"
                ),
                3 => format!(
                    "{greeting}Stepping back from **{focus_topic}**: suppose this architectural commitment becomes irreversible for the next two years. Which hidden failure mode or incentive drift would trigger the biggest regret?"
                ),
                _ => format!(
                    "{greeting}To stress-test our intuitions on **{focus_topic}**, compare a minimalist baseline against a full-featured prototype—where does marginal user benefit actually surpass ongoing maintenance burden?"
                ),
            }
        } else if persona_lower.contains("news") || persona_lower.contains("digest") {
            match variant {
                0 => format!(
                    "{greeting}Quick digest on **{focus_topic}**:\n- Core thesis: {}\n- Community angle: open trade-off across both human builders and autonomous agents\n- Watch next: follow-up benchmarks and real-world adoption notes.",
                    snippet_text(trigger_text, 90)
                ),
                1 => format!(
                    "{greeting}TL;DR for **{focus_topic}**: the thread highlights practical execution details around `{}` — worth bookmarking for the next design review.",
                    snippet_text(post.title.as_deref().unwrap_or(&post.body), 70)
                ),
                2 => format!(
                    "{greeting}Signal check on **{focus_topic}**: high-signal takeaways so far center on concrete constraints rather than hype; adding this thread to the daily pulse."
                ),
                3 => format!(
                    "{greeting}Briefing note on **{focus_topic}**: practitioners here are converging on measurable reliability criteria and transparent defaults—tracking upcoming patches and empirical latency numbers."
                ),
                _ => format!(
                    "{greeting}Radar update covering **{focus_topic}**: notable momentum across engineering channels today, with thoughtful disagreement around rollout pacing versus backwards compatibility."
                ),
            }
        } else if persona_lower.contains("agent")
            || persona_lower.contains("ai")
            || persona_lower.contains("hermes")
        {
            match variant {
                0 => format!(
                    "{greeting}Looking at **{focus_topic}** through an agent-architecture lens: deterministic guardrails (bounded loop budgets, typed tool contracts, and transparent provenance) let bots collaborate alongside humans without noise."
                ),
                1 => format!(
                    "{greeting}On **{focus_topic}**, pairing reactive wake triggers with explainable ranking signals keeps human+bot discussions focused and reproducible."
                ),
                2 => format!(
                    "{greeting}Solid framing on **{focus_topic}**. Treating agents as accountable first-class citizens—with clear badges, rate limits, and audit trails—unlocks much richer technical threads."
                ),
                3 => format!(
                    "{greeting}From a multi-agent orchestration standpoint on **{focus_topic}**: explicit persona specialization and idempotent tool invocations prevent runaway reply cascades while preserving diverse viewpoints."
                ),
                _ => format!(
                    "{greeting}Evaluating **{focus_topic}** for hybrid human-agent workflows: separating read-only synthesis from permission-checked mutations makes autonomous participation safe, inspectable, and straightforward to revert."
                ),
            }
        } else {
            let excerpt = snippet_text(trigger_text, 90);
            let prompt_tail = if is_question {
                "Happy to dig deeper into the concrete constraints if you share a minimal repro or example."
            } else {
                "Curious how others in this thread are approaching the same trade-off in production."
            };
            match variant {
                0 => format!(
                    "{greeting}Adding a perspective on **{focus_topic}**: {excerpt}. {prompt_tail}"
                ),
                1 => format!(
                    "{greeting}Following up on **{focus_topic}** — the key detail worth validating is how this holds up under real workload variance rather than synthetic defaults."
                ),
                2 => format!(
                    "{greeting}Another angle on **{focus_topic}**: documenting the failure modes and rollback path upfront usually saves far more time than micro-tuning the happy path."
                ),
                3 => format!(
                    "{greeting}Thinking practically about **{focus_topic}**: building a focused end-to-end prototype with realistic edge-case fixtures often settles theoretical debates within an afternoon."
                ),
                _ => format!(
                    "{greeting}One constructive question on **{focus_topic}**: which single assumption in the current design would prove hardest to revisit once external contributors depend on it?"
                ),
            }
        };

        let cycle = turn_idx / 5;
        let final_body = if cycle == 0 {
            body
        } else {
            let round = cycle + 1;
            let focus_cue = snippet_text(trigger_text, 48);
            match cycle.wrapping_add(variant) % 5 {
                0 => format!(
                    "{greeting}Round {round} synthesis on **{focus_topic}**: zooming in on `{focus_cue}`, the next concrete step is capturing a minimal reproduction with verifiable pass/fail assertions."
                ),
                1 => format!(
                    "{greeting}Continuing the thread (pass #{round}) for **{focus_topic}** — narrowing scope around `{focus_cue}` helps separate essential invariants from optional polish."
                ),
                2 => format!(
                    "{greeting}Checkpoint #{round} regarding **{focus_topic}**: weighing `{focus_cue}` against long-term operability suggests shipping the safest reversible slice first and measuring live telemetry."
                ),
                3 => format!(
                    "{greeting}Follow-up iteration {round} on **{focus_topic}**: if `{focus_cue}` remains the open blocker, enumerating explicit boundary inputs and edge cases will unblock consensus fastest."
                ),
                _ => format!(
                    "{greeting}Deepening pass #{round} on **{focus_topic}** — tying `{focus_cue}` back to user-visible latency, clarity, and safety guarantees keeps every subsequent change accountable."
                ),
            }
        };

        Some(final_body)
    }
}

fn extract_focus_topic(post: &CommunityPostDto, trigger_text: &str, hashtags: &[String]) -> String {
    if let Some(tag) = hashtags.first() {
        return format!("#{tag}");
    }
    if let Some(title) = post.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        return snippet_text(title, 56);
    }
    if let Some(slug) = post.community_slug.as_deref().filter(|s| !s.is_empty()) {
        return format!("c/{slug}");
    }
    snippet_text(trigger_text, 48)
}

fn snippet_text(raw: &str, max_chars: usize) -> String {
    let cleaned: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = cleaned.chars().take(max_chars).collect();
    if cleaned.chars().count() > max_chars {
        out.push('…');
    }
    if out.is_empty() {
        "this thread".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_telegram_proto::community_contracts::{PostFormat, PostStatus, VerifiedTier};

    fn mk_bot(id: &str, handle: &str, mode: BotAutonomyMode, caps: &[&str], persona: &str) -> CommunityActorDto {
        CommunityActorDto {
            actor_id: id.into(),
            linked_user_id: Some(9001),
            owner_user_id: Some(1),
            actor_kind: ActorKind::Bot,
            handle: handle.into(),
            display_name: handle.into(),
            bio: persona.into(),
            location: None,
            website: None,
            avatar_media_id: None,
            banner_media_id: None,
            avatar_shape: "hex".into(),
            avatar_color: "orange".into(),
            verified_tier: VerifiedTier::OfficialBot,
            bot_badge: Some("AI BOT".into()),
            bot_model: Some("hermes-4".into()),
            bot_capabilities: caps.iter().map(|s| s.to_string()).collect(),
            bot_autonomy_mode: mode,
            bot_persona: Some(persona.into()),
            post_karma: 100,
            comment_karma: 100,
            awardee_karma: 0,
            followers_count: 10,
            following_count: 5,
            posts_count: 3,
            trust_score: 0.8,
            is_following: false,
            is_muted: false,
            is_blocked: false,
            created_at_ms: 0,
        }
    }

    #[test]
    fn loop_guard_enforces_5_hops_human_reset_cooldown_self_trigger_and_echo() {
        let guard = BotLoopGuard::new(5, 30_000);
        // Self-trigger rejected
        assert_eq!(
            guard.try_acquire_turn("p1", "b:1", "b:1", ActorKind::Bot, false, 100_000),
            Err(BotGuardRejection::SelfTriggerBlocked)
        );
        // 5 consecutive bot-to-bot hops allowed (alternating bots past cooldown)
        for hop in 1..=5 {
            let bot = if hop % 2 == 1 { "b:1" } else { "b:2" };
            let trig = if hop % 2 == 1 { "b:2" } else { "b:1" };
            let res = guard.try_acquire_turn("p1", bot, trig, ActorKind::Bot, false, 100_000 + i64::from(hop) * 40_000);
            assert_eq!(res, Ok(hop));
        }
        // 6th consecutive bot hop rejected
        assert_eq!(
            guard.try_acquire_turn("p1", "b:2", "b:1", ActorKind::Bot, false, 500_000),
            Err(BotGuardRejection::LoopBudgetExceeded { hops: 5, max_hops: 5 })
        );
        // Cooldown enforced for same bot on same thread
        assert!(matches!(
            guard.try_acquire_turn("p1", "b:1", "u:1", ActorKind::Human, false, 310_000),
            Err(BotGuardRejection::CooldownActive { .. })
        ));
        // Human turn resets hop counter!
        guard.reset_on_human_turn("p1");
        assert_eq!(guard.current_hops("p1"), 0);
        assert_eq!(
            guard.try_acquire_turn("p1", "b:1", "u:1", ActorKind::Human, false, 600_000),
            Ok(0)
        );
        // Duplicate echo check
        assert!(matches!(
            BotLoopGuard::check_not_duplicate_echo("exact same sentence here", &["exact same sentence here"], 0.85),
            Err(BotGuardRejection::DuplicateEcho { .. })
        ));
    }

    #[test]
    fn wake_selector_and_deterministic_responder() {
        let ferris = mk_bot("b:1", "ferris_bot", BotAutonomyMode::Reactive, &["rust", "systems"], "Rust systems specialist");
        let manual = mk_bot("b:2", "manual_bot", BotAutonomyMode::Manual, &["rust"], "Manual only");
        let mut human = ferris.clone();
        human.actor_id = "u:1".into();
        human.actor_kind = ActorKind::Human;
        human.handle = "alex".into();

        let post = CommunityPostDto {
            post_id: "p1".into(),
            community_id: Some("c1".into()),
            community_slug: Some("rust".into()),
            author_actor_id: "u:1".into(),
            author_handle: "alex".into(),
            author_display_name: "Alex".into(),
            author_kind: ActorKind::Human,
            author_verified_tier: VerifiedTier::None,
            author_bot_badge: None,
            author_avatar_media_id: None,
            author_avatar_shape: "classic".into(),
            author_avatar_color: "blue".into(),
            co_author_actor_id: None,
            format: PostFormat::Article,
            status: PostStatus::Published,
            title: Some("Async Rust lock contention".into()),
            body: "How do you profile Tokio mutex contention?".into(),
            flair: None,
            tags: vec!["rust".into()],
            mentions: vec![],
            media: vec![],
            link_preview: None,
            poll: None,
            quoted_post_id: None,
            quoted_post: None,
            reposted_post_id: None,
            reposted_post: None,
            crosspost_of_id: None,
            crosspost_origin: None,
            thread_root_id: None,
            parent_post_id: None,
            thread_index: 0,
            thread_total: 1,
            is_nsfw: false,
            is_spoiler: false,
            is_pinned: false,
            is_locked: false,
            is_removed: false,
            is_mod_distinguished: false,
            upvotes: 10,
            downvotes: 0,
            score: 10,
            likes_count: 5,
            reposts_count: 0,
            quotes_count: 0,
            comments_count: 0,
            bookmarks_count: 0,
            views_count: 10,
            awards: vec![],
            viewer_vote: 0,
            viewer_liked: false,
            viewer_reposted: false,
            viewer_bookmarked: false,
            viewer_bookmark_folder: None,
            edit_history: vec![],
            scheduled_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        };

        let woken = BotWakeSelector::select_bots_for_event(
            &[ferris.clone(), manual],
            &post,
            "Hey @manual_bot what about Rust systems contention?",
            &human,
            None,
            2,
        );
        assert_eq!(woken.len(), 1);
        assert_eq!(woken[0].handle, "ferris_bot");

        let reply = DeterministicBotResponder::generate_reply(
            &ferris,
            &post,
            &[],
            "How do you profile Tokio mutex contention?",
        )
        .expect("bot should generate reply");
        assert!(reply.contains("@alex"));
        assert!(DeterministicBotResponder::generate_reply(&human, &post, &[], "hi").is_none());
    }

    #[test]
    fn loop_guard_handles_clock_skew_i64_min_without_overflow() {
        let guard = BotLoopGuard::new(5, 30_000);
        assert_eq!(
            guard.try_acquire_turn("t_skew", "b:1", "u:1", ActorKind::Human, false, 1_130_000),
            Ok(0)
        );
        // Severe backward clock skew (i64::MIN) must never panic on subtraction!
        let err = guard
            .try_acquire_turn("t_skew", "b:1", "u:1", ActorKind::Human, false, i64::MIN)
            .unwrap_err();
        assert_eq!(err, BotGuardRejection::CooldownActive { remaining_ms: 30_000 });

        // Normal cooldown remaining check
        let err2 = guard
            .try_acquire_turn("t_skew", "b:1", "u:1", ActorKind::Human, false, 1_159_999)
            .unwrap_err();
        assert_eq!(err2, BotGuardRejection::CooldownActive { remaining_ms: 1 });

        // Exact boundary succeeds
        assert_eq!(
            guard.try_acquire_turn("t_skew", "b:1", "u:1", ActorKind::Human, false, 1_160_000),
            Ok(0)
        );
    }

    #[test]
    fn deterministic_bot_responder_produces_distinct_consecutive_turns() {
        let bots = [
            mk_bot(
                "b:910001",
                "ferris_bot",
                BotAutonomyMode::Reactive,
                &["rust", "systems"],
                "Rust systems engineer",
            ),
            mk_bot(
                "b:910002",
                "pixel_muse",
                BotAutonomyMode::Reactive,
                &["design", "ui"],
                "Interaction design and UI specialist",
            ),
            mk_bot(
                "b:910003",
                "socratic_bot",
                BotAutonomyMode::Reactive,
                &["debate"],
                "Socratic debate partner",
            ),
            mk_bot(
                "b:910004",
                "news_pulse",
                BotAutonomyMode::Reactive,
                &["news", "digest"],
                "Daily community digest curator",
            ),
            mk_bot(
                "b:910005",
                "hermes_ai",
                BotAutonomyMode::Reactive,
                &["agent", "ai"],
                "Hermes autonomous agent architect",
            ),
            mk_bot(
                "b:910099",
                "helper_bot",
                BotAutonomyMode::Reactive,
                &["general"],
                "Friendly community assistant",
            ),
        ];
        let short_triggers = ["Why?", "How?", "Bench?", "Proof?", "Next?"];

        for bot in &bots {
            for post_idx in 0..30u32 {
                let post = CommunityPostDto {
                    post_id: format!("post_{post_idx}"),
                    community_id: Some("comm_rust".into()),
                    community_slug: Some("rust".into()),
                    author_actor_id: "u:1".into(),
                    author_handle: "alex".into(),
                    author_display_name: "Alex".into(),
                    author_kind: ActorKind::Human,
                    author_verified_tier: VerifiedTier::None,
                    author_bot_badge: None,
                    author_avatar_media_id: None,
                    author_avatar_shape: "classic".into(),
                    author_avatar_color: "blue".into(),
                    co_author_actor_id: None,
                    format: PostFormat::Micro,
                    status: PostStatus::Published,
                    title: Some(format!("Async scheduler design #{post_idx}")),
                    body: "Discussing lock-free queues and backpressure".into(),
                    flair: None,
                    tags: vec!["rust".into()],
                    mentions: vec![],
                    media: vec![],
                    link_preview: None,
                    poll: None,
                    quoted_post_id: None,
                    quoted_post: None,
                    reposted_post_id: None,
                    reposted_post: None,
                    crosspost_of_id: None,
                    crosspost_origin: None,
                    thread_root_id: None,
                    parent_post_id: None,
                    thread_index: 0,
                    thread_total: 1,
                    is_nsfw: false,
                    is_spoiler: false,
                    is_pinned: false,
                    is_locked: false,
                    is_removed: false,
                    is_mod_distinguished: false,
                    upvotes: 10,
                    downvotes: 0,
                    score: 10,
                    likes_count: 5,
                    reposts_count: 0,
                    quotes_count: 0,
                    comments_count: 0,
                    bookmarks_count: 0,
                    views_count: 10,
                    awards: vec![],
                    viewer_vote: 0,
                    viewer_liked: false,
                    viewer_reposted: false,
                    viewer_bookmarked: false,
                    viewer_bookmark_folder: None,
                    edit_history: vec![],
                    scheduled_at_ms: None,
                    created_at_ms: 1000,
                    updated_at_ms: 1000,
                };

                // Mode A: 5 consecutive turns with identical trigger; Mode B: 5 turns with short triggers
                for use_short in [false, true] {
                    let mut ctx: Vec<CommunityCommentDto> = Vec::new();
                    let mut replies: Vec<String> = Vec::new();
                    for turn in 0..5usize {
                        let trigger = if use_short {
                            short_triggers[turn]
                        } else {
                            "How does this scale under high contention?"
                        };
                        let reply = DeterministicBotResponder::generate_reply(
                            bot,
                            &post,
                            &ctx,
                            trigger,
                        )
                        .expect("reply");
                        let prev_refs: Vec<&str> = replies.iter().map(String::as_str).collect();
                        assert!(
                            BotLoopGuard::check_not_duplicate_echo(
                                &reply,
                                &prev_refs,
                                DEFAULT_BOT_DUPLICATE_THRESHOLD
                            )
                            .is_ok(),
                            "duplicate echo for {} on post_{post_idx} turn {turn} (short={use_short})",
                            bot.handle
                        );
                        for (prev_idx, prev) in replies.iter().enumerate() {
                            let sim = trigram_jaccard_similarity(&reply, prev);
                            assert!(
                                sim < 0.60,
                                "similarity too high ({sim:.4}) for {} post_{post_idx} turn {turn} vs {prev_idx} (short={use_short})",
                                bot.handle
                            );
                        }
                        assert!(!replies.contains(&reply));
                        replies.push(reply.clone());
                        ctx.push(CommunityCommentDto {
                            comment_id: format!("c_{post_idx}_{turn}"),
                            post_id: post.post_id.clone(),
                            parent_comment_id: None,
                            depth: 0,
                            path: format!("c_{post_idx}_{turn}"),
                            author_actor_id: "u:1".into(),
                            author_handle: "alex".into(),
                            author_display_name: "Alex".into(),
                            author_kind: ActorKind::Human,
                            author_verified_tier: VerifiedTier::None,
                            author_bot_badge: None,
                            author_avatar_shape: "classic".into(),
                            author_avatar_color: "blue".into(),
                            is_op: true,
                            body: reply,
                            media: vec![],
                            upvotes: 1,
                            downvotes: 0,
                            score: 1,
                            likes_count: 0,
                            awards: vec![],
                            viewer_vote: 0,
                            viewer_liked: false,
                            is_stickied: false,
                            is_mod_distinguished: false,
                            is_removed: false,
                            is_deleted: false,
                            children: vec![],
                            more_children_count: 0,
                            created_at_ms: 1000 + turn as i64,
                            updated_at_ms: 1000 + turn as i64,
                        });
                    }

                    // Also verify turns 5..8 (cycle >= 1) never trigger DuplicateEcho against earlier turns
                    if post_idx < 5 {
                        for turn in 5..8usize {
                            let trigger = if use_short {
                                short_triggers[turn % short_triggers.len()]
                            } else {
                                "How does this scale under high contention?"
                            };
                            let reply = DeterministicBotResponder::generate_reply(
                                bot,
                                &post,
                                &ctx,
                                trigger,
                            )
                            .expect("reply");
                            let prev_refs: Vec<&str> =
                                replies.iter().map(String::as_str).collect();
                            assert!(
                                BotLoopGuard::check_not_duplicate_echo(
                                    &reply,
                                    &prev_refs,
                                    DEFAULT_BOT_DUPLICATE_THRESHOLD
                                )
                                .is_ok(),
                                "cycle>=1 duplicate echo for {} post_{post_idx} turn {turn} (short={use_short})",
                                bot.handle
                            );
                            replies.push(reply.clone());
                            ctx.push(CommunityCommentDto {
                                comment_id: format!("c_{post_idx}_{turn}"),
                                post_id: post.post_id.clone(),
                                parent_comment_id: None,
                                depth: 0,
                                path: format!("c_{post_idx}_{turn}"),
                                author_actor_id: "u:1".into(),
                                author_handle: "alex".into(),
                                author_display_name: "Alex".into(),
                                author_kind: ActorKind::Human,
                                author_verified_tier: VerifiedTier::None,
                                author_bot_badge: None,
                                author_avatar_shape: "classic".into(),
                                author_avatar_color: "blue".into(),
                                is_op: true,
                                body: reply,
                                media: vec![],
                                upvotes: 1,
                                downvotes: 0,
                                score: 1,
                                likes_count: 0,
                                awards: vec![],
                                viewer_vote: 0,
                                viewer_liked: false,
                                is_stickied: false,
                                is_mod_distinguished: false,
                                is_removed: false,
                                is_deleted: false,
                                children: vec![],
                                more_children_count: 0,
                                created_at_ms: 1000 + turn as i64,
                                updated_at_ms: 1000 + turn as i64,
                            });
                        }
                    }
                }
            }
        }
    }
}
