use crate::client::TelegramClient;
use crate::net::http_client::{HttpClient, HttpClientError};
use crate::net::http_helpers::percent_encode_rfc3986;
use gpui_telegram_proto::automation_apps::{
    AutomationAppDetail, InstallAutomationAppResponse, InstalledAutomationApp,
    ListAutomationAppsResponse, ListInstalledAutomationAppsResponse, PublishAutomationAppRequest,
    RecordAutomationRunRequest, RecordAutomationRunResponse, UninstallAutomationAppResponse,
};
use reqwest::Method;

const MAX_CLIENT_APP_ID_LEN: usize = 128;

fn validate_client_app_id(raw_app_id: &str) -> Result<&str, HttpClientError> {
    let app_id = raw_app_id.trim();
    if app_id.is_empty()
        || app_id.len() > MAX_CLIENT_APP_ID_LEN
        || app_id.eq_ignore_ascii_case("installed")
        || !app_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(HttpClientError::Api {
            status: 400,
            message: "app_id must be 1..=128 characters of [a-zA-Z0-9_-] and cannot be 'installed'".to_string(),
        });
    }
    Ok(app_id)
}

impl HttpClient {
    pub async fn publish_automation_app(
        &self,
        token: &str,
        req: &PublishAutomationAppRequest,
    ) -> Result<AutomationAppDetail, HttpClientError> {
        self.request_json(
            Method::POST,
            "/api/v1/automation-apps",
            Some(token),
            Some(req),
        )
        .await
    }

    pub async fn search_automation_apps(
        &self,
        token: Option<&str>,
        query: Option<&str>,
        category: Option<&str>,
        domain: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<ListAutomationAppsResponse, HttpClientError> {
        let mut params = Vec::new();
        if let Some(q) = query.map(str::trim).filter(|s| !s.is_empty()) {
            params.push(format!("q={}", percent_encode_rfc3986(q)));
        }
        if let Some(cat) = category.map(str::trim).filter(|s| !s.is_empty()) {
            params.push(format!("category={}", percent_encode_rfc3986(cat)));
        }
        if let Some(dom) = domain.map(str::trim).filter(|s| !s.is_empty()) {
            params.push(format!("domain={}", percent_encode_rfc3986(dom)));
        }
        if let Some(lim) = limit {
            params.push(format!("limit={lim}"));
        }
        if let Some(off) = offset {
            params.push(format!("offset={off}"));
        }
        let path = if params.is_empty() {
            "/api/v1/automation-apps".to_string()
        } else {
            format!("/api/v1/automation-apps?{}", params.join("&"))
        };
        self.request_json(Method::GET, &path, token, None::<&()>)
            .await
    }

    pub async fn list_installed_automation_apps(
        &self,
        token: &str,
    ) -> Result<ListInstalledAutomationAppsResponse, HttpClientError> {
        self.request_json(
            Method::GET,
            "/api/v1/automation-apps/installed",
            Some(token),
            None::<&()>,
        )
        .await
    }

    pub async fn get_automation_app(
        &self,
        token: Option<&str>,
        app_id: &str,
    ) -> Result<AutomationAppDetail, HttpClientError> {
        let app_id = validate_client_app_id(app_id)?;
        let path = format!("/api/v1/automation-apps/{}", percent_encode_rfc3986(app_id));
        self.request_json(Method::GET, &path, token, None::<&()>)
            .await
    }

    pub async fn install_automation_app(
        &self,
        token: &str,
        app_id: &str,
    ) -> Result<InstallAutomationAppResponse, HttpClientError> {
        let app_id = validate_client_app_id(app_id)?;
        let path = format!(
            "/api/v1/automation-apps/{}/install",
            percent_encode_rfc3986(app_id)
        );
        self.request_json(Method::POST, &path, Some(token), None::<&()>)
            .await
    }

    pub async fn uninstall_automation_app(
        &self,
        token: &str,
        app_id: &str,
    ) -> Result<UninstallAutomationAppResponse, HttpClientError> {
        let app_id = validate_client_app_id(app_id)?;
        let path = format!(
            "/api/v1/automation-apps/{}/install",
            percent_encode_rfc3986(app_id)
        );
        self.request_json(Method::DELETE, &path, Some(token), None::<&()>)
            .await
    }

    pub async fn record_automation_app_run(
        &self,
        token: &str,
        app_id: &str,
        req: &RecordAutomationRunRequest,
    ) -> Result<RecordAutomationRunResponse, HttpClientError> {
        let app_id = validate_client_app_id(app_id)?;
        let path = format!(
            "/api/v1/automation-apps/{}/runs",
            percent_encode_rfc3986(app_id)
        );
        self.request_json(Method::POST, &path, Some(token), Some(req))
            .await
    }
}

impl TelegramClient {
    pub async fn publish_automation_app(
        &self,
        req: &PublishAutomationAppRequest,
    ) -> Result<AutomationAppDetail, HttpClientError> {
        let token = self.require_token()?;
        let detail = self
            .http_client
            .publish_automation_app(&token, req)
            .await?;
        self.local_store
            .upsert_cached_automation_app(detail.clone());
        Ok(detail)
    }

    pub async fn search_automation_apps(
        &self,
        query: Option<&str>,
        category: Option<&str>,
        domain: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<ListAutomationAppsResponse, HttpClientError> {
        let tok = self.token();
        self.http_client
            .search_automation_apps(tok.as_deref(), query, category, domain, limit, offset)
            .await
    }

    pub async fn get_automation_app(
        &self,
        app_id: &str,
    ) -> Result<AutomationAppDetail, HttpClientError> {
        let app_id = validate_client_app_id(app_id)?;
        let tok = self.token();
        let detail = self
            .http_client
            .get_automation_app(tok.as_deref(), app_id)
            .await?;
        if !detail.app_id.trim().is_empty() {
            self.local_store
                .upsert_cached_automation_app(detail.clone());
        }
        Ok(detail)
    }

    pub async fn install_automation_app(
        &self,
        app_id: &str,
    ) -> Result<InstallAutomationAppResponse, HttpClientError> {
        let app_id = validate_client_app_id(app_id)?;
        let token = self.require_token()?;
        let resp = self
            .http_client
            .install_automation_app(&token, app_id)
            .await?;
        if !resp.app.app_id.trim().is_empty() {
            self.local_store
                .upsert_installed_automation_app(resp.app.clone());
        }
        Ok(resp)
    }

    pub async fn uninstall_automation_app(
        &self,
        app_id: &str,
    ) -> Result<UninstallAutomationAppResponse, HttpClientError> {
        let app_id = validate_client_app_id(app_id)?;
        let token = self.require_token()?;
        let resp = self
            .http_client
            .uninstall_automation_app(&token, app_id)
            .await?;
        let key = if resp.app_id.trim().is_empty() {
            app_id
        } else {
            resp.app_id.trim()
        };
        self.local_store.remove_installed_automation_app(key);
        Ok(resp)
    }

    pub async fn list_installed_automation_apps(
        &self,
    ) -> Result<ListInstalledAutomationAppsResponse, HttpClientError> {
        let token = self.require_token()?;
        let resp = self
            .http_client
            .list_installed_automation_apps(&token)
            .await?;
        self.local_store
            .replace_installed_automation_apps(resp.apps.clone());
        Ok(resp)
    }

    pub async fn record_automation_app_run(
        &self,
        app_id: &str,
        req: &RecordAutomationRunRequest,
    ) -> Result<RecordAutomationRunResponse, HttpClientError> {
        let app_id = validate_client_app_id(app_id)?;
        let token = self.require_token()?;
        let resp = self
            .http_client
            .record_automation_app_run(&token, app_id, req)
            .await?;
        let key = if resp.app_id.trim().is_empty() {
            app_id
        } else {
            resp.app_id.trim()
        };
        self.local_store
            .mark_automation_app_run(key, resp.last_run_at_ms);
        Ok(resp)
    }

    pub fn list_installed_automation_apps_cached(&self) -> Vec<InstalledAutomationApp> {
        self.local_store.list_installed_automation_apps()
    }

    pub fn get_installed_automation_app_cached(
        &self,
        app_id: &str,
    ) -> Option<InstalledAutomationApp> {
        self.local_store
            .get_installed_automation_app(app_id.trim())
    }

    pub fn get_cached_automation_app(&self, app_id: &str) -> Option<AutomationAppDetail> {
        self.local_store.get_cached_automation_app(app_id.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClientConfig;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Debug, Clone)]
    struct RecordedReq {
        method: String,
        target: String,
        auth: Option<String>,
        _body: String,
    }

    async fn spawn_one_shot_mock_server(
        responses: Vec<(u16, String)>,
    ) -> (String, Arc<Mutex<Vec<RecordedReq>>>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let rec_clone = Arc::clone(&recorded);
        let handle = tokio::spawn(async move {
            for (status_code, resp_body) in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 65536];
                let n = sock.read(&mut buf).await.unwrap();
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
                let mut lines = head.lines();
                let req_line = lines.next().unwrap_or("");
                let mut parts = req_line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let auth = lines.find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.eq_ignore_ascii_case("authorization") {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                });
                rec_clone.lock().unwrap().push(RecordedReq {
                    method,
                    target,
                    auth,
                    _body: body.to_string(),
                });
                let reason = match status_code {
                    200 => "OK",
                    201 => "Created",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    409 => "Conflict",
                    422 => "Unprocessable Entity",
                    _ => "Error",
                };
                let http_resp = format!(
                    "HTTP/1.1 {status_code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                sock.write_all(http_resp.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{addr}"), recorded, handle)
    }

    #[tokio::test]
    async fn test_api_automation_apps_query_encoding_and_write_through() {
        let sample_manifest = json!({
            "schema_version": 1,
            "app_id": "x-post",
            "slug": "x-post",
            "name": "X Poster",
            "version": "1.0.0",
            "description": "Posts on X",
            "category": "social",
            "target_domains": ["x.com"],
            "parameters": [],
            "steps": [{"kind": "navigate", "url_template": "https://x.com"}]
        });
        let list_json = serde_json::to_string(&ListAutomationAppsResponse {
            apps: vec![],
            total: 0,
            limit: 20,
            offset: 5,
        })
        .unwrap();
        let installed_app = InstalledAutomationApp {
            app_id: "x-post".into(),
            slug: "x-post".into(),
            name: "X Poster".into(),
            description: "Posts on X".into(),
            category: "social".into(),
            target_domains: vec!["x.com".into()],
            installed_version: "1.0.0".into(),
            latest_version: "1.0.0".into(),
            manifest: sample_manifest,
            content_sha256: "abc123".into(),
            installed_at_ms: 1_700_000_000_000,
            last_run_at_ms: None,
            local_run_count: 0,
        };
        let install_json = serde_json::to_string(&InstallAutomationAppResponse {
            ok: true,
            app: installed_app,
            install_count: 1,
            newly_installed: true,
        })
        .unwrap();

        let (base_url, recorded, server_task) =
            spawn_one_shot_mock_server(vec![(200, list_json), (200, install_json)]).await;

        let dir = tempfile::tempdir().unwrap();
        let cfg = ClientConfig {
            server_url: base_url,
            ..Default::default()
        };
        let client = TelegramClient::open_with_dir(cfg, dir.path()).unwrap();

        // Anonymous catalog search (no Bearer token)
        let res = client
            .search_automation_apps(
                Some("x post & quote"),
                Some("social"),
                Some("x.com"),
                Some(20),
                Some(5),
            )
            .await
            .unwrap();
        assert_eq!(res.limit, 20);
        assert_eq!(res.offset, 5);

        // Authenticated install writes through to LocalStore
        client.set_token(Some("tok-123".into()));
        let inst = client.install_automation_app("x-post").await.unwrap();
        assert_eq!(inst.install_count, 1);
        assert!(client.get_installed_automation_app_cached("x-post").is_some());

        let reqs = recorded.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].method, "GET");
        assert_eq!(
            reqs[0].target,
            "/api/v1/automation-apps?q=x%20post%20%26%20quote&category=social&domain=x.com&limit=20&offset=5"
        );
        assert!(reqs[0].auth.is_none());
        assert_eq!(reqs[1].method, "POST");
        assert_eq!(reqs[1].target, "/api/v1/automation-apps/x-post/install");
        assert_eq!(reqs[1].auth.as_deref(), Some("Bearer tok-123"));

        server_task.abort();
        tokio::task::spawn_blocking(move || drop(client))
            .await
            .unwrap();
    }
}
