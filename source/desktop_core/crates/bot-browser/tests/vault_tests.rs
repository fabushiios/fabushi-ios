use std::fs;
use std::sync::Arc;

use bot_browser::{
    generate_totp_at, generate_totp_now, AssistCardPayload, AssistCardStatus, BotBrowserError,
    CredentialMeta, CredentialVault, DecryptedSecret, FileMasterKeyStore, LocalLoginSubmission,
    MasterKeyStore, MemoryMasterKeyStore, NormalizedClickPoint, NormalizedOrigin, ReusePolicy,
};
use rusqlite::Connection;
use tempfile::TempDir;
use zeroize::{Zeroize, Zeroizing};

fn test_vault_on_disk(dir: &TempDir) -> (CredentialVault, std::path::PathBuf) {
    let key_path = dir.path().join("master.key");
    let db_path = dir.path().join("vault.sqlite");
    let key_store = FileMasterKeyStore::new(&key_path);
    let vault = CredentialVault::open(&db_path, &key_store).expect("open vault on disk");
    (vault, db_path)
}

#[test]
fn normalized_origin_canonicalization_and_rejection() {
    let o1 = NormalizedOrigin::parse("HTTPS://App.Example.COM:443/login?next=/home#top")
        .expect("https default port");
    assert_eq!(o1.as_str(), "https://app.example.com");
    assert_eq!(o1.host(), "app.example.com");

    let o2 = NormalizedOrigin::parse("http://EXAMPLE.com:80/index.html").expect("http default port");
    assert_eq!(o2.as_str(), "http://example.com");
    assert_eq!(o2.host(), "example.com");

    let o3 = NormalizedOrigin::parse("https://Example.COM:8443/path").expect("non-default port");
    assert_eq!(o3.as_str(), "https://example.com:8443");
    assert_eq!(o3.host(), "example.com");

    let o4 = NormalizedOrigin::parse("http://localhost:8080/auth").expect("localhost custom port");
    assert_eq!(o4.as_str(), "http://localhost:8080");
    assert_eq!(o4.host(), "localhost");

    let o5 = NormalizedOrigin::parse("https://[::1]:443/v1").expect("ipv6 default port");
    assert_eq!(o5.as_str(), "https://[::1]");
    assert_eq!(o5.host(), "[::1]");

    let o6 = NormalizedOrigin::parse("https://[::1]:8443/v1").expect("ipv6 custom port");
    assert_eq!(o6.as_str(), "https://[::1]:8443");
    assert_eq!(o6.host(), "[::1]");

    for invalid in [
        "",
        "   ",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,hi",
        "ftp://example.com",
        "ws://example.com:9222",
        "https://user:pass@example.com/login",
        "https://user@example.com/login",
        "https://@example.com/login",
        "example.com/login",
        "https://",
    ] {
        let err = NormalizedOrigin::parse(invalid).expect_err(&format!("must reject `{invalid}`"));
        assert!(matches!(err, BotBrowserError::InvalidOrigin(_)));
    }
}

#[test]
fn high_sensitivity_domain_classifier_and_default_reuse_policy() {
    let sensitive_urls = [
        "https://stripe.com/login",
        "https://dashboard.stripe.com/payments",
        "https://checkout.stripe.com/pay",
        "https://paypal.com/signin",
        "https://www.paypal.com/myaccount",
        "https://console.aws.amazon.com/console/home",
        "https://us-east-1.console.aws.amazon.com/ec2",
        "https://signin.aws.amazon.com/signin",
        "https://console.cloud.google.com/",
        "https://portal.azure.com/",
        "https://dash.cloudflare.com/",
        "https://namecheap.com/",
        "https://ap.www.namecheap.com/",
        "https://godaddy.com/",
        "https://porkbun.com/",
        "https://vercel.com/login",
        "https://alipay.com/",
        "https://pay.weixin.qq.com/",
    ];

    for raw in sensitive_urls {
        let origin = NormalizedOrigin::parse(raw).expect(raw);
        assert!(origin.is_high_sensitivity(), "{raw} must be high sensitivity");
        assert_eq!(
            ReusePolicy::default_for_origin(&origin),
            ReusePolicy::ConfirmEveryTime,
            "{raw} must default to ConfirmEveryTime"
        );
    }

    let normal_urls = [
        "https://example.com/login",
        "https://notstripe.com/login",
        "https://stripe.com.evil.org/login",
        "https://github.com/login",
        "http://localhost:3000/login",
    ];

    for raw in normal_urls {
        let origin = NormalizedOrigin::parse(raw).expect(raw);
        assert!(
            !origin.is_high_sensitivity(),
            "{raw} must NOT be high sensitivity"
        );
        assert_eq!(
            ReusePolicy::default_for_origin(&origin),
            ReusePolicy::AutoReuse,
            "{raw} must default to AutoReuse"
        );
    }
}

#[test]
fn encrypt_decrypt_roundtrip_zeroizing_debug_and_meta_leak_check() {
    let dir = TempDir::new().expect("tempdir");
    let (vault, db_path) = test_vault_on_disk(&dir);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.path().join("master.key"))
            .expect("key metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "FileMasterKeyStore must enforce 0o600 mode");
    }

    let meta: CredentialMeta = vault
        .store_credential(
            "https://App.Example.COM:443/signin",
            "alice.supersecret@example.com",
            "P@ssw0rd!VerySecret123",
            Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"),
            None,
            None,
        )
        .expect("store credential");

    assert!(meta.credential_id.starts_with("cred_"));
    assert_eq!(meta.origin, "https://app.example.com");
    assert_eq!(meta.account_hint, "al***@example.com");
    assert!(meta.has_totp);
    assert_eq!(meta.reuse_policy, ReusePolicy::AutoReuse);
    assert!(meta.last_used_at_ms.is_none());

    let meta_json = serde_json::to_string(&meta).expect("serialize meta");
    assert!(!meta_json.contains("alice.supersecret@example.com"));
    assert!(!meta_json.contains("P@ssw0rd!VerySecret123"));
    assert!(!meta_json.contains("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));

    // Verify raw SQLite file bytes contain no plaintext secrets or raw username.
    let db_bytes = fs::read(&db_path).expect("read sqlite bytes");
    let db_lossy = String::from_utf8_lossy(&db_bytes);
    assert!(!db_lossy.contains("alice.supersecret@example.com"));
    assert!(!db_lossy.contains("P@ssw0rd!VerySecret123"));
    assert!(!db_lossy.contains("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));

    let listed = vault
        .list_for_origin("https://app.example.com/other/path")
        .expect("list for origin");
    assert_eq!(listed, vec![meta.clone()]);

    let mut decrypted: DecryptedSecret = vault
        .decrypt_for_origin(
            &meta.credential_id,
            "HTTPS://app.example.com/login?step=2",
            Some("https://app.example.com/iframe"),
        )
        .expect("decrypt same origin");

    assert_eq!(&*decrypted.username, "alice.supersecret@example.com");
    assert_eq!(&*decrypted.password, "P@ssw0rd!VerySecret123");
    assert_eq!(
        decrypted.totp_secret.as_deref().map(String::as_str),
        Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")
    );

    let debug_str = format!("{decrypted:?}");
    assert_eq!(
        debug_str,
        "DecryptedSecret { username: [REDACTED], password: [REDACTED], has_totp: true }"
    );
    assert!(!debug_str.contains("alice.supersecret"));
    assert!(!debug_str.contains("P@ssw0rd"));
    assert!(!debug_str.contains("GEZDGNBV"));

    let submission = LocalLoginSubmission {
        card_id: "card_1".into(),
        username: Zeroizing::new("alice.supersecret@example.com".into()),
        password: Zeroizing::new("P@ssw0rd!VerySecret123".into()),
        totp_secret: Some(Zeroizing::new("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into())),
        account_hint: Some("al***@example.com".into()),
        allow_auto_reuse: true,
    };
    let sub_debug = format!("{submission:?}");
    assert!(!sub_debug.contains("alice.supersecret@example.com"));
    assert!(!sub_debug.contains("P@ssw0rd!VerySecret123"));
    assert!(!sub_debug.contains("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));
    assert!(sub_debug.contains("[REDACTED]"));

    // Explicit zeroize clears DecryptedSecret buffers.
    decrypted.zeroize();
    assert!(decrypted.username.is_empty());
    assert!(decrypted.password.is_empty());
    assert!(decrypted.totp_secret.is_none());

    // last_used_at_ms updated after decryption and persists across vault reopen.
    drop(vault);
    let (reopened, _) = test_vault_on_disk(&dir);
    let updated_meta = reopened
        .get_meta(&meta.credential_id)
        .expect("get meta after reopen");
    assert!(updated_meta.last_used_at_ms.is_some());
    let decrypted_again = reopened
        .decrypt_for_origin(&meta.credential_id, "https://app.example.com", None)
        .expect("decrypt after reopen");
    assert_eq!(&*decrypted_again.password, "P@ssw0rd!VerySecret123");
}

#[test]
fn cross_origin_and_tampered_aad_decryption_blocked() {
    let dir = TempDir::new().expect("tempdir");
    let (vault, db_path) = test_vault_on_disk(&dir);

    let meta = vault
        .store_credential(
            "https://app.example.com",
            "user",
            "TopSecretPass",
            Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"),
            None,
            None,
        )
        .expect("store");
    assert_eq!(meta.account_hint, "u***");

    // Different host
    let err = vault
        .decrypt_for_origin(&meta.credential_id, "https://evil.com/login", None)
        .expect_err("different host must fail");
    assert!(matches!(err, BotBrowserError::OriginMismatch { .. }));

    // Different subdomain
    let err = vault
        .decrypt_for_origin(&meta.credential_id, "https://sub.app.example.com", None)
        .expect_err("different subdomain must fail");
    assert!(matches!(err, BotBrowserError::OriginMismatch { .. }));

    // Different scheme
    let err = vault
        .decrypt_for_origin(&meta.credential_id, "http://app.example.com", None)
        .expect_err("http vs https must fail");
    assert!(matches!(err, BotBrowserError::OriginMismatch { .. }));

    // Different port
    let err = vault
        .decrypt_for_origin(&meta.credential_id, "https://app.example.com:8443", None)
        .expect_err("different port must fail");
    assert!(matches!(err, BotBrowserError::OriginMismatch { .. }));

    // Cross-origin iframe
    let err = vault
        .decrypt_for_origin(
            &meta.credential_id,
            "https://app.example.com",
            Some("https://evil-iframe.com/form"),
        )
        .expect_err("cross-origin iframe must fail");
    assert!(matches!(err, BotBrowserError::FrameOriginMismatch { .. }));

    // Cross-origin TOTP computation also blocked
    let err = vault
        .compute_totp_for_origin(&meta.credential_id, "https://evil.com", None, 59)
        .expect_err("cross-origin totp must fail");
    assert!(matches!(err, BotBrowserError::OriginMismatch { .. }));

    // Tamper SQLite row origin directly -> AAD authentication must reject decryption!
    {
        let conn = Connection::open(&db_path).expect("raw sqlite open");
        let changed = conn
            .execute(
                "UPDATE vault_credentials SET origin = 'https://evil.com' WHERE credential_id = ?1",
                rusqlite::params![meta.credential_id],
            )
            .expect("tamper row");
        assert_eq!(changed, 1);
    }

    let err = vault
        .decrypt_for_origin(&meta.credential_id, "https://evil.com", None)
        .expect_err("tampered DB origin must fail ChaCha20-Poly1305 AAD check");
    assert!(matches!(err, BotBrowserError::Crypto(_)));
}

#[test]
fn rfc6238_sha1_totp_vectors_and_otpauth_uri_parsing() {
    let rfc_secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    // RFC 6238 Appendix B SHA-1 test vectors (both 8-digit full & 6-digit truncated).
    let vectors = [
        (59u64, "94287082", "287082"),
        (1111111109u64, "07081804", "081804"),
        (1111111111u64, "14050471", "050471"),
        (1234567890u64, "89005924", "005924"),
        (2000000000u64, "69279037", "279037"),
    ];

    for (t, expected_8, expected_6) in vectors {
        let code8 = generate_totp_at(rfc_secret, t, 8, 30).expect("8-digit totp");
        assert_eq!(&*code8, expected_8, "t={t} 8-digit");

        let code6 = generate_totp_at(rfc_secret, t, 6, 30).expect("6-digit totp");
        assert_eq!(&*code6, expected_6, "t={t} 6-digit");
    }

    // Lowercase, spaces, hyphens, and '=' padding accepted.
    let messy = " gezd gnbv-gy3t qojq gezd-gnbv gy3t qojq==== ";
    assert_eq!(
        &*generate_totp_at(messy, 59, 6, 30).expect("messy base32"),
        "287082"
    );

    // otpauth://totp URI parsing.
    let uri = "otpauth://totp/Example:alice@example.com?secret=gezdgnbvgy3tqojqgezdgnbvgy3tqojq&issuer=Example&algorithm=SHA1&digits=6&period=30";
    assert_eq!(
        &*generate_totp_at(uri, 1111111109, 6, 30).expect("otpauth totp"),
        "081804"
    );
    let now_code = generate_totp_now(uri).expect("totp now");
    assert_eq!(now_code.len(), 6);
    assert!(now_code.chars().all(|c| c.is_ascii_digit()));

    // Vault TOTP integration using otpauth URI.
    let key_store = Arc::new(MemoryMasterKeyStore::new());
    let vault = CredentialVault::open_in_memory(key_store.as_ref()).expect("mem vault");
    let meta = vault
        .store_credential(
            "https://vault-totp.example.com",
            "alice@example.com",
            "pw",
            Some(uri),
            None,
            None,
        )
        .expect("store uri totp");
    let code = vault
        .compute_totp_for_origin(&meta.credential_id, "https://vault-totp.example.com", None, 59)
        .expect("compute totp");
    assert_eq!(&*code, "287082");

    // Reject hotp & invalid secrets.
    assert!(matches!(
        generate_totp_at("otpauth://hotp/Test?secret=GEZDGNBV", 59, 6, 30),
        Err(BotBrowserError::Totp(_))
    ));
    assert!(matches!(
        generate_totp_at("NOT_VALID_0189", 59, 6, 30),
        Err(BotBrowserError::Totp(_))
    ));
}

#[test]
fn update_policy_delete_and_type_helpers() {
    let key_store = MemoryMasterKeyStore::new();
    let key1 = key_store.load_or_create_key().expect("key1");
    let key2 = key_store.load_or_create_key().expect("key2");
    assert_eq!(&*key1, &*key2);

    let vault = CredentialVault::open_in_memory(&key_store).expect("mem vault");
    let meta = vault
        .store_credential(
            "https://dashboard.stripe.com/login",
            "billing@acme.io",
            "secret",
            None,
            None,
            None,
        )
        .expect("store stripe");
    assert_eq!(meta.reuse_policy, ReusePolicy::ConfirmEveryTime);

    vault
        .update_reuse_policy(&meta.credential_id, ReusePolicy::AutoReuse)
        .expect("update policy");
    assert_eq!(
        vault.get_meta(&meta.credential_id).expect("meta").reuse_policy,
        ReusePolicy::AutoReuse
    );

    vault
        .delete_credential(&meta.credential_id)
        .expect("delete credential");
    assert!(matches!(
        vault.get_meta(&meta.credential_id),
        Err(BotBrowserError::CredentialNotFound(_))
    ));
    assert!(matches!(
        vault.delete_credential(&meta.credential_id),
        Err(BotBrowserError::CredentialNotFound(_))
    ));

    // Verify AssistCardPayload helpers & NormalizedClickPoint clamp.
    let mut card = AssistCardPayload::VaultLogin {
        card_id: "card_123".into(),
        task_id: "task_1".into(),
        session_id: "sess_1".into(),
        chat_id: Some(42),
        origin: "https://example.com".into(),
        target_url: "https://example.com/login".into(),
        site_title: "Example".into(),
        suggested_reuse_policy: ReusePolicy::AutoReuse,
        is_high_sensitivity: false,
        status: AssistCardStatus::AwaitingUser,
        status_note: None,
        saved_credential_id: None,
        saved_account_hint: None,
        created_at_ms: 1000,
    };
    assert_eq!(card.card_id(), "card_123");
    assert_eq!(card.chat_id(), Some(42));
    assert_eq!(card.status(), AssistCardStatus::AwaitingUser);
    card.set_status(
        AssistCardStatus::TimedOutResumable,
        Some("timed out".into()),
    );
    assert_eq!(card.status(), AssistCardStatus::TimedOutResumable);

    let pt = NormalizedClickPoint::new(-0.5, 1.8);
    assert_eq!(pt.norm_x, 0.0);
    assert_eq!(pt.norm_y, 1.0);
}
