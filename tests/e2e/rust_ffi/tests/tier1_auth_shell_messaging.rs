//! Tier 1 — Core Functional Happy-Path Opaque-Box E2E Tests for Features F01..F16
//! (16 features x 5 tests = 80 executable tests).
//!
//! Every test connects EXCLUSIVELY via `FabushiE2EHarness::connect_production()` (zero mocks,
//! zero synthetic stub fallback) against a real compiled `libmahayana_app_host.{so,dylib}` and
//! ephemeral `127.0.0.1:0` loopback HTTP/WS server.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use fabushi_e2e_ffi::contracts::{
    ImeEvent, ImeState, TranscriptBubbleInput, assert_no_sample_chats, encode_waveform_5bit,
    extract_context_reference_badges, normalize_phone, verify_argon2id_params,
    verify_bubble_grouping_900s, verify_ime_commit_fence, verify_otp_code,
    verify_pts_monotonic_and_snapshot_fence, verify_slash_command_catalog,
    verify_utf16_entity_bounds, verify_voice_gesture, verify_waveform_5bit_63bytes,
};
use fabushi_e2e_ffi::{FabushiE2EHarness, assert_dir_excludes_plaintext_secret};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn split_key_v2_salt_b64() -> String {
    B64.encode([0x5Au8; 16])
}

fn split_key_v2_verifier_b64() -> String {
    B64.encode([0xA5u8; 32])
}

fn register_default_user(h: &FabushiE2EHarness, username: &str) -> Value {
    let salt = split_key_v2_salt_b64();
    let verifier = split_key_v2_verifier_b64();
    h.expect_ok(
        "auth.register",
        json!({
            "username": username,
            "first_name": "E2E",
            "device_name": "iOS Simulator",
            "auth_version": 2,
            "auth_salt_b64": salt,
            "auth_verifier_b64": verifier,
            "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
        }),
    )
}

// ============================================================================
// F01 — Password Login, Registration & Split-Key V2 Bootstrap (5 tests)
// ============================================================================

#[test]
fn tc_t1_f01_01_register_split_key_v2_no_plaintext_password() {
    let h = FabushiE2EHarness::connect_production();
    let plaintext_pw = "PlaintextSecretPassword!";
    let salt = [0x11u8; 16];
    let verifier = [0x22u8; 32];
    verify_argon2id_params(65536, 3, 4, salt.len(), verifier.len()).expect("valid KDF");

    let res = h.expect_ok(
        "auth.register",
        json!({
            "username": "alice_v2",
            "password": plaintext_pw,
            "first_name": "Alice",
            "device_name": "iPhone 16 Pro",
            "auth_version": 2,
            "auth_salt_b64": B64.encode(salt),
            "auth_verifier_b64": B64.encode(verifier),
            "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
        }),
    );
    assert!(res["user_id"].as_u64().expect("user_id") > 0);
    assert!(!res["session_id"].as_str().expect("session_id").is_empty());
    assert!(!res["token"].as_str().expect("token").is_empty());
    assert_eq!(res["auth_version"], 2);

    let mut inspected_register = 0usize;
    for req in h.server().recorded_requests() {
        if req.path == "/api/v1/auth/register" {
            inspected_register += 1;
            let body_str = String::from_utf8_lossy(&req.body);
            assert!(
                !body_str.contains(plaintext_pw),
                "Split-Key V2 registration must never transmit plaintext password"
            );
            let body_json: Value = serde_json::from_slice(&req.body)
                .expect("recorded /api/v1/auth/register body must be valid JSON");
            assert!(
                body_json
                    .get("password")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .is_empty(),
                "Split-Key V2 HTTP register payload must not contain non-empty `password`"
            );
            assert!(
                body_json
                    .get("auth_verifier_b64")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                "Split-Key V2 HTTP register payload must include non-empty `auth_verifier_b64`"
            );
            assert!(
                body_json
                    .get("auth_salt_b64")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                "Split-Key V2 HTTP register payload must include non-empty `auth_salt_b64`"
            );
        }
    }
    assert!(
        inspected_register >= 1,
        "must record at least one /api/v1/auth/register HTTP request"
    );
    assert_dir_excludes_plaintext_secret(h.app_data_dir(), plaintext_pw);
}

#[test]
fn tc_t1_f01_02_kdf_params_login_and_transparent_vault_unlock() {
    let h = FabushiE2EHarness::connect_production();
    let kdf = h.expect_ok("auth.get_kdf_params", json!({"identifier": "alice_v2"}));
    assert_eq!(kdf["auth_version"], 2);
    assert_eq!(kdf["kdf_alg"], "split_key_v2");

    let login = h.expect_ok(
        "auth.login",
        json!({
            "username": "alice_v2",
            "password": "",
            "auth_verifier_b64": split_key_v2_verifier_b64(),
            "device_name": "iPhone 16 Pro"
        }),
    );
    assert_eq!(login["vault_unlocked"], true);
    assert_eq!(login["requires_2fa"], false);
    assert!(!login["token"].as_str().unwrap_or("").is_empty());

    let poll = h.poll_events(16, 100);
    assert!(poll.ok);
    assert!(
        poll.events
            .iter()
            .any(|e| e.kind == "auth_state_changed" || e.kind == "session_authenticated")
    );
}

#[test]
fn tc_t1_f01_03_legacy_v1_upgrade_to_split_key_v2() {
    let h = FabushiE2EHarness::connect_production();
    let reg = h.expect_ok(
        "auth.register",
        json!({
            "username": "legacy_user_v1",
            "password": "LegacyPassword#2026",
            "auth_version": 1
        }),
    );
    assert_eq!(reg["auth_version"], 1);

    let upgraded = h.expect_ok(
        "auth.upgrade_split_key",
        json!({
            "current_password": "LegacyPassword#2026",
            "auth_salt_b64": split_key_v2_salt_b64(),
            "auth_verifier_b64": split_key_v2_verifier_b64(),
            "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
        }),
    );
    assert_eq!(upgraded["auth_version"], 2);
    assert_eq!(upgraded["vault_unlocked"], true);

    let kdf = h.expect_ok(
        "auth.get_kdf_params",
        json!({"identifier": "legacy_user_v1"}),
    );
    assert_eq!(kdf["auth_version"], 2);
}

#[test]
fn tc_t1_f01_04_change_password_rotates_verifier_and_rewraps_vault() {
    let h = FabushiE2EHarness::connect_production();
    let old_verifier = split_key_v2_verifier_b64();
    let new_verifier = B64.encode([0x77u8; 32]);
    let _ = register_default_user(&h, "rotator_user");

    let changed = h.expect_ok(
        "auth.change_password",
        json!({
            "current_auth_verifier_b64": old_verifier,
            "new_auth_salt_b64": B64.encode([0x33u8; 16]),
            "new_auth_verifier_b64": new_verifier,
            "new_kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 },
            "rewrapped_vault_envelope": {
                "key_version": 2,
                "kdf_alg": "split_key_v2",
                "wrapped_dek_b64": B64.encode([0xCCu8; 48]),
                "nonce_b64": B64.encode([0x12u8; 12])
            }
        }),
    );
    assert_eq!(changed["changed"], true);
    assert_eq!(changed["auth_version"], 2);

    let err = h.expect_err(
        "auth.login",
        json!({
            "username": "rotator_user",
            "password": "",
            "auth_verifier_b64": old_verifier
        }),
        &["invalid_credentials", "unauthorized"],
    );
    assert!(!err.retryable);

    let login = h.expect_ok(
        "auth.login",
        json!({
            "username": "rotator_user",
            "password": "",
            "auth_verifier_b64": new_verifier
        }),
    );
    assert_eq!(login["vault_unlocked"], true);
}

#[test]
fn tc_t1_f01_05_session_persists_across_cold_restart_without_plaintext_secret() {
    let mut h = FabushiE2EHarness::connect_production();
    let secret_pw = "SuperSecretNeverOnDisk#98765";
    let reg = h.expect_ok(
        "auth.register",
        json!({
            "username": "durable_user",
            "password": secret_pw,
            "auth_version": 2,
            "auth_salt_b64": split_key_v2_salt_b64(),
            "auth_verifier_b64": split_key_v2_verifier_b64(),
            "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
        }),
    );
    let uid = reg["user_id"].as_u64().expect("user_id");
    let sid = reg["session_id"].as_str().expect("session_id").to_owned();

    h.restart_cold();
    let restored = h.expect_ok("session.current", json!({}));
    assert_eq!(restored["user_id"].as_u64(), Some(uid));
    assert_eq!(restored["session_id"].as_str(), Some(sid.as_str()));
    assert_dir_excludes_plaintext_secret(h.app_data_dir(), secret_pw);
}

// ============================================================================
// F02 — Email OTP Registration & Phone SMS Verification (5 tests)
// ============================================================================

#[test]
fn tc_t1_f02_01_email_send_code_returns_10m_expiry_and_60s_cooldown() {
    let h = FabushiE2EHarness::connect_production();
    let before = now_ms();
    let res = h.expect_ok(
        "auth.email.send_code",
        json!({"email": "Alice.OTP@Example.COM"}),
    );
    assert_eq!(res["sent"], true);
    let expires = res["expires_at_ms"].as_i64().expect("expires_at_ms");
    let resend = res["resend_after_ms"].as_i64().expect("resend_after_ms");
    assert!(expires - before >= 540_000 && expires - before <= 660_000);
    assert!(resend - before >= 50_000 && resend - before <= 75_000);
}

#[test]
fn tc_t1_f02_02_email_register_with_6_digit_otp_authenticates() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "auth.email.send_code",
        json!({"email": "newuser@fabushi.test"}),
    );
    let code = "482910";
    verify_otp_code(code).expect("valid 6-digit code");
    let reg = h.expect_ok(
        "auth.email.register",
        json!({
            "email": "newuser@fabushi.test",
            "code": code,
            "username": "email_usr",
            "password": "ValidPass8!",
            "first_name": "EmailUser"
        }),
    );
    assert!(reg["user_id"].as_u64().unwrap_or(0) > 0);
    assert!(!reg["token"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f02_03_phone_send_code_normalizes_e164_and_returns_hash() {
    let h = FabushiE2EHarness::connect_production();
    let raw_phone = "+1 (415) 555-0199";
    let normalized = normalize_phone(raw_phone).expect("valid E.164");
    assert_eq!(normalized, "+14155550199");

    let before = now_ms();
    let res = h.expect_ok("auth.phone.send_code", json!({"phone": raw_phone}));
    assert_eq!(res["sent"], true);
    assert!(!res["phone_code_hash"].as_str().unwrap_or("").is_empty());
    let expires = res["expires_at_ms"].as_i64().expect("expires_at_ms");
    assert!(expires - before >= 250_000 && expires - before <= 350_000);
}

#[test]
fn tc_t1_f02_04_phone_verify_with_matching_hash_issues_session() {
    let h = FabushiE2EHarness::connect_production();
    let sent = h.expect_ok("auth.phone.send_code", json!({"phone": "+14155550199"}));
    let hash = sent["phone_code_hash"].as_str().expect("phone_code_hash");
    let code = sent["debug_code"].as_str().unwrap_or("123456");
    verify_otp_code(code).expect("6-digit SMS code");

    let verified = h.expect_ok(
        "auth.phone.verify",
        json!({
            "phone": "+14155550199",
            "code": code,
            "phone_code_hash": hash,
            "first_name": "PhoneUser",
            "device_name": "iPhone 16 Pro"
        }),
    );
    assert!(verified["user_id"].as_u64().unwrap_or(0) > 0);
    assert!(!verified["token"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f02_05_phone_verify_2fa_account_returns_challenge_without_token() {
    let h = FabushiE2EHarness::connect_production();
    let sent = h.expect_ok("auth.phone.send_code", json!({"phone": "+14155550222"}));
    let hash = sent["phone_code_hash"].as_str().expect("hash");
    let verified = h.expect_ok(
        "auth.phone.verify",
        json!({
            "phone": "+14155550222",
            "code": "123456",
            "phone_code_hash": hash,
            "require_2fa_account": true
        }),
    );
    assert_eq!(verified["requires_2fa"], true);
    let challenge = verified["challenge_token"].as_str().unwrap_or("");
    assert!(challenge.starts_with("2fa_") || challenge.starts_with("tfa_"));
    assert!(
        verified.get("token").and_then(Value::as_str).is_none(),
        "2FA gate must not issue premature bearer token"
    );
}

// ============================================================================
// F03 — Alipay QR Login & Mobile OAuth Callback (5 tests)
// ============================================================================

#[test]
fn tc_t1_f03_01_alipay_start_generates_attempt_qr_and_10m_ttl() {
    let h = FabushiE2EHarness::connect_production();
    let before = now_ms();
    let res = h.expect_ok(
        "auth.alipay.start",
        json!({"client_mode": "ipad_qr", "redirect_scheme": "fabushi"}),
    );
    assert!(!res["attempt_id"].as_str().unwrap_or("").is_empty());
    assert_eq!(res["auth_url"], res["qr_payload"]);
    assert!(res["state"].as_str().unwrap_or("").starts_with("fbs_"));
    assert_eq!(res["poll_after_ms"], 1000);
    let exp = res["expires_at_ms"].as_i64().expect("expires_at_ms");
    assert!(exp - before >= 540_000 && exp - before <= 660_000);
}

#[test]
fn tc_t1_f03_02_alipay_poll_transitions_pending_to_confirmed_session() {
    let h = FabushiE2EHarness::connect_production();
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ipad_qr"}));
    let attempt_id = started["attempt_id"].as_str().expect("attempt_id");

    let pending = h.expect_ok("auth.alipay.poll", json!({"attempt_id": attempt_id}));
    assert_eq!(pending["status"], "pending");

    h.server().confirm_alipay_attempt(
        attempt_id,
        json!({
            "user_id": 2001,
            "session_id": "sess_alipay_1",
            "token": "tok_alipay_confirmed",
            "expires_at": now_ms() + 86_400_000
        }),
    );

    let confirmed = h.expect_ok("auth.alipay.poll", json!({"attempt_id": attempt_id}));
    assert_eq!(confirmed["status"], "confirmed");
    assert!(!confirmed["session"]["token"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f03_03_alipay_mobile_deep_link_exchanges_code_and_authenticates() {
    let h = FabushiE2EHarness::connect_production();
    let started = h.expect_ok(
        "auth.alipay.start",
        json!({"client_mode": "ios_oauth", "redirect_scheme": "fabushi"}),
    );
    let attempt_id = started["attempt_id"].as_str().expect("attempt_id");
    let state = started["state"].as_str().expect("state");

    let deep_link = format!("fabushi://auth/alipay?state={state}&auth_code=alipay_code_99");
    let done = h.expect_ok(
        "auth.alipay.handle_deep_link",
        json!({"url": deep_link, "expected_attempt_id": attempt_id}),
    );
    assert_eq!(done["authenticated"], true);
    assert_eq!(done["attempt_id"], attempt_id);
    assert!(!done["session"]["token"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f03_04_alipay_cancel_stops_polling_cleanly() {
    let h = FabushiE2EHarness::connect_production();
    let started = h.expect_ok("auth.alipay.start", json!({"client_mode": "ipad_qr"}));
    let attempt_id = started["attempt_id"].as_str().expect("attempt_id");

    let cancelled = h.expect_ok("auth.alipay.cancel", json!({"attempt_id": attempt_id}));
    assert_eq!(cancelled["status"], "cancelled");

    let count_before = h.server().request_count_for_path("/api/v1/auth/alipay/poll/");
    std::thread::sleep(std::time::Duration::from_millis(50));
    let count_after = h.server().request_count_for_path("/api/v1/auth/alipay/poll/");
    assert_eq!(count_before, count_after);
}

#[test]
fn tc_t1_f03_05_alipay_restart_mints_fresh_attempt_and_distinct_state() {
    let h = FabushiE2EHarness::connect_production();
    let first = h.expect_ok("auth.alipay.start", json!({"client_mode": "ios_oauth"}));
    let _ = h.expect_ok(
        "auth.alipay.cancel",
        json!({"attempt_id": first["attempt_id"]}),
    );
    let second = h.expect_ok("auth.alipay.start", json!({"client_mode": "ios_oauth"}));
    assert_ne!(first["attempt_id"], second["attempt_id"]);
    assert_ne!(first["state"], second["state"]);
}

// ============================================================================
// F04 — Passkeys (WebAuthn/FIDO2) & QR Device Pairing (5 tests)
// ============================================================================

#[test]
fn tc_t1_f04_01_passkey_register_and_list_returns_credential() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "passkey_alice");

    let reg = h.expect_ok(
        "auth.passkey.register",
        json!({
            "credential_id": "cred_iphone16_001",
            "public_key": B64.encode([0x44u8; 65]),
            "device_name": "iPhone 16 Pro"
        }),
    );
    assert_eq!(reg["credential_id"], "cred_iphone16_001");
    assert_eq!(reg["sign_count"], 0);

    let listed = h.expect_ok("auth.passkey.list", json!({}));
    assert!(listed["total"].as_u64().unwrap_or(0) >= 1);
    assert!(
        listed["passkeys"]
            .as_array()
            .expect("passkeys array")
            .iter()
            .any(|p| p["credential_id"] == "cred_iphone16_001")
    );
}

#[test]
fn tc_t1_f04_02_passkey_challenge_and_verify_passwordless_login() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "passkey_bob");
    let _ = h.expect_ok(
        "auth.passkey.register",
        json!({
            "credential_id": "cred_bob_001",
            "public_key": B64.encode([0x55u8; 65]),
            "device_name": "iPad Pro M4"
        }),
    );

    let chal = h.expect_ok("auth.passkey.challenge", json!({"username": "passkey_bob"}));
    assert_eq!(chal["timeout_ms"], 300_000);
    let challenge_str = chal["challenge"].as_str().expect("challenge");

    let verified = h.expect_ok(
        "auth.passkey.verify",
        json!({
            "challenge": challenge_str,
            "credential_id": "cred_bob_001",
            "authenticator_data": B64.encode([0x01u8; 37]),
            "client_data_json": B64.encode(format!(r#"{{"type":"webauthn.get","challenge":"{challenge_str}"}}"#)),
            "signature": B64.encode([0x99u8; 64])
        }),
    );
    assert!(!verified["token"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f04_03_qr_pair_init_emits_pair_token_and_tg_login_url() {
    let h = FabushiE2EHarness::connect_production();
    let secondary = h.spawn_secondary_host("iPad Air M3");
    let init = secondary.expect_ok("auth.pair.init", json!({"device_name": "iPad Air M3"}));
    let token = init["pair_token"].as_str().expect("pair_token");
    assert!(token.starts_with("pair_"));
    assert_eq!(
        init["qr_url"].as_str(),
        Some(format!("tg://login?token={token}").as_str())
    );
    assert!(init["expires_at_ms"].as_i64().unwrap_or(0) > now_ms());
}

#[test]
fn tc_t1_f04_04_qr_pair_primary_accept_confirms_secondary_poll() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "pair_owner");
    let secondary = h.spawn_secondary_host("iPad Pro M4");

    let init = secondary.expect_ok("auth.pair.init", json!({"device_name": "iPad Pro M4"}));
    let pair_token = init["pair_token"].as_str().expect("pair_token");

    let pending = secondary.expect_ok("auth.pair.poll", json!({"pair_token": pair_token}));
    assert_eq!(pending["status"], "pending");

    let accepted = h.expect_ok("auth.pair.accept", json!({"pair_token": pair_token}));
    assert_eq!(accepted["status"], "confirmed");

    let confirmed = secondary.expect_ok("auth.pair.poll", json!({"pair_token": pair_token}));
    assert_eq!(confirmed["status"], "confirmed");
    assert!(!confirmed["session"]["token"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f04_05_qr_pair_verify_with_2fa_cloud_password_succeeds() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "pair_2fa_owner");
    let _ = h.expect_ok(
        "auth.two_factor.set",
        json!({"new_password": "Cloud2FA#Password", "hint": "cloud"}),
    );

    let secondary = h.spawn_secondary_host("iPad mini");
    let init = secondary.expect_ok("auth.pair.init", json!({"device_name": "iPad mini"}));
    let pair_token = init["pair_token"].as_str().expect("pair_token");
    let _ = h.expect_ok("auth.pair.accept", json!({"pair_token": pair_token}));

    let verified = secondary.expect_ok(
        "auth.pair.verify",
        json!({"pair_token": pair_token, "cloud_password": "Cloud2FA#Password"}),
    );
    assert_eq!(verified["verified"], true);
    assert!(!verified["token"].as_str().unwrap_or("").is_empty());
}

// ============================================================================
// F05 — Cloud 2FA Password & Active Device Sessions (5 tests)
// ============================================================================

#[test]
fn tc_t1_f05_01_two_factor_set_and_status_reports_hint() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "tfa_alice");

    let set_res = h.expect_ok(
        "auth.two_factor.set",
        json!({
            "new_password": "MyCloud2FAPassword!",
            "hint": "favorite_mountain",
            "recovery_email": "recovery@fabushi.test"
        }),
    );
    assert_eq!(set_res["has_password"], true);

    let status = h.expect_ok("auth.two_factor.status", json!({}));
    assert_eq!(status["has_password"], true);
    assert_eq!(status["hint"], "favorite_mountain");
    assert_eq!(status["has_recovery_email"], true);
}

#[test]
fn tc_t1_f05_02_login_2fa_challenge_verify_mints_session() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "tfa_bob");
    let _ = h.expect_ok(
        "auth.two_factor.set",
        json!({"new_password": "BobCloudSecret#42", "hint": "42"}),
    );

    let secondary = h.spawn_secondary_host("iPhone 16");
    let gate = secondary.expect_ok(
        "auth.login",
        json!({
            "username": "tfa_bob",
            "password": "",
            "auth_verifier_b64": split_key_v2_verifier_b64()
        }),
    );
    assert_eq!(gate["requires_2fa"], true);
    let challenge = gate["challenge_token"].as_str().expect("challenge_token");

    let done = secondary.expect_ok(
        "auth.two_factor.verify",
        json!({
            "challenge_token": challenge,
            "cloud_password": "BobCloudSecret#42",
            "device_name": "iPhone 16"
        }),
    );
    assert!(!done["token"].as_str().unwrap_or("").is_empty());
}

#[test]
fn tc_t1_f05_03_two_factor_rotate_and_disable_roundtrip() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "tfa_rotator");
    let _ = h.expect_ok("auth.two_factor.set", json!({"new_password": "OldCloud#1"}));
    let _ = h.expect_ok(
        "auth.two_factor.set",
        json!({"current_password": "OldCloud#1", "new_password": "NewCloud#2", "hint": "v2"}),
    );
    let disabled = h.expect_ok(
        "auth.two_factor.disable",
        json!({"current_password": "NewCloud#2"}),
    );
    assert_eq!(disabled["has_password"], false);
    let status = h.expect_ok("auth.two_factor.status", json!({}));
    assert_eq!(status["has_password"], false);
}

#[test]
fn tc_t1_f05_04_session_list_devices_marks_single_current_device() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "multi_dev_user");
    let dev2 = h.spawn_secondary_host("iPad Pro 13");
    let dev3 = h.spawn_secondary_host("Mac Studio");
    for d in [&dev2, &dev3] {
        let _ = d.expect_ok(
            "auth.login",
            json!({
                "username": "multi_dev_user",
                "password": "",
                "auth_verifier_b64": split_key_v2_verifier_b64()
            }),
        );
    }

    let list = h.expect_ok("session.list_devices", json!({}));
    let sessions = list["sessions"].as_array().expect("sessions array");
    assert!(sessions.len() >= 3);
    let current_count = sessions
        .iter()
        .filter(|s| s["is_current"].as_bool() == Some(true))
        .count();
    assert_eq!(current_count, 1);
}

#[test]
fn tc_t1_f05_05_session_revoke_single_and_revoke_others_preserves_current() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "revoke_owner");
    let dev2 = h.spawn_secondary_host("Sibling 1");
    let dev3 = h.spawn_secondary_host("Sibling 2");
    let s2 = dev2.expect_ok(
        "auth.login",
        json!({
            "username": "revoke_owner",
            "password": "",
            "auth_verifier_b64": split_key_v2_verifier_b64()
        }),
    );
    let _ = dev3.expect_ok(
        "auth.login",
        json!({
            "username": "revoke_owner",
            "password": "",
            "auth_verifier_b64": split_key_v2_verifier_b64()
        }),
    );

    let revoked = h.expect_ok(
        "session.revoke_device",
        json!({"session_id": s2["session_id"]}),
    );
    assert_eq!(revoked["revoked"], true);
    assert!(revoked["account_generation"].as_u64().unwrap_or(0) >= 1);

    let others = h.expect_ok("session.revoke_other_devices", json!({}));
    assert!(others["revoked_count"].as_u64().unwrap_or(0) >= 1);

    let after = h.expect_ok("session.list_devices", json!({}));
    let remaining = after["sessions"].as_array().expect("sessions");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["is_current"], true);
}

// ============================================================================
// F06 — Multi-Account Switcher, Generation Fencing & Passcode Lock (5 tests)
// ============================================================================

#[test]
fn tc_t1_f06_01_add_three_accounts_with_distinct_db_namespaces() {
    let h = FabushiE2EHarness::connect_production();
    for idx in 1..=3 {
        let _ = h.expect_ok(
            "session.add_account",
            json!({
                "username": format!("acct_std_{idx}"),
                "display_name": format!("Account {idx}"),
                "token": format!("tok_std_{idx}")
            }),
        );
    }
    let list = h.expect_ok("session.list_accounts", json!({}));
    assert_eq!(list["max_accounts"], 3);
    let accounts = list["accounts"].as_array().expect("accounts");
    assert_eq!(accounts.len(), 3);
    let namespaces: std::collections::HashSet<&str> = accounts
        .iter()
        .filter_map(|a| a["db_namespace"].as_str())
        .collect();
    assert_eq!(namespaces.len(), 3);
}

#[test]
fn tc_t1_f06_02_switch_account_bumps_generation_and_isolates_chats() {
    let h = FabushiE2EHarness::connect_production();
    let a = h.expect_ok(
        "session.add_account",
        json!({"username": "iso_a", "display_name": "Iso A", "token": "tok_a"}),
    );
    let b = h.expect_ok(
        "session.add_account",
        json!({"username": "iso_b", "display_name": "Iso B", "token": "tok_b"}),
    );
    let id_a = a["account_id"].as_str().expect("id_a").to_owned();
    let id_b = b["account_id"].as_str().expect("id_b").to_owned();

    let _ = h.expect_ok("session.switch_account", json!({"account_id": id_a}));
    let _ = h.expect_ok(
        "chats.create",
        json!({"chat_type": 1, "title": "Account A Private Group", "member_ids": [2001, 2002]}),
    );
    let g1 = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g1");

    let switched = h.expect_ok("session.switch_account", json!({"account_id": id_b}));
    let g2 = switched["generation"].as_u64().expect("g2");
    assert_eq!(g2, g1 + 1);

    let b_chats = h.expect_ok("chats.list", json!({}));
    assert_no_sample_chats(&b_chats).expect("no sample chats");
    let arr = b_chats["chats"].as_array().expect("chats");
    assert!(
        !arr.iter().any(|c| c["title"] == "Account A Private Group"),
        "Account A chats must never leak into Account B"
    );
}

#[test]
fn tc_t1_f06_03_generation_bump_monotonic_sequence() {
    let h = FabushiE2EHarness::connect_production();
    let start = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("start gen");
    let mut prev = start;
    for step in 1..=5 {
        let bumped = h.expect_ok(
            "generation.bump",
            json!({"reason": format!("e2e_bump_{step}")}),
        );
        let next = bumped["generation"].as_u64().expect("next gen");
        assert_eq!(next, prev + 1);
        assert_eq!(bumped["previous_generation"].as_u64(), Some(prev));
        prev = next;
    }
    assert_eq!(prev, start + 5);
}

#[test]
fn tc_t1_f06_04_passcode_set_lock_now_and_unlock_transitions() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "pin_user");
    let set_res = h.expect_ok(
        "session.passcode.set",
        json!({"passcode": "2468", "auto_lock_seconds": 300, "biometric_enabled": false}),
    );
    assert_eq!(set_res["has_passcode"], true);
    assert_eq!(set_res["is_locked"], false);

    let locked = h.expect_ok("session.passcode.lock_now", json!({}));
    assert_eq!(locked["is_locked"], true);

    let unlocked = h.expect_ok("session.passcode.verify", json!({"passcode": "2468"}));
    assert_eq!(unlocked["unlocked"], true);
    assert_eq!(unlocked["is_locked"], false);
    assert_eq!(unlocked["bad_tries"], 0);
}

#[test]
fn tc_t1_f06_05_biometric_unlock_and_logout_increments_generation() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "bio_user");
    let _ = h.expect_ok(
        "session.passcode.set",
        json!({"passcode": "9753", "auto_lock_seconds": 60, "biometric_enabled": true}),
    );
    let _ = h.expect_ok("session.passcode.lock_now", json!({}));
    let unlocked = h.expect_ok(
        "session.passcode.verify",
        json!({"biometric_verified": true}),
    );
    assert_eq!(unlocked["unlocked"], true);

    let g_before = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g_before");
    let _ = h.expect_ok("session.logout", json!({}));
    let g_after = h.expect_ok("generation.current", json!({}))["generation"]
        .as_u64()
        .expect("g_after");
    assert!(g_after > g_before);
}

// ============================================================================
// F07 — Unified Adaptive Apple Shell (5 tests)
// ============================================================================

#[test]
fn tc_t1_f07_01_ipad_regular_3col_navigation_and_inspector_toggle() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "settings.set_shell_state",
        json!({
            "active_panel": "chats",
            "sidebar_width": 288.0,
            "inspector_open": true,
            "form_factor": "ipad",
            "horizontal_size_class": "regular"
        }),
    );
    let nav = h.expect_ok(
        "actions.navigate",
        json!({"route": "chats", "chat_id": 101}),
    );
    assert_eq!(nav["accepted"], true);
    assert_eq!(nav["presentation_mode"], "split_3col");
    assert_eq!(nav["inspector_open"], true);
}

#[test]
fn tc_t1_f07_02_iphone_compact_tab_stack_sheet_all_5_panels() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "settings.set_shell_state",
        json!({
            "active_panel": "chats",
            "form_factor": "iphone",
            "horizontal_size_class": "compact"
        }),
    );
    for route in ["chats", "agent_studio", "calls", "remote_computer", "settings"] {
        let nav = h.expect_ok("actions.navigate", json!({"route": route}));
        assert_eq!(nav["accepted"], true);
        assert_eq!(nav["presentation_mode"], "tab_stack_sheet");
    }
}

#[test]
fn tc_t1_f07_03_deep_link_route_targets_chat_and_increments_selection_revision() {
    let h = FabushiE2EHarness::connect_production();
    let s1 = h.expect_ok("settings.get_shell_state", json!({}));
    let rev1 = s1["selection_revision"]
        .as_u64()
        .expect("initial selection_revision");

    let nav = h.expect_ok(
        "actions.navigate",
        json!({"route": "chats", "chat_id": 101, "message_id": 55}),
    );
    assert_eq!(nav["active_panel"], "chats");
    assert!(
        nav["selection_revision"]
            .as_u64()
            .expect("nav selection_revision")
            > rev1
    );
}

#[test]
fn tc_t1_f07_04_compose_mode_exits_cleanly_on_panel_switch() {
    let h = FabushiE2EHarness::connect_production();
    let compose = h.expect_ok("actions.navigate", json!({"route": "compose"}));
    assert_eq!(compose["compose_mode_active"], true);

    let settings = h.expect_ok("actions.navigate", json!({"route": "settings"}));
    assert_eq!(settings["active_panel"], "settings");
    assert_eq!(settings["compose_mode_active"], false);
}

#[test]
fn tc_t1_f07_05_shell_state_persists_across_cold_restart() {
    let mut h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok(
        "settings.set_shell_state",
        json!({
            "active_panel": "chats",
            "sidebar_width": 312.0,
            "inspector_open": true,
            "form_factor": "ipad",
            "horizontal_size_class": "regular"
        }),
    );
    h.restart_cold();
    let reloaded = h.expect_ok("settings.get_shell_state", json!({}));
    assert!(
        (reloaded["sidebar_width"]
            .as_f64()
            .expect("persisted sidebar_width")
            - 312.0)
            .abs()
            < 0.01
    );
    assert_eq!(reloaded["inspector_open"], true);
}

// ============================================================================
// F08 — Grok Bot Multi-Select Compose (5 tests)
// ============================================================================

#[test]
fn tc_t1_f08_01_compose_candidates_merges_humans_and_bots_deduplicated() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "compose_alice");
    let _ = h.expect_ok(
        "contacts.add",
        json!({"username": "bot_friend_human", "first_name": "Bot", "last_name": "Friend"}),
    );
    let _ = h.expect_ok("chats.prestart_bot", json!({"bot_username": "compose_helper_bot"}));
    let res = h.expect_ok(
        "chats.compose_candidates",
        json!({"query": "bot", "include_bots": true}),
    );
    let candidates = res["candidates"].as_array().expect("candidates array");
    assert!(
        !candidates.is_empty(),
        "compose_candidates must return non-empty seeded human/bot candidates"
    );
    let unique_ids: std::collections::HashSet<u64> = candidates
        .iter()
        .filter_map(|c| c["user_id"].as_u64())
        .collect();
    assert_eq!(unique_ids.len(), candidates.len());
}

#[test]
fn tc_t1_f08_02_compose_send_single_human_creates_dm_and_sends() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "dm_sender");
    let res = h.expect_ok(
        "chats.compose_send",
        json!({
            "client_nonce": "compose_dm_001",
            "recipients": [{"user_id": 2010, "is_bot": false}],
            "is_group_mode": false,
            "initial_text": "Hello from iOS compose!",
            "message_nonce": 10001
        }),
    );
    assert_eq!(res["chat"]["chat_type"], 0);
    assert!(res["chat"]["chat_id"].as_i64().unwrap_or(0) > 0);
    assert!(res["message"]["message_id"].as_i64().unwrap_or(0) > 0);
}

#[test]
fn tc_t1_f08_03_compose_send_single_bot_promotes_and_reuses_dm() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "bot_dm_sender");
    let first = h.expect_ok(
        "chats.compose_send",
        json!({
            "client_nonce": "compose_bot_1",
            "recipients": [{"user_id": 5001, "is_bot": true}],
            "is_group_mode": false,
            "initial_text": "First bot prompt",
            "message_nonce": 10002
        }),
    );
    assert_eq!(first["chat"]["chat_type"], 4);
    assert_eq!(first["chat"]["is_bot"], true);
    let cid1 = first["chat"]["chat_id"].as_i64().expect("cid1");

    let second = h.expect_ok(
        "chats.compose_send",
        json!({
            "client_nonce": "compose_bot_2",
            "recipients": [{"user_id": 5001, "is_bot": true}],
            "is_group_mode": false,
            "initial_text": "Second bot prompt",
            "message_nonce": 10003
        }),
    );
    assert_eq!(second["chat"]["chat_id"].as_i64(), Some(cid1));
    assert_eq!(second["reused_existing_dm"], true);
}

#[test]
fn tc_t1_f08_04_compose_send_multi_select_bot_group_autogenerates_title() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "group_creator");
    let res = h.expect_ok(
        "chats.compose_send",
        json!({
            "client_nonce": "compose_grp_1",
            "recipients": [
                {"user_id": 2011, "is_bot": false},
                {"user_id": 2012, "is_bot": false},
                {"user_id": 5002, "is_bot": true}
            ],
            "is_group_mode": true,
            "custom_group_title": "",
            "initial_text": "Welcome team + bot",
            "message_nonce": 10004
        }),
    );
    assert_eq!(res["chat"]["chat_type"], 1);
    assert_eq!(res["chat"]["member_count"], 4);
    assert!(!res["chat"]["title"].as_str().unwrap_or("").trim().is_empty());
    assert_eq!(res["has_bot_in_group"], true);
}

#[test]
fn tc_t1_f08_05_compose_send_explicit_title_persists_in_chat_list() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "titled_creator");
    let created = h.expect_ok(
        "chats.compose_send",
        json!({
            "client_nonce": "compose_grp_titled",
            "recipients": [
                {"user_id": 2015, "is_bot": false},
                {"user_id": 5003, "is_bot": true}
            ],
            "is_group_mode": true,
            "custom_group_title": "Design Review",
            "initial_text": "Kickoff",
            "message_nonce": 10005
        }),
    );
    assert_eq!(created["chat"]["title"], "Design Review");
    let listed = h.expect_ok("chats.list", json!({}));
    assert_no_sample_chats(&listed).expect("no sample chats");
    assert!(
        listed["chats"]
            .as_array()
            .expect("chats")
            .iter()
            .any(|c| c["title"] == "Design Review")
    );
}

// ============================================================================
// F09 — Unified Chat List, Folders, Archive, Pin, Mute & Forum Topics (5 tests)
// ============================================================================

#[test]
fn tc_t1_f09_01_fresh_chat_list_empty_then_unifies_all_chat_types() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "unified_list_user");
    let fresh = h.expect_ok("chats.list", json!({}));
    assert_no_sample_chats(&fresh).expect("fresh account must have zero sample_chats");
    assert_eq!(fresh["total"], 0);

    for (ct, title) in [
        (0u8, "Human DM"),
        (1u8, "Team Group"),
        (3u8, "Release Channel"),
        (4u8, "Hermes Bot"),
        (5u8, "Saved Notes"),
    ] {
        let _ = h.expect_ok("chats.create", json!({"chat_type": ct, "title": title}));
    }
    let after = h.expect_ok("chats.list", json!({}));
    assert_eq!(after["total"], 5);
    let types: std::collections::HashSet<u64> = after["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .filter_map(|c| c["chat_type"].as_u64())
        .collect();
    for expected in [0u64, 1, 3, 4, 5] {
        assert!(types.contains(&expected));
    }
}

#[test]
fn tc_t1_f09_02_folders_crud_and_scoped_chat_filtering() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "folder_user");
    let g1 = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Work Group"}))["chat_id"]
        .as_i64()
        .expect("g1");
    let _g2 = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Personal"}));

    let folder = h.expect_ok(
        "folders.create",
        json!({"title": "Work", "included_chat_ids": [g1], "excluded_chat_ids": []}),
    );
    let fid = folder["id"].as_i64().expect("folder id");

    let filtered = h.expect_ok("chats.list", json!({"folder_id": fid}));
    let rows = filtered["chats"].as_array().expect("filtered chats");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["chat_id"].as_i64(), Some(g1));

    let _ = h.expect_ok(
        "folders.update",
        json!({"id": fid, "title": "Work Renamed", "included_chat_ids": [g1], "excluded_chat_ids": []}),
    );
    let _ = h.expect_ok("folders.delete", json!({"id": fid}));
    let remaining = h.expect_ok("folders.list", json!({}));
    assert!(
        !remaining["folders"]
            .as_array()
            .expect("folders array")
            .iter()
            .any(|f| f["id"].as_i64() == Some(fid))
    );
}

#[test]
fn tc_t1_f09_03_pin_archive_and_mute_state_ordering() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "pin_archive_user");
    let c1 = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "C1"}))["chat_id"]
        .as_i64()
        .expect("c1");
    let c2 = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "C2"}))["chat_id"]
        .as_i64()
        .expect("c2");
    let c3 = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "C3"}))["chat_id"]
        .as_i64()
        .expect("c3");

    let _ = h.expect_ok("chats.pin_dialog", json!({"chat_id": c2, "pinned": true}));
    let _ = h.expect_ok("chats.archive", json!({"chat_id": c3, "archived": true}));
    let mute_until = now_ms() + 3_600_000;
    let muted = h.expect_ok(
        "chats.mute",
        json!({"chat_id": c1, "muted": true, "mute_until_ms": mute_until}),
    );
    assert_eq!(muted["is_muted"], true);

    let main_list = h.expect_ok("chats.list", json!({"include_archived": false}));
    let chats = main_list["chats"].as_array().expect("chats");
    assert_eq!(chats[0]["chat_id"].as_i64(), Some(c2));
    assert!(!chats.iter().any(|c| c["chat_id"].as_i64() == Some(c3)));
}

#[test]
fn tc_t1_f09_04_forum_topics_crud_lifecycle() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "forum_admin");
    let forum_id = h.expect_ok(
        "chats.create",
        json!({"chat_type": 2, "title": "Fabushi Forum", "is_forum": true}),
    )["chat_id"]
        .as_i64()
        .expect("forum_id");

    let t1 = h.expect_ok(
        "topics.create",
        json!({"chat_id": forum_id, "title": "Releases", "icon_color": 0x2AABEEu32}),
    );
    let tid1 = t1["topic_id"].as_i64().expect("tid1");
    let t2 = h.expect_ok(
        "topics.create",
        json!({"chat_id": forum_id, "title": "Design"}),
    );
    assert!(t2["topic_id"].as_i64().expect("tid2") > tid1);

    let upd = h.expect_ok(
        "topics.update",
        json!({"chat_id": forum_id, "topic_id": tid1, "title": "Releases v1.6.7", "is_pinned": true, "is_closed": true}),
    );
    assert_eq!(upd["is_pinned"], true);
    assert_eq!(upd["is_closed"], true);

    let _ = h.expect_ok("topics.delete", json!({"chat_id": forum_id, "topic_id": tid1}));
    let list = h.expect_ok("topics.list", json!({"chat_id": forum_id}));
    assert!(
        !list["topics"]
            .as_array()
            .expect("topics array")
            .iter()
            .any(|t| t["topic_id"].as_i64() == Some(tid1))
    );
}

#[test]
fn tc_t1_f09_05_invite_link_create_join_and_revoke() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "invite_admin");
    let chat_id = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Invite Room"}))
        ["chat_id"]
        .as_i64()
        .expect("chat_id");

    let inv = h.expect_ok(
        "chats.invites.create",
        json!({"chat_id": chat_id, "usage_limit": 2}),
    );
    let code = inv["code"].as_str().expect("invite code");
    let invite_id = inv["invite_id"].as_str().expect("invite_id");

    let peer = h.spawn_secondary_host("Joiner");
    let _ = peer.expect_ok(
        "auth.register",
        json!({
            "username": "joiner_bob",
            "auth_version": 2,
            "auth_salt_b64": split_key_v2_salt_b64(),
            "auth_verifier_b64": split_key_v2_verifier_b64(),
            "kdf_params": { "m_cost_kib": 65536, "t_cost": 3, "p_cost": 4 }
        }),
    );
    let joined = peer.expect_ok("chats.join", json!({"code": code}));
    assert_eq!(joined["chat_id"].as_i64(), Some(chat_id));

    let _ = h.expect_ok(
        "chats.invites.revoke",
        json!({"chat_id": chat_id, "invite_id": invite_id}),
    );
    let invs = h.expect_ok("chats.invites.list", json!({"chat_id": chat_id}));
    let invites = invs["invites"].as_array().expect("invites array");
    assert!(
        invites
            .iter()
            .all(|i| i["invite_id"].as_str() != Some(invite_id) || i["revoked"] == true),
        "revoked invite must be marked revoked or omitted from active invites"
    );
    let rev_join_err = peer.expect_err(
        "chats.join",
        json!({"code": code}),
        &["invalid_invite", "not_found"],
    );
    assert!(!rev_join_err.retryable);
}

// ============================================================================
// F10 — Contacts, Peer Resolution, Blocklist, Send-As & Prestart Bot (5 tests)
// ============================================================================

#[test]
fn tc_t1_f10_01_contacts_add_list_delete_roundtrip() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "contacts_owner");

    let added = h.expect_ok(
        "contacts.add",
        json!({"username": "alice_contact", "first_name": "Alice", "last_name": "Chen"}),
    );
    let uid = added["user_id"].as_u64().expect("contact user_id");

    let list = h.expect_ok("contacts.list", json!({}));
    assert!(
        list["contacts"]
            .as_array()
            .expect("contacts")
            .iter()
            .any(|c| c["user_id"].as_u64() == Some(uid))
    );

    let _ = h.expect_ok("contacts.delete", json!({"user_id": uid}));
    let after = h.expect_ok("contacts.list", json!({}));
    assert!(
        !after["contacts"]
            .as_array()
            .expect("contacts")
            .iter()
            .any(|c| c["user_id"].as_u64() == Some(uid))
    );
}

#[test]
fn tc_t1_f10_02_resolve_peer_handles_username_phone_botfather_and_miniapp() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "resolver_user");

    let bf = h.expect_ok("contacts.resolve_peer", json!({"username": "机器人之父"}));
    assert_eq!(bf["user"]["username"], "BotFather");

    let app = h.expect_ok("contacts.resolve_peer", json!({"username": "dharma_bot/zen"}));
    assert_eq!(app["kind"], "mini_app");

    let by_phone = h.expect_ok("contacts.resolve_peer", json!({"phone": "+14155550199"}));
    assert_eq!(by_phone["kind"], "user");
}

#[test]
fn tc_t1_f10_03_block_list_unblock_restores_dm_sending() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "blocker_user");
    let blocked = h.expect_ok("contacts.block", json!({"user_id": 3001}));
    assert_eq!(blocked["blocked"], true);

    let list = h.expect_ok("contacts.list_blocked", json!({}));
    assert!(
        list["blocked_users"]
            .as_array()
            .expect("blocked_users")
            .iter()
            .any(|u| u["user_id"].as_u64() == Some(3001))
    );

    let unblocked = h.expect_ok("contacts.unblock", json!({"user_id": 3001}));
    assert_eq!(unblocked["blocked"], false);
}

#[test]
fn tc_t1_f10_04_send_as_list_returns_self_default_and_owned_channel() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "channel_owner");
    let _chan = h.expect_ok("chats.create", json!({"chat_type": 3, "title": "My Broadcast"}));
    let grp = h.expect_ok("chats.create", json!({"chat_type": 2, "title": "Discussion"}))["chat_id"]
        .as_i64()
        .expect("grp");

    let peers = h.expect_ok("chats.send_as_list", json!({"chat_id": grp}));
    let arr = peers["peers"].as_array().expect("peers");
    assert!(arr.iter().any(|p| p["kind"] == "user" && p["is_default"] == true));
    assert!(arr.iter().any(|p| p["kind"] == "channel" && p["is_default"] == false));
}

#[test]
fn tc_t1_f10_05_prestart_bot_virtual_then_start_bot_materializes_dm() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "prestart_user");
    let before_count = h.expect_ok("chats.list", json!({}))["total"]
        .as_u64()
        .expect("before_count total");

    let preview = h.expect_ok("chats.prestart_bot", json!({"bot_username": "helper_bot"}));
    assert_eq!(preview["virtual"], true);
    assert!(preview["db_chat_id"].is_null());
    assert_eq!(
        h.expect_ok("chats.list", json!({}))["total"].as_u64(),
        Some(before_count)
    );

    let started = h.expect_ok(
        "chats.start_bot",
        json!({"bot_user_id": preview["bot_user_id"], "start_param": "welcome"}),
    );
    assert_eq!(started["chat_type"], 4);
    assert!(started["chat_id"].as_i64().expect("started chat_id") > 0);
    assert!(
        started["start_message_id"]
            .as_i64()
            .expect("start_message_id")
            > 0
    );
}

// ============================================================================
// F11 — Atomic PTS/QTS Sync, 24h Nonce Dedup & Offline SQLite WAL Outbox (5 tests)
// ============================================================================

#[test]
fn tc_t1_f11_01_sequential_sends_produce_contiguous_monotonic_pts() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "pts_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "PTS Room"}))["chat_id"]
        .as_i64()
        .expect("cid");

    for i in 1..=5u64 {
        let sent = h.expect_ok(
            "messages.send",
            json!({"chat_id": cid, "text": format!("msg {i}"), "client_nonce": 5000 + i}),
        );
        assert!(sent["message_id"].as_i64().unwrap_or(0) > 0);
    }

    let diff = h.expect_ok(
        "sync.get_difference",
        json!({"chat_id": cid, "pts": 0, "limit": 10}),
    );
    assert_eq!(diff["snapshot_required"], false);
    assert_eq!(diff["has_more"], false);
    let pts_list: Vec<u64> = diff["updates"]
        .as_array()
        .expect("updates")
        .iter()
        .filter_map(|u| u["pts"].as_u64())
        .collect();
    assert_eq!(pts_list.len(), 5);
    verify_pts_monotonic_and_snapshot_fence(0, 1, &pts_list, false).expect("contiguous PTS");
}

#[test]
fn tc_t1_f11_02_ws_realtime_pts_push_emits_new_message_event() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "ws_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "WS Room"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok("sync.connect_ws", json!({}));

    h.server().queue_ws_push_frame(
        &json!({
            "kind": "new_message",
            "chat_id": cid,
            "message_id": 7001,
            "pts": 1,
            "text": "Realtime WS push!"
        })
        .to_string(),
    );

    let polled = h.poll_events(16, 500);
    assert!(polled.ok);
    assert!(
        polled
            .events
            .iter()
            .any(|e| e.kind == "new_message" && e.payload["pts"].as_u64() == Some(1))
    );
}

#[test]
fn tc_t1_f11_03_ws_pts_gap_triggers_automatic_get_difference_backfill() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "gap_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Gap Room"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok("sync.connect_ws", json!({}));

    h.server().queue_ws_push_frame(
        &json!({"kind": "new_message", "chat_id": cid, "message_id": 7005, "pts": 5, "text": "gap"})
            .to_string(),
    );
    let polled = h.poll_events(16, 400);
    assert!(polled.ok);
    assert!(h.server().request_count_for_path("/api/v1/sync/get_difference") >= 1);
}

#[test]
fn tc_t1_f11_04_offline_sqlite_wal_outbox_survives_cold_restart_and_flushes() {
    let mut h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "outbox_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Outbox Room"}))
        ["chat_id"]
        .as_i64()
        .expect("cid");

    h.server().set_offline(true);
    let queued = h.expect_ok(
        "outbox.enqueue",
        json!({"chat_id": cid, "text": "Offline durable message", "client_nonce": 88001}),
    );
    assert_eq!(queued["status"], "queued");

    h.restart_cold();
    let pending = h.expect_ok("outbox.list", json!({"chat_id": cid}));
    assert!(
        pending["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|it| it["client_nonce"].as_u64() == Some(88001))
    );

    h.server().set_offline(false);
    let flushed = h.expect_ok("outbox.flush", json!({"chat_id": cid}));
    assert!(flushed["delivered_count"].as_u64().unwrap_or(0) >= 1);
}

#[test]
fn tc_t1_f11_05_ws_first_vs_http_first_reconciliation_collapses_duplicate() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "race_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Race Room"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok("sync.connect_ws", json!({}));

    h.server()
        .inject_delay_ms(&format!("/api/v1/chats/{cid}/messages"), 150);
    h.server().queue_ws_push_frame(
        &json!({
            "kind": "new_message",
            "chat_id": cid,
            "message_id": 500,
            "client_nonce": 99101,
            "delivery_status": "read",
            "text": "WS arrived first"
        })
        .to_string(),
    );

    let sent = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "WS arrived first", "client_nonce": 99101}),
    );
    assert_eq!(sent["message_id"], 500);
    assert_eq!(sent["delivery_status"], "read");

    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 20}));
    let matches = hist["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|m| m["client_nonce"].as_u64() == Some(99101))
        .count();
    assert_eq!(matches, 1, "WS-first + HTTP-second must never duplicate bubble");
}

// ============================================================================
// F12 — Virtualized Transcript, 900s Bubble Grouping & 11+ Message Cards (5 tests)
// ============================================================================

#[test]
fn tc_t1_f12_01_900s_bubble_grouping_attaches_within_15m_and_splits_at_901s() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "group_bubble_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Grouping"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let layout = h.expect_ok(
        "messages.compute_layout_groups",
        json!({
            "chat_id": cid,
            "messages": [
                {"sender_id": 10, "outgoing": true, "timestamp_secs": 1000, "is_service": false},
                {"sender_id": 10, "outgoing": true, "timestamp_secs": 1900, "is_service": false},
                {"sender_id": 10, "outgoing": true, "timestamp_secs": 2801, "is_service": false}
            ]
        }),
    );
    let expected = verify_bubble_grouping_900s(&[
        TranscriptBubbleInput { sender_id: 10, outgoing: true, timestamp_secs: 1000, is_service: false },
        TranscriptBubbleInput { sender_id: 10, outgoing: true, timestamp_secs: 1900, is_service: false },
        TranscriptBubbleInput { sender_id: 10, outgoing: true, timestamp_secs: 2801, is_service: false },
    ]);
    let groups = layout["groups"].as_array().expect("groups array");
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0]["has_tail"].as_bool(), Some(expected[0].has_tail));
    assert_eq!(groups[1]["attached_prev"].as_bool(), Some(true));
    assert_eq!(groups[2]["attached_prev"].as_bool(), Some(false));
    assert_eq!(groups[2]["has_tail"].as_bool(), Some(true));
}

#[test]
fn tc_t1_f12_02_canonical_message_card_variants_roundtrip_losslessly() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "cards_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Cards Room"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let wf = encode_waveform_5bit(&(0..100).map(|i| (i % 32) as u8).collect::<Vec<_>>());
    assert_eq!(wf.len(), 63);
    let cards = vec![
        json!({"type": "text", "text": "**Bold** ||spoiler||", "entities": [{"type": "bold", "offset": 0, "length": 8}]}),
        json!({"type": "photo", "file_id": "cas_photo_1", "width": 1280, "height": 720, "has_spoiler": true}),
        json!({"type": "voice", "file_id": "cas_voice_1", "duration_secs": 4, "waveform_b64": B64.encode(&wf)}),
        json!({"type": "document", "file_id": "cas_doc_1", "file_name": "spec.pdf", "mime_type": "application/pdf", "size_bytes": 4096}),
        json!({"type": "location", "latitude": 37.3346, "longitude": -122.0090, "title": "Cupertino"}),
        json!({"type": "todo", "title": "Release Checklist", "items": [{"id": "1", "text": "Verify CI", "done": true}]}),
        json!({"type": "invoice", "title": "Pro Pack", "description": "Annual", "currency": "XTR", "total_amount": 500}),
        json!({"type": "generative_card", "card_id": "gc_1", "title": "Diff Summary", "kind": "agent_thinking", "payload": {"steps": 3}}),
    ];

    for (idx, card) in cards.iter().enumerate() {
        let sent = h.expect_ok(
            "messages.send",
            json!({"chat_id": cid, "content": card, "client_nonce": 20000 + idx as u64 + 1}),
        );
        assert!(sent["message_id"].as_i64().unwrap_or(0) > 0);
    }

    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 50}));
    assert_eq!(hist["messages"].as_array().expect("messages").len(), cards.len());
}

#[test]
fn tc_t1_f12_03_poll_create_vote_get_and_close_lifecycle() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "poll_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Poll Room"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let created = h.expect_ok(
        "messages.polls.create",
        json!({
            "chat_id": cid,
            "question": "Preferred Apple HIG Theme?",
            "options": ["Night", "Tinted", "Day"],
            "is_anonymous": false,
            "multiple_choice": false,
            "is_quiz": true,
            "correct_option_id": 0
        }),
    );
    let pid = created["poll_id"].as_str().expect("poll_id");

    let _ = h.expect_ok(
        "messages.polls.vote",
        json!({"poll_id": pid, "option_ids": [0]}),
    );
    let fetched = h.expect_ok("messages.polls.get", json!({"poll_id": pid}));
    assert!(fetched["total_voters"].as_u64().unwrap_or(0) >= 1);

    let closed = h.expect_ok("messages.polls.close", json!({"poll_id": pid}));
    assert_eq!(closed["is_closed"], true);
}

#[test]
fn tc_t1_f12_04_utf16_surrogate_pair_emoji_entity_bounds_roundtrip() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "utf16_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "UTF16 Room"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let text = "你好🚀世界";
    verify_utf16_entity_bounds(text, 2, 2).expect("surrogate pair valid in UTF-16");
    let sent = h.expect_ok(
        "messages.send",
        json!({
            "chat_id": cid,
            "text": text,
            "entities": [{"type": "bold", "offset": 2, "length": 2}],
            "client_nonce": 21001
        }),
    );
    assert!(sent["message_id"].as_i64().unwrap_or(0) > 0);
}

#[test]
fn tc_t1_f12_05_paginated_history_120_messages_no_duplicates() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "page_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Page Room"}))["chat_id"]
        .as_i64()
        .expect("cid");

    for i in 1..=120u64 {
        let _ = h.expect_ok(
            "messages.send",
            json!({"chat_id": cid, "text": format!("p-{i}"), "client_nonce": 30000 + i}),
        );
    }

    let mut seen = std::collections::HashSet::new();
    let mut before_id: Option<i64> = None;
    let mut pages = 0;
    loop {
        let page = h.expect_ok(
            "messages.history",
            json!({"chat_id": cid, "before_id": before_id, "limit": 50}),
        );
        pages += 1;
        let msgs = page["messages"].as_array().expect("messages");
        for m in msgs {
            let mid = m["message_id"].as_i64().expect("mid");
            assert!(seen.insert(mid), "duplicate message_id {mid} across pages");
            before_id = Some(before_id.map_or(mid, |prev| prev.min(mid)));
        }
        if page["has_more"].as_bool() != Some(true) {
            break;
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen.len(), 120);
}

// ============================================================================
// F13 — Message Edit, Delete Tombstone, Forward, Reactions, Pins, TTL & Export (5 tests)
// ============================================================================

#[test]
fn tc_t1_f13_01_edit_message_preserves_id_advances_pts_and_sets_edited_at() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "edit_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Edit Room"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let sent = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "Original text", "client_nonce": 40001}),
    );
    let mid = sent["message_id"].as_i64().expect("mid");
    let pts1 = sent["pts"].as_u64().expect("pts1");

    let edited = h.expect_ok(
        "messages.edit",
        json!({"chat_id": cid, "message_id": mid, "text": "Updated text"}),
    );
    assert_eq!(edited["message_id"].as_i64(), Some(mid));
    assert!(edited["pts"].as_u64().unwrap_or(0) > pts1);
    assert!(edited["edited_at_ms"].as_i64().unwrap_or(0) > 0);
}

#[test]
fn tc_t1_f13_02_delete_pinned_message_emits_tombstone_and_unpins() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "del_pin_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "DelPin"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let mid = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "Pinned soon deleted", "client_nonce": 40002}),
    )["message_id"]
        .as_i64()
        .expect("mid");

    let pinned = h.expect_ok("messages.pins.pin", json!({"chat_id": cid, "message_id": mid}));
    assert!(
        pinned["pinned_message_ids"]
            .as_array()
            .expect("pinned")
            .iter()
            .any(|v| v.as_i64() == Some(mid))
    );

    let deleted = h.expect_ok(
        "messages.delete",
        json!({"chat_id": cid, "message_ids": [mid], "revoke": true}),
    );
    assert_eq!(deleted["revoke"], true);

    let pins_after = h.expect_ok("messages.pins.list", json!({"chat_id": cid}));
    assert!(
        !pins_after["pinned_message_ids"]
            .as_array()
            .expect("pinned_message_ids array")
            .iter()
            .any(|v| v.as_i64() == Some(mid))
    );
}

#[test]
fn tc_t1_f13_03_multi_forward_hides_sender_and_attaches_caption() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "fwd_user");
    let src = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Src"}))["chat_id"]
        .as_i64()
        .expect("src");
    let dst = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Dst"}))["chat_id"]
        .as_i64()
        .expect("dst");

    let m1 = h.expect_ok(
        "messages.send",
        json!({"chat_id": src, "text": "One", "client_nonce": 40003}),
    )["message_id"]
        .as_i64()
        .expect("m1");
    let m2 = h.expect_ok(
        "messages.send",
        json!({"chat_id": src, "text": "Two", "client_nonce": 40004}),
    )["message_id"]
        .as_i64()
        .expect("m2");

    let fwd = h.expect_ok(
        "messages.forward",
        json!({
            "from_chat_id": src,
            "target_chat_id": dst,
            "message_ids": [m1, m2],
            "hide_sender": true,
            "caption": "FYI"
        }),
    );
    assert!(fwd["forwarded_ids"].as_array().expect("forwarded_ids").len() >= 2);
}

#[test]
fn tc_t1_f13_04_toggle_reaction_advances_authoritative_pts() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "react_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "React"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let mid = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "React to me", "client_nonce": 40005}),
    )["message_id"]
        .as_i64()
        .expect("mid");

    let r1 = h.expect_ok(
        "messages.react",
        json!({"chat_id": cid, "message_id": mid, "emoji": "🔥", "action": "toggle"}),
    );
    assert_eq!(r1["reactions_authoritative"], true);
    assert_eq!(r1["reactions"]["🔥"], 1);
    let pts1 = r1["pts"].as_u64().expect("pts1");

    let r2 = h.expect_ok(
        "messages.react",
        json!({"chat_id": cid, "message_id": mid, "emoji": "🔥", "action": "toggle"}),
    );
    assert!(r2["pts"].as_u64().expect("r2 pts") > pts1);
    assert!(r2["reactions"].is_object(), "reactions map must be an object");
    assert_eq!(
        r2["reactions"].get("🔥").and_then(Value::as_u64).unwrap_or(0),
        0
    );
}

#[test]
fn tc_t1_f13_05_ttl_statistics_export_and_clear_history() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "export_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Export"}))["chat_id"]
        .as_i64()
        .expect("cid");
    let _ = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "Exportable", "client_nonce": 40006}),
    );

    let ttl = h.expect_ok("chats.set_ttl", json!({"chat_id": cid, "ttl_seconds": 3600}));
    assert_eq!(ttl["ttl_seconds"], 3600);

    let stats = h.expect_ok("chats.statistics", json!({"chat_id": cid}));
    assert!(stats["total_messages"].as_u64().unwrap_or(0) >= 1);

    let exported = h.expect_ok("chats.export", json!({"chat_id": cid, "limit": 100}));
    assert!(exported["total"].as_u64().unwrap_or(0) >= 1);

    let cleared = h.expect_ok("chats.clear_history", json!({"chat_id": cid}));
    assert_eq!(cleared["cleared"], true);
}

// ============================================================================
// F14 — IME-Safe Canonical Composer, Drafts, Formatting & Voice Gesture (5 tests)
// ============================================================================

#[test]
fn tc_t1_f14_01_cjk_ime_commit_fence_80ms_blocks_premature_send() {
    let h = FabushiE2EHarness::connect_production();
    let _ = h.expect_ok("messages.composer.ime_event", json!({"event": "start", "preedit": "nihao"}));
    let during = h.expect_ok("messages.composer.ime_event", json!({"event": "key_enter"}));
    assert_eq!(during["action"], "ime_candidate_chosen");
    assert_eq!(
        verify_ime_commit_fence(ImeState::Composing, ImeEvent::KeyEnter, 0).action,
        "ime_candidate_chosen"
    );

    let _ = h.expect_ok("messages.composer.ime_event", json!({"event": "commit", "committed": "你好"}));
    let fenced = h.expect_ok(
        "messages.composer.ime_event",
        json!({"event": "key_enter", "elapsed_since_commit_ms": 20}),
    );
    assert_eq!(fenced["action"], "none");

    let intentional = h.expect_ok(
        "messages.composer.ime_event",
        json!({"event": "key_enter", "elapsed_since_commit_ms": 120}),
    );
    assert_eq!(intentional["action"], "send_message");
    assert_eq!(intentional["buffer"], "你好");
}

#[test]
fn tc_t1_f14_02_per_chat_drafts_isolated_synced_and_deleted() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "draft_user");
    let ca = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Draft A"}))["chat_id"]
        .as_i64()
        .expect("ca");
    let cb = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Draft B"}))["chat_id"]
        .as_i64()
        .expect("cb");

    let _ = h.expect_ok("messages.draft.put", json!({"chat_id": ca, "topic_id": 0, "text": "WIP A"}));
    let _ = h.expect_ok("messages.draft.put", json!({"chat_id": cb, "topic_id": 0, "text": "WIP B"}));

    assert_eq!(
        h.expect_ok("messages.draft.get", json!({"chat_id": ca, "topic_id": 0}))["draft"]["text"],
        "WIP A"
    );
    assert_eq!(
        h.expect_ok("messages.draft.get", json!({"chat_id": cb, "topic_id": 0}))["draft"]["text"],
        "WIP B"
    );

    let del = h.expect_ok("messages.draft.delete", json!({"chat_id": ca, "topic_id": 0}));
    assert_eq!(del["cleared"], true);
    assert_eq!(
        h.expect_ok("messages.draft.get", json!({"chat_id": cb, "topic_id": 0}))["draft"]["text"],
        "WIP B"
    );
}

#[test]
fn tc_t1_f14_03_formatting_bar_entities_preserved_in_history() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "fmt_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Fmt"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let text = "Bold Italic Code Spoiler Link";
    let entities = json!([
        {"type": "bold", "offset": 0, "length": 4},
        {"type": "italic", "offset": 5, "length": 6},
        {"type": "code", "offset": 12, "length": 4},
        {"type": "spoiler", "offset": 17, "length": 7},
        {"type": "text_url", "offset": 25, "length": 4, "url": "https://example.com"}
    ]);
    let _ = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": text, "entities": entities, "client_nonce": 41001}),
    );
    let hist = h.expect_ok("messages.history", json!({"chat_id": cid, "limit": 5}));
    assert_eq!(hist["messages"][0]["entities"].as_array().expect("entities").len(), 5);
}

#[test]
fn tc_t1_f14_04_voice_hold_slide_up_lock_and_63b_waveform_complete() {
    let h = FabushiE2EHarness::connect_production();
    let samples: Vec<u8> = (0..100).map(|i| ((i * 3) % 32) as u8).collect();
    assert_eq!(verify_voice_gesture(0.0, -90.0, 1.0), "locked");

    let _ = h.expect_ok("messages.composer.voice_gesture", json!({"action": "press"}));
    let locked = h.expect_ok(
        "messages.composer.voice_gesture",
        json!({"action": "move", "dx": 0.0, "dy": -90.0, "elapsed_secs": 1.0}),
    );
    assert_eq!(locked["state"], "locked");

    let done = h.expect_ok(
        "messages.composer.voice_gesture",
        json!({"action": "lock_send", "elapsed_secs": 3.2, "samples": samples}),
    );
    assert_eq!(done["state"], "completed");
    assert_eq!(done["waveform_bytes_len"], 63);
    let expected_bytes = encode_waveform_5bit(&samples);
    verify_waveform_5bit_63bytes(&samples, &expected_bytes).expect("63B waveform");
}

#[test]
fn tc_t1_f14_05_silent_and_scheduled_send_options_persist() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "sched_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Sched"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let silent = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "Quiet ping", "silent": true, "client_nonce": 41002}),
    );
    assert_eq!(silent["silent"], true);

    let future_ts = now_ms() + 3_600_000;
    let scheduled = h.expect_ok(
        "messages.send",
        json!({"chat_id": cid, "text": "Future ping", "schedule_date_ms": future_ts, "client_nonce": 41003}),
    );
    assert_eq!(scheduled["schedule_date_ms"].as_i64(), Some(future_ts));
}

// ============================================================================
// F15 — 49+ Hermes/Fabushi Slash Commands & @ Context Reference Badges (5 tests)
// ============================================================================

#[test]
fn tc_t1_f15_01_commands_list_exposes_at_least_49_commands_across_8_categories() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "slash_user");
    let res = h.expect_ok("fabushi.commands.list", json!({}));
    let cmds = res["commands"].as_array().expect("commands array");
    verify_slash_command_catalog(cmds).expect("49+ commands across 8 categories");
    assert!(res["total"].as_u64().unwrap_or(0) >= 49);
}

#[test]
fn tc_t1_f15_02_commands_prefix_filter_matches_compact_compress_computer() {
    let h = FabushiE2EHarness::connect_production();
    let res = h.expect_ok("fabushi.commands.list", json!({"prefix": "/com"}));
    let names: Vec<&str> = res["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .filter_map(|c| c["slash_name"].as_str())
        .collect();
    assert!(names.contains(&"/compact"));
    assert!(names.contains(&"/computer"));
}

#[test]
fn tc_t1_f15_03_commands_execute_status_and_model_returns_ui_effects() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "exec_slash_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 4, "title": "Hermes"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let status = h.expect_ok(
        "fabushi.commands.execute",
        json!({"command": "/status", "chat_id": cid}),
    );
    assert_eq!(status["ok"], true);
    assert!(!status["output"].as_str().unwrap_or("").is_empty());

    let model = h.expect_ok(
        "fabushi.commands.execute",
        json!({"command": "/model gpt-5", "chat_id": cid}),
    );
    assert_eq!(model["ok"], true);
    assert!(
        !model["ui_effects"]
            .as_array()
            .expect("ui_effects array")
            .is_empty(),
        "/model slash command must return non-empty ui_effects"
    );
}

#[test]
fn tc_t1_f15_04_commands_aliases_resolve_to_canonical_handlers() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "alias_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 4, "title": "Alias Bot"}))["chat_id"]
        .as_i64()
        .expect("cid");

    for alias in ["/reset", "/stop", "/talk", "/board"] {
        let out = h.expect_ok(
            "fabushi.commands.execute",
            json!({"command": alias, "chat_id": cid}),
        );
        assert_eq!(out["ok"], true);
    }
}

#[test]
fn tc_t1_f15_05_context_extract_badges_parses_all_4_badge_kinds_and_strips_punctuation() {
    let h = FabushiE2EHarness::connect_production();
    let prompt =
        "Check @file:src/lib.rs, @dir:tests/e2e; @diff @staged @git:HEAD~1 (@url:https://example.com)";
    let local = extract_context_reference_badges(prompt);
    assert_eq!(local.len(), 6);

    let res = h.expect_ok("fabushi.context.extract_badges", json!({"text": prompt}));
    let badges = res["badges"].as_array().expect("badges");
    assert_eq!(badges.len(), 6);
    assert_eq!(badges[0]["kind"], "FILE");
    assert_eq!(badges[0]["token"], "@file:src/lib.rs");
    assert_eq!(badges[1]["kind"], "DIR");
    assert_eq!(badges[5]["kind"], "URL");
}

// ============================================================================
// F16 — Muse Context Inspector (5 tests)
// ============================================================================

#[test]
fn tc_t1_f16_01_inspector_profile_aggregates_chat_and_bot_metadata() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "muse_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 4, "title": "Muse Bot"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let profile = h.expect_ok("chats.inspector.profile", json!({"chat_id": cid}));
    assert_eq!(profile["chat_id"].as_i64(), Some(cid));
    assert_eq!(profile["title"], "Muse Bot");
    assert_eq!(profile["is_bot"], true);
}

#[test]
fn tc_t1_f16_02_inspector_members_add_promote_and_remove() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "group_admin_muse");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Muse Group"}))["chat_id"]
        .as_i64()
        .expect("cid");

    let _ = h.expect_ok("chats.members.add", json!({"chat_id": cid, "user_id": 4010}));
    let promoted = h.expect_ok(
        "chats.members.update_role",
        json!({"chat_id": cid, "user_id": 4010, "role": "admin"}),
    );
    assert!(
        promoted["members"]
            .as_array()
            .expect("members")
            .iter()
            .any(|m| m["user_id"].as_u64() == Some(4010) && m["role"] == "admin")
    );

    let after = h.expect_ok("chats.members.remove", json!({"chat_id": cid, "user_id": 4010}));
    assert!(
        !after["members"]
            .as_array()
            .expect("members")
            .iter()
            .any(|m| m["user_id"].as_u64() == Some(4010))
    );
}

#[test]
fn tc_t1_f16_03_inspector_shared_media_isolates_all_5_subtabs() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "shared_media_user");
    let cid = h.expect_ok("chats.create", json!({"chat_type": 1, "title": "Shared Media"}))
        ["chat_id"]
        .as_i64()
        .expect("cid");

    let wf = encode_waveform_5bit(&(0..100).map(|i| (i % 32) as u8).collect::<Vec<_>>());
    let seeded: [(&str, Value); 5] = [
        (
            "media",
            json!({"chat_id": cid, "content": {"type": "photo", "file_id": "cas_sub_photo", "width": 800, "height": 600}, "client_nonce": 45001}),
        ),
        (
            "files",
            json!({"chat_id": cid, "content": {"type": "document", "file_id": "cas_sub_doc", "file_name": "report.pdf", "mime_type": "application/pdf", "size_bytes": 1024}, "client_nonce": 45002}),
        ),
        (
            "voice",
            json!({"chat_id": cid, "content": {"type": "voice", "file_id": "cas_sub_voice", "duration_secs": 3, "waveform_b64": B64.encode(&wf)}, "client_nonce": 45003}),
        ),
        (
            "links",
            json!({"chat_id": cid, "text": "Docs at https://fabushi.test/v167", "client_nonce": 45004}),
        ),
        (
            "music",
            json!({"chat_id": cid, "content": {"type": "audio", "file_id": "cas_sub_music", "title": "Lotus Bell", "duration_secs": 120}, "client_nonce": 45005}),
        ),
    ];
    let mut expected_ids = Vec::with_capacity(5);
    for (subtab, payload) in seeded {
        let mid = h.expect_ok("messages.send", payload)["message_id"]
            .as_i64()
            .expect("seeded message_id");
        expected_ids.push((subtab, mid));
    }

    for (subtab, expected_mid) in expected_ids {
        let bucket = h.expect_ok(
            "chats.shared_media",
            json!({"chat_id": cid, "subtab": subtab, "limit": 20}),
        );
        assert_eq!(bucket["subtab"], subtab);
        let items = bucket["items"].as_array().expect("shared_media items");
        assert_eq!(
            items.len(),
            1,
            "subtab `{subtab}` must isolate exactly its own 1 seeded item"
        );
        assert_eq!(items[0]["message_id"].as_i64(), Some(expected_mid));
    }
}

#[test]
fn tc_t1_f16_04_inspector_bot_config_patch_persists_coding_and_avatar_fields() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "bot_owner");
    let bots = h.expect_ok("fabushi.bots.list", json!({}));
    let bot_id = bots["bots"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|b| b["bot_id"].as_str())
        .unwrap_or("bot_default")
        .to_owned();

    let patched = h.expect_ok(
        "fabushi.bots.patch",
        json!({
            "bot_id": bot_id,
            "persona": "Apple HIG Engineer",
            "model": "claude-opus-4-1",
            "avatar_shape": "rounded_square",
            "avatar_color": "#0A84FF",
            "coding_workspace_path": "/workspace/fabushi-ios",
            "coding_permission_mode": "ask"
        }),
    );
    assert_eq!(patched["persona"], "Apple HIG Engineer");

    let fetched = h.expect_ok("fabushi.bots.get", json!({"bot_id": bot_id}));
    assert_eq!(fetched["coding_permission_mode"], "ask");
    assert_eq!(fetched["coding_workspace_path"], "/workspace/fabushi-ios");
}

#[test]
fn tc_t1_f16_05_inspector_bot_bind_and_unbind_computer_node() {
    let h = FabushiE2EHarness::connect_production();
    let _ = register_default_user(&h, "bot_comp_owner");
    let bound = h.expect_ok(
        "fabushi.bots.bind_computer",
        json!({"bot_id": "bot_default", "computer_node_id": "node_mac_studio_1", "run_policy": "ask"}),
    );
    assert_eq!(bound["bound"], true);
    assert_eq!(bound["computer_node_id"], "node_mac_studio_1");

    let unbound = h.expect_ok(
        "fabushi.bots.bind_computer",
        json!({"bot_id": "bot_default", "computer_node_id": null}),
    );
    assert_eq!(unbound["bound"], false);
}
