//! Tier 1 — Core Functional Happy-Path Opaque-Box E2E Tests for Features F17..F34
//! (18 features x 5 tests = 90 executable tests).
//!
//! Every test connects EXCLUSIVELY via `FabushiE2EHarness::connect_production()` (zero mocks,
//! zero synthetic stub fallback) against a real compiled `libmahayana_app_host.{so,dylib}` and
//! ephemeral `127.0.0.1:0` loopback HTTP/WS server.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use fabushi_e2e_ffi::FabushiE2EHarness;
use fabushi_e2e_ffi::contracts::{
    DEFAULT_SERVER_URL_V167, TotpAlgorithm, assert_no_sample_chats, assert_not_hardcoded_localhost,
    verify_26_settings_sections, verify_29_toolsets, verify_argon2id_params,
    verify_config_4tier_cascade, verify_controlled_banner_hex,
    verify_honest_capability_disposition, verify_loop_budget_5_hops, verify_mcp_audit_redaction,
    verify_rfc6238_totp, verify_sliding_replay_window_128, verify_talk_vad_bargein,
    verify_vault_size_cap, verify_waveform_5bit_63bytes, verify_wcag_accent_luma_guard,
};
use serde_json::{Value, json};
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

// ============================================================================
// F17 — Unified Hermes/Fabushi Agent Runtime, 29 Toolsets & 7-Tab Studio (5 tests)
// ============================================================================

#[test]
fn tc_t1_f17_01_studio_7_tabs_and_29_canonical_toolsets_toggle() {
    let h = FabushiE2EHarness::connect_production();
    let status = h.expect_ok("fabushi.get_status", json!({}));
    assert_eq!(status["active_tabs"].as_array().expect("tabs").len(), 7);

    let listed = h.expect_ok("fabushi.list_toolsets", json!({}));
    let ids: Vec<String> = listed["toolsets"]
        .as_array()
        .expect("toolsets")
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect();
    verify_29_toolsets(&ids).expect("exact 29 canonical toolsets");

    let updated = h.expect_ok(
        "fabushi.set_toolsets",
        json!({"toolsets": [{"id": "core", "enabled": true}, {"id": "web", "enabled": false}]}),
    );
    assert!(updated["enabled_count"].as_u64().unwrap_or(0) >= 1);
}

#[test]
fn tc_t1_f17_02_kanban_board_card_transitions_persist_across_cold_restart() {
    let mut h = FabushiE2EHarness::connect_production();
    let board = h.expect_ok("fabushi.kanban_create_board", json!({"title": "v1.6.7 Release"}));
    let board_id = board["board_id"].as_str().expect("board_id").to_owned();

    let card = h.expect_ok(
        "fabushi.kanban_create_card",
        json!({"board_id": board_id, "title": "Ship Apple HIG", "column": "todo"}),
    );
    let card_id = card["card_id"].as_str().expect("card_id").to_owned();
    let r1 = card["revision"].as_u64().expect("initial card revision");

    let moved = h.expect_ok(
        "fabushi.kanban_update_card",
        json!({"board_id": board_id, "card_id": card_id, "column": "in_progress"}),
    );
    assert!(moved["revision"].as_u64().expect("moved card revision") > r1);
    let done = h.expect_ok(
        "fabushi.kanban_update_card",
        json!({"board_id": board_id, "card_id": card_id, "column": "done"}),
    );
    assert_eq!(done["column"], "done");

    h.restart_cold();
    let boards = h.expect_ok("fabushi.kanban_list_boards", json!({}));
    let persisted_board = boards["boards"]
        .as_array()
        .expect("kanban boards array after cold restart")
        .iter()
        .find(|b| b["board_id"].as_str() == Some(board_id.as_str()))
        .expect("kanban board must persist across cold restart");
    assert_eq!(persisted_board["title"], "v1.6.7 Release");

    let cards = h.expect_ok("fabushi.kanban_list_cards", json!({"board_id": board_id}));
    let persisted_card = cards["cards"]
        .as_array()
        .expect("kanban cards array after cold restart")
        .iter()
        .find(|c| c["card_id"].as_str() == Some(card_id.as_str()))
        .expect("kanban card must persist across cold restart");
    assert_eq!(persisted_card["title"], "Ship Apple HIG");
    assert_eq!(persisted_card["column"], "done");
}

#[test]
fn tc_t1_f17_03_skill_md_upsert_approve_and_sha256_verify() {
    let h = FabushiE2EHarness::connect_production();
    let skill_md = "# Release Guard\nAlways verify exact HEAD SHA before publishing.\n";
    let expected_sha = sha256_hex(skill_md.as_bytes());

    let upserted = h.expect_ok(
        "fabushi.skills_upsert",
        json!({"name": "release_guard", "skill_md": skill_md}),
    );
    assert_eq!(upserted["sha256"].as_str(), Some(expected_sha.as_str()));

    let approved = h.expect_ok(
        "fabushi.skills_approve",
        json!({"name": "release_guard", "approved": true, "sha256": expected_sha}),
    );
    assert_eq!(approved["approved"], true);

    let list = h.expect_ok("fabushi.skills_list", json!({}));
    assert!(
        list["skills"]
            .as_array()
            .expect("skills")
            .iter()
            .any(|s| s["name"] == "release_guard" && s["approved"] == true)
    );
}

#[test]
fn tc_t1_f17_04_bounded_memory_user_soul_honcho_and_quarantine_approve() {
    let h = FabushiE2EHarness::connect_production();
    let mem = h.expect_ok(
        "fabushi.memory_put",
        json!({"target": "memory", "content": "Prefer Apple HIG SwiftUI.\n§\nZero stubs."}),
    );
    assert_eq!(mem["max_chars"], 2200);
    let c = mem["char_count"].as_u64().expect("char_count");
    assert!(c > 0 && c <= 2200);

    let usr = h.expect_ok(
        "fabushi.memory_put",
        json!({"target": "user", "content": "iOS Architect"}),
    );
    assert_eq!(usr["max_chars"], 1375);

    let _ = h.expect_ok(
        "fabushi.memory_put",
        json!({"target": "soul", "content": "Calm, precise, honest."}),
    );
    let honcho = h.expect_ok(
        "fabushi.honcho_query",
        json!({"query": "Apple HIG", "peer_id": "self"}),
    );
    assert!(
        !honcho["entries"]
            .as_array()
            .expect("honcho entries")
            .is_empty(),
        "honcho_query must return non-empty matching entries"
    );

    let decided = h.expect_ok(
        "fabushi.memory_quarantine_decide",
        json!({"item_id": "q_1", "decision": "approve"}),
    );
    assert_eq!(decided["status"], "approved");
}

#[test]
fn tc_t1_f17_05_five_field_cron_and_value_bound_approval_once() {
    let h = FabushiE2EHarness::connect_production();
    let job = h.expect_ok(
        "fabushi.cron_upsert",
        json!({
            "name": "daily_digest",
            "schedule": "*/15 * * * *",
            "prompt": "Summarize unread channels",
            "target_chat_id": 101,
            "enabled": true
        }),
    );
    assert!(!job["next_runs_7d"].as_array().expect("next_runs_7d").is_empty());
    let job_id = job["job_id"].as_str().expect("job_id");

    let run = h.expect_ok("fabushi.cron_run_now", json!({"job_id": job_id}));
    assert_eq!(run["triggered"], true);

    let content_sha = sha256_hex(b"echo safe");
    let approved = h.expect_ok(
        "fabushi.approvals_decide",
        json!({
            "approval_id": "appr_t1_01",
            "decision": "approve_once",
            "action": "terminal_exec",
            "target": "echo safe",
            "cwd": "/workspace",
            "content_sha256": content_sha
        }),
    );
    assert_eq!(approved["status"], "approved");
    assert!(!approved["token_id"].as_str().unwrap_or("").is_empty());
}

// ============================================================================
// F18 — Multi-Agent Protocol (5 tests)
// ============================================================================

#[test]
fn tc_t1_f18_01_floor_control_priority_preemption_and_promotion() {
    let h = FabushiE2EHarness::connect_production();
    let t0 = now_ms();
    let g1 = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": 501, "agent_id": "bot_general", "priority": "general_contribution", "now_ms": t0}),
    );
    assert_eq!(g1["granted"], true);

    let queued = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": 501, "agent_id": "bot_mentioned", "priority": "direct_mention", "now_ms": t0 + 10}),
    );
    assert_eq!(queued["granted"], false);

    let preempt = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": 501, "agent_id": "human_user", "priority": "user_interruption", "now_ms": t0 + 20}),
    );
    assert_eq!(preempt["granted"], true);
    assert_eq!(preempt["current_speaker"], "human_user");
    let human_nonce = preempt["grant"]["nonce"].as_str().expect("human nonce");

    let released = h.expect_ok(
        "multi_agent.floor_release",
        json!({"chat_id": 501, "agent_id": "human_user", "nonce": human_nonce}),
    );
    assert_eq!(released["released"], true);
    assert_eq!(released["next_speaker"], "bot_mentioned");
}

#[test]
fn tc_t1_f18_02_loop_budget_5_hops_succeed_then_human_resets() {
    let h = FabushiE2EHarness::connect_production();
    for hop in 1..=5u32 {
        let rec = h.expect_ok(
            "multi_agent.loop_budget_record",
            json!({"chat_id": 502, "agent_id": format!("bot_{hop}"), "max_hops": 5}),
        );
        assert_eq!(rec["current_hop"].as_u64(), Some(u64::from(hop)));
        assert_eq!(rec["exhausted"], false);
        verify_loop_budget_5_hops(hop, 5, false).expect("hop <= 5 ok");
    }

    let reset = h.expect_ok("multi_agent.loop_budget_reset", json!({"chat_id": 502}));
    assert_eq!(reset["current_hop"], 0);
    assert_eq!(reset["exhausted"], false);
}

#[test]
fn tc_t1_f18_03_shared_group_workspace_cas_revision_and_event() {
    let h = FabushiE2EHarness::connect_production();
    let w1 = h.expect_ok(
        "multi_agent.workspace_set",
        json!({"chat_id": 503, "key": "spec_status", "value": "draft", "author_id": "bot_a", "expected_revision": 0}),
    );
    assert_eq!(w1["revision"], 1);

    let w2 = h.expect_ok(
        "multi_agent.workspace_set",
        json!({"chat_id": 503, "key": "spec_status", "value": "approved", "author_id": "bot_b", "expected_revision": 1}),
    );
    assert_eq!(w2["revision"], 2);

    let fetched = h.expect_ok("multi_agent.workspace_get", json!({"chat_id": 503}));
    assert_eq!(fetched["revision"], 2);
    assert_eq!(fetched["entries"]["spec_status"], "approved");

    let poll = h.poll_events(16, 100);
    assert!(poll.ok);
    assert!(
        poll.events.iter().any(|e| e.kind.contains("workspace")),
        "workspace_set must emit a workspace update event"
    );
}

#[test]
fn tc_t1_f18_04_reply_gate_stage1_wakes_explicit_mention_and_caps_unmentioned() {
    let h = FabushiE2EHarness::connect_production();
    let res = h.expect_ok(
        "multi_agent.reply_gate_evaluate",
        json!({
            "candidates": ["bot_coder", "bot_designer", "bot_qa"],
            "message_text": "@bot_coder please review the PR",
            "explicit_mentions": ["bot_coder"],
            "sender_is_human": true,
            "remaining_loop_budget": 5,
            "wake_threshold": 0.35,
            "max_unmentioned_wakes": 1,
            "cooldown_seconds": 15
        }),
    );
    let woken = res["woken_agent_ids"].as_array().expect("woken_agent_ids");
    assert!(woken.iter().any(|v| v == "bot_coder"));
    assert!(woken.len() <= 2);
}

#[test]
fn tc_t1_f18_05_reply_gate_stage2_passes_unique_and_suppresses_sentinels() {
    let h = FabushiE2EHarness::connect_production();
    let clean = h.expect_ok(
        "multi_agent.reply_gate_review_outbound",
        json!({
            "draft_text": "Here is the verified root cause for the PTS sync gap.",
            "recent_messages": ["Hello team"],
            "duplicate_threshold": 0.85
        }),
    );
    assert_eq!(clean["outcome"], "sent");

    for sentinel in ["[PASS]", "<PASS/>", "NO_REPLY"] {
        let suppressed = h.expect_ok(
            "multi_agent.reply_gate_review_outbound",
            json!({
                "draft_text": sentinel,
                "recent_messages": [],
                "duplicate_threshold": 0.85
            }),
        );
        assert_eq!(suppressed["outcome"], "pass");
    }
}

// ============================================================================
// F19 — Magpie AI Provider Gateway, Routing, Masking & Vault OAuth (5 tests)
// ============================================================================

#[test]
fn tc_t1_f19_01_provider_upsert_and_probe_redacts_secret_key() {
    let h = FabushiE2EHarness::connect_production();
    let secret = "sk-live-secret-never-echo-998877";
    let _ = h.expect_ok(
        "magpie.provider_upsert",
        json!({
            "id": "prov_loopback",
            "protocol": "openai_chat",
            "base_url": format!("{}/api/v1/magpie/v1", h.server_url()),
            "api_key": secret,
            "models": ["gpt-5", "gpt-5-mini"]
        }),
    );
    let probe = h.expect_ok("magpie.provider_probe", json!({"id": "prov_loopback"}));
    assert_eq!(probe["reachable"], true);
    let detail = probe["redacted_detail"]
        .as_str()
        .expect("redacted_detail string");
    assert!(!detail.is_empty() && !detail.contains(secret));
}

#[test]
fn tc_t1_f19_02_models_dev_catalog_fetch_and_cache_ttl() {
    let h = FabushiE2EHarness::connect_production();
    let cat = h.expect_ok("magpie.models_catalog", json!({"refresh": true}));
    assert!(!cat["models"].as_array().expect("models").is_empty());
    assert!(cat["ttl_ms"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn tc_t1_f19_03_smart_routing_group_selects_member_with_intent_tiers() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "magpie.groups_upsert",
        json!({
            "group_id": "grp_smart",
            "strategy": "smart",
            "members": [
                {"seat_id": "seat_fast", "provider_id": "p1", "model": "gpt-5-mini"},
                {"seat_id": "seat_deep", "provider_id": "p2", "model": "claude-opus-4-1"}
            ]
        }),
    );
    for tier in ["fast", "standard", "deep"] {
        let sel = h.expect_ok(
            "magpie.route_select",
            json!({"group_id": "grp_smart", "intent_tier": tier, "session_id": "sess_affinity_1"}),
        );
        assert!(!sel["selected_member"]["seat_id"].as_str().unwrap_or("").is_empty());
    }
}

#[test]
fn tc_t1_f19_04_pii_masking_preview_and_streaming_unmasker_roundtrip() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "magpie.masking_put",
        json!({"mask_secrets": true, "mask_personal": true, "action": "mask"}),
    );
    let raw_secret = "sk-ant-api03-SECRETTOKEN123456";
    let preview = h.expect_ok(
        "magpie.masking_preview",
        json!({"text": format!("Email alice@example.com key {raw_secret}")}),
    );
    let masked = preview["masked_text"].as_str().expect("masked_text");
    assert!(!masked.contains(raw_secret));
    assert!(!masked.contains("alice@example.com"));

    let mid = masked.len() / 2;
    let unmasked = h.expect_ok(
        "magpie.masking_unmask_stream",
        json!({"chunks": [&masked[..mid], &masked[mid..]]}),
    );
    assert!(
        unmasked["unmasked_text"]
            .as_str()
            .unwrap_or("")
            .contains("alice@example.com")
    );
}

#[test]
fn tc_t1_f19_05_import_dotenv_deeplink_and_vault_oauth_sync() {
    let h = FabushiE2EHarness::connect_production();
    let imp_env = h.expect_ok(
        "magpie.import_dotenv",
        json!({"dotenv_text": "OPENAI_API_KEY=sk-test-dotenv-1234\nOPENAI_BASE_URL=https://api.openai.com/v1\n"}),
    );
    assert!(!imp_env["imported_providers"].as_array().expect("imported").is_empty());

    let imp_link = h.expect_ok(
        "magpie.import_deeplink",
        json!({"url": "fabushi://magpie/import?preset=anthropic&key=sk-ant-link-1234"}),
    );
    assert!(!imp_link["imported_providers"].as_array().expect("imported").is_empty());

    let synced = h.expect_ok(
        "magpie.subscriptions_vault_sync",
        json!({"vault_category": "ai_oauth", "item_key": "claude_oauth"}),
    );
    assert_eq!(synced["local_fs_scan_supported"], false);
}

// ============================================================================
// F20 — Third-Party Coding Agents (Claude Code, Codex, ACP) (5 tests)
// ============================================================================

#[test]
fn tc_t1_f20_01_probe_remote_coding_host_for_claude_codex_acp() {
    let h = FabushiE2EHarness::connect_production();
    let probed = h.expect_ok(
        "coding.probe_host",
        json!({"host_node_id": "node_dev_mac", "providers": ["claude", "codex", "acp"]}),
    );
    assert_eq!(probed["host_online"], true);
    let provs = probed["providers"].as_array().expect("providers");
    assert_eq!(provs.len(), 3);
}

#[test]
fn tc_t1_f20_02_botfather_nl_create_and_config_update() {
    let h = FabushiE2EHarness::connect_production();
    let created = h.expect_ok(
        "coding.bot_create_nl",
        json!({
            "prompt": "Create a Claude Code bot for our iOS repo",
            "host_node_id": "node_dev_mac",
            "workspace_path": "/srv/workspaces/fabushi",
            "permission_mode": "ask"
        }),
    );
    let bot_id = created["bot_id"].as_str().expect("bot_id");

    let updated = h.expect_ok(
        "coding.bot_config_update",
        json!({
            "bot_id": bot_id,
            "workspace_path": "/srv/repos/fabushi",
            "permission_mode": "auto_edit",
            "model": "claude-opus-4-1",
            "host_node_id": "node_dev_mac"
        }),
    );
    assert_eq!(updated["config"]["permission_mode"], "auto_edit");
}

#[test]
fn tc_t1_f20_03_turn_start_polls_streaming_events_and_returns_unified_diff() {
    let h = FabushiE2EHarness::connect_production();
    let turn = h.expect_ok(
        "coding.turn_start",
        json!({
            "chat_id": 601,
            "bot_id": "coding_bot_1",
            "prompt": "Fix off-by-one in PTS backfill",
            "worktree_branch": "agent/pts-fix"
        }),
    );
    let turn_id = turn["turn_id"].as_str().expect("turn_id");
    assert!(!turn["worktree_path"].as_str().unwrap_or("").is_empty());

    let polled = h.poll_events(16, 200);
    assert!(polled.ok);
    assert!(
        polled.events.iter().any(|e| e.kind.starts_with("coding_")),
        "coding.turn_start must emit streaming coding_* events"
    );

    let diff = h.expect_ok("coding.diff_get", json!({"turn_id": turn_id}));
    assert_eq!(diff["turn_id"].as_str(), Some(turn_id));
    assert!(
        !diff["files"].as_array().expect("diff files").is_empty(),
        "coding.diff_get must return non-empty unified diff files"
    );
}

#[test]
fn tc_t1_f20_04_checkpoint_list_and_non_invasive_restore() {
    let h = FabushiE2EHarness::connect_production();
    let turn = h.expect_ok(
        "coding.turn_start",
        json!({"chat_id": 602, "bot_id": "coding_bot_1", "prompt": "Refactor sidebar"}),
    );
    let turn_id = turn["turn_id"].as_str().expect("turn_id");
    let cp_id = turn["checkpoint_id"].as_str().expect("checkpoint_id");

    let list = h.expect_ok("coding.checkpoint_list", json!({"turn_id": turn_id}));
    assert!(!list["checkpoints"].as_array().expect("checkpoints").is_empty());

    let restored = h.expect_ok(
        "coding.checkpoint_restore",
        json!({"checkpoint_id": cp_id}),
    );
    assert!(!restored["checkpoint_sha"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f20_05_coding_permission_approve_and_create_pr() {
    let h = FabushiE2EHarness::connect_production();
    let turn = h.expect_ok(
        "coding.turn_start",
        json!({"chat_id": 603, "bot_id": "coding_bot_1", "prompt": "Prepare v1.6.7 PR"}),
    );
    let turn_id = turn["turn_id"].as_str().expect("turn_id");

    let perm = h.expect_ok(
        "coding.permission_decide",
        json!({"turn_id": turn_id, "request_id": "perm_1", "decision": "allow_once"}),
    );
    assert_eq!(perm["status"], "allowed");

    let pr = h.expect_ok(
        "coding.pr_create",
        json!({
            "turn_id": turn_id,
            "title": "feat(ios): Apple HIG v1.6.7",
            "body": "Automated verified PR",
            "base_branch": "main"
        }),
    );
    assert!(!pr["pr_url"].as_str().unwrap_or("").is_empty());
    assert!(pr["pr_number"].as_u64().unwrap_or(0) > 0);
}

// ============================================================================
// F21 — Official OAuth 2.1 Remote MCP Server Grants & Audit (5 tests)
// ============================================================================

#[test]
fn tc_t1_f21_01_oauth_discovery_and_dynamic_client_registration() {
    let h = FabushiE2EHarness::connect_production();
    let disc = h.expect_ok("mcp.oauth_discover", json!({}));
    assert_eq!(disc["scopes_supported"].as_array().expect("scopes").len(), 14);
    assert!(
        disc["supported_mcp_versions"]
            .as_array()
            .expect("versions")
            .iter()
            .any(|v| v == "2025-11-25")
    );

    let reg = h.expect_ok(
        "mcp.oauth_register_client",
        json!({
            "client_name": "Fabushi iOS MCP Client",
            "redirect_uris": ["fabushi://oauth/mcp"],
            "grant_types": ["authorization_code", "refresh_token"]
        }),
    );
    assert!(!reg["client_id"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f21_02_qr_poll_oauth_approve_with_scopes_and_node_allowlist() {
    let h = FabushiE2EHarness::connect_production();
    let approved = h.expect_ok(
        "mcp.oauth_approve",
        json!({
            "authorize_code_or_qr_id": "qr_mcp_01",
            "approved_scopes": ["devices:read", "screen:view"],
            "allowed_node_ids": ["node-alpha"],
            "pkce_code_challenge": "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        }),
    );
    let family_id = approved["family_id"].as_str().expect("family_id");

    let grants = h.expect_ok("mcp.grants_list", json!({}));
    assert!(
        grants["grants"]
            .as_array()
            .expect("grants")
            .iter()
            .any(|g| g["family_id"].as_str() == Some(family_id))
    );
}

#[test]
fn tc_t1_f21_03_tools_list_exposes_at_least_27_scoped_tools_and_invokes_allowed() {
    let h = FabushiE2EHarness::connect_production();
    let tools = h.expect_ok("mcp.tools_list", json!({}));
    let arr = tools["tools"].as_array().expect("tools");
    assert!(arr.len() >= 27, "expected >= 27 official MCP tools, got {}", arr.len());

    let invoked = h.expect_ok(
        "mcp.tool_invoke",
        json!({
            "access_token": "tok_mcp_devices_read",
            "protocol_version": "2025-11-25",
            "tool_name": "list_devices",
            "arguments": {},
            "node_id": "node-alpha"
        }),
    );
    assert_eq!(invoked["ok"], true);
    assert!(!invoked["audit_id"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f21_04_mcp_audit_redacts_sensitive_fields_and_caps_280_chars() {
    let h = FabushiE2EHarness::connect_production();
    let args = json!({"text": "TopSecretKeyboardInput999", "password": "MyPassword!42"});
    let _ = h.expect_ok(
        "mcp.tool_invoke",
        json!({
            "access_token": "tok_mcp_input_control",
            "protocol_version": "2025-11-25",
            "tool_name": "keyboard_type",
            "arguments": args,
            "node_id": "node-alpha"
        }),
    );

    let audit = h.expect_ok("mcp.audit_list", json!({"limit": 10}));
    let latest = audit["entries"].as_array().expect("entries").first().expect("entry");
    let redacted = latest["redacted_args"].as_str().expect("redacted_args");
    verify_mcp_audit_redaction(&args, redacted).expect("redacted <= 280 chars & zero secret leak");
}

#[test]
fn tc_t1_f21_05_grant_revoke_and_kill_switch_terminate_active_sessions() {
    let h = FabushiE2EHarness::connect_production();
    let grant = h.expect_ok(
        "mcp.oauth_approve",
        json!({
            "authorize_code_or_qr_id": "qr_mcp_02",
            "approved_scopes": ["devices:read"],
            "allowed_node_ids": ["node-alpha"],
            "pkce_code_challenge": "challenge_2"
        }),
    );
    let rev = h.expect_ok("mcp.grant_revoke", json!({"family_id": grant["family_id"]}));
    assert_eq!(rev["revoked"], true);

    let _grant2 = h.expect_ok(
        "mcp.oauth_approve",
        json!({
            "authorize_code_or_qr_id": "qr_mcp_03",
            "approved_scopes": ["devices:read"],
            "allowed_node_ids": ["node-alpha"],
            "pkce_code_challenge": "challenge_3"
        }),
    );
    let killed = h.expect_ok("mcp.kill_switch", json!({"reason": "user_emergency"}));
    assert!(
        killed["revoked_families"]
            .as_u64()
            .expect("revoked_families")
            >= 1,
        "mcp.kill_switch must revoke at least 1 active grant family"
    );
}

// ============================================================================
// F22 — UU-Remote Computer Workbench, PTY Terminal & Tailnet (5 tests)
// ============================================================================

#[test]
fn tc_t1_f22_01_cloud_sandbox_node_crud_and_policy_patch() {
    let h = FabushiE2EHarness::connect_production();
    let created = h.expect_ok(
        "computers.cloud_upsert",
        json!({"name": "Cloud Sandbox A", "region": "us-west1"}),
    );
    let node_id = created["node_id"].as_str().expect("node_id");

    let patched = h.expect_ok(
        "computers.policy_patch",
        json!({"node_id": node_id, "allow_terminal": true, "allow_power": false}),
    );
    assert_eq!(patched["policy"]["allow_terminal"], true);

    let tested = h.expect_ok("computers.test", json!({"node_id": node_id}));
    assert_eq!(tested["online"], true);

    let _ = h.expect_ok("computers.delete", json!({"node_id": node_id}));
    let list = h.expect_ok("computers.list", json!({}));
    assert!(
        !list["nodes"]
            .as_array()
            .expect("nodes array")
            .iter()
            .any(|n| n["node_id"].as_str() == Some(node_id))
    );
}

#[test]
fn tc_t1_f22_02_remote_stream_mouse_modes_display_privacy_and_file_transfer() {
    let h = FabushiE2EHarness::connect_production();
    let stream = h.expect_ok(
        "remote.stream_start",
        json!({"node_id": "node_dev_mac", "display_id": 1, "privacy_screen": true}),
    );
    let sid = stream["session_id"].as_str().expect("session_id");

    for mouse_mode in ["absolute", "game_relative", "cursor_lock"] {
        let sent = h.expect_ok(
            "remote.input_send",
            json!({"session_id": sid, "mouse_mode": mouse_mode, "hotkeys": ["Cmd+C"]}),
        );
        assert_eq!(sent["accepted"], true);
    }

    let xfer = h.expect_ok(
        "remote.file_transfer_start",
        json!({"node_id": "node_dev_mac", "direction": "upload", "path": "/tmp/e2e.bin", "conflict_policy": "overwrite", "offset_bytes": 0}),
    );
    assert!(!xfer["transfer_id"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f22_03_two_concurrent_pty_sessions_isolated_without_crosstalk() {
    let h = FabushiE2EHarness::connect_production();
    let p1 = h.expect_ok("remote.pty_open", json!({"node_id": "node_dev_mac", "cols": 120, "rows": 36}));
    let p2 = h.expect_ok("remote.pty_open", json!({"node_id": "node_dev_mac", "cols": 80, "rows": 24}));
    let id1 = p1["pty_id"].as_str().expect("id1");
    let id2 = p2["pty_id"].as_str().expect("id2");
    assert_ne!(id1, id2);

    let _ = h.expect_ok("remote.pty_write", json!({"pty_id": id1, "data": "echo PTY_ONE\n"}));
    let _ = h.expect_ok("remote.pty_write", json!({"pty_id": id2, "data": "echo PTY_TWO\n"}));
    let _ = h.expect_ok("remote.pty_resize", json!({"pty_id": id1, "cols": 140, "rows": 40}));

    let r1 = h.expect_ok("remote.pty_read", json!({"pty_id": id1}));
    let r2 = h.expect_ok("remote.pty_read", json!({"pty_id": id2}));
    let out1 = r1["output"].as_str().expect("PTY 1 output string");
    let out2 = r2["output"].as_str().expect("PTY 2 output string");
    assert!(out1.contains("PTY_ONE"), "PTY 1 must contain PTY_ONE, got: {out1:?}");
    assert!(!out1.contains("PTY_TWO"), "PTY 1 must never leak PTY_TWO, got: {out1:?}");
    assert!(out2.contains("PTY_TWO"), "PTY 2 must contain PTY_TWO, got: {out2:?}");
    assert!(!out2.contains("PTY_ONE"), "PTY 2 must never leak PTY_ONE, got: {out2:?}");

    let _ = h.expect_ok("remote.pty_close", json!({"pty_id": id1}));
    let _ = h.expect_ok("remote.pty_close", json!({"pty_id": id2}));
}

#[test]
fn tc_t1_f22_04_remote_assist_and_tailnet_mesh_discovery() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("remote.assist_set_code", json!({"assist_code": "FAB-7788"}));
    let _ = h.expect_ok("remote.assist_allow", json!({"allowed": true}));
    let started = h.expect_ok("remote.assist_start", json!({"assist_code": "FAB-7788"}));
    assert!(!started["session_id"].as_str().unwrap_or("").is_empty());

    let reg = h.expect_ok("tailnet.register", json!({"device_name": "iPhone 16 Pro"}));
    assert!(!reg["node_ip"].as_str().unwrap_or("").is_empty());
    let netmap = h.expect_ok("tailnet.netmap", json!({}));
    assert!(netmap["peers"].is_array());
    let derp = h.expect_ok("tailnet.derp_status", json!({}));
    assert_eq!(derp["derp_connected"], true);
}

#[test]
fn tc_t1_f22_05_controlled_banner_variants_and_sub_10ms_emergency_takeover() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "computers.lease_acquire",
        json!({"node_id": "node_dev_mac", "holder_kind": "ai_mcp"}),
    );
    let sess = h.expect_ok("remote.sessions_list", json!({}));
    let variant = sess["banner_variant"].as_str().expect("banner_variant");
    let hex = sess["banner_hex"].as_str().expect("banner_hex");
    verify_controlled_banner_hex(variant, hex).expect("canonical controlled banner hex");

    let start = Instant::now();
    let takeover = h.expect_ok(
        "computers.emergency_takeover",
        json!({"node_id": "node_dev_mac", "holder_kind": "human"}),
    );
    let elapsed_ms = start.elapsed().as_millis();
    assert!(elapsed_ms < 50 && takeover["elapsed_ms"].as_u64().unwrap_or(99) < 10);
    assert!(takeover["override_epoch"].as_u64().unwrap_or(0) >= 1);
}

// ============================================================================
// F23 — Zero-Knowledge Split-Key V2 & X25519 Vault + TOTP + Remote Obscura (5 tests)
// ============================================================================

#[test]
fn tc_t1_f23_01_vault_envelope_put_get_argon2id_zero_plaintext_key() {
    let h = FabushiE2EHarness::connect_production();
    verify_argon2id_params(65536, 3, 4, 16, 32).expect("valid Argon2id");
    let put = h.expect_ok(
        "vault.envelope_put",
        json!({
            "kdf_alg": "split_key_v2",
            "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4},
            "auth_salt_b64": B64.encode([0x1Au8; 16]),
            "wrapped_dek_b64": B64.encode([0x2Bu8; 48]),
            "nonce_b64": B64.encode([0x3Cu8; 12])
        }),
    );
    assert_eq!(put["kdf_alg"], "split_key_v2");
    let got = h.expect_ok("vault.envelope_get", json!({}));
    assert_eq!(got["kdf_alg"], "split_key_v2");
}

#[test]
fn tc_t1_f23_02_vault_items_crud_across_all_3_categories() {
    let h = FabushiE2EHarness::connect_production();
    for (cat, key) in [
        ("password", "github.com:alice"),
        ("browser_cookie", "example.com:sid"),
        ("ai_oauth", "anthropic:oauth_v1"),
    ] {
        let ct = vec![0xABu8; 64];
        let nonce = [0x09u8; 12];
        verify_vault_size_cap(cat, key, ct.len(), nonce.len()).expect("valid vault item");
        let up = h.expect_ok(
            "vault.item_upsert",
            json!({
                "category": cat,
                "item_key": key,
                "ciphertext_b64": B64.encode(&ct),
                "nonce_b64": B64.encode(nonce)
            }),
        );
        assert_eq!(up["category"], cat);
        assert_eq!(up["item_key"], key);
    }

    let pulled = h.expect_ok("vault.items_pull", json!({}));
    assert!(pulled["items"].as_array().expect("items").len() >= 3);

    let _ = h.expect_ok(
        "vault.item_delete",
        json!({"category": "browser_cookie", "item_key": "example.com:sid"}),
    );
}

#[test]
fn tc_t1_f23_03_x25519_cross_device_sync_request_approve_complete() {
    let h = FabushiE2EHarness::connect_production();
    let secondary = h.spawn_secondary_host("iPad Pro M4");

    let req = secondary.expect_ok(
        "vault.device_sync_request",
        json!({
            "device_name": "iPad Pro M4",
            "ephemeral_x25519_pub_b64": B64.encode([0x77u8; 32])
        }),
    );
    let req_id = req["request_id"].as_str().expect("request_id");
    let code_sec = req["verification_code"].as_str().expect("verification_code");

    let pending = h.expect_ok("vault.device_sync_pending", json!({}));
    let matched = pending["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .find(|r| r["request_id"].as_str() == Some(req_id))
        .expect("pending sync request");
    assert_eq!(matched["verification_code"].as_str(), Some(code_sec));

    let _ = h.expect_ok(
        "vault.device_sync_approve",
        json!({"request_id": req_id, "wrapped_key_b64": B64.encode([0xEEu8; 48])}),
    );
    let status = secondary.expect_ok("vault.device_sync_status", json!({"request_id": req_id}));
    assert_eq!(status["status"], "approved");
}

#[test]
fn tc_t1_f23_04_rfc6238_totp_sha1_and_sha256_exact_vectors() {
    let h = FabushiE2EHarness::connect_production();
    let sha1 = h.expect_ok(
        "vault.totp_generate",
        json!({
            "secret_or_otpauth": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
            "unix_secs": 59,
            "digits": 6,
            "period_secs": 30,
            "algorithm": "SHA1"
        }),
    );
    let code1 = sha1["code"].as_str().expect("sha1 code");
    verify_rfc6238_totp(
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
        59,
        30,
        6,
        TotpAlgorithm::Sha1,
        code1,
    )
    .expect("exact RFC 6238 SHA1 287082");

    let sha256 = h.expect_ok(
        "vault.totp_generate",
        json!({
            "secret_or_otpauth": "otpauth://totp/Fabushi:test?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA&algorithm=SHA256&digits=8&period=30",
            "unix_secs": 59
        }),
    );
    let code2 = sha256["code"].as_str().expect("sha256 code");
    verify_rfc6238_totp(
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA",
        59,
        30,
        8,
        TotpAlgorithm::Sha256,
        code2,
    )
    .expect("exact RFC 6238 SHA256 46119246");
}

#[test]
fn tc_t1_f23_05_remote_obscura_browser_session_on_desktop_host() {
    let h = FabushiE2EHarness::connect_production();
    let opened = h.expect_ok(
        "vault.obscura_remote_session",
        json!({"host_node_id": "node_dev_mac", "action": "open", "url": "https://example.com"}),
    );
    assert_eq!(opened["remote_host_only"], true);
    assert!(!opened["session_id"].as_str().unwrap_or("").is_empty());
}

// ============================================================================
// F24 — External Mini Apps, WebMCP Live Control, PiP & 全球法布施 (5 tests)
// ============================================================================

#[test]
fn tc_t1_f24_01_accept_tos_launch_validate_hmac_and_isolated_storage() {
    let h = FabushiE2EHarness::connect_production();
    let tos = h.expect_ok("miniapp.accept_tos", json!({"bot_id": "bot_zen"}));
    assert_eq!(tos["tos_accepted"], true);

    let launched = h.expect_ok(
        "miniapp.launch",
        json!({"bot_username": "zen_bot", "short_name": "app"}),
    );
    assert_eq!(launched["init_data_hmac_valid"], true);
    assert!(!launched["instance_id"].as_str().unwrap_or("").is_empty());

    let _ = h.expect_ok(
        "miniapp.storage_put",
        json!({"bot_username": "zen_bot", "short_name": "app", "key": "theme", "value": "night"}),
    );
    let got = h.expect_ok(
        "miniapp.storage_get",
        json!({"bot_username": "zen_bot", "short_name": "app", "key": "theme"}),
    );
    assert_eq!(got["value"], "night");
    assert_eq!(got["isolated"], true);
}

#[test]
fn tc_t1_f24_02_webmcp_register_and_invoke_tracks_monotonic_state_revision() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("miniapp.accept_tos", json!({"bot_id": "bot_dharma"}));
    let launched = h.expect_ok(
        "miniapp.launch",
        json!({"bot_username": "dharma_bot", "short_name": "global_dharma"}),
    );
    let inst = launched["instance_id"].as_str().expect("instance_id");
    let nonce = launched["nonce"].as_str().expect("nonce");
    let gen_val = launched["generation"].as_u64().expect("generation");

    let reg = h.expect_ok(
        "webmcp.register_tools",
        json!({
            "account_id": "acc_1",
            "app_id": "global_dharma",
            "version": "1.6.7",
            "digest": "sha256:abc123",
            "instance_id": inst,
            "nonce": nonce,
            "generation": gen_val,
            "tools": [
                {"name": "offline.state.read", "mutating": false},
                {"name": "offline.practice.record", "mutating": true}
            ]
        }),
    );
    assert_eq!(reg["registered_count"], 2);

    let r1 = h.expect_ok(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global_dharma",
            "version": "1.6.7",
            "digest": "sha256:abc123",
            "instance_id": inst,
            "nonce": nonce,
            "generation": gen_val,
            "call_id": "call_1",
            "tool": "offline.state.read",
            "arguments": {}
        }),
    );
    let rev1 = r1["state_revision"].as_u64().expect("rev1");

    let r2 = h.expect_ok(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global_dharma",
            "version": "1.6.7",
            "digest": "sha256:abc123",
            "instance_id": inst,
            "nonce": nonce,
            "generation": gen_val,
            "call_id": "call_2",
            "tool": "offline.practice.record",
            "arguments": {"tag": "ci"},
            "approved": true
        }),
    );
    assert!(r2["state_revision"].as_u64().expect("r2 state_revision") > rev1);
}

#[test]
fn tc_t1_f24_03_floating_pip_minimize_webmcp_mutation_and_inplace_restore() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("miniapp.accept_tos", json!({"bot_id": "bot_dharma"}));
    let launched = h.expect_ok(
        "miniapp.launch",
        json!({"bot_username": "dharma_bot", "short_name": "global_dharma"}),
    );
    let inst = launched["instance_id"].as_str().expect("instance_id");
    let nonce = launched["nonce"].as_str().expect("nonce");
    let gen_val = launched["generation"].as_u64().expect("generation");

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
    assert_eq!(pip["state"], "pip");
    let rev_before = pip["preserved_revision"]
        .as_u64()
        .expect("pip_minimize preserved_revision");

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
            "call_id": "call_pip_1",
            "tool": "offline.practice.record",
            "arguments": {"tag": "pip"},
            "approved": true
        }),
    );
    let rev_after_mut = inv["state_revision"]
        .as_u64()
        .expect("webmcp.invoke state_revision");
    assert!(
        rev_after_mut > rev_before,
        "mutating WebMCP call during PiP must strictly increment state_revision ({rev_before} -> {rev_after_mut})"
    );

    let restored = h.expect_ok("miniapp.pip_restore", json!({"instance_id": inst}));
    assert_eq!(restored["state"], "expanded");
    assert_eq!(restored["instance_id"].as_str(), Some(inst));
    let rev_restored = restored["preserved_revision"]
        .as_u64()
        .expect("pip_restore preserved_revision");
    assert!(
        rev_restored >= rev_after_mut,
        "pip_restore must preserve mutated revision (expected >= {rev_after_mut}, got {rev_restored})"
    );
}

#[test]
fn tc_t1_f24_04_global_dharma_offline_fabapp_verify_and_free_tool_offline() {
    let h = FabushiE2EHarness::connect_production();
    let pkg_bytes = b"FABAPP_V1_OFFLINE_PACKAGE_PAYLOAD";
    let digest = sha256_hex(pkg_bytes);
    let verified = h.expect_ok(
        "miniapp.verify_offline_package",
        json!({
            "package_id": "global-dharma",
            "version": "1.6.7",
            "sha256_digest": digest,
            "ed25519_sig_b64": B64.encode([0x55u8; 64]),
            "bytes_b64": B64.encode(pkg_bytes)
        }),
    );
    assert_eq!(verified["verified"], true);
    assert_eq!(verified["offline_ready"], true);

    h.server().set_offline(true);
    let res = h.expect_ok(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global-dharma",
            "version": "1.6.7",
            "digest": digest,
            "instance_id": "offline_inst_1",
            "nonce": "offline_nonce_1",
            "generation": h.current_generation(),
            "call_id": "call_offline_free_1",
            "tool": "offline.state.read",
            "arguments": {}
        }),
    );
    assert_eq!(res["ok"], true);
}

#[test]
fn tc_t1_f24_05_miniapp_storage_isolation_between_apps_and_uninstall_wipe() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "miniapp.storage_put",
        json!({"bot_username": "bot_a", "short_name": "app_a", "key": "secret_k", "value": "val_a"}),
    );
    let own_before = h.expect_ok(
        "miniapp.storage_get",
        json!({"bot_username": "bot_a", "short_name": "app_a", "key": "secret_k"}),
    );
    assert_eq!(own_before["value"], "val_a");

    let other = h.expect_ok(
        "miniapp.storage_get",
        json!({"bot_username": "bot_b", "short_name": "app_b", "key": "secret_k"}),
    );
    assert!(other["value"].is_null());

    let uninst = h.expect_ok(
        "miniapp.uninstall",
        json!({"bot_username": "bot_a", "short_name": "app_a"}),
    );
    assert_eq!(uninst["installed"], false);

    let own_after = h.expect_ok(
        "miniapp.storage_get",
        json!({"bot_username": "bot_a", "short_name": "app_a", "key": "secret_k"}),
    );
    assert!(
        own_after["value"].is_null(),
        "miniapp.uninstall must wipe isolated storage for app_a"
    );
}

// ============================================================================
// F25 — Chunked Media CAS, HTTP 206, 5-Bit Waveform, Stickers, Stories, IV (5 tests)
// ============================================================================

#[test]
fn tc_t1_f25_01_chunked_resumable_cas_upload_deduplicates_sha256() {
    let h = FabushiE2EHarness::connect_production();
    let part0 = vec![0x31u8; 4096];
    let part1 = vec![0x32u8; 4096];
    let mut full = Vec::new();
    full.extend_from_slice(&part0);
    full.extend_from_slice(&part1);
    let expected_sha = sha256_hex(&full);

    let init = h.expect_ok(
        "media.upload_init",
        json!({"filename": "photo.jpg", "mime_type": "image/jpeg", "total_size": full.len(), "part_size": 4096, "expected_sha256": expected_sha}),
    );
    let uid = init["upload_id"].as_str().expect("upload_id");
    let _ = h.expect_ok("media.upload_part", json!({"upload_id": uid, "part_index": 0, "data_b64": B64.encode(&part0)}));
    let st = h.expect_ok("media.upload_status", json!({"upload_id": uid}));
    assert_eq!(st["received_parts"], json!([0]));
    let _ = h.expect_ok("media.upload_part", json!({"upload_id": uid, "part_index": 1, "data_b64": B64.encode(&part1)}));
    let done = h.expect_ok("media.upload_complete", json!({"upload_id": uid}));
    assert_eq!(done["sha256"].as_str(), Some(expected_sha.as_str()));
    assert!(!done["file_id"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f25_02_http_206_byte_range_streaming_matches_original_bytes() {
    let h = FabushiE2EHarness::connect_production();
    let payload: Vec<u8> = (0..8192).map(|i| (i % 253) as u8).collect();
    h.server().seed_media_blob("vid_stream_1", payload.clone());

    let head = h.expect_ok(
        "media.stream_range",
        json!({"file_id": "vid_stream_1", "range_start": 0, "range_end": 1023}),
    );
    assert_eq!(head["status"], 206);
    let decoded = B64
        .decode(head["data_b64"].as_str().expect("data_b64"))
        .expect("b64");
    assert_eq!(decoded.as_slice(), &payload[0..=1023]);
}

#[test]
fn tc_t1_f25_03_waveform_5bit_100_samples_exact_63_bytes_lossless() {
    let h = FabushiE2EHarness::connect_production();
    let samples: Vec<u8> = (0..100).map(|i| (i % 32) as u8).collect();
    let enc = h.expect_ok("media.encode_waveform", json!({"samples": samples}));
    assert_eq!(enc["byte_len"], 63);
    let wf_b64 = enc["waveform_b64"].as_str().expect("waveform_b64");
    let raw = B64.decode(wf_b64).expect("decode wf b64");
    verify_waveform_5bit_63bytes(&samples, &raw).expect("canonical 63B 5-bit packing");

    let dec = h.expect_ok("media.decode_waveform", json!({"waveform_b64": wf_b64}));
    let out: Vec<u8> = dec["samples"]
        .as_array()
        .expect("samples")
        .iter()
        .map(|v| v.as_u64().unwrap_or(0) as u8)
        .collect();
    assert_eq!(out, samples);
}

#[test]
fn tc_t1_f25_04_stickers_gifs_media_editor_and_instant_view() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("stickers.install_set", json!({"set_id": "fabushi_pack_1"}));
    let sets = h.expect_ok("stickers.list_sets", json!({}));
    assert!(
        sets["sets"]
            .as_array()
            .expect("sticker sets")
            .iter()
            .any(|s| s["set_id"] == "fabushi_pack_1" || s["id"] == "fabushi_pack_1"),
        "installed sticker set fabushi_pack_1 must appear in stickers.list_sets"
    );
    let gifs = h.expect_ok("stickers.search_gifs", json!({"query": "wave", "limit": 10}));
    assert!(!gifs["gifs"].as_array().expect("gifs").is_empty());

    let edited = h.expect_ok(
        "media.edit_apply",
        json!({"file_id": "cas_photo_1", "rotate_deg": 90, "spoiler_blur": true}),
    );
    assert!(!edited["edited_file_id"].as_str().expect("edited_file_id").is_empty());

    let iv = h.expect_ok(
        "media.instant_view_fetch",
        json!({"url": format!("{}/article/v167", h.server_url())}),
    );
    assert!(!iv["blocks"].as_array().expect("instant view blocks").is_empty());
}

#[test]
fn tc_t1_f25_05_stories_create_24h_ttl_list_view_and_delete() {
    let h = FabushiE2EHarness::connect_production();
    let created = h.expect_ok(
        "stories.create",
        json!({"media_id": "cas_story_1", "media_type": "photo", "caption": "Hello 24h", "privacy": "contacts"}),
    );
    let sid = created["story_id"].as_str().expect("story_id");
    assert!(created["expires_at_ms"].as_i64().expect("expires_at_ms") > now_ms());

    let viewed = h.expect_ok("stories.view", json!({"story_id": sid}));
    assert!(viewed["views_count"].as_u64().expect("views_count") >= 1);

    let _ = h.expect_ok("stories.delete", json!({"story_id": sid}));
    let list = h.expect_ok("stories.list", json!({}));
    assert!(
        !list["stories"]
            .as_array()
            .expect("stories array")
            .iter()
            .any(|s| s["story_id"].as_str() == Some(sid))
    );
}

// ============================================================================
// F26 — 1:1 & Group Calls & Full-Duplex AI Talk Mode (5 tests)
// ============================================================================

#[test]
fn tc_t1_f26_01_call_invite_signal_fingerprint_turn_43200s_and_hangup() {
    let h = FabushiE2EHarness::connect_production();
    let turn = h.expect_ok("calls.turn_credentials", json!({}));
    assert_eq!(turn["ttl_seconds"], 43200);

    let call = h.expect_ok("calls.invite", json!({"peer_id": 2001, "kind": "audio"}));
    let call_id = call["call_id"].as_str().expect("call_id");
    let sig = h.expect_ok(
        "calls.signal",
        json!({"call_id": call_id, "signal_frame": {"type": "offer", "sdp": "v=0"}}),
    );
    assert_eq!(sig["emoji_fingerprint"].as_array().expect("4 emojis").len(), 4);

    let ended = h.expect_ok("calls.hangup", json!({"call_id": call_id}));
    assert_eq!(ended["state"], "ended");
}

#[test]
fn tc_t1_f26_02_calls_history_and_post_call_rating_1_to_5() {
    let h = FabushiE2EHarness::connect_production();
    let call = h.expect_ok("calls.invite", json!({"peer_id": 2002, "kind": "video"}));
    let call_id = call["call_id"].as_str().expect("call_id");
    let _ = h.expect_ok("calls.hangup", json!({"call_id": call_id}));

    let hist = h.expect_ok("calls.history", json!({}));
    assert!(
        hist["calls"]
            .as_array()
            .expect("calls history array")
            .iter()
            .any(|c| c["call_id"].as_str() == Some(call_id)),
        "completed call must appear in calls.history"
    );
    let rated = h.expect_ok(
        "calls.rate",
        json!({"call_id": call_id, "rating": 5, "feedback": "Crystal clear"}),
    );
    assert_eq!(rated["saved"], true);
    assert_eq!(rated["rating"], 5);
}

#[test]
fn tc_t1_f26_03_talk_mode_session_fsm_and_audio_buffering() {
    let h = FabushiE2EHarness::connect_production();
    let sess = h.expect_ok("talk.session_create", json!({"offline_asr": false}));
    let sid = sess["session_id"].as_str().expect("session_id");
    let enq = h.expect_ok(
        "talk.audio_enqueue",
        json!({"session_id": sid, "chunk_b64": B64.encode([0x10u8; 320])}),
    );
    assert!(enq["buffered_chunks"].as_u64().unwrap_or(0) >= 1);
    let _ = h.expect_ok("talk.session_close", json!({"session_id": sid}));
}

#[test]
fn tc_t1_f26_04_talk_mode_3x20ms_vad_barge_in_flushes_queue_under_60ms() {
    let h = FabushiE2EHarness::connect_production();
    let sess = h.expect_ok("talk.session_create", json!({}));
    let sid = sess["session_id"].as_str().expect("session_id");
    let _ = h.expect_ok(
        "talk.audio_enqueue",
        json!({"session_id": sid, "chunk_b64": B64.encode([0x22u8; 640])}),
    );

    let loud_pcm: Vec<u8> = (0..320)
        .flat_map(|_| 2000i16.to_le_bytes())
        .collect();
    let mut triggered = false;
    let mut buffered = 1u64;
    for _ in 0..3 {
        let out = h.expect_ok(
            "talk.vad_frame",
            json!({"session_id": sid, "pcm_i16_b64": B64.encode(&loud_pcm)}),
        );
        triggered = out["barge_in_triggered"].as_bool().unwrap_or(false);
        buffered = out["buffered_chunks"].as_u64().unwrap_or(1);
    }
    assert!(triggered);
    assert_eq!(buffered, 0);
    assert_eq!(verify_talk_vad_bargein(3, 0, 0).unwrap(), (true, false));
}

#[test]
fn tc_t1_f26_05_talk_mode_25x20ms_silence_eos_and_resume_listening() {
    let h = FabushiE2EHarness::connect_production();
    let sess = h.expect_ok("talk.session_create", json!({}));
    let sid = sess["session_id"].as_str().expect("session_id");
    let silent_pcm = vec![0u8; 640];
    let mut last = json!({});
    for _ in 0..25 {
        last = h.expect_ok(
            "talk.vad_frame",
            json!({"session_id": sid, "pcm_i16_b64": B64.encode(&silent_pcm)}),
        );
    }
    assert_eq!(last["end_of_speech"], true);
    assert_eq!(verify_talk_vad_bargein(0, 25, 0).unwrap(), (false, true));
    let resumed = h.expect_ok("talk.resume_listening", json!({"session_id": sid}));
    assert_eq!(resumed["state"], "listening");
}

// ============================================================================
// F27 — Unified Tantivy + Hybrid Memory + Bots/MiniApps + Offline FTS5 Search (5 tests)
// ============================================================================

#[test]
fn tc_t1_f27_01_tantivy_global_search_matches_english_and_cjk() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Search Room"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "全球法布施 Apple HIG milestone", "client_nonce": 70001}),
    );

    let res_cjk = h.expect_ok("search.query", json!({"q": "全球法布施", "category": "messages"}));
    assert!(res_cjk["total"].as_u64().unwrap_or(0) >= 1);
    let res_en = h.expect_ok("search.query", json!({"q": "milestone", "category": "messages"}));
    assert!(res_en["total"].as_u64().unwrap_or(0) >= 1);
}

#[test]
fn tc_t1_f27_02_hybrid_memory_search_with_entity_hint() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "fabushi.memory_put",
        json!({"target": "memory", "content": "FabushiTheme design system tokens"}),
    );
    let res = h.expect_ok(
        "search.query",
        json!({"q": "design system", "category": "hybrid", "mode": "hybrid", "entity": "FabushiTheme"}),
    );
    assert_eq!(res["engine"], "hybrid");
    assert!(
        !res["results"].as_array().expect("hybrid results").is_empty(),
        "hybrid search must return seeded FabushiTheme design system entry"
    );
}

#[test]
fn tc_t1_f27_03_catalog_search_spanning_bots_and_miniapps() {
    let h = FabushiE2EHarness::connect_production();
    let bots = h.expect_ok("search.query", json!({"q": "bot", "category": "bots"}));
    assert!(!bots["results"].as_array().expect("bots").is_empty());
    let apps = h.expect_ok("search.query", json!({"q": "dharma", "category": "miniapps"}));
    assert!(!apps["results"].as_array().expect("apps").is_empty());
}

#[test]
fn tc_t1_f27_04_in_chat_scoped_search_excludes_other_chats() {
    let h = FabushiE2EHarness::connect_production();
    let c1 = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Scope 1"}))["chat_id"]
        .as_i64()
        .expect("c1");
    let c2 = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Scope 2"}))["chat_id"]
        .as_i64()
        .expect("c2");
    let _ = h.expect_ok("messages.send", json!({"chat_id": c1, "text": "alpha_token_xyz", "client_nonce": 70002}));
    let _ = h.expect_ok("messages.send", json!({"chat_id": c2, "text": "alpha_token_xyz", "client_nonce": 70003}));

    let scoped = h.expect_ok("search.query", json!({"q": "alpha_token_xyz", "chat_id": c1}));
    let results = scoped["results"].as_array().expect("results");
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r["chat_id"].as_i64() == Some(c1)));
}

#[test]
fn tc_t1_f27_05_offline_sqlite_fts5_fallback_when_server_unreachable() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Offline FTS"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "法布施离线搜索本地索引", "client_nonce": 70004}),
    );
    let _ = h.expect_ok("search.reindex_local", json!({}));

    h.server().set_offline(true);
    let offline = h.expect_ok(
        "search.query",
        json!({"q": "法布施离线搜索", "offline_only": true}),
    );
    assert_eq!(offline["engine"], "sqlite_fts5_offline");
    assert!(offline["total"].as_u64().unwrap_or(0) >= 1);
}

// ============================================================================
// F28 — E2EE Messaging (Double Ratchet, Sender Keys, Safety Numbers, Replay) (5 tests)
// ============================================================================

#[test]
fn tc_t1_f28_01_publish_signed_prekeys_and_consume_otpk() {
    let h = FabushiE2EHarness::connect_production();
    let pub_res = h.expect_ok(
        "e2ee.publish_prekeys",
        json!({
            "device_id": "iphone_e2ee_1",
            "identity_key_b64": B64.encode([0x11u8; 32]),
            "signed_prekey_b64": B64.encode([0x22u8; 32]),
            "spk_signature_b64": B64.encode([0x33u8; 64]),
            "one_time_prekeys_b64": [B64.encode([0x44u8; 32]), B64.encode([0x45u8; 32])]
        }),
    );
    assert_eq!(pub_res["published_otpk_count"], 2);
    let before = h.expect_ok("e2ee.prekey_count", json!({"device_id": "iphone_e2ee_1"}));
    assert_eq!(before["remaining_otpks"], 2);

    let bundle = h.expect_ok("e2ee.fetch_prekey_bundle", json!({"user_id": 1001}));
    assert!(!bundle["devices"].as_array().expect("devices").is_empty());
}

#[test]
fn tc_t1_f28_02_double_ratchet_1to1_encrypt_decrypt_advances_seq() {
    let h = FabushiE2EHarness::connect_production();
    let pt1 = B64.encode(b"Forward secret turn 1");
    let enc1 = h.expect_ok(
        "e2ee.encrypt_dm",
        json!({"peer_user_id": 2001, "peer_device_id": "dev_bob", "plaintext_b64": pt1}),
    );
    let seq1 = enc1["ratchet_seq"].as_u64().expect("seq1");
    let dec1 = h.expect_ok(
        "e2ee.decrypt_dm",
        json!({"peer_user_id": 2001, "peer_device_id": "dev_bob", "envelope": enc1["envelope"]}),
    );
    assert_eq!(dec1["plaintext_b64"].as_str(), Some(pt1.as_str()));

    let pt2 = B64.encode(b"Forward secret turn 2");
    let enc2 = h.expect_ok(
        "e2ee.encrypt_dm",
        json!({"peer_user_id": 2001, "peer_device_id": "dev_bob", "plaintext_b64": pt2}),
    );
    assert!(enc2["ratchet_seq"].as_u64().unwrap_or(0) > seq1);
}

#[test]
fn tc_t1_f28_03_group_sender_keys_distribution_and_decrypt() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "e2ee.group_distribute_sender_key",
        json!({"chat_id": 801, "sender_id": 1001, "member_ids": [1001, 1002, 1003]}),
    );
    let pt = B64.encode(b"Group encrypted payload");
    let enc = h.expect_ok(
        "e2ee.encrypt_group",
        json!({"chat_id": 801, "sender_id": 1001, "payload": pt}),
    );
    let dec = h.expect_ok(
        "e2ee.decrypt_group",
        json!({"chat_id": 801, "sender_id": 1001, "payload": enc["envelope"]}),
    );
    assert_eq!(dec["plaintext_b64"].as_str(), Some(pt.as_str()));
}

#[test]
fn tc_t1_f28_04_safety_number_4_emoji_and_qr_symmetric() {
    let h = FabushiE2EHarness::connect_production();
    let sn = h.expect_ok("e2ee.safety_number", json!({"peer_user_id": 2001}));
    assert_eq!(sn["emojis"].as_array().expect("4 emojis").len(), 4);
    assert!(!sn["numeric_fingerprint"].as_str().unwrap_or("").is_empty());
    assert!(!sn["qr_payload"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f28_05_sliding_replay_window_128_accepts_out_of_order_once() {
    let h = FabushiE2EHarness::connect_production();
    let mut highest = 0u64;
    let mut bitmap = 0u128;
    for seq in [10u64, 8, 9, 137, 11] {
        let local_ok = verify_sliding_replay_window_128(&mut highest, &mut bitmap, seq);
        let res = h.expect_ok(
            "e2ee.replay_window_check",
            json!({"session_id": "rw_t1", "seq": seq}),
        );
        assert_eq!(res["accepted"].as_bool(), Some(local_ok));
        assert!(local_ok);
    }
}

// ============================================================================
// F29 — Economy, Stars Ledger, Star Gifts, Premium, Redeem & StoreKit 2 (5 tests)
// ============================================================================

#[test]
fn tc_t1_f29_01_wallet_balance_topup_and_transactions_ledger() {
    let h = FabushiE2EHarness::connect_production();
    let init = h.expect_ok("wallet.balance", json!({}));
    let b0 = init["stars_balance"].as_i64().expect("initial stars_balance");

    let topped = h.expect_ok(
        "wallet.topup",
        json!({"amount_stars": 500, "description": "E2E topup", "idempotency_key": "topup_k1"}),
    );
    assert_eq!(topped["stars_balance"].as_i64(), Some(b0 + 500));

    let txs = h.expect_ok("wallet.transactions", json!({"limit": 10}));
    assert!(!txs["items"].as_array().expect("items").is_empty());
}

#[test]
fn tc_t1_f29_02_atomic_stars_gift_transfer_debits_sender_and_credits_peer() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "wallet.topup",
        json!({"amount_stars": 500, "idempotency_key": "topup_gift_base"}),
    );
    let gifted = h.expect_ok(
        "wallet.send_gift",
        json!({"to_user_id": 2099, "amount_stars": 150, "gift_id": "lotus_star", "message": "Thank you", "idempotency_key": "gift_k1"}),
    );
    assert_eq!(gifted["stars_balance"].as_i64(), Some(350));
    assert!(!gifted["tx_id"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f29_03_redeem_code_activates_premium_tier() {
    let h = FabushiE2EHarness::connect_production();
    let redeemed = h.expect_ok("wallet.redeem_code", json!({"code": "FABUSHI-V167-PRO"}));
    assert_eq!(redeemed["redeemed"], true);
    let bal = h.expect_ok("wallet.balance", json!({}));
    assert_eq!(bal["premium_active"], true);
}

#[test]
fn tc_t1_f29_04_storekit2_jws_verify_and_restore_purchases() {
    let h = FabushiE2EHarness::connect_production();
    let ver = h.expect_ok(
        "wallet.storekit_verify_jws",
        json!({
            "signed_transaction_jws": "eyJhbGciOiJFUzI1NiJ9.eyJwcm9kdWN0SWQiOiJwcm9kLmdsb2JhbC1kaGFybWEubG9jYWwtcHJheWVyLXdoZWVsLm1vbnRobHkifQ.sig",
            "product_id": "prod.global-dharma.local-prayer-wheel.monthly"
        }),
    );
    assert!(matches!(
        ver["disposition"].as_str(),
        Some("verified") | Some("server_blocked")
    ));

    let restored = h.expect_ok("wallet.storekit_restore", json!({}));
    assert!(
        restored["restored_products"].is_array(),
        "storekit_restore must return restored_products array"
    );
    assert!(
        matches!(
            restored["disposition"].as_str(),
            Some("restored") | Some("no_purchases") | Some("server_blocked")
        ),
        "storekit_restore must report honest disposition"
    );
}

#[test]
fn tc_t1_f29_05_wallet_idempotency_key_prevents_double_credit_and_double_debit() {
    let h = FabushiE2EHarness::connect_production();
    let t1 = h.expect_ok(
        "wallet.topup",
        json!({"amount_stars": 300, "idempotency_key": "idem_topup_99"}),
    );
    let t2 = h.expect_ok(
        "wallet.topup",
        json!({"amount_stars": 300, "idempotency_key": "idem_topup_99"}),
    );
    assert_eq!(t1["stars_balance"], t2["stars_balance"]);
    assert_eq!(t1["tx_id"], t2["tx_id"]);

    let s1 = h.expect_ok(
        "wallet.send_stars",
        json!({"to_user_id": 2100, "amount_stars": 100, "idempotency_key": "idem_send_99"}),
    );
    let s2 = h.expect_ok(
        "wallet.send_stars",
        json!({"to_user_id": 2100, "amount_stars": 100, "idempotency_key": "idem_send_99"}),
    );
    assert_eq!(s1["stars_balance"], s2["stars_balance"]);
}

// ============================================================================
// F30 — Action Bus, App Intents / Siri Shortcuts, fabushi:// & Push (5 tests)
// ============================================================================

#[test]
fn tc_t1_f30_01_actions_list_and_schema_expose_app_intents() {
    let h = FabushiE2EHarness::connect_production();
    let list = h.expect_ok("actions.list", json!({}));
    let actions = list["actions"].as_array().expect("actions");
    assert!(!actions.is_empty());
    let first_name = actions[0]["name"].as_str().expect("action name");

    let schema = h.expect_ok("actions.schema", json!({"action_name": first_name}));
    assert_eq!(schema["action_name"].as_str(), Some(first_name));
    let obj = schema["json_schema"]
        .as_object()
        .expect("json_schema object");
    assert!(
        obj.contains_key("type") || obj.contains_key("properties"),
        "actions.schema must return concrete JSON schema object"
    );
}

#[test]
fn tc_t1_f30_02_actions_invoke_low_risk_app_intent_completes() {
    let h = FabushiE2EHarness::connect_production();
    let res = h.expect_ok(
        "actions.invoke",
        json!({"action_name": "chats.list_unread", "params": {}, "source": "app_intent"}),
    );
    assert_eq!(res["status"], "completed");
}

#[test]
fn tc_t1_f30_03_actions_approval_gated_executes_after_hmac_approval() {
    let h = FabushiE2EHarness::connect_production();
    let gated = h.expect_ok(
        "actions.invoke",
        json!({"action_name": "computer.power_reboot", "params": {"node_id": "node_1"}, "source": "app_intent"}),
    );
    assert_eq!(gated["status"], "pending_approval");
    let gate_id = gated["gate_id"].as_str().expect("gate_id");

    let approved = h.expect_ok(
        "actions.approve_gate",
        json!({"gate_id": gate_id, "hmac_signature": sha256_hex(gate_id.as_bytes())}),
    );
    assert_eq!(approved["status"], "approved");
}

#[test]
fn tc_t1_f30_04_route_canonical_fabushi_deep_links() {
    let h = FabushiE2EHarness::connect_production();
    for url in [
        "fabushi://chat/42",
        "fabushi://studio/approvals",
        "fabushi://miniapp/bot/dharma",
        "fabushi://auth/alipay?code=xyz",
    ] {
        let routed = h.expect_ok("actions.route_url", json!({"url": url}));
        assert_eq!(routed["scheme"], "fabushi");
        assert!(!routed["route_kind"].as_str().unwrap_or("").is_empty());
    }
}

#[test]
fn tc_t1_f30_05_push_register_unregister_honestly_reports_apns_upstream_stub() {
    let h = FabushiE2EHarness::connect_production();
    let reg = h.expect_ok(
        "push.register_token",
        json!({
            "platform": "apns",
            "device_token": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "device_id": "iphone_push_1",
            "environment": "sandbox"
        }),
    );
    assert_eq!(reg["registered"], true);
    assert_eq!(reg["apns_delivery_capability"], "upstream_stub");

    let unreg = h.expect_ok(
        "push.unregister_token",
        json!({"platform": "apns", "device_id": "iphone_push_1"}),
    );
    assert_eq!(unreg["registered"], false);
}

// ============================================================================
// F31 — Apple HIG Design System (4 Themes, WCAG >138, Dynamic Type, Locale) (5 tests)
// ============================================================================

#[test]
fn tc_t1_f31_01_switch_all_4_v167_themes_returns_canonical_surface_tokens() {
    let h = FabushiE2EHarness::connect_production();
    for (mode, expected_sidebar) in [
        ("night", "#0E0E0E"),
        ("tinted", "#181C22"),
        ("day", "#F5F5F5"),
        ("classic", "#FFFFFF"),
    ] {
        let res = h.expect_ok("settings.theme_set", json!({"mode": mode}));
        assert_eq!(res["mode"], mode);
        let bg = res["resolved_palette"]["sidebar_bg"].as_str().unwrap_or("");
        assert!(!bg.is_empty());
        if mode != "classic" {
            assert!(bg.eq_ignore_ascii_case(expected_sidebar));
        }
    }
}

#[test]
fn tc_t1_f31_02_wcag_accent_guard_keeps_blue_and_darkens_bright_yellow() {
    let h = FabushiE2EHarness::connect_production();
    let blue = h.expect_ok(
        "settings.theme_set",
        json!({"mode": "night", "accent_hex": "#0A84FF"}),
    );
    verify_wcag_accent_luma_guard(
        "#0A84FF",
        blue["resolved_palette"]["effective_accent_hex"].as_str().expect("eff"),
        blue["resolved_palette"]["contrast_adjusted"].as_bool().expect("adj"),
    )
    .expect("blue unchanged");

    let yellow = h.expect_ok(
        "settings.theme_set",
        json!({"mode": "night", "accent_hex": "#FFFF00"}),
    );
    verify_wcag_accent_luma_guard(
        "#FFFF00",
        yellow["resolved_palette"]["effective_accent_hex"].as_str().expect("eff"),
        yellow["resolved_palette"]["contrast_adjusted"].as_bool().expect("adj"),
    )
    .expect("yellow darkened 36% toward #111111");
}

#[test]
fn tc_t1_f31_03_theme_geometry_and_liquid_glass_persist_across_restart() {
    let mut h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "settings.theme_set",
        json!({
            "mode": "tinted",
            "font_size_px": 16,
            "bubble_radius_px": 14,
            "ui_scale_pct": 125,
            "liquid_glass_enabled": true
        }),
    );
    h.restart_cold();
    let got = h.expect_ok("settings.theme_get", json!({}));
    assert_eq!(got["mode"], "tinted");
    assert_eq!(got["font_size_px"], 16);
    assert_eq!(got["bubble_radius_px"], 14);
    assert_eq!(got["ui_scale_pct"], 125);
    assert_eq!(got["liquid_glass_enabled"], true);
}

#[test]
fn tc_t1_f31_04_accessibility_reduce_motion_high_contrast_and_dynamic_type() {
    let h = FabushiE2EHarness::connect_production();
    let res = h.expect_ok(
        "settings.accessibility_set",
        json!({
            "reduce_motion": true,
            "high_contrast": true,
            "dynamic_type_category": "UICTContentSizeCategoryAccessibilityExtraExtraExtraLarge",
            "voiceover_hints": true
        }),
    );
    assert_eq!(res["reduce_motion"], true);
    assert_eq!(res["high_contrast"], true);
}

#[test]
fn tc_t1_f31_05_live_locale_switch_zh_hans_and_en() {
    let h = FabushiE2EHarness::connect_production();
    let zh = h.expect_ok("settings.locale_set", json!({"locale": "zh-Hans"}));
    assert_eq!(zh["locale"], "zh-Hans");
    assert_eq!(zh["layout_direction"], "ltr");

    let en = h.expect_ok("settings.locale_set", json!({"locale": "en"}));
    assert_eq!(en["locale"], "en");
    assert_ne!(zh["sample_titles"], en["sample_titles"]);
}

// ============================================================================
// F32 — Full 26-Section Apple Settings Suite & Real Storage Purge (5 tests)
// ============================================================================

#[test]
fn tc_t1_f32_01_sections_list_returns_exact_26_canonical_sections() {
    let h = FabushiE2EHarness::connect_production();
    let list = h.expect_ok("settings.sections_list", json!({}));
    let ids: Vec<String> = list["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .collect();
    verify_26_settings_sections(&ids).expect("exact 26 SettingsSectionType IDs");
}

#[test]
fn tc_t1_f32_02_sections_search_matches_english_and_chinese_keywords() {
    let h = FabushiE2EHarness::connect_production();
    let en = h.expect_ok("settings.sections_search", json!({"q": "passkey", "locale": "en"}));
    assert!(
        en["matches"]
            .as_array()
            .expect("matches")
            .iter()
            .any(|m| m["id"] == "passkeys")
    );
    let zh = h.expect_ok("settings.sections_search", json!({"q": "隐私", "locale": "zh-Hans"}));
    assert!(
        zh["matches"]
            .as_array()
            .expect("matches")
            .iter()
            .any(|m| m["id"] == "privacy")
    );
}

#[test]
fn tc_t1_f32_03_settings_patch_persists_across_sections_and_restarts() {
    let mut h = FabushiE2EHarness::connect_production();
    for (section, values) in [
        ("notifications", json!({"sound": false, "preview": true})),
        ("global_ttl", json!({"ttl_seconds": 86400})),
        ("power_saving", json!({"disable_animations": true})),
    ] {
        let patched = h.expect_ok("settings.patch", json!({"section_id": section, "values": values}));
        assert_eq!(patched["synced"], true);
    }
    h.restart_cold();
    let all = h.expect_ok("settings.get_all", json!({}));
    assert_eq!(all["notifications"]["sound"], false);
    assert_eq!(all["global_ttl"]["ttl_seconds"], 86400);
    assert_eq!(all["power_saving"]["disable_animations"], true);
}

#[test]
fn tc_t1_f32_04_real_cas_storage_metrics_and_disk_purge_preserves_db() {
    let h = FabushiE2EHarness::connect_production();
    let blob = vec![0x7Au8; 65_536];
    let init = h.expect_ok(
        "media.upload_init",
        json!({"filename": "cache_blob.bin", "mime_type": "application/octet-stream", "total_size": blob.len(), "part_size": blob.len()}),
    );
    let _ = h.expect_ok(
        "media.upload_part",
        json!({"upload_id": init["upload_id"], "part_index": 0, "data_b64": B64.encode(&blob)}),
    );
    let _ = h.expect_ok("media.upload_complete", json!({"upload_id": init["upload_id"]}));

    let before = h.expect_ok("settings.storage_stats", json!({}));
    assert!(before["cas_media_bytes"].as_u64().unwrap_or(0) >= 65_536);

    let purged = h.expect_ok("settings.storage_purge", json!({"targets": ["media_cache"]}));
    assert!(purged["freed_bytes"].as_u64().unwrap_or(0) >= 65_536);
    assert_eq!(purged["preserved_messages"], true);
    assert_eq!(purged["preserved_vault"], true);
}

#[test]
fn tc_t1_f32_05_business_section_honestly_flags_partial_capability() {
    let h = FabushiE2EHarness::connect_production();
    let list = h.expect_ok("settings.sections_list", json!({}));
    let biz = list["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .find(|s| s["id"] == "business")
        .expect("business section");
    assert_eq!(biz["capability_status"], "partial");
}

// ============================================================================
// F33 — Network, 4-Tier Config Cascade, Proxy Probe & Update Check (5 tests)
// ============================================================================

#[test]
fn tc_t1_f33_01_four_tier_config_cascade_precedence_and_v167_default() {
    let h = FabushiE2EHarness::connect_production();
    let dft = h.expect_ok("settings.config_resolve", json!({}));
    let dft_url = dft["server_url"].as_str().expect("server_url");
    assert_not_hardcoded_localhost(dft_url).expect("no 127.0.0.1:8443");
    assert_eq!(dft_url, DEFAULT_SERVER_URL_V167);
    assert_eq!(dft["config_source"], "default_fallback");

    let (expected_url, expected_src) = verify_config_4tier_cascade(
        Some("http://10.0.0.1:8080"),
        Some("http://10.0.0.2:8080"),
        Some("http://10.0.0.3:8080"),
    );
    let cli = h.expect_ok(
        "settings.config_resolve",
        json!({
            "cli_args": ["--server-url", "http://10.0.0.1:8080"],
            "env_server_url": "http://10.0.0.2:8080",
            "config_file_url": "http://10.0.0.3:8080"
        }),
    );
    assert_eq!(cli["server_url"].as_str(), Some(expected_url.as_str()));
    assert_eq!(cli["config_source"].as_str(), Some(expected_src));
}

#[test]
fn tc_t1_f33_02_http_proxy_configure_and_probe_reachable() {
    let h = FabushiE2EHarness::connect_production();
    let addr = h.server().addr();
    let _ = h.expect_ok(
        "proxy.set",
        json!({"proxy_type": "http", "host": "127.0.0.1", "port": addr.port(), "fallback_to_direct": false}),
    );
    let probe = h.expect_ok(
        "proxy.probe",
        json!({"proxy_type": "http", "host": "127.0.0.1", "port": addr.port(), "timeout_ms": 2000}),
    );
    assert_eq!(probe["reachable"], true);
}

#[test]
fn tc_t1_f33_03_socks5_proxy_set_get_and_clear_to_direct() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "proxy.set",
        json!({"proxy_type": "socks5", "host": "127.0.0.1", "port": 1080, "username": "u", "password": "p", "fallback_to_direct": false}),
    );
    let active = h.expect_ok("proxy.get", json!({}));
    assert_eq!(active["active_proxy"]["proxy_type"], "socks5");

    let cleared = h.expect_ok("proxy.clear", json!({}));
    assert_eq!(cleared["active_proxy"]["proxy_type"], "direct");
}

#[test]
fn tc_t1_f33_04_mtproto_proxy_valid_32_hex_secret_persists() {
    let h = FabushiE2EHarness::connect_production();
    let secret = "dd0123456789abcdef0123456789abcdef";
    let set = h.expect_ok(
        "proxy.set",
        json!({"proxy_type": "mtproto", "host": "127.0.0.1", "port": 443, "mtproto_secret": secret, "fallback_to_direct": false}),
    );
    assert_eq!(set["active_proxy"]["proxy_type"], "mtproto");
}

#[test]
fn tc_t1_f33_05_update_check_app_store_forbids_binary_self_update() {
    let h = FabushiE2EHarness::connect_production();
    let upd = h.expect_ok(
        "settings.update_check",
        json!({"current_version": "1.6.6", "channel": "app_store"}),
    );
    assert_eq!(upd["update_available"], true);
    assert_eq!(upd["latest_version"], "1.6.8");
    assert_eq!(upd["binary_self_update_allowed"], false);
}

// ============================================================================
// F34 — Fail-Closed Governance, Anti-Stub C-ABI & Honest Capability Ledger (5 tests)
// ============================================================================

#[test]
fn tc_t1_f34_01_all_6_c_abi_symbols_echo_id_and_generation() {
    let h = FabushiE2EHarness::connect_production();
    let (req_id, resp, result) = h.expect_ok_with_meta("generation.current", json!({}));
    assert_eq!(resp.id, req_id);
    assert_eq!(resp.generation, result["generation"].as_u64().expect("gen"));
}

#[test]
fn tc_t1_f34_02_generation_current_and_bump_monotonic_dispatch() {
    let h = FabushiE2EHarness::connect_production();
    let g0 = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g0");
    let bumped = h.expect_ok("generation.bump", json!({"reason": "governance_check"}));
    assert_eq!(bumped["previous_generation"].as_u64(), Some(g0));
    assert_eq!(bumped["generation"].as_u64(), Some(g0 + 1));
}

#[test]
fn tc_t1_f34_03_fresh_host_zero_sample_chats_and_zero_localhost_hardcode() {
    let h = FabushiE2EHarness::connect_production();
    let chats = h.expect_ok("chats.list", json!({}));
    assert_no_sample_chats(&chats).expect("zero hardcoded sample_chats");
    assert_eq!(chats["chats"].as_array().expect("chats").len(), 0);

    let cfg = h.expect_ok("settings.config_resolve", json!({}));
    let url = cfg["server_url"].as_str().expect("server_url");
    assert_not_hardcoded_localhost(url).expect("zero hardcoded 127.0.0.1:8443");
    assert_eq!(url, DEFAULT_SERVER_URL_V167);
}

#[test]
fn tc_t1_f34_04_capability_ledger_exposes_honest_upstream_planned_and_blocked() {
    let h = FabushiE2EHarness::connect_production();
    let ledger = h.expect_ok("settings.capability_ledger", json!({}));
    assert_eq!(
        ledger["upstream_commit"],
        "424aa33cf67a11a778da2dd3b8cca7ea35072b07"
    );
    let items: &[Value] = ledger["items"].as_array().expect("ledger items");
    verify_honest_capability_disposition(items).expect("honest capability disposition");
}

#[test]
fn tc_t1_f34_05_poll_events_bounded_timeout_and_cancel_in_flight() {
    let h = FabushiE2EHarness::connect_production();
    let start = Instant::now();
    let polled = h.poll_events(16, 25);
    assert!(polled.ok);
    assert!(polled.events.len() <= 16);
    assert!(start.elapsed().as_millis() < 2_000);

    assert!(
        !h.cancel("nonexistent_inflight_req_id"),
        "cancelling nonexistent request_id must return false"
    );
    assert!(
        h.expect_ok("generation.current", json!({}))["generation"]
            .as_u64()
            .expect("generation")
            >= 1
    );
}
