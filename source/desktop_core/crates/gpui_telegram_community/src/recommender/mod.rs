//! 5-Stage Multi-Signal Recommendation Engine fusing Reddit math & X For You ranking.

pub mod diversity;
pub mod scoring;
pub mod signals;
pub mod trending;

pub use diversity::{DiversityConfig, ScoredCandidate, rerank_with_mmr};
pub use scoring::{
    default_prefs_for_preset, reddit_controversial, reddit_hot, reddit_rising, score_for_you,
    time_window_cutoff_ms, wilson_lower_bound,
};
pub use signals::{
    CandidateSource, EMBEDDING_DIMS, PostCandidate, ViewerContext, blend_interest_vector,
    cosine_similarity_64, embed_text_64, fnv1a_64, signal_affinity_delta, splitmix64_unit,
};
pub use trending::TrendingEngine;

use gpui_telegram_proto::community_contracts::{
    ActorFilter, AlgoPreset, FeedExplanationDto, FeedItemDto, FeedRequestDto, FeedResponseDto,
    FeedSort, FeedTab, PostStatus, SignalContributionDto,
};
use std::collections::HashSet;

pub(crate) fn normalize_community_key(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    lower
        .strip_prefix("c/")
        .or_else(|| lower.strip_prefix("r/"))
        .unwrap_or(&lower)
        .to_string()
}

/// 5-stage deterministic community feed recommender.
pub struct CommunityRecommender;

impl CommunityRecommender {
    /// Ranks `candidates` for `viewer` under `req` using the 5-stage pipeline:
    /// 1. Scope & candidate source tagging (`FeedTab`, `ActorFilter`, flair, `TimeWindow`)
    /// 2. Negative gating (blocks, mutes, keywords, hidden posts, NSFW, unpublished status)
    /// 3. Mode scoring (`Algo` / `Hot` / `Best` / `Top` / `New` / `Controversial` / `Rising`)
    /// 4. Greedy MMR diversity, Human/Bot fairness, and exploration slots
    /// 5. Cursor pagination & optional `FeedExplanationDto`
    pub fn rank_feed(
        viewer: &ViewerContext,
        candidates: Vec<PostCandidate>,
        req: &FeedRequestDto,
    ) -> FeedResponseDto {
        let now_ms = req.now_ms.unwrap_or(viewer.now_ms);
        let prefs = req
            .algo_override
            .clone()
            .unwrap_or_else(|| viewer.algo_prefs.clone());
        let limit = req.limit.clamp(1, 100);
        let offset = parse_offset_cursor(req.cursor.as_deref());
        let cutoff_ms = time_window_cutoff_ms(req.time_window, now_ms);
        let muted_keywords_lower: Vec<String> = viewer
            .muted_keywords
            .iter()
            .map(|k| k.trim().to_lowercase())
            .filter(|k| !k.is_empty())
            .collect();
        let muted_communities_norm: HashSet<String> = viewer
            .muted_communities
            .iter()
            .map(|s| normalize_community_key(s))
            .filter(|s| !s.is_empty())
            .collect();
        let joined_communities_norm: HashSet<String> = viewer
            .joined_communities
            .iter()
            .map(|s| normalize_community_key(s))
            .filter(|s| !s.is_empty())
            .collect();
        let list_communities_norm: HashSet<String> = viewer
            .list_community_slugs
            .iter()
            .map(|s| normalize_community_key(s))
            .filter(|s| !s.is_empty())
            .collect();

        let actor_blocked_or_muted = |actor_id: &str| -> bool {
            viewer.blocked_actor_ids.contains(actor_id) || viewer.muted_actor_ids.contains(actor_id)
        };

        // Stage 1 & Stage 2: Scope, ActorFilter, TimeWindow, Negative Gating
        let mut eligible: Vec<(PostCandidate, bool)> = Vec::with_capacity(candidates.len());
        for mut cand in candidates {
            let p = &cand.post;

            // Status / removal gating
            if p.is_removed || p.status == PostStatus::Deleted {
                continue;
            }
            let viewing_own_post =
                matches!(req.tab, FeedTab::ActorPosts(_)) && p.author_actor_id == viewer.actor_id;
            if p.status != PostStatus::Published && !viewing_own_post {
                continue;
            }

            // Negative gating: blocks, mutes (author, co-author, quoted/reposted/crosspost author), hidden posts, NSFW, muted communities & keywords
            if actor_blocked_or_muted(&p.author_actor_id)
                || p.co_author_actor_id
                    .as_deref()
                    .is_some_and(actor_blocked_or_muted)
                || p.quoted_post
                    .as_deref()
                    .is_some_and(|qp| actor_blocked_or_muted(&qp.author_actor_id))
                || p.reposted_post
                    .as_deref()
                    .is_some_and(|rp| actor_blocked_or_muted(&rp.author_actor_id))
                || p.crosspost_origin
                    .as_deref()
                    .is_some_and(|cp| actor_blocked_or_muted(&cp.author_actor_id))
                || viewer.hidden_post_ids.contains(&p.post_id)
            {
                continue;
            }
            if p.is_nsfw && !prefs.allow_nsfw {
                continue;
            }
            let comm_is_muted = |slug_opt: Option<&str>| -> bool {
                slug_opt
                    .map(normalize_community_key)
                    .filter(|s| !s.is_empty())
                    .is_some_and(|s| muted_communities_norm.contains(&s))
            };
            let post_comm_norm = p
                .community_slug
                .as_deref()
                .map(normalize_community_key)
                .filter(|s| !s.is_empty());
            if post_comm_norm
                .as_deref()
                .is_some_and(|slug| muted_communities_norm.contains(slug))
                || p.quoted_post
                    .as_deref()
                    .is_some_and(|qp| comm_is_muted(qp.community_slug.as_deref()))
                || p.reposted_post
                    .as_deref()
                    .is_some_and(|rp| comm_is_muted(rp.community_slug.as_deref()))
                || p.crosspost_origin
                    .as_deref()
                    .is_some_and(|cp| comm_is_muted(cp.community_slug.as_deref()))
            {
                continue;
            }
            if !muted_keywords_lower.is_empty() {
                let mut hay = String::new();
                if let Some(t) = p.title.as_deref() {
                    hay.push_str(t);
                    hay.push(' ');
                }
                hay.push_str(&p.body);
                if let Some(f) = p.flair.as_deref() {
                    hay.push(' ');
                    hay.push_str(f);
                }
                for tag in &p.tags {
                    hay.push(' ');
                    hay.push_str(tag);
                }
                if let Some(poll) = p.poll.as_ref() {
                    hay.push(' ');
                    hay.push_str(&poll.question);
                    for opt in &poll.options {
                        hay.push(' ');
                        hay.push_str(&opt.text);
                    }
                }
                if let Some(lp) = p.link_preview.as_ref() {
                    if !lp.title.is_empty() {
                        hay.push(' ');
                        hay.push_str(&lp.title);
                    }
                    if let Some(desc) = lp.description.as_deref() {
                        hay.push(' ');
                        hay.push_str(desc);
                    }
                }
                for embedded in [
                    p.quoted_post.as_deref(),
                    p.reposted_post.as_deref(),
                    p.crosspost_origin.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(et) = embedded.title.as_deref() {
                        hay.push(' ');
                        hay.push_str(et);
                    }
                    if !embedded.body_preview.is_empty() {
                        hay.push(' ');
                        hay.push_str(&embedded.body_preview);
                    }
                }
                let hay_lower = hay.to_lowercase();
                if muted_keywords_lower.iter().any(|kw| hay_lower.contains(kw)) {
                    continue;
                }
            }

            // Time window filter
            if let Some(min_ts) = cutoff_ms {
                if p.created_at_ms < min_ts {
                    continue;
                }
            }

            // Flair filter
            if let Some(req_flair) = req.flair.as_deref().filter(|f| !f.trim().is_empty()) {
                let matches_flair = p
                    .flair
                    .as_deref()
                    .is_some_and(|f| f.eq_ignore_ascii_case(req_flair.trim()));
                if !matches_flair {
                    continue;
                }
            }

            // ActorFilter
            match req.actor_filter {
                ActorFilter::AllCitizens => {}
                ActorFilter::HumansOnly => {
                    if p.author_kind.is_bot_like() {
                        continue;
                    }
                }
                ActorFilter::BotsOnly => {
                    if !p.author_kind.is_bot_like() {
                        continue;
                    }
                }
                ActorFilter::HumanBotDebates => {
                    if !(cand.has_human_participants && cand.has_bot_participants) {
                        continue;
                    }
                }
            }

            // Tab scope check
            let in_network_author = viewer.following_actor_ids.contains(&p.author_actor_id);
            let in_joined_comm = post_comm_norm
                .as_deref()
                .is_some_and(|s| joined_communities_norm.contains(s));
            let tab_matches = match &req.tab {
                FeedTab::ForYou | FeedTab::Popular | FeedTab::All => true,
                FeedTab::Following => {
                    in_network_author || in_joined_comm || p.author_actor_id == viewer.actor_id
                }
                FeedTab::Community(slug) => {
                    let target_slug = normalize_community_key(slug);
                    post_comm_norm.as_deref() == Some(target_slug.as_str())
                }
                FeedTab::List(_) => {
                    viewer.list_member_actors.contains(&p.author_actor_id)
                        || post_comm_norm
                            .as_deref()
                            .is_some_and(|s| list_communities_norm.contains(s))
                }
                FeedTab::Saved => {
                    p.viewer_bookmarked || viewer.saved_post_ids.contains(&p.post_id)
                }
                FeedTab::ActorPosts(handle_or_id) => {
                    let target = handle_or_id.trim();
                    let target_handle = target.trim_start_matches('@').trim();
                    !target.is_empty()
                        && (p.author_actor_id.eq_ignore_ascii_case(target)
                            || (!target_handle.is_empty()
                                && p.author_handle.eq_ignore_ascii_case(target_handle)))
                }
            };
            if !tab_matches {
                continue;
            }

            // Candidate source tagging
            if cand.sources.is_empty() {
                if in_network_author {
                    cand.sources.push(CandidateSource::FollowGraph);
                }
                if in_joined_comm {
                    cand.sources.push(CandidateSource::JoinedCommunity);
                }
                if viewer.second_hop_weights.contains_key(&p.author_actor_id) {
                    cand.sources.push(CandidateSource::SecondHopSocial);
                }
                if cosine_similarity_64(&viewer.interest_embedding, &cand.content_embedding) > 0.25 {
                    cand.sources.push(CandidateSource::EmbeddingSimilar);
                }
                if cand.recent_engagement_1h >= 10 {
                    cand.sources.push(CandidateSource::TrendingVelocity);
                }
                if cand.sources.is_empty() {
                    cand.sources.push(CandidateSource::ColdStartQuality);
                }
            }
            let is_out_of_network =
                !in_network_author && !in_joined_comm && p.author_actor_id != viewer.actor_id;
            eligible.push((cand, is_out_of_network));
        }

        let total_candidates = eligible.len();
        let effective_sort = match (req.sort, prefs.preset) {
            (FeedSort::Algo, AlgoPreset::Chronological) => FeedSort::New,
            (s, _) => s,
        };

        // Stage 3: Mode Scoring
        let mut scored: Vec<ScoredCandidate> = eligible
            .into_iter()
            .map(|(cand, is_out_of_network)| {
                let p = &cand.post;
                let age_hours = (now_ms.saturating_sub(p.created_at_ms).max(0) as f64) / 3_600_000.0;
                let (score, explanation) = match effective_sort {
                    FeedSort::Algo => score_for_you(viewer, &cand, &prefs, now_ms),
                    FeedSort::Hot => (
                        reddit_hot(p.upvotes, p.downvotes, p.created_at_ms),
                        simple_explanation("Reddit Hot ranking", "Hot Score"),
                    ),
                    FeedSort::Best => (
                        wilson_lower_bound(p.upvotes, p.downvotes, 1.96),
                        simple_explanation("Reddit Best (Wilson 95% lower bound)", "Wilson Score"),
                    ),
                    FeedSort::Top => (
                        p.score as f64,
                        simple_explanation("Top voted posts", "Net Score"),
                    ),
                    FeedSort::New => (
                        p.created_at_ms as f64,
                        simple_explanation("Chronological latest", "Timestamp"),
                    ),
                    FeedSort::Controversial => (
                        reddit_controversial(p.upvotes, p.downvotes),
                        simple_explanation("Most debated (Controversial)", "Controversy"),
                    ),
                    FeedSort::Rising => (
                        reddit_rising(p.upvotes, p.downvotes, p.comments_count, age_hours),
                        simple_explanation("Rising velocity", "Rising Burst"),
                    ),
                };
                ScoredCandidate {
                    candidate: cand,
                    score,
                    explanation,
                    is_out_of_network,
                    is_exploration_slot: false,
                }
            })
            .collect();

        // Stage 4: Diversity & Ordering
        let use_mmr = matches!(effective_sort, FeedSort::Algo)
            && matches!(req.tab, FeedTab::ForYou | FeedTab::Popular | FeedTab::All);
        let ordered = if use_mmr {
            let needed = offset.saturating_add(limit).saturating_add(1).min(scored.len());
            let is_single_comm = matches!(req.tab, FeedTab::Community(_));
            let cfg = DiversityConfig {
                mmr_lambda: 0.78,
                max_per_author_per_page: 2,
                max_per_community_per_page: if is_single_comm { None } else { Some(3) },
                no_consecutive_same_author: true,
                target_bot_ratio: prefs.bot_content_ratio,
                exploration_ratio: prefs.discovery_ratio,
                viewer_seed: fnv1a_64(viewer.actor_id.as_bytes()),
            };
            rerank_with_mmr(scored, needed, &cfg)
        } else {
            let pin_community_posts = matches!(req.tab, FeedTab::Community(_));
            scored.sort_by(|a, b| {
                let pin_ord = if pin_community_posts {
                    b.candidate.post.is_pinned.cmp(&a.candidate.post.is_pinned)
                } else {
                    std::cmp::Ordering::Equal
                };
                let sa = if a.score.is_finite() { a.score } else { f64::NEG_INFINITY };
                let sb = if b.score.is_finite() { b.score } else { f64::NEG_INFINITY };
                pin_ord
                    .then_with(|| sb.total_cmp(&sa))
                    .then_with(|| {
                        b.candidate
                            .post
                            .created_at_ms
                            .cmp(&a.candidate.post.created_at_ms)
                    })
                    .then_with(|| a.candidate.post.post_id.cmp(&b.candidate.post.post_id))
            });
            scored
        };

        // Stage 5: Cursor Pagination & Explanation Attachment
        let page_slice: Vec<FeedItemDto> = ordered
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|sc| FeedItemDto {
                post: sc.candidate.post,
                explanation: prefs.show_explanations.then_some(sc.explanation),
                rank_score: sc.score,
                is_exploration_slot: sc.is_exploration_slot,
            })
            .collect();

        let next_offset = offset.saturating_add(page_slice.len());
        let next_cursor = if next_offset < total_candidates && !page_slice.is_empty() {
            Some(format!("offset:{next_offset}"))
        } else {
            None
        };

        FeedResponseDto {
            items: page_slice,
            next_cursor,
            total_candidates,
            applied_preset: prefs.preset,
            generated_at_ms: now_ms,
        }
    }
}

fn parse_offset_cursor(cursor: Option<&str>) -> usize {
    cursor
        .and_then(|c| c.trim().strip_prefix("offset:"))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0)
}

fn simple_explanation(reason: &str, label: &str) -> FeedExplanationDto {
    FeedExplanationDto {
        primary_reason: reason.to_string(),
        signal_breakdown: vec![SignalContributionDto {
            label: label.to_string(),
            contribution_pct: 100.0,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_telegram_proto::community_contracts::{
        ActorKind, CommunityPostDto, PostFormat, PostStatus, VerifiedTier,
    };

    fn sample_post(
        id: &str,
        author_id: &str,
        handle: &str,
        kind: ActorKind,
        slug: &str,
        title: &str,
        body: &str,
        up: i64,
        down: i64,
        ts: i64,
    ) -> CommunityPostDto {
        CommunityPostDto {
            post_id: id.into(),
            community_id: Some(format!("comm_{slug}")),
            community_slug: Some(slug.into()),
            author_actor_id: author_id.into(),
            author_handle: handle.into(),
            author_display_name: handle.into(),
            author_kind: kind,
            author_verified_tier: VerifiedTier::None,
            author_bot_badge: kind.is_bot_like().then(|| "AI BOT".into()),
            author_avatar_media_id: None,
            author_avatar_shape: "classic".into(),
            author_avatar_color: "blue".into(),
            co_author_actor_id: None,
            format: PostFormat::Article,
            status: PostStatus::Published,
            title: Some(title.into()),
            body: body.into(),
            flair: Some("Discussion".into()),
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
            upvotes: up,
            downvotes: down,
            score: up - down,
            likes_count: up.max(0) as u64,
            reposts_count: 2,
            quotes_count: 1,
            comments_count: 6,
            bookmarks_count: 3,
            views_count: 100,
            awards: vec![],
            viewer_vote: 0,
            viewer_liked: false,
            viewer_reposted: false,
            viewer_bookmarked: false,
            viewer_bookmark_folder: None,
            edit_history: vec![],
            scheduled_at_ms: None,
            created_at_ms: ts,
            updated_at_ms: ts,
        }
    }

    #[test]
    fn reddit_math_and_embedding_safety() {
        assert_eq!(wilson_lower_bound(0, 0, 1.96), 0.0);
        let w_small = wilson_lower_bound(2, 0, 1.96);
        let w_big = wilson_lower_bound(100, 5, 1.96);
        assert!(w_big > w_small && w_big < 1.0);

        let hot_new = reddit_hot(50, 2, 1_760_000_000_000);
        let hot_old = reddit_hot(50, 2, 1_759_000_000_000);
        assert!(hot_new > hot_old);

        assert_eq!(reddit_controversial(10, 0), 0.0);
        assert!(reddit_controversial(50, 48) > reddit_controversial(90, 8));
        assert!(reddit_rising(30, 0, 10, 0.5) > reddit_rising(30, 0, 10, 20.0));

        let a = embed_text_64("Rust async allocator 推荐算法");
        let b = embed_text_64("Rust async allocator 推荐算法优化");
        let c = embed_text_64("banana smoothie recipe");
        assert!(cosine_similarity_64(&a, &b) > cosine_similarity_64(&a, &c));
        assert_eq!(cosine_similarity_64(&[0.0; 64], &a), 0.0);
        assert_eq!(cosine_similarity_64(&[f32::NAN; 64], &a), 0.0);
    }

    #[test]
    fn recommender_determinism_presets_gating_mmr_and_explanations() {
        let now = 1_760_100_000_000_i64;
        let mut viewer = ViewerContext {
            actor_id: "u:1".into(),
            now_ms: now,
            interest_embedding: embed_text_64("rust systems wal sqlite"),
            ..Default::default()
        };
        viewer.following_actor_ids.insert("u:2".into());
        viewer.joined_communities.insert("rust".into());
        viewer.blocked_actor_ids.insert("u:blocked".into());
        viewer.muted_keywords.push("crypto_scam".into());

        let mut p_nsfw = sample_post("p_nsfw", "u:3", "nsfw_u", ActorKind::Human, "rust", "NSFW", "adult", 100, 0, now - 1000);
        p_nsfw.is_nsfw = true;

        let mut cands = vec![
            PostCandidate::from_post(sample_post("p_h1", "u:2", "alex", ActorKind::Human, "rust", "SQLite WAL tuning", "Deep dive on WAL checkpoints #rust", 80, 2, now - 3600_000)),
            PostCandidate::from_post(sample_post("p_h2", "u:2", "alex", ActorKind::Human, "rust", "SQLite WAL part 2", "More WAL benchmark numbers #rust", 75, 1, now - 3500_000)),
            PostCandidate::from_post(sample_post("p_h3", "u:2", "alex", ActorKind::Human, "rust", "SQLite WAL part 3", "Even more WAL notes #rust", 70, 1, now - 3400_000)),
            PostCandidate::from_post(sample_post("p_b1", "b:10", "ferris_bot", ActorKind::Bot, "ai_agents", "Agent loop budgets", "Bounding multi-agent turns cleanly #agents", 120, 3, now - 1800_000)),
            PostCandidate::from_post(sample_post("p_fresh", "u:5", "maya", ActorKind::Human, "design_ui", "Fresh UI drop", "Just shipped adaptive 3-column layout #gpui", 5, 0, now - 60_000)),
            PostCandidate::from_post(sample_post("p_blocked", "u:blocked", "spam", ActorKind::Human, "rust", "Blocked", "Should never appear", 999, 0, now - 1000)),
            PostCandidate::from_post(sample_post("p_muted_kw", "u:6", "botx", ActorKind::Human, "rust", "Promo", "Buy crypto_scam now", 999, 0, now - 1000)),
            PostCandidate::from_post(p_nsfw),
        ];
        // Mark p_b1 as active Human+Bot debate
        cands[3].has_human_participants = true;
        cands[3].has_bot_participants = true;

        let req = FeedRequestDto {
            tab: FeedTab::ForYou,
            sort: FeedSort::Algo,
            limit: 10,
            ..Default::default()
        };
        let r1 = CommunityRecommender::rank_feed(&viewer, cands.clone(), &req);
        let r2 = CommunityRecommender::rank_feed(&viewer, cands.clone(), &req);
        assert_eq!(r1.items.len(), r2.items.len());
        let ids1: Vec<&str> = r1.items.iter().map(|i| i.post.post_id.as_str()).collect();
        let ids2: Vec<&str> = r2.items.iter().map(|i| i.post.post_id.as_str()).collect();
        assert_eq!(ids1, ids2, "ranking must be strictly deterministic");

        // Negative gating verified
        assert!(!ids1.contains(&"p_blocked"));
        assert!(!ids1.contains(&"p_muted_kw"));
        assert!(!ids1.contains(&"p_nsfw"));

        // Non-consecutive same author enforced by MMR when other authors exist
        for pair in r1.items.windows(2) {
            if pair[0].post.author_actor_id == "u:2" {
                assert_ne!(pair[1].post.author_actor_id, "u:2");
            }
        }

        // Explanations sum to ~100%
        for item in &r1.items {
            let expl = item.explanation.as_ref().expect("explanation attached");
            let sum: f64 = expl.signal_breakdown.iter().map(|s| s.contribution_pct).sum();
            assert!((sum - 100.0).abs() <= 0.6, "sum was {sum}");
        }

        // Chronological preset puts p_fresh first
        let mut chrono_req = req.clone();
        chrono_req.algo_override = Some(default_prefs_for_preset(AlgoPreset::Chronological));
        let chrono_res = CommunityRecommender::rank_feed(&viewer, cands.clone(), &chrono_req);
        assert_eq!(chrono_res.items[0].post.post_id, "p_fresh");

        // ActorFilter::HumanBotDebates isolates p_b1
        let mut debate_req = req.clone();
        debate_req.actor_filter = ActorFilter::HumanBotDebates;
        let debate_res = CommunityRecommender::rank_feed(&viewer, cands.clone(), &debate_req);
        assert_eq!(debate_res.items.len(), 1);
        assert_eq!(debate_res.items[0].post.post_id, "p_b1");

        // TrendingEngine produces 6-bin sparkline with burst velocity
        let posts: Vec<CommunityPostDto> = cands.into_iter().map(|c| c.post).collect();
        let trends = TrendingEngine::compute(&posts, now, 10);
        assert!(!trends.is_empty());
        assert_eq!(trends[0].sparkline.len(), 6);
    }

    #[test]
    fn all_seven_presets_fatigue_time_window_and_cursor_pagination() {
        let now = 1_760_100_000_000_i64;
        let h = 3_600_000_i64;
        let mut viewer = ViewerContext {
            actor_id: "u:99".into(),
            now_ms: now,
            ..Default::default()
        };
        let cands = vec![
            PostCandidate::from_post(sample_post("p_human_old_gold", "u:1", "h1", ActorKind::Human, "rust", "Deep Rust Guide", "Quality longform #rust", 500, 3, now - 48 * h)),
            PostCandidate::from_post(sample_post("p_bot_recent", "b:1", "bot1", ActorKind::Bot, "ai_agents", "Bot Insight", "Fresh bot analysis #agents", 80, 1, now - 20 * 60_000)),
            PostCandidate::from_post(sample_post("p_human_recent", "u:2", "h2", ActorKind::Human, "systems", "Human Update", "Fresh human note #systems", 80, 1, now - 20 * 60_000)),
            PostCandidate::from_post(sample_post("p_ancient", "u:3", "h3", ActorKind::Human, "rust", "Last Year Post", "Old post", 1000, 1, now - 200 * 24 * h)),
        ];

        // Verify all 7 presets produce valid ranked outputs with applied_preset reflected
        for preset in [
            AlgoPreset::Balanced,
            AlgoPreset::RedditDeepDive,
            AlgoPreset::XRealTime,
            AlgoPreset::Discovery,
            AlgoPreset::HumanFocused,
            AlgoPreset::BotHeavy,
            AlgoPreset::Chronological,
        ] {
            let req = FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                limit: 10,
                algo_override: Some(default_prefs_for_preset(preset)),
                ..Default::default()
            };
            let res = CommunityRecommender::rank_feed(&viewer, cands.clone(), &req);
            assert_eq!(res.applied_preset, preset);
            assert_eq!(res.items.len(), 4);
        }

        // BotHeavy boosts bot post over equal-score human post; HumanFocused boosts human over equal-score bot
        let bot_heavy = CommunityRecommender::rank_feed(
            &viewer,
            vec![cands[1].clone(), cands[2].clone()],
            &FeedRequestDto {
                algo_override: Some(default_prefs_for_preset(AlgoPreset::BotHeavy)),
                ..Default::default()
            },
        );
        assert_eq!(bot_heavy.items[0].post.post_id, "p_bot_recent");

        let human_focused = CommunityRecommender::rank_feed(
            &viewer,
            vec![cands[1].clone(), cands[2].clone()],
            &FeedRequestDto {
                algo_override: Some(default_prefs_for_preset(AlgoPreset::HumanFocused)),
                ..Default::default()
            },
        );
        assert_eq!(human_focused.items[0].post.post_id, "p_human_recent");

        // Impression fatigue lowers score of repeatedly seen post
        let before = CommunityRecommender::rank_feed(&viewer, vec![cands[1].clone()], &FeedRequestDto::default());
        viewer.seen_impressions.insert("p_bot_recent".into(), 5);
        let after = CommunityRecommender::rank_feed(&viewer, vec![cands[1].clone()], &FeedRequestDto::default());
        assert!(after.items[0].rank_score < before.items[0].rank_score);

        // TimeWindow::Day excludes 48h and 200d posts
        let day_res = CommunityRecommender::rank_feed(
            &viewer,
            cands.clone(),
            &FeedRequestDto {
                sort: FeedSort::Top,
                time_window: gpui_telegram_proto::community_contracts::TimeWindow::Day,
                ..Default::default()
            },
        );
        assert_eq!(day_res.items.len(), 2);

        // Cursor pagination slices deterministically with next_cursor
        let page1 = CommunityRecommender::rank_feed(
            &viewer,
            cands.clone(),
            &FeedRequestDto { sort: FeedSort::Top, limit: 2, ..Default::default() },
        );
        assert_eq!(page1.items.len(), 2);
        assert_eq!(page1.next_cursor.as_deref(), Some("offset:2"));
        let page2 = CommunityRecommender::rank_feed(
            &viewer,
            cands,
            &FeedRequestDto { sort: FeedSort::Top, limit: 2, cursor: page1.next_cursor, ..Default::default() },
        );
        assert_eq!(page2.items.len(), 2);
        assert!(page2.next_cursor.is_none());
    }

    #[test]
    fn mmr_pagination_is_prefix_consistent_zero_dups_and_delivers_page1_exploration() {
        let now = 1_760_100_000_000_i64;
        let mut viewer = ViewerContext {
            actor_id: "u:viewer".into(),
            now_ms: now,
            ..Default::default()
        };
        viewer.joined_communities.insert("rust".into());
        viewer.joined_communities.insert("systems".into());

        // 16 in-network + 14 out-of-network = 30 posts across distinct authors & communities
        let mut cands = Vec::new();
        for i in 0..16usize {
            let slug = if i % 2 == 0 { "rust" } else { "systems" };
            let aid = format!("u:in_{}", i % 10);
            let h = format!("in_user_{}", i % 10);
            cands.push(PostCandidate::from_post(sample_post(
                &format!("in_{i:02}"),
                &aid,
                &h,
                if i % 3 == 0 { ActorKind::Bot } else { ActorKind::Human },
                slug,
                &format!("In-network topic {i}"),
                &format!("Systems notes {i} #rust"),
                100 - (i as i64) * 2,
                2,
                now - (i as i64) * 60_000,
            )));
        }
        for i in 0..14usize {
            let slug = format!("oon_comm_{}", i % 5);
            let aid = format!("u:oon_{i}");
            let h = format!("oon_user_{i}");
            cands.push(PostCandidate::from_post(sample_post(
                &format!("oon_{i:02}"),
                &aid,
                &h,
                if i % 2 == 0 { ActorKind::Bot } else { ActorKind::Human },
                &slug,
                &format!("Discovery topic {i}"),
                &format!("Fresh out-of-network insight {i}"),
                80 - (i as i64),
                1,
                now - (i as i64) * 90_000,
            )));
        }

        let single_30 = CommunityRecommender::rank_feed(
            &viewer,
            cands.clone(),
            &FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                limit: 30,
                ..Default::default()
            },
        );
        let p1 = CommunityRecommender::rank_feed(
            &viewer,
            cands.clone(),
            &FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                limit: 10,
                ..Default::default()
            },
        );
        assert!(
            p1.items.iter().any(|it| it.is_exploration_slot),
            "Page 1 (limit=10, discovery_ratio=0.15) must include at least 1 exploration slot"
        );
        let p2 = CommunityRecommender::rank_feed(
            &viewer,
            cands.clone(),
            &FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                limit: 10,
                cursor: p1.next_cursor.clone(),
                ..Default::default()
            },
        );
        let p3 = CommunityRecommender::rank_feed(
            &viewer,
            cands,
            &FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                limit: 10,
                cursor: p2.next_cursor.clone(),
                ..Default::default()
            },
        );

        let stitched: Vec<String> = p1
            .items
            .iter()
            .chain(p2.items.iter())
            .chain(p3.items.iter())
            .map(|i| i.post.post_id.clone())
            .collect();
        let single_ids: Vec<String> = single_30
            .items
            .iter()
            .map(|i| i.post.post_id.clone())
            .collect();
        assert_eq!(stitched.len(), 30);
        assert_eq!(
            stitched, single_ids,
            "stitched Page1+Page2+Page3 must match single limit=30 verbatim"
        );
        let uniq: HashSet<&str> = stitched.iter().map(String::as_str).collect();
        assert_eq!(uniq.len(), 30, "zero duplicate or dropped posts across pages");
    }

    #[test]
    fn mmr_staged_relaxation_preserves_author_cap_and_no_consecutive() {
        let now = 1_760_100_000_000_i64;
        let mut viewer = ViewerContext {
            actor_id: "u:viewer".into(),
            now_ms: now,
            ..Default::default()
        };
        let comms = ["c0", "c1", "c2", "c3", "c4", "c5"];
        for c in comms {
            viewer.joined_communities.insert(c.into());
        }

        // Repro A: 100 posts across 6 communities (6*3=18 < limit=20) with a dominant spammer + 19 other authors
        let mut cands = Vec::new();
        for i in 0..40usize {
            cands.push(PostCandidate::from_post(sample_post(
                &format!("spam_{i:02}"),
                "u:spammer",
                "spammer",
                ActorKind::Human,
                comms[i % 6],
                &format!("Spam post {i}"),
                &format!("High score spam {i}"),
                1000 - i as i64,
                0,
                now - (i as i64) * 1000,
            )));
        }
        for i in 0..60usize {
            let author_idx = i % 19;
            cands.push(PostCandidate::from_post(sample_post(
                &format!("reg_{i:02}"),
                &format!("u:author_{author_idx}"),
                &format!("author_{author_idx}"),
                if i % 2 == 0 { ActorKind::Bot } else { ActorKind::Human },
                comms[i % 6],
                &format!("Regular post {i}"),
                &format!("Regular content {i}"),
                200 - i as i64,
                1,
                now - (i as i64) * 2000,
            )));
        }

        let res = CommunityRecommender::rank_feed(
            &viewer,
            cands.clone(),
            &FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                limit: 20,
                ..Default::default()
            },
        );
        assert_eq!(res.items.len(), 20);
        let spammer_count = res
            .items
            .iter()
            .filter(|i| i.post.author_actor_id == "u:spammer")
            .count();
        assert!(
            spammer_count <= 2,
            "author cap must hold when comm_cap relaxes (got {spammer_count})"
        );
        for w in res.items.windows(2) {
            assert_ne!(
                w[0].post.author_actor_id, w[1].post.author_actor_id,
                "no consecutive same author violation allowed"
            );
        }

        // Repro B: all-in-network viewer (0 OON posts) on limit=15 must still honor author cap=2
        let res_in = CommunityRecommender::rank_feed(
            &viewer,
            cands,
            &FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                limit: 15,
                ..Default::default()
            },
        );
        let spam_in = res_in
            .items
            .iter()
            .filter(|i| i.post.author_actor_id == "u:spammer")
            .count();
        assert!(spam_in <= 2, "all-in-network exploration fallback must keep author cap <= 2");
    }

    #[test]
    fn own_profile_handle_shows_draft_and_scheduled_and_safety_gating_closes_all_leaks() {
        use gpui_telegram_proto::community_contracts::{CommunityPollOptionDto, PollDto};

        let now = 1_760_100_000_000_i64;
        let mut viewer = ViewerContext {
            actor_id: "u:900001".into(),
            now_ms: now,
            ..Default::default()
        };
        viewer.muted_communities.insert("C/MixedCaseComm".into());
        viewer.joined_communities.insert("C/Rust".into());
        viewer.community_affinities.insert("C/Rust".into(), 5.0);
        viewer.blocked_actor_ids.insert("u:blocked".into());
        viewer.muted_actor_ids.insert("u:muted".into());
        viewer.muted_keywords.push("forbidden_kw".into());

        // Own draft & scheduled posts
        let mut own_draft = sample_post("p_own_draft", "u:900001", "alex_chen", ActorKind::Human, "rust", "Draft", "WIP", 1, 0, now - 1000);
        own_draft.status = PostStatus::Draft;
        let mut own_sched = sample_post("p_own_sched", "u:900001", "alex_chen", ActorKind::Human, "rust", "Sched", "Later", 1, 0, now - 1000);
        own_sched.status = PostStatus::Scheduled;
        own_sched.scheduled_at_ms = Some(now + 60_000);
        let mut other_draft = sample_post("p_other_draft", "u:900002", "other", ActorKind::Human, "rust", "Other", "Hidden", 1, 0, now - 1000);
        other_draft.status = PostStatus::Draft;

        // Safety-gating leak probes
        let p_muted_comm = sample_post("p_muted_comm", "u:2", "u2", ActorKind::Human, "mixedcasecomm", "Hi", "Body", 50, 0, now - 1000);
        let mut p_tag_kw = sample_post("p_tag_kw", "u:2", "u2", ActorKind::Human, "rust", "Clean", "Clean", 50, 0, now - 1000);
        p_tag_kw.tags = vec!["FORBIDDEN_KW".into()];
        let mut p_flair_kw = sample_post("p_flair_kw", "u:2", "u2", ActorKind::Human, "rust", "Clean", "Clean", 50, 0, now - 1000);
        p_flair_kw.flair = Some("forbidden_kw flair".into());
        let mut p_poll_kw = sample_post("p_poll_kw", "u:2", "u2", ActorKind::Human, "rust", "Clean", "Clean", 50, 0, now - 1000);
        p_poll_kw.poll = Some(PollDto {
            poll_id: "pl1".into(),
            question: "Which one?".into(),
            options: vec![CommunityPollOptionDto {
                option_id: "o0".into(),
                text: "Option with forbidden_kw".into(),
                all_votes: 1,
                human_votes: 1,
                bot_votes: 0,
            }],
            multi_select: false,
            expires_at_ms: now + 60_000,
            is_closed: false,
            total_votes: 1,
            total_human_votes: 1,
            total_bot_votes: 0,
            viewer_voted_option_ids: vec![],
        });
        let mut p_coauthor = sample_post("p_coauthor", "u:2", "u2", ActorKind::Human, "rust", "Co", "Body", 50, 0, now - 1000);
        p_coauthor.co_author_actor_id = Some("u:blocked".into());
        let mut p_quoted = sample_post("p_quoted", "u:2", "u2", ActorKind::Human, "rust", "Quote", "Body", 50, 0, now - 1000);
        p_quoted.quoted_post = Some(Box::new(
            gpui_telegram_proto::community_contracts::EmbeddedPostSummaryDto {
                post_id: "p_inner".into(),
                community_slug: Some("rust".into()),
                author_actor_id: "u:muted".into(),
                author_handle: "muted".into(),
                author_display_name: "Muted".into(),
                author_kind: ActorKind::Human,
                author_avatar_shape: "classic".into(),
                author_avatar_color: "blue".into(),
                title: Some("Inner".into()),
                body_preview: "Body".into(),
                format: PostFormat::Micro,
                score: 10,
                comments_count: 0,
                created_at_ms: now - 2000,
            },
        ));
        let p_clean = sample_post("p_clean", "u:2", "u2", ActorKind::Human, "Rust", "Clean Rust", "Good body", 50, 0, now - 1000);

        let cands: Vec<PostCandidate> = vec![
            own_draft, own_sched, other_draft, p_muted_comm, p_tag_kw, p_flair_kw, p_poll_kw,
            p_coauthor, p_quoted, p_clean.clone(),
        ]
        .into_iter()
        .map(PostCandidate::from_post)
        .collect();

        // 1. Viewing own profile by @handle or handle shows own Draft & Scheduled posts
        for target in ["alex_chen", "@alex_chen", "u:900001"] {
            let prof = CommunityRecommender::rank_feed(
                &viewer,
                cands.clone(),
                &FeedRequestDto {
                    tab: FeedTab::ActorPosts(target.into()),
                    sort: FeedSort::New,
                    ..Default::default()
                },
            );
            let ids: Vec<&str> = prof.items.iter().map(|i| i.post.post_id.as_str()).collect();
            assert!(ids.contains(&"p_own_draft"), "own draft missing for {target}");
            assert!(ids.contains(&"p_own_sched"), "own scheduled missing for {target}");
            assert!(!ids.contains(&"p_other_draft"));
        }

        // 2. Public ForYou feed excludes all drafts, scheduled, muted comm, muted keywords (tags/flair/poll), blocked co-author & muted quoted post
        let pub_feed = CommunityRecommender::rank_feed(
            &viewer,
            cands,
            &FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                ..Default::default()
            },
        );
        let pub_ids: Vec<&str> = pub_feed.items.iter().map(|i| i.post.post_id.as_str()).collect();
        assert_eq!(pub_ids, vec!["p_clean"]);

        // 3. Case-insensitive community scoring gives identical score for "rust" and "Rust"
        let mut p_lower = p_clean.clone();
        p_lower.community_slug = Some("rust".into());
        let (s_upper, _) = score_for_you(&viewer, &PostCandidate::from_post(p_clean), &viewer.algo_prefs, now);
        let (s_lower, _) = score_for_you(&viewer, &PostCandidate::from_post(p_lower), &viewer.algo_prefs, now);
        assert!((s_upper - s_lower).abs() < 1e-12, "score differed by case: {s_upper} vs {s_lower}");
    }

    #[test]
    fn overflow_boundaries_negative_wilson_and_trending_determinism() {
        let now = 1_760_100_000_000_i64;
        // Negative z in wilson_lower_bound must return 0.0 (never upper bound)
        assert_eq!(wilson_lower_bound(10, 10, -1.96), 0.0);
        assert_eq!(wilson_lower_bound(10, 10, f64::NAN), 0.0);

        // reddit_hot on i64::MIN score must not panic
        let h_min = reddit_hot(i64::MIN, 0, now);
        assert!(h_min.is_finite());
        let h_min2 = reddit_hot(-1, i64::MAX, now);
        assert!(h_min2.is_finite());

        // rank_feed on i64::MIN created_at_ms and usize::MAX cursor must not panic
        let viewer = ViewerContext {
            actor_id: "u:1".into(),
            now_ms: now,
            ..Default::default()
        };
        let p_extreme = sample_post("p_min_ts", "u:2", "u2", ActorKind::Human, "rust", "Ancient", "Body", 10, 0, i64::MIN);
        let res = CommunityRecommender::rank_feed(
            &viewer,
            vec![PostCandidate::from_post(p_extreme)],
            &FeedRequestDto {
                tab: FeedTab::ForYou,
                sort: FeedSort::Algo,
                cursor: Some(format!("offset:{}", usize::MAX)),
                limit: 10,
                ..Default::default()
            },
        );
        assert!(res.items.is_empty());
        assert!(res.next_cursor.is_none());

        // TrendingEngine::compute must never flip tie order between #rust and c/rust across 50 runs
        let tie_post = sample_post("p_tie", "u:2", "u2", ActorKind::Human, "rust", "Rust", "Talking about #rust", 50, 0, now - 60_000);
        let first_ids: Vec<String> = TrendingEngine::compute(std::slice::from_ref(&tie_post), now, 10)
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(first_ids.contains(&"trend_comm_rust".to_string()));
        assert!(first_ids.contains(&"trend_tag_rust".to_string()));
        for _ in 0..50 {
            let run_ids: Vec<String> = TrendingEngine::compute(std::slice::from_ref(&tie_post), now, 10)
                .into_iter()
                .map(|t| t.id)
                .collect();
            assert_eq!(run_ids, first_ids, "TrendingEngine tie-break order must be strictly deterministic");
        }
    }

    #[test]
    fn nan_sanitization_mixed_case_affinities_trending_c_prefix_embedded_gating_and_limit25_cap() {
        use gpui_telegram_proto::community_contracts::{EmbeddedPostSummaryDto, LinkPreviewDto};

        let now = 1_760_100_000_000_i64;

        // 1. blend_interest_vector NaN/Inf alpha and element guard
        let base = embed_text_64("rust systems");
        let item = embed_text_64("gpui desktop");
        for bad_alpha in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let blended = blend_interest_vector(&base, &item, bad_alpha);
            assert!(
                blended.iter().all(|v| v.is_finite()),
                "blend_interest_vector produced non-finite value for alpha={bad_alpha}"
            );
            assert!(cosine_similarity_64(&blended, &base) > 0.99);
        }
        let nan_vec = [f32::NAN; EMBEDDING_DIMS];
        let clean = blend_interest_vector(&nan_vec, &nan_vec, 0.5);
        assert!(clean.iter().all(|&v| v == 0.0));

        // 2. rerank_with_mmr NaN/Inf hijack prevention
        let sc_nan = ScoredCandidate {
            candidate: PostCandidate::from_post(sample_post(
                "p_nan", "u:1", "u1", ActorKind::Human, "rust", "NaN", "Body", 1, 0, now,
            )),
            score: f64::NAN,
            explanation: simple_explanation("test", "test"),
            is_out_of_network: false,
            is_exploration_slot: false,
        };
        let sc_inf = ScoredCandidate {
            candidate: PostCandidate::from_post(sample_post(
                "p_inf", "u:3", "u3", ActorKind::Human, "rust", "Inf", "Body", 1, 0, now,
            )),
            score: f64::INFINITY,
            explanation: simple_explanation("test", "test"),
            is_out_of_network: false,
            is_exploration_slot: false,
        };
        let sc_good = ScoredCandidate {
            candidate: PostCandidate::from_post(sample_post(
                "p_good", "u:2", "u2", ActorKind::Human, "systems", "Good", "Body", 10, 0, now,
            )),
            score: 10.0,
            explanation: simple_explanation("test", "test"),
            is_out_of_network: false,
            is_exploration_slot: false,
        };
        let ranked_mmr = rerank_with_mmr(vec![sc_nan, sc_inf, sc_good], 3, &DiversityConfig::default());
        assert_eq!(ranked_mmr[0].candidate.post.post_id, "p_good");

        // 3. community_affinities mixed-case duplicate keys stay 100% deterministic
        let mut v_aff = ViewerContext {
            actor_id: "u:viewer".into(),
            now_ms: now,
            ..Default::default()
        };
        v_aff.community_affinities.insert("Rust".into(), 10.0);
        v_aff.community_affinities.insert("C/Rust".into(), -10.0);
        let probe_cand = PostCandidate::from_post(sample_post(
            "p_aff", "u:2", "u2", ActorKind::Human, "rust", "Title", "Body", 20, 0, now - 1000,
        ));
        let (s0, _) = score_for_you(&v_aff, &probe_cand, &v_aff.algo_prefs, now);
        for _ in 0..40 {
            let (si, _) = score_for_you(&v_aff, &probe_cand, &v_aff.algo_prefs, now);
            assert_eq!(si.to_bits(), s0.to_bits(), "mixed-case community_affinities must be deterministic");
        }

        // 4. TrendingEngine C/Rust normalization -> trend_comm_rust & label c/rust
        let mut p_cr = sample_post("p_cr", "u:2", "u2", ActorKind::Human, "C/Rust", "Rust", "Body", 40, 0, now - 1000);
        p_cr.community_slug = Some("C/Rust".into());
        let trends = TrendingEngine::compute(&[p_cr], now, 10);
        let comm_trend = trends
            .iter()
            .find(|t| t.kind == gpui_telegram_proto::community_contracts::TrendingItemKind::Community)
            .expect("community trend");
        assert_eq!(comm_trend.id, "trend_comm_rust");
        assert_eq!(comm_trend.label, "c/rust");
        assert_eq!(comm_trend.slug_or_tag, "rust");

        // 5. Embedded muted_communities, embedded muted_keywords, link_preview muted_keywords, and ActorPosts(" @alex ")
        let mut viewer = ViewerContext {
            actor_id: "u:900001".into(),
            now_ms: now,
            ..Default::default()
        };
        viewer.muted_communities.insert("C/MutedComm".into());
        viewer.muted_keywords.push("banned_secret".into());

        let mk_embed = |slug: Option<&str>, title: Option<&str>, body: &str| {
            Box::new(EmbeddedPostSummaryDto {
                post_id: "p_inner".into(),
                community_slug: slug.map(str::to_string),
                author_actor_id: "u:other".into(),
                author_handle: "other".into(),
                author_display_name: "Other".into(),
                author_kind: ActorKind::Human,
                author_avatar_shape: "classic".into(),
                author_avatar_color: "blue".into(),
                title: title.map(str::to_string),
                body_preview: body.into(),
                format: PostFormat::Micro,
                score: 5,
                comments_count: 0,
                created_at_ms: now - 2000,
            })
        };

        let mut p_q_mc = sample_post("p_q_mc", "u:2", "alex", ActorKind::Human, "rust", "Clean", "Clean", 10, 0, now - 1000);
        p_q_mc.quoted_post = Some(mk_embed(Some("mutedcomm"), Some("Inner"), "Clean"));
        let mut p_r_mc = sample_post("p_r_mc", "u:2", "alex", ActorKind::Human, "rust", "Clean", "Clean", 10, 0, now - 1000);
        p_r_mc.reposted_post = Some(mk_embed(Some("C/MutedComm"), Some("Inner"), "Clean"));
        let mut p_x_kw = sample_post("p_x_kw", "u:2", "alex", ActorKind::Human, "rust", "Clean", "Clean", 10, 0, now - 1000);
        p_x_kw.crosspost_origin = Some(mk_embed(Some("systems"), Some("Title with BANNED_SECRET"), "Clean"));
        let mut p_q_kw = sample_post("p_q_kw", "u:2", "alex", ActorKind::Human, "rust", "Clean", "Clean", 10, 0, now - 1000);
        p_q_kw.quoted_post = Some(mk_embed(Some("systems"), None, "Contains banned_secret preview"));
        let mut p_lp_kw = sample_post("p_lp_kw", "u:2", "alex", ActorKind::Human, "rust", "Clean", "Clean", 10, 0, now - 1000);
        p_lp_kw.link_preview = Some(LinkPreviewDto {
            url: "https://example.org".into(),
            domain: "example.org".into(),
            title: "Clean Title".into(),
            description: Some("Hidden banned_secret inside link card".into()),
            image_url: None,
        });
        let p_ok = sample_post("p_ok", "u:2", "alex", ActorKind::Human, "rust", "Clean", "Clean", 10, 0, now - 1000);

        let safety_cands: Vec<PostCandidate> = vec![p_q_mc, p_r_mc, p_x_kw, p_q_kw, p_lp_kw, p_ok]
            .into_iter()
            .map(PostCandidate::from_post)
            .collect();
        let gated = CommunityRecommender::rank_feed(&viewer, safety_cands.clone(), &FeedRequestDto::default());
        let gated_ids: Vec<&str> = gated.items.iter().map(|i| i.post.post_id.as_str()).collect();
        assert_eq!(gated_ids, vec!["p_ok"]);

        // FeedTab::ActorPosts(" @alex ") with surrounding whitespace matches @alex cleanly
        let trimmed_actor = CommunityRecommender::rank_feed(
            &viewer,
            safety_cands,
            &FeedRequestDto {
                tab: FeedTab::ActorPosts(" @alex ".into()),
                sort: FeedSort::New,
                ..Default::default()
            },
        );
        assert_eq!(trimmed_actor.items.len(), 1);
        assert_eq!(trimmed_actor.items[0].post.post_id, "p_ok");

        // 6. Default FeedRequestDto::default() (limit = 25) enforces max_per_author_per_page <= 2
        let mut cap_cands = Vec::new();
        for i in 0..30usize {
            cap_cands.push(PostCandidate::from_post(sample_post(
                &format!("sp_{i:02}"),
                "u:spammer",
                "spammer",
                ActorKind::Human,
                &format!("c{}", i % 10),
                &format!("Spam {i}"),
                &format!("Body {i}"),
                1000 - i as i64,
                0,
                now - (i as i64) * 1000,
            )));
        }
        for i in 0..30usize {
            cap_cands.push(PostCandidate::from_post(sample_post(
                &format!("ok_{i:02}"),
                &format!("u:reg_{}", i % 25),
                &format!("reg_{}", i % 25),
                if i % 2 == 0 { ActorKind::Bot } else { ActorKind::Human },
                &format!("c{}", i % 10),
                &format!("Regular {i}"),
                &format!("Normal post {i}"),
                200 - i as i64,
                0,
                now - (i as i64) * 2000,
            )));
        }
        let def_res = CommunityRecommender::rank_feed(&viewer, cap_cands, &FeedRequestDto::default());
        assert_eq!(def_res.items.len(), 25);
        let spammer_on_page = def_res
            .items
            .iter()
            .filter(|i| i.post.author_actor_id == "u:spammer")
            .count();
        assert!(
            spammer_on_page <= 2,
            "default limit=25 page must cap spammer to <= 2 (got {spammer_on_page})"
        );
    }
}
