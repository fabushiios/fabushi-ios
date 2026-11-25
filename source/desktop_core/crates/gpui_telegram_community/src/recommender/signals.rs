//! Signal definitions, deterministic 64-dim text embeddings, affinity deltas, and viewer context.

use gpui_telegram_proto::community_contracts::{
    AlgoPreferencesDto, ActorKind, CommunityPostDto, EngagementSignalKind,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const EMBEDDING_DIMS: usize = 64;

/// Deterministic 64-bit FNV-1a hash (zero external RNG).
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Deterministic SplitMix64 pseudorandom float in `[0.0, 1.0)`.
pub fn splitmix64_unit(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    // Use top 53 bits for uniform IEEE-754 f64 in [0, 1)
    ((z >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

fn project_feature(vec: &mut [f32; EMBEDDING_DIMS], tag: &str, weight: f32) {
    let digest = Sha256::digest(tag.as_bytes());
    let bucket = ((u16::from(digest[0]) | (u16::from(digest[1]) << 8)) as usize) % EMBEDDING_DIMS;
    let sign = if digest[2] & 1 == 0 { 1.0_f32 } else { -1.0_f32 };
    vec[bucket] += sign * weight;
}

/// Deterministic 64-dim L2-normalized signed random projection embedding with token, byte-trigram, and CJK bigram features.
pub fn embed_text_64(text: &str) -> [f32; EMBEDDING_DIMS] {
    let mut vec = [0.0_f32; EMBEDDING_DIMS];
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() {
        return vec;
    }

    // 1. Word tokens + ASCII byte trigrams
    for token in normalized
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
    {
        project_feature(&mut vec, &format!("tok:{token}"), 1.0);
        if token.is_ascii() && token.len() >= 3 {
            let bytes = token.as_bytes();
            for win in bytes.windows(3) {
                if let Ok(tri) = std::str::from_utf8(win) {
                    project_feature(&mut vec, &format!("tri:{tri}"), 0.5);
                }
            }
        }
    }

    // 2. CJK character unigrams and bigrams for Chinese/Japanese/Korean semantic matching
    let cjk_chars: Vec<char> = normalized.chars().filter(|c| is_cjk(*c)).collect();
    for &c in &cjk_chars {
        project_feature(&mut vec, &format!("cjk1:{c}"), 0.6);
    }
    for pair in cjk_chars.windows(2) {
        project_feature(&mut vec, &format!("cjk2:{}{}", pair[0], pair[1]), 1.2);
    }

    let norm_sq: f32 = vec.iter().map(|v| v * v).sum();
    let norm = norm_sq.sqrt();
    if norm > 0.0 && norm.is_finite() {
        for v in &mut vec {
            *v /= norm;
        }
    } else {
        vec.fill(0.0);
    }
    vec
}

/// NaN/Inf/zero-norm safe cosine similarity in `[-1.0, 1.0]`.
pub fn cosine_similarity_64(a: &[f32; EMBEDDING_DIMS], b: &[f32; EMBEDDING_DIMS]) -> f64 {
    let mut dot = 0.0_f64;
    let mut na = 0.0_f64;
    let mut nb = 0.0_f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return 0.0;
        }
        let xf = f64::from(x);
        let yf = f64::from(y);
        dot += xf * yf;
        na += xf * xf;
        nb += yf * yf;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= 0.0 || !denom.is_finite() {
        return 0.0;
    }
    (dot / denom).clamp(-1.0, 1.0)
}

/// Exponential moving average update of a viewer's 64-dim interest vector with L2 re-normalization.
pub fn blend_interest_vector(
    base: &[f32; EMBEDDING_DIMS],
    item: &[f32; EMBEDDING_DIMS],
    alpha: f32,
) -> [f32; EMBEDDING_DIMS] {
    let a = if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut out = [0.0_f32; EMBEDDING_DIMS];
    for i in 0..EMBEDDING_DIMS {
        let bv = if base[i].is_finite() { base[i] } else { 0.0 };
        let iv = if item[i].is_finite() { item[i] } else { 0.0 };
        let blended = (1.0 - a) * bv + a * iv;
        out[i] = if blended.is_finite() { blended } else { 0.0 };
    }
    let norm = out
        .iter()
        .map(|v| f64::from(*v) * f64::from(*v))
        .sum::<f64>()
        .sqrt() as f32;
    if norm > 0.0 && norm.is_finite() {
        for v in &mut out {
            let scaled = *v / norm;
            *v = if scaled.is_finite() { scaled } else { 0.0 };
        }
    } else {
        out.fill(0.0);
    }
    out
}

/// Maps engagement feedback signals to affinity matrix deltas.
pub fn signal_affinity_delta(kind: EngagementSignalKind, dwell_ms: Option<u64>) -> f64 {
    match kind {
        EngagementSignalKind::Impression => 0.0,
        EngagementSignalKind::Click => 0.5,
        EngagementSignalKind::Dwell => match dwell_ms.unwrap_or(0) {
            0..=2_999 => 0.1,
            3_000..=7_999 => 0.4,
            8_000..=29_999 => 0.8,
            _ => 1.2,
        },
        EngagementSignalKind::Upvote => 1.5,
        EngagementSignalKind::Downvote => -1.5,
        EngagementSignalKind::Like => 1.0,
        EngagementSignalKind::Repost => 2.5,
        EngagementSignalKind::Quote => 2.5,
        EngagementSignalKind::Bookmark => 2.0,
        EngagementSignalKind::Reply => 2.5,
        EngagementSignalKind::Share => 2.0,
        EngagementSignalKind::NotInterested => -4.0,
        EngagementSignalKind::ShowMore => 3.0,
        EngagementSignalKind::HidePost => -5.0,
        EngagementSignalKind::Report => -6.0,
    }
}

/// Candidate generation source labels for transparency and exploration tagging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateSource {
    FollowGraph,
    JoinedCommunity,
    SecondHopSocial,
    EmbeddingSimilar,
    TrendingVelocity,
    ColdStartQuality,
}

/// Complete viewer state consumed by `CommunityRecommender::rank_feed`.
#[derive(Debug, Clone)]
pub struct ViewerContext {
    pub actor_id: String,
    pub actor_kind: ActorKind,
    pub following_actor_ids: HashSet<String>,
    pub second_hop_weights: HashMap<String, f64>,
    pub joined_communities: HashSet<String>,
    pub list_member_actors: HashSet<String>,
    pub list_community_slugs: HashSet<String>,
    pub saved_post_ids: HashSet<String>,
    pub author_affinities: HashMap<String, f64>,
    pub community_affinities: HashMap<String, f64>,
    pub interest_embedding: [f32; EMBEDDING_DIMS],
    pub muted_actor_ids: HashSet<String>,
    pub blocked_actor_ids: HashSet<String>,
    pub muted_communities: HashSet<String>,
    pub muted_keywords: Vec<String>,
    pub hidden_post_ids: HashSet<String>,
    pub seen_impressions: HashMap<String, u32>,
    pub algo_prefs: AlgoPreferencesDto,
    pub now_ms: i64,
}

impl Default for ViewerContext {
    fn default() -> Self {
        Self {
            actor_id: "u:0".to_string(),
            actor_kind: ActorKind::Human,
            following_actor_ids: HashSet::new(),
            second_hop_weights: HashMap::new(),
            joined_communities: HashSet::new(),
            list_member_actors: HashSet::new(),
            list_community_slugs: HashSet::new(),
            saved_post_ids: HashSet::new(),
            author_affinities: HashMap::new(),
            community_affinities: HashMap::new(),
            interest_embedding: [0.0; EMBEDDING_DIMS],
            muted_actor_ids: HashSet::new(),
            blocked_actor_ids: HashSet::new(),
            muted_communities: HashSet::new(),
            muted_keywords: Vec::new(),
            hidden_post_ids: HashSet::new(),
            seen_impressions: HashMap::new(),
            algo_prefs: AlgoPreferencesDto::default(),
            now_ms: 0,
        }
    }
}

/// Candidate post decorated with trust, content embedding, velocity, and participant metadata.
#[derive(Debug, Clone)]
pub struct PostCandidate {
    pub post: CommunityPostDto,
    pub author_trust_score: f64,
    pub content_embedding: [f32; EMBEDDING_DIMS],
    pub recent_engagement_1h: u64,
    pub has_human_participants: bool,
    pub has_bot_participants: bool,
    pub sources: Vec<CandidateSource>,
}

impl PostCandidate {
    /// Builds a `PostCandidate` directly from a `CommunityPostDto` with computed 64-dim embedding.
    pub fn from_post(post: CommunityPostDto) -> Self {
        let mut text = String::new();
        if let Some(slug) = post.community_slug.as_deref() {
            text.push_str(slug);
            text.push(' ');
        }
        if let Some(title) = post.title.as_deref() {
            text.push_str(title);
            text.push(' ');
        }
        text.push_str(&post.body);
        for tag in &post.tags {
            text.push(' ');
            text.push_str(tag);
        }
        let content_embedding = embed_text_64(&text);
        let is_bot = post.author_kind.is_bot_like();
        let recent_engagement_1h = (post.score.max(0) as u64)
            .saturating_add(post.comments_count)
            .saturating_add(post.likes_count)
            .saturating_add(post.reposts_count);
        Self {
            post,
            author_trust_score: 0.65,
            content_embedding,
            recent_engagement_1h,
            has_human_participants: !is_bot,
            has_bot_participants: is_bot,
            sources: Vec::new(),
        }
    }
}
