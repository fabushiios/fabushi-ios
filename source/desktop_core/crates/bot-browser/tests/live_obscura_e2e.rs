use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bot_browser::cdp::LoginFormSelectors;
use bot_browser::orchestrator::{BotBrowserOrchestrator, OrchestratorConfig};
use bot_browser::sidecar::{ObscuraBinaryManifest, ObscuraDownloader, ProfileNamespace};
use bot_browser::{
    Argon2KdfParams, AssistCardPayload, BotBrowserEvent, BrowserProfileSnapshot, CredentialVault,
    LocalLoginSubmission, NormalizedOrigin, UserVaultCategory, UserVaultCrypto,
    VaultSyncCredentialPayload, generate_totp_at,
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::form_urlencoded;
use zeroize::Zeroizing;

const TEST_USERNAME: &str = "alice@obscura.test";
const TEST_PASSWORD: &str = "Sup3rS3cret!Pw#2026";
const TEST_TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";
const TEST_SESSION_TOKEN: &str = "obscura_live_session_tok_2026";

fn seed_cached_binary_into_device_dir(cache_dir: &Path, device_dir: &Path) {
    let src_dir = cache_dir.join("bin").join("0.2.7");
    let dst_dir = device_dir.join("bin").join("0.2.7");
    fs::create_dir_all(&dst_dir).expect("create device bin dir");

    let bin_name = if cfg!(windows) {
        "obscura.exe"
    } else {
        "obscura"
    };
    fs::copy(src_dir.join(bin_name), dst_dir.join(bin_name)).expect("copy cached obscura bin");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dst_dir.join(bin_name), fs::Permissions::from_mode(0o755))
            .expect("chmod cached obscura bin");
    }
    let stamp_name = format!("{bin_name}.sha256");
    if src_dir.join(&stamp_name).is_file() {
        fs::copy(src_dir.join(&stamp_name), dst_dir.join(&stamp_name))
            .expect("copy cached obscura stamp");
    }
}

async fn start_test_login_server() -> (
    u16,
    Arc<AtomicUsize>,
    tokio::sync::oneshot::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test login server");
    let port = listener.local_addr().unwrap().port();
    let login_hits = Arc::new(AtomicUsize::new(0));
    let hits_ref = Arc::clone(&login_hits);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = listener.accept() => {
                    let hits = Arc::clone(&hits_ref);
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 16384];
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                        let (head, body_start) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
                        let mut lines = head.lines();
                        let req_line = lines.next().unwrap_or("");
                        let mut parts = req_line.split_whitespace();
                        let method = parts.next().unwrap_or("GET");
                        let path = parts.next().unwrap_or("/");

                        let mut content_length = 0usize;
                        let mut cookie_header = String::new();
                        for line in lines {
                            if let Some((k, v)) = line.split_once(':') {
                                if k.eq_ignore_ascii_case("Content-Length") {
                                    content_length = v.trim().parse().unwrap_or(0);
                                } else if k.eq_ignore_ascii_case("Cookie") {
                                    cookie_header = v.trim().to_string();
                                }
                            }
                        }

                        let mut body_bytes = body_start.as_bytes().to_vec();
                        while body_bytes.len() < content_length {
                            let mut extra = vec![0u8; content_length - body_bytes.len()];
                            match sock.read(&mut extra).await {
                                Ok(0) | Err(_) => break,
                                Ok(m) => body_bytes.extend_from_slice(&extra[..m]),
                            }
                        }
                        let body_str = String::from_utf8_lossy(&body_bytes);

                        let resp = match (method, path) {
                            ("GET", "/login") => {
                                let html = r#"<!doctype html>
<html>
<head><title>Obscura Live Test Login</title></head>
<body>
  <h1>Sign In</h1>
  <form id="login-form" method="POST" action="/login">
    <input id="username" name="username" type="text" value="stale_user" />
    <input id="password" name="password" type="password" value="stale_secret_pw" />
    <input id="totp" name="totp" type="text" autocomplete="one-time-code" />
    <button id="login-btn" type="submit">Log In</button>
  </form>
</body>
</html>"#;
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    html.len(),
                                    html
                                )
                            }
                            ("POST", "/login") => {
                                let form: HashMap<String, String> =
                                    form_urlencoded::parse(body_str.as_bytes())
                                        .into_owned()
                                        .collect();
                                let now_secs = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs();
                                let totp_valid = [-30i64, 0, 30].into_iter().any(|delta| {
                                    let t = now_secs.saturating_add_signed(delta);
                                    generate_totp_at(TEST_TOTP_SECRET, t, 6, 30)
                                        .is_ok_and(|code| form.get("totp").map(String::as_str) == Some(code.as_str()))
                                });
                                let creds_ok = form.get("username").map(String::as_str) == Some(TEST_USERNAME)
                                    && form.get("password").map(String::as_str) == Some(TEST_PASSWORD)
                                    && totp_valid;

                                if creds_ok {
                                    hits.fetch_add(1, Ordering::SeqCst);
                                    format!(
                                        "HTTP/1.1 302 Found\r\nSet-Cookie: session_token={TEST_SESSION_TOKEN}; Path=/; HttpOnly\r\nLocation: /dashboard\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                    )
                                } else {
                                    let err_body = "Invalid credentials or TOTP";
                                    format!(
                                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                        err_body.len(),
                                        err_body
                                    )
                                }
                            }
                            ("GET", "/dashboard") => {
                                let has_session = cookie_header
                                    .split(';')
                                    .any(|c| c.trim() == format!("session_token={TEST_SESSION_TOKEN}"));
                                if has_session {
                                    let html = "<!doctype html><html><head><title>Dashboard</title></head><body><h1>Authenticated Dashboard for alice</h1></body></html>";
                                    format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                        html.len(),
                                        html
                                    )
                                } else {
                                    let html = "<!doctype html><html><head><title>Unauthorized</title></head><body><h1>Unauthorized</h1></body></html>";
                                    format!(
                                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                        html.len(),
                                        html
                                    )
                                }
                            }
                            _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                        };

                        let _ = sock.write_all(resp.as_bytes()).await;
                    });
                }
            }
        }
    });

    (port, login_hits, shutdown_tx)
}

#[tokio::test]
#[ignore = "downloads and runs real obscura v0.2.7 binary"]
async fn live_obscura_v027_totp_login_and_e2ee_profile_restore_e2e() {
    // Allow local loopback test server fetches in Obscura v0.2.7 (SSRF protection blocks 127.0.0.1 by default).
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("OBSCURA_ALLOW_PRIVATE_NETWORK", "1");
    }

    // 1. Download & SHA-256 verify real Obscura v0.2.7 binary (cached under system temp dir).
    let cache_dir = std::env::temp_dir().join("bot-browser-obscura-cache");
    let http_client = reqwest::Client::builder().no_proxy().build().unwrap();
    let bin_path = ObscuraDownloader::ensure_binary(
        &cache_dir,
        &ObscuraBinaryManifest::default_obscura(),
        &http_client,
    )
    .await
    .expect("download/verify real obscura v0.2.7 binary");
    assert!(bin_path.is_file(), "obscura binary must exist");

    let ver_out = std::process::Command::new(&bin_path)
        .arg("--version")
        .output()
        .expect("run obscura --version");
    assert!(ver_out.status.success());
    let ver_str = String::from_utf8_lossy(&ver_out.stdout);
    assert!(
        ver_str.contains("0.2.7"),
        "expected obscura 0.2.7, got: {ver_str}"
    );

    // 2. Start local HTTP TOTP login server.
    let (http_port, login_hits, http_shutdown_tx) = start_test_login_server().await;
    let origin_str = format!("http://127.0.0.1:{http_port}");
    let login_url = format!("{origin_str}/login");
    let dashboard_url = format!("{origin_str}/dashboard");
    let parsed_origin = NormalizedOrigin::parse(&origin_str).unwrap();

    // 3. Initialize E2EE Vault Key + Device A orchestrator using real ObscuraSessionFactory.
    let user_id = 88_001u64;
    let passphrase = "LiveObscuraE2EE!Passphrase#2026";
    let (unlocked_a, envelope, _recovery_key) =
        UserVaultCrypto::initialize_new(user_id, passphrase, Argon2KdfParams::fast_for_tests())
            .expect("initialize E2EE user vault");

    let device_a_dir = TempDir::new().expect("device_a_dir");
    let device_a_path = device_a_dir.path().to_path_buf();
    seed_cached_binary_into_device_dir(&cache_dir, device_a_dir.path());

    let vault_a = Arc::new(
        CredentialVault::open(device_a_dir.path().join("vault.db"), &unlocked_a)
            .expect("open vault_a"),
    );
    let orch_a = Arc::new(BotBrowserOrchestrator::new(
        OrchestratorConfig::new(device_a_dir.path()),
        Arc::clone(&vault_a),
    ));

    // 4. Navigate to /login & verify browser_snapshot redacts prefilled password.
    let nav_res = orch_a
        .browser_navigate("sess_live", &login_url)
        .await
        .expect("navigate to /login");
    assert_eq!(nav_res.origin, origin_str);

    let snap = orch_a
        .tool_browser_snapshot("sess_live", None)
        .await
        .expect("browser_snapshot");
    assert_eq!(snap.origin, origin_str);
    assert!(snap.redacted_field_count >= 2);
    let pw_elem = snap
        .interactive_elements
        .iter()
        .find(|el| el.id.as_deref() == Some("password"))
        .expect("#password element in snapshot");
    assert!(pw_elem.is_password);
    assert_eq!(pw_elem.value.as_deref(), Some("[REDACTED]"));
    let snap_json = serde_json::to_string(&snap).unwrap();
    assert!(!snap_json.contains("stale_secret_pw"));

    // 5. Submit credentials via vault_request_card + submit_login_card + vault_fill (real TOTP!).
    let selectors = LoginFormSelectors {
        username_selector: "#username".to_string(),
        password_selector: "#password".to_string(),
        totp_selector: Some("#totp".to_string()),
        submit_selector: Some("#login-btn".to_string()),
    };

    let mut events_rx = orch_a.subscribe_events();
    let orch_req = Arc::clone(&orch_a);
    let selectors_req = selectors.clone();
    let req_handle = tokio::spawn(async move {
        orch_req
            .tool_vault_request_card(
                "sess_live",
                "task_live_1",
                Some(5001),
                "Obscura Live Test",
                selectors_req,
            )
            .await
    });

    let card_id = loop {
        match tokio::time::timeout(Duration::from_secs(10), events_rx.recv())
            .await
            .expect("receive UpsertCard timeout")
            .expect("events_rx")
        {
            BotBrowserEvent::UpsertCard(AssistCardPayload::VaultLogin { card_id, .. }) => {
                break card_id;
            }
            _ => continue,
        }
    };

    orch_a
        .submit_login_card(LocalLoginSubmission {
            card_id,
            username: Zeroizing::new(TEST_USERNAME.to_string()),
            password: Zeroizing::new(TEST_PASSWORD.to_string()),
            totp_secret: Some(Zeroizing::new(TEST_TOTP_SECRET.to_string())),
            account_hint: None,
            allow_auto_reuse: true,
        })
        .await
        .expect("submit_login_card");

    let card_res = req_handle.await.unwrap().expect("tool_vault_request_card");
    assert_eq!(card_res.status, "saved");
    let cred_id = card_res.credential_id.expect("credential_id");

    let fill_res = orch_a
        .tool_vault_fill("sess_live", "task_live_1", Some(5001), &cred_id, selectors)
        .await
        .expect("tool_vault_fill");
    assert_eq!(fill_res.status, "filled_and_submitted");
    assert!(fill_res.used_totp);

    // Wait briefly for redirect to /dashboard and verify authenticated page.
    let mut dashboard_ok = false;
    for _ in 0..30 {
        let s = orch_a
            .tool_browser_snapshot("sess_live", None)
            .await
            .expect("post-login snapshot");
        if s.url.contains("/dashboard") && s.text_excerpt.contains("Authenticated Dashboard") {
            dashboard_ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(dashboard_ok, "expected redirect to authenticated /dashboard");
    assert_eq!(login_hits.load(Ordering::SeqCst), 1);

    // 6. Close session gracefully so Obscura flushes cookies.json, then E2EE export Password + BrowserCookie.
    orch_a
        .close_session("sess_live")
        .await
        .expect("close session A");

    let storage_dir_a =
        ProfileNamespace::storage_dir_for(device_a_dir.path(), "sess_live", Some(&parsed_origin));
    assert!(
        storage_dir_a.join("cookies.json").is_file(),
        "Obscura must persist cookies.json on graceful shutdown"
    );

    let exported_creds = vault_a.export_all_for_sync().expect("export creds");
    assert_eq!(exported_creds.len(), 1);
    let pw_blob = unlocked_a
        .seal_item(
            user_id,
            UserVaultCategory::Password,
            &exported_creds[0].credential_id,
            1,
            &serde_json::to_vec(&exported_creds[0]).unwrap(),
        )
        .expect("seal password blob");

    let packed_profile =
        BrowserProfileSnapshot::pack_storage_dir(&storage_dir_a, None, 24 * 1024 * 1024)
            .expect("pack obscura storage_dir");
    let cookie_blob = unlocked_a
        .seal_item(
            user_id,
            UserVaultCategory::BrowserCookie,
            "sess_live",
            2,
            &packed_profile,
        )
        .expect("seal browser profile blob");

    // 7. Wipe local storage completely (Device A gone).
    drop(orch_a);
    drop(vault_a);
    drop(device_a_dir);
    assert!(!device_a_path.exists());

    // 8. Restore into fresh Device B via E2EE decrypt.
    let device_b_dir = TempDir::new().expect("device_b_dir");
    seed_cached_binary_into_device_dir(&cache_dir, device_b_dir.path());

    let unlocked_b = UserVaultCrypto::unlock_with_passphrase(user_id, passphrase, &envelope)
        .expect("unlock on device B");
    let vault_b = Arc::new(
        CredentialVault::open(device_b_dir.path().join("vault.db"), &unlocked_b)
            .expect("open vault_b"),
    );

    let opened_pw = unlocked_b
        .open_item(user_id, &pw_blob)
        .expect("open password blob on device B");
    let restored_cred: VaultSyncCredentialPayload =
        serde_json::from_slice(&opened_pw).expect("deserialize password payload");
    assert!(vault_b.import_from_sync(&restored_cred).expect("import cred on B"));
    assert_eq!(vault_b.list_for_origin(&origin_str).unwrap().len(), 1);

    let opened_profile = unlocked_b
        .open_item(user_id, &cookie_blob)
        .expect("open cookie blob on device B");
    let storage_dir_b =
        ProfileNamespace::storage_dir_for(device_b_dir.path(), "sess_live", Some(&parsed_origin));
    BrowserProfileSnapshot::unpack_storage_dir(&opened_profile, &storage_dir_b)
        .expect("unpack restored obscura profile on device B");
    assert!(storage_dir_b.join("cookies.json").is_file());

    // 9. Restart real Obscura on Device B & navigate directly to /dashboard without re-logging in!
    let orch_b = Arc::new(BotBrowserOrchestrator::new(
        OrchestratorConfig::new(device_b_dir.path()),
        Arc::clone(&vault_b),
    ));

    orch_b
        .browser_navigate("sess_live", &dashboard_url)
        .await
        .expect("navigate to /dashboard on restored device B");
    let snap_b = orch_b
        .tool_browser_snapshot("sess_live", None)
        .await
        .expect("snapshot on device B");
    assert!(
        snap_b.url.contains("/dashboard"),
        "expected /dashboard url, got {}",
        snap_b.url
    );
    assert!(
        snap_b.text_excerpt.contains("Authenticated Dashboard for alice"),
        "expected restored session cookie auth on /dashboard, got: {}",
        snap_b.text_excerpt
    );

    orch_b
        .close_session("sess_live")
        .await
        .expect("close session B");
    let _ = http_shutdown_tx.send(());
}
