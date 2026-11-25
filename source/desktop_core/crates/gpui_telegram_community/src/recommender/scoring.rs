//! Authentic Reddit mathematical ranking formulas + X/Hybrid For You composite scoring + presets.

use crate::recommender::signals::{
    PostCandidate, ViewerContext, cosine_similarity_64, fnv1a_64, splitmix64_unit,
};
use gpui_telegram_proto::community_contracts::{
    AlgoPreferencesDto, AlgoPreset, FeedExplanationDto, SignalContributionDto, TimeWindow,
};

const REDDIT_EPOCH_SECS: f64 = 1_134_028_003.0;

/// Wilson score lower bound (authentic Reddit `Best` sort, `z = 1.96` for 95% confidence).
pub fn wilson_lower_bound(upvotes: i64, downvotes: i64, z: f64) -> f64 {
    let up = upvotes.max(0) as f64;
    let down = downvotes.max(0) as f64;
    let n = up + down;
    if n <= 0.0 || !z.is_finite() || z < 0.0 {
        return 0.0;
    }
    let phat = up / n;
    let z2 = z * z;
    let numerator = phat + z2 / (2.0 * n) - z * ((phat * (1.0 - phat) + z2 / (4.0 * n)) / n).sqrt();
    let denominator = 1.0 + z2 / n;
    let res = numerator / denominator;
    if res.is_finite() { res.max(0.0) } else { 0.0 }
}

/// Authentic Reddit `Hot` ranking formula.
pub fn reddit_hot(upvotes: i64, downvotes: i64, created_at_ms: i64) -> f64 {
    let score = upvotes.saturating_sub(downvotes);
    let order = (score.unsigned_abs().max(1) as f64).log10();
    let sign = if score > 0 {
        1.0
    } else if score < 0 {
        -1.0
    } else {
        0.0
    };
    let seconds = (created_at_ms as f64 / 1000.0) - REDDIT_EPOCH_SECS;
    let res = sign * order + seconds / 45_000.0;
    if res.is_finite() { res } else { 0.0 }
}

/// Authentic Reddit `Controversial` formula: magnitude raised to balance exponent.
pub fn reddit_controversial(upvotes: i64, downvotes: i64) -> f64 {
    if upvotes <= 0 || downvotes <= 0 {
        return 0.0;
    }
    let up = upvotes as f64;
    let down = downvotes as f64;
    let magnitude = up + down;
    let balance = if up > down { down / up } else { up / down };
    let res = magnitude.powf(balance);
    if res.is_finite() { res } else { 0.0 }
}

/// Reddit `Rising` burst-velocity formula over recent age in hours.
pub fn reddit_rising(upvotes: i64, downvotes: i64, comments_count: u64, age_hours: f64) -> f64 {
    let score = upvotes.saturating_sub(downvotes).max(0) as f64;
    let numerator = score + 1.5 * (comments_count as f64);
    let denom = (age_hours.max(0.0) + 0.5).powf(1.5);
    let res = numerator / denom;
    if res.is_finite() { res } else { 0.0 }
}

/// Returns the inclusive timestamp cutoff (in milliseconds) for a `TimeWindow`.
pub fn time_window_cutoff_ms(window: TimeWindow, now_ms: i64) -> Option<i64> {
    let span_ms = match window {
        TimeWindow::Hour => 3_600_000_i64,
        TimeWindow::Day => 86_400_000_i64,
        TimeWindow::Week => 7 * 86_400_000_i64,
        TimeWindow::Month => 30 * 86_400_000_i64,
        TimeWindow::Year => 365 * 86_400_000_i64,
        TimeWindow::All => return None,
    };
    Some(now_ms.saturating_sub(span_ms))
}

/// Canonical default tuning weights for all 7 `AlgoPreset`s.
pub fn default_prefs_for_preset(preset: AlgoPreset) -> AlgoPreferencesDto {
    let (recency, affinity, quality, discovery, bot_ratio) = match preset {
        AlgoPreset::Balanced => (0.35, 0.35, 0.30, 0.15, 0.50),
        AlgoPreset::RedditDeepDive => (0.15, 0.25, 0.60, 0.15, 0.45),
        AlgoPreset::XRealTime => (0.60, 0.25, 0.15, 0.10, 0.50),
        AlgoPreset::Discovery => (0.25, 0.15, 0.60, 0.35, 0.50),
        AlgoPreset::HumanFocused => (0.35, 0.35, 0.30, 0.15, 0.10),
        AlgoPreset::BotHeavy => (0.35, 0.30, 0.35, 0.20, 0.85),
        AlgoPreset::Chronological => (1.00, 0.00, 0.00, 0.00, 0.50),
    };
    AlgoPreferencesDto {
        preset,
        recency_weight: recency,
        affinity_weight: affinity,
        quality_weight: quality,
        discovery_ratio: discovery,
        bot_content_ratio: bot_ratio,
        show_explanations: true,
        allow_nsfw: false,
    }
}

/// Computes the composite X + Reddit `For You` score alongside an accurate `FeedExplanationDto` (breakdown summing to 100%).
pub fn score_for_you(
    viewer: &ViewerContext,
    cand: &PostCandidate,
    prefs: &AlgoPreferencesDto,
    now_ms: i64,
) -> (f64, FeedExplanationDto) {
    let post = &cand.post;

    // 1. Quality signal: Wilson score + log-scaled multi-engagement + author trust
    let wilson = wilson_lower_bound(post.upvotes, post.downvotes, 1.96);
    let eng_raw = (post.score.max(0) as f64)
        + 1.6 * (post.comments_count as f64)
        + 1.2 * (post.likes_count as f64)
        + 2.0 * (post.reposts_count as f64)
        + 2.2 * (post.quotes_count as f64)
        + 1.8 * (post.bookmarks_count as f64)
        + 3.0 * (post.awards.len() as f64);
    let eng_norm = ((1.0 + eng_raw).ln() / 7.0).clamp(0.0, 1.0);
    let trust = if cand.author_trust_score.is_finite() {
        cand.author_trust_score.clamp(0.05, 1.0)
    } else {
        0.50
    };
    let quality_signal = (0.45 * wilson + 0.35 * eng_norm + 0.20 * trust).clamp(0.0, 1.2);

    // 2. Recency signal: exponential decay (10h half-life for XRealTime, 20h default, 36h RedditDeepDive)
    let age_hours = (now_ms.saturating_sub(post.created_at_ms).max(0) as f64) / 3_600_000.0;
    let half_life_hours = match prefs.preset {
        AlgoPreset::XRealTime => 10.0,
        AlgoPreset::RedditDeepDive => 36.0,
        _ => 20.0,
    };
    let recency_signal = (-std::f64::consts::LN_2 * age_hours / half_life_hours).exp();

    // 3. Affinity signal: direct follow + joined community + 2-hop social + historical affinity
    let direct_follow = if viewer.following_actor_ids.contains(&post.author_actor_id) {
        0.55
    } else {
        0.0
    };
    let norm_slug = post
        .community_slug
        .as_deref()
        .map(super::normalize_community_key)
        .filter(|s| !s.is_empty());
    let joined_comm = norm_slug.as_deref().is_some_and(|s| {
        viewer.joined_communities.contains(s)
            || viewer
                .joined_communities
                .iter()
                .any(|jc| super::normalize_community_key(jc) == s)
    });
    let comm_bonus = if joined_comm { 0.35 } else { 0.0 };
    let two_hop_raw = viewer
        .second_hop_weights
        .get(&post.author_actor_id)
        .copied()
        .unwrap_or(0.0);
    let two_hop = if two_hop_raw.is_finite() {
        two_hop_raw.clamp(0.0, 1.0) * 0.25
    } else {
        0.0
    };
    let author_hist_raw = viewer
        .author_affinities
        .get(&post.author_actor_id)
        .copied()
        .unwrap_or(0.0);
    let author_hist = if author_hist_raw.is_finite() {
        author_hist_raw
    } else {
        0.0
    };
    let comm_hist_raw = norm_slug
        .as_deref()
        .and_then(|s| {
            viewer.community_affinities.get(s).copied().or_else(|| {
                let mut matches: Vec<(&str, f64)> = viewer
                    .community_affinities
                    .iter()
                    .filter(|(k, _)| super::normalize_community_key(k) == s)
                    .map(|(k, &v)| (k.as_str(), v))
                    .collect();
                matches.sort_by(|(ka, _), (kb, _)| ka.cmp(kb));
                matches.first().map(|(_, v)| *v)
            })
        })
        .unwrap_or(0.0);
    let comm_hist = if comm_hist_raw.is_finite() {
        comm_hist_raw
    } else {
        0.0
    };
    let hist_norm = ((author_hist + 0.7 * comm_hist) / 10.0).tanh() * 0.35;
    let affinity_signal = (direct_follow + comm_bonus + two_hop + hist_norm).clamp(0.0, 1.3);

    // 4. Semantic similarity + burst velocity
    let semantic_signal = cosine_similarity_64(&viewer.interest_embedding, &cand.content_embedding)
        .max(0.0)
        .clamp(0.0, 1.0);
    let velocity_raw = reddit_rising(post.upvotes, post.downvotes, post.comments_count, age_hours);
    let velocity_signal = ((1.0 + velocity_raw).ln() / 5.0).clamp(0.0, 1.0);

    // 5. First-class Human + Bot citizen balance & Human+Bot debate bonus
    let defaults = default_prefs_for_preset(prefs.preset);
    let bot_target = if prefs.bot_content_ratio.is_finite() {
        prefs.bot_content_ratio.clamp(0.0, 1.0)
    } else {
        defaults.bot_content_ratio
    };
    let citizen_align = if post.author_kind.is_bot_like() {
        0.5 + 0.8 * bot_target
    } else {
        0.5 + 0.8 * (1.0 - bot_target)
    };
    let debate_bonus = if cand.has_human_participants && cand.has_bot_participants {
        0.12
    } else {
        0.0
    };
    let citizen_signal = (citizen_align + debate_bonus).clamp(0.2, 1.4);

    // Normalize user-tunable weights safely (NaN/Inf guarded)
    let w_rec = if prefs.recency_weight.is_finite() {
        prefs.recency_weight.clamp(0.0, 1.0)
    } else {
        defaults.recency_weight
    };
    let w_aff = if prefs.affinity_weight.is_finite() {
        prefs.affinity_weight.clamp(0.0, 1.0)
    } else {
        defaults.affinity_weight
    };
    let w_qual = if prefs.quality_weight.is_finite() {
        prefs.quality_weight.clamp(0.0, 1.0)
    } else {
        defaults.quality_weight
    };
    let w_sum = (w_rec + w_aff + w_qual).max(0.001);
    let nr = w_rec / w_sum;
    let na = w_aff / w_sum;
    let nq = w_qual / w_sum;

    let c_quality = 0.75 * nq * quality_signal;
    let c_recency = 0.75 * nr * recency_signal;
    let c_affinity = 0.75 * na * affinity_signal;
    let c_semantic = 0.12 * semantic_signal;
    let c_velocity = 0.08 * velocity_signal;
    let c_citizen = 0.10 * citizen_signal;

    // Deterministic tie-breaking micro-jitter <= 0.02 (zero RNG!)
    let seed = fnv1a_64(format!("{}:{}:{}", viewer.actor_id, post.post_id, now_ms / 60_000).as_bytes());
    let jitter = splitmix64_unit(seed) * 0.02;

    let seen_count = viewer.seen_impressions.get(&post.post_id).copied().unwrap_or(0);
    let fatigue = 1.0 / (1.0 + 0.45 * f64::from(seen_count));
    let total = (c_quality + c_recency + c_affinity + c_semantic + c_velocity + c_citizen + jitter) * fatigue;

    let components = [
        ("Quality & Trust", c_quality),
        ("Recency", c_recency),
        ("Social Affinity", c_affinity),
        ("Topic Match", c_semantic),
        ("Rising Velocity", c_velocity),
        ("Citizen Balance", c_citizen),
    ];
    let comp_sum: f64 = components.iter().map(|(_, v)| v.max(0.0)).sum();
    let mut breakdown = Vec::with_capacity(components.len());
    let mut running_pct: f64 = 0.0;
    for (idx, (label, val)) in components.iter().enumerate() {
        let pct = if comp_sum <= 1e-9 {
            if idx == 0 { 100.0 } else { 0.0 }
        } else if idx + 1 == components.len() {
            ((100.0_f64 - running_pct).max(0.0) * 10.0).round() / 10.0
        } else {
            let p = ((val.max(0.0) / comp_sum) * 1000.0).round() / 10.0;
            running_pct += p;
            p
        };
        breakdown.push(SignalContributionDto {
            label: (*label).to_string(),
            contribution_pct: pct,
        });
    }

    let primary_reason = if direct_follow > 0.0 && c_affinity >= c_quality && c_affinity >= c_recency {
        format!("Because you follow @{}", post.author_handle)
    } else if joined_comm && c_affinity >= c_quality {
        format!(
            "Active in c/{} which you joined",
            post.community_slug.as_deref().unwrap_or("community")
        )
    } else if cand.has_human_participants && cand.has_bot_participants && debate_bonus > 0.0 {
        "Active Human + Bot debate".to_string()
    } else if c_recency >= c_quality && c_recency >= c_affinity {
        "Fresh right now".to_string()
    } else if c_semantic >= 0.06 {
        "Matches your interests".to_string()
    } else if let Some(slug) = post.community_slug.as_deref() {
        format!("High quality in c/{slug}")
    } else {
        "High community quality & engagement".to_string()
    };

    let explanation = FeedExplanationDto {
        primary_reason,
        signal_breakdown: breakdown,
    };
    (if total.is_finite() { total } else { 0.0 }, explanation)
}
