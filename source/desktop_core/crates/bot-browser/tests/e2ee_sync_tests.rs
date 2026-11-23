use std::fs;
use std::io::Cursor;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use bot_browser::{
    AiOauthVaultBundle, Argon2KdfParams, BotBrowserError, BrowserProfileSnapshot,
    CredentialVault, DeviceKeyTransfer, DeviceSyncKeypair, EncryptedVaultBlob,
    MemoryMasterKeyStore, ReusePolicy, SplitKeyBundle, SplitKeyVaultKek,
    USER_VAULT_KDF_ALG_DEVICE_KEYCHAIN_V2, USER_VAULT_KDF_ALG_SPLIT_KEY_V2,
    UnlockedUserVaultKey, UserVaultCategory, UserVaultCrypto, VaultSyncCredentialPayload,
    derive_split_key, generate_split_key_salt_b64,
};
use sha2::{Digest, Sha256};
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::json;
use tar::{Builder, EntryType, Header};
use tempfile::TempDir;
use zeroize::Zeroize;

#[test]
fn argon2id_initialize_unlock_and_recovery_key_roundtrip() {
    let user_id = 42_001u64;
    let passphrase = "Correct Horse Battery Staple #2026!";
    let (unlocked, envelope, recovery_key) =
        UserVaultCrypto::initialize_new(user_id, passphrase, Argon2KdfParams::fast_for_tests())
            .expect("initialize user vault");

    assert_eq!(envelope.key_version, 1);
    assert_eq!(envelope.kdf_alg, "argon2id");
    assert!(envelope.recovery_wrapped_root_key_b64.is_some());
    assert!(recovery_key.contains('-'));

    let debug_str = format!("{unlocked:?}");
    assert!(debug_str.contains("[REDACTED]"));

    // Passphrase unlock succeeds with matching user_id.
    let unlocked_pw =
        UserVaultCrypto::unlock_with_passphrase(user_id, passphrase, &envelope).expect("unlock pw");
    assert_eq!(unlocked_pw.as_bytes(), unlocked.as_bytes());

    // Recovery key unlock succeeds with original formatting AND normalized lowercase/no-dash input.
    let unlocked_rec =
        UserVaultCrypto::unlock_with_recovery_key(user_id, &recovery_key, &envelope)
            .expect("unlock recovery");
    assert_eq!(unlocked_rec.as_bytes(), unlocked.as_bytes());

    let normalized_rec = recovery_key.replace('-', " ").to_ascii_lowercase();
    let unlocked_rec_norm =
        UserVaultCrypto::unlock_with_recovery_key(user_id, &normalized_rec, &envelope)
            .expect("unlock normalized recovery");
    assert_eq!(unlocked_rec_norm.as_bytes(), unlocked.as_bytes());

    // Wrong passphrase, empty passphrase, wrong recovery key, and wrong user_id MUST fail.
    assert!(matches!(
        UserVaultCrypto::unlock_with_passphrase(user_id, "wrong-passphrase", &envelope),
        Err(BotBrowserError::Crypto(_))
    ));
    assert!(matches!(
        UserVaultCrypto::unlock_with_passphrase(user_id, "", &envelope),
        Err(BotBrowserError::Crypto(_))
    ));
    assert!(matches!(
        UserVaultCrypto::unlock_with_recovery_key(user_id, "AAAA-BBBB-CCCC-DDDD", &envelope),
        Err(BotBrowserError::Crypto(_))
    ));
    assert!(matches!(
        UserVaultCrypto::unlock_with_passphrase(user_id + 1, passphrase, &envelope),
        Err(BotBrowserError::Crypto(_))
    ));
    assert!(matches!(
        UserVaultCrypto::unlock_with_recovery_key(user_id + 1, &recovery_key, &envelope),
        Err(BotBrowserError::Crypto(_))
    ));
}

#[test]
fn item_seal_and_open_enforces_user_category_item_key_and_revision_aad() {
    let key = UnlockedUserVaultKey::from_bytes([0x5A; 32]);
    let user_id = 777u64;
    let plaintext = b"top-secret-e2ee-payload";

    let blob = key
        .seal_item(user_id, UserVaultCategory::Password, "cred_alpha", 3, plaintext)
        .expect("seal item");
    assert_eq!(blob.category, UserVaultCategory::Password);
    assert_eq!(blob.item_key, "cred_alpha");
    assert_eq!(blob.revision, 3);
    assert!(!blob.deleted);

    let opened = key.open_item(user_id, &blob).expect("open valid item");
    assert_eq!(&*opened, plaintext);

    // Cross-user decryption MUST fail even with identical key bytes because user_id is in HKDF+AAD.
    assert!(matches!(
        key.open_item(user_id + 1, &blob),
        Err(BotBrowserError::Crypto(_))
    ));

    // Tampered category MUST fail.
    let mut bad_cat = blob.clone();
    bad_cat.category = UserVaultCategory::BrowserCookie;
    assert!(matches!(
        key.open_item(user_id, &bad_cat),
        Err(BotBrowserError::Crypto(_))
    ));

    // Tampered item_key MUST fail.
    let mut bad_key = blob.clone();
    bad_key.item_key = "cred_beta".to_string();
    assert!(matches!(
        key.open_item(user_id, &bad_key),
        Err(BotBrowserError::Crypto(_))
    ));

    // Tampered revision MUST fail.
    let mut bad_rev = blob.clone();
    bad_rev.revision = 4;
    assert!(matches!(
        key.open_item(user_id, &bad_rev),
        Err(BotBrowserError::Crypto(_))
    ));

    // Tampered ciphertext MUST fail.
    let mut bad_ct = blob.clone();
    let mut chars: Vec<char> = bad_ct.ciphertext_b64.chars().collect();
    chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
    bad_ct.ciphertext_b64 = chars.into_iter().collect();
    assert!(matches!(
        key.open_item(user_id, &bad_ct),
        Err(BotBrowserError::Crypto(_))
    ));

    // Category roundtrip helpers.
    assert_eq!(
        UserVaultCategory::from_str(UserVaultCategory::BrowserCookie.as_str()).unwrap(),
        UserVaultCategory::BrowserCookie
    );
    assert_eq!(
        UserVaultCategory::from_str("ai_oauth").unwrap(),
        UserVaultCategory::AiOauth
    );
    assert!(UserVaultCategory::from_str("unknown").is_err());
}

#[test]
fn password_export_seal_open_import_and_tombstone_into_fresh_vault() {
    let dir_a = TempDir::new().expect("dir_a");
    let dir_b = TempDir::new().expect("dir_b");
    let dir_c = TempDir::new().expect("dir_c");
    let user_id = 9001u64;

    let (unlocked_a, envelope, _) =
        UserVaultCrypto::initialize_new(user_id, "master-pass-123", Argon2KdfParams::fast_for_tests())
            .expect("init");
    let vault_a = CredentialVault::open(dir_a.path().join("vault.sqlite"), &unlocked_a)
        .expect("open vault_a");

    let meta_norm = vault_a
        .store_credential(
            "https://Example.COM:443/login",
            "alice@example.com",
            "SecretPass!1",
            Some("JBSWY3DPEHPK3PXP"),
            None,
            None,
        )
        .expect("store norm");
    let meta_stripe = vault_a
        .store_credential(
            "https://dashboard.stripe.com/signin",
            "ops@acme.io",
            "StripeSecret!9",
            None,
            Some("ops@acme"),
            None,
        )
        .expect("store stripe");

    // Touch last_used_at_ms on meta_norm.
    let _ = vault_a
        .decrypt_for_origin(&meta_norm.credential_id, "https://example.com", None)
        .expect("decrypt_for_origin");

    let exported = vault_a.export_all_for_sync().expect("export_all_for_sync");
    assert_eq!(exported.len(), 2);
    for rec in &exported {
        let dbg = format!("{rec:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("SecretPass!1"));
        assert!(!dbg.contains("StripeSecret!9"));
        assert!(!dbg.contains("JBSWY3DPEHPK3PXP"));
    }

    // Seal each record as E2EE Password blob.
    let mut blobs: Vec<EncryptedVaultBlob> = Vec::new();
    for (idx, rec) in exported.iter().enumerate() {
        let json_bytes = serde_json::to_vec(rec).expect("serialize sync rec");
        let blob = unlocked_a
            .seal_item(
                user_id,
                UserVaultCategory::Password,
                &rec.credential_id,
                (idx + 1) as i64,
                &json_bytes,
            )
            .expect("seal password item");
        blobs.push(blob);
    }

    // Fresh device B unlocks same root key via passphrase and imports into a fresh SQLite vault.
    let unlocked_b =
        UserVaultCrypto::unlock_with_passphrase(user_id, "master-pass-123", &envelope)
            .expect("unlock_b");
    let vault_b = CredentialVault::open(dir_b.path().join("vault.sqlite"), &unlocked_b)
        .expect("open vault_b");

    for blob in &blobs {
        let pt = unlocked_b.open_item(user_id, blob).expect("open password blob");
        let rec: VaultSyncCredentialPayload =
            serde_json::from_slice(&pt).expect("deserialize sync rec");
        assert!(vault_b.import_from_sync(&rec).expect("first import changes"));
        // Re-importing identical record is an idempotent no-op returning false.
        assert!(!vault_b.import_from_sync(&rec).expect("duplicate import noop"));
    }

    let sec_b = vault_b
        .decrypt_for_origin(&meta_norm.credential_id, "https://example.com", None)
        .expect("decrypt on vault_b");
    assert_eq!(&*sec_b.username, "alice@example.com");
    assert_eq!(&*sec_b.password, "SecretPass!1");
    assert_eq!(
        sec_b.totp_secret.as_deref().map(String::as_str),
        Some("JBSWY3DPEHPK3PXP")
    );
    let stripe_meta_b = vault_b
        .get_meta(&meta_stripe.credential_id)
        .expect("stripe meta on b");
    assert_eq!(stripe_meta_b.reuse_policy, ReusePolicy::ConfirmEveryTime);

    // Cross-origin decryption on vault_b still fails!
    assert!(matches!(
        vault_b.decrypt_for_origin(&meta_norm.credential_id, "https://evil.com", None),
        Err(BotBrowserError::OriginMismatch { .. })
    ));

    // Also verify importing into vault_c with a DIFFERENT local MasterKeyStore re-encrypts locally.
    let local_store_c = Arc::new(MemoryMasterKeyStore::from_bytes([0x99; 32]));
    let vault_c = CredentialVault::open(dir_c.path().join("vault.sqlite"), local_store_c.as_ref())
        .expect("open vault_c");
    for rec in &exported {
        assert!(vault_c.import_from_sync(rec).expect("import into vault_c"));
    }
    let sec_c = vault_c
        .decrypt_for_origin(&meta_norm.credential_id, "https://example.com", None)
        .expect("decrypt on vault_c");
    assert_eq!(&*sec_c.password, "SecretPass!1");

    // Tombstone sync deletes credential cleanly.
    let mut tombstone = vault_b
        .export_for_sync(&meta_stripe.credential_id)
        .expect("export_for_sync");
    tombstone.deleted = true;
    assert!(vault_b.import_from_sync(&tombstone).expect("delete tombstone"));
    assert!(!vault_b.import_from_sync(&tombstone).expect("repeat tombstone noop"));
    assert!(matches!(
        vault_b.get_meta(&meta_stripe.credential_id),
        Err(BotBrowserError::CredentialNotFound(_))
    ));
}

#[test]
fn browser_profile_pack_seal_open_unpack_and_path_traversal_rejection() {
    let src_dir = TempDir::new().expect("src_dir");
    let dst_dir = TempDir::new().expect("dst_dir");
    let outside_dir = TempDir::new().expect("outside_dir");

    fs::create_dir_all(src_dir.path().join("Default/Local Storage")).unwrap();
    fs::create_dir_all(src_dir.path().join("Cache")).unwrap();
    fs::write(src_dir.path().join("Default/Cookies"), b"sqlite-cookie-bytes").unwrap();
    fs::write(
        src_dir.path().join("Default/Local Storage/000001.ldb"),
        b"local-storage-bytes",
    )
    .unwrap();
    // Transient files that MUST be skipped on pack:
    fs::write(src_dir.path().join("SingletonLock"), b"lock").unwrap();
    fs::write(src_dir.path().join("Default/LOCK"), b"lock").unwrap();
    fs::write(src_dir.path().join("Default/temp.tmp"), b"tmp").unwrap();
    fs::write(src_dir.path().join("Cache/blob.bin"), b"cached").unwrap();

    let cookies = vec![json!({
        "name": "session_token",
        "value": "secret-cookie-val-xyz",
        "domain": "127.0.0.1",
        "path": "/"
    })];

    // Exceeding max_bytes must error cleanly.
    assert!(matches!(
        BrowserProfileSnapshot::pack_storage_dir(src_dir.path(), Some(&cookies), 16),
        Err(BotBrowserError::Storage(_))
    ));

    let packed = BrowserProfileSnapshot::pack_storage_dir(
        src_dir.path(),
        Some(&cookies),
        24 * 1024 * 1024,
    )
    .expect("pack storage dir");

    let key = UnlockedUserVaultKey::from_bytes([0x33; 32]);
    let blob = key
        .seal_item(100, UserVaultCategory::BrowserCookie, "sess_main", 1, &packed)
        .expect("seal cookie profile");
    let opened = key.open_item(100, &blob).expect("open cookie profile");

    let restored_cookies = BrowserProfileSnapshot::unpack_storage_dir(&opened, dst_dir.path())
        .expect("unpack storage dir")
        .expect("cookies present");
    assert_eq!(restored_cookies, cookies);
    assert_eq!(
        fs::read(dst_dir.path().join("Default/Cookies")).unwrap(),
        b"sqlite-cookie-bytes"
    );
    assert_eq!(
        fs::read(dst_dir.path().join("Default/Local Storage/000001.ldb")).unwrap(),
        b"local-storage-bytes"
    );
    assert!(!dst_dir.path().join("SingletonLock").exists());
    assert!(!dst_dir.path().join("Default/LOCK").exists());
    assert!(!dst_dir.path().join("Default/temp.tmp").exists());
    assert!(!dst_dir.path().join("Cache").exists());

    // Path traversal / absolute path / symlink / hardlink archives MUST be rejected.
    let target_sub = outside_dir.path().join("profile_target");
    fs::create_dir_all(&target_sub).unwrap();

    for malicious_path in ["../escape.txt", "storage/../../escape.txt", "/etc/evil"] {
        let bad_tar = build_malicious_targz_regular(malicious_path, b"evil");
        let err = BrowserProfileSnapshot::unpack_storage_dir(&bad_tar, &target_sub)
            .expect_err("must reject path traversal");
        assert!(matches!(err, BotBrowserError::Storage(_)));
        assert!(!outside_dir.path().join("escape.txt").exists());
    }

    let bad_symlink = build_malicious_targz_link("storage/bad_sym", "/etc/passwd", EntryType::Symlink);
    assert!(matches!(
        BrowserProfileSnapshot::unpack_storage_dir(&bad_symlink, &target_sub),
        Err(BotBrowserError::Storage(_))
    ));

    let bad_hardlink = build_malicious_targz_link("storage/bad_hard", "etc/passwd", EntryType::Link);
    assert!(matches!(
        BrowserProfileSnapshot::unpack_storage_dir(&bad_hardlink, &target_sub),
        Err(BotBrowserError::Storage(_))
    ));
}

#[test]
fn ai_oauth_vault_bundle_roundtrip_debug_redaction_and_zeroize() {
    let mut bundle = AiOauthVaultBundle {
        providers_json: json!([{
            "id": "codex",
            "access_token": "oauth_secret_token_999"
        }]),
        subscriptions_json: json!({
            "refresh_token": "refresh_secret_token_888"
        }),
        updated_at_ms: 1_760_000_000_000,
    };

    let dbg = format!("{bundle:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains("oauth_secret_token_999"));
    assert!(!dbg.contains("refresh_secret_token_888"));

    let json_bytes = bundle.to_json_bytes().expect("serialize AiOauthVaultBundle");
    let key = UnlockedUserVaultKey::from_bytes([0x7E; 32]);
    let blob = key
        .seal_item(55, UserVaultCategory::AiOauth, "subscriptions_v1", 2, &json_bytes)
        .expect("seal oauth");
    let opened = key.open_item(55, &blob).expect("open oauth");
    let restored = AiOauthVaultBundle::from_json_bytes(&opened).expect("deserialize oauth");
    assert_eq!(restored, bundle);

    bundle.zeroize();
    assert_eq!(bundle.providers_json, serde_json::Value::Null);
    assert_eq!(bundle.subscriptions_json, serde_json::Value::Null);
    assert_eq!(bundle.updated_at_ms, 0);
}

#[test]
fn split_key_v2_domain_separation_redaction_and_validation() {
    let salt_a = generate_split_key_salt_b64();
    let salt_b = generate_split_key_salt_b64();
    assert_ne!(salt_a, salt_b);
    assert_eq!(BASE64_STANDARD.decode(&salt_a).unwrap().len(), 16);

    let params = Argon2KdfParams::fast_for_tests();
    let bundle_1: SplitKeyBundle =
        derive_split_key("My-Strong-Password!#1", &salt_a, &params).expect("derive split key");
    let bundle_2: SplitKeyBundle =
        derive_split_key("My-Strong-Password!#1", &salt_a, &params).expect("deterministic");

    assert_eq!(
        bundle_1.auth_verifier_b64.as_str(),
        bundle_2.auth_verifier_b64.as_str()
    );
    assert_eq!(bundle_1.vault_kek.as_bytes(), bundle_2.vault_kek.as_bytes());

    let verifier_bytes = BASE64_STANDARD
        .decode(bundle_1.auth_verifier_b64.as_str())
        .expect("decode verifier");
    assert_eq!(verifier_bytes.len(), 32);
    // Domain separation: server auth verifier MUST NEVER equal client vault KEK.
    assert_ne!(verifier_bytes.as_slice(), bundle_1.vault_kek.as_bytes());

    // Changing salt or password changes both auth_verifier and vault_kek.
    let diff_salt = derive_split_key("My-Strong-Password!#1", &salt_b, &params).unwrap();
    assert_ne!(
        diff_salt.auth_verifier_b64.as_str(),
        bundle_1.auth_verifier_b64.as_str()
    );
    assert_ne!(diff_salt.vault_kek.as_bytes(), bundle_1.vault_kek.as_bytes());

    let diff_pw = derive_split_key("Different-Password!#2", &salt_a, &params).unwrap();
    assert_ne!(
        diff_pw.auth_verifier_b64.as_str(),
        bundle_1.auth_verifier_b64.as_str()
    );
    assert_ne!(diff_pw.vault_kek.as_bytes(), bundle_1.vault_kek.as_bytes());

    // Debug output MUST redact both auth_verifier_b64 and SplitKeyVaultKek.
    let kek_dbg = format!("{:?}", bundle_1.vault_kek);
    assert!(kek_dbg.contains("[REDACTED]"));
    let bundle_dbg = format!("{bundle_1:?}");
    assert!(bundle_dbg.contains("[REDACTED]"));
    assert!(!bundle_dbg.contains(bundle_1.auth_verifier_b64.as_str()));

    // Empty password and non-16-byte salt MUST fail.
    assert!(matches!(
        derive_split_key("", &salt_a, &params),
        Err(BotBrowserError::Crypto(_))
    ));
    let short_salt = BASE64_STANDARD.encode([0x11u8; 8]);
    assert!(matches!(
        derive_split_key("valid-pw", &short_salt, &params),
        Err(BotBrowserError::Crypto(_))
    ));
    let _: SplitKeyVaultKek = SplitKeyVaultKek::from_bytes(*bundle_1.vault_kek.as_bytes());
}

#[test]
fn split_key_v2_initialize_unlock_wrong_pw_and_rewrap_preserves_recovery_and_items() {
    let user_id = 12_345u64;
    let params = Argon2KdfParams::fast_for_tests();
    let salt_v1 = generate_split_key_salt_b64();
    let split_v1 = derive_split_key("InitialPass!2026", &salt_v1, &params).expect("split v1");

    let (envelope, unlocked, emergency_kit) = UserVaultCrypto::initialize_with_split_key(
        user_id,
        &split_v1.vault_kek,
        &salt_v1,
        params,
    )
    .expect("initialize_with_split_key");

    assert_eq!(envelope.key_version, 2);
    assert_eq!(envelope.kdf_alg, USER_VAULT_KDF_ALG_SPLIT_KEY_V2);
    assert_eq!(envelope.kdf_salt_b64, salt_v1);
    assert!(emergency_kit.starts_with("EK-"));
    assert!(envelope.recovery_wrapped_root_key_b64.is_some());

    // Correct Split-Key unlocks identical VaultRootKey.
    let unlocked_2 =
        UserVaultCrypto::unlock_with_split_key(user_id, &envelope, &split_v1.vault_kek)
            .expect("unlock_with_split_key");
    assert_eq!(unlocked_2.as_bytes(), unlocked.as_bytes());

    // Wrong password KEK or wrong user_id MUST fail.
    let wrong_split = derive_split_key("WrongPass!2026", &salt_v1, &params).unwrap();
    assert!(matches!(
        UserVaultCrypto::unlock_with_split_key(user_id, &envelope, &wrong_split.vault_kek),
        Err(BotBrowserError::Crypto(_))
    ));
    assert!(matches!(
        UserVaultCrypto::unlock_with_split_key(user_id + 1, &envelope, &split_v1.vault_kek),
        Err(BotBrowserError::Crypto(_))
    ));

    // Emergency Kit unlocks both formatted (`EK-...`) and normalized (`ek ...`) input.
    let unlocked_ek =
        UserVaultCrypto::unlock_with_recovery_key(user_id, &emergency_kit, &envelope)
            .expect("unlock with EK");
    assert_eq!(unlocked_ek.as_bytes(), unlocked.as_bytes());
    let norm_ek = emergency_kit.replace('-', " ").to_ascii_lowercase();
    let unlocked_ek_norm =
        UserVaultCrypto::unlock_with_recovery_key(user_id, &norm_ek, &envelope)
            .expect("unlock normalized EK");
    assert_eq!(unlocked_ek_norm.as_bytes(), unlocked.as_bytes());

    // Seal a vault item BEFORE password rotation.
    let old_blob = unlocked
        .seal_item(
            user_id,
            UserVaultCategory::Password,
            "cred_before_pw_change",
            1,
            b"persisted-secret-across-rotation",
        )
        .expect("seal old item");

    // Password change -> rewrap_root_key_with_kek under a new salt + new KEK.
    let salt_v2 = generate_split_key_salt_b64();
    let split_v2 = derive_split_key("RotatedNewPass!2026", &salt_v2, &params).expect("split v2");
    let rotated_env = UserVaultCrypto::rewrap_root_key_with_kek(
        user_id,
        &unlocked,
        &envelope,
        &split_v2.vault_kek,
        &salt_v2,
        params,
    )
    .expect("rewrap_root_key_with_kek");

    assert_eq!(rotated_env.kdf_alg, USER_VAULT_KDF_ALG_SPLIT_KEY_V2);
    assert_eq!(rotated_env.kdf_salt_b64, salt_v2);
    assert_eq!(
        rotated_env.recovery_wrapped_root_key_b64,
        envelope.recovery_wrapped_root_key_b64
    );

    // Old password KEK MUST fail on rotated envelope.
    assert!(matches!(
        UserVaultCrypto::unlock_with_split_key(user_id, &rotated_env, &split_v1.vault_kek),
        Err(BotBrowserError::Crypto(_))
    ));

    // New password KEK unlocks same root key and decrypts existing item without re-encrypting items!
    let unlocked_rotated =
        UserVaultCrypto::unlock_with_split_key(user_id, &rotated_env, &split_v2.vault_kek)
            .expect("unlock rotated");
    assert_eq!(unlocked_rotated.as_bytes(), unlocked.as_bytes());
    let opened_old_item = unlocked_rotated
        .open_item(user_id, &old_blob)
        .expect("open old item after rewrap");
    assert_eq!(&*opened_old_item, b"persisted-secret-across-rotation");

    // Existing Emergency Kit STILL unlocks the rotated envelope!
    let unlocked_ek_after =
        UserVaultCrypto::unlock_with_recovery_key(user_id, &emergency_kit, &rotated_env)
            .expect("EK preserved after rewrap");
    assert_eq!(unlocked_ek_after.as_bytes(), unlocked.as_bytes());
}

#[test]
fn keychain_only_envelope_and_recovery_unlock_lifecycle() {
    let user_id = 88_990u64;
    let (envelope, unlocked, emergency_kit) =
        UserVaultCrypto::initialize_keychain_only(user_id).expect("initialize_keychain_only");

    assert_eq!(envelope.key_version, 2);
    assert_eq!(envelope.kdf_alg, USER_VAULT_KDF_ALG_DEVICE_KEYCHAIN_V2);
    assert!(envelope.kdf_salt_b64.is_empty());
    assert!(envelope.wrapped_root_key_b64.is_empty());
    assert!(envelope.recovery_wrapped_root_key_b64.is_some());
    assert!(emergency_kit.starts_with("EK-"));

    // Split-key unlock MUST refuse device_keychain_v2 envelope.
    let dummy_kek = SplitKeyVaultKek::from_bytes([0x42; 32]);
    assert!(matches!(
        UserVaultCrypto::unlock_with_split_key(user_id, &envelope, &dummy_kek),
        Err(BotBrowserError::Crypto(_))
    ));

    // Emergency Kit unlocks device_keychain_v2 cleanly.
    let unlocked_rec =
        UserVaultCrypto::unlock_with_recovery_key(user_id, &emergency_kit, &envelope)
            .expect("recover device_keychain_v2");
    assert_eq!(unlocked_rec.as_bytes(), unlocked.as_bytes());

    // Wrong user_id or bad recovery key fails.
    assert!(matches!(
        UserVaultCrypto::unlock_with_recovery_key(user_id + 1, &emergency_kit, &envelope),
        Err(BotBrowserError::Crypto(_))
    ));
    assert!(matches!(
        UserVaultCrypto::unlock_with_recovery_key(user_id, "EK-AAAA-BBBB-CCCC-DDDD", &envelope),
        Err(BotBrowserError::Crypto(_))
    ));
}

#[test]
fn x25519_device_trust_ring_verification_code_seal_open_and_aad_isolation() {
    let user_id = 55_123u64;
    let request_id = "req-device-trust-001";
    let requester = DeviceSyncKeypair::generate();
    let other_device = DeviceSyncKeypair::generate();
    let pub_b64 = requester.public_key_b64();

    let dbg = format!("{requester:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(dbg.contains(&pub_b64));

    // Verification code is deterministic "NNN-NNN" matching canonical SHA256 domain separation.
    let code_1 = DeviceSyncKeypair::verification_code(&pub_b64).expect("code 1");
    let code_2 = DeviceSyncKeypair::verification_code(&pub_b64).expect("code 2");
    assert_eq!(code_1, code_2);
    assert_eq!(code_1.len(), 7);
    assert_eq!(&code_1[3..4], "-");
    assert!(code_1[..3].chars().all(|c| c.is_ascii_digit()));
    assert!(code_1[4..].chars().all(|c| c.is_ascii_digit()));

    let pub_bytes = BASE64_STANDARD.decode(&pub_b64).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(b"gpui-telegram/device-sync-code/v1");
    hasher.update(&pub_bytes);
    let digest = hasher.finalize();
    let expected_num = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    assert_eq!(
        code_1,
        format!("{:03}-{:03}", expected_num / 1_000, expected_num % 1_000)
    );

    // Zero or malformed public keys MUST be rejected.
    let zero_pub = BASE64_STANDARD.encode([0u8; 32]);
    assert!(matches!(
        DeviceSyncKeypair::verification_code(&zero_pub),
        Err(BotBrowserError::Crypto(_))
    ));
    assert!(matches!(
        DeviceSyncKeypair::verification_code("not-valid-b64!!"),
        Err(BotBrowserError::Crypto(_))
    ));

    // Trusted device seals VaultRootKey for requester.
    let root_key = UnlockedUserVaultKey::from_bytes([0xA7; 32]);
    let sealed_item = root_key
        .seal_item(user_id, UserVaultCategory::Password, "site_1", 1, b"pw123")
        .expect("seal item");

    let transfer: DeviceKeyTransfer =
        DeviceSyncKeypair::seal_root_key_for_device(request_id, user_id, &root_key, &pub_b64)
            .expect("seal_root_key_for_device");

    // Wrong request_id AAD MUST fail.
    let wrong_req_kp = DeviceSyncKeypair::generate();
    let wrong_req_transfer = DeviceSyncKeypair::seal_root_key_for_device(
        request_id,
        user_id,
        &root_key,
        &wrong_req_kp.public_key_b64(),
    )
    .unwrap();
    assert!(matches!(
        wrong_req_kp.open_root_key_transfer("wrong-request-id", user_id, &wrong_req_transfer),
        Err(BotBrowserError::Crypto(_))
    ));

    // Wrong user_id AAD MUST fail.
    let wrong_uid_kp = DeviceSyncKeypair::generate();
    let wrong_uid_transfer = DeviceSyncKeypair::seal_root_key_for_device(
        request_id,
        user_id,
        &root_key,
        &wrong_uid_kp.public_key_b64(),
    )
    .unwrap();
    assert!(matches!(
        wrong_uid_kp.open_root_key_transfer(request_id, user_id + 1, &wrong_uid_transfer),
        Err(BotBrowserError::Crypto(_))
    ));

    // Unrelated device keypair MUST fail.
    assert!(matches!(
        other_device.open_root_key_transfer(request_id, user_id, &transfer),
        Err(BotBrowserError::Crypto(_))
    ));

    // Tampered ciphertext MUST fail.
    let tamper_kp = DeviceSyncKeypair::generate();
    let mut tampered = DeviceSyncKeypair::seal_root_key_for_device(
        request_id,
        user_id,
        &root_key,
        &tamper_kp.public_key_b64(),
    )
    .unwrap();
    let mut ct_bytes = BASE64_STANDARD
        .decode(&tampered.encrypted_root_key_b64)
        .unwrap();
    ct_bytes[0] ^= 0xFF;
    tampered.encrypted_root_key_b64 = BASE64_STANDARD.encode(ct_bytes);
    assert!(matches!(
        tamper_kp.open_root_key_transfer(request_id, user_id, &tampered),
        Err(BotBrowserError::Crypto(_))
    ));

    // Matching requester opens transfer and decrypts sealed vault item!
    let opened_root = requester
        .open_root_key_transfer(request_id, user_id, &transfer)
        .expect("open_root_key_transfer");
    assert_eq!(opened_root.as_bytes(), root_key.as_bytes());
    let opened_item = opened_root
        .open_item(user_id, &sealed_item)
        .expect("decrypt item after device transfer");
    assert_eq!(&*opened_item, b"pw123");
}

fn build_malicious_targz_regular(raw_path: &str, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let gz = GzEncoder::new(&mut out, Compression::default());
        let mut tar = Builder::new(gz);
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Regular);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        // Write raw path bytes directly into header name field to bypass tar::Builder path sanitizer.
        let header_bytes = header.as_mut_bytes();
        let path_bytes = raw_path.as_bytes();
        let copy_len = path_bytes.len().min(100);
        header_bytes[..100].fill(0);
        header_bytes[..copy_len].copy_from_slice(&path_bytes[..copy_len]);
        header.set_cksum();
        tar.append(&header, Cursor::new(data)).unwrap();
        let gz = tar.into_inner().unwrap();
        gz.finish().unwrap();
    }
    out
}

fn build_malicious_targz_link(path: &str, link_target: &str, kind: EntryType) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let gz = GzEncoder::new(&mut out, Compression::default());
        let mut tar = Builder::new(gz);
        let mut header = Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(0);
        header.set_mode(0o777);
        tar.append_link(&mut header, path, link_target).unwrap();
        let gz = tar.into_inner().unwrap();
        gz.finish().unwrap();
    }
    out
}
