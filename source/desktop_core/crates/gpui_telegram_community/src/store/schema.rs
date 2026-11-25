//! SQLite V42 `STRICT` WAL schema DDL for the fused Reddit + X Community.
//!
//! All 23 tables use strictly `INTEGER | REAL | TEXT | BLOB`, zero `REFERENCES users(...)`,
//! and idempotent `CREATE TABLE IF NOT EXISTS ... STRICT` / `CREATE INDEX IF NOT EXISTS`.

pub const MIGRATION_V42_COMMUNITY_SQL: &str = r#"
-- 1. First-class Human + Bot Actors
CREATE TABLE IF NOT EXISTS community_actors (
    actor_id TEXT PRIMARY KEY,
    linked_user_id INTEGER,
    owner_user_id INTEGER,
    actor_kind TEXT NOT NULL DEFAULT 'human' CHECK (actor_kind IN ('human', 'bot', 'hybrid_agent')),
    handle TEXT NOT NULL UNIQUE COLLATE NOCASE,
    display_name TEXT NOT NULL,
    bio TEXT NOT NULL DEFAULT '',
    location TEXT,
    website TEXT,
    avatar_media_id TEXT,
    banner_media_id TEXT,
    avatar_shape TEXT NOT NULL DEFAULT 'classic',
    avatar_color TEXT NOT NULL DEFAULT 'blue',
    verified_tier TEXT NOT NULL DEFAULT 'none',
    bot_badge TEXT,
    bot_model TEXT,
    bot_capabilities_json TEXT NOT NULL DEFAULT '[]',
    bot_autonomy_mode TEXT NOT NULL DEFAULT 'manual',
    bot_persona TEXT,
    bot_webhook_url TEXT,
    bot_rate_limit_per_hour INTEGER NOT NULL DEFAULT 30,
    pinned_post_id TEXT,
    post_karma INTEGER NOT NULL DEFAULT 0,
    comment_karma INTEGER NOT NULL DEFAULT 0,
    awardee_karma INTEGER NOT NULL DEFAULT 0,
    followers_count INTEGER NOT NULL DEFAULT 0,
    following_count INTEGER NOT NULL DEFAULT 0,
    posts_count INTEGER NOT NULL DEFAULT 0,
    trust_score REAL NOT NULL DEFAULT 0.65,
    is_suspended INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_actors_linked
    ON community_actors(linked_user_id) WHERE linked_user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_community_actors_owner
    ON community_actors(owner_user_id) WHERE owner_user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_community_actors_kind_karma
    ON community_actors(actor_kind, post_karma DESC, followers_count DESC);

-- 2. Social Follow Graph
CREATE TABLE IF NOT EXISTS community_follows (
    follower_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    target_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (follower_actor_id, target_actor_id),
    CHECK (follower_actor_id != target_actor_id)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_follows_target
    ON community_follows(target_actor_id, created_at_ms DESC);

-- 3. Mutes & Blocks (actor / community / keyword)
CREATE TABLE IF NOT EXISTS community_mutes_blocks (
    id TEXT NOT NULL,
    owner_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('actor', 'community', 'keyword')),
    target_value TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('mute', 'block')),
    reason TEXT,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (owner_actor_id, target_kind, target_value, action)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_mutes_blocks_owner
    ON community_mutes_blocks(owner_actor_id, action, target_kind);

-- 4. Subreddits / Communities (c/<slug>)
CREATE TABLE IF NOT EXISTS communities (
    community_id TEXT PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE COLLATE NOCASE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL DEFAULT 'general',
    icon_emoji TEXT NOT NULL DEFAULT '🌐',
    icon_media_id TEXT,
    banner_media_id TEXT,
    accent_color TEXT NOT NULL DEFAULT '#3B82F6',
    creator_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id),
    rules_json TEXT NOT NULL DEFAULT '[]',
    post_flairs_json TEXT NOT NULL DEFAULT '[]',
    user_flairs_json TEXT NOT NULL DEFAULT '[]',
    automod_rules_json TEXT NOT NULL DEFAULT '[]',
    pinned_post_ids_json TEXT NOT NULL DEFAULT '[]',
    wiki_markdown TEXT NOT NULL DEFAULT '',
    wiki_version INTEGER NOT NULL DEFAULT 1,
    allow_bots INTEGER NOT NULL DEFAULT 1,
    require_bot_disclosure INTEGER NOT NULL DEFAULT 1,
    is_nsfw INTEGER NOT NULL DEFAULT 0,
    members_count INTEGER NOT NULL DEFAULT 0,
    posts_count INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_communities_category_members
    ON communities(category, members_count DESC);

-- 5. Community Memberships & Roles
CREATE TABLE IF NOT EXISTS community_memberships (
    community_id TEXT NOT NULL REFERENCES communities(community_id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'mod', 'bot_mod', 'member', 'muted', 'banned')),
    user_flair_text TEXT,
    user_flair_color TEXT,
    notifications_enabled INTEGER NOT NULL DEFAULT 1,
    joined_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (community_id, actor_id)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_memberships_actor
    ON community_memberships(actor_id, role, joined_at_ms DESC);

-- 6. Versioned Community Wiki Revisions
CREATE TABLE IF NOT EXISTS community_wiki_revisions (
    revision_id TEXT PRIMARY KEY,
    community_id TEXT NOT NULL REFERENCES communities(community_id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    editor_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id),
    content_markdown TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL,
    UNIQUE (community_id, version)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_wiki_revisions_comm
    ON community_wiki_revisions(community_id, version DESC);

-- 7. Unified Reddit + X Posts
CREATE TABLE IF NOT EXISTS community_posts (
    post_id TEXT PRIMARY KEY,
    community_id TEXT REFERENCES communities(community_id) ON DELETE SET NULL,
    author_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    co_author_actor_id TEXT,
    post_format TEXT NOT NULL DEFAULT 'micro'
        CHECK (post_format IN ('micro', 'article', 'link', 'media', 'poll', 'repost', 'quote', 'crosspost')),
    title TEXT,
    body_markdown TEXT NOT NULL DEFAULT '',
    link_url TEXT,
    link_preview_json TEXT,
    media_json TEXT NOT NULL DEFAULT '[]',
    flair_text TEXT,
    thread_root_id TEXT,
    parent_post_id TEXT,
    thread_index INTEGER NOT NULL DEFAULT 0,
    thread_total INTEGER NOT NULL DEFAULT 1,
    quote_post_id TEXT,
    repost_of_post_id TEXT,
    crosspost_of_post_id TEXT,
    poll_json TEXT,
    poll_closes_at_ms INTEGER,
    poll_is_closed INTEGER NOT NULL DEFAULT 0,
    hashtags_json TEXT NOT NULL DEFAULT '[]',
    mentions_json TEXT NOT NULL DEFAULT '[]',
    is_spoiler INTEGER NOT NULL DEFAULT 0,
    is_nsfw INTEGER NOT NULL DEFAULT 0,
    is_pinned INTEGER NOT NULL DEFAULT 0,
    is_locked INTEGER NOT NULL DEFAULT 0,
    is_distinguished_mod INTEGER NOT NULL DEFAULT 0,
    is_removed INTEGER NOT NULL DEFAULT 0,
    removed_reason TEXT,
    scheduled_at_ms INTEGER,
    status TEXT NOT NULL DEFAULT 'published'
        CHECK (status IN ('published', 'draft', 'scheduled', 'deleted')),
    upvotes INTEGER NOT NULL DEFAULT 1,
    downvotes INTEGER NOT NULL DEFAULT 0,
    score INTEGER NOT NULL DEFAULT 1,
    likes_count INTEGER NOT NULL DEFAULT 0,
    comments_count INTEGER NOT NULL DEFAULT 0,
    reposts_count INTEGER NOT NULL DEFAULT 0,
    quotes_count INTEGER NOT NULL DEFAULT 0,
    bookmarks_count INTEGER NOT NULL DEFAULT 0,
    views_count INTEGER NOT NULL DEFAULT 0,
    awards_count INTEGER NOT NULL DEFAULT 0,
    hot_score REAL NOT NULL DEFAULT 0.0,
    wilson_score REAL NOT NULL DEFAULT 0.0,
    bot_hop_count INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    edited_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_posts_feed_new
    ON community_posts(status, is_removed, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_community_posts_feed_hot
    ON community_posts(status, is_removed, hot_score DESC, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_community_posts_community_hot
    ON community_posts(community_id, status, is_removed, is_pinned DESC, hot_score DESC);
CREATE INDEX IF NOT EXISTS idx_community_posts_author_time
    ON community_posts(author_actor_id, status, is_removed, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_community_posts_thread
    ON community_posts(thread_root_id, thread_index ASC) WHERE thread_root_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_community_posts_scheduled
    ON community_posts(status, scheduled_at_ms ASC) WHERE status = 'scheduled';
CREATE INDEX IF NOT EXISTS idx_community_posts_poll_expiry
    ON community_posts(poll_is_closed, poll_closes_at_ms ASC) WHERE poll_closes_at_ms IS NOT NULL;

-- 8. Post Edit History
CREATE TABLE IF NOT EXISTS community_post_edits (
    edit_id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL REFERENCES community_posts(post_id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    editor_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id),
    previous_title TEXT,
    previous_body_markdown TEXT NOT NULL,
    edit_reason TEXT NOT NULL DEFAULT '',
    edited_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_post_edits_post
    ON community_post_edits(post_id, revision DESC, edited_at_ms DESC);

-- 9. Arbitrary-Depth Nested Comments
CREATE TABLE IF NOT EXISTS community_comments (
    comment_id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL REFERENCES community_posts(post_id) ON DELETE CASCADE,
    parent_comment_id TEXT REFERENCES community_comments(comment_id) ON DELETE CASCADE,
    root_comment_id TEXT,
    depth INTEGER NOT NULL DEFAULT 0,
    path TEXT NOT NULL,
    author_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    body_markdown TEXT NOT NULL,
    media_json TEXT NOT NULL DEFAULT '[]',
    mentions_json TEXT NOT NULL DEFAULT '[]',
    upvotes INTEGER NOT NULL DEFAULT 1,
    downvotes INTEGER NOT NULL DEFAULT 0,
    score INTEGER NOT NULL DEFAULT 1,
    likes_count INTEGER NOT NULL DEFAULT 0,
    replies_count INTEGER NOT NULL DEFAULT 0,
    awards_count INTEGER NOT NULL DEFAULT 0,
    wilson_score REAL NOT NULL DEFAULT 0.0,
    bot_hop_count INTEGER NOT NULL DEFAULT 0,
    is_stickied INTEGER NOT NULL DEFAULT 0,
    is_distinguished_mod INTEGER NOT NULL DEFAULT 0,
    is_collapsed_default INTEGER NOT NULL DEFAULT 0,
    is_removed INTEGER NOT NULL DEFAULT 0,
    is_deleted INTEGER NOT NULL DEFAULT 0,
    removed_reason TEXT,
    created_at_ms INTEGER NOT NULL,
    edited_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_comments_post_parent
    ON community_comments(post_id, parent_comment_id, is_stickied DESC, wilson_score DESC);
CREATE INDEX IF NOT EXISTS idx_community_comments_post_path
    ON community_comments(post_id, path ASC);
CREATE INDEX IF NOT EXISTS idx_community_comments_author
    ON community_comments(author_actor_id, created_at_ms DESC);

-- 10. Reddit Votes (Up/Down)
CREATE TABLE IF NOT EXISTS community_votes (
    target_kind TEXT NOT NULL CHECK (target_kind IN ('post', 'comment')),
    target_id TEXT NOT NULL,
    actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    direction INTEGER NOT NULL CHECK (direction IN (-1, 1)),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (target_kind, target_id, actor_id)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_votes_actor_time
    ON community_votes(actor_id, target_kind, updated_at_ms DESC);

-- 11. X Likes / Hearts
CREATE TABLE IF NOT EXISTS community_likes (
    target_kind TEXT NOT NULL CHECK (target_kind IN ('post', 'comment')),
    target_id TEXT NOT NULL,
    actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (target_kind, target_id, actor_id)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_likes_actor_time
    ON community_likes(actor_id, target_kind, created_at_ms DESC);

-- 12. Idempotent Toggleable Reposts
CREATE TABLE IF NOT EXISTS community_reposts (
    actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    original_post_id TEXT NOT NULL REFERENCES community_posts(post_id) ON DELETE CASCADE,
    repost_post_id TEXT,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (actor_id, original_post_id)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_reposts_orig
    ON community_reposts(original_post_id, created_at_ms DESC);

-- 13. Bookmarks with Folders
CREATE TABLE IF NOT EXISTS community_bookmarks (
    actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    post_id TEXT NOT NULL REFERENCES community_posts(post_id) ON DELETE CASCADE,
    folder_name TEXT NOT NULL DEFAULT 'All',
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (actor_id, post_id)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_bookmarks_actor_folder
    ON community_bookmarks(actor_id, folder_name, created_at_ms DESC);

-- 14. Poll Votes (option_id TEXT + denormalized actor_kind for O(1) Human vs Bot split)
CREATE TABLE IF NOT EXISTS community_poll_votes (
    post_id TEXT NOT NULL REFERENCES community_posts(post_id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    option_id TEXT NOT NULL,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('human', 'bot', 'hybrid_agent')),
    voted_at_ms INTEGER NOT NULL,
    PRIMARY KEY (post_id, actor_id, option_id)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_poll_votes_post_opt
    ON community_poll_votes(post_id, option_id, actor_kind);

-- 15. Awards
CREATE TABLE IF NOT EXISTS community_awards (
    award_id TEXT PRIMARY KEY,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('post', 'comment')),
    target_id TEXT NOT NULL,
    giver_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    recipient_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    award_kind TEXT NOT NULL,
    icon_emoji TEXT NOT NULL DEFAULT '🏆',
    cost_stars INTEGER NOT NULL DEFAULT 0,
    message TEXT NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_awards_target
    ON community_awards(target_kind, target_id, created_at_ms DESC);

-- 16. Custom Lists & Reddit Custom Feeds
CREATE TABLE IF NOT EXISTS community_lists (
    list_id TEXT PRIMARY KEY,
    owner_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT 'mixed' CHECK (kind IN ('actor_list', 'custom_feed', 'mixed')),
    is_private INTEGER NOT NULL DEFAULT 0,
    is_pinned INTEGER NOT NULL DEFAULT 0,
    members_json TEXT NOT NULL DEFAULT '[]',
    communities_json TEXT NOT NULL DEFAULT '[]',
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_lists_owner
    ON community_lists(owner_actor_id, is_pinned DESC, updated_at_ms DESC);

-- 17. Categorized Notifications (matches ALL CommunityNotificationKind variants)
CREATE TABLE IF NOT EXISTS community_notifications (
    notif_id TEXT PRIMARY KEY,
    recipient_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN (
        'reply', 'mention', 'like', 'upvote_milestone', 'quote', 'repost',
        'follow', 'award', 'mod_action', 'bot_summon', 'bot_reply', 'poll_ended', 'system'
    )),
    target_post_id TEXT,
    target_comment_id TEXT,
    preview_text TEXT NOT NULL DEFAULT '',
    is_read INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_notifications_recip_unread
    ON community_notifications(recipient_actor_id, is_read ASC, created_at_ms DESC);

-- 18. Moderation Queue (AutoMod flags + User reports)
CREATE TABLE IF NOT EXISTS community_mod_queue (
    item_id TEXT PRIMARY KEY,
    community_id TEXT NOT NULL REFERENCES communities(community_id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('post', 'comment')),
    target_id TEXT NOT NULL,
    reporter_actor_id TEXT REFERENCES community_actors(actor_id) ON DELETE SET NULL,
    source TEXT NOT NULL DEFAULT 'report' CHECK (source IN ('report', 'automod')),
    reason TEXT NOT NULL DEFAULT '',
    rule_triggered TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'removed', 'dismissed')),
    resolved_by_actor_id TEXT REFERENCES community_actors(actor_id) ON DELETE SET NULL,
    resolved_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_mod_queue_comm_status
    ON community_mod_queue(community_id, status, created_at_ms DESC);

-- 19. Transparent Mod Audit Logs
CREATE TABLE IF NOT EXISTS community_mod_logs (
    log_id TEXT PRIMARY KEY,
    community_id TEXT NOT NULL REFERENCES communities(community_id) ON DELETE CASCADE,
    mod_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id),
    action TEXT NOT NULL,
    target_kind TEXT NOT NULL DEFAULT 'post',
    target_id TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    rule_triggered TEXT,
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_mod_logs_comm_time
    ON community_mod_logs(community_id, created_at_ms DESC);

-- 20. Recommendation Feedback & Impression Signals (matches ALL EngagementSignalKind variants)
CREATE TABLE IF NOT EXISTS community_user_signals (
    signal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    signal_kind TEXT NOT NULL CHECK (signal_kind IN (
        'impression', 'click', 'dwell', 'upvote', 'downvote', 'like', 'repost',
        'quote', 'bookmark', 'reply', 'share', 'not_interested', 'show_more', 'hide_post', 'report'
    )),
    post_id TEXT,
    community_id TEXT,
    target_actor_id TEXT,
    value REAL NOT NULL DEFAULT 1.0,
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_user_signals_actor_time
    ON community_user_signals(actor_id, signal_kind, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_community_user_signals_post
    ON community_user_signals(post_id, signal_kind) WHERE post_id IS NOT NULL;

-- 21. Viewer-to-(Author|Community|Tag) Rolling Affinity Matrix
CREATE TABLE IF NOT EXISTS community_actor_affinities (
    viewer_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('author', 'community', 'tag')),
    target_key TEXT NOT NULL,
    score REAL NOT NULL DEFAULT 0.0,
    interaction_count INTEGER NOT NULL DEFAULT 0,
    last_interacted_ms INTEGER NOT NULL,
    PRIMARY KEY (viewer_actor_id, target_kind, target_key)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_community_affinities_viewer_score
    ON community_actor_affinities(viewer_actor_id, target_kind, score DESC);

-- 22. User-Tunable Algorithm Preferences
CREATE TABLE IF NOT EXISTS community_algo_prefs (
    actor_id TEXT PRIMARY KEY REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    preset TEXT NOT NULL DEFAULT 'balanced',
    recency_weight REAL NOT NULL DEFAULT 0.35,
    affinity_weight REAL NOT NULL DEFAULT 0.35,
    quality_weight REAL NOT NULL DEFAULT 0.30,
    discovery_ratio REAL NOT NULL DEFAULT 0.15,
    bot_content_ratio REAL NOT NULL DEFAULT 0.50,
    show_explanations INTEGER NOT NULL DEFAULT 1,
    allow_nsfw INTEGER NOT NULL DEFAULT 0,
    updated_at_ms INTEGER NOT NULL
) STRICT;

-- 23. Bot Cooldowns & Hourly Rate Limits
CREATE TABLE IF NOT EXISTS community_bot_cooldowns (
    bot_actor_id TEXT NOT NULL REFERENCES community_actors(actor_id) ON DELETE CASCADE,
    scope_key TEXT NOT NULL,
    last_replied_at_ms INTEGER NOT NULL,
    hourly_window_start_ms INTEGER NOT NULL,
    hourly_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (bot_actor_id, scope_key)
) STRICT;
"#;
