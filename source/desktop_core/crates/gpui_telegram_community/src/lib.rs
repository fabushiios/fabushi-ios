//! GPUI-Telegram Fused Reddit + X (Twitter) Community Core Engine.
//!
//! Pure-Rust default build provides identity, models, nested comment trees, 5-stage recommender,
//! real-time trending, AutoMod moderation, deterministic bot runtime, and rich deterministic seeding.
//! Optional features:
//! - `sqlite`: SQLite STRICT WAL V42 schema & `CommunityStore`
//! - `search`: Tantivy BM25 `CommunitySearchIndexer`
//! - `storage`: enables both `sqlite` and `search`

pub mod bot_runtime;
pub mod identity;
pub mod model;
pub mod moderation;
pub mod recommender;
pub mod seed;
pub mod thread;

#[cfg(feature = "sqlite")]
pub mod store;

#[cfg(feature = "search")]
pub mod search;

// Canonical crate-root re-exports matching PROJECT.md § Interface Contracts #2
pub use bot_runtime::{
    BotGuardRejection, BotLoopGuard, BotWakeSelector, DEFAULT_BOT_COOLDOWN_MS,
    DEFAULT_BOT_DUPLICATE_THRESHOLD, DEFAULT_BOT_MAX_LOOP_HOPS, DeterministicBotResponder,
};
pub use identity::{
    ActorGraph, KarmaKind, apply_karma_delta, bot_actor_id, can_user_act_as, compute_trust_score,
    human_actor_id, human_actor_id_from_uid, new_bot_actor, new_human_actor, normalize_handle,
    user_id_from_i64, validate_act_as,
};
pub use model::{
    ExtractedEntities, ParsedSearchQuery, apply_poll_vote, extract_entities,
    normalize_community_slug, parse_search_operators, validate_create_comment_request,
    validate_create_post_request,
};
pub use moderation::{
    AutoModDecision, AutoModEngine, ModPermissionValidator, trigram_jaccard_similarity,
};
pub use recommender::{
    CandidateSource, CommunityRecommender, DiversityConfig, EMBEDDING_DIMS, PostCandidate,
    ScoredCandidate, TrendingEngine, ViewerContext, blend_interest_vector, cosine_similarity_64,
    default_prefs_for_preset, embed_text_64, fnv1a_64, reddit_controversial, reddit_hot,
    reddit_rising, rerank_with_mmr, score_for_you, signal_affinity_delta, splitmix64_unit,
    time_window_cutoff_ms, wilson_lower_bound,
};
pub use seed::{SeedDataset, SeedMembership, SeedPollVote, build_starter_dataset};
pub use thread::{
    CommentTreeBuilder, CommentTreeOptions, ThreadAssembler, comment_depth_from_path,
    make_comment_path,
};

#[cfg(feature = "sqlite")]
pub use store::{
    CommunityStore, MIGRATION_V42_COMMUNITY_SQL, TelegramBotSyncInput, TelegramHumanSyncInput,
};

#[cfg(feature = "search")]
pub use search::{CommunityIndexDoc, CommunitySearchIndexer, ParsedCommunityQuery};
