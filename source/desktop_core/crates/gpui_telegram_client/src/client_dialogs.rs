use crate::client::TelegramClient;
use crate::net::{
    ChatDetailDto, ChatDetails, ChatFolderDto, ChatMemberItem, ChatSummary, CreateChatRequest,
    DraftDto, ForumTopicItem, HttpClientError, InviteLinkDto, PinnedMessageDto,
};
use gpui_telegram_core::types::{ChatId, MessageId};

impl TelegramClient {
    pub fn get_chats(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<ChatSummary>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_chats_with_token(&token, expected_epoch).await
        }
    }

    pub async fn get_chats_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<ChatSummary>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_chats(token).await
    }

    pub fn create_chat<'a>(
        &'a self,
        req: &'a CreateChatRequest,
    ) -> impl std::future::Future<Output = std::result::Result<ChatDetailDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.create_chat_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub async fn create_chat_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &CreateChatRequest,
    ) -> std::result::Result<ChatDetailDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let chat = self.http_client.create_chat(token, req).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.upsert_chat(chat.clone());
        }
        Ok(chat)
    }

    pub fn provision_welcome_chat(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<ChatDetailDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            let chat = self.http_client.provision_welcome_chat(&token).await?;
            if self.ensure_pinned_session(expected_epoch, &token).is_ok() {
                self.local_store.upsert_chat(chat.clone());
            }
            Ok(chat)
        }
    }

    pub fn get_chat(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<ChatDetails, HttpClientError>> + Send + '_
    {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_chat_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn get_chat_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<ChatDetails, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_chat(token, chat_id).await
    }

    pub fn update_chat_info<'a>(
        &'a self,
        chat_id: i64,
        title: Option<&'a str>,
        about: Option<&'a str>,
    ) -> impl std::future::Future<Output = std::result::Result<bool, HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.update_chat_info_with_token(&token, expected_epoch, chat_id, title, about)
                .await
        }
    }

    pub async fn update_chat_info_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        title: Option<&str>,
        about: Option<&str>,
    ) -> std::result::Result<bool, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .update_chat_info(token, chat_id, title, about)
            .await
    }

    pub fn delete_chat(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.delete_chat_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn delete_chat_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.delete_chat(token, chat_id).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.remove_chat(ChatId(chat_id));
        }
        Ok(())
    }

    pub fn clear_chat_history(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.clear_chat_history_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn clear_chat_history_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.clear_chat_history(token, chat_id).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.clear_chat_messages(ChatId(chat_id));
        }
        Ok(())
    }

    pub fn leave_or_delete_chat(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.leave_or_delete_chat_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn leave_or_delete_chat_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .leave_or_delete_chat(token, chat_id)
            .await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.remove_chat(ChatId(chat_id));
        }
        Ok(())
    }

    pub fn set_chat_ttl(
        &self,
        chat_id: i64,
        ttl_seconds: u32,
    ) -> impl std::future::Future<Output = std::result::Result<u32, HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.set_chat_ttl_with_token(&token, expected_epoch, chat_id, ttl_seconds)
                .await
        }
    }

    pub async fn set_chat_ttl_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        ttl_seconds: u32,
    ) -> std::result::Result<u32, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .set_chat_ttl(token, chat_id, ttl_seconds)
            .await
    }

    pub fn get_chat_members(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<ChatMemberItem>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_chat_members_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn get_chat_members_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<Vec<ChatMemberItem>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_chat_members(token, chat_id).await
    }

    pub fn add_chat_members<'a>(
        &'a self,
        chat_id: i64,
        user_ids: &'a [u64],
    ) -> impl std::future::Future<Output = std::result::Result<bool, HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.add_chat_members_with_token(&token, expected_epoch, chat_id, user_ids)
                .await
        }
    }

    pub async fn add_chat_members_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        user_ids: &[u64],
    ) -> std::result::Result<bool, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .add_chat_members(token, chat_id, user_ids)
            .await
    }

    pub fn update_chat_member_role<'a>(
        &'a self,
        chat_id: i64,
        user_id: u64,
        role: &'a str,
        admin_title: Option<&'a str>,
        admin_rights: Option<u64>,
    ) -> impl std::future::Future<Output = std::result::Result<bool, HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .update_chat_member_role(&token, chat_id, user_id, role, admin_title, admin_rights)
                .await
        }
    }

    pub fn remove_chat_member(
        &self,
        chat_id: i64,
        user_id: u64,
    ) -> impl std::future::Future<Output = std::result::Result<bool, HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .remove_chat_member(&token, chat_id, user_id)
                .await
        }
    }

    pub fn list_invites(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<InviteLinkDto>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_invites_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn list_invites_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<Vec<InviteLinkDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_invites(token, chat_id).await
    }

    pub fn create_invite<'a>(
        &'a self,
        chat_id: i64,
        title: Option<&'a str>,
        usage_limit: Option<u32>,
        expire_date_ms: Option<i64>,
        request_needed: bool,
    ) -> impl std::future::Future<Output = std::result::Result<InviteLinkDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .create_invite(
                    &token,
                    chat_id,
                    title,
                    usage_limit,
                    expire_date_ms,
                    request_needed,
                )
                .await
        }
    }

    pub fn revoke_invite<'a>(
        &'a self,
        chat_id: i64,
        invite_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.revoke_invite_with_token(&token, expected_epoch, chat_id, invite_id)
                .await
        }
    }

    pub async fn revoke_invite_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        invite_id: &str,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.revoke_invite(token, chat_id, invite_id).await
    }

    pub fn join_invite<'a>(
        &'a self,
        code: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<ChatDetailDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            let chat = self.http_client.join_invite(&token, code).await?;
            if self.ensure_pinned_session(expected_epoch, &token).is_ok() {
                self.local_store.upsert_chat(chat.clone());
            }
            Ok(chat)
        }
    }

    pub fn list_forum_topics(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<ForumTopicItem>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.list_forum_topics(&token, chat_id).await
        }
    }

    pub fn create_forum_topic<'a>(
        &'a self,
        chat_id: i64,
        title: &'a str,
        icon_color: i32,
        icon_emoji_id: Option<i64>,
    ) -> impl std::future::Future<Output = std::result::Result<ForumTopicItem, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .create_forum_topic(&token, chat_id, title, icon_color, icon_emoji_id)
                .await
        }
    }

    pub fn update_forum_topic<'a>(
        &'a self,
        chat_id: i64,
        topic_id: i64,
        title: Option<&'a str>,
        is_pinned: Option<bool>,
        is_closed: Option<bool>,
    ) -> impl std::future::Future<Output = std::result::Result<bool, HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .update_forum_topic(&token, chat_id, topic_id, title, is_pinned, is_closed)
                .await
        }
    }

    pub fn delete_forum_topic(
        &self,
        chat_id: i64,
        topic_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<bool, HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .delete_forum_topic(&token, chat_id, topic_id)
                .await
        }
    }

    pub fn list_folders(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<ChatFolderDto>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_folders_with_token(&token, expected_epoch).await
        }
    }

    pub async fn list_folders_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<ChatFolderDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_folders(token).await
    }

    pub fn save_folder<'a>(
        &'a self,
        folder: &'a ChatFolderDto,
    ) -> impl std::future::Future<Output = std::result::Result<ChatFolderDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.save_folder_with_token(&token, expected_epoch, folder)
                .await
        }
    }

    pub async fn save_folder_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        folder: &ChatFolderDto,
    ) -> std::result::Result<ChatFolderDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.save_folder(token, folder).await
    }

    pub fn update_folder<'a>(
        &'a self,
        folder_id: i64,
        folder: &'a ChatFolderDto,
    ) -> impl std::future::Future<Output = std::result::Result<ChatFolderDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client
                .update_folder(&token, folder_id, folder)
                .await
        }
    }

    pub fn delete_folder(
        &self,
        folder_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.delete_folder_with_token(&token, expected_epoch, folder_id)
                .await
        }
    }

    pub async fn delete_folder_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        folder_id: i64,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.delete_folder(token, folder_id).await
    }

    pub fn set_chat_archived(
        &self,
        chat_id: i64,
        archived: bool,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.set_chat_archived_with_token(&token, expected_epoch, chat_id, archived)
                .await
        }
    }

    pub async fn set_chat_archived_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        archived: bool,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .set_chat_archived(token, chat_id, archived)
            .await
    }

    pub fn set_chat_pinned(
        &self,
        chat_id: i64,
        pinned_flag: bool,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.set_chat_pinned_with_token(&token, expected_epoch, chat_id, pinned_flag)
                .await
        }
    }

    pub async fn set_chat_pinned_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        pinned: bool,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.set_chat_pinned(token, chat_id, pinned).await
    }

    pub fn set_chat_muted(
        &self,
        chat_id: i64,
        muted: bool,
        mute_until_ms: Option<i64>,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.set_chat_muted_with_token(&token, expected_epoch, chat_id, muted, mute_until_ms)
                .await
        }
    }

    pub async fn set_chat_muted_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        muted: bool,
        mute_until_ms: Option<i64>,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client
            .set_chat_muted(token, chat_id, muted, mute_until_ms)
            .await
    }

    pub fn list_pinned_messages(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<PinnedMessageDto>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_pinned_messages_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn list_pinned_messages_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<Vec<PinnedMessageDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let pins = self.http_client.list_pinned_messages(token, chat_id).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.set_pinned_messages(
                ChatId(chat_id),
                pins.iter().map(|p| MessageId(p.message_id)).collect(),
            );
        }
        Ok(pins)
    }

    pub fn pin_message(
        &self,
        chat_id: i64,
        message_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<PinnedMessageDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.pin_message_with_token(&token, expected_epoch, chat_id, message_id)
                .await
        }
    }

    pub async fn pin_message_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        message_id: i64,
    ) -> std::result::Result<PinnedMessageDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.pin_message(token, chat_id, message_id).await
    }

    pub fn unpin_message(
        &self,
        chat_id: i64,
        message_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.unpin_message_with_token(&token, expected_epoch, chat_id, message_id)
                .await
        }
    }

    pub async fn unpin_message_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        message_id: i64,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.unpin_message(token, chat_id, message_id).await
    }

    pub fn get_draft(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<Option<DraftDto>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_draft_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn get_draft_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<Option<DraftDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let draft = self.http_client.get_draft(token, chat_id).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.set_draft(ChatId(chat_id), draft.clone());
        }
        Ok(draft)
    }

    pub fn save_draft<'a>(
        &'a self,
        chat_id: i64,
        draft: &'a DraftDto,
    ) -> impl std::future::Future<Output = std::result::Result<DraftDto, HttpClientError>> + Send + 'a
    {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.save_draft_with_token(&token, expected_epoch, chat_id, draft)
                .await
        }
    }

    pub async fn save_draft_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        draft: &DraftDto,
    ) -> std::result::Result<DraftDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let saved = self.http_client.save_draft(token, chat_id, draft).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.set_draft(ChatId(chat_id), Some(saved.clone()));
        }
        Ok(saved)
    }

    pub fn clear_draft(
        &self,
        chat_id: i64,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.clear_draft_with_token(&token, expected_epoch, chat_id)
                .await
        }
    }

    pub async fn clear_draft_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.clear_draft(token, chat_id).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.set_draft(ChatId(chat_id), None);
        }
        Ok(())
    }
}
