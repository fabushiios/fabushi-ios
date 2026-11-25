//! Greedy MMR diversity re-ranking, author/community spacing, Human/Bot fairness, and exploration slots.

use crate::recommender::signals::{
    PostCandidate, cosine_similarity_64, fnv1a_64, splitmix64_unit,
};
use gpui_telegram_proto::community_contracts::FeedExplanationDto;
use std::collections::HashMap;

/// Fixed sliding window size used for per-page author/community caps so `rerank_with_mmr`
/// enforces exact per-page caps over any 1..=25 page (matching `default_feed_limit()`)
/// while staying strictly prefix-consistent across arbitrary pagination offsets & limits.
const CAP_WINDOW: usize = 25;

#[inline]
fn sanitize_rank_score(score: f64) -> f64 {
    if score.is_finite() {
        score
    } else {
        f64::NEG_INFINITY
    }
}

/// A candidate with its Stage-3 relevance score and optional explanation.
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub candidate: PostCandidate,
    pub score: f64,
    pub explanation: FeedExplanationDto,
    pub is_out_of_network: bool,
    pub is_exploration_slot: bool,
}

/// Configuration for Stage-4 Greedy MMR diversity & fairness re-ranking.
#[derive(Debug, Clone)]
pub struct DiversityConfig {
    pub mmr_lambda: f64,
    pub max_per_author_per_page: usize,
    pub max_per_community_per_page: Option<usize>,
    pub no_consecutive_same_author: bool,
    pub target_bot_ratio: f64,
    pub exploration_ratio: f64,
    pub viewer_seed: u64,
}

impl Default for DiversityConfig {
    fn default() -> Self {
        Self {
            mmr_lambda: 0.78,
            max_per_author_per_page: 2,
            max_per_community_per_page: Some(3),
            no_consecutive_same_author: true,
            target_bot_ratio: 0.50,
            exploration_ratio: 0.15,
            viewer_seed: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct RelaxationStage {
    require_oon: bool,
    enforce_no_consecutive: bool,
    enforce_author_cap: bool,
    enforce_comm_cap: bool,
}

/// Re-ranks scored candidates using Greedy Maximal Marginal Relevance (MMR) with
/// strictly prefix-consistent pagination, 5-stage progressive cap relaxation, sliding-window
/// author/community caps, Human/Bot ratio alignment, and ratio-spaced exploration slots.
pub fn rerank_with_mmr(
    mut scored: Vec<ScoredCandidate>,
    limit: usize,
    cfg: &DiversityConfig,
) -> Vec<ScoredCandidate> {
    if scored.is_empty() || limit == 0 {
        return Vec::new();
    }

    // Deterministic initial ordering so tie breaks never depend on HashMap iteration order,
    // and non-finite scores (NaN/Inf) are demoted below all finite scores.
    scored.sort_by(|a, b| {
        sanitize_rank_score(b.score)
            .total_cmp(&sanitize_rank_score(a.score))
            .then_with(|| b.candidate.post.created_at_ms.cmp(&a.candidate.post.created_at_ms))
            .then_with(|| a.candidate.post.post_id.cmp(&b.candidate.post.post_id))
    });

    let target_len = limit.min(scored.len());
    // Derive exploration stride purely from `cfg.exploration_ratio` (independent of `target_len`)
    // so prefix slots 0..k are 100% invariant across pagination offsets & limits.
    let exp_ratio = if cfg.exploration_ratio.is_finite() {
        cfg.exploration_ratio.clamp(0.0, 0.5)
    } else {
        0.0
    };
    let exploration_stride = if exp_ratio > 0.0 {
        ((1.0 / exp_ratio.clamp(0.01, 1.0)).round() as usize).max(3)
    } else {
        usize::MAX
    };

    let mut selected: Vec<ScoredCandidate> = Vec::with_capacity(target_len);
    let mut remaining = scored;
    let mut selected_bots: usize = 0;

    let lambda = if cfg.mmr_lambda.is_finite() {
        cfg.mmr_lambda.clamp(0.1, 1.0)
    } else {
        0.78
    };
    let target_bot_ratio = if cfg.target_bot_ratio.is_finite() {
        cfg.target_bot_ratio.clamp(0.0, 1.0)
    } else {
        0.50
    };

    // 5-stage progressive relaxation order:
    // Stage 0 (only when `want_exploration`): out-of-network + all caps
    // Stage 1: any candidate + all caps (no_consecutive, author_cap, comm_cap)
    // Stage 2: any candidate + no_consecutive + author_cap (relaxes comm_cap only)
    // Stage 3: any candidate + no_consecutive only
    // Stage 4: unconstrained fallback
    const ALL_STAGES: [RelaxationStage; 5] = [
        RelaxationStage {
            require_oon: true,
            enforce_no_consecutive: true,
            enforce_author_cap: true,
            enforce_comm_cap: true,
        },
        RelaxationStage {
            require_oon: false,
            enforce_no_consecutive: true,
            enforce_author_cap: true,
            enforce_comm_cap: true,
        },
        RelaxationStage {
            require_oon: false,
            enforce_no_consecutive: true,
            enforce_author_cap: true,
            enforce_comm_cap: false,
        },
        RelaxationStage {
            require_oon: false,
            enforce_no_consecutive: true,
            enforce_author_cap: false,
            enforce_comm_cap: false,
        },
        RelaxationStage {
            require_oon: false,
            enforce_no_consecutive: false,
            enforce_author_cap: false,
            enforce_comm_cap: false,
        },
    ];

    for slot_idx in 0..target_len {
        if remaining.is_empty() {
            break;
        }
        let want_exploration = exploration_stride != usize::MAX
            && slot_idx > 0
            && ((slot_idx + 1) % exploration_stride == 0);

        // Fixed sliding window over the preceding `CAP_WINDOW - 1` items so any contiguous
        // 25-item page enforces exact caps without depending on `limit` or `offset`.
        let win_start = selected.len().saturating_sub(CAP_WINDOW - 1);
        let mut author_counts: HashMap<&str, usize> = HashMap::new();
        let mut community_counts: HashMap<String, usize> = HashMap::new();
        for picked in &selected[win_start..] {
            *author_counts
                .entry(picked.candidate.post.author_actor_id.as_str())
                .or_insert(0) += 1;
            if let Some(slug) = picked.candidate.post.community_slug.as_deref() {
                *community_counts
                    .entry(super::normalize_community_key(slug))
                    .or_insert(0) += 1;
            }
        }

        let last_author = selected
            .last()
            .map(|s| s.candidate.post.author_actor_id.as_str());
        let has_both_kinds = remaining
            .iter()
            .any(|c| c.candidate.post.author_kind.is_bot_like())
            && remaining
                .iter()
                .any(|c| !c.candidate.post.author_kind.is_bot_like());

        let stages: &[RelaxationStage] = if want_exploration {
            &ALL_STAGES[..]
        } else {
            &ALL_STAGES[1..]
        };

        let mut best_idx: Option<usize> = None;
        let mut best_mmr = f64::NEG_INFINITY;

        for stage in stages {
            for (idx, item) in remaining.iter().enumerate() {
                if stage.require_oon && !item.is_out_of_network {
                    continue;
                }
                let author_id = item.candidate.post.author_actor_id.as_str();
                if stage.enforce_no_consecutive
                    && cfg.no_consecutive_same_author
                    && last_author == Some(author_id)
                {
                    continue;
                }
                if stage.enforce_author_cap {
                    let ac = author_counts.get(author_id).copied().unwrap_or(0);
                    if ac >= cfg.max_per_author_per_page {
                        continue;
                    }
                }
                if stage.enforce_comm_cap {
                    if let (Some(comm_cap), Some(slug)) = (
                        cfg.max_per_community_per_page,
                        item.candidate.post.community_slug.as_deref(),
                    ) {
                        let norm = super::normalize_community_key(slug);
                        let cc = community_counts.get(&norm).copied().unwrap_or(0);
                        if cc >= comm_cap {
                            continue;
                        }
                    }
                }

                let max_sim = selected
                    .iter()
                    .map(|picked| {
                        cosine_similarity_64(
                            &item.candidate.content_embedding,
                            &picked.candidate.content_embedding,
                        )
                        .max(0.0)
                    })
                    .fold(0.0_f64, f64::max);

                // Soft Human/Bot balance nudge when both citizen types are available.
                let balance_nudge = if has_both_kinds && !selected.is_empty() {
                    let current_bot_ratio = (selected_bots as f64) / (selected.len() as f64);
                    let is_bot = item.candidate.post.author_kind.is_bot_like();
                    let delta = target_bot_ratio - current_bot_ratio;
                    if is_bot {
                        (delta * 0.18).clamp(-0.12, 0.12)
                    } else {
                        (-delta * 0.18).clamp(-0.12, 0.12)
                    }
                } else {
                    0.0
                };

                let base_score = sanitize_rank_score(item.score);
                let mmr_raw = lambda * base_score - (1.0 - lambda) * max_sim + balance_nudge;
                let mmr_score = if mmr_raw.is_finite() {
                    mmr_raw
                } else {
                    f64::NEG_INFINITY
                };
                let better = match best_idx {
                    None => true,
                    Some(prev_i) => {
                        if mmr_score > best_mmr + 1e-12 {
                            true
                        } else if mmr_score == best_mmr || (mmr_score - best_mmr).abs() <= 1e-12 {
                            let cur_tie = splitmix64_unit(
                                fnv1a_64(item.candidate.post.post_id.as_bytes()) ^ cfg.viewer_seed,
                            );
                            let prev_tie = splitmix64_unit(
                                fnv1a_64(remaining[prev_i].candidate.post.post_id.as_bytes())
                                    ^ cfg.viewer_seed,
                            );
                            cur_tie
                                .total_cmp(&prev_tie)
                                .then_with(|| {
                                    remaining[prev_i]
                                        .candidate
                                        .post
                                        .post_id
                                        .cmp(&item.candidate.post.post_id)
                                })
                                .is_gt()
                        } else {
                            false
                        }
                    }
                };
                if better {
                    best_mmr = mmr_score;
                    best_idx = Some(idx);
                }
            }
            if best_idx.is_some() {
                break;
            }
        }

        let pick_idx = best_idx.unwrap_or(0);
        let mut chosen = remaining.remove(pick_idx);
        if want_exploration && chosen.is_out_of_network {
            chosen.is_exploration_slot = true;
            chosen.explanation.primary_reason =
                format!("Discovery pick · {}", chosen.explanation.primary_reason);
        }
        if chosen.candidate.post.author_kind.is_bot_like() {
            selected_bots += 1;
        }
        selected.push(chosen);
    }

    selected
}
