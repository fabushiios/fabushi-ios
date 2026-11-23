//! Tier 3 — 36 Pairwise Cross-Feature Interaction Opaque-Box E2E Tests (`TC-T3-PAIR-01..36`).
//!
//! Mathematically verified full coverage: every feature `F01..F34` appears in >= 2 distinct pairs
//! (30 features x 2 + 4 architectural hubs [F11, F17, F22, F23] x 3 = 72 endpoints = 36 pairs).
//! Every test calls `FabushiE2EHarness::connect_production()` (zero mocks, zero synthetic stubs).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use fabushi_e2e_ffi::FabushiE2EHarness;
use fabushi_e2e_ffi::contracts::{
    ImeEvent, ImeState, TotpAlgorithm, assert_no_sample_chats, encode_waveform_5bit,
    extract_context_reference_badges, verify_26_settings_sections, verify_29_toolsets,
    verify_argon2id_params, verify_controlled_banner_hex,
    verify_honest_capability_disposition, verify_ime_commit_fence, verify_loop_budget_5_hops,
    verify_mcp_audit_redaction, verify_rfc6238_totp, verify_sliding_replay_window_128,
    verify_talk_vad_bargein, verify_waveform_5bit_63bytes, verify_wcag_accent_luma_guard,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[test]
fn tc_t3_pair_01_f01_x_f23_split_key_v2_login_unlocks_zero_knowledge_vault() {
    let h = FabushiE2EHarness::connect_production();
    verify_argon2id_params(65536, 3, 4, 16, 32).expect("valid Argon2id");
    let login = h.expect_ok(
        "auth.register",
        json!({
            "username": "pair01_alice",
            "auth_version": 2,
            "auth_salt_b64": B64.encode([0x11u8; 16]),
            "auth_verifier_b64": B64.encode([0x22u8; 32]),
            "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}
        }),
    );
    assert_eq!(login["vault_unlocked"], true);

    let _ = h.expect_ok(
        "vault.item_upsert",
        json!({"category": "password", "item_key": "apple.com:alice", "ciphertext_b64": B64.encode([0x99u8; 48]), "nonce_b64": B64.encode([0x01u8; 12])}),
    );
    let pulled = h.expect_ok("vault.items_pull", json!({}));
    assert!(
        pulled["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|i| i["item_key"] == "apple.com:alice")
    );
}

#[test]
fn tc_t3_pair_02_f01_x_f33_unreachable_proxy_blocks_login_without_direct_leak() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "proxy.set",
        json!({"proxy_type": "http", "host": "127.0.0.1", "port": 1, "fallback_to_direct": false}),
    );
    h.server().clear_recorded_requests();
    let err = h.expect_err(
        "auth.login",
        json!({"username": "pair02_user", "password": "Password1!"}),
        &["proxy_unreachable", "network_unavailable"],
    );
    assert!(err.retryable);
    assert_eq!(h.server().request_count_for_path("/api/v1/auth/login"), 0);
}

#[test]
fn tc_t3_pair_03_f02_x_f05_otp_registration_enables_cloud_2fa_and_lists_session() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("auth.email.send_code", json!({"email": "pair03@fabushi.test"}));
    let _ = h.expect_ok(
        "auth.email.register",
        json!({"email": "pair03@fabushi.test", "code": "654321", "username": "pair03_u", "password": "ValidPass8!"}),
    );
    let tfa = h.expect_ok(
        "auth.two_factor.set",
        json!({"new_password": "Cloud2FAPair03!", "hint": "p3"}),
    );
    assert_eq!(tfa["has_password"], true);
    let devices = h.expect_ok("session.list_devices", json!({}));
    assert!(!devices["sessions"].as_array().expect("sessions").is_empty());
}

#[test]
fn tc_t3_pair_04_f02_x_f10_otp_user_resolves_peer_adds_contact_and_blocks() {
    let h = FabushiE2EHarness::connect_production();
    let sent = h.expect_ok("auth.phone.send_code", json!({"phone": "+14155550304"}));
    let _ = h.expect_ok(
        "auth.phone.verify",
        json!({"phone": "+14155550304", "code": "123456", "phone_code_hash": sent["phone_code_hash"]}),
    );
    let contact = h.expect_ok("contacts.add", json!({"username": "peer_04", "first_name": "Peer"}));
    let uid = contact["user_id"].as_u64().expect("uid");
    let resolved = h.expect_ok("contacts.resolve_peer", json!({"username": "@peer_04"}));
    assert_eq!(resolved["kind"], "user");
    let blocked = h.expect_ok("contacts.block", json!({"user_id": uid}));
    assert_eq!(blocked["blocked"], true);
}

#[test]
fn tc_t3_pair_05_f03_x_f30_alipay_deep_link_routes_through_action_bus() {
    let h = FabushiE2EHarness::connect_production();
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ios_oauth"}));
    let state = started["state"].as_str().expect("state");
    let url = format!("fabushi://auth/alipay?state={state}&auth_code=pair05_code");
    let routed = h.expect_ok("actions.route_url", json!({"url": url}));
    assert_eq!(routed["scheme"], "fabushi");
    let done = h.expect_ok(
        "auth.alipay.handle_deep_link",
        json!({"url": url, "expected_attempt_id": started["attempt_id"]}),
    );
    assert_eq!(done["authenticated"], true);
}

#[test]
fn tc_t3_pair_06_f03_x_f29_alipay_session_reconciles_stars_and_storekit_policy() {
    let h = FabushiE2EHarness::connect_production();
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ios_oauth"}));
    let link = format!("fabushi://auth/alipay?state={}&auth_code=p06", started["state"].as_str().expect("s"));
    let done = h.expect_ok(
        "auth.alipay.handle_deep_link",
        json!({"url": link, "expected_attempt_id": started["attempt_id"]}),
    );
    assert_eq!(done["authenticated"], true);
    let bal = h.expect_ok("wallet.balance", json!({}));
    assert!(bal["stars_balance"].as_i64().expect("stars_balance") >= 0);
    let sk = h.expect_ok("wallet.storekit_restore", json!({}));
    assert!(sk["restored_products"].is_array());
    assert!(matches!(
        sk["disposition"].as_str(),
        Some("restored") | Some("no_purchases") | Some("server_blocked")
    ));
}

#[test]
fn tc_t3_pair_07_f04_x_f05_qr_paired_device_revocation_invalidates_session() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "auth.register",
        json!({"username": "pair07_owner", "auth_version": 2, "auth_salt_b64": B64.encode([1u8; 16]), "auth_verifier_b64": B64.encode([2u8; 32]), "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}}),
    );
    let sec = h.spawn_secondary_host("iPad Pair07");
    let init = sec.expect_ok("auth.pair.init", json!({"device_name": "iPad Pair07"}));
    let tok = init["pair_token"].as_str().expect("tok");
    let _ = h.expect_ok("auth.pair.accept", json!({"pair_token": tok}));
    let paired = sec.expect_ok("auth.pair.poll", json!({"pair_token": tok}));
    let sid = paired["session"]["session_id"].as_str().expect("sid");

    let rev = h.expect_ok("session.revoke_device", json!({"session_id": sid}));
    assert_eq!(rev["revoked"], true);
    let err = sec.expect_err("chats.list", json!({}), &["unauthorized"]);
    assert!(!err.retryable);
}

#[test]
fn tc_t3_pair_08_f04_x_f23_qr_paired_device_completes_x25519_vault_sync_and_totp() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "auth.register",
        json!({"username": "pair08_owner", "auth_version": 2, "auth_salt_b64": B64.encode([3u8; 16]), "auth_verifier_b64": B64.encode([4u8; 32]), "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}}),
    );
    let sec = h.spawn_secondary_host("iPad VaultSync");
    let init = sec.expect_ok("auth.pair.init", json!({"device_name": "iPad VaultSync"}));
    let _ = h.expect_ok("auth.pair.accept", json!({"pair_token": init["pair_token"]}));
    let _ = sec.expect_ok("auth.pair.poll", json!({"pair_token": init["pair_token"]}));

    let sync_req = sec.expect_ok(
        "vault.device_sync_request",
        json!({"device_name": "iPad VaultSync", "ephemeral_x25519_pub_b64": B64.encode([0x55u8; 32])}),
    );
    let rid = sync_req["request_id"].as_str().expect("rid");
    let _ = h.expect_ok("vault.device_sync_approve", json!({"request_id": rid, "wrapped_key_b64": B64.encode([0x66u8; 48])}));
    let status = sec.expect_ok("vault.device_sync_status", json!({"request_id": rid}));
    assert_eq!(status["status"], "approved");

    let totp = sec.expect_ok(
        "vault.totp_generate",
        json!({"secret_or_otpauth": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", "unix_secs": 59, "digits": 6, "period_secs": 30, "algorithm": "SHA1"}),
    );
    verify_rfc6238_totp("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59, 30, 6, TotpAlgorithm::Sha1, totp["code"].as_str().expect("code")).expect("287082");
}

#[test]
fn tc_t3_pair_09_f06_x_f11_account_switch_bumps_generation_and_fences_late_pts_sync() {
    let h = FabushiE2EHarness::connect_production();
    let a = h.expect_ok("session.add_account", json!({"username": "p09_a", "display_name": "A", "token": "ta"}));
    let b = h.expect_ok("session.add_account", json!({"username": "p09_b", "display_name": "B", "token": "tb"}));
    let _ = h.expect_ok("session.switch_account", json!({"account_id": a["account_id"]}));
    let g_a = h.current_generation();

    let switched = h.expect_ok("session.switch_account", json!({"account_id": b["account_id"]}));
    assert!(switched["generation"].as_u64().expect("generation") > g_a);

    let stale = h.expect_err_with_gen(
        "sync.get_difference",
        g_a,
        a["account_id"].as_str(),
        json!({"chat_id": 1, "pts": 0}),
        &["stale_generation"],
    );
    assert!(!stale.retryable);
}

#[test]
fn tc_t3_pair_10_f06_x_f34_multi_account_generation_bump_and_panic_isolation() {
    let h = FabushiE2EHarness::connect_production();
    let acc1 = h.expect_ok("session.add_account", json!({"username": "p10_a", "display_name": "A", "token": "t10a"}));
    let acc2 = h.expect_ok("session.add_account", json!({"username": "p10_b", "display_name": "B", "token": "t10b"}));
    let _ = h.expect_ok("session.switch_account", json!({"account_id": acc1["account_id"]}));
    let g0 = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g0");
    let switched = h.expect_ok("session.switch_account", json!({"account_id": acc2["account_id"]}));
    assert!(switched["generation"].as_u64().expect("switched generation") > g0);

    let panic_err = h.expect_err("debug.trigger_panic", json!({}), &["host_panic"]);
    assert!(!panic_err.retryable);
    assert!(!panic_err.message.is_empty());

    let stale = h.expect_err_with_gen("chats.list", g0, acc1["account_id"].as_str(), json!({}), &["stale_generation"]);
    assert!(!stale.retryable);
    let fresh = h.expect_ok("chats.list", json!({}));
    assert_no_sample_chats(&fresh).expect("no sample chats after panic recovery");
}

#[test]
fn tc_t3_pair_11_f07_x_f31_adaptive_shell_propagates_themes_wcag_and_locale() {
    let h = FabushiE2EHarness::connect_production();
    let nav = h.expect_ok("actions.navigate", json!({"route": "settings"}));
    assert_eq!(nav["active_panel"], "settings");
    let theme = h.expect_ok("settings.theme_set", json!({"mode": "tinted", "accent_hex": "#FFFF00"}));
    verify_wcag_accent_luma_guard(
        "#FFFF00",
        theme["resolved_palette"]["effective_accent_hex"].as_str().expect("hex"),
        theme["resolved_palette"]["contrast_adjusted"].as_bool().expect("adj"),
    )
    .expect("WCAG darkened");
    let loc = h.expect_ok("settings.locale_set", json!({"locale": "zh-Hans"}));
    assert_eq!(loc["locale"], "zh-Hans");
}

#[test]
fn tc_t3_pair_12_f07_x_f32_unified_shell_traverses_26_settings_and_storage_stats() {
    let h = FabushiE2EHarness::connect_production();
    let nav = h.expect_ok("actions.navigate", json!({"route": "settings", "section": "local_storage"}));
    assert_eq!(nav["active_panel"], "settings");
    let list = h.expect_ok("settings.sections_list", json!({}));
    let ids: Vec<String> = list["sections"].as_array().expect("s").iter().filter_map(|v| v["id"].as_str().map(str::to_owned)).collect();
    verify_26_settings_sections(&ids).expect("26 sections");
    let stats = h.expect_ok("settings.storage_stats", json!({}));
    let cas = stats["cas_media_bytes"].as_u64().expect("cas_media_bytes");
    let total = stats["total_bytes"].as_u64().expect("total_bytes");
    assert!(total >= cas);
}

#[test]
fn tc_t3_pair_13_f08_x_f18_multi_select_bot_group_activates_floor_and_loop_budget() {
    let h = FabushiE2EHarness::connect_production();
    let grp = h.expect_ok(
        "chats.compose_send",
        json!({
            "client_nonce": "p13_grp",
            "recipients": [{"user_id": 5101, "is_bot": true}, {"user_id": 5102, "is_bot": true}],
            "is_group_mode": true,
            "custom_group_title": "Multi-Agent Council",
            "initial_text": "Coordinate architecture",
            "message_nonce": 13001
        }),
    );
    let cid = grp["chat"]["chat_id"].as_i64().expect("cid");
    let floor = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": cid, "agent_id": "bot_5101", "priority": "direct_mention", "now_ms": now_ms()}),
    );
    assert_eq!(floor["granted"], true);
    let hop = h.expect_ok("multi_agent.loop_budget_record", json!({"chat_id": cid, "agent_id": "bot_5101", "max_hops": 5}));
    verify_loop_budget_5_hops(1, 5, hop["exhausted"].as_bool().unwrap_or(true)).expect("hop 1 ok");
}

#[test]
fn tc_t3_pair_14_f08_x_f10_compose_excludes_blocked_peers_and_materializes_prestart_bot() {
    let h = FabushiE2EHarness::connect_production();
    let allowed = h.expect_ok("contacts.add", json!({"username": "p14_allowed", "first_name": "Allowed"}));
    let allowed_uid = allowed["user_id"].as_u64().expect("allowed_uid");
    let blocked = h.expect_ok("contacts.add", json!({"username": "p14_blocked", "first_name": "Blocked"}));
    let blocked_uid = blocked["user_id"].as_u64().expect("blocked_uid");

    let _ = h.expect_ok("contacts.block", json!({"user_id": blocked_uid}));
    let cands = h.expect_ok("chats.compose_candidates", json!({"query": "", "include_bots": true}));
    let arr = cands["candidates"].as_array().expect("candidates");
    assert!(arr.iter().any(|c| c["user_id"].as_u64() == Some(allowed_uid)));
    assert!(!arr.iter().any(|c| c["user_id"].as_u64() == Some(blocked_uid)));
    let pre = h.expect_ok("chats.prestart_bot", json!({"bot_username": "helper_bot"}));
    let started = h.expect_ok("chats.start_bot", json!({"bot_user_id": pre["bot_user_id"]}));
    assert_eq!(started["chat_type"], 4);
}

#[test]
fn tc_t3_pair_15_f09_x_f11_folders_pins_topics_survive_ws_disconnect_and_pts_catchup() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 2, "title": "SyncForum", "is_forum": true}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok("chats.pin_dialog", json!({"chat_id": cid, "pinned": true}));
    let topic = h.expect_ok("topics.create", json!({"chat_id": cid, "title": "General"}));
    let tid = topic["topic_id"].as_i64().expect("topic_id");
    let folder = h.expect_ok("folders.create", json!({"title": "PinnedForum", "included_chat_ids": [cid], "excluded_chat_ids": []}));
    let fid = folder["id"].as_i64().expect("folder_id");
    let _ = h.expect_ok("messages.send", json!({"chat_id": cid, "text": "pre-disconnect msg", "client_nonce": 15001}));

    h.server().drop_next_ws_connection();
    let diff = h.expect_ok("sync.get_difference", json!({"chat_id": cid, "pts": 0, "limit": 20}));
    assert_eq!(diff["snapshot_required"], false);
    assert!(
        h.expect_ok("folders.list", json!({}))["folders"]
            .as_array()
            .expect("folders")
            .iter()
            .any(|f| f["id"].as_i64() == Some(fid))
    );
    assert!(
        h.expect_ok("topics.list", json!({"chat_id": cid}))["topics"]
            .as_array()
            .expect("topics")
            .iter()
            .any(|t| t["topic_id"].as_i64() == Some(tid))
    );
}

#[test]
fn tc_t3_pair_16_f09_x_f27_folder_and_archive_scoped_search_respects_membership() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "ArchivedRoom"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok("folders.create", json!({"title": "ArchiveSearchFolder", "included_chat_ids": [cid], "excluded_chat_ids": []}));
    let _ = h.expect_ok("messages.send", json!({"chat_id": cid, "text": "archive_search_token_16", "client_nonce": 16001}));
    let _ = h.expect_ok("chats.archive", json!({"chat_id": cid, "archived": true}));
    let found = h.expect_ok("search.query", json!({"q": "archive_search_token_16", "chat_id": cid}));
    assert!(found["total"].as_u64().expect("search total") >= 1);
}

#[test]
fn tc_t3_pair_17_f12_x_f13_transcript_updates_edit_tombstone_reaction_and_pin() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "EditTrans"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let mid = h.expect_ok("messages.send", json!({"chat_id": cid, "text": "v1", "client_nonce": 17001}))["message_id"]
        .as_i64()
        .expect("mid");
    let _ = h.expect_ok("messages.edit", json!({"chat_id": cid, "message_id": mid, "text": "v2"}));
    let _ = h.expect_ok("messages.react", json!({"chat_id": cid, "message_id": mid, "emoji": "✅"}));
    let _ = h.expect_ok("messages.pins.pin", json!({"chat_id": cid, "message_id": mid}));
    let del = h.expect_ok("messages.delete", json!({"chat_id": cid, "message_ids": [mid], "revoke": true}));
    assert_eq!(del["revoke"], true);

    let pins = h.expect_ok("messages.pins.list", json!({"chat_id": cid}));
    assert!(
        !pins["pinned_message_ids"]
            .as_array()
            .expect("pinned_message_ids")
            .iter()
            .any(|v| v.as_i64() == Some(mid))
    );
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    assert!(
        hist["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .all(|m| m["message_id"].as_i64() != Some(mid) || m["deleted"] == true || m["revoked"] == true)
    );
}

#[test]
fn tc_t3_pair_18_f12_x_f25_transcript_cards_render_cas_206_waveform_sticker_story_iv() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "RichMedia"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let samples: Vec<u8> = (0..100).map(|i| (i % 32) as u8).collect();
    let wf = encode_waveform_5bit(&samples);
    verify_waveform_5bit_63bytes(&samples, &wf).expect("63B");

    h.server().seed_media_blob("p18_vid", vec![0x42u8; 2048]);
    let stream = h.expect_ok("media.stream_range", json!({"file_id": "p18_vid", "range_start": 0, "range_end": 511}));
    assert_eq!(stream["status"], 206);

    let sent = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "content": {"type": "voice", "file_id": "p18_vid", "duration_secs": 3, "waveform_b64": B64.encode(&wf)}, "client_nonce": 18001}),
    );
    let mid = sent["message_id"].as_i64().expect("message_id");
    assert!(mid > 0);
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    assert!(
        hist["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .any(|m| m["message_id"].as_i64() == Some(mid)),
        "voice card message must roundtrip via messages.history"
    );
}

#[test]
fn tc_t3_pair_19_f13_x_f27_revoke_tombstone_and_ttl_evict_from_tantivy_and_fts5() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "EvictSearch"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let mid = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "evict_token_pair19", "client_nonce": 19001}),
    )["message_id"]
        .as_i64()
        .expect("mid");
    let pre = h.expect_ok("search.query", json!({"q": "evict_token_pair19"}));
    assert!(
        pre["total"].as_u64().expect("pre-delete search total") >= 1,
        "message must be indexed before deletion"
    );
    let ttl = h.expect_ok("chats.set_ttl", json!({"chat_id": cid, "ttl_seconds": 60}));
    assert_eq!(ttl["ttl_seconds"], 60);

    let _ = h.expect_ok("messages.delete", json!({"chat_id": cid, "message_ids": [mid], "revoke": true}));
    let online = h.expect_ok("search.query", json!({"q": "evict_token_pair19"}));
    assert_eq!(online["total"], 0);
    let offline = h.expect_ok("search.query", json!({"q": "evict_token_pair19", "offline_only": true}));
    assert_eq!(offline["total"], 0);
}

#[test]
fn tc_t3_pair_20_f14_x_f15_cjk_ime_fence_protects_slash_autocomplete_and_context_badges() {
    let h = FabushiE2EHarness::connect_production();
    assert_eq!(verify_ime_commit_fence(ImeState::Composing, ImeEvent::KeyEnter, 0).action, "ime_candidate_chosen");
    let _ = h.expect_ok("messages.composer.ime_event", json!({"event": "start", "preedit": "zhuangtai"}));
    let _ = h.expect_ok("messages.composer.ime_event", json!({"event": "commit", "committed": "/status @file:src/lib.rs"}));
    let fenced = h.expect_ok("messages.composer.ime_event", json!({"event": "key_enter", "elapsed_since_commit_ms": 30}));
    assert_eq!(fenced["action"], "none");

    let badges = extract_context_reference_badges("/status @file:src/lib.rs @git:HEAD");
    assert_eq!(badges.len(), 2);
    let extracted = h.expect_ok(
        "fabushi.context.extract_badges",
        json!({"text": "/status @file:src/lib.rs @git:HEAD"}),
    );
    assert_eq!(extracted["badges"].as_array().expect("badges").len(), 2);
    let cmds = h.expect_ok("fabushi.commands.list", json!({"prefix": "/stat"}));
    assert!(!cmds["commands"].as_array().expect("cmds").is_empty());
}

#[test]
fn tc_t3_pair_21_f14_x_f25_voice_gesture_encodes_63b_waveform_and_uploads_cas() {
    let h = FabushiE2EHarness::connect_production();
    let samples: Vec<u8> = (0..100).map(|i| ((i + 5) % 32) as u8).collect();
    let _ = h.expect_ok("messages.composer.voice_gesture", json!({"action": "press"}));
    let _ = h.expect_ok("messages.composer.voice_gesture", json!({"action": "move", "dx": 0.0, "dy": -90.0, "elapsed_secs": 1.0}));
    let done = h.expect_ok("messages.composer.voice_gesture", json!({"action": "lock_send", "elapsed_secs": 2.5, "samples": samples}));
    assert_eq!(done["waveform_bytes_len"], 63);

    let audio = vec![0x55u8; 4096];
    let expected_sha = sha256_hex(&audio);
    let init = h.expect_ok(
        "media.upload_init",
        json!({"filename": "voice.ogg", "mime_type": "audio/ogg", "total_size": audio.len(), "part_size": audio.len(), "expected_sha256": expected_sha}),
    );
    let _ = h.expect_ok("media.upload_part", json!({"upload_id": init["upload_id"], "part_index": 0, "data_b64": B64.encode(&audio)}));
    let comp = h.expect_ok("media.upload_complete", json!({"upload_id": init["upload_id"]}));
    assert_eq!(comp["sha256"].as_str(), Some(expected_sha.as_str()));
    assert!(!comp["file_id"].as_str().expect("file_id").is_empty());
}

#[test]
fn tc_t3_pair_22_f15_x_f17_slash_commands_mutate_persisted_agent_studio_state() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 4, "title": "StudioBot"}))["chat_id"]
        .as_i64()
        .expect("cid");
    for cmd in ["/skills", "/memory", "/cron", "/kanban", "/toolsets"] {
        let out = h.expect_ok("fabushi.commands.execute", json!({"command": cmd, "chat_id": cid}));
        assert_eq!(out["ok"], true);
    }
    let st = h.expect_ok("fabushi.get_status", json!({}));
    assert_eq!(st["active_tabs"].as_array().expect("7 tabs").len(), 7);
}

#[test]
fn tc_t3_pair_23_f16_x_f20_muse_inspector_updates_coding_bot_and_probes_remote_host() {
    let h = FabushiE2EHarness::connect_production();
    let patched = h.expect_ok(
        "fabushi.bots.patch",
        json!({"bot_id": "coding_bot_1", "coding_workspace_path": "/srv/work/ios", "coding_permission_mode": "ask"}),
    );
    assert_eq!(patched["coding_workspace_path"], "/srv/work/ios");
    let probe = h.expect_ok(
        "coding.probe_host",
        json!({"host_node_id": "node_dev_mac", "providers": ["claude", "codex", "acp"]}),
    );
    assert_eq!(probe["host_online"], true);
}

#[test]
fn tc_t3_pair_24_f16_x_f22_muse_inspector_binds_bot_computer_and_opens_pty() {
    let h = FabushiE2EHarness::connect_production();
    let bound = h.expect_ok(
        "fabushi.bots.bind_computer",
        json!({"bot_id": "bot_default", "computer_node_id": "node_dev_mac", "run_policy": "ask"}),
    );
    assert_eq!(bound["bound"], true);
    let pty = h.expect_ok("remote.pty_open", json!({"node_id": "node_dev_mac", "cols": 100, "rows": 30}));
    let pid = pty["pty_id"].as_str().expect("pty_id");
    let _ = h.expect_ok("remote.pty_close", json!({"pty_id": pid}));
}

#[test]
fn tc_t3_pair_25_f19_x_f33_magpie_probe_obeys_proxy_fail_closed_no_leak() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "proxy.set",
        json!({"proxy_type": "socks5", "host": "127.0.0.1", "port": 1, "fallback_to_direct": false}),
    );
    let _ = h.expect_ok(
        "magpie.provider_upsert",
        json!({"id": "prov_proxy", "protocol": "openai_chat", "base_url": format!("{}/api/v1/magpie/v1", h.server_url()), "api_key": "sk-p25", "models": ["gpt-5"]}),
    );
    h.server().clear_recorded_requests();
    let err = h.expect_err(
        "magpie.provider_probe",
        json!({"id": "prov_proxy"}),
        &["proxy_unreachable", "provider_probe_failed"],
    );
    assert!(err.retryable);
    assert_eq!(h.server().request_count_for_path("/api/v1/magpie"), 0);
}

#[test]
fn tc_t3_pair_26_f17_x_f21_agent_studio_enforces_official_mcp_scopes_and_audit() {
    let h = FabushiE2EHarness::connect_production();
    // F17 — Agent Studio verifies 7 tabs, canonical 29 toolsets (including `mcp`), enables `mcp`, and issues value-bound approval.
    let status = h.expect_ok("fabushi.get_status", json!({}));
    assert_eq!(status["active_tabs"].as_array().expect("7 tabs").len(), 7);
    let toolset_ids: Vec<String> = h.expect_ok("fabushi.list_toolsets", json!({}))["toolsets"]
        .as_array()
        .expect("toolsets")
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect();
    verify_29_toolsets(&toolset_ids).expect("29 canonical toolsets including mcp");
    let toggled = h.expect_ok(
        "fabushi.set_toolsets",
        json!({"toolsets": [{"id": "mcp", "enabled": true}]}),
    );
    assert!(toggled["enabled_count"].as_u64().expect("enabled_count") >= 1);
    let appr = h.expect_ok(
        "fabushi.approvals_decide",
        json!({
            "approval_id": "pair26_mcp_appr",
            "decision": "approve_once",
            "action": "mcp_tool_invoke",
            "target": "keyboard_type",
            "cwd": "/workspace",
            "content_sha256": sha256_hex(b"keyboard_type:node-alpha")
        }),
    );
    assert_eq!(appr["status"], "approved");

    // F21 — Official OAuth 2.1 Remote MCP exposes >=27 tools, enforces scope & redacts audit entry <= 280 chars.
    let tools = h.expect_ok("mcp.tools_list", json!({}));
    assert!(tools["tools"].as_array().expect("tools").len() >= 27);
    let args = json!({"text": "StudioAuditSecret"});
    let invoked = h.expect_ok(
        "mcp.tool_invoke",
        json!({
            "access_token": "tok_mcp_input_control",
            "protocol_version": "2025-11-25",
            "tool_name": "keyboard_type",
            "arguments": args,
            "node_id": "node-alpha"
        }),
    );
    assert_eq!(invoked["ok"], true);
    let audit = h.expect_ok("mcp.audit_list", json!({"limit": 5}));
    verify_mcp_audit_redaction(&args, audit["entries"][0]["redacted_args"].as_str().expect("redacted_args"))
        .expect("redacted");
}

#[test]
fn tc_t3_pair_27_f18_x_f20_multi_agent_coding_turn_with_workspace_cas_and_preemption() {
    let h = FabushiE2EHarness::connect_production();
    let w = h.expect_ok(
        "multi_agent.workspace_set",
        json!({"chat_id": 627, "key": "branch", "value": "feat/p27", "author_id": "coding_bot_1", "expected_revision": 0}),
    );
    assert_eq!(w["revision"], 1);
    let turn = h.expect_ok("coding.turn_start", json!({"chat_id": 627, "bot_id": "coding_bot_1", "prompt": "Apply fix"}));
    assert!(!turn["turn_id"].as_str().expect("turn_id").is_empty());

    let preempt = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": 627, "agent_id": "human", "priority": "user_interruption", "now_ms": now_ms()}),
    );
    assert_eq!(preempt["granted"], true);
}

#[test]
fn tc_t3_pair_28_f19_x_f23_magpie_oauth_syncs_via_zero_knowledge_vault_never_local_fs() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "vault.item_upsert",
        json!({"category": "ai_oauth", "item_key": "claude_code_oauth", "ciphertext_b64": B64.encode([0x77u8; 64]), "nonce_b64": B64.encode([0x05u8; 12])}),
    );
    let synced = h.expect_ok(
        "magpie.subscriptions_vault_sync",
        json!({"vault_category": "ai_oauth", "item_key": "claude_code_oauth"}),
    );
    assert_eq!(synced["local_fs_scan_supported"], false);
}

#[test]
fn tc_t3_pair_29_f21_x_f22_mcp_session_controlled_banner_and_sub_10ms_kill_switch() {
    let h = FabushiE2EHarness::connect_production();
    let _grant = h.expect_ok(
        "mcp.oauth_approve",
        json!({
            "authorize_code_or_qr_id": "qr_p29",
            "approved_scopes": ["devices:read", "input:control"],
            "allowed_node_ids": ["node_dev_mac"],
            "pkce_code_challenge": "chal_p29"
        }),
    );
    let _ = h.expect_ok("computers.lease_acquire", json!({"node_id": "node_dev_mac", "holder_kind": "ai_mcp"}));
    let sess = h.expect_ok("remote.sessions_list", json!({}));
    verify_controlled_banner_hex(
        sess["banner_variant"].as_str().expect("var"),
        sess["banner_hex"].as_str().expect("hex"),
    )
    .expect("canonical teal/indigo hex");

    let t0 = Instant::now();
    let tk = h.expect_ok("computers.emergency_takeover", json!({"node_id": "node_dev_mac", "holder_kind": "human"}));
    assert!(
        t0.elapsed().as_millis() < 50
            && tk["elapsed_ms"].as_u64().expect("elapsed_ms") < 10
    );
    let killed = h.expect_ok("mcp.kill_switch", json!({"reason": "emergency_takeover"}));
    assert!(
        killed["revoked_families"]
            .as_u64()
            .expect("revoked_families")
            >= 1
    );
}

#[test]
fn tc_t3_pair_30_f22_x_f30_remote_deep_link_routes_and_reports_honest_apns_stub() {
    let h = FabushiE2EHarness::connect_production();
    let routed = h.expect_ok("actions.route_url", json!({"url": "fabushi://remote/node_dev_mac"}));
    assert_eq!(routed["scheme"], "fabushi");
    let _ = h.expect_ok("remote.assist_set_code", json!({"assist_code": "PAIR-3030"}));
    let _ = h.expect_ok("remote.assist_allow", json!({"allowed": true}));
    let info = h.expect_ok("remote.assist_info", json!({}));
    assert_eq!(info["allowed"], true);
    assert_eq!(info["assist_code"], "PAIR-3030");
    let push = h.expect_ok(
        "push.register_token",
        json!({"platform": "apns", "device_token": "0123456789abcdef0123456789abcdef", "device_id": "p30", "environment": "sandbox"}),
    );
    assert_eq!(push["apns_delivery_capability"], "upstream_stub");
}

#[test]
fn tc_t3_pair_31_f24_x_f17_hermes_agent_invokes_webmcp_during_floating_pip_restore() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("miniapp.accept_tos", json!({"bot_id": "bot_dharma"}));
    let launch = h.expect_ok("miniapp.launch", json!({"bot_username": "dharma_bot", "short_name": "global_dharma"}));
    let inst = launch["instance_id"].as_str().expect("inst");
    let nonce = launch["nonce"].as_str().expect("nonce");
    let gen_val = launch["generation"].as_u64().expect("generation");

    let _ = h.expect_ok(
        "webmcp.register_tools",
        json!({
            "account_id": "acc_1",
            "app_id": "global_dharma",
            "version": "1.6.7",
            "digest": "sha256:abc123",
            "instance_id": inst,
            "nonce": nonce,
            "generation": gen_val,
            "tools": [{"name": "offline.practice.record", "mutating": true}]
        }),
    );

    let pip = h.expect_ok("miniapp.pip_minimize", json!({"instance_id": inst}));
    let rev_before = pip["preserved_revision"]
        .as_u64()
        .expect("pip_minimize preserved_revision");
    let status = h.expect_ok("fabushi.get_status", json!({}));
    assert_eq!(status["active_tabs"].as_array().expect("7 tabs").len(), 7);
    let inv = h.expect_ok(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global_dharma",
            "version": "1.6.7",
            "digest": "sha256:abc123",
            "instance_id": inst,
            "nonce": nonce,
            "generation": gen_val,
            "call_id": "p31_pip_call",
            "tool": "offline.practice.record",
            "arguments": {"tag": "agent_pip"},
            "approved": true
        }),
    );
    let rev = inv["state_revision"].as_u64().expect("rev");
    assert!(rev > rev_before);
    let restored = h.expect_ok("miniapp.pip_restore", json!({"instance_id": inst}));
    assert_eq!(restored["state"], "expanded");
    assert!(
        restored["preserved_revision"]
            .as_u64()
            .expect("restored preserved_revision")
            >= rev
    );
}

#[test]
fn tc_t3_pair_32_f24_x_f29_global_dharma_offline_free_vs_paid_storekit_entitlement() {
    let h = FabushiE2EHarness::connect_production();
    let pkg = b"GLOBAL_DHARMA_OFFLINE_P32_V167";
    let pkg_sha = sha256_hex(pkg);
    let verified = h.expect_ok(
        "miniapp.verify_offline_package",
        json!({
            "package_id": "global-dharma",
            "version": "1.6.7",
            "sha256_digest": pkg_sha,
            "ed25519_sig_b64": B64.encode([0x55u8; 64]),
            "bytes_b64": B64.encode(pkg)
        }),
    );
    assert_eq!(verified["verified"], true);
    assert_eq!(verified["offline_ready"], true);

    h.server().set_offline(true);
    let free = h.expect_ok(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global-dharma",
            "version": "1.6.7",
            "digest": pkg_sha,
            "instance_id": "p32_inst",
            "nonce": "p32_nonce",
            "generation": h.current_generation(),
            "call_id": "p32_free",
            "tool": "offline.state.read",
            "arguments": {}
        }),
    );
    assert_eq!(free["ok"], true);

    let paid_err = h.expect_err(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global-dharma",
            "version": "1.6.7",
            "digest": pkg_sha,
            "instance_id": "p32_inst",
            "nonce": "p32_nonce",
            "generation": h.current_generation(),
            "call_id": "p32_paid",
            "tool": "local.prayer-wheel.start",
            "arguments": {},
            "approved": true
        }),
        &["entitlement_required"],
    );
    assert!(!paid_err.retryable);
    let sk_err = h.expect_err(
        "wallet.storekit_verify_jws",
        json!({"signed_transaction_jws": "offline_jws", "require_live_apple_binding": true}),
        &["storekit_server_binding_blocked", "network_unavailable"],
    );
    assert!(!sk_err.message.is_empty());
    assert!(!sk_err.retryable || sk_err.code == "network_unavailable");
}

#[test]
fn tc_t3_pair_33_f26_x_f28_call_4_emoji_dtls_fingerprint_and_e2ee_replay_window() {
    let h = FabushiE2EHarness::connect_production();
    let sn = h.expect_ok("e2ee.safety_number", json!({"peer_user_id": 2033}));
    assert_eq!(sn["emojis"].as_array().expect("4 emojis").len(), 4);

    let call = h.expect_ok("calls.invite", json!({"peer_id": 2033, "kind": "audio"}));
    let sig = h.expect_ok(
        "calls.signal",
        json!({"call_id": call["call_id"], "signal_frame": {"type": "offer", "sdp": "v=0"}}),
    );
    assert_eq!(sig["emoji_fingerprint"].as_array().expect("4 emojis").len(), 4);

    let mut highest = 0u64;
    let mut bitmap = 0u128;
    assert!(verify_sliding_replay_window_128(&mut highest, &mut bitmap, 1));
    assert!(!verify_sliding_replay_window_128(&mut highest, &mut bitmap, 1));
}

#[test]
fn tc_t3_pair_34_f26_x_f34_calls_and_talk_mode_honest_governance_and_vad_bounds() {
    let h = FabushiE2EHarness::connect_production();
    let inv = h.expect_ok("calls.invite", json!({"peer_id": 2034, "kind": "audio"}));
    assert!(matches!(
        inv["transport_capability"]["nat_sfu_status"].as_str(),
        Some("upstream_loopback_stub") | Some("livekit_ios")
    ));
    let talk = h.expect_ok("talk.session_create", json!({}));
    let sid = talk["session_id"].as_str().expect("talk session_id");
    let loud: Vec<u8> = (0..320).flat_map(|_| 2400i16.to_le_bytes()).collect();
    let mut barge_in = false;
    for _ in 0..3 {
        let out = h.expect_ok("talk.vad_frame", json!({"session_id": sid, "pcm_i16_b64": B64.encode(&loud)}));
        barge_in = out["barge_in_triggered"].as_bool().unwrap_or(false);
    }
    assert!(barge_in);
    let ledger = h.expect_ok("settings.capability_ledger", json!({}));
    let items = ledger["items"].as_array().expect("items");
    verify_honest_capability_disposition(items).expect("honest capability disposition");
    assert!(items.iter().any(|i| i["id"] == "WEBRTC_NAT_SFU_TRANSPORT"));
    assert_eq!(verify_talk_vad_bargein(3, 0, 0).unwrap(), (true, false));
}

#[test]
fn tc_t3_pair_35_f28_x_f11_e2ee_envelope_survives_offline_outbox_with_nonce_dedup() {
    let h = FabushiE2EHarness::connect_production();
    let enc = h.expect_ok(
        "e2ee.encrypt_dm",
        json!({"peer_user_id": 2035, "peer_device_id": "dev_35", "plaintext_b64": B64.encode(b"e2ee offline")}),
    );
    let cid = h.expect_ok("chats.create", json!({"chat_type": 0, "peer_user_id": 2035}))["chat_id"]
        .as_i64()
        .expect("cid");

    h.server().set_offline(true);
    let queued = h.expect_ok(
        "outbox.enqueue",
        json!({"chat_id": cid, "content": {"type": "e2ee_envelope", "envelope": enc["envelope"]}, "client_nonce": 35001}),
    );
    assert_eq!(queued["status"], "queued");

    h.server().set_offline(false);
    let flushed = h.expect_ok("outbox.flush", json!({"chat_id": cid}));
    assert!(flushed["delivered_count"].as_u64().expect("delivered_count") >= 1);
}

#[test]
fn tc_t3_pair_36_f31_x_f32_theme_wcag_and_locale_persist_across_real_storage_purge() {
    let h = FabushiE2EHarness::connect_production();
    let th = h.expect_ok("settings.theme_set", json!({"mode": "classic", "accent_hex": "#FFFF00"}));
    verify_wcag_accent_luma_guard(
        "#FFFF00",
        th["resolved_palette"]["effective_accent_hex"].as_str().expect("hex"),
        th["resolved_palette"]["contrast_adjusted"].as_bool().expect("adj"),
    )
    .expect("WCAG");
    let _ = h.expect_ok("settings.locale_set", json!({"locale": "zh-Hans"}));
    let purged = h.expect_ok("settings.storage_purge", json!({"targets": ["temp_cache"]}));
    assert_eq!(purged["preserved_vault"], true);
    let after = h.expect_ok("settings.theme_get", json!({}));
    assert_eq!(after["mode"], "classic");
    assert_eq!(
        h.expect_ok("settings.get_all", json!({}))["language"]["locale"],
        "zh-Hans"
    );
}
