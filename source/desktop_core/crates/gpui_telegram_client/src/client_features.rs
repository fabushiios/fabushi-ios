use crate::cache::sqlite_cache::sha256_hex;
use crate::client::TelegramClient;
use crate::net::{
    CallInviteRequest, CallRecordDto, CallSignalRequest, ChannelStatisticsDto, GifResultDto,
    HttpClientError, MediaUploadCompleteResponse, SearchResponse, StarGiftRequest, StickerSetDto,
    StoryDto, WalletBalanceDto, WalletTransactionDto,
};
use gpui_telegram_proto::ws::WsClientFrame;
use std::path::PathBuf;

const MULTIPART_UPLOAD_THRESHOLD_BYTES: usize = 256 * 1024;

impl TelegramClient {
    pub fn local_media_path(&self, file_id: &str) -> Option<PathBuf> {
        self.local_store.media_cas_path(file_id)
    }

    pub fn upload_file_resumable<'a>(
        &'a self,
        file_path: impl AsRef<std::path::Path>,
        mime_type: &'a str,
        chat_id: Option<i64>,
        transfer_id: Option<&'a str>,
    ) -> impl std::future::Future<
        Output = gpui_telegram_core::error::Result<MediaUploadCompleteResponse>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        let path_buf = file_path.as_ref().to_path_buf();
        async move {
            let (expected_epoch, token) =
                pinned.map_err(|e| gpui_telegram_core::error::TelegramError::Auth(e.to_string()))?;
            self.ensure_pinned_session(expected_epoch, &token)
                .map_err(|e| gpui_telegram_core::error::TelegramError::Auth(e.to_string()))?;
            self.chunked_transfers
                .start_or_resume_upload(&path_buf, mime_type, chat_id, transfer_id)
                .await
        }
    }

    pub fn upload_media<'a>(
        &'a self,
        filename: &'a str,
        mime_type: &'a str,
        data: Vec<u8>,
    ) -> impl std::future::Future<Output = std::result::Result<MediaUploadCompleteResponse, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.upload_media_inner(&token, expected_epoch, filename, mime_type, data)
                .await
        }
    }

    pub async fn upload_media_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        filename: &str,
        mime_type: &str,
        data: Vec<u8>,
    ) -> std::result::Result<MediaUploadCompleteResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.upload_media_inner(token, expected_epoch, filename, mime_type, data)
            .await
    }

    async fn upload_media_inner(
        &self,
        token: &str,
        expected_epoch: u64,
        filename: &str,
        mime_type: &str,
        data: Vec<u8>,
    ) -> std::result::Result<MediaUploadCompleteResponse, HttpClientError> {
        let res = if data.len() <= MULTIPART_UPLOAD_THRESHOLD_BYTES {
            self.http_client
                .upload_media_multipart(token, filename, mime_type, data.clone())
                .await?
        } else {
            let file_id = self
                .http_client
                .upload_media(token, filename, &data)
                .await?;
            MediaUploadCompleteResponse {
                file_id,
                sha256: sha256_hex(&data),
                size_bytes: data.len() as u64,
                mime_type: mime_type.to_string(),
                filename: Some(filename.to_string()),
                width: None,
                height: None,
                duration_seconds: None,
                waveform_b64: None,
                waveform_bars: None,
                thumbnail_data_url: None,
            }
        };
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            let _ = self
                .local_store
                .put_media_cas(&res.file_id, &data, Some(&res.mime_type));
        }
        Ok(res)
    }

    pub fn get_media_bytes<'a>(
        &'a self,
        file_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<u8>, HttpClientError>> + Send + 'a
    {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_media_bytes_with_token(&token, expected_epoch, file_id)
                .await
        }
    }

    pub async fn get_media_bytes_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        file_id: &str,
    ) -> std::result::Result<Vec<u8>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        if let Some(cached) = self.local_store.get_media_cas(file_id) {
            return Ok(cached);
        }
        let bytes = self.http_client.download_media(token, file_id).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            let _ = self.local_store.put_media_cas(file_id, &bytes, None);
        }
        Ok(bytes)
    }

    pub fn download_media_range<'a>(
        &'a self,
        file_id: &'a str,
        start: u64,
        end_inclusive: Option<u64>,
    ) -> impl std::future::Future<
        Output = std::result::Result<(u16, Vec<u8>, Option<String>), HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.download_media_range_with_token(&token, expected_epoch, file_id, start, end_inclusive)
                .await
        }
    }

    pub async fn download_media_range_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        file_id: &str,
        start: u64,
        end_inclusive: Option<u64>,
    ) -> std::result::Result<(u16, Vec<u8>, Option<String>), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .download_media_range(token, file_id, start, end_inclusive)
            .await
    }

    pub fn list_sticker_sets<'a>(
        &'a self,
        query: Option<&'a str>,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<StickerSetDto>, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.list_sticker_sets(&token, query).await
        }
    }

    pub fn install_sticker_set<'a>(
        &'a self,
        set_id: &'a str,
        installed: bool,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .install_sticker_set(&token, set_id, installed)
                .await
        }
    }

    pub fn search_gifs<'a>(
        &'a self,
        query: &'a str,
        limit: usize,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<GifResultDto>, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.search_gifs(&token, query, limit).await
        }
    }

    pub fn list_stories(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<StoryDto>, HttpClientError>> + Send + '_
    {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.list_stories(&token).await
        }
    }

    pub fn create_story<'a>(
        &'a self,
        media_id: Option<&'a str>,
        media_type: &'a str,
        caption: Option<&'a str>,
        privacy: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<StoryDto, HttpClientError>> + Send + 'a
    {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .create_story(&token, media_id, media_type, caption, privacy)
                .await
        }
    }

    pub fn delete_story<'a>(
        &'a self,
        story_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.delete_story(&token, story_id).await
        }
    }

    pub fn list_calls(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<CallRecordDto>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_calls_with_token(&token, expected_epoch).await
        }
    }

    pub async fn list_calls_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<CallRecordDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_calls(token).await
    }

    pub fn call_invite<'a>(
        &'a self,
        req: &'a CallInviteRequest,
    ) -> impl std::future::Future<Output = std::result::Result<CallRecordDto, HttpClientError>> + Send + 'a
    {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.call_invite_with_token(&token, expected_epoch, req).await
        }
    }

    pub async fn call_invite_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &CallInviteRequest,
    ) -> std::result::Result<CallRecordDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.call_invite(token, req).await
    }

    pub fn call_accept<'a>(
        &'a self,
        call_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<CallRecordDto, HttpClientError>> + Send + 'a
    {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.call_accept_with_token(&token, expected_epoch, call_id)
                .await
        }
    }

    pub async fn call_accept_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        call_id: &str,
    ) -> std::result::Result<CallRecordDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.call_accept(token, call_id).await
    }

    pub fn call_decline<'a>(
        &'a self,
        call_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.call_decline_with_token(&token, expected_epoch, call_id)
                .await
        }
    }

    pub async fn call_decline_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        call_id: &str,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.call_decline(token, call_id).await
    }

    pub fn call_hangup<'a>(
        &'a self,
        call_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.call_hangup_with_token(&token, expected_epoch, call_id)
                .await
        }
    }

    pub async fn call_hangup_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        call_id: &str,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.call_hangup(token, call_id).await
    }

    pub fn rate_call<'a>(
        &'a self,
        call_id: &'a str,
        rating: u8,
    ) -> impl std::future::Future<Output = std::result::Result<CallRecordDto, HttpClientError>> + Send + 'a
    {
        self.rate_call_with_feedback(call_id, rating, None)
    }

    pub fn rate_call_with_feedback<'a>(
        &'a self,
        call_id: &'a str,
        rating: u8,
        feedback: Option<&'a str>,
    ) -> impl std::future::Future<Output = std::result::Result<CallRecordDto, HttpClientError>> + Send + 'a
    {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.rate_call_with_feedback_with_token(
                &token,
                expected_epoch,
                call_id,
                rating,
                feedback,
            )
            .await
        }
    }

    pub async fn rate_call_with_feedback_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        call_id: &str,
        rating: u8,
        feedback: Option<&str>,
    ) -> std::result::Result<CallRecordDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .rate_call(token, call_id, rating, feedback)
            .await
    }

    pub fn send_call_signal(
        &self,
        req: CallSignalRequest,
        target_user_id: Option<u64>,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.send_call_signal_with_token(&token, expected_epoch, req, target_user_id)
                .await
        }
    }

    pub async fn send_call_signal_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: CallSignalRequest,
        target_user_id: Option<u64>,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        if self.is_connected() {
            let _ = self.send_ws_frame(WsClientFrame::CallSignal {
                call_id: req.call_id.clone(),
                signal_type: req.signal_type.clone(),
                target_user_id,
                sdp: req.sdp.clone(),
                candidate: req.candidate.clone(),
                sdp_mid: req.sdp_mid.clone(),
                sdp_mline_index: req.sdp_mline_index,
            });
        }
        self.http_client.call_signal(token, &req).await
    }

    pub fn get_turn_credentials(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_turn_credentials_with_token(&token, expected_epoch)
                .await
        }
    }

    pub async fn get_turn_credentials_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<serde_json::Value, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_turn_credentials(token).await
    }

    pub fn search_messages<'a>(
        &'a self,
        query: &'a str,
        filter: Option<&'a str>,
        chat_id: Option<i64>,
        limit: usize,
    ) -> impl std::future::Future<Output = std::result::Result<SearchResponse, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .search_messages(&token, query, filter, chat_id, limit)
                .await
        }
    }

    pub fn get_channel_statistics(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<ChannelStatisticsDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_channel_statistics_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn get_channel_statistics_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<ChannelStatisticsDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_channel_statistics(token, chat_id).await
    }

    pub fn export_chat_history<'a>(
        &'a self,
        chat_id: i64,
        format: Option<&'a str>,
    ) -> impl std::future::Future<Output = std::result::Result<String, HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.export_chat_history_with_token(&token, expected_epoch, chat_id, format)
                .await
        }
    }

    pub async fn export_chat_history_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        format: Option<&str>,
    ) -> std::result::Result<String, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .export_chat_history(token, chat_id, format)
            .await
    }

    pub fn wallet_balance(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<WalletBalanceDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.wallet_balance_with_token(&token, expected_epoch).await
        }
    }

    pub async fn wallet_balance_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<WalletBalanceDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.wallet_balance(token).await
    }

    pub fn wallet_transactions(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<WalletTransactionDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.wallet_transactions_with_token(&token, expected_epoch)
                .await
        }
    }

    pub async fn wallet_transactions_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<WalletTransactionDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.wallet_transactions(token).await
    }

    pub fn wallet_topup(
        &self,
        amount_stars: i64,
    ) -> impl std::future::Future<Output = std::result::Result<WalletBalanceDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.wallet_topup_with_token(&token, expected_epoch, amount_stars)
                .await
        }
    }

    pub async fn wallet_topup_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        amount_stars: i64,
    ) -> std::result::Result<WalletBalanceDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.wallet_topup(token, amount_stars).await
    }

    pub fn wallet_gift_stars<'a>(
        &'a self,
        req: &'a StarGiftRequest,
    ) -> impl std::future::Future<Output = std::result::Result<WalletBalanceDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.wallet_gift_stars_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub async fn wallet_gift_stars_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &StarGiftRequest,
    ) -> std::result::Result<WalletBalanceDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.wallet_gift_stars(token, req).await
    }
}
