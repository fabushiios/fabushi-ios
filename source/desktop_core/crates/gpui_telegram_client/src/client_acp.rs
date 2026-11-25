use crate::client::TelegramClient;
use crate::net::HttpClientError;

impl TelegramClient {
    pub async fn list_tasks_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<serde_json::Value>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_tasks(token).await
    }

    pub fn list_tasks(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<serde_json::Value>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_tasks_with_token(&token, expected_epoch).await
        }
    }

    pub async fn create_task_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        payload: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.create_task(token, payload).await
    }

    pub fn create_task<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.create_task_with_token(&token, expected_epoch, payload)
                .await
        }
    }

    pub async fn approve_task_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        task_id: &str,
        approved: bool,
    ) -> std::result::Result<serde_json::Value, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.approve_task(token, task_id, approved).await
    }

    pub fn approve_task<'a>(
        &'a self,
        task_id: &'a str,
        approved: bool,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.approve_task_with_token(&token, expected_epoch, task_id, approved)
                .await
        }
    }

    pub async fn list_mcp_servers_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<serde_json::Value>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_mcp_servers(token).await
    }

    pub fn list_mcp_servers(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<serde_json::Value>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_mcp_servers_with_token(&token, expected_epoch)
                .await
        }
    }

    pub fn register_mcp_server<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.register_mcp_server(&token, payload).await
        }
    }

    pub async fn list_mcp_tools_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<serde_json::Value>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_mcp_tools(token).await
    }

    pub fn list_mcp_tools(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<serde_json::Value>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_mcp_tools_with_token(&token, expected_epoch).await
        }
    }

    pub async fn call_mcp_tool_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        payload: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.call_mcp_tool(token, payload).await
    }

    pub fn call_mcp_tool<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.call_mcp_tool_with_token(&token, expected_epoch, payload)
                .await
        }
    }

    pub async fn approve_mcp_permission_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        payload: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.approve_mcp_permission(token, payload).await
    }

    pub fn approve_mcp_permission<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.approve_mcp_permission_with_token(&token, expected_epoch, payload)
                .await
        }
    }

    pub fn acquire_computer_lease<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.acquire_computer_lease(&token, payload).await
        }
    }

    pub fn execute_computer_action<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .execute_computer_action(&token, payload)
                .await
        }
    }

    pub fn create_talk_session<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.create_talk_session(&token, payload).await
        }
    }

    pub fn talk_barge_in<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.talk_barge_in(&token, payload).await
        }
    }

    pub fn enqueue_talk_audio<'a>(
        &'a self,
        payload: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.enqueue_talk_audio(&token, payload).await
        }
    }
}
