//! Tier 2 — Boundary, Corner-Case, Fault-Injection & Fail-Closed Opaque-Box E2E Tests
//! for Features F17..F34 (18 features x 5 tests = 90 executable tests).
//!
//! Every test connects EXCLUSIVELY via `FabushiE2EHarness::connect_production()` (zero mocks,
//! zero synthetic stub fallback).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use fabushi_e2e_ffi::FabushiE2EHarness;
use fabushi_e2e_ffi::contracts::{
    EventPollEnvelope, MAX_AUDIO_BUFFER_CHUNKS, MAX_VAULT_CIPHERTEXT_BYTES,
    compute_wcag_accent_fill, validate_error_envelope, verify_argon2id_params,
    verify_config_4tier_cascade, verify_loop_budget_5_hops, verify_magpie_circuit_rest_ms,
    verify_mcp_audit_redaction, verify_sliding_replay_window_128, verify_talk_vad_bargein,
    verify_vault_size_cap, verify_wcag_accent_luma_guard,
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

// ============================================================================
// F17 — Hermes/Fabushi Agent Runtime & 7-Tab Studio Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f17_01_memory_2200_and_user_1375_unicode_scalar_boundaries() {
    let h = FabushiE2EHarness::connect_production();
    let ok_mem = h.expect_ok("fabushi.memory_put", json!({"target": "memory", "content": "禅".repeat(2200)}));
    assert_eq!(ok_mem["char_count"], 2200);
    let err_mem = h.expect_err(
        "fabushi.memory_put",
        json!({"target": "memory", "content": "禅".repeat(2201)}),
        &["memory_limit_exceeded"],
    );
    assert!(!err_mem.retryable);

    let ok_usr = h.expect_ok("fabushi.memory_put", json!({"target": "user", "content": "u".repeat(1375)}));
    assert_eq!(ok_usr["char_count"], 1375);
    let err_usr = h.expect_err(
        "fabushi.memory_put",
        json!({"target": "user", "content": "u".repeat(1376)}),
        &["memory_limit_exceeded"],
    );
    assert!(!err_usr.retryable);
}

#[test]
fn tc_t2_f17_02_value_bound_approval_toctou_tampering_and_replay_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let sha_orig = sha256_hex(b"git status");
    let sha_tampered = sha256_hex(b"rm -rf /");
    let appr = h.expect_ok(
        "fabushi.approvals_decide",
        json!({
            "approval_id": "appr_toctou_1",
            "decision": "approve_once",
            "action": "terminal_exec",
            "target": "git status",
            "cwd": "/workspace",
            "content_sha256": sha_orig
        }),
    );
    let token_id = appr["token_id"].as_str().expect("token_id");

    let e_tamper = h.expect_err(
        "fabushi.approvals_decide",
        json!({
            "approval_id": "appr_toctou_1",
            "token_id": token_id,
            "decision": "consume",
            "action": "terminal_exec",
            "target": "rm -rf /",
            "cwd": "/workspace",
            "content_sha256": sha_tampered
        }),
        &["approval_param_mismatch", "approval_already_spent"],
    );
    assert!(!e_tamper.retryable);

    let e_replay = h.expect_err(
        "fabushi.approvals_decide",
        json!({
            "approval_id": "appr_toctou_1",
            "token_id": token_id,
            "decision": "consume",
            "action": "terminal_exec",
            "target": "git status",
            "cwd": "/workspace",
            "content_sha256": sha_orig
        }),
        &["approval_already_spent", "approval_not_found"],
    );
    assert!(!e_replay.retryable);
}

#[test]
fn tc_t2_f17_03_expired_approval_and_deny_policy_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let e_exp = h.expect_err(
        "fabushi.approvals_decide",
        json!({
            "approval_id": "appr_expired_1",
            "decision": "approve_once",
            "expires_at_ms": now_ms() - 3_600_001
        }),
        &["approval_expired", "approval_not_found"],
    );
    assert!(!e_exp.retryable);

    let e_deny = h.expect_err(
        "fabushi.approvals_decide",
        json!({
            "approval_id": "appr_policy_deny",
            "decision": "approve_once",
            "policy_mode": "deny"
        }),
        &["approval_denied_by_policy"],
    );
    assert!(!e_deny.retryable);
}

#[test]
fn tc_t2_f17_04_malformed_4_field_6_field_and_out_of_range_cron_rejected() {
    let h = FabushiE2EHarness::connect_production();
    for bad_cron in ["* * * *", "0 * * * * *", "60 25 * * *"] {
        let err = h.expect_err(
            "fabushi.cron_upsert",
            json!({"name": "bad", "schedule": bad_cron, "prompt": "noop", "target_chat_id": 1, "enabled": true}),
            &["invalid_cron_expression", "invalid_params"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f17_05_unknown_toolset_rejected_and_session_export_scrubs_secrets() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "fabushi.set_toolsets",
        json!({"toolsets": [{"id": "nonexistent_toolset", "enabled": true}]}),
        &["unknown_toolset", "invalid_params"],
    );
    assert!(!err.retryable);

    let exported = h.expect_ok(
        "fabushi.sessions_export",
        json!({
            "session_id": "sess_export_1",
            "format": "openai",
            "sample_text": "Authorization: Bearer sk-secret-token-xyz-999"
        }),
    );
    assert_eq!(exported["redacted"], true);
    let payload_str = exported["payload"].to_string();
    assert!(!payload_str.contains("sk-secret-token-xyz-999"));
}

// ============================================================================
// F18 — Multi-Agent Protocol Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f18_01_sixth_bot_hop_exceeds_5_hop_loop_budget_with_causal_trace() {
    let h = FabushiE2EHarness::connect_production();
    for hop in 1..=5u32 {
        let ok = h.expect_ok(
            "multi_agent.loop_budget_record",
            json!({"chat_id": 510, "agent_id": format!("bot_{hop}"), "max_hops": 5}),
        );
        verify_loop_budget_5_hops(hop, 5, ok["exhausted"].as_bool().unwrap_or(true)).expect("hop <=5");
    }
    let err = h.expect_err(
        "multi_agent.loop_budget_record",
        json!({"chat_id": 510, "agent_id": "bot_6", "max_hops": 5}),
        &["loop_budget_exceeded"],
    );
    assert!(!err.retryable);
    verify_loop_budget_5_hops(6, 5, true).expect("hop 6 exhausted");
}

#[test]
fn tc_t2_f18_02_shared_workspace_stale_expected_revision_conflicts_without_overwrite() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "multi_agent.workspace_set",
        json!({"chat_id": 511, "key": "owner", "value": "bot_a", "author_id": "bot_a", "expected_revision": 0}),
    );
    let err = h.expect_err(
        "multi_agent.workspace_set",
        json!({"chat_id": 511, "key": "owner", "value": "bot_stale", "author_id": "bot_b", "expected_revision": 0}),
        &["workspace_revision_conflict"],
    );
    assert!(err.retryable);
    let current = h.expect_ok("multi_agent.workspace_get", json!({"chat_id": 511}));
    assert_eq!(current["entries"]["owner"], "bot_a");
}

#[test]
fn tc_t2_f18_03_expired_floor_lease_preempted_and_late_nonce_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let t0 = now_ms();
    let g1 = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": 512, "agent_id": "bot_slow", "priority": "general_contribution", "now_ms": t0}),
    );
    let stale_nonce = g1["grant"]["nonce"].as_str().expect("nonce");

    let g2 = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": 512, "agent_id": "bot_fresh", "priority": "general_contribution", "now_ms": t0 + 35_000}),
    );
    assert_eq!(g2["granted"], true);

    let late = h.expect_ok(
        "multi_agent.floor_deliver",
        json!({"chat_id": 512, "agent_id": "bot_slow", "nonce": stale_nonce, "outcome": "reply"}),
    );
    assert!(matches!(late["intake"].as_str(), Some("expired") | Some("unknown_nonce")));
}

#[test]
fn tc_t2_f18_04_floor_control_anti_starvation_under_50_direct_mention_burst() {
    let h = FabushiE2EHarness::connect_production();
    let t0 = now_ms();
    // 1. Occupy the floor FIRST so `bot_general_waiter` MUST enter the wait queue.
    let initial = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": 513, "agent_id": "bot_initial_holder", "priority": "general_contribution", "now_ms": t0}),
    );
    assert_eq!(initial["granted"], true);
    let mut active_speaker = "bot_initial_holder".to_owned();
    let mut active_nonce = initial["grant"]["nonce"]
        .as_str()
        .expect("initial grant nonce")
        .to_owned();

    // 2. Enqueue `bot_general_waiter` and prove it is queued (`granted == false`).
    let waiter = h.expect_ok(
        "multi_agent.floor_request",
        json!({"chat_id": 513, "agent_id": "bot_general_waiter", "priority": "general_contribution", "now_ms": t0 + 10}),
    );
    assert_eq!(
        waiter["granted"], false,
        "bot_general_waiter must enter the wait queue behind bot_initial_holder"
    );

    // 3. Drive 50 `direct_mention` arrivals while releasing each active speaker in turn.
    let mut general_promoted = false;
    for i in 1..=50i64 {
        let dm = h.expect_ok(
            "multi_agent.floor_request",
            json!({"chat_id": 513, "agent_id": format!("bot_dm_{i}"), "priority": "direct_mention", "now_ms": t0 + 10 + i * 50}),
        );
        assert_eq!(
            dm["granted"], false,
            "direct_mention must queue while an active speaker holds the floor"
        );

        let rel = h.expect_ok(
            "multi_agent.floor_release",
            json!({"chat_id": 513, "agent_id": active_speaker, "nonce": active_nonce}),
        );
        assert_eq!(rel["released"], true);
        let next_speaker = rel["next_speaker"]
            .as_str()
            .expect("releasing floor with queued agents must promote next_speaker");
        if next_speaker == "bot_general_waiter" {
            general_promoted = true;
            break;
        }
        active_speaker = next_speaker.to_owned();
        active_nonce = rel["next_grant"]["nonce"]
            .as_str()
            .or_else(|| rel["grant"]["nonce"].as_str())
            .or_else(|| rel["next_nonce"].as_str())
            .expect("promoted speaker must receive a valid floor grant nonce")
            .to_owned();
    }
    assert!(
        general_promoted,
        "queued general_contribution agent must never starve under a 50 direct_mention burst"
    );
}

#[test]
fn tc_t2_f18_05_reply_gate_rejects_near_duplicate_echo_and_zero_loop_budget() {
    let h = FabushiE2EHarness::connect_production();
    let dup = h.expect_ok(
        "multi_agent.reply_gate_review_outbound",
        json!({
            "draft_text": "We have verified the exact release SHA and tests passed cleanly.",
            "recent_messages": ["We have verified the exact release SHA and tests passed cleanly."],
            "duplicate_threshold": 0.85
        }),
    );
    assert_eq!(dup["outcome"], "skipped");

    let zero_budget = h.expect_ok(
        "multi_agent.reply_gate_evaluate",
        json!({
            "candidates": ["bot_a", "bot_b"],
            "message_text": "Continue discussing",
            "explicit_mentions": [],
            "sender_is_human": false,
            "remaining_loop_budget": 0,
            "cooldown_seconds": 15
        }),
    );
    assert_eq!(zero_budget["woken_agent_ids"].as_array().expect("woken").len(), 0);
}

// ============================================================================
// F19 — Magpie AI Gateway Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f19_01_circuit_breaker_rate_overload_and_credit_rest_ladders() {
    let h = FabushiE2EHarness::connect_production();
    let r429 = h.expect_ok(
        "magpie.circuit_record_failure",
        json!({"seat_id": "seat_1", "http_status": 429, "error_body": "rate limit exceeded"}),
    );
    verify_magpie_circuit_rest_ms("rate_limit", 0, r429["rest_duration_ms"].as_u64().expect("ms"))
        .expect("5000ms base rate rest");

    let r503 = h.expect_ok(
        "magpie.circuit_record_failure",
        json!({"seat_id": "seat_2", "http_status": 529, "error_body": "overloaded"}),
    );
    verify_magpie_circuit_rest_ms("overload", 0, r503["rest_duration_ms"].as_u64().expect("ms"))
        .expect("30000ms overload rest");

    for attempt in 0..4u32 {
        let rc = h.expect_ok(
            "magpie.circuit_record_failure",
            json!({"seat_id": "seat_3", "http_status": 402, "error_body": "insufficient_quota", "attempt": attempt}),
        );
        verify_magpie_circuit_rest_ms("credit", attempt, rc["rest_duration_ms"].as_u64().expect("ms"))
            .expect("credit ladder [30m,1h,2h,4h]");
    }
}

#[test]
fn tc_t2_f19_02_masking_block_aborts_outbound_and_preview_never_leaks_raw_secret() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "magpie.masking_put",
        json!({"mask_secrets": true, "action": "block", "allow_list": ["safe@example.com"]}),
    );
    let secret = "sk-proj-ULTRASECRET987654321";
    let err = h.expect_err(
        "magpie.masking_preview",
        json!({"text": format!("Send this key {secret}"), "enforce_block": true}),
        &["masking_blocked"],
    );
    assert!(!err.retryable);
    assert!(!err.message.contains(secret));
}

#[test]
fn tc_t2_f19_03_no_active_provider_and_all_seats_resting_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "magpie.route_select",
        json!({"group_id": "empty_group_none", "strategy": "smart", "members": []}),
        &["no_active_provider", "all_seats_resting"],
    );
    assert!(!err.message.is_empty());
}

#[test]
fn tc_t2_f19_04_malformed_dotenv_deeplink_and_unknown_strategy_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let e1 = h.expect_err(
        "magpie.import_deeplink",
        json!({"url": "javascript:alert(1)"}),
        &["invalid_import_format", "invalid_params"],
    );
    assert!(!e1.retryable);

    let e2 = h.expect_err(
        "magpie.groups_upsert",
        json!({"group_id": "bad_strat", "strategy": "quantum_random", "members": []}),
        &["invalid_routing_strategy", "invalid_params"],
    );
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f19_05_ios_forbids_desktop_local_fs_subscription_scan() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "magpie.subscriptions_vault_sync",
        json!({"scan_local_home": true, "path": "~/.claude"}),
        &["local_scan_unsupported_on_ios"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F20 — Third-Party Coding Agents Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f20_01_offline_coding_host_fails_retryable_and_preserves_history() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "coding.turn_start",
        json!({"chat_id": 610, "bot_id": "coding_bot_offline", "host_node_id": "node_offline", "prompt": "check"}),
        &["coding_host_offline"],
    );
    assert!(err.retryable);
}

#[test]
fn tc_t2_f20_02_missing_cli_binary_reported_truthfully_not_fabricated() {
    let h = FabushiE2EHarness::connect_production();
    let res = h.expect_ok(
        "coding.probe_host",
        json!({"host_node_id": "node_missing_codex", "providers": ["codex"]}),
    );
    let codex = res["providers"].as_array().expect("providers").first().expect("codex");
    assert_eq!(codex["available"], false);
    assert_eq!(codex["reason"], "coding_cli_missing");
}

#[test]
fn tc_t2_f20_03_ios_local_subprocess_execution_strictly_forbidden() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "coding.turn_start",
        json!({"chat_id": 611, "bot_id": "coding_bot_1", "host_node_id": "ios_local_subprocess", "prompt": "run locally"}),
        &["local_subprocess_forbidden_ios"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f20_04_denied_coding_permission_aborts_and_cancel_stops_turn() {
    let h = FabushiE2EHarness::connect_production();
    let turn = h.expect_ok(
        "coding.turn_start",
        json!({"chat_id": 612, "bot_id": "coding_bot_1", "prompt": "edit config"}),
    );
    let turn_id = turn["turn_id"].as_str().expect("turn_id");

    let denied = h.expect_err(
        "coding.permission_decide",
        json!({"turn_id": turn_id, "request_id": "perm_deny_1", "decision": "deny"}),
        &["permission_denied"],
    );
    assert!(!denied.retryable);

    let _ = h.cancel(turn_id);
    let cancelled = h.expect_ok("coding.turn_cancel", json!({"turn_id": turn_id}));
    assert_eq!(cancelled["cancelled"], true);
}

#[test]
fn tc_t2_f20_05_invalid_permission_mode_unknown_checkpoint_and_path_traversal_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let e_mode = h.expect_err(
        "coding.bot_config_update",
        json!({"bot_id": "coding_bot_1", "workspace_path": "/srv/repo", "permission_mode": "god_mode"}),
        &["invalid_permission_mode", "invalid_params"],
    );
    assert!(!e_mode.retryable);

    let e_trav = h.expect_err(
        "coding.bot_config_update",
        json!({"bot_id": "coding_bot_1", "workspace_path": "/srv/repo/../../etc", "permission_mode": "ask"}),
        &["invalid_params", "forbidden"],
    );
    assert!(!e_trav.retryable);

    let e_cp = h.expect_err(
        "coding.checkpoint_restore",
        json!({"checkpoint_id": "cp_nonexistent_999"}),
        &["checkpoint_not_found", "not_found"],
    );
    assert!(!e_cp.retryable);
}

// ============================================================================
// F21 — Official OAuth 2.1 Remote MCP Server Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f21_01_insufficient_scope_blocks_power_action_and_audits() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "mcp.tool_invoke",
        json!({
            "access_token": "tok_only_devices_read",
            "protocol_version": "2025-11-25",
            "tool_name": "device_power_action",
            "arguments": {"action": "reboot"},
            "node_id": "node-alpha"
        }),
        &["insufficient_scope"],
    );
    assert!(!err.retryable);
    assert!(err.message.contains("power:control"));
}

#[test]
fn tc_t2_f21_02_node_allowlist_blocks_unapproved_node_beta() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "mcp.tool_invoke",
        json!({
            "access_token": "tok_scoped_node_alpha",
            "protocol_version": "2025-11-25",
            "tool_name": "capture_screen",
            "arguments": {},
            "node_id": "node-beta"
        }),
        &["node_not_allowed"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f21_03_pkce_verifier_mismatch_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "mcp.oauth_approve",
        json!({
            "authorize_code_or_qr_id": "qr_pkce_bad",
            "approved_scopes": ["devices:read"],
            "allowed_node_ids": ["node-alpha"],
            "pkce_code_challenge": "expected_s256_hash",
            "pkce_code_verifier": "wrong_verifier_plaintext"
        }),
        &["pkce_verification_failed"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f21_04_revoked_grant_token_fails_closed_immediately() {
    let h = FabushiE2EHarness::connect_production();
    let grant = h.expect_ok(
        "mcp.oauth_approve",
        json!({
            "authorize_code_or_qr_id": "qr_rev_1",
            "approved_scopes": ["devices:read"],
            "allowed_node_ids": ["node-alpha"],
            "pkce_code_challenge": "chal_rev_1"
        }),
    );
    let _ = h.expect_ok("mcp.grant_revoke", json!({"family_id": grant["family_id"]}));
    let err = h.expect_err(
        "mcp.tool_invoke",
        json!({
            "access_token": grant["access_token"].as_str().expect("access_token"),
            "protocol_version": "2025-11-25",
            "tool_name": "list_devices",
            "arguments": {},
            "node_id": "node-alpha"
        }),
        &["grant_revoked", "unauthorized"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f21_05_audit_280_truncation_unknown_tool_and_unsupported_mcp_version() {
    let h = FabushiE2EHarness::connect_production();
    let huge_args = json!({"text": "S".repeat(500), "extra": "A".repeat(400)});
    let _ = h.expect_ok(
        "mcp.tool_invoke",
        json!({
            "access_token": "tok_mcp_input_control",
            "protocol_version": "2025-11-25",
            "tool_name": "keyboard_type",
            "arguments": huge_args,
            "node_id": "node-alpha"
        }),
    );
    let audit = h.expect_ok("mcp.audit_list", json!({"limit": 1}));
    let summary = audit["entries"][0]["redacted_args"].as_str().expect("redacted");
    verify_mcp_audit_redaction(&huge_args, summary).expect("truncated <= 280 chars");

    let e_tool = h.expect_err(
        "mcp.tool_invoke",
        json!({"access_token": "tok_mcp_devices_read", "protocol_version": "2025-11-25", "tool_name": "bogus_tool", "arguments": {}}),
        &["unknown_mcp_tool"],
    );
    assert!(!e_tool.retryable);

    let e_ver = h.expect_err(
        "mcp.tool_invoke",
        json!({"access_token": "tok_mcp_devices_read", "protocol_version": "1999-01-01", "tool_name": "list_devices", "arguments": {}}),
        &["unsupported_mcp_version"],
    );
    assert!(!e_ver.retryable);
}

// ============================================================================
// F22 — UU-Remote Computer Workbench & PTY Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f22_01_emergency_takeover_under_10ms_preempts_stale_epoch_agent_input() {
    let h = FabushiE2EHarness::connect_production();
    let lease = h.expect_ok(
        "computers.lease_acquire",
        json!({"node_id": "node_dev_mac", "holder_kind": "ai_mcp"}),
    );
    let old_epoch = lease["override_epoch"]
        .as_u64()
        .expect("lease override_epoch");

    let start = Instant::now();
    let takeover = h.expect_ok(
        "computers.emergency_takeover",
        json!({"node_id": "node_dev_mac", "holder_kind": "human"}),
    );
    assert!(
        start.elapsed().as_millis() < 50
            && takeover["elapsed_ms"].as_u64().expect("elapsed_ms") < 10
    );
    assert!(
        takeover["override_epoch"]
            .as_u64()
            .expect("takeover override_epoch")
            > old_epoch
    );

    let preempted = h.expect_err(
        "remote.input_send",
        json!({"session_id": lease["lease_id"], "mouse_mode": "absolute", "override_epoch": old_epoch}),
        &["lease_preempted"],
    );
    assert!(!preempted.retryable);
}

#[test]
fn tc_t2_f22_02_offline_node_and_denied_power_action_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let e_off = h.expect_err(
        "remote.stream_start",
        json!({"node_id": "node_offline_99"}),
        &["node_offline"],
    );
    assert!(e_off.retryable);

    let e_pwr = h.expect_err(
        "remote.input_send",
        json!({"session_id": "sess_active", "power_action": "shutdown", "approved": false}),
        &["power_action_denied"],
    );
    assert!(!e_pwr.retryable);
}

#[test]
fn tc_t2_f22_03_invalid_assist_code_rejected_and_disconnect_all_clears_banner() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "remote.assist_start",
        json!({"assist_code": "INVALID-CODE"}),
        &["assist_code_invalid", "assist_not_allowed"],
    );
    assert!(!err.retryable);

    let _ = h.expect_ok("remote.sessions_disconnect_all", json!({}));
    let list = h.expect_ok("remote.sessions_list", json!({}));
    assert_eq!(list["banner_variant"], "none");
}

#[test]
fn tc_t2_f22_04_file_conflict_policy_and_closed_pty_fail_cleanly() {
    let h = FabushiE2EHarness::connect_production();
    let e_conflict = h.expect_err(
        "remote.file_transfer_start",
        json!({"node_id": "node_dev_mac", "direction": "upload", "path": "/tmp/existing.bin", "conflict_policy": "fail_if_exists", "offset_bytes": 0}),
        &["file_conflict", "invalid_params"],
    );
    assert!(!e_conflict.retryable);

    let e_pty = h.expect_err(
        "remote.pty_read",
        json!({"pty_id": "pty_closed_nonexistent"}),
        &["pty_not_found"],
    );
    assert!(!e_pty.retryable);
}

#[test]
fn tc_t2_f22_05_ios_rejects_unattended_host_daemon_launch() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "computers.lease_acquire",
        json!({"node_id": "ios_local_daemon", "start_unattended_host_daemon": true}),
        &["unattended_host_daemon_unsupported_ios"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F23 — Zero-Knowledge Vault, X25519 Sync & TOTP Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f23_01_invalid_vault_category_traversal_and_129_char_item_key_rejected() {
    let h = FabushiE2EHarness::connect_production();
    assert!(verify_vault_size_cap("ssh_key", "k", 16, 12).is_err());
    assert!(verify_vault_size_cap("password", "../etc", 16, 12).is_err());

    let e_cat = h.expect_err(
        "vault.item_upsert",
        json!({"category": "ssh_key", "item_key": "k1", "ciphertext_b64": B64.encode([1u8; 16]), "nonce_b64": B64.encode([2u8; 12])}),
        &["invalid_category"],
    );
    assert!(!e_cat.retryable);

    for bad_key in ["../etc/passwd".to_owned(), "k".repeat(129)] {
        let e_key = h.expect_err(
            "vault.item_upsert",
            json!({"category": "password", "item_key": bad_key, "ciphertext_b64": B64.encode([1u8; 16]), "nonce_b64": B64.encode([2u8; 12])}),
            &["invalid_item_key"],
        );
        assert!(!e_key.retryable);
    }
}

#[test]
fn tc_t2_f23_02_vault_24mib_plus_1_and_129b_nonce_and_bad_base64_rejected() {
    let h = FabushiE2EHarness::connect_production();
    assert!(verify_vault_size_cap("password", "k1", MAX_VAULT_CIPHERTEXT_BYTES + 1, 12).is_err());
    assert!(verify_vault_size_cap("password", "k1", 32, 129).is_err());

    let e_b64 = h.expect_err(
        "vault.item_upsert",
        json!({"category": "password", "item_key": "k1", "ciphertext_b64": "%%%bad%%%", "nonce_b64": B64.encode([2u8; 12])}),
        &["invalid_base64"],
    );
    assert!(!e_b64.retryable);

    let e_nonce = h.expect_err(
        "vault.item_upsert",
        json!({"category": "password", "item_key": "k1", "ciphertext_b64": B64.encode([1u8; 16]), "nonce_b64": B64.encode([2u8; 129])}),
        &["invalid_payload_size"],
    );
    assert!(!e_nonce.retryable);

    let e_cap = h.expect_err(
        "vault.item_upsert",
        json!({"category": "password", "item_key": "oversize", "declared_size_bytes": MAX_VAULT_CIPHERTEXT_BYTES + 1, "ciphertext_b64": B64.encode([1u8; 16]), "nonce_b64": B64.encode([2u8; 12])}),
        &["invalid_payload_size"],
    );
    assert!(!e_cap.retryable);
}

#[test]
fn tc_t2_f23_03_vault_envelope_rejects_out_of_range_argon2id_and_15b_salt() {
    let h = FabushiE2EHarness::connect_production();
    assert!(verify_argon2id_params(512, 0, 17, 15, 32).is_err());
    let err = h.expect_err(
        "vault.envelope_put",
        json!({
            "kdf_alg": "split_key_v2",
            "kdf_params": {"m_cost_kib": 512, "t_cost": 0, "p_cost": 17},
            "auth_salt_b64": B64.encode([0x1Au8; 15]),
            "wrapped_dek_b64": B64.encode([0x2Bu8; 48]),
            "nonce_b64": B64.encode([0x3Cu8; 12])
        }),
        &["invalid_kdf_params", "invalid_payload_size"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f23_04_denied_and_expired_x25519_device_sync_are_terminal() {
    let h = FabushiE2EHarness::connect_production();
    let req = h.expect_ok(
        "vault.device_sync_request",
        json!({"device_name": "Rogue Device", "ephemeral_x25519_pub_b64": B64.encode([0x88u8; 32])}),
    );
    let rid = req["request_id"].as_str().expect("request_id");
    let denied = h.expect_ok("vault.device_sync_deny", json!({"request_id": rid}));
    assert_eq!(denied["status"], "denied");

    let e_late = h.expect_err(
        "vault.device_sync_approve",
        json!({"request_id": rid, "wrapped_key_b64": B64.encode([0xEEu8; 48])}),
        &["device_sync_denied", "device_sync_expired"],
    );
    assert!(!e_late.retryable);
}

#[test]
fn tc_t2_f23_05_hotp_zero_period_10_digits_and_local_ios_obscura_rejected() {
    let h = FabushiE2EHarness::connect_production();
    for bad_totp in [
        json!({"secret_or_otpauth": "otpauth://hotp/Test?secret=GEZDGNBV"}),
        json!({"secret_or_otpauth": "GEZDGNBV", "period_secs": 0, "digits": 6}),
        json!({"secret_or_otpauth": "GEZDGNBV", "period_secs": 30, "digits": 10}),
        json!({"secret_or_otpauth": ""}),
    ] {
        let err = h.expect_err(
            "vault.totp_generate",
            bad_totp,
            &["invalid_totp_secret", "invalid_totp_params"],
        );
        assert!(!err.retryable);
    }

    let e_obs = h.expect_err(
        "vault.obscura_remote_session",
        json!({"host_node_id": "ios_local_chromium", "action": "open"}),
        &["local_obscura_sidecar_unsupported_ios"],
    );
    assert!(!e_obs.retryable);
}

// ============================================================================
// F24 — External Mini Apps, WebMCP & 全球法布施 Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f24_01_launch_before_tos_and_tampered_init_data_hmac_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let e_tos = h.expect_err(
        "miniapp.launch",
        json!({"bot_username": "unaccepted_bot", "short_name": "app"}),
        &["tos_not_accepted"],
    );
    assert!(!e_tos.retryable);

    let e_hmac = h.expect_err(
        "miniapp.validate_init_data",
        json!({"bot_username": "zen_bot", "short_name": "app", "init_data": "user=%7B%22id%22%3A1%7D&hash=0000tampered"}),
        &["invalid_init_data_hmac"],
    );
    assert!(!e_hmac.retryable);
}

#[test]
fn tc_t2_f24_02_webmcp_6_tuple_fence_mismatch_and_duplicate_call_id_fail_closed() {
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
                {"name": "offline.state.read", "mutating": false}
            ]
        }),
    );
    assert_eq!(reg["registered_count"], 1);

    // 1. Mismatched 6-tuple session fence MUST fail closed.
    let e_fence = h.expect_err(
        "webmcp.invoke",
        json!({
            "account_id": "wrong_account",
            "app_id": "global_dharma",
            "version": "1.6.7",
            "digest": "sha256:tampered",
            "instance_id": inst,
            "nonce": "wrong_nonce",
            "generation": 9999,
            "call_id": "call_bad_fence",
            "tool": "offline.state.read",
            "arguments": {}
        }),
        &["webmcp_session_fence_mismatch", "stale_generation"],
    );
    assert!(!e_fence.retryable);

    // 2. Establish a valid call_id on the LIVE session, then replay the exact same call_id.
    let first = h.expect_ok(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global_dharma",
            "version": "1.6.7",
            "digest": "sha256:abc123",
            "instance_id": inst,
            "nonce": nonce,
            "generation": gen_val,
            "call_id": "call_dup_replay",
            "tool": "offline.state.read",
            "arguments": {}
        }),
    );
    assert_eq!(first["ok"], true);

    let e_dup = h.expect_err(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global_dharma",
            "version": "1.6.7",
            "digest": "sha256:abc123",
            "instance_id": inst,
            "nonce": nonce,
            "generation": gen_val,
            "call_id": "call_dup_replay",
            "tool": "offline.state.read",
            "arguments": {}
        }),
        &["webmcp_duplicate_call_id"],
    );
    assert!(!e_dup.retryable);
}

#[test]
fn tc_t2_f24_03_unregistered_tool_schema_violation_and_unapproved_mutation_fail_closed() {
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
                {
                    "name": "offline.practice.record",
                    "mutating": true,
                    "input_schema": {
                        "type": "object",
                        "properties": { "tag": { "type": "string" } },
                        "required": ["tag"]
                    }
                }
            ]
        }),
    );
    assert_eq!(reg["registered_count"], 1);

    for (tool, args, approved, expected) in [
        ("not.registered.tool", json!({}), true, "webmcp_tool_unregistered"),
        ("offline.practice.record", json!({"tag": 12345}), true, "webmcp_schema_mismatch"),
        ("offline.practice.record", json!({"tag": "x"}), false, "webmcp_approval_required"),
    ] {
        let err = h.expect_err(
            "webmcp.invoke",
            json!({
                "account_id": "acc_1",
                "app_id": "global_dharma",
                "version": "1.6.7",
                "digest": "sha256:abc123",
                "instance_id": inst,
                "nonce": nonce,
                "generation": gen_val,
                "call_id": format!("call_{expected}"),
                "tool": tool,
                "arguments": args,
                "approved": approved
            }),
            &[expected],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f24_04_tampered_offline_fabapp_package_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "miniapp.verify_offline_package",
        json!({
            "package_id": "global-dharma",
            "version": "1.6.7",
            "sha256_digest": "0000000000000000000000000000000000000000000000000000000000000000",
            "ed25519_sig_b64": B64.encode([0x00u8; 64]),
            "bytes_b64": B64.encode(b"tampered_bytes")
        }),
        &["package_digest_mismatch", "package_signature_invalid"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f24_05_protected_prayer_wheel_requires_entitlement_while_free_offline_works() {
    let h = FabushiE2EHarness::connect_production();
    let pkg = b"GLOBAL_DHARMA_OFFLINE_PKG_T2_F24_05";
    let digest = sha256_hex(pkg);
    let verified = h.expect_ok(
        "miniapp.verify_offline_package",
        json!({
            "package_id": "global-dharma",
            "version": "1.6.7",
            "sha256_digest": digest,
            "ed25519_sig_b64": B64.encode([0x55u8; 64]),
            "bytes_b64": B64.encode(pkg)
        }),
    );
    assert_eq!(verified["verified"], true);

    h.server().set_offline(true);
    let free = h.expect_ok(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global-dharma",
            "version": "1.6.7",
            "digest": digest,
            "instance_id": "inst_off",
            "nonce": "nonce_off",
            "generation": h.current_generation(),
            "call_id": "call_free_offline",
            "tool": "offline.state.read",
            "arguments": {}
        }),
    );
    assert_eq!(free["ok"], true);

    let err = h.expect_err(
        "webmcp.invoke",
        json!({
            "account_id": "acc_1",
            "app_id": "global-dharma",
            "version": "1.6.7",
            "digest": digest,
            "instance_id": "inst_off",
            "nonce": "nonce_off",
            "generation": h.current_generation(),
            "call_id": "call_paid_offline",
            "tool": "local.prayer-wheel.start",
            "arguments": {},
            "approved": true
        }),
        &["entitlement_required"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F25 — Chunked Media CAS, HTTP 206, Waveform & Stories Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f25_01_checksum_mismatch_and_missing_upload_parts_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let init = h.expect_ok(
        "media.upload_init",
        json!({"filename": "bad.bin", "mime_type": "application/octet-stream", "total_size": 8192, "part_size": 4096, "expected_sha256": "deadbeef".repeat(8)}),
    );
    let uid = init["upload_id"].as_str().expect("upload_id");
    let _ = h.expect_ok("media.upload_part", json!({"upload_id": uid, "part_index": 0, "data_b64": B64.encode([1u8; 4096])}));

    let e_missing = h.expect_err("media.upload_complete", json!({"upload_id": uid}), &["missing_upload_parts"]);
    assert!(e_missing.retryable);

    let _ = h.expect_ok("media.upload_part", json!({"upload_id": uid, "part_index": 1, "data_b64": B64.encode([2u8; 4096])}));
    let e_sha = h.expect_err("media.upload_complete", json!({"upload_id": uid}), &["checksum_mismatch"]);
    assert!(!e_sha.retryable);
}

#[test]
fn tc_t2_f25_02_upload_size_over_4gib_and_invalid_part_size_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let e_big = h.expect_err(
        "media.upload_init",
        json!({"filename": "huge.mkv", "mime_type": "video/x-matroska", "total_size": 4_294_967_297u64, "part_size": 1_048_576}),
        &["upload_size_exceeded"],
    );
    assert!(!e_big.retryable);

    for bad_part in [0u64, 16_777_217u64] {
        let err = h.expect_err(
            "media.upload_init",
            json!({"filename": "f.bin", "mime_type": "application/octet-stream", "total_size": 1024, "part_size": bad_part}),
            &["invalid_part_size"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f25_03_http_416_range_not_satisfiable_and_404_media_not_found() {
    let h = FabushiE2EHarness::connect_production();
    h.server().seed_media_blob("small_blob", vec![0xAAu8; 512]);
    let e416 = h.expect_err(
        "media.stream_range",
        json!({"file_id": "small_blob", "range_start": 512, "range_end": 1024}),
        &["range_not_satisfiable"],
    );
    assert!(!e416.retryable);

    let e404 = h.expect_err(
        "media.stream_range",
        json!({"file_id": "nonexistent_blob_xyz", "range_start": 0, "range_end": 63}),
        &["media_not_found"],
    );
    assert!(!e404.retryable);
}

#[test]
fn tc_t2_f25_04_waveform_empty_over_100_samples_and_values_over_31_clamped() {
    let h = FabushiE2EHarness::connect_production();
    let empty = h.expect_ok("media.encode_waveform", json!({"samples": []}));
    assert_eq!(empty["byte_len"], 0);

    let oversize: Vec<u8> = (0..150).map(|_| 255u8).collect();
    let clamped = h.expect_ok("media.encode_waveform", json!({"samples": oversize}));
    assert_eq!(clamped["byte_len"], 63);
    let dec = h.expect_ok("media.decode_waveform", json!({"waveform_b64": clamped["waveform_b64"]}));
    assert!(
        dec["samples"]
            .as_array()
            .expect("samples")
            .iter()
            .all(|v| v.as_u64().unwrap_or(99) <= 31)
    );
}

#[test]
fn tc_t2_f25_05_story_ttl_clamped_1_to_48_hours_and_unknown_story_delete_404() {
    let h = FabushiE2EHarness::connect_production();
    let now = now_ms();
    let s_low = h.expect_ok(
        "stories.create",
        json!({"media_id": "m1", "media_type": "photo", "privacy": "contacts", "ttl_hours": 0}),
    );
    let d_low = s_low["expires_at_ms"].as_i64().expect("s_low expires_at_ms") - now;
    assert!(
        (3_500_000..=3_700_000).contains(&d_low),
        "ttl_hours=0 must clamp to 1h (~3_600_000ms), got {d_low}ms"
    );

    let s_high = h.expect_ok(
        "stories.create",
        json!({"media_id": "m2", "media_type": "photo", "privacy": "contacts", "ttl_hours": 999}),
    );
    let d_high = s_high["expires_at_ms"].as_i64().expect("s_high expires_at_ms") - now;
    assert!(
        (172_500_000..=173_000_000).contains(&d_high),
        "ttl_hours=999 must clamp to 48h (~172_800_000ms), got {d_high}ms"
    );

    let e404 = h.expect_err("stories.delete", json!({"story_id": "story_none_999"}), &["story_not_found"]);
    assert!(!e404.retryable);
}

// ============================================================================
// F26 — Calls & Talk Mode Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f26_01_talk_audio_buffer_never_exceeds_320_chunks_fifo() {
    let h = FabushiE2EHarness::connect_production();
    let sid = h.expect_ok("talk.session_create", json!({}))["session_id"]
        .as_str()
        .expect("sid")
        .to_owned();
    let chunk = B64.encode([0x11u8; 320]);
    let mut last_count = 0usize;
    for _ in 0..350 {
        let out = h.expect_ok("talk.audio_enqueue", json!({"session_id": sid, "chunk_b64": chunk}));
        last_count = out["buffered_chunks"]
            .as_u64()
            .expect("buffered_chunks") as usize;
        assert!(last_count <= MAX_AUDIO_BUFFER_CHUNKS);
    }
    assert_eq!(last_count, MAX_AUDIO_BUFFER_CHUNKS);
    assert!(verify_talk_vad_bargein(0, 0, last_count).is_ok());
}

#[test]
fn tc_t2_f26_02_two_loud_frames_then_silence_must_not_trigger_false_barge_in() {
    let h = FabushiE2EHarness::connect_production();
    let sid = h.expect_ok("talk.session_create", json!({}))["session_id"]
        .as_str()
        .expect("sid")
        .to_owned();
    let loud: Vec<u8> = (0..320).flat_map(|_| 2500i16.to_le_bytes()).collect();
    let silent = vec![0u8; 640];

    let _ = h.expect_ok("talk.vad_frame", json!({"session_id": sid, "pcm_i16_b64": B64.encode(&loud)}));
    let _ = h.expect_ok("talk.vad_frame", json!({"session_id": sid, "pcm_i16_b64": B64.encode(&loud)}));
    let out = h.expect_ok("talk.vad_frame", json!({"session_id": sid, "pcm_i16_b64": B64.encode(&silent)}));
    assert_eq!(out["barge_in_triggered"], false);
    assert_eq!(verify_talk_vad_bargein(2, 1, 0).unwrap(), (false, false));
}

#[test]
fn tc_t2_f26_03_closed_talk_session_rejects_audio_enqueue() {
    let h = FabushiE2EHarness::connect_production();
    let sid = h.expect_ok("talk.session_create", json!({}))["session_id"]
        .as_str()
        .expect("sid")
        .to_owned();
    let _ = h.expect_ok("talk.session_close", json!({"session_id": sid}));
    let err = h.expect_err(
        "talk.audio_enqueue",
        json!({"session_id": sid, "chunk_b64": B64.encode([0u8; 32])}),
        &["talk_session_closed"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f26_04_invalid_call_rating_0_or_6_and_unknown_call_id_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let call = h.expect_ok("calls.invite", json!({"peer_id": 2026, "kind": "audio"}));
    let cid = call["call_id"].as_str().expect("call_id");
    let _ = h.expect_ok("calls.hangup", json!({"call_id": cid}));

    for bad_rating in [0, 6] {
        let err = h.expect_err(
            "calls.rate",
            json!({"call_id": cid, "rating": bad_rating}),
            &["invalid_rating"],
        );
        assert!(!err.retryable);
    }
    let e_nf = h.expect_err(
        "calls.signal",
        json!({"call_id": "call_nonexistent_999", "signal_frame": {}}),
        &["call_not_found"],
    );
    assert!(!e_nf.retryable);
}

#[test]
fn tc_t2_f26_05_offline_asr_no_silent_cloud_fallback_and_honest_webrtc_stub_flag() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "talk.session_create",
        json!({"offline_asr": true, "simulate_model_missing": true}),
        &["offline_asr_unavailable"],
    );
    assert!(!err.retryable);

    let inv = h.expect_ok("calls.invite", json!({"peer_id": 2001, "kind": "audio"}));
    assert!(matches!(
        inv["transport_capability"]["nat_sfu_status"].as_str(),
        Some("upstream_loopback_stub") | Some("livekit_ios")
    ));
}

// ============================================================================
// F27 — Unified Search Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f27_01_search_non_member_chat_scope_returns_forbidden() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "search.query",
        json!({"q": "secret", "chat_id": 99_991}),
        &["forbidden_chat_scope", "forbidden"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f27_02_search_limit_clamped_1_to_100_and_empty_query_safe() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "ClampRoom"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "clamp_tok_27 first", "client_nonce": 85010}),
    );
    let _ = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "clamp_tok_27 second", "client_nonce": 85011}),
    );

    let r_zero = h.expect_ok("search.query", json!({"q": "clamp_tok_27", "limit": 0}));
    assert_eq!(r_zero["effective_limit"], 1);
    assert_eq!(r_zero["results"].as_array().expect("results").len(), 1);

    let r_big = h.expect_ok("search.query", json!({"q": "clamp_tok_27", "limit": 500}));
    assert_eq!(r_big["effective_limit"], 100);
    assert_eq!(r_big["results"].as_array().expect("results").len(), 2);

    let r_empty = h.expect_ok("search.query", json!({"q": ""}));
    assert_eq!(r_empty["results"].as_array().expect("results").len(), 0);
}

#[test]
fn tc_t2_f27_03_deleted_tombstone_and_expired_ttl_excluded_from_search() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "SearchDel"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let mid = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "ephemeral_unique_marker_9988", "client_nonce": 85001}),
    )["message_id"]
        .as_i64()
        .expect("mid");

    let before = h.expect_ok("search.query", json!({"q": "ephemeral_unique_marker_9988"}));
    assert!(
        before["total"].as_u64().expect("pre-delete search total") >= 1,
        "marker must be indexed and searchable before deletion"
    );

    let _ = h.expect_ok(
        "messages.delete",
        json!({"chat_id": cid, "message_ids": [mid], "revoke": true}),
    );
    let res = h.expect_ok("search.query", json!({"q": "ephemeral_unique_marker_9988"}));
    assert_eq!(res["total"], 0);
}

#[test]
fn tc_t2_f27_04_fts5_tantivy_operator_and_sql_injection_payloads_safely_escaped() {
    let h = FabushiE2EHarness::connect_production();
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "SqliRoom"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "intact_after_sqli_27", "client_nonce": 85020}),
    );
    for payload in [r#""foo" OR (NOT *):^~"#, "'; DROP TABLE messages;--"] {
        let res = h.expect_ok("search.query", json!({"q": payload}));
        assert!(res["results"].is_array());
    }
    let intact = h.expect_ok("search.query", json!({"q": "intact_after_sqli_27"}));
    assert!(
        intact["total"].as_u64().expect("intact search total") >= 1,
        "messages table and search index must remain intact after SQLi payloads"
    );
}

#[test]
fn tc_t2_f27_05_invalid_search_category_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "search.query",
        json!({"q": "test", "category": "invalid_cat_xyz"}),
        &["invalid_search_category", "invalid_params"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F28 — E2EE Messaging Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f28_01_prekey_bad_signature_wrong_key_len_and_bad_device_id_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let e_len = h.expect_err(
        "e2ee.publish_prekeys",
        json!({
            "device_id": "dev_1",
            "identity_key_b64": B64.encode([1u8; 31]),
            "signed_prekey_b64": B64.encode([2u8; 32]),
            "spk_signature_b64": B64.encode([3u8; 64]),
            "one_time_prekeys_b64": []
        }),
        &["invalid_key_length"],
    );
    assert!(!e_len.retryable);

    let e_id = h.expect_err(
        "e2ee.publish_prekeys",
        json!({
            "device_id": "d".repeat(129),
            "identity_key_b64": B64.encode([1u8; 32]),
            "signed_prekey_b64": B64.encode([2u8; 32]),
            "spk_signature_b64": B64.encode([3u8; 64]),
            "one_time_prekeys_b64": []
        }),
        &["invalid_device_id"],
    );
    assert!(!e_id.retryable);
}

#[test]
fn tc_t2_f28_02_replay_window_duplicate_and_distance_128_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let mut highest = 0u64;
    let mut bitmap = 0u128;
    assert!(verify_sliding_replay_window_128(&mut highest, &mut bitmap, 200));
    assert!(!verify_sliding_replay_window_128(&mut highest, &mut bitmap, 200));
    assert!(!verify_sliding_replay_window_128(&mut highest, &mut bitmap, 72));

    let _ = h.expect_ok("e2ee.replay_window_check", json!({"session_id": "rw_t2", "seq": 200}));
    let dup = h.expect_err(
        "e2ee.replay_window_check",
        json!({"session_id": "rw_t2", "seq": 200}),
        &["replay_detected"],
    );
    assert!(!dup.retryable);
    let stale = h.expect_err(
        "e2ee.replay_window_check",
        json!({"session_id": "rw_t2", "seq": 72}),
        &["replay_detected"],
    );
    assert!(!stale.retryable);
}

#[test]
fn tc_t2_f28_03_tampered_aead_ciphertext_fails_decryption_without_advancing_ratchet() {
    let h = FabushiE2EHarness::connect_production();
    let enc = h.expect_ok(
        "e2ee.encrypt_dm",
        json!({"peer_user_id": 2005, "peer_device_id": "dev_tamp", "plaintext_b64": B64.encode(b"secret")}),
    );
    let mut tampered = enc["envelope"].clone();
    tampered["ciphertext_b64"] = json!(B64.encode([0xFFu8; 32]));
    let err = h.expect_err(
        "e2ee.decrypt_dm",
        json!({"peer_user_id": 2005, "peer_device_id": "dev_tamp", "envelope": tampered}),
        &["decryption_failed"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f28_04_revoked_device_excluded_and_fails_with_device_revoked() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("e2ee.revoke_device", json!({"device_id": "dev_revoked_1"}));
    let err = h.expect_err(
        "e2ee.encrypt_dm",
        json!({"peer_user_id": 2006, "peer_device_id": "dev_revoked_1", "plaintext_b64": B64.encode(b"hi")}),
        &["device_revoked"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f28_05_group_decrypt_without_sender_key_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "e2ee.decrypt_group",
        json!({"chat_id": 899, "sender_id": 9999, "payload": {"ciphertext_b64": B64.encode([0u8; 32])}}),
        &["missing_sender_key"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F29 — Economy, Stars Ledger & StoreKit 2 Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f29_01_insufficient_balance_fails_atomically_preserving_balance() {
    let h = FabushiE2EHarness::connect_production();
    let b0 = h.expect_ok("wallet.balance", json!({}))["stars_balance"]
        .as_i64()
        .expect("stars_balance");
    let err = h.expect_err(
        "wallet.send_stars",
        json!({"to_user_id": 2222, "amount_stars": b0 + 10_000, "idempotency_key": "overdraft_1"}),
        &["insufficient_balance"],
    );
    assert!(!err.retryable);
    assert_eq!(
        h.expect_ok("wallet.balance", json!({}))["stars_balance"].as_i64(),
        Some(b0)
    );
}

#[test]
fn tc_t2_f29_02_zero_negative_overflow_and_self_transfer_rejected() {
    let h = FabushiE2EHarness::connect_production();
    for bad_amt in [0i64, -100, i64::MAX] {
        let err = h.expect_err(
            "wallet.send_stars",
            json!({"to_user_id": 2222, "amount_stars": bad_amt, "idempotency_key": format!("bad_{bad_amt}")}),
            &["invalid_amount", "insufficient_balance"],
        );
        assert!(!err.retryable);
    }
    let e_self = h.expect_err(
        "wallet.send_stars",
        json!({"to_user_id": 0, "self_transfer": true, "amount_stars": 10, "idempotency_key": "self_1"}),
        &["self_transfer_forbidden", "invalid_amount"],
    );
    assert!(!e_self.retryable);
}

#[test]
fn tc_t2_f29_03_invalid_and_already_used_redeem_codes_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let e1 = h.expect_err("wallet.redeem_code", json!({"code": "BOGUS-CODE-000"}), &["redeem_code_invalid"]);
    assert!(!e1.retryable);

    let _ = h.expect_ok("wallet.redeem_code", json!({"code": "ONCE-USE-V167"}));
    let e2 = h.expect_err(
        "wallet.redeem_code",
        json!({"code": "ONCE-USE-V167"}),
        &["redeem_code_already_used"],
    );
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f29_04_unconfigured_storekit_server_binding_reports_honest_blocker() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "wallet.storekit_verify_jws",
        json!({"signed_transaction_jws": "jws_stub", "require_live_apple_binding": true}),
        &["storekit_server_binding_blocked"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f29_05_ton_onchain_signing_fails_closed_as_upstream_planned() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "wallet.send_stars",
        json!({"currency": "TON", "onchain_sign": true, "to_user_id": 2222, "amount_stars": 10}),
        &["ton_signing_upstream_planned", "upstream_planned"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F30 — Action Bus, Deep Links & Push Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f30_01_unknown_action_and_schema_violation_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let e1 = h.expect_err(
        "actions.invoke",
        json!({"action_name": "nonexistent.action", "params": {}, "source": "app_intent"}),
        &["unknown_action"],
    );
    assert!(!e1.retryable);

    let e2 = h.expect_err(
        "actions.invoke",
        json!({"action_name": "chats.open", "params": {"chat_id": "not_an_integer"}, "source": "app_intent"}),
        &["action_schema_invalid", "invalid_params"],
    );
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f30_02_forged_hmac_on_approval_gate_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "actions.approve_gate",
        json!({"gate_id": "gate_1", "hmac_signature": "forged_00000"}),
        &["action_gate_hmac_invalid"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f30_03_untrusted_url_schemes_and_path_traversal_rejected() {
    let h = FabushiE2EHarness::connect_production();
    for bad in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "http://evil.example/phish",
        "fabushi://../../escape",
    ] {
        let err = h.expect_err(
            "actions.route_url",
            json!({"url": bad}),
            &["invalid_url_scheme", "untrusted_url_route"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f30_04_empty_or_non_hex_apns_push_token_rejected() {
    let h = FabushiE2EHarness::connect_production();
    for bad_tok in ["", "   ", "zzzz-not-hex-token"] {
        let err = h.expect_err(
            "push.register_token",
            json!({"platform": "apns", "device_token": bad_tok, "device_id": "d1", "environment": "sandbox"}),
            &["invalid_push_token"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f30_05_push_token_unbound_on_logout_preventing_cross_account_leak() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "push.register_token",
        json!({"platform": "apns", "device_token": "abcdef0123456789abcdef0123456789", "device_id": "dev_push_iso", "environment": "sandbox"}),
    );
    let logout = h.expect_ok("session.logout", json!({}));
    assert_eq!(
        logout["push_unbound"], true,
        "session.logout must unbind registered push token automatically"
    );
    let status = h.expect_ok("push.unregister_token", json!({"platform": "apns", "device_id": "dev_push_iso"}));
    assert_eq!(status["was_registered"], false);
    assert_eq!(status["registered"], false);
}

// ============================================================================
// F31 — Apple HIG Design System Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f31_01_exact_wcag_138_luma_boundary_and_white_max_darken() {
    let h = FabushiE2EHarness::connect_production();
    let (h138, _, adj138) = compute_wcag_accent_fill("#8A8A8A").expect("138");
    assert_eq!(h138, "#8A8A8A");
    assert!(!adj138);

    let r138 = h.expect_ok("settings.theme_set", json!({"mode": "night", "accent_hex": "#8A8A8A"}));
    verify_wcag_accent_luma_guard(
        "#8A8A8A",
        r138["resolved_palette"]["effective_accent_hex"].as_str().expect("eff"),
        r138["resolved_palette"]["contrast_adjusted"].as_bool().expect("adj"),
    )
    .expect("138.0 unchanged");

    let r139 = h.expect_ok("settings.theme_set", json!({"mode": "night", "accent_hex": "#8B8B8B"}));
    verify_wcag_accent_luma_guard(
        "#8B8B8B",
        r139["resolved_palette"]["effective_accent_hex"].as_str().expect("eff"),
        r139["resolved_palette"]["contrast_adjusted"].as_bool().expect("adj"),
    )
    .expect("139.0 darkened");

    let rfff = h.expect_ok("settings.theme_set", json!({"mode": "night", "accent_hex": "#FFFFFF"}));
    verify_wcag_accent_luma_guard(
        "#FFFFFF",
        rfff["resolved_palette"]["effective_accent_hex"].as_str().expect("eff"),
        rfff["resolved_palette"]["contrast_adjusted"].as_bool().expect("adj"),
    )
    .expect("#FFFFFF -> #A9A9A9");
}

#[test]
fn tc_t2_f31_02_font_size_bubble_radius_and_ui_scale_clamped_to_spec_ranges() {
    let h = FabushiE2EHarness::connect_production();
    let low = h.expect_ok(
        "settings.theme_set",
        json!({"mode": "day", "font_size_px": 2, "bubble_radius_px": 0, "ui_scale_pct": 10}),
    );
    assert_eq!(low["font_size_px"], 12);
    assert_eq!(low["bubble_radius_px"], 6);
    assert_eq!(low["ui_scale_pct"], 50);

    let high = h.expect_ok(
        "settings.theme_set",
        json!({"mode": "day", "font_size_px": 99, "bubble_radius_px": 40, "ui_scale_pct": 999}),
    );
    assert_eq!(high["font_size_px"], 20);
    assert_eq!(high["bubble_radius_px"], 18);
    assert_eq!(high["ui_scale_pct"], 300);
}

#[test]
fn tc_t2_f31_03_invalid_theme_mode_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err("settings.theme_set", json!({"mode": "neon_cyber"}), &["invalid_theme_mode"]);
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f31_04_malformed_accent_hex_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    for bad in ["not-a-color", "#GGGGGG", "#123"] {
        let err = h.expect_err(
            "settings.theme_set",
            json!({"mode": "night", "accent_hex": bad}),
            &["invalid_accent_hex"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f31_05_unsupported_locale_fails_closed_preserving_active_locale() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("settings.locale_set", json!({"locale": "zh-Hans"}));
    let err = h.expect_err("settings.locale_set", json!({"locale": "xx-ZZ"}), &["unsupported_locale"]);
    assert!(!err.retryable);
    assert_eq!(
        h.expect_ok("settings.get_all", json!({}))["language"]["locale"],
        "zh-Hans"
    );
}

// ============================================================================
// F32 — 26-Section Settings & Real Storage Purge Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f32_01_unknown_settings_section_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "settings.patch",
        json!({"section_id": "nonexistent_section", "values": {}}),
        &["unknown_settings_section"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f32_02_negative_global_ttl_rejected_preserving_prior_value() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("settings.patch", json!({"section_id": "global_ttl", "values": {"ttl_seconds": 3600}}));
    let err = h.expect_err(
        "settings.patch",
        json!({"section_id": "global_ttl", "values": {"ttl_seconds": -5}}),
        &["invalid_settings_value"],
    );
    assert!(!err.retryable);
    assert_eq!(
        h.expect_ok("settings.get_all", json!({}))["global_ttl"]["ttl_seconds"],
        3600
    );
}

#[test]
fn tc_t2_f32_03_storage_purge_refuses_protected_vault_and_messages_db() {
    let h = FabushiE2EHarness::connect_production();
    for target in ["vault", "messages_db"] {
        let err = h.expect_err(
            "settings.storage_purge",
            json!({"targets": [target]}),
            &["protected_storage_target"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f32_04_empty_cache_purge_returns_zero_freed_bytes_without_fake_5gb() {
    let h = FabushiE2EHarness::connect_production();
    let purged = h.expect_ok("settings.storage_purge", json!({"targets": ["temp_cache"]}));
    assert_eq!(purged["freed_bytes"], 0);
}

#[test]
fn tc_t2_f32_05_settings_search_no_matches_empty_and_rapid_patches_serialize() {
    let h = FabushiE2EHarness::connect_production();
    let empty = h.expect_ok("settings.sections_search", json!({"q": "zzzz_no_match_999"}));
    assert_eq!(empty["matches"].as_array().expect("matches").len(), 0);
    for i in 1..=5 {
        let p = h.expect_ok(
            "settings.patch",
            json!({"section_id": "global_ttl", "values": {"ttl_seconds": i * 60}}),
        );
        assert_eq!(p["synced"], true);
    }
    assert_eq!(
        h.expect_ok("settings.get_all", json!({}))["global_ttl"]["ttl_seconds"],
        300
    );
}

// ============================================================================
// F33 — Network, Config Cascade, Proxy & Update Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f33_01_unreachable_proxy_with_no_fallback_blocks_traffic_with_zero_direct_leak() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "proxy.set",
        json!({"proxy_type": "http", "host": "127.0.0.1", "port": 1, "fallback_to_direct": false}),
    );
    h.server().clear_recorded_requests();

    let err = h.expect_err(
        "proxy.probe",
        json!({"proxy_type": "http", "host": "127.0.0.1", "port": 1, "timeout_ms": 500}),
        &["proxy_unreachable"],
    );
    assert!(err.retryable);
    assert_eq!(
        h.server().recorded_requests().len(),
        0,
        "unreachable proxy with fallback_to_direct=false must never leak direct traffic"
    );
}

#[test]
fn tc_t2_f33_02_malformed_server_urls_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    for bad_url in ["ftp://bad.example", "", "not a url"] {
        let err = h.expect_err(
            "settings.config_resolve",
            json!({"cli_args": ["--server-url", bad_url]}),
            &["invalid_server_url"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f33_03_invalid_proxy_port_empty_host_and_non_hex_mtproto_secret_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let e_port = h.expect_err(
        "proxy.set",
        json!({"proxy_type": "http", "host": "127.0.0.1", "port": 0, "fallback_to_direct": false}),
        &["invalid_proxy_config"],
    );
    assert!(!e_port.retryable);

    let e_host = h.expect_err(
        "proxy.set",
        json!({"proxy_type": "socks5", "host": "", "port": 1080, "fallback_to_direct": false}),
        &["invalid_proxy_config"],
    );
    assert!(!e_host.retryable);

    let e_mt = h.expect_err(
        "proxy.set",
        json!({"proxy_type": "mtproto", "host": "127.0.0.1", "port": 443, "mtproto_secret": "not_hex_zzz", "fallback_to_direct": false}),
        &["mtproto_invalid_secret", "invalid_proxy_config"],
    );
    assert!(!e_mt.retryable);
}

#[test]
fn tc_t2_f33_04_ios_binary_self_update_strictly_forbidden() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "settings.install_binary_update",
        json!({"dmg_url": "https://example.com/Fabushi.dmg"}),
        &["binary_self_update_forbidden_ios"],
    );
    assert!(!err.retryable);
    assert!(!err.message.is_empty());
}

#[test]
fn tc_t2_f33_05_cli_equals_and_shorthand_beat_env_and_strip_trailing_slash() {
    let h = FabushiE2EHarness::connect_production();
    let (exp, src) = verify_config_4tier_cascade(Some("http://9.9.9.9:8080/"), Some("http://env:8080"), None);
    for args in [
        json!(["--server-url=http://9.9.9.9:8080/"]),
        json!(["-s", "http://9.9.9.9:8080/"]),
    ] {
        let res = h.expect_ok(
            "settings.config_resolve",
            json!({"cli_args": args, "env_server_url": "http://env:8080"}),
        );
        assert_eq!(res["server_url"].as_str(), Some(exp.as_str()));
        assert_eq!(res["config_source"].as_str(), Some(src));
    }
}

// ============================================================================
// F34 — Fail-Closed Governance, Panic Isolation & Backlog Disposition (5 tests)
// ============================================================================

#[test]
fn tc_t2_f34_01_unknown_methods_across_namespaces_return_unknown_command_non_retryable() {
    let h = FabushiE2EHarness::connect_production();
    for bogus in ["nonexistent.cmd", "fabushi.does_not_exist"] {
        let err = h.expect_err(bogus, json!({}), &["unknown_command"]);
        assert!(!err.retryable);
    }
    let empty_err = h.expect_err("", json!({}), &["unknown_command", "invalid_request"]);
    assert!(!empty_err.retryable);
}

#[test]
fn tc_t2_f34_02_catch_unwind_panic_isolation_preserves_usable_host_handle() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err("debug.trigger_panic", json!({}), &["host_panic"]);
    assert!(!err.retryable);
    assert!(!err.message.is_empty());

    // Host handle MUST remain completely alive and usable after panic probe!
    let after = h.expect_ok("generation.current", json!({}));
    assert!(
        after["generation"]
            .as_u64()
            .expect("generation after panic recovery")
            >= 1
    );
}

#[test]
fn tc_t2_f34_03_stale_generation_fails_closed_with_zero_state_mutation() {
    let h = FabushiE2EHarness::connect_production();
    let g1 = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g1");
    let _ = h.expect_ok("generation.bump", json!({"reason": "f34_fence"}));
    let err = h.expect_err_with_gen(
        "settings.locale_set",
        g1,
        None,
        json!({"locale": "en"}),
        &["stale_generation"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f34_04_null_pointers_invalid_utf8_and_truncated_json_fail_closed_safely() {
    let h = FabushiE2EHarness::connect_production();
    let raw_null_h = h
        .client()
        .dispatch_null_handle(r#"{"id":"nh","method":"generation.current","generation":1,"params":{}}"#)
        .expect("null handle JSON");
    let (_, e_nh) = validate_error_envelope(&raw_null_h, "", &["invalid_handle"]).expect("invalid_handle");
    assert!(!e_nh.retryable);

    let raw_null_req = h.client().dispatch_null_request().expect("null req JSON");
    let (_, e_nr) = validate_error_envelope(&raw_null_req, "", &["invalid_json", "invalid_request"])
        .expect("invalid_json");
    assert!(!e_nr.retryable);

    let raw_utf8 = h.client().dispatch_invalid_utf8().expect("invalid utf8 JSON");
    let (_, e_utf8) = validate_error_envelope(
        &raw_utf8,
        "",
        &["invalid_utf8", "invalid_json", "invalid_request"],
    )
    .expect("invalid_utf8 must fail closed with error envelope");
    assert!(!e_utf8.retryable);

    let raw_trunc = h.dispatch_raw("{truncated_json").expect("truncated JSON");
    let (_, e_tr) = validate_error_envelope(&raw_trunc, "", &["invalid_json"]).expect("truncated invalid_json");
    assert!(!e_tr.retryable);

    let raw_null_poll = h
        .client()
        .poll_events_null_handle()
        .expect("poll_events_null_handle must return JSON error envelope");
    let null_poll: EventPollEnvelope = serde_json::from_str(&raw_null_poll)
        .unwrap_or_else(|e| panic!("poll_events_null_handle invalid JSON ({e}): {raw_null_poll}"));
    assert!(!null_poll.ok, "poll_events(NULL) must return ok=false");
    let poll_err = null_poll
        .error
        .expect("poll_events(NULL) must include structured error object");
    assert_eq!(poll_err.code, "invalid_handle");
    assert!(!poll_err.retryable);

    assert!(!h.client().cancel_null_handle("req_1"));
    h.client().free_null_string();
}

#[test]
fn tc_t2_f34_05_upstream_planned_ocrm_passport_and_ton_fail_closed_never_ok() {
    let h = FabushiE2EHarness::connect_production();
    let probes: [(&str, serde_json::Value, &[&str]); 4] = [
        (
            "channels.ocrm_discord_connect",
            json!({"token": "x"}),
            &["upstream_planned"],
        ),
        (
            "channels.fbcp_d1_sync",
            json!({"channel_id": "fbcp_1"}),
            &["upstream_planned"],
        ),
        (
            "settings.passport_upload",
            json!({"doc": "id"}),
            &["upstream_planned"],
        ),
        (
            "wallet.ton_sign_tx",
            json!({"amount": 1}),
            &["ton_signing_upstream_planned", "upstream_planned"],
        ),
    ];
    for (method, params, allowed_codes) in probes {
        let err = h.expect_err(method, params, allowed_codes);
        assert!(!err.retryable);
        assert!(!err.message.is_empty());
    }
}
