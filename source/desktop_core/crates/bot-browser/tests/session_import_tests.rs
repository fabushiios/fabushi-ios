use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aes::Aes128;
use async_trait::async_trait;
use cbc::cipher::generic_array::GenericArray;
use cbc::cipher::{BlockEncryptMut, KeyIvInit};
use pbkdf2::pbkdf2_hmac;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use bot_browser::vault::{CredentialVault, MemoryMasterKeyStore};
use bot_browser::{
    BotBrowserError, BrowserBackendKind, BrowserCookie, BrowserProfileData, BrowserProfileStore,
    BrowserSessionBackend, BrowserSnapshot, ChromeImportOptions, ChromeImportProbe,
    KeyInputEvent, LocalStorageMap, NoopChromeKeyProvider, NormalizedOrigin,
    PointerInputEvent, ProfileNamespace, SameSitePolicy, SemanticLocator, StaticChromeKeyProvider,
    UnlockedUserVaultKey, UserVaultCategory, classify_probe_io_error,
    import_cookies_from_chrome_sqlite, import_cookies_from_json, import_cookies_from_netscape,
    import_cookies_from_text, is_valid_vault_item_key, probe_chrome_cookies_at,
};

#[test]
fn browser_cookie_and_profile_data_redact_debug_and_zeroize() {
    let mut cookie = BrowserCookie::new("auth_token", "SUPER_SECRET_TOKEN_XYZ", ".x.com");
    cookie.http_only = true;
    cookie.secure = true;
    cookie.same_site = Some(SameSitePolicy::Lax);
    let dbg = format!("{cookie:?}");
    assert!(!dbg.contains("SUPER_SECRET_TOKEN_XYZ"));
    assert!(dbg.contains("[REDACTED]"));

    let mut ls: LocalStorageMap = BTreeMap::new();
    let mut origin_map = BTreeMap::new();
    origin_map.insert("oauth_secret".to_string(), "TOP_SECRET_LS_VALUE".to_string());
    ls.insert("https://x.com".to_string(), origin_map);

    let profile = BrowserProfileData {
        cookies: vec![cookie],
        local_storage: ls,
        updated_at_ms: 1_700_000_000_000,
    };
    let prof_dbg = format!("{profile:?}");
    assert!(!prof_dbg.contains("SUPER_SECRET_TOKEN_XYZ"));
    assert!(!prof_dbg.contains("TOP_SECRET_LS_VALUE"));
    assert!(prof_dbg.contains("cookie_count: 1"));
    assert!(prof_dbg.contains("origin_count: 1"));
}

#[test]
fn import_cookies_from_json_supports_cdp_editthiscookie_and_netscape() {
    // 1. Standard array
    let std_json = r#"[
        {
            "name": "auth_token",
            "value": "tok_123",
            "domain": ".x.com",
            "path": "/",
            "secure": true,
            "httpOnly": true,
            "sameSite": "Lax",
            "expires": 1999999999
        }
    ]"#;
    let c1 = import_cookies_from_json(std_json).expect("std json");
    assert_eq!(c1.len(), 1);
    assert_eq!(c1[0].name, "auth_token");
    assert_eq!(c1[0].value, "tok_123");
    assert!(c1[0].http_only);
    assert_eq!(c1[0].same_site, Some(SameSitePolicy::Lax));
    assert_eq!(c1[0].expires, Some(1999999999));
    assert!(!c1[0].session);

    // 2. EditThisCookie JSON (float expirationDate, no_restriction / unspecified)
    let etc_json = r#"[
        {
            "domain": ".x.com",
            "expirationDate": 1850000000.75,
            "hostOnly": false,
            "httpOnly": true,
            "name": "ct0",
            "path": "/",
            "sameSite": "no_restriction",
            "secure": true,
            "session": false,
            "value": "csrf_abc"
        },
        {
            "domain": "x.com",
            "hostOnly": true,
            "httpOnly": false,
            "name": "lang",
            "path": "/",
            "sameSite": "unspecified",
            "secure": false,
            "session": true,
            "value": "en"
        }
    ]"#;
    let c2 = import_cookies_from_text(etc_json).expect("editthiscookie");
    assert_eq!(c2.len(), 2);
    assert_eq!(c2[0].same_site, Some(SameSitePolicy::None));
    assert_eq!(c2[0].expires, Some(1850000001));
    assert_eq!(c2[1].same_site, None);
    assert!(c2[1].session);

    // 3. CDP Network.getCookies and raw JSON-RPC wrapper
    let cdp_obj = r#"{"cookies":[{"name":"sid","value":"v1","domain":"example.com"}]}"#;
    assert_eq!(import_cookies_from_json(cdp_obj).unwrap().len(), 1);
    let cdp_rpc = r#"{"id":1,"result":{"cookies":[{"name":"sid2","value":"v2","domain":"example.com"}]}}"#;
    assert_eq!(import_cookies_from_json(cdp_rpc).unwrap().len(), 1);

    // 4. Netscape cookies.txt (with BOM, comments, #HttpOnly_, TRUE/FALSE, 0 session expiry)
    let netscape = "\u{feff}# Netscape HTTP Cookie File\n\
        #HttpOnly_.x.com\tTRUE\t/\tTRUE\t1950000000\tauth_token\tnetscape_secret\n\
        x.com\tFALSE\t/\tFALSE\t0\tguest_id\tg123\n";
    let c4 = import_cookies_from_netscape(netscape).expect("netscape");
    assert_eq!(c4.len(), 2);
    assert!(c4[0].http_only);
    assert!(!c4[0].host_only);
    assert_eq!(c4[0].value, "netscape_secret");
    assert!(!c4[1].http_only);
    assert!(c4[1].host_only);
    assert!(c4[1].session);

    // Also works through import_cookies_from_json auto-delegation
    assert_eq!(import_cookies_from_json(netscape).unwrap().len(), 2);

    // 5. Rejects malformed/injection inputs without echoing secret values
    assert!(import_cookies_from_json("   ").is_err());
    let err_bad_json = import_cookies_from_json("[{bad_secret_token").unwrap_err();
    assert!(!err_bad_json.to_string().contains("bad_secret_token"));

    let err_inj = import_cookies_from_json(
        r#"[{"name":"a","value":"SECRET_CRLF\r\nSet-Cookie: evil=1","domain":"x.com"}]"#,
    )
    .unwrap_err();
    assert!(!err_inj.to_string().contains("SECRET_CRLF"));
}

#[test]
fn browser_profile_store_local_and_e2ee_cookie_sync_roundtrip() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let store1 = BrowserProfileStore::new(tmp1.path());
    let store2 = BrowserProfileStore::new(tmp2.path());

    let origin = NormalizedOrigin::parse("https://x.com").unwrap();
    let ns = "x-account-main";

    // Profile key length cap (> 200 char namespace/origin -> <= 128 and valid E2EE item_key)
    let huge_ns = "a".repeat(250);
    let huge_origin = NormalizedOrigin::parse(&format!("https://{}.example.com", "sub".repeat(60))).unwrap();
    let key_long = ProfileNamespace::profile_key_for(&huge_ns, Some(&huge_origin));
    assert!(key_long.len() <= 128, "key len = {}", key_long.len());
    assert!(is_valid_vault_item_key(&key_long));

    let mut c1 = BrowserCookie::new("auth_token", "secret_val_1", ".x.com");
    c1.http_only = true;
    c1.secure = true;
    c1.expires = Some(2_000_000_000);
    c1.session = false;

    let mut c_expired = BrowserCookie::new("old_tok", "stale", ".x.com");
    c_expired.expires = Some(1_000_000);
    c_expired.session = false;

    let mut ls: LocalStorageMap = BTreeMap::new();
    let mut kv = BTreeMap::new();
    kv.insert("theme".to_string(), "dark".to_string());
    kv.insert("draft_id".to_string(), "d-42".to_string());
    ls.insert("https://x.com:443/home".to_string(), kv);

    let data = BrowserProfileData {
        cookies: vec![c1.clone(), c_expired],
        local_storage: ls,
        updated_at_ms: 1_700_000_000_000,
    };

    let dir = store1.save_profile(ns, Some(&origin), &data).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("cookies.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    let active = store1
        .load_active_cookies(ns, Some(&origin), 1_700_000_000)
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "auth_token");

    // E2EE sync roundtrip via UnlockedUserVaultKey -> UserVaultCategory::BrowserCookie
    let user_key = UnlockedUserVaultKey::from_bytes([0x42u8; 32]);
    let blob = store1
        .seal_to_vault_blob(&user_key, 42, ns, Some(&origin), 1, 16 * 1024 * 1024)
        .unwrap();
    assert_eq!(blob.category, UserVaultCategory::BrowserCookie);

    let (restored_key, restored_data) = store2.restore_from_vault_blob(&user_key, 42, &blob).unwrap();
    assert_eq!(restored_key, BrowserProfileStore::profile_key(ns, Some(&origin)));
    assert_eq!(restored_data.cookies.len(), 2);
    assert_eq!(restored_data.cookies[0].value, "secret_val_1");
    assert_eq!(
        restored_data
            .local_storage
            .get("https://x.com")
            .and_then(|m| m.get("theme"))
            .map(String::as_str),
        Some("dark")
    );

    // Local CredentialVault encrypted backup/restore roundtrip
    let vault = CredentialVault::open_in_memory(&MemoryMasterKeyStore::new()).unwrap();
    store1
        .backup_to_credential_vault(&vault, ns, Some(&origin), 16 * 1024 * 1024)
        .unwrap();
    assert_eq!(vault.list_profile_keys().unwrap().len(), 1);

    let tmp3 = TempDir::new().unwrap();
    let store3 = BrowserProfileStore::new(tmp3.path());
    assert!(
        store3
            .restore_from_credential_vault(&vault, ns, Some(&origin))
            .unwrap()
    );
    let restored3 = store3.load_profile(ns, Some(&origin)).unwrap();
    assert_eq!(restored3.cookies.len(), 2);
    assert_eq!(restored3.cookies[0].value, "secret_val_1");
}

struct LocalHttpBackend {
    client: reqwest::Client,
    cookies: Mutex<Vec<BrowserCookie>>,
    current_url: Mutex<String>,
    last_body: Mutex<String>,
    last_status: Mutex<u16>,
}

impl LocalHttpBackend {
    fn new() -> Self {
        Self {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            cookies: Mutex::new(Vec::new()),
            current_url: Mutex::new(String::new()),
            last_body: Mutex::new(String::new()),
            last_status: Mutex::new(0),
        }
    }

    async fn cookie_header_for(&self, url: &str) -> String {
        let origin = NormalizedOrigin::parse(url).unwrap();
        let guard = self.cookies.lock().await;
        guard
            .iter()
            .filter(|c| c.matches_host(origin.host()))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn apply_set_cookies(&self, url: &str, headers: &reqwest::header::HeaderMap) {
        let origin = NormalizedOrigin::parse(url).unwrap();
        let mut guard = self.cookies.lock().await;
        for val in headers.get_all(reqwest::header::SET_COOKIE) {
            if let Ok(s) = val.to_str() {
                if let Some((nv, attrs)) = s.split_once(';').or(Some((s, ""))) {
                    if let Some((name, value)) = nv.split_once('=') {
                        let mut c = BrowserCookie::new(name.trim(), value.trim(), origin.host());
                        c.http_only = attrs.to_ascii_lowercase().contains("httponly");
                        guard.push(c);
                    }
                }
            }
        }
    }
}

#[async_trait]
impl BrowserSessionBackend for LocalHttpBackend {
    fn backend_kind(&self) -> BrowserBackendKind {
        BrowserBackendKind::MockCdp
    }

    async fn navigate(&self, url: &str) -> Result<bot_browser::NavigationResult, BotBrowserError> {
        let origin = NormalizedOrigin::parse(url)?;
        let cookie_hdr = self.cookie_header_for(url).await;
        let mut req = self.client.get(url);
        if !cookie_hdr.is_empty() {
            req = req.header(reqwest::header::COOKIE, cookie_hdr);
        }
        let resp = req.send().await.map_err(|e| BotBrowserError::Io(e.to_string()))?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let body = resp.text().await.map_err(|e| BotBrowserError::Io(e.to_string()))?;
        self.apply_set_cookies(url, &headers).await;
        *self.current_url.lock().await = url.to_string();
        *self.last_status.lock().await = status;
        *self.last_body.lock().await = body;
        Ok(bot_browser::NavigationResult {
            url: url.to_string(),
            origin: origin.as_str().to_string(),
            title: "Mock".to_string(),
            duration_ms: 1,
            backend: BrowserBackendKind::MockCdp,
        })
    }

    async fn go_back(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }
    async fn go_forward(&self) -> Result<(), BotBrowserError> {
        Ok(())
    }
    async fn reload(&self) -> Result<(), BotBrowserError> {
        let u = self.current_url.lock().await.clone();
        self.navigate(&u).await.map(|_| ())
    }
    async fn current_url(&self) -> Result<String, BotBrowserError> {
        Ok(self.current_url.lock().await.clone())
    }
    async fn page_title(&self) -> Result<String, BotBrowserError> {
        Ok("Mock".to_string())
    }
    async fn snapshot_dom(&self) -> Result<BrowserSnapshot, BotBrowserError> {
        let url = self.current_url.lock().await.clone();
        let body = self.last_body.lock().await.clone();
        Ok(BrowserSnapshot {
            url: url.clone(),
            origin: NormalizedOrigin::parse(&url)?.as_str().to_string(),
            title: "Mock".to_string(),
            text_excerpt: body,
            interactive_elements: Vec::new(),
            redacted_field_count: 0,
        })
    }
    async fn eval_js(&self, _script: &str) -> Result<Value, BotBrowserError> {
        Ok(json!(null))
    }
    async fn click_locator(&self, _locator: &SemanticLocator) -> Result<bot_browser::ActionOutcome, BotBrowserError> {
        let base = self.current_url.lock().await.clone();
        let origin = NormalizedOrigin::parse(&base)?;
        let login_url = format!("{}/login", origin.as_str());
        let resp = self
            .client
            .post(&login_url)
            .send()
            .await
            .map_err(|e| BotBrowserError::Io(e.to_string()))?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let body = resp.text().await.map_err(|e| BotBrowserError::Io(e.to_string()))?;
        self.apply_set_cookies(&login_url, &headers).await;
        *self.last_status.lock().await = status;
        *self.last_body.lock().await = body;
        Ok(bot_browser::ActionOutcome {
            ok: true,
            operation: "click_locator".into(),
            duration_ms: 1,
            matched_strategy: Some("test_id".into()),
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn fill_locator(&self, _locator: &SemanticLocator, _value: &str, _is_secret: bool) -> Result<bot_browser::ActionOutcome, BotBrowserError> {
        Ok(bot_browser::ActionOutcome {
            ok: true,
            operation: "fill_locator".into(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn press_key(&self, _key: &str) -> Result<bot_browser::ActionOutcome, BotBrowserError> {
        Ok(bot_browser::ActionOutcome {
            ok: true,
            operation: "press_key".into(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn scroll(&self, _dx: f64, _dy: f64) -> Result<bot_browser::ActionOutcome, BotBrowserError> {
        Ok(bot_browser::ActionOutcome {
            ok: true,
            operation: "scroll".into(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn wait_for_locator(&self, _locator: &SemanticLocator, _timeout_ms: u64) -> Result<bot_browser::ActionOutcome, BotBrowserError> {
        Ok(bot_browser::ActionOutcome {
            ok: true,
            operation: "wait_for_locator".into(),
            duration_ms: 1,
            matched_strategy: None,
            matched_selector: None,
            x: None,
            y: None,
            extracted_text: None,
        })
    }
    async fn capture_screenshot(&self) -> Result<Vec<u8>, BotBrowserError> {
        Ok(Vec::new())
    }
    async fn export_cookies(&self, domain_filter: Option<&str>) -> Result<Vec<BrowserCookie>, BotBrowserError> {
        let guard = self.cookies.lock().await;
        Ok(guard
            .iter()
            .filter(|c| domain_filter.is_none_or(|d| c.matches_host(d)))
            .cloned()
            .collect())
    }
    async fn import_cookies(&self, cookies: &[BrowserCookie]) -> Result<usize, BotBrowserError> {
        let mut guard = self.cookies.lock().await;
        guard.extend(cookies.iter().cloned());
        Ok(cookies.len())
    }
    async fn dispatch_pointer_event(&self, _event: PointerInputEvent) -> Result<(), BotBrowserError> {
        Ok(())
    }
    async fn dispatch_key_event(&self, _event: KeyInputEvent) -> Result<(), BotBrowserError> {
        Ok(())
    }
}

#[tokio::test]
async fn session_cookie_persistence_reuses_login_on_local_http_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let login_hits = Arc::new(AtomicUsize::new(0));
    let login_hits_srv = Arc::clone(&login_hits);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = listener.accept() => {
                    let hits = Arc::clone(&login_hits_srv);
                    tokio::spawn(async move {
                        let mut buf = [0u8; 4096];
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let first_line = req.lines().next().unwrap_or("");
                        let has_auth = req.lines().any(|l| {
                            l.to_ascii_lowercase().starts_with("cookie:") && l.contains("auth_token=session_valid_99")
                        });
                        let resp = if first_line.starts_with("POST /login") {
                            hits.fetch_add(1, Ordering::SeqCst);
                            "HTTP/1.1 200 OK\r\nSet-Cookie: auth_token=session_valid_99; Path=/; HttpOnly\r\nContent-Length: 9\r\nConnection: close\r\n\r\nlogged_in".to_string()
                        } else if first_line.starts_with("GET /dashboard") {
                            if has_auth {
                                "HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\nwelcome_dashboard".to_string()
                            } else {
                                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 12\r\nConnection: close\r\n\r\nunauthorized".to_string()
                            }
                        } else {
                            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nlogin_page".to_string()
                        };
                        let _ = sock.write_all(resp.as_bytes()).await;
                    });
                }
            }
        }
    });

    let base_url = format!("http://{addr}");
    let origin = NormalizedOrigin::parse(&base_url).unwrap();
    let tmp = TempDir::new().unwrap();
    let store = BrowserProfileStore::new(tmp.path());

    // Session 1: login, verify dashboard, export cookies to store
    let session1 = LocalHttpBackend::new();
    session1.navigate(&format!("{base_url}/login")).await.unwrap();
    session1
        .click_locator(&SemanticLocator::by_test_id("login-submit"))
        .await
        .unwrap();
    session1
        .navigate(&format!("{base_url}/dashboard"))
        .await
        .unwrap();
    assert_eq!(*session1.last_status.lock().await, 200);
    assert_eq!(session1.snapshot_dom().await.unwrap().text_excerpt, "welcome_dashboard");
    assert_eq!(login_hits.load(Ordering::SeqCst), 1);

    let exported = session1.export_cookies(None).await.unwrap();
    store.save_cookies("x-reuse", Some(&origin), &exported).unwrap();

    // Session 2: fresh backend restores cookies and hits /dashboard without re-logging in
    let session2 = LocalHttpBackend::new();
    let restored = store.load_cookies("x-reuse", Some(&origin)).unwrap();
    session2.import_cookies(&restored).await.unwrap();
    session2
        .navigate(&format!("{base_url}/dashboard"))
        .await
        .unwrap();
    assert_eq!(*session2.last_status.lock().await, 200);
    assert_eq!(session2.snapshot_dom().await.unwrap().text_excerpt, "welcome_dashboard");
    assert_eq!(login_hits.load(Ordering::SeqCst), 1, "must NOT re-hit /login");

    let _ = shutdown_tx.send(());
}

#[test]
fn probe_local_chrome_cookies_handles_not_found_available_and_tcc_eperm() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("Default/Cookies");
    assert_eq!(probe_chrome_cookies_at(&missing), ChromeImportProbe::NotFound);

    // Available file
    let prof_dir = tmp.path().join("Profile 1");
    std::fs::create_dir_all(&prof_dir).unwrap();
    let cookies_path = prof_dir.join("Cookies");
    std::fs::write(&cookies_path, b"SQLite format 3\0test").unwrap();
    match probe_chrome_cookies_at(&cookies_path) {
        ChromeImportProbe::Available {
            path,
            profile_name,
            size_bytes,
            ..
        } => {
            assert_eq!(path, cookies_path);
            assert_eq!(profile_name, "Profile 1");
            assert!(size_bytes > 0);
        }
        other => panic!("expected Available, got {other:?}"),
    }

    // Synthetic TCC EPERM (1) and EACCES (13) and ErrorKind::PermissionDenied
    let eperm = std::io::Error::from_raw_os_error(1);
    let eacces = std::io::Error::from_raw_os_error(13);
    let perm_kind = std::io::Error::new(ErrorKind::PermissionDenied, "TCC denied");
    for err in [eperm, eacces, perm_kind] {
        match classify_probe_io_error(&cookies_path, &err) {
            ChromeImportProbe::PermissionDeniedTcc { path, guidance } => {
                assert_eq!(path, cookies_path);
                assert!(!guidance.is_empty());
            }
            other => panic!("expected PermissionDeniedTcc, got {other:?}"),
        }
    }

    // Filesystem 0o000 test when not running as root
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let denied_path = prof_dir.join("CookiesDenied");
        std::fs::write(&denied_path, b"secret").unwrap();
        std::fs::set_permissions(&denied_path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Only assert if OS enforces 0o000 (skip if running as root UID 0).
        if std::fs::File::open(&denied_path).is_err() {
            assert!(matches!(
                probe_chrome_cookies_at(&denied_path),
                ChromeImportProbe::PermissionDeniedTcc { .. }
            ));
        }
        let _ = std::fs::set_permissions(&denied_path, std::fs::Permissions::from_mode(0o600));
    }
}

fn encrypt_chrome_v10(password: &str, plaintext: &[u8]) -> Vec<u8> {
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), b"saltysalt", 1003, &mut key);

    let pad_len = 16 - (plaintext.len() % 16);
    let mut padded = plaintext.to_vec();
    padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));

    let mut enc = cbc::Encryptor::<Aes128>::new_from_slices(&key, &[0x20u8; 16]).unwrap();
    for chunk in padded.chunks_exact_mut(16) {
        enc.encrypt_block_mut(GenericArray::from_mut_slice(chunk));
    }

    let mut out = b"v10".to_vec();
    out.extend_from_slice(&padded);
    out
}

#[test]
fn import_cookies_from_chrome_sqlite_decrypts_v10_and_skips_with_noop() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("Default/Cookies");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    let pw = "test-chrome-safe-storage-pw";
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
         INSERT INTO meta (key, value) VALUES ('version', '24');
         CREATE TABLE cookies (
             host_key TEXT NOT NULL,
             name TEXT NOT NULL,
             value TEXT NOT NULL,
             encrypted_value BLOB NOT NULL,
             path TEXT NOT NULL,
             expires_utc INTEGER NOT NULL,
             is_secure INTEGER NOT NULL,
             is_httponly INTEGER NOT NULL,
             samesite INTEGER NOT NULL
         );",
    )
    .unwrap();

    // Row 1: plaintext cookie
    conn.execute(
        "INSERT INTO cookies VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            ".x.com",
            "lang",
            "en",
            Vec::<u8>::new(),
            "/",
            0i64,
            0i64,
            0i64,
            1i64
        ],
    )
    .unwrap();

    // Row 2: v10 encrypted cookie with Chrome v24 32-byte SHA256(host_key) prefix
    let mut v24_payload = Sha256::digest(b".x.com").to_vec();
    v24_payload.extend_from_slice(b"v24_secret_auth_token");
    let v10_blob = encrypt_chrome_v10(pw, &v24_payload);
    let future_webkit_us = (2_000_000_000i64 + 11_644_473_600i64) * 1_000_000i64;
    conn.execute(
        "INSERT INTO cookies VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            ".x.com",
            "auth_token",
            "",
            v10_blob,
            "/",
            future_webkit_us,
            1i64,
            1i64,
            2i64
        ],
    )
    .unwrap();
    drop(conn);

    // 1. StaticChromeKeyProvider decrypts both plaintext and v10 v24 cookie
    let static_provider = StaticChromeKeyProvider::new(pw);
    assert!(!format!("{static_provider:?}").contains(pw));
    let report = import_cookies_from_chrome_sqlite(
        &db_path,
        &static_provider,
        &ChromeImportOptions {
            domain_filter: Some("x.com".into()),
            include_expired: false,
            skip_undecryptable: false,
        },
    )
    .unwrap();
    assert!(report.used_key_provider);
    assert_eq!(report.imported_count, 2);
    let auth = report
        .cookies
        .iter()
        .find(|c| c.name == "auth_token")
        .unwrap();
    assert_eq!(auth.value, "v24_secret_auth_token");
    assert_eq!(auth.same_site, Some(SameSitePolicy::Strict));
    assert_eq!(auth.expires, Some(2_000_000_000));

    // 2. NoopChromeKeyProvider with skip_undecryptable=true never touches Keychain & imports plaintext only
    let noop = NoopChromeKeyProvider;
    let noop_report = import_cookies_from_chrome_sqlite(
        &db_path,
        &noop,
        &ChromeImportOptions {
            domain_filter: None,
            include_expired: false,
            skip_undecryptable: true,
        },
    )
    .unwrap();
    assert!(!noop_report.used_key_provider);
    assert_eq!(noop_report.imported_count, 1);
    assert_eq!(noop_report.skipped_encrypted_count, 1);
    assert_eq!(noop_report.cookies[0].name, "lang");
}

#[test]
fn netscape_utf8_boundary_cookie_path_sanitization_and_dot_profile_key_rejected() {
    // 1. Multi-byte UTF-8 character spanning byte index 10 must not panic
    let utf8_netscape = "123456789\u{00e9}.com\tTRUE\t/\tFALSE\t0\ta\tb\n";
    let parsed = import_cookies_from_netscape(utf8_netscape).expect("UTF-8 domain must not panic");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].domain, "123456789\u{00e9}.com");
    assert_eq!(parsed[0].name, "a");
    assert_eq!(parsed[0].value, "b");

    let utf8_httponly = "#HttpOnly_123456789\u{00e9}.com\tTRUE\t/\tFALSE\t0\tc\td\n";
    let parsed_ho = import_cookies_from_netscape(utf8_httponly).expect("HttpOnly + UTF-8");
    assert_eq!(parsed_ho.len(), 1);
    assert!(parsed_ho[0].http_only);
    assert_eq!(parsed_ho[0].domain, "123456789\u{00e9}.com");

    // 2. cookie.path containing \r\n, ;, \0, or control chars rejected in JSON and Netscape
    for bad_json in [
        r#"[{"name":"sid","value":"v1","domain":"x.com","path":"/\r\nSet-Cookie: evil=1"}]"#,
        r#"[{"name":"sid","value":"v1","domain":"x.com","path":"/; HttpOnly"}]"#,
        r#"[{"name":"sid","value":"v1","domain":"x.com","path":"/\u0000evil"}]"#,
        r#"[{"name":"sid","value":"v1","domain":"x.com","path":"/\u0007bell"}]"#,
    ] {
        assert!(
            matches!(
                import_cookies_from_json(bad_json),
                Err(BotBrowserError::CookieImport(_))
            ),
            "expected CookieImport error for `{bad_json}`"
        );
    }

    for bad_netscape in [
        "x.com\tFALSE\t/; evil=1\tFALSE\t0\tsid\tv1\n",
        "x.com\tFALSE\t/\0evil\tFALSE\t0\tsid\tv1\n",
        "x.com\tFALSE\t/\x07evil\tFALSE\t0\tsid\tv1\n",
    ] {
        assert!(
            matches!(
                import_cookies_from_netscape(bad_netscape),
                Err(BotBrowserError::CookieImport(_))
            ),
            "expected CookieImport error for `{bad_netscape}`"
        );
    }

    // 3. BrowserProfileStore::profile_dir_by_key(".") must be rejected
    let tmp = TempDir::new().unwrap();
    let store = BrowserProfileStore::new(tmp.path());
    assert!(
        matches!(
            store.profile_dir_by_key("."),
            Err(BotBrowserError::Storage(_))
        ),
        "profile_dir_by_key(\".\") must be rejected"
    );
}
