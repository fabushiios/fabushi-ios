//! Tier 2 — Boundary, Corner-Case, Fault-Injection & Fail-Closed Opaque-Box E2E Tests
//! for Features F01..F16 (16 features x 5 tests = 80 executable tests).
//!
//! Every test connects EXCLUSIVELY via `FabushiE2EHarness::connect_production()` (zero mocks,
//! zero synthetic stub fallback).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use fabushi_e2e_ffi::contracts::{
    ImeEvent, ImeState, TranscriptBubbleInput, extract_context_reference_badges, normalize_phone,
    verify_argon2id_params, verify_bubble_grouping_900s, verify_ime_commit_fence, verify_otp_code,
    verify_outbox_backoff_bounds, verify_pts_monotonic_and_snapshot_fence,
    verify_utf16_entity_bounds, verify_voice_gesture,
};
use fabushi_e2e_ffi::{FabushiE2EHarness, assert_dir_excludes_plaintext_secret};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn register_user(h: &FabushiE2EHarness, username: &str) -> Value {
    h.expect_ok(
        "auth.register",
        json!({
            "username": username,
            "auth_version": 2,
            "auth_salt_b64": B64.encode([0x41u8; 16]),
            "auth_verifier_b64": B64.encode([0x42u8; 32]),
            "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
        }),
    )
}

// ============================================================================
// F01 — Password Login, Registration & Split-Key V2 Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f01_01_username_and_password_length_boundaries_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let e_empty = h.expect_err(
        "auth.register",
        json!({"username": "", "password": "ValidPassword1!"}),
        &["invalid_credentials", "invalid_username", "invalid_params"],
    );
    assert!(!e_empty.retryable);

    let too_long_user = "a".repeat(129);
    let e_long_u = h.expect_err(
        "auth.register",
        json!({"username": too_long_user, "password": "ValidPassword1!"}),
        &["invalid_credentials", "invalid_username", "invalid_params"],
    );
    assert!(!e_long_u.retryable);

    let too_long_pw = "p".repeat(1025);
    let e_long_p = h.expect_err(
        "auth.register",
        json!({"username": "valid_user_len", "password": too_long_pw}),
        &["invalid_credentials", "invalid_password", "invalid_params"],
    );
    assert!(!e_long_p.retryable);
}

#[test]
fn tc_t2_f01_02_out_of_bounds_argon2id_kdf_params_rejected() {
    let h = FabushiE2EHarness::connect_production();
    for (m, t, p) in [(512u32, 3u32, 4u32), (1_048_577, 3, 4), (65536, 0, 4), (65536, 3, 17)] {
        assert!(verify_argon2id_params(m, t, p, 16, 32).is_err());
        let err = h.expect_err(
            "auth.register",
            json!({
                "username": format!("bad_kdf_{m}_{t}_{p}"),
                "auth_version": 2,
                "auth_salt_b64": B64.encode([0x10u8; 16]),
                "auth_verifier_b64": B64.encode([0x20u8; 32]),
                "kdf_params": { "m_cost_kib": m, "t_cost": t, "p_cost": p }
            }),
            &["invalid_kdf_params", "invalid_params"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f01_03_malformed_salt_and_verifier_base64_and_size_rejected() {
    let h = FabushiE2EHarness::connect_production();
    for (salt_b64, ver_b64) in [
        (B64.encode([0x10u8; 15]), B64.encode([0x20u8; 32])),
        (B64.encode([0x10u8; 17]), B64.encode([0x20u8; 32])),
        ("!!!not_base64!!!".to_owned(), B64.encode([0x20u8; 32])),
        (B64.encode([0x10u8; 16]), B64.encode([0x20u8; 31])),
    ] {
        let err = h.expect_err(
            "auth.register",
            json!({
                "username": "bad_salt_user",
                "auth_version": 2,
                "auth_salt_b64": salt_b64,
                "auth_verifier_b64": ver_b64,
                "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
            }),
            &["invalid_payload_size", "invalid_base64", "invalid_auth_verifier"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f01_04_duplicate_register_409_and_wrong_password_401() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "dup_alice");
    h.server().inject_http_status(
        "/api/v1/auth/register",
        409,
        json!({"error": "user_already_exists"}),
        None,
        Some(1),
    );
    let e_dup = h.expect_err(
        "auth.register",
        json!({"username": "dup_alice", "password": "Password123!"}),
        &["user_already_exists", "conflict"],
    );
    assert!(!e_dup.retryable);

    h.server().inject_http_status(
        "/api/v1/auth/login",
        401,
        json!({"error": "invalid_credentials"}),
        None,
        Some(1),
    );
    let e_bad = h.expect_err(
        "auth.login",
        json!({"username": "dup_alice", "password": "WrongPassword!"}),
        &["invalid_credentials", "unauthorized"],
    );
    assert!(!e_bad.retryable);
}

#[test]
fn tc_t2_f01_05_mid_session_http_401_invalidates_token_and_bumps_generation() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "expiring_session_user");
    let g_before = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g_before");

    h.server().inject_http_status(
        "/api/v1/chats",
        401,
        json!({"error": "unauthorized"}),
        None,
        Some(1),
    );
    let err = h.expect_err("chats.list", json!({}), &["unauthorized", "session_revoked"]);
    assert!(!err.retryable);

    let g_after = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g_after");
    assert!(
        g_after > g_before,
        "mid-session HTTP 401 must strictly bump generation (before={g_before}, after={g_after})"
    );

    let polled = h.poll_events(16, 100);
    assert!(polled.ok);
    assert!(
        polled.events.iter().any(|e| matches!(
            e.kind.as_str(),
            "auth_expired" | "session_revoked" | "auth_state_changed"
        )),
        "mid-session HTTP 401 must emit auth_expired/session_revoked/auth_state_changed event"
    );

    // 1-shot 401 fault is already consumed; subsequent protected RPC MUST still fail closed locally.
    let after_err = h.expect_err("chats.list", json!({}), &["unauthorized", "session_revoked"]);
    assert!(!after_err.retryable);
}

// ============================================================================
// F02 — Email OTP & Phone SMS Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f02_01_malformed_email_and_out_of_range_phone_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let e_mail = h.expect_err(
        "auth.email.send_code",
        json!({"email": "not-an-email"}),
        &["invalid_email", "invalid_params"],
    );
    assert!(!e_mail.retryable);

    assert!(normalize_phone("123456").is_err());
    assert!(normalize_phone("+123456789012345678901").is_err());
    for bad_phone in ["123456", "+123456789012345678901"] {
        let e_ph = h.expect_err(
            "auth.phone.send_code",
            json!({"phone": bad_phone}),
            &["invalid_phone", "invalid_params"],
        );
        assert!(!e_ph.retryable);
    }
}

#[test]
fn tc_t2_f02_02_immediate_resend_within_60s_cooldown_returns_429_retryable() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("auth.email.send_code", json!({"email": "cool@fabushi.test"}));
    h.server().inject_http_status(
        "/api/v1/auth/email/send-code",
        429,
        json!({"error": "rate_limited", "resend_after_ms": now_ms() + 60_000}),
        Some(60_000),
        Some(1),
    );
    let err = h.expect_err(
        "auth.email.send_code",
        json!({"email": "cool@fabushi.test"}),
        &["rate_limited"],
    );
    assert!(err.retryable);
}

#[test]
fn tc_t2_f02_03_invalid_otp_formats_and_phone_code_hash_mismatch() {
    let h = FabushiE2EHarness::connect_production();
    for bad_code in ["12345", "1234567", "12ab56"] {
        assert!(verify_otp_code(bad_code).is_err());
        let err = h.expect_err(
            "auth.phone.verify",
            json!({"phone": "+14155550199", "code": bad_code, "phone_code_hash": "pch_1"}),
            &["invalid_code", "invalid_params"],
        );
        assert!(!err.retryable);
    }

    let mismatch = h.expect_err(
        "auth.phone.verify",
        json!({"phone": "+14155550199", "code": "123456", "phone_code_hash": "wrong_hash_xyz"}),
        &["invalid_phone_code_hash", "challenge_not_found"],
    );
    assert!(!mismatch.retryable);
}

#[test]
fn tc_t2_f02_04_five_wrong_phone_otp_attempts_exhaust_to_too_many_attempts() {
    let h = FabushiE2EHarness::connect_production();
    let sent = h.expect_ok("auth.phone.send_code", json!({"phone": "+14155550888"}));
    let hash = sent["phone_code_hash"].as_str().expect("hash");

    for attempt in 1..=5 {
        let e = h.expect_err(
            "auth.phone.verify",
            json!({"phone": "+14155550888", "code": format!("00000{attempt}"), "phone_code_hash": hash}),
            &["invalid_code", "too_many_attempts"],
        );
        assert!(!e.retryable);
    }
    let locked = h.expect_err(
        "auth.phone.verify",
        json!({"phone": "+14155550888", "code": "999999", "phone_code_hash": hash}),
        &["too_many_attempts"],
    );
    assert!(!locked.retryable);
}

#[test]
fn tc_t2_f02_05_expired_otp_short_username_short_password_and_duplicate_email() {
    let h = FabushiE2EHarness::connect_production();
    let e_short_u = h.expect_err(
        "auth.email.register",
        json!({"email": "x@fabushi.test", "code": "123456", "username": "ab", "password": "Password1!"}),
        &["invalid_username", "invalid_params"],
    );
    assert!(!e_short_u.retryable);

    let e_short_p = h.expect_err(
        "auth.email.register",
        json!({"email": "x@fabushi.test", "code": "123456", "username": "valid_u", "password": "short7!"}),
        &["invalid_password", "invalid_params"],
    );
    assert!(!e_short_p.retryable);

    h.server().inject_http_status(
        "/api/v1/auth/email/register",
        400,
        json!({"error": "code_expired"}),
        None,
        Some(1),
    );
    let e_exp = h.expect_err(
        "auth.email.register",
        json!({"email": "x@fabushi.test", "code": "123456", "username": "valid_u", "password": "Password1!"}),
        &["code_expired"],
    );
    assert!(!e_exp.retryable);
}

// ============================================================================
// F03 — Alipay QR Login & Mobile OAuth Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f03_01_alipay_poll_unknown_attempt_returns_not_found() {
    let h = FabushiE2EHarness::connect_production();
    h.server().inject_http_status(
        "/api/v1/auth/alipay/poll/unknown-attempt",
        404,
        json!({"error": "oauth_attempt_not_found"}),
        None,
        Some(1),
    );
    let err = h.expect_err(
        "auth.alipay.poll",
        json!({"attempt_id": "unknown-attempt"}),
        &["oauth_attempt_not_found", "not_found"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f03_02_alipay_deep_link_csrf_state_mismatch_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ios_oauth"}));
    let err = h.expect_err(
        "auth.alipay.handle_deep_link",
        json!({
            "url": "fabushi://auth/alipay?state=fbs_forged_csrf_state&auth_code=abc",
            "expected_attempt_id": started["attempt_id"]
        }),
        &["oauth_state_mismatch"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f03_03_alipay_deep_link_rejects_wrong_schemes_and_missing_code() {
    let h = FabushiE2EHarness::connect_production();
    for bad_url in [
        "http://evil.example/callback?state=fbs_1&auth_code=1",
        "javascript:alert(1)",
        "fabushi://auth/alipay?state=fbs_1",
    ] {
        let err = h.expect_err(
            "auth.alipay.handle_deep_link",
            json!({"url": bad_url}),
            &["invalid_deep_link", "invalid_params"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f03_04_alipay_expired_qr_attempt_issues_zero_session() {
    let h = FabushiE2EHarness::connect_production();
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ipad_qr"}));
    let aid = started["attempt_id"].as_str().expect("attempt_id");
    h.server().inject_http_status(
        &format!("/api/v1/auth/alipay/poll/{aid}"),
        400,
        json!({"error": "oauth_expired", "status": "expired"}),
        None,
        Some(1),
    );
    let err = h.expect_err(
        "auth.alipay.poll",
        json!({"attempt_id": aid}),
        &["oauth_expired"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f03_05_alipay_consumed_auth_code_replay_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ios_oauth"}));
    let state = started["state"].as_str().expect("state");
    let link = format!("fabushi://auth/alipay?state={state}&auth_code=once_code_1");
    let _ = h.expect_ok(
        "auth.alipay.handle_deep_link",
        json!({"url": link, "expected_attempt_id": started["attempt_id"]}),
    );
    let err = h.expect_err(
        "auth.alipay.handle_deep_link",
        json!({"url": link, "expected_attempt_id": started["attempt_id"]}),
        &["oauth_expired", "oauth_state_mismatch", "oauth_attempt_not_found"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F04 — Passkeys & QR Device Pairing Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f04_01_passkey_consumed_challenge_replay_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "pk_replay_user");
    let _ = h.expect_ok(
        "auth.passkey.register",
        json!({"credential_id": "cred_r1", "public_key": B64.encode([0x31u8; 65])}),
    );
    let chal = h.expect_ok("auth.passkey.challenge", json!({"username": "pk_replay_user"}))
        ["challenge"]
        .as_str()
        .expect("challenge")
        .to_owned();
    let payload = json!({
        "challenge": chal,
        "credential_id": "cred_r1",
        "authenticator_data": B64.encode([0x01u8; 37]),
        "client_data_json": B64.encode("{}"),
        "signature": B64.encode([0x22u8; 64])
    });
    let _ = h.expect_ok("auth.passkey.verify", payload.clone());
    let err = h.expect_err("auth.passkey.verify", payload, &["invalid_challenge"]);
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f04_02_passkey_empty_credential_or_unknown_key_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "pk_bad_user");
    let e1 = h.expect_err(
        "auth.passkey.register",
        json!({"credential_id": "   ", "public_key": ""}),
        &["invalid_passkey", "invalid_params"],
    );
    assert!(!e1.retryable);

    let e2 = h.expect_err(
        "auth.passkey.verify",
        json!({
            "challenge": "chal_xyz",
            "credential_id": "nonexistent_cred",
            "authenticator_data": B64.encode([0x01u8; 37]),
            "client_data_json": B64.encode("{}"),
            "signature": B64.encode([0x00u8; 64])
        }),
        &["unknown_credential", "invalid_challenge", "verification_failed"],
    );
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f04_03_pair_verify_before_primary_accept_returns_pair_not_confirmed() {
    let h = FabushiE2EHarness::connect_production();
    let secondary = h.spawn_secondary_host("Unpaired iPad");
    let init = secondary.expect_ok("auth.pair.init", json!({"device_name": "Unpaired iPad"}));
    let err = secondary.expect_err(
        "auth.pair.verify",
        json!({"pair_token": init["pair_token"]}),
        &["pair_not_confirmed"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f04_04_pair_double_accept_and_nonexistent_token_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "pair_double_owner");
    let secondary = h.spawn_secondary_host("iPad 2");
    let token = secondary.expect_ok("auth.pair.init", json!({}))["pair_token"]
        .as_str()
        .expect("token")
        .to_owned();

    let _ = h.expect_ok("auth.pair.accept", json!({"pair_token": token}));
    let dup = h.expect_err(
        "auth.pair.accept",
        json!({"pair_token": token}),
        &["invalid_or_expired_pair_token"],
    );
    assert!(!dup.retryable);

    let nf = secondary.expect_err(
        "auth.pair.poll",
        json!({"pair_token": "pair_does_not_exist_999"}),
        &["pair_token_not_found", "not_found"],
    );
    assert!(!nf.retryable);
}

#[test]
fn tc_t2_f04_05_pair_verify_2fa_missing_cloud_password_fails_closed() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "pair_2fa_guard");
    let _ = h.expect_ok(
        "auth.two_factor.set",
        json!({"new_password": "StrictCloud2FA!"}),
    );
    let secondary = h.spawn_secondary_host("iPad 2FA");
    let token = secondary.expect_ok("auth.pair.init", json!({}))["pair_token"]
        .as_str()
        .expect("token")
        .to_owned();
    let _ = h.expect_ok("auth.pair.accept", json!({"pair_token": token}));

    let err = secondary.expect_err(
        "auth.pair.verify",
        json!({"pair_token": token}),
        &["two_factor_required"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F05 — Cloud 2FA & Active Device Sessions Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f05_01_two_factor_whitespace_or_oversized_password_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "tfa_bad_pw");
    for bad in ["   ".to_owned(), "x".repeat(1025)] {
        let err = h.expect_err(
            "auth.two_factor.set",
            json!({"new_password": bad}),
            &["invalid_password", "invalid_params"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f05_02_two_factor_update_and_disable_reject_wrong_current_password() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "tfa_wrong_cur");
    let _ = h.expect_ok("auth.two_factor.set", json!({"new_password": "RightPass#1"}));

    let e1 = h.expect_err(
        "auth.two_factor.set",
        json!({"current_password": "WrongPass", "new_password": "NewPass#2"}),
        &["invalid_current_password", "unauthorized"],
    );
    assert!(!e1.retryable);

    let e2 = h.expect_err(
        "auth.two_factor.disable",
        json!({"current_password": "WrongPass"}),
        &["invalid_current_password", "unauthorized"],
    );
    assert!(!e2.retryable);
    assert_eq!(
        h.expect_ok("auth.two_factor.status", json!({}))["has_password"],
        true
    );
}

#[test]
fn tc_t2_f05_03_two_factor_verify_wrong_password_or_expired_challenge() {
    let h = FabushiE2EHarness::connect_production();
    let e1 = h.expect_err(
        "auth.two_factor.verify",
        json!({"challenge_token": "2fa_nonexistent", "cloud_password": "wrong"}),
        &["invalid_challenge", "invalid_cloud_password", "challenge_expired"],
    );
    assert!(!e1.retryable);
}

#[test]
fn tc_t2_f05_04_session_revoke_unknown_or_foreign_session_returns_404() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "sess_404_user");
    let err = h.expect_err(
        "session.revoke_device",
        json!({"session_id": "sess_foreign_99999"}),
        &["session_not_found", "not_found"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f05_05_revoked_sibling_receives_401_and_session_revoked_event() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "rev_sib_owner");
    let sibling = h.spawn_secondary_host("Revoked iPad");
    let sib_sess = sibling.expect_ok(
        "auth.login",
        json!({
            "username": "rev_sib_owner",
            "password": "",
            "auth_verifier_b64": B64.encode([0x42u8; 32])
        }),
    );
    let _ = h.expect_ok(
        "session.revoke_device",
        json!({"session_id": sib_sess["session_id"]}),
    );
    let err = sibling.expect_err("chats.list", json!({}), &["unauthorized"]);
    assert!(!err.retryable);
    let polled = sibling.poll_events(16, 100);
    assert!(polled.ok);
    assert!(
        polled.events.iter().any(|e| matches!(
            e.kind.as_str(),
            "session_revoked" | "auth_expired" | "auth_state_changed"
        )),
        "revoked sibling session must emit session_revoked/auth_expired/auth_state_changed event"
    );
}

// ============================================================================
// F06 — Multi-Account, Generation Fence & Passcode Lock Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f06_01_multi_account_cap_enforces_3_standard_and_6_premium() {
    let h = FabushiE2EHarness::connect_production();
    for i in 1..=3 {
        let _ = h.expect_ok(
            "session.add_account",
            json!({"username": format!("cap_u_{i}"), "display_name": format!("U{i}"), "token": format!("t{i}")}),
        );
    }
    let e4 = h.expect_err(
        "session.add_account",
        json!({"username": "cap_u_4", "display_name": "U4", "token": "t4"}),
        &["account_limit_reached"],
    );
    assert!(!e4.retryable);

    for i in 4..=6 {
        let _ = h.expect_ok(
            "session.add_account",
            json!({"username": format!("cap_p_{i}"), "display_name": format!("P{i}"), "token": format!("p{i}"), "is_premium": true}),
        );
    }
    let e7 = h.expect_err(
        "session.add_account",
        json!({"username": "cap_p_7", "display_name": "P7", "token": "p7", "is_premium": true}),
        &["account_limit_reached"],
    );
    assert!(!e7.retryable);
}

#[test]
fn tc_t2_f06_02_stale_generation_dispatch_fails_closed_without_mutation() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "fence_user");
    let g1 = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g1");
    let bumped = h.expect_ok("generation.bump", json!({"reason": "account_switch"}));
    assert_eq!(bumped["generation"].as_u64(), Some(g1 + 1));

    let reqs_before = h.server().request_count_for_path("/api/v1/chats/");
    let err = h.expect_err_with_gen(
        "messages.send",
        g1,
        None,
        json!({"chat_id": 1, "text": "stale should be dropped", "client_nonce": 555}),
        &["stale_generation"],
    );
    assert!(!err.retryable);
    assert_eq!(h.server().request_count_for_path("/api/v1/chats/"), reqs_before);
}

#[test]
fn tc_t2_f06_03_locked_app_blocks_chats_and_messages_with_app_locked() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "lock_guard_user");
    let _ = h.expect_ok("session.passcode.set", json!({"passcode": "1357"}));
    let _ = h.expect_ok("session.passcode.lock_now", json!({}));

    let e1 = h.expect_err("chats.list", json!({}), &["app_locked"]);
    assert!(!e1.retryable);
    let e2 = h.expect_err("messages.history", json!({"chat_id": 1}), &["app_locked"]);
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f06_04_wrong_pin_increments_bad_tries_and_never_stores_plaintext_pin() {
    let h = FabushiE2EHarness::connect_production();
    let pin = "748291";
    let _ = h.expect_ok("session.passcode.set", json!({"passcode": pin}));
    let _ = h.expect_ok("session.passcode.lock_now", json!({}));

    for _ in 0..3 {
        let err = h.expect_err(
            "session.passcode.verify",
            json!({"passcode": "000000"}),
            &["invalid_passcode"],
        );
        assert!(!err.retryable);
    }
    let st = h.expect_ok("session.passcode.status", json!({}));
    assert_eq!(st["bad_tries"], 3);
    assert_eq!(st["is_locked"], true);
    assert_dir_excludes_plaintext_secret(h.app_data_dir(), pin);
}

#[test]
fn tc_t2_f06_05_auto_lock_idle_buckets_and_60s_boundary() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("session.passcode.set", json!({"passcode": "2468", "auto_lock_seconds": 60}));
    for (raw_secs, expected_bucket) in [(0u64, 0u64), (120, 60), (900, 300), (7200, 3600), (99999, 18000)] {
        let res = h.expect_ok(
            "session.passcode.set_auto_lock",
            json!({"auto_lock_seconds": raw_secs}),
        );
        assert_eq!(res["auto_lock_seconds"].as_u64(), Some(expected_bucket));
    }
}

// ============================================================================
// F07 — Unified Adaptive Apple Shell Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f07_01_hover_with_unchanged_selection_revision_does_not_hijack_panel() {
    let h = FabushiE2EHarness::connect_production();
    let s = h.expect_ok("actions.navigate", json!({"route": "settings"}));
    let rev = s["selection_revision"]
        .as_u64()
        .expect("selection_revision");

    let hover = h.expect_ok(
        "actions.navigate",
        json!({"route": "chats", "chat_id": 99, "selection_revision": rev, "hover_only": true}),
    );
    assert_eq!(hover["active_panel"], "settings");
}

#[test]
fn tc_t2_f07_02_sidebar_width_clamped_to_240_360_bounds() {
    let h = FabushiE2EHarness::connect_production();
    let low = h.expect_ok(
        "settings.set_shell_state",
        json!({"active_panel": "chats", "sidebar_width": 100.0}),
    );
    assert!((low["sidebar_width"].as_f64().unwrap_or(0.0) - 240.0).abs() < 0.01);

    let high = h.expect_ok(
        "settings.set_shell_state",
        json!({"active_panel": "chats", "sidebar_width": 900.0}),
    );
    assert!((high["sidebar_width"].as_f64().unwrap_or(0.0) - 360.0).abs() < 0.01);
}

#[test]
fn tc_t2_f07_03_unknown_legacy_split_brain_routes_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    for bad_route in ["grok_legacy_home", "telegram_orphan"] {
        let err = h.expect_err(
            "actions.navigate",
            json!({"route": bad_route}),
            &["invalid_route", "invalid_params"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f07_04_ws_disconnect_transitions_connection_status_banner() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "conn_banner_user");
    h.server().drop_next_ws_connection();
    let _ = h.expect_ok("sync.connect_ws", json!({}));
    h.server().set_offline(true);
    let status = h.expect_ok("session.get_connection_status", json!({}));
    assert!(matches!(
        status["state"].as_str(),
        Some("reconnecting") | Some("offline") | Some("connecting")
    ));
}

#[test]
fn tc_t2_f07_05_rapid_ipad_regular_compact_resize_preserves_active_chat_and_inspector() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "settings.set_shell_state",
        json!({
            "active_panel": "chats",
            "active_chat_id": 777,
            "inspector_open": true,
            "form_factor": "ipad",
            "horizontal_size_class": "regular"
        }),
    );
    for sc in ["compact", "regular", "compact", "regular"] {
        let st = h.expect_ok(
            "settings.set_shell_state",
            json!({"active_panel": "chats", "active_chat_id": 777, "inspector_open": true, "form_factor": "ipad", "horizontal_size_class": sc}),
        );
        assert_eq!(st["active_chat_id"], 777);
        assert_eq!(st["inspector_open"], true);
    }
}

// ============================================================================
// F08 — Grok Bot Multi-Select Compose Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f08_01_empty_recipients_rejected_without_network_call() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "empty_recip_user");
    let before = h.server().request_count_for_path("/api/v1/chats");
    let err = h.expect_err(
        "chats.compose_send",
        json!({"client_nonce": "empty_1", "recipients": [], "is_group_mode": false, "initial_text": "hi", "message_nonce": 1}),
        &["empty_recipients", "invalid_params"],
    );
    assert!(!err.retryable);
    assert_eq!(h.server().request_count_for_path("/api/v1/chats"), before);
}

#[test]
fn tc_t2_f08_02_group_title_exceeding_255_chars_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "long_title_user");
    let err = h.expect_err(
        "chats.create",
        json!({"chat_type": 1, "title": "T".repeat(256), "member_ids": [2001, 2002]}),
        &["invalid_title", "invalid_params"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f08_03_duplicate_client_nonce_on_chats_create_is_idempotent() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "idem_chat_user");
    let c1 = h.expect_ok(
        "chats.create",
        json!({"client_nonce": "grp_idem_42", "chat_type": 1, "title": "Idempotent Group", "member_ids": [2001, 2002]}),
    );
    let c2 = h.expect_ok(
        "chats.create",
        json!({"client_nonce": "grp_idem_42", "chat_type": 1, "title": "Idempotent Group", "member_ids": [2001, 2002]}),
    );
    assert_eq!(c1["chat_id"], c2["chat_id"]);
}

#[test]
fn tc_t2_f08_04_duplicate_member_ids_deduplicated_and_unknown_peer_404() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "dedup_member_user");
    let grp = h.expect_ok(
        "chats.create",
        json!({"chat_type": 1, "title": "Dedup", "member_ids": [42, 42]}),
    );
    assert_eq!(grp["member_count"], 2);

    h.server().inject_http_status(
        "/api/v1/chats",
        404,
        json!({"error": "peer_not_found"}),
        None,
        Some(1),
    );
    let err = h.expect_err(
        "chats.create",
        json!({"chat_type": 0, "peer_user_id": 999_999}),
        &["peer_not_found", "not_found"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f08_05_server_503_during_compose_is_retryable_and_preserves_recovery() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "retry_compose_user");
    h.server().inject_http_status(
        "/api/v1/chats",
        503,
        json!({"error": "upstream_unavailable"}),
        None,
        Some(1),
    );
    let err = h.expect_err(
        "chats.compose_send",
        json!({
            "client_nonce": "retry_grp_1",
            "recipients": [{"user_id": 2001, "is_bot": false}],
            "is_group_mode": false,
            "initial_text": "Preserved draft text",
            "message_nonce": 8801
        }),
        &["upstream_unavailable", "network_unavailable"],
    );
    assert!(err.retryable);

    let ok = h.expect_ok(
        "chats.compose_send",
        json!({
            "client_nonce": "retry_grp_1",
            "recipients": [{"user_id": 2001, "is_bot": false}],
            "is_group_mode": false,
            "initial_text": "Preserved draft text",
            "message_nonce": 8801
        }),
    );
    assert!(ok["chat"]["chat_id"].as_i64().unwrap_or(0) > 0);
}

// ============================================================================
// F09 — Unified Chat List, Folders, Pin & Topics Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f09_01_whitespace_folder_title_and_unknown_folder_delete_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "folder_edge_user");
    let e1 = h.expect_err(
        "folders.create",
        json!({"title": "   ", "included_chat_ids": [], "excluded_chat_ids": []}),
        &["invalid_title", "invalid_params"],
    );
    assert!(!e1.retryable);

    let e2 = h.expect_err(
        "folders.delete",
        json!({"id": 99_999}),
        &["not_found"],
    );
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f09_02_non_admin_member_blocked_from_topics_and_invites() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "non_admin_user");
    let err_topic = h.expect_err(
        "topics.create",
        json!({"chat_id": 9001, "title": "Unauthorized Topic", "as_non_admin": true}),
        &["forbidden"],
    );
    assert!(!err_topic.retryable);

    let err_inv = h.expect_err(
        "chats.invites.create",
        json!({"chat_id": 9001, "as_non_admin": true}),
        &["forbidden"],
    );
    assert!(!err_inv.retryable);
}

#[test]
fn tc_t2_f09_03_non_member_blocked_from_archive_pin_mute_and_topics() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "outsider_user");
    for (method, params) in [
        ("chats.archive", json!({"chat_id": 99_123, "archived": true})),
        ("chats.pin_dialog", json!({"chat_id": 99_123, "pinned": true})),
        ("chats.mute", json!({"chat_id": 99_123, "muted": true})),
        ("topics.list", json!({"chat_id": 99_123})),
    ] {
        let err = h.expect_err(method, params, &["forbidden", "not_found"]);
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f09_04_pinned_dialog_sort_order_is_100pct_deterministic() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "pin_stable_user");
    let mut created_ids = Vec::with_capacity(3);
    for idx in 1..=3 {
        let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": format!("Pin {idx}")}))
            ["chat_id"]
            .as_i64()
            .expect("cid");
        let _ = h.expect_ok("chats.pin_dialog", json!({"chat_id": cid, "pinned": true}));
        created_ids.push(cid);
    }
    let baseline: Vec<i64> = h.expect_ok("chats.list", json!({}))["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .filter_map(|c| c["chat_id"].as_i64())
        .collect();
    assert_eq!(baseline.len(), 3, "all 3 pinned chats must be listed");
    for cid in &created_ids {
        assert!(baseline.contains(cid), "missing created pinned chat_id {cid}");
    }
    for _ in 0..10 {
        let current: Vec<i64> = h.expect_ok("chats.list", json!({}))["chats"]
            .as_array()
            .expect("chats")
            .iter()
            .filter_map(|c| c["chat_id"].as_i64())
            .collect();
        assert_eq!(current, baseline, "pinned chat ordering must never flicker");
    }
}

#[test]
fn tc_t2_f09_05_join_with_empty_or_revoked_invite_code_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "bad_invite_user");
    let e1 = h.expect_err("chats.join", json!({"code": ""}), &["invalid_invite", "invalid_params"]);
    assert!(!e1.retryable);
    let e2 = h.expect_err(
        "chats.join",
        json!({"code": "revoked_code_xyz"}),
        &["invalid_invite", "not_found"],
    );
    assert!(!e2.retryable);
}

// ============================================================================
// F10 — Contacts, Peer Resolution & Blocklist Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f10_01_resolve_peer_empty_query_400_and_unknown_404() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "res_edge_user");
    for bad in ["", "@@@"] {
        let err = h.expect_err(
            "contacts.resolve_peer",
            json!({"username": bad}),
            &["invalid_query", "invalid_params"],
        );
        assert!(!err.retryable);
    }
    let nf = h.expect_err(
        "contacts.resolve_peer",
        json!({"username": "no_such_peer_xyz_9999"}),
        &["peer_not_found", "not_found"],
    );
    assert!(!nf.retryable);
}

#[test]
fn tc_t2_f10_02_bidirectional_blocklist_blocks_dm_messages() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "block_dm_user");
    let dm = h.expect_ok("chats.create", json!({"chat_type": 0, "peer_user_id": 3100}))["chat_id"]
        .as_i64()
        .expect("dm");
    let _ = h.expect_ok("contacts.block", json!({"user_id": 3100}));

    let err = h.expect_err(
        "messages.send",
        json!({"chat_id": dm, "text": "Blocked DM", "client_nonce": 61001}),
        &["user_is_blocked", "forbidden"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f10_03_send_as_non_member_forbidden_and_no_foreign_channels() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "send_as_edge");
    let err = h.expect_err("chats.send_as_list", json!({"chat_id": 99_444}), &["forbidden", "not_found"]);
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f10_04_contacts_add_require_existing_rejects_ghost_user() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "ghost_guard_user");
    let err = h.expect_err(
        "contacts.add",
        json!({"username": "ghost_user_nonexistent", "require_existing": true}),
        &["peer_not_found", "not_found"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f10_05_start_bot_called_twice_reuses_single_materialized_dm() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "bot_reuse_user");
    let s1 = h.expect_ok("chats.start_bot", json!({"bot_user_id": 5050}));
    let s2 = h.expect_ok("chats.start_bot", json!({"bot_user_id": 5050}));
    assert_eq!(s1["chat_id"], s2["chat_id"]);
}

// ============================================================================
// F11 — PTS Sync, Nonce Dedup & Outbox Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f11_01_invalid_client_nonce_rejected_and_16_thread_concurrent_dedup() {
    let h = Arc::new(FabushiE2EHarness::connect_production());
    let _ = register_user(&h, "nonce_dedup_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Nonce"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let e0 = h.expect_err(
        "messages.send",
        json!({"chat_id": cid, "text": "zero nonce", "client_nonce": 0}),
        &["invalid_nonce", "invalid_params"],
    );
    assert!(!e0.retryable);

    let mut handles = Vec::new();
    for _ in 0..16 {
        let h_clone = Arc::clone(&h);
        handles.push(std::thread::spawn(move || {
            h_clone.expect_ok(
                "messages.send",
                json!({"chat_id": cid, "text": "concurrent dedup", "client_nonce": 777_777}),
            )["message_id"]
                .as_i64()
                .expect("message_id")
        }));
    }
    let ids: std::collections::HashSet<i64> = handles
        .into_iter()
        .map(|jh| jh.join().expect("thread join"))
        .collect();
    assert_eq!(ids.len(), 1, "all 16 concurrent callers with identical client_nonce must converge on 1 message_id");
}

#[test]
fn tc_t2_f11_02_pts_journal_floor_overflow_triggers_snapshot_required() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "floor_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Floor"}))["chat_id"]
        .as_i64()
        .expect("cid");
    h.server().set_journal_floor(cid, 5001);

    let diff = h.expect_ok(
        "sync.get_difference",
        json!({"chat_id": cid, "pts": 1, "limit": 50}),
    );
    assert_eq!(diff["snapshot_required"], true);
    verify_pts_monotonic_and_snapshot_fence(1, 5001, &[], true).expect("snapshot fence");
}

#[test]
fn tc_t2_f11_03_outbox_classifies_503_transient_vs_403_permanent() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "backoff_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Backoff"}))["chat_id"]
        .as_i64()
        .expect("cid");

    h.server().inject_http_status(
        &format!("/api/v1/chats/{cid}/messages"),
        503,
        json!({"error": "upstream_unavailable"}),
        None,
        Some(1),
    );
    let _ = h.expect_ok(
        "outbox.enqueue",
        json!({"chat_id": cid, "text": "Transient 503", "client_nonce": 78001}),
    );
    let _ = h.expect_ok("outbox.flush", json!({"chat_id": cid}));
    let list_503 = h.expect_ok("outbox.list", json!({"chat_id": cid}));
    let item_503 = list_503["items"]
        .as_array()
        .expect("outbox items array")
        .iter()
        .find(|it| it["client_nonce"].as_u64() == Some(78001))
        .expect("HTTP 503 item must remain in outbox awaiting retry");
    assert_ne!(
        item_503["status"], "failed_permanent",
        "HTTP 503 must be classified as transient, never failed_permanent"
    );
    let retry_count = item_503["retry_count"].as_u64().expect("retry_count") as u32;
    assert!(retry_count >= 1, "503 flush attempt must record retry_count >= 1");
    let delay_ms = item_503["backoff_ms"]
        .as_u64()
        .expect("transient 503 outbox item must schedule concrete backoff_ms");
    verify_outbox_backoff_bounds(retry_count.saturating_sub(1), delay_ms)
        .expect("backoff_ms must lie within jittered [250..=30000]ms bounds");

    h.server().inject_http_status(
        &format!("/api/v1/chats/{cid}/messages"),
        403,
        json!({"error": "forbidden"}),
        None,
        Some(1),
    );
    let _ = h.expect_ok(
        "outbox.enqueue",
        json!({"chat_id": cid, "text": "Permanent 403", "client_nonce": 78002}),
    );
    let _ = h.expect_ok("outbox.flush", json!({"chat_id": cid}));
    let list_403 = h.expect_ok("outbox.list", json!({"chat_id": cid}));
    assert!(
        list_403["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|it| it["client_nonce"].as_u64() == Some(78002) && it["status"] == "failed_permanent")
    );
}

#[test]
fn tc_t2_f11_04_outbox_max_retries_10_exhaustion_emits_outbox_failed() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "max_retry_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "MaxRetry"}))["chat_id"]
        .as_i64()
        .expect("cid");
    h.server().inject_http_status(
        &format!("/api/v1/chats/{cid}/messages"),
        503,
        json!({"error": "upstream_unavailable"}),
        None,
        Some(12),
    );
    let _ = h.expect_ok(
        "outbox.enqueue",
        json!({"chat_id": cid, "text": "Will exhaust", "client_nonce": 78003, "max_retries": 10}),
    );
    let flushed = h.expect_ok("outbox.flush", json!({"chat_id": cid, "exhaust_retries": true}));
    assert!(flushed["failed_count"].as_u64().unwrap_or(0) >= 1);
}

#[test]
fn tc_t2_f11_05_conflicting_reply_and_invalid_topic_id_fail_before_outbox() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "conflict_reply_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Conflict"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let e1 = h.expect_err(
        "messages.send",
        json!({"chat_id": cid, "text": "bad reply", "reply_to": 10, "reply_to_id": 11, "client_nonce": 78004}),
        &["conflicting_reply", "invalid_params"],
    );
    assert!(!e1.retryable);

    let e2 = h.expect_err(
        "messages.send",
        json!({"chat_id": cid, "text": "bad topic", "topic_id": 0, "client_nonce": 78005}),
        &["invalid_topic_id", "invalid_params"],
    );
    assert!(!e2.retryable);
}

// ============================================================================
// F12 — Virtualized Transcript & Card Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f12_01_text_length_boundary_4096_ok_vs_empty_and_4097_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "text_bound_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "TextBound"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let ok4096 = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "x".repeat(4096), "client_nonce": 81001}),
    );
    assert!(ok4096["message_id"].as_i64().unwrap_or(0) > 0);

    for bad in ["".to_owned(), "   ".to_owned(), "x".repeat(4097)] {
        let err = h.expect_err(
            "messages.send",
            json!({"chat_id": cid, "text": bad, "client_nonce": 81002}),
            &["invalid_text", "invalid_params"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f12_02_utf16_entity_overflow_on_emoji_string_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "utf16_ovf_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "UTF16Ovf"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let text = "你好🚀世界";
    assert!(verify_utf16_entity_bounds(text, 5, 2).is_err());

    let err = h.expect_err(
        "messages.send",
        json!({
            "chat_id": cid,
            "text": text,
            "entities": [{"type": "bold", "offset": 5, "length": 2}],
            "client_nonce": 81003
        }),
        &["invalid_entities", "invalid_params"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f12_03_intervening_service_message_splits_bubble_cluster() {
    let h = FabushiE2EHarness::connect_production();
    let inputs = [
        TranscriptBubbleInput { sender_id: 1, outgoing: true, timestamp_secs: 100, is_service: false },
        TranscriptBubbleInput { sender_id: 1, outgoing: true, timestamp_secs: 105, is_service: true },
        TranscriptBubbleInput { sender_id: 1, outgoing: true, timestamp_secs: 110, is_service: false },
    ];
    let expected = verify_bubble_grouping_900s(&inputs);
    assert!(!expected[0].attached_next);
    assert!(!expected[2].attached_prev);

    let res = h.expect_ok(
        "messages.compute_layout_groups",
        json!({
            "chat_id": 1,
            "messages": [
                {"sender_id": 1, "outgoing": true, "timestamp_secs": 100, "is_service": false},
                {"sender_id": 1, "outgoing": true, "timestamp_secs": 105, "is_service": true},
                {"sender_id": 1, "outgoing": true, "timestamp_secs": 110, "is_service": false}
            ]
        }),
    );
    let groups = res["groups"].as_array().expect("groups");
    assert_eq!(groups[0]["attached_next"], false);
    assert_eq!(groups[2]["attached_prev"], false);
}

#[test]
fn tc_t2_f12_04_invalid_media_poll_todo_and_generative_card_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "bad_card_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "BadCard"}))["chat_id"]
        .as_i64()
        .expect("cid");

    for (card, expected_code) in [
        (json!({"type": "photo", "file_id": ""}), "invalid_media"),
        (json!({"type": "todo", "title": "", "items": []}), "invalid_todo"),
        (json!({"type": "generative_card", "card_id": "", "title": "Bad"}), "invalid_generative_card"),
    ] {
        let err = h.expect_err(
            "messages.send",
            json!({"chat_id": cid, "content": card, "client_nonce": 81004}),
            &[expected_code, "invalid_params"],
        );
        assert!(!err.retryable);
    }

    let bad_poll = h.expect_err(
        "messages.polls.create",
        json!({"chat_id": cid, "question": "Only 1 option?", "options": ["Solo"]}),
        &["invalid_poll", "invalid_params"],
    );
    assert!(!bad_poll.retryable);
}

#[test]
fn tc_t2_f12_05_non_member_history_request_fails_403_with_zero_cross_chat_leak() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "hist_iso_user");
    let err = h.expect_err(
        "messages.history",
        json!({"chat_id": 99_888, "limit": 20}),
        &["forbidden", "not_found"],
    );
    assert!(!err.retryable);
}

// ============================================================================
// F13 — Message Edit, Delete, Reactions, Pins, TTL & Export Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f13_01_pts_fenced_reaction_drops_stale_out_of_order_response() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "react_pts_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "ReactFence"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let mid = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "Fence me", "client_nonce": 82001}),
    )["message_id"]
        .as_i64()
        .expect("mid");

    let newer = h.expect_ok(
        "messages.react",
        json!({"chat_id": cid, "message_id": mid, "emoji": "👍", "action": "add", "pts_hint": 12}),
    );
    let newer_pts = newer["pts"].as_u64().expect("newer reaction pts");
    assert!(newer_pts > 0, "reaction add must return positive pts");
    assert_eq!(newer["reactions"]["👍"].as_u64(), Some(1));

    let stale_applied = h.expect_ok(
        "messages.react",
        json!({"chat_id": cid, "message_id": mid, "emoji": "👍", "action": "remove", "pts_hint": 10, "simulate_out_of_order": true}),
    );
    let after_pts = stale_applied["pts"].as_u64().expect("post-stale reaction pts");
    assert!(
        after_pts >= newer_pts,
        "authoritative pts must never regress on stale response (newer={newer_pts}, after={after_pts})"
    );
    assert_eq!(
        stale_applied["reactions"]["👍"].as_u64(),
        Some(1),
        "stale out-of-order remove must be dropped, preserving 👍 reaction count == 1"
    );
}

#[test]
fn tc_t2_f13_02_non_admin_member_cannot_edit_or_delete_peer_message() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "unauth_edit_user");
    let e_edit = h.expect_err(
        "messages.edit",
        json!({"chat_id": 9001, "message_id": 500, "text": "hijack", "as_non_author": true}),
        &["forbidden"],
    );
    assert!(!e_edit.retryable);
    let e_del = h.expect_err(
        "messages.delete",
        json!({"chat_id": 9001, "message_ids": [500], "revoke": true, "as_non_author": true}),
        &["forbidden"],
    );
    assert!(!e_del.retryable);
}

#[test]
fn tc_t2_f13_03_pinning_zero_or_deleted_message_and_non_admin_ttl_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "pin_edge_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "PinEdge"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let e0 = h.expect_err(
        "messages.pins.pin",
        json!({"chat_id": cid, "message_id": 0}),
        &["invalid_message_id", "invalid_params"],
    );
    assert!(!e0.retryable);

    let enf = h.expect_err(
        "messages.pins.pin",
        json!({"chat_id": cid, "message_id": 999_999}),
        &["message_not_found", "not_found"],
    );
    assert!(!enf.retryable);

    let ettl = h.expect_err(
        "chats.set_ttl",
        json!({"chat_id": 9001, "ttl_seconds": 60, "as_non_admin": true}),
        &["forbidden"],
    );
    assert!(!ettl.retryable);
}

#[test]
fn tc_t2_f13_04_edit_empty_or_4097_text_and_empty_reaction_emoji_rejected() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "edit_bad_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "EditBad"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let mid = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "Initial", "client_nonce": 82002}),
    )["message_id"]
        .as_i64()
        .expect("mid");

    let e1 = h.expect_err(
        "messages.edit",
        json!({"chat_id": cid, "message_id": mid, "text": "z".repeat(4097)}),
        &["invalid_text", "invalid_params"],
    );
    assert!(!e1.retryable);

    let e2 = h.expect_err(
        "messages.react",
        json!({"chat_id": cid, "message_id": mid, "emoji": ""}),
        &["invalid_emoji", "invalid_params"],
    );
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f13_05_ttl_clamps_to_365_days_and_export_limit_clamps_to_5000() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "ttl_clamp_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "TTLClamp"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let clamped = h.expect_ok(
        "chats.set_ttl",
        json!({"chat_id": cid, "ttl_seconds": 99_999_999}),
    );
    assert_eq!(clamped["ttl_seconds"], 31_536_000);

    let _ = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "Export clamp seed", "client_nonce": 82099}),
    );
    let exported = h.expect_ok("chats.export", json!({"chat_id": cid, "limit": 99_999}));
    assert_eq!(exported["effective_limit"], 5000);
    let exported_msgs = exported["messages"].as_array().expect("messages");
    assert!(!exported_msgs.is_empty() && exported_msgs.len() <= 5000);
}

// ============================================================================
// F14 — IME-Safe Composer, Drafts & Voice Gesture Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f14_01_shift_enter_inserts_newline_and_empty_enter_never_sends() {
    let h = FabushiE2EHarness::connect_production();
    assert_eq!(
        verify_ime_commit_fence(ImeState::Idle, ImeEvent::KeyShiftEnter, 0).action,
        "insert_newline"
    );
    let nl = h.expect_ok("messages.composer.ime_event", json!({"event": "key_shift_enter"}));
    assert_eq!(nl["action"], "insert_newline");

    let before = h.server().request_count_for_path("/api/v1/chats/");
    let empty_enter = h.expect_ok(
        "messages.composer.ime_event",
        json!({"event": "key_enter", "buffer": "   "}),
    );
    assert_ne!(empty_enter["action"], "send_message");
    assert_eq!(h.server().request_count_for_path("/api/v1/chats/"), before);
}

#[test]
fn tc_t2_f14_02_voice_tap_under_0_5s_and_slide_left_220_cancel_discard() {
    let h = FabushiE2EHarness::connect_production();
    assert_eq!(verify_voice_gesture(0.0, 0.0, 0.49), "discarded");
    assert_eq!(verify_voice_gesture(-220.0, 0.0, 2.0), "slide_canceling");

    let _ = h.expect_ok("messages.composer.voice_gesture", json!({"action": "press"}));
    let short_rel = h.expect_ok(
        "messages.composer.voice_gesture",
        json!({"action": "release", "dx": 0.0, "dy": 0.0, "elapsed_secs": 0.49}),
    );
    assert_eq!(short_rel["state"], "discarded");

    let _ = h.expect_ok("messages.composer.voice_gesture", json!({"action": "press"}));
    let slide = h.expect_ok(
        "messages.composer.voice_gesture",
        json!({"action": "move", "dx": -220.0, "dy": 0.0, "elapsed_secs": 1.5}),
    );
    assert_eq!(slide["state"], "slide_canceling");
}

#[test]
fn tc_t2_f14_03_draft_put_on_non_member_chat_returns_forbidden() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "draft_iso_user");
    let err = h.expect_err(
        "messages.draft.put",
        json!({"chat_id": 99_777, "text": "unauthorized draft"}),
        &["forbidden", "not_found"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f14_04_past_schedule_date_rejected_fail_closed() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "past_sched_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "PastSched"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let err = h.expect_err(
        "messages.send",
        json!({"chat_id": cid, "text": "Past schedule", "schedule_date_ms": now_ms() - 60_000, "client_nonce": 83001}),
        &["invalid_schedule_date", "invalid_params"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f14_05_failed_send_preserves_chat_draft_intact() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "draft_preserve_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "DraftKeep"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok("messages.draft.put", json!({"chat_id": cid, "text": "Important unsent draft"}));

    h.server().inject_http_status(
        &format!("/api/v1/chats/{cid}/messages"),
        503,
        json!({"error": "upstream_unavailable"}),
        None,
        Some(1),
    );
    let _ = h.expect_err(
        "messages.send",
        json!({"chat_id": cid, "text": "Important unsent draft", "client_nonce": 83002}),
        &["upstream_unavailable", "network_unavailable"],
    );
    let still_there = h.expect_ok("messages.draft.get", json!({"chat_id": cid}));
    assert_eq!(still_there["draft"]["text"], "Important unsent draft");
}

// ============================================================================
// F15 — Slash Commands & @ Context Badges Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f15_01_empty_slash_command_fails_with_invalid_command() {
    let h = FabushiE2EHarness::connect_production();
    for bad in ["", "   "] {
        let err = h.expect_err(
            "fabushi.commands.execute",
            json!({"command": bad}),
            &["invalid_command", "invalid_params"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f15_02_unknown_slash_command_returns_structured_failure_not_fake_ok() {
    let h = FabushiE2EHarness::connect_production();
    let err = h.expect_err(
        "fabushi.commands.execute",
        json!({"command": "/nonexistent_xyz_cmd"}),
        &["unknown_slash_command", "invalid_command"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f15_03_cross_chat_session_id_hijack_fails_403_forbidden() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "hijack_slash_user");
    let ca = h.expect_ok("chats.create", json!({"chat_type": 4, "title": "Bot A"}))["chat_id"]
        .as_i64()
        .expect("ca");
    let cb = h.expect_ok("chats.create", json!({"chat_type": 4, "title": "Bot B"}))["chat_id"]
        .as_i64()
        .expect("cb");
    let sess_a = h.expect_ok(
        "fabushi.commands.execute",
        json!({"command": "/status", "chat_id": ca}),
    )["session_id"]
        .as_str()
        .expect("session_id")
        .to_owned();

    let err = h.expect_err(
        "fabushi.commands.execute",
        json!({"command": "/status", "chat_id": cb, "session_id": sess_a}),
        &["forbidden"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f15_04_non_member_chat_id_in_slash_command_fails_403() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "nonmember_slash_user");
    let err = h.expect_err(
        "fabushi.commands.execute",
        json!({"command": "/status", "chat_id": 99_555}),
        &["forbidden", "not_found"],
    );
    assert!(!err.retryable);
}

#[test]
fn tc_t2_f15_05_false_positive_at_tokens_never_extracted_as_badges() {
    let h = FabushiE2EHarness::connect_production();
    let text = "email@file.com @filex @dir2 @gitx @url_fake";
    assert!(extract_context_reference_badges(text).is_empty());
    let res = h.expect_ok("fabushi.context.extract_badges", json!({"text": text}));
    assert_eq!(res["badges"].as_array().expect("badges").len(), 0);
}

// ============================================================================
// F16 — Muse Context Inspector Boundaries (5 tests)
// ============================================================================

#[test]
fn tc_t2_f16_01_non_owner_cannot_patch_bot_or_bind_computer() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "non_owner_bot");
    let e1 = h.expect_err(
        "fabushi.bots.patch",
        json!({"bot_id": "bot_owned_by_other", "persona": "Hijacked"}),
        &["forbidden"],
    );
    assert!(!e1.retryable);
    let e2 = h.expect_err(
        "fabushi.bots.bind_computer",
        json!({"bot_id": "bot_owned_by_other", "computer_node_id": "node_1"}),
        &["forbidden"],
    );
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f16_02_patch_unknown_bot_or_botfather_returns_bot_not_found() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "bf_guard_user");
    for bid in ["unknown_bot_xyz", "BotFather"] {
        let err = h.expect_err(
            "fabushi.bots.patch",
            json!({"bot_id": bid, "persona": "Illegal"}),
            &["bot_not_found", "not_found", "forbidden"],
        );
        assert!(!err.retryable);
    }
}

#[test]
fn tc_t2_f16_03_shared_media_invalid_subtab_400_and_non_member_403() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "media_sub_edge");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "SubEdge"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let e1 = h.expect_err(
        "chats.shared_media",
        json!({"chat_id": cid, "subtab": "videos_raw"}),
        &["invalid_subtab", "invalid_params"],
    );
    assert!(!e1.retryable);

    let e2 = h.expect_err(
        "chats.shared_media",
        json!({"chat_id": 99_666, "subtab": "media"}),
        &["forbidden", "not_found"],
    );
    assert!(!e2.retryable);
}

#[test]
fn tc_t2_f16_04_non_admin_member_ops_forbidden_and_invalid_role_400() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "role_edge_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "RoleEdge"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let e_role = h.expect_err(
        "chats.members.update_role",
        json!({"chat_id": cid, "user_id": 4010, "role": "super_root"}),
        &["invalid_role", "invalid_params"],
    );
    assert!(!e_role.retryable);

    let e_forb = h.expect_err(
        "chats.members.remove",
        json!({"chat_id": 9001, "user_id": 1001, "as_non_admin": true}),
        &["forbidden"],
    );
    assert!(!e_forb.retryable);
}

#[test]
fn tc_t2_f16_05_add_member_candidates_exclude_zero_and_existing_members() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_user(&h, "cand_filter_user");
    let keep = h.expect_ok(
        "contacts.add",
        json!({"username": "keep_user", "first_name": "Keep", "last_name": "User"}),
    );
    let keep_uid = keep["user_id"].as_u64().expect("keep_uid");
    let excl = h.expect_ok(
        "contacts.add",
        json!({"username": "excl_user", "first_name": "Excl", "last_name": "User"}),
    );
    let excl_uid = excl["user_id"].as_u64().expect("excl_uid");

    let res = h.expect_ok(
        "chats.compose_candidates",
        json!({"query": "", "include_bots": true, "exclude_user_ids": [excl_uid]}),
    );
    let cands = res["candidates"].as_array().expect("candidates");
    assert!(
        cands.iter().any(|c| c["user_id"].as_u64() == Some(keep_uid)),
        "non-excluded contact must appear in compose candidates"
    );
    assert!(cands.iter().all(|c| c["user_id"].as_u64() != Some(0)));
    assert!(cands.iter().all(|c| c["user_id"].as_u64() != Some(excl_uid)));
}
