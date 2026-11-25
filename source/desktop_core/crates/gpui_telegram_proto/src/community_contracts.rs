use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_feed_limit() -> usize {
    25
}

fn default_thread_total() -> u32 {
    1
}

// =============================================================================
// 1. Canonical Identity & First-Class Human + Bot Actor Types
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    #[default]
    Human,
    Bot,
    HybridAgent,
}

impl ActorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Bot => "bot",
            Self::HybridAgent => "hybrid_agent",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "human" | "humans" => Some(Self::Human),
            "bot" | "bots" => Some(Self::Bot),
            "hybrid_agent" | "hybrid" => Some(Self::HybridAgent),
            _ => None,
        }
    }

    pub const fn is_bot_like(self) -> bool {
        matches!(self, Self::Bot | Self::HybridAgent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifiedTier {
    #[default]
    None,
    HumanVerified,
    OfficialBot,
    VerifiedOrg,
    CommunityMod,
}

impl VerifiedTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::HumanVerified => "human_verified",
            Self::OfficialBot => "official_bot",
            Self::VerifiedOrg => "verified_org",
            Self::CommunityMod => "community_mod",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "human_verified" => Some(Self::HumanVerified),
            "official_bot" => Some(Self::OfficialBot),
            "verified_org" => Some(Self::VerifiedOrg),
            "community_mod" => Some(Self::CommunityMod),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotAutonomyMode {
    #[default]
    Manual,
    MentionOnly,
    Reactive,
    Autonomous,
}

impl BotAutonomyMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::MentionOnly => "mention_only",
            Self::Reactive => "reactive",
            Self::Autonomous => "autonomous",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "manual" => Some(Self::Manual),
            "mention_only" => Some(Self::MentionOnly),
            "reactive" => Some(Self::Reactive),
            "autonomous" => Some(Self::Autonomous),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunityActorDto {
    pub actor_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_user_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<i64>,
    #[serde(default)]
    pub actor_kind: ActorKind,
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_media_id: Option<String>,
    #[serde(default)]
    pub avatar_shape: String,
    #[serde(default)]
    pub avatar_color: String,
    #[serde(default)]
    pub verified_tier: VerifiedTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_model: Option<String>,
    #[serde(default)]
    pub bot_capabilities: Vec<String>,
    #[serde(default)]
    pub bot_autonomy_mode: BotAutonomyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_persona: Option<String>,
    #[serde(default)]
    pub post_karma: i64,
    #[serde(default)]
    pub comment_karma: i64,
    #[serde(default)]
    pub awardee_karma: i64,
    #[serde(default)]
    pub followers_count: u64,
    #[serde(default)]
    pub following_count: u64,
    #[serde(default)]
    pub posts_count: u64,
    #[serde(default)]
    pub trust_score: f64,
    #[serde(default)]
    pub is_following: bool,
    #[serde(default)]
    pub is_muted: bool,
    #[serde(default)]
    pub is_blocked: bool,
    #[serde(default)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunityIdentitiesResponse {
    pub active_actor_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_actor: Option<CommunityActorDto>,
    #[serde(default)]
    pub bot_actors: Vec<CommunityActorDto>,
    #[serde(default)]
    pub all_actors: Vec<CommunityActorDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterHumanActorRequest {
    pub handle: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterBotActorRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_kind: Option<ActorKind>,
    pub handle: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_model: Option<String>,
    #[serde(default)]
    pub bot_capabilities: Vec<String>,
    #[serde(default)]
    pub bot_autonomy_mode: BotAutonomyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_posts_per_hour: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCommunityActorRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_capabilities: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_autonomy_mode: Option<BotAutonomyMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_persona: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorProfileResponse {
    pub actor: CommunityActorDto,
    #[serde(default)]
    pub recent_posts: Vec<CommunityPostDto>,
    #[serde(default)]
    pub recent_comments: Vec<CommunityCommentDto>,
    #[serde(default)]
    pub moderated_communities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowActionResponse {
    pub actor_id: String,
    pub is_following: bool,
    pub followers_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MuteBlockTargetKind {
    #[default]
    Actor,
    Community,
    Keyword,
}

impl MuteBlockTargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Actor => "actor",
            Self::Community => "community",
            Self::Keyword => "keyword",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "actor" => Some(Self::Actor),
            "community" => Some(Self::Community),
            "keyword" => Some(Self::Keyword),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MuteBlockActionKind {
    #[default]
    Mute,
    Block,
}

impl MuteBlockActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mute => "mute",
            Self::Block => "block",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mute" => Some(Self::Mute),
            "block" => Some(Self::Block),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuteBlockRequest {
    pub target_kind: MuteBlockTargetKind,
    pub target_value: String,
    pub action: MuteBlockActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityMuteBlockDto {
    pub id: String,
    pub owner_actor_id: String,
    pub target_kind: MuteBlockTargetKind,
    pub target_value: String,
    pub action: MuteBlockActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuteBlockListResponse {
    #[serde(default)]
    pub items: Vec<CommunityMuteBlockDto>,
}

// =============================================================================
// 2. Content, Poll, Post & Comment DTOs
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostFormat {
    #[default]
    Micro,
    Article,
    Link,
    Media,
    Poll,
    Repost,
    Quote,
    Crosspost,
}

impl PostFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Micro => "micro",
            Self::Article => "article",
            Self::Link => "link",
            Self::Media => "media",
            Self::Poll => "poll",
            Self::Repost => "repost",
            Self::Quote => "quote",
            Self::Crosspost => "crosspost",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "micro" => Some(Self::Micro),
            "article" => Some(Self::Article),
            "link" => Some(Self::Link),
            "media" => Some(Self::Media),
            "poll" => Some(Self::Poll),
            "repost" => Some(Self::Repost),
            "quote" => Some(Self::Quote),
            "crosspost" => Some(Self::Crosspost),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostStatus {
    #[default]
    Published,
    Draft,
    Scheduled,
    Deleted,
}

impl PostStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Published => "published",
            Self::Draft => "draft",
            Self::Scheduled => "scheduled",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "published" => Some(Self::Published),
            "draft" => Some(Self::Draft),
            "scheduled" => Some(Self::Scheduled),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentSort {
    #[default]
    Best,
    Top,
    New,
    Controversial,
    Old,
    Qa,
}

impl CommentSort {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::Top => "top",
            Self::New => "new",
            Self::Controversial => "controversial",
            Self::Old => "old",
            Self::Qa => "qa",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "best" => Some(Self::Best),
            "top" => Some(Self::Top),
            "new" => Some(Self::New),
            "controversial" => Some(Self::Controversial),
            "old" => Some(Self::Old),
            "qa" | "q&a" => Some(Self::Qa),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostMediaAttachmentDto {
    pub media_id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPreviewDto {
    pub url: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityPollOptionDto {
    pub option_id: String,
    pub text: String,
    #[serde(default)]
    pub all_votes: u64,
    #[serde(default)]
    pub human_votes: u64,
    #[serde(default)]
    pub bot_votes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollDto {
    pub poll_id: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<CommunityPollOptionDto>,
    #[serde(default)]
    pub multi_select: bool,
    pub expires_at_ms: i64,
    #[serde(default)]
    pub is_closed: bool,
    #[serde(default)]
    pub total_votes: u64,
    #[serde(default)]
    pub total_human_votes: u64,
    #[serde(default)]
    pub total_bot_votes: u64,
    #[serde(default)]
    pub viewer_voted_option_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostAwardDto {
    pub award_id: String,
    pub award_kind: String,
    #[serde(default)]
    pub icon_emoji: String,
    pub giver_actor_id: String,
    #[serde(default)]
    pub giver_handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostEditRevisionDto {
    pub revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub body: String,
    pub edited_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedPostSummaryDto {
    pub post_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_slug: Option<String>,
    pub author_actor_id: String,
    pub author_handle: String,
    pub author_display_name: String,
    #[serde(default)]
    pub author_kind: ActorKind,
    #[serde(default)]
    pub author_avatar_shape: String,
    #[serde(default)]
    pub author_avatar_color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub body_preview: String,
    #[serde(default)]
    pub format: PostFormat,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub comments_count: u64,
    #[serde(default)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunityPostDto {
    pub post_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_slug: Option<String>,
    pub author_actor_id: String,
    pub author_handle: String,
    pub author_display_name: String,
    #[serde(default)]
    pub author_kind: ActorKind,
    #[serde(default)]
    pub author_verified_tier: VerifiedTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_bot_badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_avatar_media_id: Option<String>,
    #[serde(default)]
    pub author_avatar_shape: String,
    #[serde(default)]
    pub author_avatar_color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub co_author_actor_id: Option<String>,
    #[serde(default)]
    pub format: PostFormat,
    #[serde(default)]
    pub status: PostStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flair: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub media: Vec<PostMediaAttachmentDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_preview: Option<LinkPreviewDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<PollDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted_post_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted_post: Option<Box<EmbeddedPostSummaryDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reposted_post_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reposted_post: Option<Box<EmbeddedPostSummaryDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosspost_of_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosspost_origin: Option<Box<EmbeddedPostSummaryDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_post_id: Option<String>,
    #[serde(default)]
    pub thread_index: u32,
    #[serde(default = "default_thread_total")]
    pub thread_total: u32,
    #[serde(default)]
    pub is_nsfw: bool,
    #[serde(default)]
    pub is_spoiler: bool,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_locked: bool,
    #[serde(default)]
    pub is_removed: bool,
    #[serde(default)]
    pub is_mod_distinguished: bool,
    #[serde(default)]
    pub upvotes: i64,
    #[serde(default)]
    pub downvotes: i64,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub likes_count: u64,
    #[serde(default)]
    pub reposts_count: u64,
    #[serde(default)]
    pub quotes_count: u64,
    #[serde(default)]
    pub comments_count: u64,
    #[serde(default)]
    pub bookmarks_count: u64,
    #[serde(default)]
    pub views_count: u64,
    #[serde(default)]
    pub awards: Vec<PostAwardDto>,
    #[serde(default)]
    pub viewer_vote: i8,
    #[serde(default)]
    pub viewer_liked: bool,
    #[serde(default)]
    pub viewer_reposted: bool,
    #[serde(default)]
    pub viewer_bookmarked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_bookmark_folder: Option<String>,
    #[serde(default)]
    pub edit_history: Vec<PostEditRevisionDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_at_ms: Option<i64>,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunityCommentDto {
    pub comment_id: String,
    pub post_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_comment_id: Option<String>,
    #[serde(default)]
    pub depth: u32,
    #[serde(default)]
    pub path: String,
    pub author_actor_id: String,
    pub author_handle: String,
    pub author_display_name: String,
    #[serde(default)]
    pub author_kind: ActorKind,
    #[serde(default)]
    pub author_verified_tier: VerifiedTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_bot_badge: Option<String>,
    #[serde(default)]
    pub author_avatar_shape: String,
    #[serde(default)]
    pub author_avatar_color: String,
    #[serde(default)]
    pub is_op: bool,
    pub body: String,
    #[serde(default)]
    pub media: Vec<PostMediaAttachmentDto>,
    #[serde(default)]
    pub upvotes: i64,
    #[serde(default)]
    pub downvotes: i64,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub likes_count: u64,
    #[serde(default)]
    pub awards: Vec<PostAwardDto>,
    #[serde(default)]
    pub viewer_vote: i8,
    #[serde(default)]
    pub viewer_liked: bool,
    #[serde(default)]
    pub is_stickied: bool,
    #[serde(default)]
    pub is_mod_distinguished: bool,
    #[serde(default)]
    pub is_removed: bool,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(default)]
    pub children: Vec<CommunityCommentDto>,
    #[serde(default)]
    pub more_children_count: u32,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePollInputDto {
    pub question: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default)]
    pub duration_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateThreadPartDto {
    pub body: String,
    #[serde(default)]
    pub media: Vec<PostMediaAttachmentDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_preview: Option<LinkPreviewDto>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePostRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_slug: Option<String>,
    #[serde(default)]
    pub format: PostFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flair: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub media: Vec<PostMediaAttachmentDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_preview: Option<LinkPreviewDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<CreatePollInputDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted_post_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reposted_post_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosspost_of_id: Option<String>,
    #[serde(default)]
    pub thread_parts: Vec<CreateThreadPartDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub co_author_actor_id: Option<String>,
    #[serde(default)]
    pub is_nsfw: bool,
    #[serde(default)]
    pub is_spoiler: bool,
    #[serde(default)]
    pub status: PostStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatePostResponse {
    pub post: CommunityPostDto,
    #[serde(default)]
    pub thread_posts: Vec<CommunityPostDto>,
    #[serde(default)]
    pub automod_action: AutoModActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automod_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostDetailResponse {
    pub post: CommunityPostDto,
    #[serde(default)]
    pub thread_chain: Vec<CommunityPostDto>,
    #[serde(default)]
    pub quote_chain: Vec<CommunityPostDto>,
    #[serde(default)]
    pub comments: Vec<CommunityCommentDto>,
    #[serde(default)]
    pub total_comments: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePostRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flair: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_nsfw: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_spoiler: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<PostStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteEntityResponse {
    pub id: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteRequest {
    pub direction: i8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteResponse {
    pub target_id: String,
    pub direction: i8,
    pub upvotes: i64,
    pub downvotes: i64,
    pub score: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToggleActionResponse {
    pub target_id: String,
    pub active: bool,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepostRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepostResponse {
    pub post_id: String,
    pub reposted: bool,
    pub reposts_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repost_post_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkResponse {
    pub post_id: String,
    pub bookmarked: bool,
    pub folder: String,
    pub bookmarks_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityPollVoteRequest {
    pub option_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiveAwardRequest {
    pub award_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwardResponse {
    pub target_id: String,
    #[serde(default)]
    pub awards: Vec<PostAwardDto>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummonBotRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_comment_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummonBotResponse {
    pub summoned_bot_actor_id: String,
    pub triggered: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_comment: Option<CommunityCommentDto>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentsQueryParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<CommentSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentsListResponse {
    pub post_id: String,
    #[serde(default)]
    pub sort: CommentSort,
    #[serde(default)]
    pub comments: Vec<CommunityCommentDto>,
    #[serde(default)]
    pub total_count: u64,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCommentRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_comment_id: Option<String>,
    pub body: String,
    #[serde(default)]
    pub media: Vec<PostMediaAttachmentDto>,
    #[serde(default)]
    pub is_mod_distinguished: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateCommentResponse {
    pub comment: CommunityCommentDto,
    #[serde(default)]
    pub automod_action: AutoModActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automod_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCommentRequest {
    pub body: String,
}

// =============================================================================
// 3. Feed, Recommendation Preferences, Explanations & Signals DTOs
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedTab {
    #[default]
    ForYou,
    Following,
    Popular,
    All,
    Community(String),
    List(String),
    Saved,
    ActorPosts(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedSort {
    #[default]
    Algo,
    Hot,
    Best,
    Top,
    New,
    Controversial,
    Rising,
}

impl FeedSort {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Algo => "algo",
            Self::Hot => "hot",
            Self::Best => "best",
            Self::Top => "top",
            Self::New => "new",
            Self::Controversial => "controversial",
            Self::Rising => "rising",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "algo" | "for_you" => Some(Self::Algo),
            "hot" => Some(Self::Hot),
            "best" => Some(Self::Best),
            "top" => Some(Self::Top),
            "new" | "latest" => Some(Self::New),
            "controversial" => Some(Self::Controversial),
            "rising" => Some(Self::Rising),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeWindow {
    Hour,
    Day,
    Week,
    Month,
    Year,
    #[default]
    All,
}

impl TimeWindow {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
            Self::All => "all",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "hour" | "1h" => Some(Self::Hour),
            "day" | "24h" | "today" => Some(Self::Day),
            "week" | "7d" => Some(Self::Week),
            "month" | "30d" => Some(Self::Month),
            "year" | "1y" => Some(Self::Year),
            "all" | "all_time" => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorFilter {
    #[default]
    AllCitizens,
    HumansOnly,
    BotsOnly,
    HumanBotDebates,
}

impl ActorFilter {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllCitizens => "all_citizens",
            Self::HumansOnly => "humans_only",
            Self::BotsOnly => "bots_only",
            Self::HumanBotDebates => "human_bot_debates",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "all" | "all_citizens" => Some(Self::AllCitizens),
            "humans" | "humans_only" | "human" => Some(Self::HumansOnly),
            "bots" | "bots_only" | "bot" => Some(Self::BotsOnly),
            "debates" | "human_bot_debates" => Some(Self::HumanBotDebates),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlgoPreset {
    #[default]
    Balanced,
    RedditDeepDive,
    XRealTime,
    Discovery,
    HumanFocused,
    BotHeavy,
    Chronological,
}

impl AlgoPreset {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::RedditDeepDive => "reddit_deep_dive",
            Self::XRealTime => "x_real_time",
            Self::Discovery => "discovery",
            Self::HumanFocused => "human_focused",
            Self::BotHeavy => "bot_heavy",
            Self::Chronological => "chronological",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "balanced" => Some(Self::Balanced),
            "reddit_deep_dive" => Some(Self::RedditDeepDive),
            "x_real_time" => Some(Self::XRealTime),
            "discovery" => Some(Self::Discovery),
            "human_focused" => Some(Self::HumanFocused),
            "bot_heavy" => Some(Self::BotHeavy),
            "chronological" => Some(Self::Chronological),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlgoPreferencesDto {
    #[serde(default)]
    pub preset: AlgoPreset,
    #[serde(default = "default_recency_weight")]
    pub recency_weight: f64,
    #[serde(default = "default_affinity_weight")]
    pub affinity_weight: f64,
    #[serde(default = "default_quality_weight")]
    pub quality_weight: f64,
    #[serde(default = "default_discovery_ratio")]
    pub discovery_ratio: f64,
    #[serde(default = "default_bot_content_ratio")]
    pub bot_content_ratio: f64,
    #[serde(default = "default_true")]
    pub show_explanations: bool,
    #[serde(default)]
    pub allow_nsfw: bool,
}

fn default_recency_weight() -> f64 {
    0.35
}
fn default_affinity_weight() -> f64 {
    0.35
}
fn default_quality_weight() -> f64 {
    0.30
}
fn default_discovery_ratio() -> f64 {
    0.15
}
fn default_bot_content_ratio() -> f64 {
    0.50
}

impl Default for AlgoPreferencesDto {
    fn default() -> Self {
        Self {
            preset: AlgoPreset::Balanced,
            recency_weight: default_recency_weight(),
            affinity_weight: default_affinity_weight(),
            quality_weight: default_quality_weight(),
            discovery_ratio: default_discovery_ratio(),
            bot_content_ratio: default_bot_content_ratio(),
            show_explanations: true,
            allow_nsfw: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalContributionDto {
    pub label: String,
    pub contribution_pct: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedExplanationDto {
    pub primary_reason: String,
    #[serde(default)]
    pub signal_breakdown: Vec<SignalContributionDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedRequestDto {
    #[serde(default)]
    pub tab: FeedTab,
    #[serde(default)]
    pub sort: FeedSort,
    #[serde(default)]
    pub time_window: TimeWindow,
    #[serde(default)]
    pub actor_filter: ActorFilter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flair: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default = "default_feed_limit")]
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algo_override: Option<AlgoPreferencesDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now_ms: Option<i64>,
}

impl Default for FeedRequestDto {
    fn default() -> Self {
        Self {
            tab: FeedTab::ForYou,
            sort: FeedSort::Algo,
            time_window: TimeWindow::All,
            actor_filter: ActorFilter::AllCitizens,
            flair: None,
            cursor: None,
            limit: default_feed_limit(),
            algo_override: None,
            now_ms: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedQueryParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flair: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

fn normalize_community_slug_param(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    lower
        .strip_prefix("c/")
        .or_else(|| lower.strip_prefix("r/"))
        .unwrap_or(&lower)
        .to_string()
}

impl FeedQueryParams {
    pub fn into_feed_request(&self) -> FeedRequestDto {
        let tab = if let Some(slug) = self.community.as_deref().filter(|s| !s.trim().is_empty()) {
            FeedTab::Community(normalize_community_slug_param(slug))
        } else if let Some(list_id) = self.list_id.as_deref().filter(|s| !s.trim().is_empty()) {
            FeedTab::List(list_id.trim().to_string())
        } else if let Some(actor) = self.actor.as_deref().filter(|s| !s.trim().is_empty()) {
            FeedTab::ActorPosts(actor.trim().trim_start_matches('@').to_string())
        } else {
            let raw_tab = self.tab.as_deref().unwrap_or("for_you").trim();
            let lower_tab = raw_tab.to_ascii_lowercase();
            match lower_tab.as_str() {
                "following" => FeedTab::Following,
                "popular" => FeedTab::Popular,
                "all" | "latest" => FeedTab::All,
                "saved" | "bookmarks" => FeedTab::Saved,
                other if other.starts_with("community:") => {
                    let rest = &raw_tab["community:".len()..];
                    FeedTab::Community(normalize_community_slug_param(rest))
                }
                other if other.starts_with("list:") => {
                    let rest = raw_tab["list:".len()..].trim();
                    FeedTab::List(rest.to_string())
                }
                other if other.starts_with("actor:") => {
                    let rest = raw_tab["actor:".len()..].trim().trim_start_matches('@');
                    FeedTab::ActorPosts(rest.to_string())
                }
                _ => FeedTab::ForYou,
            }
        };

        let sort = self
            .sort
            .as_deref()
            .and_then(FeedSort::parse)
            .unwrap_or_else(|| {
                if self
                    .tab
                    .as_deref()
                    .is_some_and(|t| t.eq_ignore_ascii_case("latest"))
                {
                    FeedSort::New
                } else {
                    FeedSort::Algo
                }
            });

        let time_window = self
            .time
            .as_deref()
            .and_then(TimeWindow::parse)
            .unwrap_or(TimeWindow::All);

        let actor_filter = self
            .actor_filter
            .as_deref()
            .and_then(ActorFilter::parse)
            .unwrap_or(ActorFilter::AllCitizens);

        FeedRequestDto {
            tab,
            sort,
            time_window,
            actor_filter,
            flair: self.flair.clone().filter(|s| !s.trim().is_empty()),
            cursor: self.cursor.clone().filter(|s| !s.trim().is_empty()),
            limit: self.limit.unwrap_or(default_feed_limit()).clamp(1, 100),
            algo_override: None,
            now_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedItemDto {
    pub post: CommunityPostDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<FeedExplanationDto>,
    #[serde(default)]
    pub rank_score: f64,
    #[serde(default)]
    pub is_exploration_slot: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedResponseDto {
    #[serde(default)]
    pub items: Vec<FeedItemDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub total_candidates: usize,
    #[serde(default)]
    pub applied_preset: AlgoPreset,
    #[serde(default)]
    pub generated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngagementSignalKind {
    #[default]
    Impression,
    Click,
    Dwell,
    Upvote,
    Downvote,
    Like,
    Repost,
    Quote,
    Bookmark,
    Reply,
    Share,
    NotInterested,
    ShowMore,
    HidePost,
    Report,
}

impl EngagementSignalKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Impression => "impression",
            Self::Click => "click",
            Self::Dwell => "dwell",
            Self::Upvote => "upvote",
            Self::Downvote => "downvote",
            Self::Like => "like",
            Self::Repost => "repost",
            Self::Quote => "quote",
            Self::Bookmark => "bookmark",
            Self::Reply => "reply",
            Self::Share => "share",
            Self::NotInterested => "not_interested",
            Self::ShowMore => "show_more",
            Self::HidePost => "hide_post",
            Self::Report => "report",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "impression" => Some(Self::Impression),
            "click" => Some(Self::Click),
            "dwell" => Some(Self::Dwell),
            "upvote" => Some(Self::Upvote),
            "downvote" => Some(Self::Downvote),
            "like" => Some(Self::Like),
            "repost" => Some(Self::Repost),
            "quote" => Some(Self::Quote),
            "bookmark" => Some(Self::Bookmark),
            "reply" => Some(Self::Reply),
            "share" => Some(Self::Share),
            "not_interested" => Some(Self::NotInterested),
            "show_more" => Some(Self::ShowMore),
            "hide_post" | "hide" => Some(Self::HidePost),
            "report" => Some(Self::Report),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordSignalRequest {
    pub post_id: String,
    pub kind: EngagementSignalKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dwell_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_slug: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordSignalResponse {
    pub recorded: bool,
    pub signal_id: String,
}

// =============================================================================
// 4. Communities (c/<slug>), Rules, Flairs, Wiki & Moderation DTOs
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityMemberRole {
    Owner,
    Mod,
    BotMod,
    #[default]
    Member,
    Muted,
    Banned,
}

impl CommunityMemberRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Mod => "mod",
            Self::BotMod => "bot_mod",
            Self::Member => "member",
            Self::Muted => "muted",
            Self::Banned => "banned",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "owner" => Some(Self::Owner),
            "mod" => Some(Self::Mod),
            "bot_mod" => Some(Self::BotMod),
            "member" => Some(Self::Member),
            "muted" => Some(Self::Muted),
            "banned" => Some(Self::Banned),
            _ => None,
        }
    }

    pub const fn is_moderator(self) -> bool {
        matches!(self, Self::Owner | Self::Mod | Self::BotMod)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoModActionKind {
    #[default]
    Approve,
    FlagForReview,
    Remove,
    AssignFlair,
    AutoReply,
}

impl AutoModActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::FlagForReview => "flag_for_review",
            Self::Remove => "remove",
            Self::AssignFlair => "assign_flair",
            Self::AutoReply => "auto_reply",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModActionKind {
    #[default]
    Approve,
    Remove,
    Lock,
    Unlock,
    Pin,
    Unpin,
    Distinguish,
    Undistinguish,
    BanActor,
    UnbanActor,
    MuteActor,
    UnmuteActor,
    SetPostFlair,
    SetUserFlair,
    PromoteMod,
    DemoteMod,
    ResolveReport,
    WikiEdit,
}

impl ModActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Remove => "remove",
            Self::Lock => "lock",
            Self::Unlock => "unlock",
            Self::Pin => "pin",
            Self::Unpin => "unpin",
            Self::Distinguish => "distinguish",
            Self::Undistinguish => "undistinguish",
            Self::BanActor => "ban_actor",
            Self::UnbanActor => "unban_actor",
            Self::MuteActor => "mute_actor",
            Self::UnmuteActor => "unmute_actor",
            Self::SetPostFlair => "set_post_flair",
            Self::SetUserFlair => "set_user_flair",
            Self::PromoteMod => "promote_mod",
            Self::DemoteMod => "demote_mod",
            Self::ResolveReport => "resolve_report",
            Self::WikiEdit => "wiki_edit",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "approve" => Some(Self::Approve),
            "remove" => Some(Self::Remove),
            "lock" => Some(Self::Lock),
            "unlock" => Some(Self::Unlock),
            "pin" => Some(Self::Pin),
            "unpin" => Some(Self::Unpin),
            "distinguish" => Some(Self::Distinguish),
            "undistinguish" => Some(Self::Undistinguish),
            "ban_actor" | "ban" => Some(Self::BanActor),
            "unban_actor" | "unban" => Some(Self::UnbanActor),
            "mute_actor" | "mute" => Some(Self::MuteActor),
            "unmute_actor" | "unmute" => Some(Self::UnmuteActor),
            "set_post_flair" | "flair" => Some(Self::SetPostFlair),
            "set_user_flair" => Some(Self::SetUserFlair),
            "promote_mod" => Some(Self::PromoteMod),
            "demote_mod" => Some(Self::DemoteMod),
            "resolve_report" => Some(Self::ResolveReport),
            "wiki_edit" => Some(Self::WikiEdit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityReportReason {
    #[default]
    Spam,
    Harassment,
    Misinformation,
    OffTopic,
    RuleViolation,
    UndisclosedBot,
    NsfwUnmarked,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityRuleDto {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub order: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityFlairDto {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub bg_color: String,
    #[serde(default)]
    pub text_color: String,
    #[serde(default)]
    pub is_mod_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityWikiRevisionDto {
    pub revision: u32,
    pub markdown: String,
    pub editor_actor_id: String,
    #[serde(default)]
    pub editor_handle: String,
    #[serde(default)]
    pub summary: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoModRuleDto {
    pub rule_id: String,
    pub community_id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_author_karma: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_account_age_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_links: Option<usize>,
    #[serde(default)]
    pub block_duplicate_spam: bool,
    #[serde(default = "default_true")]
    pub apply_to_bots: bool,
    #[serde(default = "default_true")]
    pub apply_to_humans: bool,
    #[serde(default)]
    pub action: AutoModActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assign_flair: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_reply_template: Option<String>,
    #[serde(default)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubCommunityDto {
    pub community_id: String,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_media_id: Option<String>,
    #[serde(default)]
    pub icon_emoji: String,
    #[serde(default)]
    pub accent_color: String,
    #[serde(default)]
    pub rules: Vec<CommunityRuleDto>,
    #[serde(default)]
    pub post_flairs: Vec<CommunityFlairDto>,
    #[serde(default)]
    pub user_flairs: Vec<CommunityFlairDto>,
    #[serde(default)]
    pub wiki_markdown: String,
    #[serde(default)]
    pub wiki_revision: u32,
    #[serde(default)]
    pub pinned_post_ids: Vec<String>,
    #[serde(default)]
    pub automod_rules: Vec<AutoModRuleDto>,
    #[serde(default = "default_true")]
    pub allow_bots: bool,
    #[serde(default = "default_true")]
    pub require_bot_disclosure: bool,
    #[serde(default)]
    pub is_nsfw: bool,
    #[serde(default)]
    pub member_count: u64,
    #[serde(default)]
    pub human_member_count: u64,
    #[serde(default)]
    pub bot_member_count: u64,
    #[serde(default)]
    pub posts_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_role: Option<CommunityMemberRole>,
    #[serde(default)]
    pub is_joined: bool,
    #[serde(default)]
    pub notifications_enabled: bool,
    pub created_by_actor_id: String,
    #[serde(default)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCommunitiesResponse {
    #[serde(default)]
    pub communities: Vec<SubCommunityDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCommunityRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
    #[serde(default)]
    pub rules: Vec<CommunityRuleDto>,
    #[serde(default)]
    pub post_flairs: Vec<CommunityFlairDto>,
    #[serde(default)]
    pub user_flairs: Vec<CommunityFlairDto>,
    #[serde(default)]
    pub wiki_markdown: String,
    #[serde(default)]
    pub automod_rules: Vec<AutoModRuleDto>,
    #[serde(default = "default_true")]
    pub allow_bots: bool,
    #[serde(default = "default_true")]
    pub require_bot_disclosure: bool,
    #[serde(default)]
    pub is_nsfw: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCommunityRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<CommunityRuleDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_flairs: Option<Vec<CommunityFlairDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_flairs: Option<Vec<CommunityFlairDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automod_rules: Option<Vec<AutoModRuleDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_bots: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_bot_disclosure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_nsfw: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunityDetailResponse {
    pub community: SubCommunityDto,
    #[serde(default)]
    pub pinned_posts: Vec<CommunityPostDto>,
    #[serde(default)]
    pub moderators: Vec<CommunityActorDto>,
    #[serde(default)]
    pub wiki_revisions: Vec<CommunityWikiRevisionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinCommunityResponse {
    pub community_slug: String,
    pub is_joined: bool,
    pub member_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_role: Option<CommunityMemberRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCommunityWikiRequest {
    pub markdown: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModQueueStatus {
    #[default]
    Pending,
    Approved,
    Removed,
    Dismissed,
}

impl ModQueueStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Removed => "removed",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "removed" => Some(Self::Removed),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModQueueItemDto {
    pub item_id: String,
    pub community_id: String,
    pub target_kind: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporter_actor_id: Option<String>,
    pub source: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_triggered: Option<String>,
    #[serde(default)]
    pub status: ModQueueStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by_actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModQueueResponse {
    #[serde(default)]
    pub items: Vec<ModQueueItemDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModActionRequest {
    pub action: ModActionKind,
    pub target_kind: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flair: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_item_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModLogEntryDto {
    pub log_id: String,
    pub community_id: String,
    pub moderator_actor_id: String,
    #[serde(default)]
    pub moderator_handle: String,
    #[serde(default)]
    pub moderator_kind: ActorKind,
    pub action: ModActionKind,
    pub target_kind: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_triggered: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModLogsResponse {
    #[serde(default)]
    pub logs: Vec<ModLogEntryDto>,
}

// =============================================================================
// 5. Lists, Trending, Search & Notifications DTOs
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityListKind {
    ActorList,
    CustomFeed,
    #[default]
    Mixed,
}

impl CommunityListKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActorList => "actor_list",
            Self::CustomFeed => "custom_feed",
            Self::Mixed => "mixed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "actor_list" => Some(Self::ActorList),
            "custom_feed" => Some(Self::CustomFeed),
            "mixed" => Some(Self::Mixed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityListDto {
    pub list_id: String,
    pub owner_actor_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: CommunityListKind,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub member_actor_ids: Vec<String>,
    #[serde(default)]
    pub community_slugs: Vec<String>,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityListsResponse {
    #[serde(default)]
    pub lists: Vec<CommunityListDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCommunityListRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: CommunityListKind,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub member_actor_ids: Vec<String>,
    #[serde(default)]
    pub community_slugs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCommunityListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<CommunityListKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_actor_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_slugs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendingItemKind {
    #[default]
    Hashtag,
    Topic,
    Community,
    Debate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendingTopicDto {
    pub id: String,
    #[serde(default)]
    pub kind: TrendingItemKind,
    pub label: String,
    pub slug_or_tag: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub posts_count_24h: u64,
    #[serde(default)]
    pub velocity_score: f64,
    #[serde(default)]
    pub human_posts_count: u64,
    #[serde(default)]
    pub bot_posts_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_post_id: Option<String>,
    #[serde(default)]
    pub sparkline: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrendingResponse {
    #[serde(default)]
    pub topics: Vec<TrendingTopicDto>,
    #[serde(default)]
    pub generated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunitySearchTab {
    #[default]
    Top,
    Latest,
    People,
    Bots,
    Communities,
    Media,
    Polls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunitySearchSort {
    #[default]
    Relevance,
    Latest,
    Top,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunitySearchRequestDto {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub tab: CommunitySearchTab,
    #[serde(default)]
    pub sort: CommunitySearchSort,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flair: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_kind: Option<ActorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_media: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_poll: Option<bool>,
    #[serde(default = "default_feed_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

impl Default for CommunitySearchRequestDto {
    fn default() -> Self {
        Self {
            query: String::new(),
            tab: CommunitySearchTab::Top,
            sort: CommunitySearchSort::Relevance,
            community_slug: None,
            author_handle: None,
            flair: None,
            actor_kind: None,
            has_media: None,
            has_poll: None,
            limit: default_feed_limit(),
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunitySearchHit {
    pub post_id: String,
    pub bm25_score: f32,
    pub created_at_ms: i64,
    pub post_score: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommunitySearchResultDto {
    pub query: String,
    #[serde(default)]
    pub tab: CommunitySearchTab,
    #[serde(default)]
    pub posts: Vec<CommunityPostDto>,
    #[serde(default)]
    pub actors: Vec<CommunityActorDto>,
    #[serde(default)]
    pub communities: Vec<SubCommunityDto>,
    #[serde(default)]
    pub total_hits: usize,
    #[serde(default)]
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityNotificationKind {
    #[default]
    Reply,
    Mention,
    Like,
    UpvoteMilestone,
    Quote,
    Repost,
    Follow,
    Award,
    ModAction,
    BotSummon,
    BotReply,
    PollEnded,
    System,
}

impl CommunityNotificationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Mention => "mention",
            Self::Like => "like",
            Self::UpvoteMilestone => "upvote_milestone",
            Self::Quote => "quote",
            Self::Repost => "repost",
            Self::Follow => "follow",
            Self::Award => "award",
            Self::ModAction => "mod_action",
            Self::BotSummon => "bot_summon",
            Self::BotReply => "bot_reply",
            Self::PollEnded => "poll_ended",
            Self::System => "system",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "reply" => Some(Self::Reply),
            "mention" => Some(Self::Mention),
            "like" => Some(Self::Like),
            "upvote_milestone" => Some(Self::UpvoteMilestone),
            "quote" => Some(Self::Quote),
            "repost" => Some(Self::Repost),
            "follow" => Some(Self::Follow),
            "award" => Some(Self::Award),
            "mod_action" => Some(Self::ModAction),
            "bot_summon" => Some(Self::BotSummon),
            "bot_reply" => Some(Self::BotReply),
            "poll_ended" => Some(Self::PollEnded),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityNotificationDto {
    pub notif_id: String,
    pub recipient_actor_id: String,
    pub actor_id: String,
    #[serde(default)]
    pub actor_handle: String,
    #[serde(default)]
    pub actor_display_name: String,
    #[serde(default)]
    pub actor_kind: ActorKind,
    #[serde(default)]
    pub kind: CommunityNotificationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_post_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_comment_id: Option<String>,
    #[serde(default)]
    pub preview_text: String,
    #[serde(default)]
    pub is_read: bool,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationsListResponse {
    #[serde(default)]
    pub notifications: Vec<CommunityNotificationDto>,
    #[serde(default)]
    pub unread_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkNotificationsReadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notif_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkNotificationsReadResponse {
    pub marked_count: u64,
    pub unread_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_enums_roundtrip_and_parse() {
        let kinds = [ActorKind::Human, ActorKind::Bot, ActorKind::HybridAgent];
        for k in kinds {
            assert_eq!(ActorKind::parse(k.as_str()), Some(k));
            let s = serde_json::to_string(&k).unwrap();
            assert_eq!(serde_json::from_str::<ActorKind>(&s).unwrap(), k);
        }

        let presets = [
            AlgoPreset::Balanced,
            AlgoPreset::RedditDeepDive,
            AlgoPreset::XRealTime,
            AlgoPreset::Discovery,
            AlgoPreset::HumanFocused,
            AlgoPreset::BotHeavy,
            AlgoPreset::Chronological,
        ];
        for p in presets {
            assert_eq!(AlgoPreset::parse(p.as_str()), Some(p));
            let s = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<AlgoPreset>(&s).unwrap(), p);
        }

        let notif_kinds = [
            CommunityNotificationKind::Reply,
            CommunityNotificationKind::Mention,
            CommunityNotificationKind::Like,
            CommunityNotificationKind::UpvoteMilestone,
            CommunityNotificationKind::Quote,
            CommunityNotificationKind::Repost,
            CommunityNotificationKind::Follow,
            CommunityNotificationKind::Award,
            CommunityNotificationKind::ModAction,
            CommunityNotificationKind::BotSummon,
            CommunityNotificationKind::BotReply,
            CommunityNotificationKind::PollEnded,
            CommunityNotificationKind::System,
        ];
        for nk in notif_kinds {
            assert_eq!(CommunityNotificationKind::parse(nk.as_str()), Some(nk));
        }

        let sig_kinds = [
            EngagementSignalKind::Impression,
            EngagementSignalKind::Click,
            EngagementSignalKind::Dwell,
            EngagementSignalKind::Upvote,
            EngagementSignalKind::Downvote,
            EngagementSignalKind::Like,
            EngagementSignalKind::Repost,
            EngagementSignalKind::Quote,
            EngagementSignalKind::Bookmark,
            EngagementSignalKind::Reply,
            EngagementSignalKind::Share,
            EngagementSignalKind::NotInterested,
            EngagementSignalKind::ShowMore,
            EngagementSignalKind::HidePost,
            EngagementSignalKind::Report,
        ];
        for sk in sig_kinds {
            assert_eq!(EngagementSignalKind::parse(sk.as_str()), Some(sk));
        }
    }

    #[test]
    fn feed_query_params_converts_tabs_sorts_and_clamps_limit() {
        let q = FeedQueryParams {
            tab: Some("for_you".into()),
            sort: Some("top".into()),
            time: Some("week".into()),
            actor_filter: Some("human_bot_debates".into()),
            community: Some("c/Rust".into()),
            list_id: None,
            actor: None,
            flair: Some("Deep Dive".into()),
            cursor: Some("offset:25".into()),
            limit: Some(999),
        };
        let req = q.into_feed_request();
        assert_eq!(req.tab, FeedTab::Community("rust".into()));
        assert_eq!(req.sort, FeedSort::Top);
        assert_eq!(req.time_window, TimeWindow::Week);
        assert_eq!(req.actor_filter, ActorFilter::HumanBotDebates);
        assert_eq!(req.flair.as_deref(), Some("Deep Dive"));
        assert_eq!(req.limit, 100);

        let latest = FeedQueryParams {
            tab: Some("latest".into()),
            limit: Some(0),
            ..Default::default()
        }
        .into_feed_request();
        assert_eq!(latest.tab, FeedTab::All);
        assert_eq!(latest.sort, FeedSort::New);
        assert_eq!(latest.limit, 1);

        // Regression tests: uppercase C/ prefix, mixed-case list_id preservation, @ stripping on actor: tab
        let upper_comm = FeedQueryParams {
            community: Some("C/Rust".into()),
            ..Default::default()
        }
        .into_feed_request();
        assert_eq!(upper_comm.tab, FeedTab::Community("rust".into()));

        let upper_tab_comm = FeedQueryParams {
            tab: Some("Community:C/Rust".into()),
            ..Default::default()
        }
        .into_feed_request();
        assert_eq!(upper_tab_comm.tab, FeedTab::Community("rust".into()));

        let mixed_list = FeedQueryParams {
            tab: Some("list:list_MixedCase_42".into()),
            ..Default::default()
        }
        .into_feed_request();
        assert_eq!(mixed_list.tab, FeedTab::List("list_MixedCase_42".into()));

        let actor_tab = FeedQueryParams {
            tab: Some("actor:@Alex_Chen".into()),
            ..Default::default()
        }
        .into_feed_request();
        assert_eq!(actor_tab.tab, FeedTab::ActorPosts("Alex_Chen".into()));

        let actor_param = FeedQueryParams {
            actor: Some("@Alex_Chen".into()),
            ..Default::default()
        }
        .into_feed_request();
        assert_eq!(actor_param.tab, FeedTab::ActorPosts("Alex_Chen".into()));
    }

    #[test]
    fn poll_actor_post_and_explanation_json_roundtrip() {
        let poll = PollDto {
            poll_id: "poll_1".into(),
            question: "Best systems language?".into(),
            options: vec![
                CommunityPollOptionDto {
                    option_id: "opt_0".into(),
                    text: "Rust".into(),
                    all_votes: 12,
                    human_votes: 7,
                    bot_votes: 5,
                },
                CommunityPollOptionDto {
                    option_id: "opt_1".into(),
                    text: "Zig".into(),
                    all_votes: 4,
                    human_votes: 3,
                    bot_votes: 1,
                },
            ],
            multi_select: false,
            expires_at_ms: 1_760_086_400_000,
            is_closed: false,
            total_votes: 16,
            total_human_votes: 10,
            total_bot_votes: 6,
            viewer_voted_option_ids: vec!["opt_0".into()],
        };
        let encoded = serde_json::to_string(&poll).unwrap();
        let decoded: PollDto = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, poll);

        let prefs = AlgoPreferencesDto::default();
        let prefs_json = serde_json::to_string(&prefs).unwrap();
        assert_eq!(serde_json::from_str::<AlgoPreferencesDto>(&prefs_json).unwrap(), prefs);

        let expl = FeedExplanationDto {
            primary_reason: "High quality in c/rust".into(),
            signal_breakdown: vec![
                SignalContributionDto { label: "Quality".into(), contribution_pct: 55.0 },
                SignalContributionDto { label: "Affinity".into(), contribution_pct: 45.0 },
            ],
        };
        let expl_json = serde_json::to_string(&expl).unwrap();
        assert_eq!(serde_json::from_str::<FeedExplanationDto>(&expl_json).unwrap(), expl);
    }
}
