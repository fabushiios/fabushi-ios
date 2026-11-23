//! Tier 4 — 18 Multi-Feature Real-World Lifecycle Scenarios (`TC-T4-SCEN-01..18`).
//!
//! Each scenario combines >= 4 distinct features (100% 1:1 aligned with `feature_matrix.json`,
//! covering all 34 features F01..F34 across the 18 scenarios), exercising cold restarts
//! (`restart_cold`), real loopback HTTP/WS traffic, `fabushi_host_poll_events`,
//! `fabushi_host_cancel`, and strict fail-closed governance assertions.
//! Every test calls `FabushiE2EHarness::connect_production()` (zero mocks, zero synthetic stubs).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use fabushi_e2e_ffi::FabushiE2EHarness;
use fabushi_e2e_ffi::contracts::{
    DEFAULT_SERVER_URL_V167, ImeEvent, ImeState, TotpAlgorithm, assert_no_sample_chats,
    assert_not_hardcoded_localhost, encode_waveform_5bit, verify_26_settings_sections,
    verify_29_toolsets, verify_argon2id_params, verify_config_4tier_cascade,
    verify_controlled_banner_hex, verify_honest_capability_disposition, verify_ime_commit_fence,
    verify_loop_budget_5_hops, verify_magpie_circuit_rest_ms, verify_mcp_audit_redaction,
    verify_rfc6238_totp, verify_sliding_replay_window_128, verify_talk_vad_bargein,
    verify_waveform_5bit_63bytes, verify_wcag_accent_luma_guard,
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
fn tc_t4_scen_01_onboarding_split_key_v2_vault_first_chat_and_cold_restart() {
    // Features: F01, F02, F07, F11, F23
    let mut h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("auth.email.send_code", json!({"email": "scen01@fabushi.test"}));
    verify_argon2id_params(65536, 3, 4, 16, 32).expect("Argon2id bounds");
    let reg = h.expect_ok(
        "auth.email.register",
        json!({
            "email": "scen01@fabushi.test",
            "code": "123456",
            "username": "scen01_alice",
            "auth_version": 2,
            "auth_salt_b64": B64.encode([0x11u8; 16]),
            "auth_verifier_b64": B64.encode([0x22u8; 32]),
            "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}
        }),
    );
    let uid = reg["user_id"].as_u64().expect("uid");
    let kdf = h.expect_ok("auth.get_kdf_params", json!({"identifier": "scen01_alice"}));
    assert_eq!(kdf["auth_version"], 2);

    let _ = h.expect_ok(
        "vault.item_upsert",
        json!({"category": "password", "item_key": "github:scen01", "ciphertext_b64": B64.encode([0xAAu8; 48]), "nonce_b64": B64.encode([0x01u8; 12])}),
    );
    let totp = h.expect_ok(
        "vault.totp_generate",
        json!({"secret_or_otpauth": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", "unix_secs": 59, "digits": 6, "period_secs": 30, "algorithm": "SHA1"}),
    );
    verify_rfc6238_totp("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59, 30, 6, TotpAlgorithm::Sha1, totp["code"].as_str().expect("code")).expect("287082");

    let _ = h.expect_ok(
        "settings.set_shell_state",
        json!({"active_panel": "chats", "sidebar_width": 288.0, "form_factor": "ipad", "horizontal_size_class": "regular"}),
    );
    let nav = h.expect_ok("actions.navigate", json!({"route": "chats"}));
    assert_eq!(nav["active_panel"], "chats");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Onboarding Chat"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok("messages.send", json!({"chat_id": cid, "text": "Hello v1.6.7", "client_nonce": 401001}));
    let diff = h.expect_ok("sync.get_difference", json!({"chat_id": cid, "pts": 0, "limit": 10}));
    assert_eq!(diff["snapshot_required"], false);

    h.restart_cold();
    let restored = h.expect_ok("session.current", json!({}));
    assert_eq!(restored["user_id"].as_u64(), Some(uid));
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    assert!(!hist["messages"].as_array().expect("messages").is_empty());
    let pulled = h.expect_ok("vault.items_pull", json!({}));
    assert!(
        pulled["items"]
            .as_array()
            .expect("vault items")
            .iter()
            .any(|i| i["item_key"] == "github:scen01")
    );
    let outbox = h.expect_ok("outbox.list", json!({"chat_id": cid}));
    assert!(
        outbox["items"].as_array().expect("outbox items").is_empty(),
        "online message must not remain queued in outbox after delivery"
    );
}

#[test]
fn tc_t4_scen_02_cross_device_passkey_qr_pair_x25519_vault_and_rogue_revoke() {
    // Features: F04, F05, F06, F23
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "auth.register",
        json!({"username": "scen02_owner", "auth_version": 2, "auth_salt_b64": B64.encode([0x12u8; 16]), "auth_verifier_b64": B64.encode([0x23u8; 32]), "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}}),
    );
    // F04 — Passkey & QR Pair
    let _ = h.expect_ok("auth.passkey.register", json!({"credential_id": "cred_s02", "public_key": B64.encode([0x33u8; 65])}));
    // F05 — Cloud 2FA
    let _ = h.expect_ok("auth.two_factor.set", json!({"new_password": "Scen02Cloud2FA!"}));

    let sec = h.spawn_secondary_host("iPad Pro M4");
    let init = sec.expect_ok("auth.pair.init", json!({"device_name": "iPad Pro M4"}));
    let _ = h.expect_ok("auth.pair.accept", json!({"pair_token": init["pair_token"]}));
    let pair_ok = sec.expect_ok("auth.pair.verify", json!({"pair_token": init["pair_token"], "cloud_password": "Scen02Cloud2FA!"}));
    assert_eq!(pair_ok["verified"], true);

    // F23 — X25519 Cross-Device Vault Sync
    let sync_req = sec.expect_ok(
        "vault.device_sync_request",
        json!({"device_name": "iPad Pro M4", "ephemeral_x25519_pub_b64": B64.encode([0x44u8; 32])}),
    );
    let _ = h.expect_ok("vault.device_sync_approve", json!({"request_id": sync_req["request_id"], "wrapped_key_b64": B64.encode([0x55u8; 48])}));
    assert_eq!(sec.expect_ok("vault.device_sync_status", json!({"request_id": sync_req["request_id"]}))["status"], "approved");

    // F05 — Unconditional 2FA challenge -> verify -> revoke rogue session (zero silent-skip!)
    let rogue = h.spawn_secondary_host("Rogue Device");
    let rogue_gate = rogue.expect_ok(
        "auth.login",
        json!({"username": "scen02_owner", "password": "", "auth_verifier_b64": B64.encode([0x23u8; 32])}),
    );
    assert_eq!(rogue_gate["requires_2fa"], true);
    let challenge = rogue_gate["challenge_token"].as_str().expect("2FA challenge_token");
    let rogue_sess = rogue.expect_ok(
        "auth.two_factor.verify",
        json!({"challenge_token": challenge, "cloud_password": "Scen02Cloud2FA!", "device_name": "Rogue Device"}),
    );
    let sid = rogue_sess["session_id"].as_str().expect("rogue session_id");
    let rev = h.expect_ok("session.revoke_device", json!({"session_id": sid}));
    assert_eq!(rev["revoked"], true);
    let revoked_err = rogue.expect_err("chats.list", json!({}), &["unauthorized"]);
    assert!(!revoked_err.retryable);

    // F06 — Local Passcode App Lock & Monotonic Generation Fence
    let g_before = h.expect_ok("generation.current", json!({}))["generation"].as_u64().expect("g_before");
    let bumped = h.expect_ok("generation.bump", json!({"reason": "scen02_rogue_revoked"}));
    assert_eq!(bumped["generation"].as_u64(), Some(g_before + 1));
    let _ = h.expect_ok("session.passcode.set", json!({"passcode": "4321", "auto_lock_seconds": 60}));
    let _ = h.expect_ok("session.passcode.lock_now", json!({}));
    let locked_err = h.expect_err("chats.list", json!({}), &["app_locked"]);
    assert!(!locked_err.retryable);
    let unlocked = h.expect_ok("session.passcode.verify", json!({"passcode": "4321"}));
    assert_eq!(unlocked["unlocked"], true);
}

#[test]
fn tc_t4_scen_03_multi_account_isolation_stale_generation_and_idle_passcode_lock() {
    // Features: F06, F09, F11, F31, F34
    let h = FabushiE2EHarness::connect_production();
    let acc_a = h.expect_ok("session.add_account", json!({"username": "s03_a", "display_name": "A", "token": "ta"}));
    let acc_b = h.expect_ok("session.add_account", json!({"username": "s03_b", "display_name": "B", "token": "tb"}));
    let _ = h.expect_ok("session.switch_account", json!({"account_id": acc_a["account_id"]}));
    let _ = h.expect_ok("settings.theme_set", json!({"mode": "night"}));
    let _ = h.expect_ok("settings.locale_set", json!({"locale": "zh-Hans"}));
    let cid_a = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Account A Secret"}))["chat_id"].as_i64().expect("cid");
    let _ = h.expect_ok("chats.pin_dialog", json!({"chat_id": cid_a, "pinned": true}));
    let g_a = h.current_generation();

    let switched = h.expect_ok("session.switch_account", json!({"account_id": acc_b["account_id"]}));
    assert!(switched["generation"].as_u64().expect("switched generation") > g_a);
    let _ = h.expect_ok("settings.theme_set", json!({"mode": "day"}));
    let _ = h.expect_ok("settings.locale_set", json!({"locale": "en"}));

    let stale = h.expect_err_with_gen("messages.send", g_a, acc_a["account_id"].as_str(), json!({"chat_id": cid_a, "text": "late", "client_nonce": 40301}), &["stale_generation"]);
    assert!(!stale.retryable);
    let stale_sync = h.expect_err_with_gen("sync.get_difference", g_a, acc_a["account_id"].as_str(), json!({"chat_id": cid_a, "pts": 0}), &["stale_generation"]);
    assert!(!stale_sync.retryable);

    let _ = h.expect_ok("session.passcode.set", json!({"passcode": "8642", "auto_lock_seconds": 60}));
    let _ = h.expect_ok("session.passcode.lock_now", json!({}));
    let _ = h.expect_err("session.passcode.verify", json!({"passcode": "0000"}), &["invalid_passcode"]);
    let _ = h.expect_ok("session.passcode.verify", json!({"passcode": "8642"}));
    let b_diff = h.expect_ok("sync.get_difference", json!({"chat_id": 1, "pts": 0}));
    assert_eq!(b_diff["snapshot_required"], false);

    let b_chats = h.expect_ok("chats.list", json!({}));
    assert_no_sample_chats(&b_chats).expect("no sample chats");
    assert!(!b_chats["chats"].as_array().expect("chats").iter().any(|c| c["title"] == "Account A Secret"));

    // F34 — Fail-closed unknown_command & honest capability ledger
    let e_unk = h.expect_err("nonexistent.scen03_governance", json!({}), &["unknown_command"]);
    assert!(!e_unk.retryable);
    let ledger = h.expect_ok("settings.capability_ledger", json!({}));
    verify_honest_capability_disposition(ledger["items"].as_array().expect("items")).expect("honest ledger");
}

#[test]
fn tc_t4_scen_04_grok_multi_select_multi_agent_room_and_human_interruption() {
    // Features: F08, F10, F12, F15, F18
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("contacts.resolve_peer", json!({"username": "机器人之父"}));
    let pre = h.expect_ok("chats.prestart_bot", json!({"bot_username": "helper_bot"}));
    assert_eq!(pre["virtual"], true);

    let room = h.expect_ok(
        "chats.compose_send",
        json!({
            "client_nonce": "s04_room",
            "recipients": [{"user_id": 5201, "is_bot": true}, {"user_id": 5202, "is_bot": true}],
            "is_group_mode": true,
            "custom_group_title": "AI Council",
            "initial_text": "Plan v1.6.7 architecture",
            "message_nonce": 40401
        }),
    );
    let cid = room["chat"]["chat_id"].as_i64().expect("cid");
    let _ = h.expect_ok("multi_agent.workspace_set", json!({"chat_id": cid, "key": "plan", "value": "v1", "author_id": "bot_5201", "expected_revision": 0}));
    for hop in 1..=5u32 {
        let rec = h.expect_ok("multi_agent.loop_budget_record", json!({"chat_id": cid, "agent_id": format!("bot_{hop}"), "max_hops": 5}));
        verify_loop_budget_5_hops(hop, 5, rec["exhausted"].as_bool().unwrap_or(true)).expect("hop ok");
    }
    let _ = h.expect_err("multi_agent.loop_budget_record", json!({"chat_id": cid, "agent_id": "bot_6", "max_hops": 5}), &["loop_budget_exceeded"]);

    let preempt = h.expect_ok("multi_agent.floor_request", json!({"chat_id": cid, "agent_id": "human", "priority": "user_interruption", "now_ms": now_ms()}));
    assert_eq!(preempt["granted"], true);

    // F12 — Human override transcript send & history verification
    let _ = h.expect_ok("messages.send", json!({"chat_id": cid, "text": "Human override decision", "client_nonce": 40402}));
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    assert!(!hist["messages"].as_array().expect("messages").is_empty());

    let cmd = h.expect_ok("fabushi.commands.execute", json!({"command": "/status", "chat_id": cid}));
    assert_eq!(cmd["ok"], true);
}

#[test]
fn tc_t4_scen_05_offline_rich_messaging_cjk_ime_cas_206_and_ttl_purge() {
    // Features: F11, F12, F13, F14, F25, F27
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "OfflineRich"}))["chat_id"].as_i64().expect("cid");
    assert_eq!(verify_ime_commit_fence(ImeState::Committed, ImeEvent::KeyEnter, 20).action, "none");
    let _ = h.expect_ok("messages.composer.ime_event", json!({"event": "commit", "committed": "离线语音测试"}));

    let samples: Vec<u8> = (0..100).map(|i| (i % 32) as u8).collect();
    let wf = encode_waveform_5bit(&samples);
    verify_waveform_5bit_63bytes(&samples, &wf).expect("63B");

    h.server().set_offline(true);
    let q = h.expect_ok("outbox.enqueue", json!({"chat_id": cid, "text": "离线消息scen05", "client_nonce": 40501}));
    assert_eq!(q["status"], "queued");

    h.server().set_offline(false);
    let flushed = h.expect_ok("outbox.flush", json!({"chat_id": cid}));
    assert!(flushed["delivered_count"].as_u64().expect("delivered_count") >= 1);

    // F12 & F13 — Verify flushed message in transcript history & edit it
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    let msgs = hist["messages"].as_array().expect("messages");
    assert!(!msgs.is_empty());
    let mid = msgs[0]["message_id"].as_i64().expect("mid");
    let edited = h.expect_ok("messages.edit", json!({"chat_id": cid, "message_id": mid, "text": "离线消息scen05_edited"}));
    assert_eq!(edited["text"], "离线消息scen05_edited");

    h.server().seed_media_blob("s05_media", vec![0x7Eu8; 4096]);
    let range = h.expect_ok("media.stream_range", json!({"file_id": "s05_media", "range_start": 0, "range_end": 1023}));
    assert_eq!(range["status"], 206);

    let _ = h.expect_ok("chats.set_ttl", json!({"chat_id": cid, "ttl_seconds": 60}));
    let search = h.expect_ok("search.query", json!({"q": "离线消息scen05", "chat_id": cid}));
    assert!(search["total"].as_u64().expect("search total") >= 1);
}

#[test]
fn tc_t4_scen_06_full_7_tab_hermes_fabushi_agent_studio_lifecycle() {
    // Features: F12, F15, F16, F17
    let h = FabushiE2EHarness::connect_production();
    // F16 — Muse Context Inspector bot chat profile & bot config patch
    let cid = h.expect_ok("chats.create", json!({"chat_type": 4, "title": "Studio Hermes Bot"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let profile = h.expect_ok("chats.inspector.profile", json!({"chat_id": cid}));
    assert_eq!(profile["is_bot"], true);
    let patched = h.expect_ok(
        "fabushi.bots.patch",
        json!({"bot_id": "bot_default", "persona": "Studio Operator", "model": "claude-opus-4-1"}),
    );
    assert_eq!(patched["persona"], "Studio Operator");

    // F15 — Slash commands & @ context reference badge extraction
    let badges = h.expect_ok(
        "fabushi.context.extract_badges",
        json!({"text": "Inspect @file:src/lib.rs and @git:HEAD"}),
    );
    assert_eq!(badges["badges"].as_array().expect("badges").len(), 2);
    let slash_out = h.expect_ok(
        "fabushi.commands.execute",
        json!({"command": "/kanban", "chat_id": cid}),
    );
    assert_eq!(slash_out["ok"], true);

    // F12 — Virtualized transcript card send & history roundtrip
    let sent = h.expect_ok(
        "messages.send",
        json!({
            "chat_id": cid,
            "content": {"type": "generative_card", "card_id": "s06_card", "title": "Studio Run", "kind": "agent_thinking", "payload": {"step": 1}},
            "client_nonce": 40601
        }),
    );
    assert!(sent["message_id"].as_i64().expect("message_id") > 0);
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    assert!(!hist["messages"].as_array().expect("messages").is_empty());

    // F17 — Full 7-Tab Agent Studio & 29 Canonical Toolsets
    let status = h.expect_ok("fabushi.get_status", json!({}));
    assert_eq!(status["active_tabs"].as_array().expect("7 tabs").len(), 7);
    let toolsets: Vec<String> = h.expect_ok("fabushi.list_toolsets", json!({}))["toolsets"]
        .as_array()
        .expect("toolsets")
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect();
    verify_29_toolsets(&toolsets).expect("29 toolsets");

    let board = h.expect_ok("fabushi.kanban_create_board", json!({"title": "Studio Lifecycle"}));
    let _ = h.expect_ok("fabushi.kanban_create_card", json!({"board_id": board["board_id"], "title": "Card 1", "column": "todo"}));
    let skill_md = "# Studio Skill\nVerify all invariants.\n";
    let _ = h.expect_ok("fabushi.skills_upsert", json!({"name": "studio_skill", "skill_md": skill_md}));
    let _ = h.expect_ok("fabushi.skills_approve", json!({"name": "studio_skill", "approved": true, "sha256": sha256_hex(skill_md.as_bytes())}));
    let _ = h.expect_ok("fabushi.memory_put", json!({"target": "memory", "content": "Bounded memory entry"}));
    let _ = h.expect_ok("fabushi.memory_put", json!({"target": "user", "content": "Bounded user entry"}));
    let cron = h.expect_ok("fabushi.cron_upsert", json!({"name": "nightly", "schedule": "0 3 * * *", "prompt": "check", "target_chat_id": cid, "enabled": true}));
    let _ = h.expect_ok("fabushi.cron_run_now", json!({"job_id": cron["job_id"]}));

    let cmd_sha = sha256_hex(b"ls -la");
    let appr = h.expect_ok("fabushi.approvals_decide", json!({"approval_id": "s06_appr", "decision": "approve_once", "action": "terminal_exec", "target": "ls -la", "cwd": "/srv", "content_sha256": cmd_sha}));
    assert_eq!(appr["status"], "approved");
    let exp = h.expect_ok("fabushi.sessions_export", json!({"session_id": "s06_sess", "format": "sharegpt"}));
    assert_eq!(exp["redacted"], true);
}

#[test]
fn tc_t4_scen_07_magpie_gateway_failover_pii_masking_and_vault_oauth_sync() {
    // Features: F17, F19, F23, F33
    let h = FabushiE2EHarness::connect_production();
    // F17 — Agent Studio status & reasoning toolset activation
    let status = h.expect_ok("fabushi.get_status", json!({}));
    assert_eq!(status["active_tabs"].as_array().expect("7 tabs").len(), 7);
    let toggled = h.expect_ok("fabushi.set_toolsets", json!({"toolsets": [{"id": "reasoning", "enabled": true}]}));
    assert!(toggled["enabled_count"].as_u64().expect("enabled_count") >= 1);

    // F23 — Zero-Knowledge Vault OAuth item & F19 Magpie sync (never local FS scan)
    let _ = h.expect_ok("vault.item_upsert", json!({"category": "ai_oauth", "item_key": "openai_oauth", "ciphertext_b64": B64.encode([0x31u8; 64]), "nonce_b64": B64.encode([0x02u8; 12])}));
    let synced = h.expect_ok("magpie.subscriptions_vault_sync", json!({"vault_category": "ai_oauth", "item_key": "openai_oauth"}));
    assert_eq!(synced["local_fs_scan_supported"], false);

    // F33 — Proxy fail-closed no-direct-leak check during Magpie provider probe, then clear back to direct
    let _ = h.expect_ok("magpie.provider_upsert", json!({"id": "s07_prov", "protocol": "openai_chat", "base_url": format!("{}/api/v1/magpie/v1", h.server_url()), "api_key": "sk-s07-probe", "models": ["gpt-5"]}));
    let _ = h.expect_ok("proxy.set", json!({"proxy_type": "socks5", "host": "127.0.0.1", "port": 1, "fallback_to_direct": false}));
    h.server().clear_recorded_requests();
    let proxy_err = h.expect_err("magpie.provider_probe", json!({"id": "s07_prov"}), &["proxy_unreachable", "provider_probe_failed"]);
    assert!(proxy_err.retryable);
    assert_eq!(h.server().request_count_for_path("/api/v1/magpie"), 0);
    let cleared = h.expect_ok("proxy.clear", json!({}));
    assert_eq!(cleared["active_proxy"]["proxy_type"], "direct");
    let cfg = h.expect_ok("settings.config_resolve", json!({}));
    assert_eq!(cfg["server_url"].as_str(), Some(DEFAULT_SERVER_URL_V167));

    // F19 — Magpie dotenv import, PII masking, circuit failover (429 -> 5000ms), streaming unmask
    let _ = h.expect_ok("magpie.import_dotenv", json!({"dotenv_text": "OPENAI_API_KEY=sk-s07-primary\nANTHROPIC_API_KEY=sk-ant-s07-backup\n"}));
    let _ = h.expect_ok("magpie.masking_put", json!({"mask_secrets": true, "mask_personal": true, "action": "mask"}));
    let _ = h.expect_ok("magpie.groups_upsert", json!({"group_id": "s07_grp", "strategy": "order", "members": [{"seat_id": "p1", "provider_id": "openai", "model": "gpt-5"}, {"seat_id": "p2", "provider_id": "anthropic", "model": "claude-opus-4-1"}]}));

    let r429 = h.expect_ok("magpie.circuit_record_failure", json!({"seat_id": "p1", "http_status": 429, "error_body": "rate limit"}));
    verify_magpie_circuit_rest_ms("rate_limit", 0, r429["rest_duration_ms"].as_u64().expect("ms")).expect("5000ms");

    let sel = h.expect_ok("magpie.route_select", json!({"group_id": "s07_grp", "intent_tier": "deep"}));
    assert_eq!(sel["selected_member"]["seat_id"], "p2");

    let masked = h.expect_ok("magpie.masking_preview", json!({"text": "Contact dev@fabushi.test"}));
    let mtext = masked["masked_text"].as_str().expect("masked");
    assert!(!mtext.contains("dev@fabushi.test"));
    let unmasked = h.expect_ok("magpie.masking_unmask_stream", json!({"chunks": [mtext]}));
    assert!(unmasked["unmasked_text"].as_str().expect("unmasked_text").contains("dev@fabushi.test"));
}

#[test]
fn tc_t4_scen_08_coding_agent_mobile_workflow_diff_checkpoint_and_pr() {
    // Features: F12, F16, F18, F20
    let h = FabushiE2EHarness::connect_production();
    // F12 & F16 — Real coding group chat, transcript message, and Muse Inspector profile + bot config patch
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Coding Council"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let profile = h.expect_ok("chats.inspector.profile", json!({"chat_id": cid}));
    assert_eq!(profile["title"], "Coding Council");
    let bot = h.expect_ok("coding.bot_create_nl", json!({"prompt": "Build iOS SwiftUI coder bot", "host_node_id": "node_dev_mac", "workspace_path": "/srv/ios", "permission_mode": "ask"}));
    let bid = bot["bot_id"].as_str().expect("bid");
    let patched = h.expect_ok("fabushi.bots.patch", json!({"bot_id": bid, "coding_workspace_path": "/srv/ios", "coding_permission_mode": "ask"}));
    assert_eq!(patched["coding_workspace_path"], "/srv/ios");

    // F18 — Multi-Agent floor control, shared workspace CAS revision & 5-hop loop budget
    let floor = h.expect_ok("multi_agent.floor_request", json!({"chat_id": cid, "agent_id": bid, "priority": "direct_mention", "now_ms": now_ms()}));
    assert_eq!(floor["granted"], true);
    let ws = h.expect_ok("multi_agent.workspace_set", json!({"chat_id": cid, "key": "target_branch", "value": "feat/hig-tokens", "author_id": bid, "expected_revision": 0}));
    assert_eq!(ws["revision"], 1);
    let hop = h.expect_ok("multi_agent.loop_budget_record", json!({"chat_id": cid, "agent_id": bid, "max_hops": 5}));
    verify_loop_budget_5_hops(1, 5, hop["exhausted"].as_bool().unwrap_or(true)).expect("hop 1 ok");

    // F20 — Coding host probe, turn start, permission approval, diff, checkpoint restore & PR
    let probe = h.expect_ok("coding.probe_host", json!({"host_node_id": "node_dev_mac", "providers": ["claude", "codex", "acp"]}));
    assert_eq!(probe["host_online"], true);

    let turn = h.expect_ok("coding.turn_start", json!({"chat_id": cid, "bot_id": bid, "prompt": "Add HIG theme tokens"}));
    let tid = turn["turn_id"].as_str().expect("tid");
    let _ = h.expect_ok("coding.permission_decide", json!({"turn_id": tid, "request_id": "p_s08", "decision": "allow_once"}));
    let diff = h.expect_ok("coding.diff_get", json!({"turn_id": tid}));
    assert!(!diff["files"].as_array().expect("diff files").is_empty());
    let _ = h.expect_ok("coding.checkpoint_restore", json!({"checkpoint_id": turn["checkpoint_id"]}));
    let pr = h.expect_ok("coding.pr_create", json!({"turn_id": tid, "title": "feat: HIG tokens", "body": "Verified", "base_branch": "main"}));
    assert!(pr["pr_number"].as_u64().expect("pr_number") > 0);

    // F12 — Post PR summary card into transcript and verify history roundtrip
    let _ = h.expect_ok("messages.send", json!({"chat_id": cid, "text": "PR created for HIG tokens", "client_nonce": 40801}));
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    assert!(!hist["messages"].as_array().expect("messages").is_empty());
}

#[test]
fn tc_t4_scen_09_official_mcp_oauth21_audit_redaction_and_sub_10ms_kill_switch() {
    // Features: F17, F21, F22, F34
    let h = FabushiE2EHarness::connect_production();
    // F17 — Agent Studio canonical 29 toolsets + value-bound approval for remote MCP session
    let toolset_ids: Vec<String> = h.expect_ok("fabushi.list_toolsets", json!({}))["toolsets"]
        .as_array()
        .expect("toolsets")
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect();
    verify_29_toolsets(&toolset_ids).expect("29 toolsets");
    let appr = h.expect_ok(
        "fabushi.approvals_decide",
        json!({"approval_id": "s09_mcp", "decision": "approve_once", "action": "mcp_session", "target": "node-alpha", "cwd": "/srv", "content_sha256": sha256_hex(b"node-alpha")}),
    );
    assert_eq!(appr["status"], "approved");

    // F21 — Official OAuth 2.1 MCP discovery, DCR, approve, audit redaction & scope enforcement
    let _ = h.expect_ok("mcp.oauth_discover", json!({}));
    let _ = h.expect_ok("mcp.oauth_register_client", json!({"client_name": "Scen09", "redirect_uris": ["fabushi://oauth/mcp"], "grant_types": ["authorization_code"]}));
    let grant = h.expect_ok("mcp.oauth_approve", json!({"authorize_code_or_qr_id": "qr_s09", "approved_scopes": ["devices:read", "screen:view"], "allowed_node_ids": ["node-alpha"], "pkce_code_challenge": "chal_s09"}));

    let raw_args = json!({"text": "SecretScen09"});
    let _ = h.expect_ok("mcp.tool_invoke", json!({"access_token": "tok_mcp_devices_read", "protocol_version": "2025-11-25", "tool_name": "capture_screen", "arguments": raw_args, "node_id": "node-alpha"}));
    let audit = h.expect_ok("mcp.audit_list", json!({"limit": 1}));
    verify_mcp_audit_redaction(&raw_args, audit["entries"][0]["redacted_args"].as_str().expect("red")).expect("redacted");

    let e_scope = h.expect_err("mcp.tool_invoke", json!({"access_token": "tok_only_devices_read", "protocol_version": "2025-11-25", "tool_name": "device_power_action", "arguments": {"action": "reboot"}, "node_id": "node-alpha"}), &["insufficient_scope"]);
    assert!(!e_scope.retryable);

    // F22 — UU-Remote Controlled Banner + <10ms Emergency Takeover
    let _ = h.expect_ok("computers.lease_acquire", json!({"node_id": "node-alpha", "holder_kind": "ai_mcp"}));
    let sess = h.expect_ok("remote.sessions_list", json!({}));
    verify_controlled_banner_hex(sess["banner_variant"].as_str().expect("v"), sess["banner_hex"].as_str().expect("h")).expect("banner hex");

    let start = Instant::now();
    let tk = h.expect_ok("computers.emergency_takeover", json!({"node_id": "node-alpha", "holder_kind": "human"}));
    assert!(start.elapsed().as_millis() < 50 && tk["elapsed_ms"].as_u64().expect("elapsed_ms") < 10);
    let rev = h.expect_ok("mcp.grant_revoke", json!({"family_id": grant["family_id"]}));
    assert_eq!(rev["revoked"], true);

    // F34 — Fail-closed governance: unknown_command + generation fence + honest capability ledger
    let g0 = h.expect_ok("generation.current", json!({}))["generation"].as_u64().expect("g0");
    let _ = h.expect_ok("generation.bump", json!({"reason": "s09_kill_switch"}));
    let e_stale = h.expect_err_with_gen("mcp.tools_list", g0, None, json!({}), &["stale_generation"]);
    assert!(!e_stale.retryable);
    let e_unk = h.expect_err("nonexistent.scen09_governance", json!({}), &["unknown_command"]);
    assert!(!e_unk.retryable);
    let ledger = h.expect_ok("settings.capability_ledger", json!({}));
    verify_honest_capability_disposition(ledger["items"].as_array().expect("items")).expect("honest ledger");
}

#[test]
fn tc_t4_scen_10_uu_remote_workbench_multi_pty_tailnet_and_file_resume() {
    // Features: F16, F22, F30, F33
    let h = FabushiE2EHarness::connect_production();
    // F33 — Network proxy state & canonical 4-tier server_url resolution
    let px = h.expect_ok("proxy.get", json!({}));
    assert_eq!(px["active_proxy"]["proxy_type"], "direct");
    let cfg = h.expect_ok("settings.config_resolve", json!({}));
    assert_eq!(cfg["server_url"].as_str(), Some(DEFAULT_SERVER_URL_V167));

    let _ = h.expect_ok("tailnet.register", json!({"device_name": "iPad Workbench"}));
    let _ = h.expect_ok("tailnet.netmap", json!({}));
    let _ = h.expect_ok("fabushi.bots.bind_computer", json!({"bot_id": "bot_default", "computer_node_id": "node_dev_mac"}));
    let routed = h.expect_ok("actions.route_url", json!({"url": "fabushi://remote/node_dev_mac"}));
    assert_eq!(routed["scheme"], "fabushi");

    let stream = h.expect_ok("remote.stream_start", json!({"node_id": "node_dev_mac", "display_id": 1, "privacy_screen": true}));
    let _ = h.expect_ok("remote.input_send", json!({"session_id": stream["session_id"], "mouse_mode": "cursor_lock"}));

    let p1 = h.expect_ok("remote.pty_open", json!({"node_id": "node_dev_mac", "cols": 120, "rows": 32}));
    let p2 = h.expect_ok("remote.pty_open", json!({"node_id": "node_dev_mac", "cols": 80, "rows": 24}));
    let _ = h.expect_ok("remote.pty_write", json!({"pty_id": p1["pty_id"], "data": "echo S10_PTY1\n"}));
    let _ = h.expect_ok("remote.pty_write", json!({"pty_id": p2["pty_id"], "data": "echo S10_PTY2\n"}));
    let out1 = h.expect_ok("remote.pty_read", json!({"pty_id": p1["pty_id"]}))["output"]
        .as_str()
        .expect("p1 output")
        .to_owned();
    let out2 = h.expect_ok("remote.pty_read", json!({"pty_id": p2["pty_id"]}))["output"]
        .as_str()
        .expect("p2 output")
        .to_owned();
    assert!(out1.contains("S10_PTY1") && !out1.contains("S10_PTY2"));
    assert!(out2.contains("S10_PTY2") && !out2.contains("S10_PTY1"));

    let xfer = h.expect_ok("remote.file_transfer_start", json!({"node_id": "node_dev_mac", "direction": "upload", "path": "/tmp/s10.dat", "conflict_policy": "overwrite", "offset_bytes": 0}));
    let resumed = h.expect_ok("remote.file_transfer_resume", json!({"transfer_id": xfer["transfer_id"], "offset_bytes": 4096}));
    assert!(resumed["bytes_transferred"].as_u64().expect("bytes_transferred") >= 4096);

    let tk = h.expect_ok("computers.emergency_takeover", json!({"node_id": "node_dev_mac", "holder_kind": "human"}));
    assert!(tk["override_epoch"].as_u64().expect("override_epoch") >= 1);
}

#[test]
fn tc_t4_scen_11_miniapp_tos_live_webmcp_pip_and_global_dharma_offline() {
    // Features: F17, F24, F29, F34
    let h = FabushiE2EHarness::connect_production();
    // F17 — Agent Studio runtime status & approval for WebMCP practice mutation
    let status = h.expect_ok("fabushi.get_status", json!({}));
    assert_eq!(status["active_tabs"].as_array().expect("7 tabs").len(), 7);
    let appr = h.expect_ok(
        "fabushi.approvals_decide",
        json!({"approval_id": "s11_webmcp", "decision": "approve_once", "action": "webmcp_invoke", "target": "offline.practice.record", "cwd": "/", "content_sha256": sha256_hex(b"s11")}),
    );
    assert_eq!(appr["status"], "approved");

    // F24 — External Mini App ToS gate, launch, live WebMCP in PiP, in-place restore & Global Dharma offline
    let e_tos = h.expect_err("miniapp.launch", json!({"bot_username": "dharma_bot", "short_name": "global_dharma"}), &["tos_not_accepted"]);
    assert!(!e_tos.retryable);
    let _ = h.expect_ok("miniapp.accept_tos", json!({"bot_id": "bot_dharma"}));
    let launched = h.expect_ok("miniapp.launch", json!({"bot_username": "dharma_bot", "short_name": "global_dharma"}));
    assert_eq!(launched["init_data_hmac_valid"], true);
    let inst = launched["instance_id"].as_str().expect("inst");
    let nonce = launched["nonce"].as_str().expect("nonce");
    let gen_val = launched["generation"].as_u64().expect("generation");

    let _ = h.expect_ok(
        "webmcp.register_tools",
        json!({
            "account_id": "acc_1", "app_id": "global_dharma", "version": "1.6.7", "digest": "sha256:abc123",
            "instance_id": inst, "nonce": nonce, "generation": gen_val,
            "tools": [{"name": "offline.state.read", "mutating": false}, {"name": "offline.practice.record", "mutating": true}]
        }),
    );
    let pip = h.expect_ok("miniapp.pip_minimize", json!({"instance_id": inst}));
    let rev_before = pip["preserved_revision"].as_u64().expect("pip preserved_revision");
    let inv = h.expect_ok("webmcp.invoke", json!({"account_id": "acc_1", "app_id": "global_dharma", "version": "1.6.7", "digest": "sha256:abc123", "instance_id": inst, "nonce": nonce, "generation": gen_val, "call_id": "s11_mut", "tool": "offline.practice.record", "arguments": {"tag": "s11"}, "approved": true}));
    let rev = inv["state_revision"].as_u64().expect("rev");
    assert!(rev > rev_before);
    let restored = h.expect_ok("miniapp.pip_restore", json!({"instance_id": inst}));
    assert!(restored["preserved_revision"].as_u64().expect("restored preserved_revision") >= rev);

    let pkg = b"GLOBAL_DHARMA_OFFLINE_WASM_V167";
    let pkg_sha = sha256_hex(pkg);
    let _ = h.expect_ok("miniapp.verify_offline_package", json!({"package_id": "global-dharma", "version": "1.6.7", "sha256_digest": pkg_sha, "ed25519_sig_b64": B64.encode([0x44u8; 64]), "bytes_b64": B64.encode(pkg)}));
    h.server().set_offline(true);
    let free = h.expect_ok("webmcp.invoke", json!({"account_id": "acc_1", "app_id": "global-dharma", "version": "1.6.7", "digest": pkg_sha, "instance_id": inst, "nonce": nonce, "generation": gen_val, "call_id": "s11_free", "tool": "offline.state.read", "arguments": {}}));
    assert_eq!(free["ok"], true);
    let e_ent = h.expect_err("webmcp.invoke", json!({"account_id": "acc_1", "app_id": "global-dharma", "version": "1.6.7", "digest": pkg_sha, "instance_id": inst, "nonce": nonce, "generation": gen_val, "call_id": "s11_paid", "tool": "local.prayer-wheel.start", "arguments": {}, "approved": true}), &["entitlement_required"]);
    assert!(!e_ent.retryable);

    // F29 — Economy / StoreKit 2 entitlement boundary
    let bal = h.expect_ok("wallet.balance", json!({}));
    assert!(bal["stars_balance"].as_i64().expect("stars_balance") >= 0);
    let e_sk = h.expect_err("wallet.storekit_verify_jws", json!({"signed_transaction_jws": "s11_jws", "require_live_apple_binding": true}), &["storekit_server_binding_blocked", "network_unavailable"]);
    assert!(!e_sk.message.is_empty());

    // F34 — Honest capability ledger & fail-closed unknown_command
    let ledger = h.expect_ok("settings.capability_ledger", json!({}));
    verify_honest_capability_disposition(ledger["items"].as_array().expect("items")).expect("honest ledger");
    let e_unk = h.expect_err("nonexistent.scen11_governance", json!({}), &["unknown_command"]);
    assert!(!e_unk.retryable);
}

#[test]
fn tc_t4_scen_12_e2ee_ratchet_sender_keys_call_fingerprint_and_talk_vad_barge_in() {
    // Features: F11, F12, F26, F28
    let h = FabushiE2EHarness::connect_production();
    // F28 — E2EE PreKeys, Double Ratchet 1:1 & 128-bit Sliding Replay Window
    let _ = h.expect_ok("e2ee.publish_prekeys", json!({"device_id": "s12_dev", "identity_key_b64": B64.encode([1u8; 32]), "signed_prekey_b64": B64.encode([2u8; 32]), "spk_signature_b64": B64.encode([3u8; 64]), "one_time_prekeys_b64": [B64.encode([4u8; 32])]}));
    let pt = B64.encode(b"E2EE Scen12");
    let enc = h.expect_ok("e2ee.encrypt_dm", json!({"peer_user_id": 2012, "peer_device_id": "dev_peer", "plaintext_b64": pt}));
    let dec = h.expect_ok("e2ee.decrypt_dm", json!({"peer_user_id": 2012, "peer_device_id": "dev_peer", "envelope": enc["envelope"]}));
    assert_eq!(dec["plaintext_b64"].as_str(), Some(pt.as_str()));

    let mut hi = 0u64;
    let mut bm = 0u128;
    assert!(verify_sliding_replay_window_128(&mut hi, &mut bm, 1));
    assert!(!verify_sliding_replay_window_128(&mut hi, &mut bm, 1));

    // F11 & F12 — Offline WAL outbox delivery of E2EE envelope, PTS sync & transcript history
    let cid = h.expect_ok("chats.create", json!({"chat_type": 0, "peer_user_id": 2012}))["chat_id"]
        .as_i64()
        .expect("cid");
    h.server().set_offline(true);
    let queued = h.expect_ok("outbox.enqueue", json!({"chat_id": cid, "content": {"type": "e2ee_envelope", "envelope": enc["envelope"]}, "client_nonce": 41201}));
    assert_eq!(queued["status"], "queued");
    h.server().set_offline(false);
    let flushed = h.expect_ok("outbox.flush", json!({"chat_id": cid}));
    assert!(flushed["delivered_count"].as_u64().expect("delivered_count") >= 1);
    let diff = h.expect_ok("sync.get_difference", json!({"chat_id": cid, "pts": 0, "limit": 10}));
    assert_eq!(diff["snapshot_required"], false);
    let _ = h.expect_ok("messages.send", json!({"chat_id": cid, "text": "Post-call follow-up", "client_nonce": 41202}));
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    assert!(!hist["messages"].as_array().expect("messages").is_empty());

    // F26 — Calls 4-emoji DTLS fingerprint + Talk Mode <60ms VAD barge-in
    let call = h.expect_ok("calls.invite", json!({"peer_id": 2012, "kind": "audio"}));
    let sig = h.expect_ok("calls.signal", json!({"call_id": call["call_id"], "signal_frame": {"type": "offer", "sdp": "v=0"}}));
    assert_eq!(sig["emoji_fingerprint"].as_array().expect("4 emojis").len(), 4);
    let _ = h.expect_ok("calls.hangup", json!({"call_id": call["call_id"]}));
    let _ = h.expect_ok("calls.rate", json!({"call_id": call["call_id"], "rating": 5}));

    let talk = h.expect_ok("talk.session_create", json!({}));
    let sid = talk["session_id"].as_str().expect("sid");
    let _ = h.expect_ok("talk.audio_enqueue", json!({"session_id": sid, "chunk_b64": B64.encode([0x10u8; 640])}));
    let loud: Vec<u8> = (0..320).flat_map(|_| 2400i16.to_le_bytes()).collect();
    let mut flushed_audio = false;
    for _ in 0..3 {
        let out = h.expect_ok("talk.vad_frame", json!({"session_id": sid, "pcm_i16_b64": B64.encode(&loud)}));
        flushed_audio = out["barge_in_triggered"].as_bool().unwrap_or(false) && out["buffered_chunks"].as_u64() == Some(0);
    }
    assert!(flushed_audio);
    assert_eq!(verify_talk_vad_bargein(3, 0, 0).unwrap(), (true, false));
}

#[test]
fn tc_t4_scen_13_media_stories_stickers_instant_view_and_offline_fts5_search() {
    // Features: F09, F12, F16, F25, F27
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "MediaHub"}))["chat_id"].as_i64().expect("cid");
    let _ = h.expect_ok("chats.pin_dialog", json!({"chat_id": cid, "pinned": true}));
    let _ = h.expect_ok("folders.create", json!({"title": "MediaFolder", "included_chat_ids": [cid], "excluded_chat_ids": []}));

    let data = vec![0x9Au8; 4096];
    let init = h.expect_ok("media.upload_init", json!({"filename": "story.jpg", "mime_type": "image/jpeg", "total_size": data.len(), "part_size": data.len(), "expected_sha256": sha256_hex(&data)}));
    let _ = h.expect_ok("media.upload_part", json!({"upload_id": init["upload_id"], "part_index": 0, "data_b64": B64.encode(&data)}));
    let up = h.expect_ok("media.upload_complete", json!({"upload_id": init["upload_id"]}));

    let story = h.expect_ok("stories.create", json!({"media_id": up["file_id"], "media_type": "photo", "caption": "24h story", "privacy": "contacts"}));
    let _ = h.expect_ok("stories.view", json!({"story_id": story["story_id"]}));
    let _ = h.expect_ok("stickers.install_set", json!({"set_id": "fabushi_pack_1"}));
    let photo_msg = h.expect_ok(
        "messages.send",
        json!({
            "chat_id": cid,
            "content": {"type": "photo", "file_id": up["file_id"], "width": 800, "height": 600},
            "client_nonce": 41300
        }),
    );
    assert!(photo_msg["message_id"].as_i64().expect("photo message_id") > 0);
    let sm = h.expect_ok("chats.shared_media", json!({"chat_id": cid, "subtab": "media"}));
    assert_eq!(sm["subtab"], "media");
    assert!(
        !sm["items"].as_array().expect("shared_media items").is_empty(),
        "seeded photo message must appear in chats.shared_media media subtab"
    );
    let _ = h.expect_ok("media.instant_view_fetch", json!({"url": format!("{}/iv/s13", h.server_url())}));
    let _ = h.expect_ok("messages.send", json!({"chat_id": cid, "text": "离线全文索引验证S13", "client_nonce": 41301}));
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 10}));
    assert!(!hist["messages"].as_array().expect("messages").is_empty());
    let _ = h.expect_ok("search.reindex_local", json!({}));

    h.server().set_offline(true);
    let offline = h.expect_ok("search.query", json!({"q": "离线全文索引验证S13", "offline_only": true}));
    assert_eq!(offline["engine"], "sqlite_fts5_offline");
    assert!(offline["total"].as_u64().expect("offline total") >= 1);
}

#[test]
fn tc_t4_scen_14_unified_commerce_stars_ledger_and_honest_storekit_ton_boundaries() {
    // Features: F03, F29, F32, F34
    let h = FabushiE2EHarness::connect_production();
    // F03 — Alipay mobile OAuth sign-in before commerce operations
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ios_oauth", "redirect_scheme": "fabushi"}));
    let state = started["state"].as_str().expect("state");
    let done = h.expect_ok(
        "auth.alipay.handle_deep_link",
        json!({"url": format!("fabushi://auth/alipay?state={state}&auth_code=s14_alipay"), "expected_attempt_id": started["attempt_id"]}),
    );
    assert_eq!(done["authenticated"], true);

    // F32 — 26-section Apple Settings (verifying `credits` & `premium` sections + settings snapshot)
    let ids: Vec<String> = h.expect_ok("settings.sections_list", json!({}))["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .collect();
    verify_26_settings_sections(&ids).expect("26 sections");
    let _ = h.expect_ok("settings.get_all", json!({}));

    // F29 — Stars ledger, gift debit, overdraft guard, redeem code, StoreKit 2 & TON boundaries
    let _ = h.expect_ok("wallet.topup", json!({"amount_stars": 200, "idempotency_key": "s14_topup"}));
    let gifted = h.expect_ok("wallet.send_gift", json!({"to_user_id": 2014, "amount_stars": 80, "gift_id": "star_1", "idempotency_key": "s14_gift"}));
    assert_eq!(gifted["stars_balance"], 120);
    let e_over = h.expect_err("wallet.send_stars", json!({"to_user_id": 2014, "amount_stars": 9999, "idempotency_key": "s14_over"}), &["insufficient_balance"]);
    assert!(!e_over.retryable);
    let redeemed = h.expect_ok("wallet.redeem_code", json!({"code": "SCEN14-PRO"}));
    assert_eq!(redeemed["redeemed"], true);
    let e_sk = h.expect_err("wallet.storekit_verify_jws", json!({"signed_transaction_jws": "jws", "require_live_apple_binding": true}), &["storekit_server_binding_blocked"]);
    assert!(!e_sk.retryable);
    let e_ton = h.expect_err("wallet.send_stars", json!({"currency": "TON", "onchain_sign": true, "to_user_id": 2014, "amount_stars": 1}), &["ton_signing_upstream_planned", "upstream_planned"]);
    assert!(!e_ton.retryable);

    // F34 — Honest upstream/server blocker ledger
    let ledger = h.expect_ok("settings.capability_ledger", json!({}));
    verify_honest_capability_disposition(ledger["items"].as_array().expect("items")).expect("honest capability ledger");
}

#[test]
fn tc_t4_scen_15_hig_4_themes_wcag_26_settings_and_real_storage_purge() {
    // Features: F07, F25, F31, F32
    let h = FabushiE2EHarness::connect_production();
    // F07 — Adaptive Apple Shell state & navigation
    let _ = h.expect_ok(
        "settings.set_shell_state",
        json!({"active_panel": "settings", "sidebar_width": 300.0, "form_factor": "ipad", "horizontal_size_class": "regular"}),
    );
    let nav = h.expect_ok("actions.navigate", json!({"route": "settings", "section": "local_storage"}));
    assert_eq!(nav["active_panel"], "settings");
    assert_eq!(nav["presentation_mode"], "split_3col");

    for mode in ["night", "tinted", "day", "classic"] {
        let th = h.expect_ok("settings.theme_set", json!({"mode": mode, "accent_hex": "#FFFF00"}));
        verify_wcag_accent_luma_guard("#FFFF00", th["resolved_palette"]["effective_accent_hex"].as_str().expect("h"), th["resolved_palette"]["contrast_adjusted"].as_bool().expect("a")).expect("WCAG 36%");
    }
    let _ = h.expect_ok("settings.accessibility_set", json!({"reduce_motion": true, "high_contrast": true, "dynamic_type_category": "UICTContentSizeCategoryAccessibilityExtraExtraExtraLarge"}));
    let _ = h.expect_ok("settings.locale_set", json!({"locale": "zh-Hans"}));
    let _ = h.expect_ok("settings.locale_set", json!({"locale": "en"}));

    let ids: Vec<String> = h.expect_ok("settings.sections_list", json!({}))["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .collect();
    verify_26_settings_sections(&ids).expect("26 sections");

    let blob = vec![0x6Bu8; 65_536];
    let init = h.expect_ok("media.upload_init", json!({"filename": "s15.bin", "mime_type": "application/octet-stream", "total_size": blob.len(), "part_size": blob.len()}));
    let _ = h.expect_ok("media.upload_part", json!({"upload_id": init["upload_id"], "part_index": 0, "data_b64": B64.encode(&blob)}));
    let _ = h.expect_ok("media.upload_complete", json!({"upload_id": init["upload_id"]}));

    let stats = h.expect_ok("settings.storage_stats", json!({}));
    assert!(stats["cas_media_bytes"].as_u64().expect("cas_media_bytes") >= 65_536);
    let purged = h.expect_ok("settings.storage_purge", json!({"targets": ["media_cache"]}));
    assert!(purged["freed_bytes"].as_u64().expect("freed_bytes") >= 65_536);
    assert_eq!(purged["preserved_messages"], true);
}

#[test]
fn tc_t4_scen_16_action_bus_app_intents_deep_links_and_honest_apns_tracking() {
    // Features: F03, F17, F30, F34
    let h = FabushiE2EHarness::connect_production();
    // F03 + F30 — Alipay OAuth callback routed through Action Bus `fabushi://` URL router
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ios_oauth", "redirect_scheme": "fabushi"}));
    let state = started["state"].as_str().expect("state");
    let alipay_link = format!("fabushi://auth/alipay?state={state}&auth_code=s16_code");
    let routed_alipay = h.expect_ok("actions.route_url", json!({"url": alipay_link}));
    assert_eq!(routed_alipay["scheme"], "fabushi");
    let done = h.expect_ok("auth.alipay.handle_deep_link", json!({"url": alipay_link, "expected_attempt_id": started["attempt_id"]}));
    assert_eq!(done["authenticated"], true);

    // F17 — Agent Studio 7-tab status & value-bound approval linked via `fabushi://studio/approvals`
    let studio = h.expect_ok("fabushi.get_status", json!({}));
    assert_eq!(studio["active_tabs"].as_array().expect("7 tabs").len(), 7);
    let appr = h.expect_ok(
        "fabushi.approvals_decide",
        json!({"approval_id": "s16_studio_appr", "decision": "approve_once", "action": "terminal_exec", "target": "uname", "cwd": "/srv", "content_sha256": sha256_hex(b"uname")}),
    );
    assert_eq!(appr["status"], "approved");

    // F30 — Action Bus App Intents, HMAC approval gate, canonical deep links & honest APNs registration
    let actions = h.expect_ok("actions.list", json!({}));
    assert!(!actions["actions"].as_array().expect("actions").is_empty());
    let gated = h.expect_ok("actions.invoke", json!({"action_name": "computer.power_reboot", "params": {"node_id": "n1"}, "source": "app_intent"}));
    let gid = gated["gate_id"].as_str().expect("gid");
    let gate_ok = h.expect_ok("actions.approve_gate", json!({"gate_id": gid, "hmac_signature": sha256_hex(gid.as_bytes())}));
    assert_eq!(gate_ok["status"], "approved");

    for link in ["fabushi://chat/10", "fabushi://studio/approvals", "fabushi://miniapp/bot/dharma"] {
        let routed = h.expect_ok("actions.route_url", json!({"url": link}));
        assert_eq!(routed["scheme"], "fabushi");
    }
    let reg = h.expect_ok("push.register_token", json!({"platform": "apns", "device_token": "abcdef0123456789abcdef0123456789", "device_id": "s16", "environment": "sandbox"}));
    assert_eq!(reg["apns_delivery_capability"], "upstream_stub");
    let unreg = h.expect_ok("push.unregister_token", json!({"platform": "apns", "device_id": "s16"}));
    assert_eq!(unreg["registered"], false);

    // F34 — Honest APNs upstream_stub ledger & fail-closed unknown_command
    let ledger = h.expect_ok("settings.capability_ledger", json!({}));
    verify_honest_capability_disposition(ledger["items"].as_array().expect("items")).expect("honest ledger");
    let e_unk = h.expect_err("nonexistent.scen16_governance", json!({}), &["unknown_command"]);
    assert!(!e_unk.retryable);
}

#[test]
fn tc_t4_scen_17_network_resilience_4_tier_cascade_proxy_no_leak_and_update_check() {
    // Features: F01, F11, F32, F33
    let h = FabushiE2EHarness::connect_production();
    let (exp_dft, exp_src) = verify_config_4tier_cascade(None, None, None);
    let dft = h.expect_ok("settings.config_resolve", json!({}));
    assert_eq!(dft["server_url"].as_str(), Some(exp_dft.as_str()));
    assert_eq!(dft["config_source"].as_str(), Some(exp_src));

    // F01 — Split-Key V2 registration + proxy-blocked login zero direct leak
    verify_argon2id_params(65536, 3, 4, 16, 32).expect("Argon2id bounds");
    let reg = h.expect_ok(
        "auth.register",
        json!({
            "username": "scen17_proxy_user",
            "auth_version": 2,
            "auth_salt_b64": B64.encode([0x17u8; 16]),
            "auth_verifier_b64": B64.encode([0x27u8; 32]),
            "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}
        }),
    );
    assert!(reg["user_id"].as_u64().expect("user_id") > 0);

    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "ProxyResilience"}))["chat_id"].as_i64().expect("cid");
    let _ = h.expect_ok("proxy.set", json!({"proxy_type": "http", "host": "127.0.0.1", "port": 1, "fallback_to_direct": false}));
    h.server().clear_recorded_requests();
    let _ = h.expect_err("proxy.probe", json!({"proxy_type": "http", "host": "127.0.0.1", "port": 1, "timeout_ms": 500}), &["proxy_unreachable"]);
    let e_login = h.expect_err(
        "auth.login",
        json!({"username": "scen17_proxy_user", "auth_verifier_b64": B64.encode([0x27u8; 32])}),
        &["proxy_unreachable", "network_unavailable"],
    );
    assert!(e_login.retryable);
    assert_eq!(h.server().request_count_for_path("/api/v1/auth/login"), 0);
    assert_eq!(h.server().recorded_requests().len(), 0);

    let _ = h.expect_ok("outbox.enqueue", json!({"chat_id": cid, "text": "Queued behind dead proxy", "client_nonce": 41701}));
    let _ = h.expect_ok("proxy.clear", json!({}));
    let flushed = h.expect_ok("outbox.flush", json!({"chat_id": cid}));
    assert!(flushed["delivered_count"].as_u64().expect("delivered_count") >= 1);

    // F32 — 26-section Apple Settings & storage stats verification
    let ids: Vec<String> = h.expect_ok("settings.sections_list", json!({}))["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .collect();
    verify_26_settings_sections(&ids).expect("26 sections");
    let stats = h.expect_ok("settings.storage_stats", json!({}));
    let _ = stats["total_bytes"].as_u64().expect("total_bytes");

    let upd = h.expect_ok("settings.update_check", json!({"current_version": "1.6.6", "channel": "testflight"}));
    assert_eq!(upd["binary_self_update_allowed"], false);
}

#[test]
fn tc_t4_scen_18_full_fail_closed_governance_panic_recovery_and_backlog_closure() {
    // Features: F06, F26, F29, F30, F34
    let h = FabushiE2EHarness::connect_production();
    // F06 — Multi-account generation bump & stale-generation fence
    let acc1 = h.expect_ok("session.add_account", json!({"username": "s18_a", "display_name": "A", "token": "t18a"}));
    let acc2 = h.expect_ok("session.add_account", json!({"username": "s18_b", "display_name": "B", "token": "t18b"}));
    let _ = h.expect_ok("session.switch_account", json!({"account_id": acc1["account_id"]}));
    let (req_id, resp, cur) = h.expect_ok_with_meta("generation.current", json!({}));
    assert_eq!(resp.id, req_id);
    let g_old = cur["generation"].as_u64().expect("g_old");
    let sw = h.expect_ok("session.switch_account", json!({"account_id": acc2["account_id"]}));
    assert!(sw["generation"].as_u64().expect("g_new") > g_old);
    let e_stale = h.expect_err_with_gen("chats.list", g_old, acc1["account_id"].as_str(), json!({}), &["stale_generation"]);
    assert!(!e_stale.retryable);

    // F34 — Strict unknown_command & strict catch_unwind `host_panic` (NEVER accept `unknown_command`!)
    let e_unk = h.expect_err("totally.bogus_method", json!({}), &["unknown_command"]);
    assert!(!e_unk.retryable);
    let e_panic = h.expect_err("debug.trigger_panic", json!({}), &["host_panic"]);
    assert!(!e_panic.retryable);
    assert!(!e_panic.message.is_empty());

    // Host handle remains alive; zero sample chats & zero hardcoded localhost
    let chats = h.expect_ok("chats.list", json!({}));
    assert_no_sample_chats(&chats).expect("zero sample_chats");

    let cfg = h.expect_ok("settings.config_resolve", json!({}));
    let url = cfg["server_url"].as_str().expect("url");
    assert_not_hardcoded_localhost(url).expect("zero 127.0.0.1:8443");
    assert_eq!(url, DEFAULT_SERVER_URL_V167);

    // F26 — Honest WebRTC NAT/SFU capability reporting on live call invite
    let inv = h.expect_ok("calls.invite", json!({"peer_id": 2018, "kind": "audio"}));
    assert!(matches!(
        inv["transport_capability"]["nat_sfu_status"].as_str(),
        Some("upstream_loopback_stub") | Some("livekit_ios")
    ));
    let _ = h.expect_ok("calls.hangup", json!({"call_id": inv["call_id"]}));

    // F29 — Honest StoreKit server binding blocker & TON upstream_planned fail-closed
    let e_sk = h.expect_err("wallet.storekit_verify_jws", json!({"signed_transaction_jws": "s18_jws", "require_live_apple_binding": true}), &["storekit_server_binding_blocked"]);
    assert!(!e_sk.retryable);
    let e_ton = h.expect_err("wallet.send_stars", json!({"currency": "TON", "onchain_sign": true, "to_user_id": 2018, "amount_stars": 1}), &["ton_signing_upstream_planned", "upstream_planned"]);
    assert!(!e_ton.retryable);

    // F30 — Honest APNs upstream_stub tracking & untrusted deep-link rejection
    let push = h.expect_ok("push.register_token", json!({"platform": "apns", "device_token": "0123456789abcdef0123456789abcdef", "device_id": "s18", "environment": "sandbox"}));
    assert_eq!(push["apns_delivery_capability"], "upstream_stub");
    let e_url = h.expect_err("actions.route_url", json!({"url": "javascript:alert(1)"}), &["invalid_url_scheme", "untrusted_url_route"]);
    assert!(!e_url.retryable);

    // F34 — Complete 7-entry honest capability ledger, event poll & non-discarded cancel
    let ledger = h.expect_ok("settings.capability_ledger", json!({}));
    verify_honest_capability_disposition(ledger["items"].as_array().expect("items")).expect("all 7 honest backlog dispositions");

    let polled = h.poll_events(16, 20);
    assert!(polled.ok);
    assert!(!h.cancel("s18_cancel_probe"));
}
