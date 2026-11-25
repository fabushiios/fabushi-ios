//! Real-time burst-velocity TrendingEngine with 6-bin 4-hour sparklines and Human/Bot breakdowns.

use crate::model::extract_entities;
use gpui_telegram_proto::community_contracts::{
    CommunityPostDto, PostStatus, TrendingItemKind, TrendingTopicDto,
};
use std::collections::{HashMap, HashSet};

const WINDOW_24H_MS: i64 = 86_400_000;
const BIN_COUNT: usize = 6;
const BIN_SPAN_MS: i64 = WINDOW_24H_MS / (BIN_COUNT as i64); // 4 hours per bin

struct TopicAccumulator {
    kind: TrendingItemKind,
    label: String,
    slug_or_tag: String,
    category: String,
    bins: [u32; BIN_COUNT],
    authors: HashSet<String>,
    human_posts: u64,
    bot_posts: u64,
    total_engagement: u64,
    sample_post_id: Option<String>,
    best_post_score: i64,
}

impl TopicAccumulator {
    fn new(kind: TrendingItemKind, label: String, slug_or_tag: String, category: String) -> Self {
        Self {
            kind,
            label,
            slug_or_tag,
            category,
            bins: [0; BIN_COUNT],
            authors: HashSet::new(),
            human_posts: 0,
            bot_posts: 0,
            total_engagement: 0,
            sample_post_id: None,
            best_post_score: i64::MIN,
        }
    }

    fn record(&mut self, post: &CommunityPostDto, bin_idx: usize) {
        self.bins[bin_idx] = self.bins[bin_idx].saturating_add(1);
        self.authors.insert(post.author_actor_id.clone());
        if post.author_kind.is_bot_like() {
            self.bot_posts = self.bot_posts.saturating_add(1);
        } else {
            self.human_posts = self.human_posts.saturating_add(1);
        }
        let eng = (post.score.max(0) as u64)
            .saturating_add(post.comments_count.saturating_mul(2))
            .saturating_add(post.likes_count)
            .saturating_add(post.reposts_count.saturating_mul(2))
            .saturating_add(post.quotes_count.saturating_mul(2));
        self.total_engagement = self.total_engagement.saturating_add(eng);
        if post.score > self.best_post_score || self.sample_post_id.is_none() {
            self.best_post_score = post.score;
            self.sample_post_id = Some(post.post_id.clone());
        }
    }

    fn into_dto(self) -> TrendingTopicDto {
        let posts_24h: u64 = self.bins.iter().map(|&c| u64::from(c)).sum();
        // bins[0] = oldest (20-24h ago), bins[5] = most recent (0-4h ago)
        let recent_4h = f64::from(self.bins[5]);
        let mid_4_8h = f64::from(self.bins[4]);
        let older: f64 = self.bins[0..4].iter().map(|&c| f64::from(c)).sum();
        let burst = 3.0 * recent_4h + 1.5 * mid_4_8h + 0.5 * older;
        let author_factor = 1.0 + (1.0 + self.authors.len() as f64).ln();
        let eng_factor = 1.0 + (1.0 + self.total_engagement as f64).ln() / 5.0;
        let velocity = (burst * author_factor * eng_factor * 100.0).round() / 100.0;

        let id_prefix = match self.kind {
            TrendingItemKind::Hashtag => "tag",
            TrendingItemKind::Community => "comm",
            TrendingItemKind::Debate => "debate",
            TrendingItemKind::Topic => "topic",
        };
        TrendingTopicDto {
            id: format!("trend_{id_prefix}_{}", self.slug_or_tag),
            kind: self.kind,
            label: self.label,
            slug_or_tag: self.slug_or_tag,
            category: self.category,
            posts_count_24h: posts_24h,
            velocity_score: if velocity.is_finite() { velocity } else { 0.0 },
            human_posts_count: self.human_posts,
            bot_posts_count: self.bot_posts,
            sample_post_id: self.sample_post_id,
            sparkline: self.bins.to_vec(),
        }
    }
}

/// Computes burst-velocity trending hashtags, communities, and active debates.
pub struct TrendingEngine;

impl TrendingEngine {
    pub fn compute(posts: &[CommunityPostDto], now_ms: i64, limit: usize) -> Vec<TrendingTopicDto> {
        if posts.is_empty() || limit == 0 {
            return Vec::new();
        }
        let cutoff_ms = now_ms.saturating_sub(WINDOW_24H_MS);
        let mut topics: HashMap<String, TopicAccumulator> = HashMap::new();

        for post in posts {
            if post.status != PostStatus::Published || post.is_removed {
                continue;
            }
            if post.created_at_ms < cutoff_ms || post.created_at_ms > now_ms.saturating_add(60_000) {
                continue;
            }
            let age_ms = now_ms
                .saturating_sub(post.created_at_ms)
                .clamp(0, WINDOW_24H_MS - 1);
            // bin 5 = newest [0..4h), bin 0 = oldest [20..24h)
            let reverse_bin = (age_ms / BIN_SPAN_MS) as usize;
            let bin_idx = (BIN_COUNT - 1).saturating_sub(reverse_bin.min(BIN_COUNT - 1));

            // 1. Hashtags (explicit post.tags + extracted from title/body)
            let mut combined_tags: HashSet<String> = post
                .tags
                .iter()
                .map(|t| t.trim().trim_start_matches('#').to_lowercase())
                .filter(|t| !t.is_empty())
                .collect();
            let extracted = extract_entities(&format!(
                "{} {}",
                post.title.as_deref().unwrap_or(""),
                post.body
            ));
            for tag in extracted.hashtags {
                combined_tags.insert(tag);
            }
            let norm_comm = post
                .community_slug
                .as_deref()
                .map(super::normalize_community_key)
                .filter(|s| !s.is_empty());

            for tag in combined_tags {
                let key = format!("tag:{tag}");
                topics
                    .entry(key)
                    .or_insert_with(|| {
                        TopicAccumulator::new(
                            TrendingItemKind::Hashtag,
                            format!("#{tag}"),
                            tag.clone(),
                            norm_comm
                                .clone()
                                .unwrap_or_else(|| "general".to_string()),
                        )
                    })
                    .record(post, bin_idx);
            }

            // 2. Active communities (c/<slug>)
            if let Some(norm) = norm_comm.as_deref() {
                let key = format!("comm:{norm}");
                topics
                    .entry(key)
                    .or_insert_with(|| {
                        TopicAccumulator::new(
                            TrendingItemKind::Community,
                            format!("c/{norm}"),
                            norm.to_string(),
                            "community".to_string(),
                        )
                    })
                    .record(post, bin_idx);
            }

            // 3. High-discussion debate posts
            if post.comments_count >= 4 {
                let label = post
                    .title
                    .as_deref()
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| t.trim().to_string())
                    .unwrap_or_else(|| post.body.chars().take(64).collect());
                let key = format!("debate:{}", post.post_id);
                topics
                    .entry(key)
                    .or_insert_with(|| {
                        TopicAccumulator::new(
                            TrendingItemKind::Debate,
                            label,
                            post.post_id.clone(),
                            norm_comm
                                .clone()
                                .unwrap_or_else(|| "debate".to_string()),
                        )
                    })
                    .record(post, bin_idx);
            }
        }

        let mut results: Vec<TrendingTopicDto> =
            topics.into_values().map(TopicAccumulator::into_dto).collect();
        results.sort_by(|a, b| {
            b.velocity_score
                .total_cmp(&a.velocity_score)
                .then_with(|| b.posts_count_24h.cmp(&a.posts_count_24h))
                .then_with(|| a.slug_or_tag.cmp(&b.slug_or_tag))
                .then_with(|| a.id.cmp(&b.id))
        });
        results.truncate(limit);
        results
    }
}
