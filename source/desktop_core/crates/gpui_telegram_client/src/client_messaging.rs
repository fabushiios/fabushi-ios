use crate::client::TelegramClient;
use crate::net::http_client::HistoricalReaction;
use crate::net::{
    CreatePollHttpRequest, EditMessageHttpRequest, HistoricalMessage, HttpClientError,
    PollHttpResponse, ReactionHttpRequest, ReactionHttpResponse, ReadChatHttpRequest,
    ReadChatHttpResponse, RichSendMessageRequest, RichSendMessageResponse,
};
use crate::outbox::OutboxMessageRecord;
use gpui_telegram_core::types::{ChatId, MessageId, PeerId, TopicId, UserId};
use gpui_telegram_proto::message::{
    MessageContent, MessageEntity, MessageItem, ReactionCount, ReplyHeader,
};
use gpui_telegram_proto::miniapp_contracts::{MiniAppTakeoverEvent, MiniAppWebMcpResponse};
use gpui_telegram_proto::sync::{SyncGetDifferenceRequest, SyncGetDifferenceResponse, TypingAction, TypingEvent};
use gpui_telegram_proto::ws::{WsClientFrame, WsServerFrame};

impl TelegramClient {
    fn stage_provisional_message(&self, req: &RichSendMessageRequest, nonce: u64) {
        let chat_id = ChatId(req.chat_id);
        let sender_id = UserId(self.current_user_id().unwrap_or(0));
        let content = req.content.clone().unwrap_or_else(|| {
            MessageContent::text_with_entities(
                req.text.clone().unwrap_or_default(),
                req.entities.clone(),
            )
        });
        let mut provisional = MessageItem::new(
            self.next_provisional_msg_id(),
            chat_id,
            sender_id,
            PeerId::Chat(chat_id),
            content,
        )
        .with_client_nonce(nonce);
        provisional.topic_id = req.topic_id.map(TopicId);
        if let Some(reply_id) = req.reply_to {
            let mut rh = ReplyHeader::new(MessageId(reply_id));
            if let Some(ref qt) = req.quote_text {
                rh = rh.with_quote(qt.clone());
            }
            provisional = provisional.with_reply_to(rh);
        }
        self.local_store.insert_message(provisional);
    }

    pub fn enqueue_message(
        &self,
        mut req: RichSendMessageRequest,
    ) -> gpui_telegram_core::error::Result<OutboxMessageRecord> {
        let nonce = req.client_nonce.unwrap_or_else(|| self.next_client_nonce());
        req.client_nonce = Some(nonce);
        self.stage_provisional_message(&req, nonce);
        self.outbox.enqueue(&req, nonce)
    }

    pub fn send_message_durable(
        &self,
        mut req: RichSendMessageRequest,
    ) -> impl std::future::Future<Output = std::result::Result<RichSendMessageResponse, HttpClientError>>
           + Send
           + '_ {
        let start_epoch = self.session_epoch();
        let start_token = self.token();
        async move {
            if !self.is_same_session_epoch(start_epoch) {
                return Err(HttpClientError::Unauthorized(
                    "Session changed before send".to_string(),
                ));
            }
            let nonce = req.client_nonce.unwrap_or_else(|| self.next_client_nonce());
            req.client_nonce = Some(nonce);
            self.stage_provisional_message(&req, nonce);
            match start_token.filter(|t| !t.trim().is_empty()) {
                Some(tok) => {
                    self.ensure_pinned_session(start_epoch, &tok)?;
                    self.outbox
                        .send_or_enqueue_direct_with_token(&tok, start_epoch, &req, nonce)
                        .await
                }
                None => self.outbox.send_or_enqueue_direct(&req, nonce).await,
            }
        }
    }

    pub async fn send_message_durable_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        mut req: RichSendMessageRequest,
    ) -> std::result::Result<RichSendMessageResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let nonce = req.client_nonce.unwrap_or_else(|| self.next_client_nonce());
        req.client_nonce = Some(nonce);
        self.stage_provisional_message(&req, nonce);
        self.outbox
            .send_or_enqueue_direct_with_token(token, expected_epoch, &req, nonce)
            .await
    }

    pub fn send_encrypted_dm<'a>(
        &'a self,
        chat_id: i64,
        recipient_user_id: u64,
        plaintext: &'a [u8],
    ) -> impl std::future::Future<Output = std::result::Result<RichSendMessageResponse, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            let env = self
                .e2ee
                .encrypt_dm(recipient_user_id, plaintext)
                .await
                .map_err(|e| HttpClientError::Api {
                    status: 400,
                    message: e.to_string(),
                })?;
            let req = RichSendMessageRequest {
                chat_id,
                text: None,
                content: Some(MessageContent::Encrypted(env)),
                entities: Vec::new(),
                reply_to: None,
                quote_text: None,
                topic_id: None,
                client_nonce: Some(self.next_client_nonce()),
                forward_from_chat_id: None,
                forward_from_msg_id: None,
                silent: false,
                scheduled_at_ms: None,
                inline_keyboard: None,
                send_as_peer_id: None,
                skip_server_bot_reply: false,
            };
            self.send_message_durable_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub fn send_encrypted_group<'a>(
        &'a self,
        chat_id: i64,
        member_user_ids: &'a [u64],
        plaintext: &'a [u8],
    ) -> impl std::future::Future<Output = std::result::Result<RichSendMessageResponse, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            let (dist_opt, group_env) = self
                .e2ee
                .encrypt_group(chat_id, member_user_ids, plaintext)
                .await
                .map_err(|e| HttpClientError::Api {
                    status: 400,
                    message: e.to_string(),
                })?;
            if let Some(dist_env) = dist_opt {
                let dist_req = RichSendMessageRequest {
                    chat_id,
                    text: None,
                    content: Some(MessageContent::Encrypted(dist_env)),
                    entities: Vec::new(),
                    reply_to: None,
                    quote_text: None,
                    topic_id: None,
                    client_nonce: Some(self.next_client_nonce()),
                    forward_from_chat_id: None,
                    forward_from_msg_id: None,
                    silent: true,
                    scheduled_at_ms: None,
                    inline_keyboard: None,
                    send_as_peer_id: None,
                    skip_server_bot_reply: false,
                };
                self.send_message_durable_with_token(&token, expected_epoch, dist_req)
                    .await?;
            }
            let msg_req = RichSendMessageRequest {
                chat_id,
                text: None,
                content: Some(MessageContent::Encrypted(group_env)),
                entities: Vec::new(),
                reply_to: None,
                quote_text: None,
                topic_id: None,
                client_nonce: Some(self.next_client_nonce()),
                forward_from_chat_id: None,
                forward_from_msg_id: None,
                silent: false,
                scheduled_at_ms: None,
                inline_keyboard: None,
                send_as_peer_id: None,
                skip_server_bot_reply: false,
            };
            self.send_message_durable_with_token(&token, expected_epoch, msg_req)
                .await
        }
    }

    pub fn send_text_message<'a>(
        &'a self,
        chat_id: i64,
        text: impl Into<String>,
        reply_to: Option<i64>,
    ) -> impl std::future::Future<Output = std::result::Result<RichSendMessageResponse, HttpClientError>>
           + Send
           + 'a {
        let text_str = text.into();
        let req = RichSendMessageRequest {
            chat_id,
            text: Some(text_str.clone()),
            content: Some(MessageContent::plain_text(text_str)),
            entities: Vec::new(),
            reply_to,
            quote_text: None,
            topic_id: None,
            client_nonce: Some(self.next_client_nonce()),
            forward_from_chat_id: None,
            forward_from_msg_id: None,
            silent: false,
            scheduled_at_ms: None,
            inline_keyboard: None,
            send_as_peer_id: None,
            skip_server_bot_reply: false,
        };
        self.send_message_durable(req)
    }

    pub fn send_ws_typing(&self, chat_id: i64, action: TypingAction) -> bool {
        let uid = UserId(self.current_user_id().unwrap_or(0));
        self.send_ws_frame(WsClientFrame::SendTyping(TypingEvent::new(
            ChatId(chat_id),
            uid,
            action,
        )))
    }

    pub fn subscribe_chat(&self, chat_id: i64) -> bool {
        self.send_ws_frame(WsClientFrame::SubscribeChat { chat_id })
    }

    pub fn unsubscribe_chat(&self, chat_id: i64) -> bool {
        self.send_ws_frame(WsClientFrame::UnsubscribeChat { chat_id })
    }

    pub fn send_webmcp_result(&self, resp: MiniAppWebMcpResponse) -> bool {
        self.send_ws_frame(WsClientFrame::MiniAppWebMcpResult(resp))
    }

    pub fn send_miniapp_takeover_state(&self, evt: MiniAppTakeoverEvent) -> bool {
        self.send_ws_frame(WsClientFrame::MiniAppTakeoverState(evt))
    }

    pub fn send_coding_host_event(
        &self,
        ev: gpui_telegram_proto::CodingHostEvent,
    ) -> gpui_telegram_core::error::Result<()> {
        if self.send_ws_frame(WsClientFrame::CodingHostEvent(ev)) {
            Ok(())
        } else {
            Err(gpui_telegram_core::error::TelegramError::Network(
                "WebSocket outbound channel is not connected".to_string(),
            ))
        }
    }

    pub fn get_chat_messages(
        &self,
        chat_id: u64,
        limit: Option<usize>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HistoricalMessage>, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_chat_messages_for_session(&token, expected_epoch, chat_id, limit)
                .await
        }
    }

    pub fn get_chat_messages_with_token<'a>(
        &'a self,
        token: &'a str,
        chat_id: u64,
        limit: Option<usize>,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<HistoricalMessage>, HttpClientError>,
    > + Send
           + 'a {
        let start_epoch = self.session_epoch();
        let pin_res = self.ensure_pinned_session(start_epoch, token);
        async move {
            pin_res?;
            self.ensure_pinned_session(start_epoch, token)?;
            self.get_chat_messages_inner(token, start_epoch, chat_id, limit)
                .await
        }
    }

    pub async fn get_chat_messages_for_session(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: u64,
        limit: Option<usize>,
    ) -> std::result::Result<Vec<HistoricalMessage>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.get_chat_messages_inner(token, expected_epoch, chat_id, limit)
            .await
    }

    async fn get_chat_messages_inner(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: u64,
        limit: Option<usize>,
    ) -> std::result::Result<Vec<HistoricalMessage>, HttpClientError> {
        let mut msgs = self.http_client.get_chat_messages(token, chat_id, limit).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_err() {
            return Ok(msgs);
        }
        let cid = ChatId(chat_id as i64);
        for hm in &mut msgs {
            let mid = MessageId(hm.message_id as i64);
            if hm.has_authoritative_reactions() {
                let counts: Vec<ReactionCount> = hm
                    .reactions
                    .iter()
                    .map(|r| ReactionCount::new(r.emoji.clone(), r.count, r.chosen))
                    .collect();
                self.local_store.update_reactions(cid, mid, counts);
            } else if let Some(cached) = self.local_store.get_reactions(cid, mid) {
                hm.reactions = cached
                    .into_iter()
                    .map(|r| HistoricalReaction {
                        emoji: r.emoji,
                        count: r.count,
                        chosen: r.is_chosen_by_me,
                        user_ids: Vec::new(),
                    })
                    .collect();
                hm.reactions_authoritative = true;
            }
        }
        Ok(msgs)
    }

    pub fn edit_message<'a>(
        &'a self,
        chat_id: i64,
        message_id: i64,
        text: impl Into<String>,
        entities: Vec<MessageEntity>,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        let text_str = text.into();
        async move {
            let (expected_epoch, token) = pinned?;
            self.edit_message_with_token(&token, expected_epoch, chat_id, message_id, text_str, entities)
                .await
        }
    }

    pub async fn edit_message_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        message_id: i64,
        text: impl Into<String>,
        entities: Vec<MessageEntity>,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let req = EditMessageHttpRequest {
            chat_id: Some(chat_id),
            message_id: Some(message_id),
            text: text.into(),
            entities,
        };
        self.http_client
            .edit_message(token, chat_id, message_id, &req)
            .await
    }

    pub fn delete_message(
        &self,
        chat_id: i64,
        message_id: i64,
        revoke: bool,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.delete_message_with_token(&token, expected_epoch, chat_id, message_id, revoke)
                .await
        }
    }

    pub async fn delete_message_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        message_id: i64,
        revoke: bool,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.delete_message(token, chat_id, message_id, revoke).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store
                .delete_messages(ChatId(chat_id), &[MessageId(message_id)]);
        }
        Ok(())
    }

    pub fn react_to_message<'a>(
        &'a self,
        chat_id: i64,
        message_id: i64,
        emoji: impl Into<String>,
        action: Option<&'a str>,
    ) -> impl std::future::Future<Output = std::result::Result<ReactionHttpResponse, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        let emoji_str = emoji.into();
        async move {
            let (expected_epoch, token) = pinned?;
            self.react_to_message_with_token(
                &token,
                expected_epoch,
                chat_id,
                message_id,
                emoji_str,
                action,
            )
            .await
        }
    }

    pub async fn react_to_message_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        message_id: i64,
        emoji: impl Into<String>,
        action: Option<&str>,
    ) -> std::result::Result<ReactionHttpResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.react_to_message_inner(token, expected_epoch, chat_id, message_id, emoji, action)
            .await
    }

    async fn react_to_message_inner(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        message_id: i64,
        emoji: impl Into<String>,
        action: Option<&str>,
    ) -> std::result::Result<ReactionHttpResponse, HttpClientError> {
        let req = ReactionHttpRequest {
            emoji: emoji.into(),
            action: action.map(|s| s.to_string()),
        };
        let mut resp = self
            .http_client
            .react_to_message(token, chat_id, message_id, &req)
            .await?;
        if self.ensure_pinned_session(expected_epoch, token).is_err() {
            return Ok(resp);
        }
        if resp.has_authoritative_reactions() {
            self.local_store.update_reactions(
                ChatId(chat_id),
                MessageId(message_id),
                resp.reactions.clone(),
            );
        } else {
            let updated = self.local_store.apply_viewer_reaction_mutation(
                ChatId(chat_id),
                MessageId(message_id),
                &req.emoji,
                req.action.as_deref(),
            );
            resp.reactions = updated;
        }
        Ok(resp)
    }

    pub fn create_poll<'a>(
        &'a self,
        chat_id: i64,
        req: &'a CreatePollHttpRequest,
    ) -> impl std::future::Future<Output = std::result::Result<PollHttpResponse, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.create_poll_with_token(&token, expected_epoch, chat_id, req)
                .await
        }
    }

    pub async fn create_poll_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        req: &CreatePollHttpRequest,
    ) -> std::result::Result<PollHttpResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.create_poll(token, chat_id, req).await
    }

    pub fn vote_poll(
        &self,
        poll_id: u64,
        option_ids: Vec<u8>,
    ) -> impl std::future::Future<Output = std::result::Result<PollHttpResponse, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.vote_poll_with_token(&token, expected_epoch, poll_id, option_ids)
                .await
        }
    }

    pub async fn vote_poll_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        poll_id: u64,
        option_ids: Vec<u8>,
    ) -> std::result::Result<PollHttpResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.vote_poll(token, poll_id, option_ids).await
    }

    pub fn close_poll(
        &self,
        poll_id: u64,
    ) -> impl std::future::Future<Output = std::result::Result<PollHttpResponse, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.close_poll(&token, poll_id).await
        }
    }

    pub fn mark_chat_read(
        &self,
        chat_id: i64,
        max_id: i64,
        topic_id: Option<i64>,
    ) -> impl std::future::Future<Output = std::result::Result<ReadChatHttpResponse, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.mark_chat_read_with_token(&token, expected_epoch, chat_id, max_id, topic_id)
                .await
        }
    }

    pub async fn mark_chat_read_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: i64,
        max_id: i64,
        topic_id: Option<i64>,
    ) -> std::result::Result<ReadChatHttpResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let req = ReadChatHttpRequest { max_id, topic_id };
        let resp = self.http_client.mark_chat_read(token, chat_id, &req).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            self.local_store.mark_read_inbox(
                ChatId(chat_id),
                MessageId(resp.read_inbox_max_id),
                Some(resp.unread_count),
            );
        }
        Ok(resp)
    }

    async fn sync_chat_difference_inner(
        &self,
        token: &str,
        expected_epoch: u64,
        chat_id: ChatId,
        limit: u32,
    ) -> std::result::Result<SyncGetDifferenceResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let pts = self.sync_coordinator.last_pts(chat_id);
        let req = SyncGetDifferenceRequest::new(chat_id, pts, limit);
        let diff = self.http_client.sync_get_difference(token, &req).await?;
        if self.ensure_pinned_session(expected_epoch, token).is_ok() {
            let _ = self.process_ws_frame(WsServerFrame::SyncDiff(diff.clone()));
            if !diff.has_more {
                self.clear_gap_chat(chat_id);
            }
        }
        Ok(diff)
    }

    pub fn sync_chat_difference(
        &self,
        chat_id: ChatId,
        limit: u32,
    ) -> impl std::future::Future<
        Output = std::result::Result<SyncGetDifferenceResponse, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.sync_chat_difference_inner(&token, expected_epoch, chat_id, limit)
                .await
        }
    }

    pub fn sync_all_gaps(
        &self,
        limit: u32,
    ) -> impl std::future::Future<
        Output = std::result::Result<Vec<SyncGetDifferenceResponse>, HttpClientError>,
    > + Send
           + '_ {
        let start_epoch = self.session_epoch();
        let pinned = self.snapshot_pinned_session();
        let chats = self.pending_gap_chats();
        async move {
            if chats.is_empty() {
                if !self.is_same_session_epoch(start_epoch) {
                    return Err(HttpClientError::Unauthorized(
                        "Session changed during gap sync".to_string(),
                    ));
                }
                return Ok(Vec::new());
            }
            let (expected_epoch, token) = pinned?;
            let mut results = Vec::with_capacity(chats.len());
            for cid in chats {
                self.ensure_pinned_session(expected_epoch, &token)?;
                results.push(
                    self.sync_chat_difference_inner(&token, expected_epoch, cid, limit)
                        .await?,
                );
            }
            Ok(results)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::client::TelegramClient;
    use gpui_telegram_proto::{CodingHostEvent, CodingProviderKind, DetectedCodingProvider, WsClientFrame};

    #[test]
    fn send_coding_host_event_forwards_ws_frame_when_connected() {
        let client = TelegramClient::new_with_url("http://127.0.0.1:0".to_string());
        let ev = CodingHostEvent::HostRegister {
            device_id: "dev-mac-1".to_string(),
            display_name: "MacBook Pro".to_string(),
            os_name: "macos".to_string(),
            providers: vec![DetectedCodingProvider {
                kind: CodingProviderKind::ClaudeCode,
                display_name: "Claude Code".to_string(),
                binary_path: Some("/usr/local/bin/claude".to_string()),
                version: Some("1.0.2".to_string()),
                installed: true,
                recommended_username: "ClaudeCodeBot".to_string(),
                supports_acp: true,
                login_hint: String::new(),
            }],
        };

        assert!(
            client.send_coding_host_event(ev.clone()).is_err(),
            "disconnected ws channel must return error"
        );

        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_in_tx, in_rx) = tokio::sync::mpsc::unbounded_channel();
        client.ws_channel.attach_channels(out_tx, in_rx);

        client
            .send_coding_host_event(ev.clone())
            .expect("connected ws channel must send CodingHostEvent");

        match out_rx.try_recv().expect("expected outbound ws frame") {
            WsClientFrame::CodingHostEvent(sent) => assert_eq!(sent, ev),
            other => panic!("expected WsClientFrame::CodingHostEvent, got {other:?}"),
        }
    }
}
