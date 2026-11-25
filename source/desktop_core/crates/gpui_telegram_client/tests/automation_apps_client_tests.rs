use gpui_telegram_client::{
    AuthResponse, AutomationAppDetail, ClientConfig, HttpClientError, InstalledAutomationApp,
    ListAutomationAppsResponse, ListInstalledAutomationAppsResponse, PublishAutomationAppRequest,
    RecordAutomationRunRequest, RecordAutomationRunResponse, TelegramClient,
    UninstallAutomationAppResponse,
};
use gpui_telegram_core::error::TelegramError;
use gpui_telegram_proto::automation_apps::{
    compute_automation_manifest_sha256, AutomationAppSummary, InstallAutomationAppResponse,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
struct CapturedHttpRequest {
    method: String,
    target: String,
    auth_header: Option<String>,
    _body: String,
}

/// Persistent SQLite-backed mock HTTP store server so client tests can exercise
/// Client A publish -> Client B search/get/install -> drop/reopen offline -> sync -> uninstall
/// without depending on `gpui_telegram_server`.
struct PersistentMockStoreServer {
    base_url: String,
    captured: Arc<Mutex<Vec<CapturedHttpRequest>>>,
    task: tokio::task::JoinHandle<()>,
}

impl PersistentMockStoreServer {
    async fn start(db_path: std::path::PathBuf) -> Self {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS apps (
                app_id TEXT PRIMARY KEY,
                author_user_id INTEGER NOT NULL,
                version TEXT NOT NULL,
                manifest_json TEXT NOT NULL,
                content_sha256 TEXT NOT NULL,
                install_count INTEGER NOT NULL DEFAULT 0,
                run_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS installs (
                user_id INTEGER NOT NULL,
                app_id TEXT NOT NULL,
                installed_at_ms INTEGER NOT NULL,
                last_run_at_ms INTEGER,
                PRIMARY KEY (user_id, app_id)
            );",
        )
        .unwrap();
        let db = Arc::new(Mutex::new(conn));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let db_clone = Arc::clone(&db);
        let cap_clone = Arc::clone(&captured);

        let task = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let db = Arc::clone(&db_clone);
                let cap = Arc::clone(&cap_clone);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 131072];
                    let mut total = 0usize;
                    let mut content_length = 0usize;
                    let mut header_end = None;
                    loop {
                        let n = match sock.read(&mut buf[total..]).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        total += n;
                        if header_end.is_none() {
                            if let Some(pos) = buf[..total]
                                .windows(4)
                                .position(|w| w == b"\r\n\r\n")
                            {
                                header_end = Some(pos + 4);
                                let head = String::from_utf8_lossy(&buf[..pos]);
                                for line in head.lines() {
                                    if let Some((k, v)) = line.split_once(':') {
                                        if k.eq_ignore_ascii_case("content-length") {
                                            content_length = v.trim().parse().unwrap_or(0);
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(he) = header_end {
                            if total >= he + content_length {
                                break;
                            }
                        }
                    }

                    let he = header_end.unwrap_or(total);
                    let head = String::from_utf8_lossy(&buf[..he]).to_string();
                    let body = String::from_utf8_lossy(&buf[he..total]).to_string();
                    let mut lines = head.lines();
                    let req_line = lines.next().unwrap_or("");
                    let mut parts = req_line.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let target = parts.next().unwrap_or("").to_string();
                    let auth_header = lines.find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        if k.eq_ignore_ascii_case("authorization") {
                            Some(v.trim().to_string())
                        } else {
                            None
                        }
                    });
                    cap.lock().unwrap().push(CapturedHttpRequest {
                        method: method.clone(),
                        target: target.clone(),
                        auth_header: auth_header.clone(),
                        _body: body.clone(),
                    });

                    let (status, resp_body) =
                        handle_mock_request(&db, &method, &target, auth_header.as_deref(), &body);
                    let reason = match status {
                        200 => "OK",
                        201 => "Created",
                        400 => "Bad Request",
                        401 => "Unauthorized",
                        403 => "Forbidden",
                        404 => "Not Found",
                        409 => "Conflict",
                        422 => "Unprocessable Entity",
                        _ => "Error",
                    };
                    let raw = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = sock.write_all(raw.as_bytes()).await;
                });
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            captured,
            task,
        }
    }
}

fn user_id_from_auth(auth: Option<&str>) -> Option<u64> {
    let tok = auth?.strip_prefix("Bearer ")?.trim();
    match tok {
        "tok_alice" => Some(1001),
        "tok_bob" => Some(2002),
        "tok_charlie" => Some(3003),
        _ => None,
    }
}

fn handle_mock_request(
    db: &Arc<Mutex<Connection>>,
    method: &str,
    target: &str,
    auth: Option<&str>,
    body: &str,
) -> (u16, String) {
    let (path, query_str) = target.split_once('?').unwrap_or((target, ""));
    let uid = user_id_from_auth(auth);

    // Forced error simulation path
    if path == "/api/v1/automation-apps/force-400" {
        return (
            400,
            json!({ "error": "invalid_manifest", "message": "bad manifest" }).to_string(),
        );
    }
    if path == "/api/v1/automation-apps/force-403" {
        return (
            403,
            json!({ "error": "forbidden", "message": "not owner" }).to_string(),
        );
    }
    if path == "/api/v1/automation-apps/force-409" {
        return (
            409,
            json!({ "error": "version_conflict", "message": "duplicate version" }).to_string(),
        );
    }
    if path == "/api/v1/automation-apps/force-422" {
        return (
            422,
            json!({ "error": "secret_leak_detected", "message": "plaintext auth_token detected" })
                .to_string(),
        );
    }
    if path == "/api/v1/automation-apps/empty-detail" {
        return (
            200,
            json!({ "apps": [], "total": 0 }).to_string(),
        );
    }

    let conn = db.lock().unwrap();
    match (method, path) {
        ("POST", "/api/v1/automation-apps") => {
            let Some(u) = uid else {
                return (
                    401,
                    json!({ "error": "missing_token", "message": "auth required" }).to_string(),
                );
            };
            let req: PublishAutomationAppRequest = serde_json::from_str(body).unwrap();
            let app_id = req.manifest["app_id"].as_str().unwrap().to_string();
            let slug = req.manifest["slug"].as_str().unwrap().to_string();
            let name = req.manifest["name"].as_str().unwrap().to_string();
            let version = req.manifest["version"].as_str().unwrap().to_string();
            let description = req.manifest["description"].as_str().unwrap_or("").to_string();
            let category = req.manifest["category"].as_str().unwrap_or("general").to_string();
            let target_domains: Vec<String> = serde_json::from_value(
                req.manifest.get("target_domains").cloned().unwrap_or(json!([])),
            )
            .unwrap_or_default();
            let manifest_json = serde_json::to_string(&req.manifest).unwrap();
            let sha = compute_automation_manifest_sha256(&manifest_json);

            conn.execute(
                "INSERT OR REPLACE INTO apps (app_id, author_user_id, version, manifest_json, content_sha256, install_count, run_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
                params![app_id, u as i64, version, manifest_json, sha],
            )
            .unwrap();

            let detail = AutomationAppDetail {
                app_id,
                slug,
                author_user_id: u,
                author_username: Some("alice".into()),
                name,
                description,
                category,
                target_domains,
                latest_version: version.clone(),
                versions: vec![version],
                manifest: req.manifest,
                content_sha256: sha,
                install_count: 0,
                run_count: 0,
                is_published: req.is_published,
                is_installed: false,
                installed_version: None,
                created_at_ms: 1_700_000_000_000,
                updated_at_ms: 1_700_000_000_000,
            };
            (201, serde_json::to_string(&detail).unwrap())
        }
        ("GET", "/api/v1/automation-apps") => {
            let params_map: HashMap<&str, &str> = query_str
                .split('&')
                .filter_map(|p| p.split_once('='))
                .collect();
            let limit: u32 = params_map
                .get("limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            let offset: u32 = params_map
                .get("offset")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let mut stmt = conn
                .prepare("SELECT app_id, author_user_id, version, manifest_json, install_count, run_count FROM apps")
                .unwrap();
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                })
                .unwrap();
            let mut summaries = Vec::new();
            for row in rows {
                let (app_id, author_id, version, manifest_json, installs, runs) = row.unwrap();
                let manifest: Value = serde_json::from_str(&manifest_json).unwrap();
                let name = manifest["name"].as_str().unwrap_or("").to_string();
                let slug = manifest["slug"].as_str().unwrap_or("").to_string();
                let description = manifest["description"].as_str().unwrap_or("").to_string();
                let category = manifest["category"].as_str().unwrap_or("").to_string();
                let target_domains: Vec<String> =
                    serde_json::from_value(manifest["target_domains"].clone()).unwrap_or_default();
                summaries.push(AutomationAppSummary {
                    app_id,
                    slug,
                    author_user_id: author_id as u64,
                    author_username: None,
                    name,
                    description,
                    category,
                    target_domains,
                    latest_version: version,
                    required_capabilities: vec![],
                    step_count: manifest["steps"].as_array().map(|a| a.len()).unwrap_or(0),
                    parameters: vec![],
                    install_count: installs as u64,
                    run_count: runs as u64,
                    is_published: true,
                    is_installed: false,
                    created_at_ms: 1_700_000_000_000,
                    updated_at_ms: 1_700_000_000_000,
                });
            }
            let total = summaries.len();
            let resp = ListAutomationAppsResponse {
                apps: summaries,
                total,
                limit,
                offset,
            };
            (200, serde_json::to_string(&resp).unwrap())
        }
        ("GET", "/api/v1/automation-apps/installed") => {
            let Some(u) = uid else {
                return (
                    401,
                    json!({ "error": "missing_token", "message": "auth required" }).to_string(),
                );
            };
            let mut stmt = conn
                .prepare(
                    "SELECT a.app_id, a.version, a.manifest_json, a.content_sha256, i.installed_at_ms, i.last_run_at_ms
                     FROM installs i JOIN apps a ON a.app_id = i.app_id WHERE i.user_id = ?1",
                )
                .unwrap();
            let rows = stmt
                .query_map(params![u as i64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                    ))
                })
                .unwrap();
            let mut apps = Vec::new();
            for row in rows {
                let (app_id, version, manifest_json, sha, inst_at, last_run) = row.unwrap();
                let manifest: Value = serde_json::from_str(&manifest_json).unwrap();
                apps.push(InstalledAutomationApp {
                    app_id,
                    slug: manifest["slug"].as_str().unwrap_or("").into(),
                    name: manifest["name"].as_str().unwrap_or("").into(),
                    description: manifest["description"].as_str().unwrap_or("").into(),
                    category: manifest["category"].as_str().unwrap_or("").into(),
                    target_domains: serde_json::from_value(manifest["target_domains"].clone())
                        .unwrap_or_default(),
                    installed_version: version.clone(),
                    latest_version: version,
                    manifest,
                    content_sha256: sha,
                    installed_at_ms: inst_at,
                    last_run_at_ms: last_run,
                    local_run_count: 0,
                });
            }
            let total = apps.len();
            (
                200,
                serde_json::to_string(&ListInstalledAutomationAppsResponse { apps, total })
                    .unwrap(),
            )
        }
        _ if method == "POST"
            && path.starts_with("/api/v1/automation-apps/")
            && path.ends_with("/install") =>
        {
            let Some(u) = uid else {
                return (
                    401,
                    json!({ "error": "missing_token", "message": "auth required" }).to_string(),
                );
            };
            let app_id = path
                .trim_start_matches("/api/v1/automation-apps/")
                .trim_end_matches("/install");
            let row: Option<(String, String, String, i64)> = conn
                .query_row(
                    "SELECT version, manifest_json, content_sha256, install_count FROM apps WHERE app_id = ?1",
                    params![app_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
                .unwrap();
            let Some((version, manifest_json, sha, prev_installs)) = row else {
                return (
                    404,
                    json!({ "error": "not_found", "message": "not found" }).to_string(),
                );
            };
            let inst_at = 1_700_000_010_000i64;
            conn.execute(
                "INSERT OR IGNORE INTO installs (user_id, app_id, installed_at_ms) VALUES (?1, ?2, ?3)",
                params![u as i64, app_id, inst_at],
            )
            .unwrap();
            let new_count = prev_installs + 1;
            conn.execute(
                "UPDATE apps SET install_count = ?1 WHERE app_id = ?2",
                params![new_count, app_id],
            )
            .unwrap();
            let manifest: Value = serde_json::from_str(&manifest_json).unwrap();
            let installed = InstalledAutomationApp {
                app_id: app_id.into(),
                slug: manifest["slug"].as_str().unwrap_or("").into(),
                name: manifest["name"].as_str().unwrap_or("").into(),
                description: manifest["description"].as_str().unwrap_or("").into(),
                category: manifest["category"].as_str().unwrap_or("").into(),
                target_domains: serde_json::from_value(manifest["target_domains"].clone())
                    .unwrap_or_default(),
                installed_version: version.clone(),
                latest_version: version,
                manifest,
                content_sha256: sha,
                installed_at_ms: inst_at,
                last_run_at_ms: None,
                local_run_count: 0,
            };
            (
                200,
                serde_json::to_string(&InstallAutomationAppResponse {
                    ok: true,
                    app: installed,
                    install_count: new_count as u64,
                    newly_installed: true,
                })
                .unwrap(),
            )
        }
        _ if method == "DELETE"
            && path.starts_with("/api/v1/automation-apps/")
            && path.ends_with("/install") =>
        {
            let Some(u) = uid else {
                return (
                    401,
                    json!({ "error": "missing_token", "message": "auth required" }).to_string(),
                );
            };
            let app_id = path
                .trim_start_matches("/api/v1/automation-apps/")
                .trim_end_matches("/install");
            let deleted = conn
                .execute(
                    "DELETE FROM installs WHERE user_id = ?1 AND app_id = ?2",
                    params![u as i64, app_id],
                )
                .unwrap();
            (
                200,
                serde_json::to_string(&UninstallAutomationAppResponse {
                    ok: true,
                    app_id: app_id.into(),
                    removed: deleted > 0,
                })
                .unwrap(),
            )
        }
        _ if method == "POST"
            && path.starts_with("/api/v1/automation-apps/")
            && path.ends_with("/runs") =>
        {
            let Some(u) = uid else {
                return (
                    401,
                    json!({ "error": "missing_token", "message": "auth required" }).to_string(),
                );
            };
            let app_id = path
                .trim_start_matches("/api/v1/automation-apps/")
                .trim_end_matches("/runs");
            let _req: RecordAutomationRunRequest = serde_json::from_str(body).unwrap();
            let now_ms = 1_700_000_020_000i64;
            conn.execute(
                "UPDATE apps SET run_count = run_count + 1 WHERE app_id = ?1",
                params![app_id],
            )
            .unwrap();
            conn.execute(
                "UPDATE installs SET last_run_at_ms = ?1 WHERE user_id = ?2 AND app_id = ?3",
                params![now_ms, u as i64, app_id],
            )
            .unwrap();
            let run_count: i64 = conn
                .query_row(
                    "SELECT run_count FROM apps WHERE app_id = ?1",
                    params![app_id],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            (
                200,
                serde_json::to_string(&RecordAutomationRunResponse {
                    ok: true,
                    app_id: app_id.into(),
                    run_count: run_count as u64,
                    last_run_at_ms: now_ms,
                })
                .unwrap(),
            )
        }
        _ if method == "GET" && path.starts_with("/api/v1/automation-apps/") => {
            let app_id = path.trim_start_matches("/api/v1/automation-apps/");
            let row: Option<(i64, String, String, String, i64, i64)> = conn
                .query_row(
                    "SELECT author_user_id, version, manifest_json, content_sha256, install_count, run_count
                     FROM apps WHERE app_id = ?1",
                    params![app_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .optional()
                .unwrap();
            let Some((author_id, version, manifest_json, sha, installs, runs)) = row else {
                return (
                    404,
                    json!({ "error": "not_found", "message": "Automation app not found" })
                        .to_string(),
                );
            };
            let manifest: Value = serde_json::from_str(&manifest_json).unwrap();
            let detail = AutomationAppDetail {
                app_id: app_id.into(),
                slug: manifest["slug"].as_str().unwrap_or("").into(),
                author_user_id: author_id as u64,
                author_username: Some("alice".into()),
                name: manifest["name"].as_str().unwrap_or("").into(),
                description: manifest["description"].as_str().unwrap_or("").into(),
                category: manifest["category"].as_str().unwrap_or("").into(),
                target_domains: serde_json::from_value(manifest["target_domains"].clone())
                    .unwrap_or_default(),
                latest_version: version.clone(),
                versions: vec![version],
                manifest,
                content_sha256: sha,
                install_count: installs as u64,
                run_count: runs as u64,
                is_published: true,
                is_installed: false,
                installed_version: None,
                created_at_ms: 1_700_000_000_000,
                updated_at_ms: 1_700_000_000_000,
            };
            (200, serde_json::to_string(&detail).unwrap())
        }
        _ => (
            404,
            json!({ "error": "not_found", "message": "route not found" }).to_string(),
        ),
    }
}

fn make_auth_session(user_id: u64, username: &str, token: &str) -> AuthResponse {
    AuthResponse {
        user_id,
        username: username.to_string(),
        session_id: format!("sess_{username}"),
        token: token.to_string(),
        first_name: Some(username.to_string()),
        created_at: None,
        expires_at: None,
    }
}

#[tokio::test]
async fn test_automation_apps_http_error_mapping_and_query_encoding() {
    let server_dir = tempfile::tempdir().unwrap();
    let srv = PersistentMockStoreServer::start(server_dir.path().join("mock_store.db")).await;

    let client_dir = tempfile::tempdir().unwrap();
    let cfg = ClientConfig {
        server_url: srv.base_url.clone(),
        ..Default::default()
    };
    let client = TelegramClient::open_with_dir(cfg, client_dir.path()).unwrap();

    // Missing token -> Unauthorized locally before hitting wire
    let unauth_err = client
        .install_automation_app("any-app")
        .await
        .expect_err("must require token");
    assert!(matches!(unauth_err, HttpClientError::Unauthorized(_)));

    // Anonymous search with special characters -> RFC 3986 encoding & no Authorization header
    let search_res = client
        .search_automation_apps(
            Some("post & reply?"),
            Some("social/media"),
            Some("x.com"),
            Some(15),
            Some(5),
        )
        .await
        .unwrap();
    assert_eq!(search_res.limit, 15);
    assert_eq!(search_res.offset, 5);

    // Status code mapping checks (400, 403, 404, 409, 422)
    client.set_token(Some("tok_alice".into()));

    let err_400 = client.get_automation_app("force-400").await.unwrap_err();
    assert!(matches!(err_400, HttpClientError::Api { status: 400, .. }));
    assert!(matches!(TelegramError::from(err_400), TelegramError::Validation(_)));

    let err_403 = client.get_automation_app("force-403").await.unwrap_err();
    assert!(matches!(err_403, HttpClientError::Api { status: 403, .. }));
    assert!(matches!(
        TelegramError::from(err_403),
        TelegramError::PermissionDenied(_)
    ));

    let err_404 = client.get_automation_app("nonexistent-app").await.unwrap_err();
    assert!(matches!(err_404, HttpClientError::Api { status: 404, .. }));
    assert!(matches!(TelegramError::from(err_404), TelegramError::NotFound(_)));

    let err_409 = client.get_automation_app("force-409").await.unwrap_err();
    assert!(matches!(err_409, HttpClientError::Conflict(_)));
    assert!(matches!(TelegramError::from(err_409), TelegramError::Validation(_)));

    let err_422 = client.get_automation_app("force-422").await.unwrap_err();
    match &err_422 {
        HttpClientError::Api { status, message } => {
            assert_eq!(*status, 422);
            assert!(message.contains("auth_token"));
        }
        other => panic!("expected 422 Api error, got {other:?}"),
    }
    assert!(matches!(TelegramError::from(err_422), TelegramError::Validation(_)));

    let reqs = srv.captured.lock().unwrap().clone();
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(
        reqs[0].target,
        "/api/v1/automation-apps?q=post%20%26%20reply%3F&category=social%2Fmedia&domain=x.com&limit=15&offset=5"
    );
    assert!(reqs[0].auth_header.is_none());
    assert_eq!(reqs[1].auth_header.as_deref(), Some("Bearer tok_alice"));

    srv.task.abort();
    tokio::task::spawn_blocking(move || drop(client))
        .await
        .unwrap();
}

#[test]
fn test_automation_apps_non_tokio_thread_safe() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server_dir = tempfile::tempdir().unwrap();
    let srv = rt.block_on(PersistentMockStoreServer::start(
        server_dir.path().join("mock_store.db"),
    ));

    let client_dir = tempfile::tempdir().unwrap();
    let cfg = ClientConfig {
        server_url: srv.base_url.clone(),
        ..Default::default()
    };

    // Execute entirely on a plain non-Tokio OS thread via futures::executor::block_on
    let handle = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        let client = TelegramClient::open_with_dir(cfg, client_dir.path()).unwrap();
        let res = futures::executor::block_on(client.search_automation_apps(
            Some("hello"),
            None,
            None,
            Some(10),
            Some(0),
        ))
        .unwrap();
        assert_eq!(res.limit, 10);
        drop(client);
    });
    handle.join().unwrap();
    srv.task.abort();
}

#[tokio::test]
async fn test_cross_client_publish_install_offline_persistence_and_account_isolation() {
    let server_dir = tempfile::tempdir().unwrap();
    let srv = PersistentMockStoreServer::start(server_dir.path().join("mock_store.db")).await;

    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    let cfg = ClientConfig {
        server_url: srv.base_url.clone(),
        ..Default::default()
    };

    // 1. Client A (Alice) publishes a sanitized automation script app
    let client_a = TelegramClient::open_with_dir(cfg.clone(), alice_dir.path()).unwrap();
    client_a.set_auth_session(make_auth_session(1001, "alice", "tok_alice"));
    let manifest = json!({
        "schema_version": 1,
        "app_id": "x-bookmark-export",
        "slug": "x-bookmark-export",
        "name": "X Bookmark Exporter",
        "version": "1.0.0",
        "description": "Exports bookmarks",
        "category": "social",
        "target_domains": ["x.com"],
        "parameters": [
            { "name": "max_items", "description": "Limit", "required": false, "default_value": "20", "is_secret": false }
        ],
        "steps": [
            { "kind": "navigate", "url_template": "https://x.com/i/bookmarks" },
            { "kind": "scroll", "delta_x": 0.0, "delta_y": 500.0 }
        ],
        "required_capabilities": ["browser"],
        "author_id": "1001"
    });
    let published = client_a
        .publish_automation_app(&PublishAutomationAppRequest {
            manifest: manifest.clone(),
            is_published: true,
            changelog: Some("v1.0.0".into()),
        })
        .await
        .unwrap();
    assert_eq!(published.app_id, "x-bookmark-export");
    assert_eq!(published.author_user_id, 1001);

    // 2. Client B (Bob) discovers, fetches detail, installs into LocalStore, records run
    let client_b = TelegramClient::open_with_dir(cfg.clone(), bob_dir.path()).unwrap();
    client_b.set_auth_session(make_auth_session(2002, "bob", "tok_bob"));
    let catalog = client_b
        .search_automation_apps(Some("Bookmark"), Some("social"), Some("x.com"), None, None)
        .await
        .unwrap();
    assert_eq!(catalog.total, 1);
    assert_eq!(catalog.apps[0].app_id, "x-bookmark-export");

    let fetched = client_b
        .get_automation_app("x-bookmark-export")
        .await
        .unwrap();
    assert_eq!(fetched.manifest, manifest);
    assert!(client_b
        .get_cached_automation_app("x-bookmark-export")
        .is_some());

    let inst = client_b
        .install_automation_app("x-bookmark-export")
        .await
        .unwrap();
    assert_eq!(inst.install_count, 1);
    assert_eq!(
        client_b
            .get_installed_automation_app_cached("x-bookmark-export")
            .unwrap()
            .manifest,
        manifest
    );

    let run_resp = client_b
        .record_automation_app_run(
            "x-bookmark-export",
            &RecordAutomationRunRequest {
                version: Some("1.0.0".into()),
                success: true,
                steps_executed: 2,
                total_duration_ms: 350,
                used_llm: false,
                error: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(run_resp.run_count, 1);

    // 3. Drop Client B via spawn_blocking, shut down server, and verify offline LocalStore hydration!
    tokio::task::spawn_blocking(move || drop(client_b))
        .await
        .unwrap();
    srv.task.abort();

    let client_b_offline = TelegramClient::open_with_dir(cfg.clone(), bob_dir.path()).unwrap();
    client_b_offline.set_auth_session(make_auth_session(2002, "bob", "tok_bob"));
    let offline_apps = client_b_offline.list_installed_automation_apps_cached();
    assert_eq!(offline_apps.len(), 1);
    assert_eq!(offline_apps[0].app_id, "x-bookmark-export");
    assert_eq!(offline_apps[0].manifest, manifest);
    assert_eq!(offline_apps[0].last_run_at_ms, Some(1_700_000_020_000));
    assert_eq!(offline_apps[0].local_run_count, 1);
    assert!(client_b_offline
        .get_cached_automation_app("x-bookmark-export")
        .is_some());

    // 4. Restart server on the same DB and verify online sync + uninstall
    let srv2 = PersistentMockStoreServer::start(server_dir.path().join("mock_store.db")).await;
    let cfg2 = ClientConfig {
        server_url: srv2.base_url.clone(),
        ..Default::default()
    };
    tokio::task::spawn_blocking(move || drop(client_b_offline))
        .await
        .unwrap();

    let client_b_online = TelegramClient::open_with_dir(cfg2.clone(), bob_dir.path()).unwrap();
    client_b_online.set_auth_session(make_auth_session(2002, "bob", "tok_bob"));
    let synced = client_b_online.list_installed_automation_apps().await.unwrap();
    assert_eq!(synced.total, 1);
    assert_eq!(synced.apps[0].app_id, "x-bookmark-export");

    // 5. Account switch isolation: switching to Charlie on Bob's cache dir wipes installed & cached apps
    tokio::task::spawn_blocking(move || drop(client_b_online))
        .await
        .unwrap();
    let client_charlie = TelegramClient::open_with_dir(cfg2, bob_dir.path()).unwrap();
    client_charlie.set_auth_session(make_auth_session(3003, "charlie", "tok_charlie"));
    assert!(client_charlie
        .list_installed_automation_apps_cached()
        .is_empty());
    assert!(client_charlie
        .get_cached_automation_app("x-bookmark-export")
        .is_none());

    srv2.task.abort();
    tokio::task::spawn_blocking(move || drop((client_a, client_charlie)))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_client_app_id_normalization_and_route_collision_guards() {
    let server_dir = tempfile::tempdir().unwrap();
    let srv = PersistentMockStoreServer::start(server_dir.path().join("mock_store.db")).await;
    let client_dir = tempfile::tempdir().unwrap();
    let cfg = ClientConfig {
        server_url: srv.base_url.clone(),
        ..Default::default()
    };
    let client = TelegramClient::open_with_dir(cfg, client_dir.path()).unwrap();
    client.set_auth_session(make_auth_session(1001, "alice", "tok_alice"));

    // Publish and install "trim-app"
    let manifest = json!({
        "schema_version": 1,
        "app_id": "trim-app",
        "slug": "trim-app",
        "name": "Trim App",
        "version": "1.0.0",
        "description": "Tests whitespace normalization",
        "category": "general",
        "target_domains": ["example.com"],
        "parameters": [],
        "steps": [{ "kind": "navigate", "url_template": "https://example.com" }]
    });
    client
        .publish_automation_app(&PublishAutomationAppRequest {
            manifest,
            is_published: true,
            changelog: None,
        })
        .await
        .unwrap();
    client.install_automation_app("  trim-app  ").await.unwrap();
    assert!(client
        .get_installed_automation_app_cached("  trim-app  ")
        .is_some());
    assert!(client.get_cached_automation_app("  trim-app  ").is_some());

    // Fix 5: record_automation_app_run and uninstall_automation_app with whitespace-padded app_id
    let run_resp = client
        .record_automation_app_run(
            "  trim-app  ",
            &RecordAutomationRunRequest {
                version: Some("1.0.0".into()),
                success: true,
                steps_executed: 1,
                total_duration_ms: 42,
                used_llm: false,
                error: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(run_resp.run_count, 1);
    let after_run = client
        .get_installed_automation_app_cached("trim-app")
        .unwrap();
    assert_eq!(after_run.local_run_count, 1);
    assert_eq!(after_run.last_run_at_ms, Some(run_resp.last_run_at_ms));

    let uninst = client
        .uninstall_automation_app("  trim-app  ")
        .await
        .unwrap();
    assert!(uninst.removed);
    assert!(
        client
            .get_installed_automation_app_cached("trim-app")
            .is_none(),
        "uninstall with whitespace-padded app_id must remove LocalStore entry"
    );

    // Fix 6: reject invalid/reserved app_ids locally before sending HTTP requests
    let captured_before = srv.captured.lock().unwrap().len();
    let long_id = "a".repeat(129);
    for bad_id in [
        "",
        "   ",
        ".",
        "..",
        "%2e%2e",
        "foo.",
        "foo bar",
        long_id.as_str(),
        "installed",
        "INSTALLED",
        "  InStAlLeD  ",
        "../users",
        "a/b",
        "a\\b",
        "a?q=1",
        "a#frag",
    ] {
        let err_get = client.get_automation_app(bad_id).await.unwrap_err();
        assert!(
            matches!(err_get, HttpClientError::Api { status: 400, .. }),
            "expected 400 for get_automation_app({bad_id:?}), got {err_get:?}"
        );
        let err_inst = client.install_automation_app(bad_id).await.unwrap_err();
        assert!(
            matches!(err_inst, HttpClientError::Api { status: 400, .. }),
            "expected 400 for install_automation_app({bad_id:?}), got {err_inst:?}"
        );
        let err_uninst = client.uninstall_automation_app(bad_id).await.unwrap_err();
        assert!(
            matches!(err_uninst, HttpClientError::Api { status: 400, .. }),
            "expected 400 for uninstall_automation_app({bad_id:?}), got {err_uninst:?}"
        );
        let err_run = client
            .record_automation_app_run(bad_id, &RecordAutomationRunRequest::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err_run, HttpClientError::Api { status: 400, .. }),
            "expected 400 for record_automation_app_run({bad_id:?}), got {err_run:?}"
        );
    }
    let captured_after = srv.captured.lock().unwrap().len();
    assert_eq!(
        captured_before, captured_after,
        "invalid/reserved app_id calls must not send HTTP requests"
    );
    assert!(client.get_cached_automation_app("").is_none());
    assert!(client.get_cached_automation_app("installed").is_none());

    // Even if server returns an empty app_id payload, LocalStore must NOT be polluted under key ""
    let empty_detail = client.get_automation_app("empty-detail").await.unwrap();
    assert!(empty_detail.app_id.is_empty());
    assert!(client.get_cached_automation_app("").is_none());

    srv.task.abort();
    tokio::task::spawn_blocking(move || drop(client))
        .await
        .unwrap();
}
