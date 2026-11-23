use crate::net::http_client::{HttpClient, HttpClientError};
use crate::net::http_helpers::percent_encode_rfc3986;
use gpui_telegram_proto::user_vault::{
    ApproveDeviceSyncRequest, ArchiveLocalBotReplyRequest, ArchiveLocalBotReplyResponse,
    DeviceSyncActionResponse, DeviceSyncRequestBody, DeviceSyncRequestResponse,
    DeviceSyncStatusResponse, PendingDeviceSyncListResponse, UpsertUserVaultItemRequest,
    UpsertUserVaultItemResponse, UserVaultEnvelopeResponse, UserVaultKeyEnvelopeDto,
    UserVaultSyncPullResponse,
};
use reqwest::Method;

impl HttpClient {
    pub async fn get_user_vault_envelope(
        &self,
        token: &str,
    ) -> Result<UserVaultEnvelopeResponse, HttpClientError> {
        self.request_json(
            Method::GET,
            "/api/v1/user-vault/envelope",
            Some(token),
            None::<&()>,
        )
        .await
    }

    pub async fn put_user_vault_envelope(
        &self,
        token: &str,
        envelope: &UserVaultKeyEnvelopeDto,
    ) -> Result<UserVaultEnvelopeResponse, HttpClientError> {
        self.request_json(
            Method::PUT,
            "/api/v1/user-vault/envelope",
            Some(token),
            Some(envelope),
        )
        .await
    }

    pub async fn pull_user_vault_items(
        &self,
        token: &str,
        since_revision: Option<i64>,
        category: Option<&str>,
    ) -> Result<UserVaultSyncPullResponse, HttpClientError> {
        let mut params = Vec::new();
        if let Some(rev) = since_revision {
            params.push(format!("since_revision={rev}"));
        }
        if let Some(cat) = category.map(str::trim).filter(|s| !s.is_empty()) {
            params.push(format!("category={}", percent_encode_rfc3986(cat)));
        }
        let path = if params.is_empty() {
            "/api/v1/user-vault/items".to_string()
        } else {
            format!("/api/v1/user-vault/items?{}", params.join("&"))
        };
        self.request_json(Method::GET, &path, Some(token), None::<&()>)
            .await
    }

    pub async fn put_user_vault_item(
        &self,
        token: &str,
        category: &str,
        item_key: &str,
        req: &UpsertUserVaultItemRequest,
    ) -> Result<UpsertUserVaultItemResponse, HttpClientError> {
        let path = format!(
            "/api/v1/user-vault/items/{}/{}",
            percent_encode_rfc3986(category.trim()),
            percent_encode_rfc3986(item_key.trim())
        );
        self.request_json(Method::PUT, &path, Some(token), Some(req))
            .await
    }

    pub async fn delete_user_vault_item(
        &self,
        token: &str,
        category: &str,
        item_key: &str,
        expected_revision: Option<i64>,
    ) -> Result<UpsertUserVaultItemResponse, HttpClientError> {
        let mut path = format!(
            "/api/v1/user-vault/items/{}/{}",
            percent_encode_rfc3986(category.trim()),
            percent_encode_rfc3986(item_key.trim())
        );
        if let Some(rev) = expected_revision {
            path.push_str(&format!("?expected_revision={rev}"));
        }
        self.request_json(Method::DELETE, &path, Some(token), None::<&()>)
            .await
    }

    pub async fn archive_local_bot_reply(
        &self,
        token: &str,
        chat_id: i64,
        req: &ArchiveLocalBotReplyRequest,
    ) -> Result<ArchiveLocalBotReplyResponse, HttpClientError> {
        let path = format!("/api/v1/chats/{}/messages/archive-local-bot-reply", chat_id);
        self.request_json(Method::POST, &path, Some(token), Some(req))
            .await
    }

    pub async fn device_sync_request(
        &self,
        token: &str,
        req: &DeviceSyncRequestBody,
    ) -> Result<DeviceSyncRequestResponse, HttpClientError> {
        self.request_json(
            Method::POST,
            "/api/v1/user-vault/device-sync/request",
            Some(token),
            Some(req),
        )
        .await
    }

    pub async fn device_sync_pending(
        &self,
        token: &str,
    ) -> Result<PendingDeviceSyncListResponse, HttpClientError> {
        self.request_json(
            Method::GET,
            "/api/v1/user-vault/device-sync/pending",
            Some(token),
            None::<&()>,
        )
        .await
    }

    pub async fn device_sync_approve(
        &self,
        token: &str,
        request_id: &str,
        req: &ApproveDeviceSyncRequest,
    ) -> Result<DeviceSyncActionResponse, HttpClientError> {
        let path = format!(
            "/api/v1/user-vault/device-sync/{}/approve",
            percent_encode_rfc3986(request_id.trim())
        );
        self.request_json(Method::POST, &path, Some(token), Some(req))
            .await
    }

    pub async fn device_sync_deny(
        &self,
        token: &str,
        request_id: &str,
    ) -> Result<DeviceSyncActionResponse, HttpClientError> {
        let path = format!(
            "/api/v1/user-vault/device-sync/{}/deny",
            percent_encode_rfc3986(request_id.trim())
        );
        self.request_json(Method::POST, &path, Some(token), None::<&()>)
            .await
    }

    pub async fn device_sync_status(
        &self,
        token: &str,
        request_id: &str,
    ) -> Result<DeviceSyncStatusResponse, HttpClientError> {
        let path = format!(
            "/api/v1/user-vault/device-sync/{}/status",
            percent_encode_rfc3986(request_id.trim())
        );
        self.request_json(Method::GET, &path, Some(token), None::<&()>)
            .await
    }
}
