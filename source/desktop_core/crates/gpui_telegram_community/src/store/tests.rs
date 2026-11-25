//! Comprehensive SQLite V42 `STRICT` WAL store integration tests.

use super::*;
use gpui_telegram_proto::community_contracts::{
    ActorFilter, AlgoPreferencesDto, AlgoPreset, AutoModActionKind, AutoModRuleDto, CommentSort,
    CommunityListKind, CommunityMemberRole, CreateCommentRequest, CreateCommunityListRequest,
    CreateCommunityRequest, CreatePollInputDto, CreatePostRequest, CreateThreadPartDto,
    EngagementSignalKind, FeedRequestDto, FeedSort, FeedTab, GiveAwardRequest, ModActionKind,
    ModActionRequest, ModQueueStatus, MuteBlockActionKind, MuteBlockRequest, MuteBlockTargetKind,
    PostFormat, PostMediaAttachmentDto, PostStatus, RecordSignalRequest,
    UpdateCommunityActorRequest, UpdatePostRequest,
};

#[test]
fn v42_strict_tables_pragmas_idempotency_and_tempdir_cleanup() {
    let store = CommunityStore::open_in_memory().unwrap();
    let db_path = store.db_path().unwrap().to_path_buf();
    let parent_dir = db_path.parent().unwrap().to_path_buf();
    assert!(db_path.exists());

    // Idempotent second migration pass
    store.with_conn(|c| CommunityStore::apply_migrations(c)).unwrap();

    store
        .with_conn(|conn| {
            let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
            assert_eq!(fk, 1, "foreign_keys pragma must be ON");
            let jm: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
            assert_eq!(jm.to_ascii_lowercase(), "wal");

            // Verify all 23 community tables exist AND are STRICT (`strict = 1`)
            let mut stmt = conn
                .prepare("SELECT name, strict FROM pragma_table_list WHERE type = 'table' AND (name = 'communities' OR name LIKE 'community_%')")
                .unwrap();
            let tables: Vec<(String, i64)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            assert_eq!(tables.len(), 23, "expected exactly 23 V42 STRICT tables, got {:?}", tables);
            for (name, strict) in &tables {
                assert_eq!(*strict, 1, "table {name} must be STRICT");
            }
            Ok(())
        })
        .unwrap();

    // Dropping store must close Connection first and clean up TempDir cleanly!
    drop(store);
    assert!(!parent_dir.exists(), "ephemeral TempDir should be deleted on drop");
}

#[test]
fn seed_if_empty_sync_identities_and_feed_execution() {
    let now = 1_760_100_000_000_i64;
    let store = CommunityStore::open_in_memory().unwrap();

    assert!(store.seed_if_empty(now).unwrap());
    assert!(!store.seed_if_empty(now).unwrap(), "second seed must be idempotent no-op");

    let comms = store.list_communities(Some("u:900001"), None, 50).unwrap();
    assert_eq!(comms.len(), 8);
    assert!(comms.iter().all(|c| c.human_member_count > 0 && c.bot_member_count > 0));

    // Sync Telegram identities (including handle collision resolution against seeded @alex_chen)
    let synced = store
        .sync_telegram_identities(
            &TelegramHumanSyncInput {
                user_id: 555,
                username: Some("alex_chen".into()), // collides with seeded u:900001!
                display_name: "Alex Real".into(),
                bio: Some("Real account".into()),
                avatar_media_id: Some("media_avatar_555".into()),
            },
            &[TelegramBotSyncInput {
                bot_user_id: 888,
                username: "my_rust_bot".into(),
                display_name: "My Rust Bot".into(),
                bio: Some("Helper".into()),
                avatar_media_id: None,
                avatar_shape: Some("hex".into()),
                avatar_color: Some("cyan".into()),
                bot_model: Some("hermes-4".into()),
                bot_capabilities: vec!["rust".into()],
                bot_persona: Some("Helpful Rust bot".into()),
            }],
            now,
        )
        .unwrap();
    assert_eq!(synced.len(), 2);
    assert_eq!(synced[0].actor_id, "u:555");
    assert_eq!(synced[0].handle, "alex_chen_555");
    assert_eq!(synced[1].actor_id, "b:888");
    assert_eq!(synced[1].owner_user_id, Some(555));
    assert!(store.is_media_referenced_publicly("media_avatar_555").unwrap());
    assert!(!store.is_media_referenced_publicly("media_unreferenced").unwrap());

    // Execute get_feed across multiple tabs & filters
    let for_you = store
        .get_feed("u:900001", &FeedRequestDto { tab: FeedTab::ForYou, sort: FeedSort::Algo, limit: 15, ..Default::default() }, now)
        .unwrap();
    assert!(!for_you.items.is_empty());
    assert!(for_you.items[0].explanation.is_some());

    let debates = store
        .get_feed(
            "u:900001",
            &FeedRequestDto { tab: FeedTab::All, actor_filter: ActorFilter::HumanBotDebates, limit: 20, ..Default::default() },
            now,
        )
        .unwrap();
    assert!(!debates.items.is_empty(), "seeded dataset must have Human+Bot debates");

    let bots_only = store
        .get_feed(
            "u:900001",
            &FeedRequestDto { tab: FeedTab::All, actor_filter: ActorFilter::BotsOnly, limit: 20, ..Default::default() },
            now,
        )
        .unwrap();
    assert!(bots_only.items.iter().all(|i| i.post.author_kind.is_bot_like()));

    let rust_tab = store
        .get_feed("u:900001", &FeedRequestDto { tab: FeedTab::Community("rust".into()), sort: FeedSort::Hot, limit: 10, ..Default::default() }, now)
        .unwrap();
    assert!(rust_tab.items.iter().all(|i| i.post.community_slug.as_deref() == Some("rust")));

    let trends = store.get_trending(now, 10).unwrap();
    assert!(!trends.is_empty());
}

#[test]
fn atomic_votes_self_karma_exclusion_repost_poll_wiki_mod_and_schedulers() {
    let now = 1_760_100_000_000_i64;
    let store = CommunityStore::open_in_memory().unwrap();
    store.seed_if_empty(now).unwrap();

    // 1. Create a thread + poll + media post + scheduled post
    let created = store
        .create_post(
            "u:900001",
            &CreatePostRequest {
                community_slug: Some("rust".into()),
                format: PostFormat::Poll,
                title: Some("Best SQLite mode?".into()),
                body: "Testing poll + media + thread in c/rust #sqlite".into(),
                media: vec![PostMediaAttachmentDto {
                    media_id: "pub_media_1".into(),
                    kind: "image".into(),
                    url: None,
                    thumbnail_url: None,
                    alt_text: None,
                    width: None,
                    height: None,
                    duration_secs: None,
                }],
                poll: Some(CreatePollInputDto {
                    question: "WAL or rollback?".into(),
                    options: vec!["WAL".into(), "Rollback".into()],
                    multi_select: false,
                    duration_secs: 600,
                }),
                thread_parts: vec![CreateThreadPartDto {
                    body: "2/2 Follow-up note on checkpoints.".into(),
                    media: vec![],
                    link_preview: None,
                }],
                ..Default::default()
            },
            now,
        )
        .unwrap();
    let pid = created.post.post_id.clone();
    assert_eq!(created.thread_posts.len(), 1);
    assert!(store.is_media_referenced_publicly("pub_media_1").unwrap());

    // 2. Vote transitions + self-vote karma exclusion
    let karma_before = store.get_actor("u:900001", None).unwrap().unwrap().post_karma;
    let v_self = store.vote_post("u:900001", &pid, -1, now + 10).unwrap();
    assert_eq!(v_self.direction, -1);
    assert_eq!(store.get_actor("u:900001", None).unwrap().unwrap().post_karma, karma_before, "self-vote must not change karma");

    let v1 = store.vote_post("b:910001", &pid, 1, now + 20).unwrap();
    assert_eq!(v1.direction, 1);
    assert_eq!(store.get_actor("u:900001", None).unwrap().unwrap().post_karma, karma_before + 1);
    let _v2 = store.vote_post("b:910001", &pid, -1, now + 30).unwrap();
    assert_eq!(store.get_actor("u:900001", None).unwrap().unwrap().post_karma, karma_before - 1);
    let _v3 = store.vote_post("b:910001", &pid, 0, now + 40).unwrap();
    assert_eq!(store.get_actor("u:900001", None).unwrap().unwrap().post_karma, karma_before);

    // 3. Idempotent repost toggle + bookmark + award + edit history
    let rep1 = store.toggle_repost("u:900002", &pid, now + 50).unwrap();
    assert!(rep1.reposted && rep1.reposts_count == 1);
    let rep2 = store.toggle_repost("u:900002", &pid, now + 60).unwrap();
    assert!(!rep2.reposted && rep2.reposts_count == 0);

    let bm = store.toggle_bookmark("u:900002", &pid, Some("Reading List"), None, now + 70).unwrap();
    assert!(bm.bookmarked && bm.folder == "Reading List");

    let awards = store.give_award("b:910002", "post", &pid, &GiveAwardRequest { award_kind: "gold".into(), message: Some("Nice!".into()) }, now + 80).unwrap();
    assert_eq!(awards.len(), 1);

    let edited = store.update_post(&pid, "u:900001", &UpdatePostRequest { body: Some("Updated body #sqlite".into()), ..Default::default() }, now + 90).unwrap();
    assert_eq!(edited.edit_history.len(), 1);

    // 4. Poll vote Human vs Bot split + poll closing scheduler
    let p_after_h = store.vote_poll("u:900002", &pid, &["opt_0".into()], now + 100).unwrap();
    assert_eq!(p_after_h.options[0].human_votes, 1);
    assert_eq!(p_after_h.options[0].bot_votes, 0);
    let p_after_b = store.vote_poll("b:910001", &pid, &["opt_0".into()], now + 110).unwrap();
    assert_eq!(p_after_b.options[0].all_votes, 2);
    assert_eq!(p_after_b.options[0].human_votes, 1);
    assert_eq!(p_after_b.options[0].bot_votes, 1);

    let closed_ids = store.close_expired_polls(now + 700_000).unwrap();
    assert!(closed_ids.contains(&pid));
    assert!(store.vote_poll("u:900003", &pid, &["opt_1".into()], now + 700_001).is_err());

    // 5. Scheduled post publishing
    let due = store.publish_due_scheduled_posts(now + 30 * 3_600_000).unwrap();
    assert!(due.iter().any(|p| p.post_id == "seed_post_26"));

    // 6. Comments nested tree, wiki revision, signals/report -> mod queue -> mod action -> mod log
    let c1 = store.create_comment("u:900002", &pid, &CreateCommentRequest { parent_comment_id: None, body: "Top comment".into(), media: vec![], is_mod_distinguished: false }, now + 120).unwrap();
    let _c2 = store.create_comment("b:910001", &pid, &CreateCommentRequest { parent_comment_id: Some(c1.comment.comment_id.clone()), body: "Bot nested reply".into(), media: vec![], is_mod_distinguished: false }, now + 130).unwrap();
    assert!(store.delete_comment(&c1.comment.comment_id, "u:900002", now + 140).unwrap());
    let tree = store.list_comment_tree(&pid, Some("u:900001"), CommentSort::Best, None, 50).unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].body, "[deleted]", "deleted parent with children must tombstone");
    assert_eq!(tree[0].children.len(), 1);

    let rev = store.update_wiki("rust", "u:900001", "# Updated c/rust Wiki v2", "Added WAL guide", now + 150).unwrap();
    assert_eq!(rev.revision, 2);
    assert_eq!(store.list_wiki_revisions("rust", 10).unwrap().len(), 2);

    store.record_signal("u:900002", &RecordSignalRequest { post_id: pid.clone(), kind: EngagementSignalKind::Report, dwell_ms: None, reason: Some("Check spam".into()), target_actor_id: None, community_slug: None }, now + 160).unwrap();
    let queue = store.list_mod_queue("rust", Some(ModQueueStatus::Pending), 20).unwrap();
    assert!(queue.iter().any(|q| q.target_id == pid));

    let log = store.apply_mod_action("rust", "b:910006", &ModActionRequest { action: ModActionKind::Approve, target_kind: "post".into(), target_id: pid.clone(), reason: Some("Looks good".into()), flair: None, queue_item_id: None }, now + 170).unwrap();
    assert_eq!(log.action, ModActionKind::Approve);
    assert!(store.list_mod_logs("rust", 20).unwrap().iter().any(|l| l.log_id == log.log_id));

    // 7. Lists, AlgoPrefs, Mute/Block, Notifications, Bot Cooldown, Custom Community
    let list = store.create_list("u:900001", &CreateCommunityListRequest { name: "My List".into(), description: "Desc".into(), kind: CommunityListKind::Mixed, is_private: false, is_pinned: true, member_actor_ids: vec!["b:910001".into()], community_slugs: vec!["rust".into()] }, now + 180).unwrap();
    assert!(store.list_lists("u:900001").unwrap().iter().any(|l| l.list_id == list.list_id));

    let prefs = store.put_algo_prefs("u:900001", &AlgoPreferencesDto { preset: AlgoPreset::BotHeavy, bot_content_ratio: 0.85, ..Default::default() }, now + 190).unwrap();
    assert_eq!(store.get_algo_prefs("u:900001").unwrap().preset, prefs.preset);

    store.set_mute_block("u:900001", &MuteBlockRequest { target_kind: MuteBlockTargetKind::Keyword, target_value: "spamword".into(), action: MuteBlockActionKind::Mute, reason: None }, now + 200).unwrap();
    assert_eq!(store.list_mutes_blocks("u:900001").unwrap().len(), 1);

    let notifs = store.list_notifications("u:900001", None, 20).unwrap();
    assert!(notifs.unread_count > 0);
    assert!(store.mark_notifications_read("u:900001", None).unwrap() > 0);
    assert_eq!(store.list_notifications("u:900001", None, 20).unwrap().unread_count, 0);

    assert!(store.check_and_record_bot_cooldown("b:910001", "thread:1", 30_000, 10, now + 210).unwrap());
    assert!(!store.check_and_record_bot_cooldown("b:910001", "thread:1", 30_000, 10, now + 215).unwrap());

    let new_comm = store.create_community("u:900002", &CreateCommunityRequest { slug: "c/new_lab".into(), name: "New Lab".into(), description: "Test".into(), category: "tech".into(), banner_media_id: None, icon_media_id: None, icon_emoji: None, accent_color: None, rules: vec![], post_flairs: vec![], user_flairs: vec![], wiki_markdown: String::new(), automod_rules: vec![], allow_bots: true, require_bot_disclosure: true, is_nsfw: false }, now + 220).unwrap();
    assert_eq!(new_comm.slug, "new_lab");

    // 8. Follow -> Block auto-unfollows both directions; SQL fallback search & thread pool work
    assert!(store.follow_actor("u:900007", "u:900002", now + 230).unwrap());
    assert!(store.get_actor("u:900002", Some("u:900007")).unwrap().unwrap().is_following);
    store.set_mute_block("u:900007", &MuteBlockRequest { target_kind: MuteBlockTargetKind::Actor, target_value: "u:900002".into(), action: MuteBlockActionKind::Block, reason: Some("test block".into()) }, now + 240).unwrap();
    let blocked_view = store.get_actor("u:900002", Some("u:900007")).unwrap().unwrap();
    assert!(blocked_view.is_blocked && !blocked_view.is_following);

    let sql_hits = store.search_posts_sql("c:rust kind:bot", Some("u:900001"), 20).unwrap();
    assert!(sql_hits.iter().any(|p| p.post_id == "seed_post_10"));

    let p05 = store.get_post("seed_post_05", Some("u:900001")).unwrap().unwrap();
    let thread_pool = store.list_thread_and_quote_pool(&p05, Some("u:900001")).unwrap();
    assert_eq!(thread_pool.len(), 4, "4-part X thread must load completely");
}

#[test]
fn cross_community_mod_idor_and_owner_ban_bypass_blocked() {
    let now = 1_760_100_000_000_i64;
    let store = CommunityStore::open_in_memory().unwrap();
    store.seed_if_empty(now).unwrap();

    // Regular member u:900007 creates evil_lab (becoming Owner of c/evil_lab)
    store
        .create_community(
            "u:900007",
            &CreateCommunityRequest {
                slug: "evil_lab".into(),
                name: "Evil Lab".into(),
                description: "Attacker community".into(),
                category: "tech".into(),
                banner_media_id: None,
                icon_media_id: None,
                icon_emoji: None,
                accent_color: None,
                rules: vec![],
                post_flairs: vec![],
                user_flairs: vec![],
                wiki_markdown: String::new(),
                automod_rules: vec![],
                allow_bots: true,
                require_bot_disclosure: true,
                is_nsfw: false,
            },
            now,
        )
        .unwrap();

    // 1. Cross-community post mutations against seed_post_01 (in c/announcements) MUST fail
    for action in [
        ModActionKind::Remove,
        ModActionKind::Approve,
        ModActionKind::Lock,
        ModActionKind::Pin,
        ModActionKind::Distinguish,
        ModActionKind::SetPostFlair,
    ] {
        let res = store.apply_mod_action(
            "evil_lab",
            "u:900007",
            &ModActionRequest {
                action,
                target_kind: "post".into(),
                target_id: "seed_post_01".into(),
                reason: Some("idor".into()),
                flair: Some("Pwned".into()),
                queue_item_id: None,
            },
            now + 10,
        );
        assert!(res.is_err(), "cross-community {action:?} on post must be rejected");
    }
    let p01 = store.get_post("seed_post_01", None).unwrap().unwrap();
    assert!(!p01.is_removed && !p01.is_locked);

    // 2. Cross-community comment mutation MUST fail
    let rust_comment = store
        .create_comment(
            "u:900002",
            "seed_post_02",
            &CreateCommentRequest {
                parent_comment_id: None,
                body: "Legit comment in c/rust".into(),
                media: vec![],
                is_mod_distinguished: false,
            },
            now + 20,
        )
        .unwrap();
    assert!(
        store
            .apply_mod_action(
                "evil_lab",
                "u:900007",
                &ModActionRequest {
                    action: ModActionKind::Remove,
                    target_kind: "comment".into(),
                    target_id: rust_comment.comment.comment_id.clone(),
                    reason: Some("idor comment".into()),
                    flair: None,
                    queue_item_id: None,
                },
                now + 30,
            )
            .is_err()
    );

    // 3. Cross-community mod queue dismissal MUST fail and leave c/rust's queue intact
    store
        .record_signal(
            "u:900002",
            &RecordSignalRequest {
                post_id: "seed_post_02".into(),
                kind: EngagementSignalKind::Report,
                dwell_ms: None,
                reason: Some("needs review".into()),
                target_actor_id: None,
                community_slug: None,
            },
            now + 40,
        )
        .unwrap();
    let rust_queue = store
        .list_mod_queue("rust", Some(ModQueueStatus::Pending), 20)
        .unwrap();
    let rust_qid = rust_queue
        .iter()
        .find(|q| q.target_id == "seed_post_02")
        .expect("pending queue item")
        .item_id
        .clone();
    assert!(
        store
            .apply_mod_action(
                "evil_lab",
                "u:900007",
                &ModActionRequest {
                    action: ModActionKind::ResolveReport,
                    target_kind: "post".into(),
                    target_id: "seed_post_02".into(),
                    reason: Some("dismiss foreign queue".into()),
                    flair: None,
                    queue_item_id: Some(rust_qid),
                },
                now + 50,
            )
            .is_err()
    );
    let rust_queue_after = store
        .list_mod_queue("rust", Some(ModQueueStatus::Pending), 20)
        .unwrap();
    assert!(rust_queue_after.iter().any(|q| q.target_id == "seed_post_02"));

    // 4. Owner-ban bypass via mismatched target_kind ("user" instead of "actor") MUST fail
    assert!(
        store
            .apply_mod_action(
                "rust",
                "b:910006",
                &ModActionRequest {
                    action: ModActionKind::BanActor,
                    target_kind: "user".into(),
                    target_id: "u:900001".into(),
                    reason: Some("bypass attempt".into()),
                    flair: None,
                    queue_item_id: None,
                },
                now + 60,
            )
            .is_err()
    );
    // Direct BanActor / UnbanActor against Owner MUST also fail
    assert!(
        store
            .apply_mod_action(
                "rust",
                "b:910006",
                &ModActionRequest {
                    action: ModActionKind::BanActor,
                    target_kind: "actor".into(),
                    target_id: "u:900001".into(),
                    reason: Some("direct owner ban".into()),
                    flair: None,
                    queue_item_id: None,
                },
                now + 65,
            )
            .is_err()
    );
    assert!(
        store
            .apply_mod_action(
                "rust",
                "b:910006",
                &ModActionRequest {
                    action: ModActionKind::UnbanActor,
                    target_kind: "actor".into(),
                    target_id: "u:900001".into(),
                    reason: Some("owner downgrade attempt".into()),
                    flair: None,
                    queue_item_id: None,
                },
                now + 66,
            )
            .is_err()
    );
    let rust_owner_view = store.get_community("rust", Some("u:900001")).unwrap().unwrap();
    assert_eq!(rust_owner_view.viewer_role, Some(CommunityMemberRole::Owner));

    // 5. Peer-mod escalation: Owner promotes u:900002 to Mod; BotMod b:910006 cannot ban/mute u:900002
    store
        .apply_mod_action(
            "rust",
            "u:900001",
            &ModActionRequest {
                action: ModActionKind::PromoteMod,
                target_kind: "actor".into(),
                target_id: "u:900002".into(),
                reason: None,
                flair: None,
                queue_item_id: None,
            },
            now + 70,
        )
        .unwrap();
    assert!(
        store
            .apply_mod_action(
                "rust",
                "b:910006",
                &ModActionRequest {
                    action: ModActionKind::BanActor,
                    target_kind: "actor".into(),
                    target_id: "u:900002".into(),
                    reason: Some("peer mod ban".into()),
                    flair: None,
                    queue_item_id: None,
                },
                now + 75,
            )
            .is_err()
    );
    assert!(
        store
            .apply_mod_action(
                "rust",
                "b:910006",
                &ModActionRequest {
                    action: ModActionKind::MuteActor,
                    target_kind: "actor".into(),
                    target_id: "u:900002".into(),
                    reason: Some("peer mod mute".into()),
                    flair: None,
                    queue_item_id: None,
                },
                now + 76,
            )
            .is_err()
    );
    // Owner CAN mute a Mod
    store
        .apply_mod_action(
            "rust",
            "u:900001",
            &ModActionRequest {
                action: ModActionKind::MuteActor,
                target_kind: "actor".into(),
                target_id: "u:900002".into(),
                reason: Some("owner mute".into()),
                flair: None,
                queue_item_id: None,
            },
            now + 80,
        )
        .unwrap();
}

#[test]
fn private_list_feed_access_is_owner_only() {
    let now = 1_760_100_000_000_i64;
    let store = CommunityStore::open_in_memory().unwrap();
    store.seed_if_empty(now).unwrap();

    let priv_list = store
        .create_list(
            "u:900001",
            &CreateCommunityListRequest {
                name: "Secret Watchlist".into(),
                description: "Private".into(),
                kind: CommunityListKind::Mixed,
                is_private: true,
                is_pinned: false,
                member_actor_ids: vec!["u:900003".into()],
                community_slugs: vec!["C/Rust".into()],
            },
            now,
        )
        .unwrap();
    assert_eq!(priv_list.community_slugs, vec!["rust"]);

    // Owner sees posts in private list
    let owner_feed = store
        .get_feed(
            "u:900001",
            &FeedRequestDto {
                tab: FeedTab::List(priv_list.list_id.clone()),
                sort: FeedSort::New,
                limit: 20,
                ..Default::default()
            },
            now,
        )
        .unwrap();
    assert!(!owner_feed.items.is_empty());

    // Unrelated viewer MUST NOT see any posts from private list!
    let other_feed = store
        .get_feed(
            "u:900007",
            &FeedRequestDto {
                tab: FeedTab::List(priv_list.list_id),
                sort: FeedSort::New,
                limit: 20,
                ..Default::default()
            },
            now,
        )
        .unwrap();
    assert!(
        other_feed.items.is_empty(),
        "private list must not leak posts to non-owner viewers"
    );
}

#[test]
fn multipart_thread_automod_schedule_and_self_vote_integrity() {
    let now = 1_760_100_000_000_i64;
    let store = CommunityStore::open_in_memory().unwrap();
    store.seed_if_empty(now).unwrap();

    // Create a community with a keyword Remove AutoMod rule
    store
        .create_community(
            "u:900001",
            &CreateCommunityRequest {
                slug: "strict_lab".into(),
                name: "Strict Lab".into(),
                description: "AutoMod test".into(),
                category: "tech".into(),
                banner_media_id: None,
                icon_media_id: None,
                icon_emoji: None,
                accent_color: None,
                rules: vec![],
                post_flairs: vec![],
                user_flairs: vec![],
                wiki_markdown: String::new(),
                automod_rules: vec![AutoModRuleDto {
                    rule_id: "r_ban_scam".into(),
                    community_id: String::new(),
                    name: "Block scam".into(),
                    enabled: true,
                    priority: 1,
                    keywords: vec!["badscam".into()],
                    regex_pattern: None,
                    min_author_karma: None,
                    min_account_age_secs: None,
                    max_links: None,
                    block_duplicate_spam: false,
                    apply_to_bots: true,
                    apply_to_humans: true,
                    action: AutoModActionKind::Remove,
                    assign_flair: None,
                    auto_reply_template: None,
                    created_at_ms: now,
                }],
                allow_bots: true,
                require_bot_disclosure: true,
                is_nsfw: false,
            },
            now,
        )
        .unwrap();

    // 1. AutoMod Remove on 2-part thread must mark BOTH root 1/2 and part 2/2 as is_removed
    let removed_thread = store
        .create_post(
            "u:900002",
            &CreatePostRequest {
                community_slug: Some("strict_lab".into()),
                format: PostFormat::Micro,
                body: "1/2 Check out this badscam offer".into(),
                thread_parts: vec![CreateThreadPartDto {
                    body: "2/2 Continuation part must also be removed".into(),
                    media: vec![],
                    link_preview: None,
                }],
                ..Default::default()
            },
            now + 10,
        )
        .unwrap();
    assert!(removed_thread.post.is_removed);
    assert_eq!(removed_thread.thread_posts.len(), 1);
    assert!(
        removed_thread.thread_posts[0].is_removed,
        "thread part 2/2 must inherit AutoMod is_removed"
    );

    // 2. Scheduled 2-part thread must publish BOTH parts when due & block self double-upvote
    let sched_time = now + 3_600_000;
    let sched_thread = store
        .create_post(
            "u:900001",
            &CreatePostRequest {
                community_slug: Some("rust".into()),
                format: PostFormat::Micro,
                status: PostStatus::Scheduled,
                scheduled_at_ms: Some(sched_time),
                body: "1/2 Scheduled thread start".into(),
                thread_parts: vec![CreateThreadPartDto {
                    body: "2/2 Scheduled thread continuation".into(),
                    media: vec![],
                    link_preview: None,
                }],
                ..Default::default()
            },
            now + 20,
        )
        .unwrap();
    let root_id = sched_thread.post.post_id.clone();
    let part2_id = sched_thread.thread_posts[0].post_id.clone();
    assert_eq!(sched_thread.thread_posts[0].scheduled_at_ms, Some(sched_time));

    let due = store.publish_due_scheduled_posts(sched_time).unwrap();
    let due_ids: Vec<&str> = due.iter().map(|p| p.post_id.as_str()).collect();
    assert!(due_ids.contains(&root_id.as_str()));
    assert!(due_ids.contains(&part2_id.as_str()), "scheduled thread part 2/2 must publish");

    // 3. Author calling vote_post(+1) on thread part 2/2 must NOT double-count 1 -> 2
    let self_vote = store.vote_post("u:900001", &part2_id, 1, sched_time + 10).unwrap();
    assert_eq!(self_vote.upvotes, 1);
    assert_eq!(self_vote.score, 1);
}

#[test]
fn sync_secondary_collision_underscore_nan_clear_and_store_edge_cases() {
    let now = 1_760_100_000_000_i64;
    let store = CommunityStore::open_in_memory().unwrap();
    store.seed_if_empty(now).unwrap();

    // 1. Secondary handle collision (alex_chen + alex_chen_700 both taken) & underscore-only "___"
    store
        .create_human_actor(
            699,
            &gpui_telegram_proto::community_contracts::RegisterHumanActorRequest {
                handle: "alex_chen_700".into(),
                display_name: "Squatter".into(),
                bio: None,
                location: None,
                website: None,
                avatar_media_id: None,
                banner_media_id: None,
                avatar_shape: None,
                avatar_color: None,
            },
            now,
        )
        .unwrap();
    let synced_700 = store
        .sync_telegram_identities(
            &TelegramHumanSyncInput {
                user_id: 700,
                username: Some("alex_chen".into()),
                display_name: "Alex 700".into(),
                bio: None,
                avatar_media_id: None,
            },
            &[],
            now,
        )
        .unwrap();
    assert_eq!(synced_700[0].handle, "alex_chen_700_2");

    let synced_under = store
        .sync_telegram_identities(
            &TelegramHumanSyncInput {
                user_id: 701,
                username: Some("___".into()),
                display_name: "Underscore".into(),
                bio: None,
                avatar_media_id: None,
            },
            &[],
            now,
        )
        .unwrap();
    assert_eq!(synced_under[0].handle, "user_701");

    // 2. put_algo_prefs NaN/Inf guard & bump_affinity NaN guard
    let saved_prefs = store
        .put_algo_prefs(
            "u:900001",
            &AlgoPreferencesDto {
                preset: AlgoPreset::Balanced,
                recency_weight: f64::NAN,
                affinity_weight: f64::INFINITY,
                quality_weight: f64::NEG_INFINITY,
                discovery_ratio: f64::NAN,
                bot_content_ratio: f64::NAN,
                show_explanations: true,
                allow_nsfw: false,
            },
            now,
        )
        .unwrap();
    assert!(saved_prefs.recency_weight.is_finite());
    assert!(saved_prefs.discovery_ratio.is_finite());
    store
        .with_conn(|c| actors::bump_affinity_conn(c, "u:900001", "author", "u:900002", f64::NAN, now))
        .unwrap();

    // 3. update_actor_tx clearing optional fields via Some("") persists NULL in SQLite
    store
        .update_actor(
            "b:910001",
            &UpdateCommunityActorRequest {
                avatar_media_id: Some("media_av_temp".into()),
                banner_media_id: Some("media_bn_temp".into()),
                bot_badge: Some("CUSTOM".into()),
                bot_model: Some("model-x".into()),
                bot_persona: Some("Persona X".into()),
                ..Default::default()
            },
            now + 10,
        )
        .unwrap();
    store
        .update_actor(
            "b:910001",
            &UpdateCommunityActorRequest {
                avatar_media_id: Some(String::new()),
                banner_media_id: Some(String::new()),
                bot_badge: Some(String::new()),
                bot_model: Some(String::new()),
                bot_persona: Some(String::new()),
                ..Default::default()
            },
            now + 20,
        )
        .unwrap();
    let reloaded = store.get_actor("b:910001", None).unwrap().unwrap();
    assert!(reloaded.avatar_media_id.is_none());
    assert!(reloaded.banner_media_id.is_none());
    assert!(reloaded.bot_badge.is_none());
    assert!(reloaded.bot_model.is_none());
    assert!(reloaded.bot_persona.is_none());

    // 4. Comment media public access reference check
    let c_media = store
        .create_comment(
            "u:900002",
            "seed_post_02",
            &CreateCommentRequest {
                parent_comment_id: None,
                body: "See attached chart".into(),
                media: vec![PostMediaAttachmentDto {
                    media_id: "comment_media_99".into(),
                    kind: "image".into(),
                    url: None,
                    thumbnail_url: None,
                    alt_text: None,
                    width: None,
                    height: None,
                    duration_secs: None,
                }],
                is_mod_distinguished: false,
            },
            now + 30,
        )
        .unwrap();
    assert!(store.is_media_referenced_publicly("comment_media_99").unwrap());
    store
        .delete_comment(&c_media.comment.comment_id, "u:900002", now + 35)
        .unwrap();
    assert!(!store.is_media_referenced_publicly("comment_media_99").unwrap());

    // 5. search_posts_sql honors has:link and uppercase c:C/Rust
    let link_hits = store.search_posts_sql("has:link", Some("u:900001"), 20).unwrap();
    assert!(!link_hits.is_empty(), "has:link must return link posts");
    let upper_comm_hits = store.search_posts_sql("c:C/Rust", Some("u:900001"), 20).unwrap();
    assert!(!upper_comm_hits.is_empty());

    // 6. update_post: Draft -> Published increments posts_count; Scheduled without timestamp rejected; Deleted cannot be edited
    let actor_posts_before = store.get_actor("u:900001", None).unwrap().unwrap().posts_count;
    let comm_posts_before = store.get_community("rust", None).unwrap().unwrap().posts_count;
    let draft = store
        .create_post(
            "u:900001",
            &CreatePostRequest {
                community_slug: Some("rust".into()),
                format: PostFormat::Micro,
                status: PostStatus::Draft,
                body: "Draft work in progress".into(),
                ..Default::default()
            },
            now + 40,
        )
        .unwrap();
    assert_eq!(store.get_actor("u:900001", None).unwrap().unwrap().posts_count, actor_posts_before);
    // Scheduled without scheduled_at_ms rejected
    assert!(
        store
            .update_post(
                &draft.post.post_id,
                "u:900001",
                &UpdatePostRequest {
                    status: Some(PostStatus::Scheduled),
                    ..Default::default()
                },
                now + 45,
            )
            .is_err()
    );
    // Publish draft -> increments actor & community posts_count
    store
        .update_post(
            &draft.post.post_id,
            "u:900001",
            &UpdatePostRequest {
                status: Some(PostStatus::Published),
                ..Default::default()
            },
            now + 50,
        )
        .unwrap();
    assert_eq!(
        store.get_actor("u:900001", None).unwrap().unwrap().posts_count,
        actor_posts_before + 1
    );
    assert_eq!(
        store.get_community("rust", None).unwrap().unwrap().posts_count,
        comm_posts_before + 1
    );
    // Delete then attempt edit -> rejected
    store.delete_post(&draft.post.post_id, "u:900001", now + 55).unwrap();
    assert!(
        store
            .update_post(
                &draft.post.post_id,
                "u:900001",
                &UpdatePostRequest {
                    body: Some("Resurrect?".into()),
                    status: Some(PostStatus::Published),
                    ..Default::default()
                },
                now + 60,
            )
            .is_err()
    );

    // 7. check_and_record_bot_cooldown returns Ok(false) immediately when max_per_hour <= 0
    assert!(!store.check_and_record_bot_cooldown("b:910001", "zero_cap", 0, 0, now + 70).unwrap());
    assert!(!store.check_and_record_bot_cooldown("b:910001", "neg_cap", 0, -5, now + 71).unwrap());

    // 8. Uppercase "C/Rust" community mute actually mutes c/rust in get_feed
    let mb = store
        .set_mute_block(
            "u:900007",
            &MuteBlockRequest {
                target_kind: MuteBlockTargetKind::Community,
                target_value: "C/Rust".into(),
                action: MuteBlockActionKind::Mute,
                reason: None,
            },
            now + 80,
        )
        .unwrap();
    assert_eq!(mb.target_value, "rust");
    let feed = store
        .get_feed(
            "u:900007",
            &FeedRequestDto {
                tab: FeedTab::All,
                sort: FeedSort::New,
                limit: 50,
                ..Default::default()
            },
            now + 85,
        )
        .unwrap();
    assert!(
        feed.items
            .iter()
            .all(|i| i.post.community_slug.as_deref() != Some("rust")),
        "muting C/Rust must exclude c/rust posts from feed"
    );
    assert!(
        store
            .remove_mute_block(
                "u:900007",
                MuteBlockTargetKind::Community,
                "C/Rust",
                MuteBlockActionKind::Mute,
            )
            .unwrap()
    );
}

#[test]
fn r3_regression_set_user_flair_thread_part_automod_comment_tombstone_and_signal_norm() {
    use crate::model::MAX_COMMENT_BODY_CHARS;
    use gpui_telegram_proto::community_contracts::{
        AutoModActionKind, AutoModRuleDto, CreateCommunityRequest, CreateThreadPartDto,
        EngagementSignalKind, RecordSignalRequest,
    };

    let now = 1_760_100_000_000_i64;
    let store = CommunityStore::open_in_memory().unwrap();
    store.seed_if_empty(now).unwrap();

    // 1. SetUserFlair privilege escalation blocked in SQLite store:
    // Promote u:900002 to Mod in c/rust; BotMod b:910006 must be blocked from setting flair on Owner u:900001 and Mod u:900002
    store
        .apply_mod_action(
            "rust",
            "u:900001",
            &ModActionRequest {
                action: ModActionKind::PromoteMod,
                target_kind: "actor".into(),
                target_id: "u:900002".into(),
                reason: Some("Promote peer mod".into()),
                flair: None,
                queue_item_id: None,
            },
            now + 5,
        )
        .unwrap();
    for target in ["u:900001", "u:900002"] {
        let err = store
            .apply_mod_action(
                "rust",
                "b:910006",
                &ModActionRequest {
                    action: ModActionKind::SetUserFlair,
                    target_kind: "actor".into(),
                    target_id: target.into(),
                    reason: None,
                    flair: Some("PWNED_FLAIR".into()),
                    queue_item_id: None,
                },
                now + 10,
            )
            .unwrap_err();
        assert!(matches!(err, TelegramError::PermissionDenied(_)));
    }
    // Owner CAN set flair on Owner and Mod; BotMod CAN set flair on regular Member u:900007
    store
        .join_community("rust", "u:900007", now + 11)
        .unwrap();
    store
        .apply_mod_action(
            "rust",
            "u:900001",
            &ModActionRequest {
                action: ModActionKind::SetUserFlair,
                target_kind: "actor".into(),
                target_id: "u:900001".into(),
                reason: None,
                flair: Some("Founder".into()),
                queue_item_id: None,
            },
            now + 12,
        )
        .unwrap();
    store
        .apply_mod_action(
            "rust",
            "b:910006",
            &ModActionRequest {
                action: ModActionKind::SetUserFlair,
                target_kind: "actor".into(),
                target_id: "u:900007".into(),
                reason: None,
                flair: Some("Contributor".into()),
                queue_item_id: None,
            },
            now + 13,
        )
        .unwrap();

    // 2. Multi-part thread part 2/3 AutoMod Remove catches banned content in part 2/3
    store
        .create_community(
            "u:900001",
            &CreateCommunityRequest {
                slug: "strict_thread_lab".into(),
                name: "Strict Thread Lab".into(),
                description: "Thread AutoMod test".into(),
                category: "tech".into(),
                banner_media_id: None,
                icon_media_id: None,
                icon_emoji: None,
                accent_color: None,
                rules: vec![],
                post_flairs: vec![],
                user_flairs: vec![],
                wiki_markdown: String::new(),
                automod_rules: vec![AutoModRuleDto {
                    rule_id: "r_ban_thread_spam".into(),
                    community_id: String::new(),
                    name: "Block thread spam".into(),
                    enabled: true,
                    priority: 1,
                    keywords: vec!["forbidden_spam_xyz".into()],
                    regex_pattern: None,
                    min_author_karma: None,
                    min_account_age_secs: None,
                    max_links: None,
                    block_duplicate_spam: false,
                    apply_to_bots: true,
                    apply_to_humans: true,
                    action: AutoModActionKind::Remove,
                    assign_flair: None,
                    auto_reply_template: None,
                    created_at_ms: now,
                }],
                allow_bots: true,
                require_bot_disclosure: true,
                is_nsfw: false,
            },
            now + 15,
        )
        .unwrap();
    let smuggled = store
        .create_post(
            "u:900002",
            &CreatePostRequest {
                community_slug: Some("strict_thread_lab".into()),
                format: PostFormat::Micro,
                body: "1/3 Completely innocent opening post".into(),
                thread_parts: vec![
                    CreateThreadPartDto {
                        body: "2/3 Smuggled forbidden_spam_xyz payload here".into(),
                        media: vec![],
                        link_preview: None,
                    },
                    CreateThreadPartDto {
                        body: "3/3 Closing note".into(),
                        media: vec![],
                        link_preview: None,
                    },
                ],
                ..Default::default()
            },
            now + 20,
        )
        .unwrap();
    assert_eq!(smuggled.automod_action, AutoModActionKind::Remove);
    assert!(smuggled.post.is_removed);
    assert_eq!(smuggled.thread_posts.len(), 2);
    assert!(smuggled.thread_posts.iter().all(|p| p.is_removed));

    // 3. update_comment rejects tombstoned comments AND comments > MAX_COMMENT_BODY_CHARS
    let parent_c = store
        .create_comment(
            "u:900002",
            "seed_post_02",
            &CreateCommentRequest {
                parent_comment_id: None,
                body: "Parent comment".into(),
                media: vec![],
                is_mod_distinguished: false,
            },
            now + 25,
        )
        .unwrap();
    let _child_c = store
        .create_comment(
            "u:900001",
            "seed_post_02",
            &CreateCommentRequest {
                parent_comment_id: Some(parent_c.comment.comment_id.clone()),
                body: "Child reply".into(),
                media: vec![],
                is_mod_distinguished: false,
            },
            now + 26,
        )
        .unwrap();
    // Oversized edit on live comment rejected
    let oversized = "x".repeat(MAX_COMMENT_BODY_CHARS + 1);
    assert!(matches!(
        store.update_comment(&parent_c.comment.comment_id, "u:900002", &oversized, now + 27),
        Err(TelegramError::Validation(_))
    ));
    // Delete parent (tombstoned because child exists) -> subsequent edit MUST be rejected
    store
        .delete_comment(&parent_c.comment.comment_id, "u:900002", now + 28)
        .unwrap();
    assert!(matches!(
        store.update_comment(
            &parent_c.comment.comment_id,
            "u:900002",
            "Trying to resurrect tombstone",
            now + 29
        ),
        Err(TelegramError::Validation(_))
    ));
    let body_after: String = store
        .with_conn(|c| {
            c.query_row(
                "SELECT body_markdown FROM community_comments WHERE comment_id = ?1",
                rusqlite::params![parent_c.comment.comment_id],
                |r| r.get(0),
            )
            .map_err(|e| TelegramError::Database(e.to_string()))
        })
        .unwrap();
    assert_eq!(body_after, "[deleted]");

    // 4. record_signal with "C/Rust" stores normalized target_key = "rust" in community_actor_affinities
    store
        .record_signal(
            "u:900007",
            &RecordSignalRequest {
                post_id: "seed_post_02".into(),
                kind: EngagementSignalKind::ShowMore,
                dwell_ms: None,
                target_actor_id: None,
                community_slug: Some("C/Rust".into()),
                reason: None,
            },
            now + 35,
        )
        .unwrap();
    let stored_keys: Vec<String> = store
        .with_conn(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT target_key FROM community_actor_affinities
                     WHERE viewer_actor_id = 'u:900007' AND target_kind = 'community'",
                )
                .map_err(|e| TelegramError::Database(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| TelegramError::Database(e.to_string()))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(|e| TelegramError::Database(e.to_string()))?);
            }
            Ok(out)
        })
        .unwrap();
    assert_eq!(stored_keys, vec!["rust".to_string()]);
}
