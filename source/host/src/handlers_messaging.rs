use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use chrono::Utc;
use gpui_telegram_client::e2ee::keys::{b64_decode_32, b64_encode, random_32, x25519_pub_from_secret};
use gpui_telegram_client::e2ee::{
    compute_safety_number_emojis, DoubleRatchetSession, SenderKeyState,
};
use gpui_telegram_client::net::{
    AddContactHttpRequest, ChatFolderDto, CreateChatRequest, CreatePollHttpRequest, DraftDto,
    ProtoUpdateProfileRequest, RichSendMessageRequest,
};
use gpui_telegram_client::outbox::OutboxStatus;
use gpui_telegram_client::sync::SyncOutcome;
use gpui_telegram_core::types::{ChatId, MessageId, PeerId, UserId};
use gpui_telegram_media::chunked_upload::{recommend_part_size, ChunkedUploadSession};
use gpui_telegram_media::waveform::{
    decode_waveform_5bit, decode_waveform_to_floats, encode_waveform_5bit,
    encode_waveform_from_floats, normalize_pcm_peaks, TELEGRAM_WAVEFORM_BARS,
};
use gpui_telegram_proto::e2ee::{
    DevicePreKeyBundle, EncryptedEnvelope, RatchetHeaderDto, SenderKeyDistributionMessage,
};
use gpui_telegram_proto::message::{MessageContent, MessageEntity, MessageItem, ReplyHeader};
use gpui_telegram_proto::sync::{PtsUpdate, TypingAction};
use gpui_telegram_proto::ws::WsServerFrame;
use std::sync::Arc;

use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::dedup::{CachedMessage, DedupWaitError, InFlightSlot, ReservationOutcome};
use crate::error::{HostError, HostResult};
use crate::events::domain_event_to_host_event;
use crate::host::FabushiAppHost;
use crate::storage::{compute_storage_metrics, purge_media_cas_dir};

fn parse_req<T: serde::de::DeserializeOwned>(params: &Value) -> HostResult<T> {
    serde_json::from_value(params.clone())
        .map_err(|e| HostError::invalid_arguments(format!("Invalid request parameters: {e}")))
}

fn req_str<'a>(params: &'a Value, key: &str) -> HostResult<&'a str> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required string '{key}'")))
}

fn req_raw_str<'a>(params: &'a Value, key: &str) -> HostResult<&'a str> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required string '{key}'")))
}

fn req_i64(params: &Value, key: &str) -> HostResult<i64> {
    params
        .get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required i64 '{key}'")))
}

fn req_u64(params: &Value, key: &str) -> HostResult<u64> {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required u64 '{key}'")))
}

fn to_val<T: serde::Serialize>(v: T) -> HostResult<Value> {
    serde_json::to_value(v).map_err(|e| HostError::internal(format!("Serialization error: {e}")))
}

struct LeaderGuard<'a> {
    host: &'a FabushiAppHost,
    sender_id: u64,
    nonce: u64,
    slot: Arc<InFlightSlot>,
    completed: bool,
}

impl<'a> Drop for LeaderGuard<'a> {
    fn drop(&mut self) {
        if !self.completed {
            self.host
                .dedup_store
                .lock()
                .abort_slot(self.sender_id, self.nonce, &self.slot);
        }
    }
}

pub fn handle_chats(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "list" => {
            let local_only = params
                .get("local_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !local_only && client.token().is_some() {
                let c = std::sync::Arc::clone(&client);
                match host.run_cancellable(req_id, async move {
                    c.get_chats().await.map_err(HostError::from)
                }) {
                    Ok(chats) => return to_val(chats),
                    Err(e) if !params.get("fallback_local").and_then(|v| v.as_bool()).unwrap_or(false) => {
                        return Err(e);
                    }
                    Err(_) => {}
                }
            }
            // Offline/unauthenticated: strictly return real persisted LocalStore chats (empty if none)
            let local = client.local_store.list_chats();
            to_val(local)
        }
        "get" => {
            let chat_id = req_i64(params, "chat_id")?;
            if client.token().is_some() {
                let c = std::sync::Arc::clone(&client);
                let res = host.run_cancellable(req_id, async move {
                    c.get_chat(chat_id).await.map_err(HostError::from)
                })?;
                return to_val(res);
            }
            let local = client
                .local_store
                .get_chat(ChatId(chat_id))
                .ok_or_else(|| HostError::not_found(format!("Chat {chat_id} not found")))?;
            to_val(local)
        }
        "create" => {
            let req: CreateChatRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.create_chat(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "provisionWelcome" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .provision_welcome_chat()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "updateInfo" => {
            let chat_id = req_i64(params, "chat_id")?;
            let title = params.get("title").and_then(|v| v.as_str()).map(ToOwned::to_owned);
            let about = params.get("about").and_then(|v| v.as_str()).map(ToOwned::to_owned);
            let ok = host.run_cancellable(req_id, async move {
                client
                    .update_chat_info(chat_id, title.as_deref(), about.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "updated": ok }))
        }
        "delete" => {
            let chat_id = req_i64(params, "chat_id")?;
            host.run_cancellable(req_id, async move {
                client.delete_chat(chat_id).await.map_err(HostError::from)
            })?;
            Ok(json!({ "deleted": true }))
        }
        "clearHistory" => {
            let chat_id = req_i64(params, "chat_id")?;
            host.run_cancellable(req_id, async move {
                client
                    .clear_chat_history(chat_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "cleared": true }))
        }
        "leave" => {
            let chat_id = req_i64(params, "chat_id")?;
            host.run_cancellable(req_id, async move {
                client
                    .leave_or_delete_chat(chat_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "left": true }))
        }
        "setTtl" => {
            let chat_id = req_i64(params, "chat_id")?;
            let ttl_seconds = params
                .get("ttl_seconds")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| HostError::invalid_arguments("Missing u32 'ttl_seconds'"))?
                as u32;
            let has_token = host.with_fenced_session_read(&client, || {
                client.local_store.set_chat_ttl(ChatId(chat_id), ttl_seconds);
                Ok(client.token().is_some())
            })?;
            if has_token {
                let applied = host.run_cancellable(req_id, async move {
                    client
                        .set_chat_ttl(chat_id, ttl_seconds)
                        .await
                        .map_err(HostError::from)
                })?;
                return Ok(json!({ "chat_id": chat_id, "ttl_seconds": applied }));
            }
            Ok(json!({ "chat_id": chat_id, "ttl_seconds": ttl_seconds }))
        }
        "setArchived" => {
            let chat_id = req_i64(params, "chat_id")?;
            let archived = params
                .get("archived")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| HostError::invalid_arguments("Missing bool 'archived'"))?;
            host.run_cancellable(req_id, async move {
                client
                    .set_chat_archived(chat_id, archived)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "chat_id": chat_id, "archived": archived }))
        }
        "setPinned" => {
            let chat_id = req_i64(params, "chat_id")?;
            let pinned = params
                .get("pinned")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| HostError::invalid_arguments("Missing bool 'pinned'"))?;
            host.run_cancellable(req_id, async move {
                client
                    .set_chat_pinned(chat_id, pinned)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "chat_id": chat_id, "pinned": pinned }))
        }
        "setMuted" => {
            let chat_id = req_i64(params, "chat_id")?;
            let muted = params
                .get("muted")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| HostError::invalid_arguments("Missing bool 'muted'"))?;
            let mute_until_ms = params.get("mute_until_ms").and_then(|v| v.as_i64());
            host.run_cancellable(req_id, async move {
                client
                    .set_chat_muted(chat_id, muted, mute_until_ms)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "chat_id": chat_id, "muted": muted }))
        }
        "membersList" => {
            let chat_id = req_i64(params, "chat_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_chat_members(chat_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "membersAdd" => {
            let chat_id = req_i64(params, "chat_id")?;
            let user_ids: Vec<u64> = params
                .get("user_ids")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
                .ok_or_else(|| HostError::invalid_arguments("Missing array 'user_ids'"))?;
            let ok = host.run_cancellable(req_id, async move {
                client
                    .add_chat_members(chat_id, &user_ids)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "added": ok }))
        }
        "memberRoleUpdate" => {
            let chat_id = req_i64(params, "chat_id")?;
            let user_id = req_u64(params, "user_id")?;
            let role = req_str(params, "role")?.to_string();
            let admin_title = params
                .get("admin_title")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let admin_rights = params.get("admin_rights").and_then(|v| v.as_u64());
            let ok = host.run_cancellable(req_id, async move {
                client
                    .update_chat_member_role(
                        chat_id,
                        user_id,
                        &role,
                        admin_title.as_deref(),
                        admin_rights,
                    )
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "updated": ok }))
        }
        "memberRemove" => {
            let chat_id = req_i64(params, "chat_id")?;
            let user_id = req_u64(params, "user_id")?;
            let ok = host.run_cancellable(req_id, async move {
                client
                    .remove_chat_member(chat_id, user_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "removed": ok }))
        }
        "invitesList" => {
            let chat_id = req_i64(params, "chat_id")?;
            let res = host.run_cancellable(req_id, async move {
                client.list_invites(chat_id).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "inviteCreate" => {
            let chat_id = req_i64(params, "chat_id")?;
            let title = params.get("title").and_then(|v| v.as_str()).map(ToOwned::to_owned);
            let usage_limit = params.get("usage_limit").and_then(|v| v.as_u64()).map(|n| n as u32);
            let expire_date_ms = params.get("expire_date_ms").and_then(|v| v.as_i64());
            let request_needed = params
                .get("request_needed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let res = host.run_cancellable(req_id, async move {
                client
                    .create_invite(
                        chat_id,
                        title.as_deref(),
                        usage_limit,
                        expire_date_ms,
                        request_needed,
                    )
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "inviteRevoke" => {
            let chat_id = req_i64(params, "chat_id")?;
            let invite_id = req_str(params, "invite_id")?.to_string();
            host.run_cancellable(req_id, async move {
                client
                    .revoke_invite(chat_id, &invite_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "revoked": true }))
        }
        "inviteJoin" => {
            let code = req_str(params, "code")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client.join_invite(&code).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "pinsList" => {
            let chat_id = req_i64(params, "chat_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_pinned_messages(chat_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "pinMessage" => {
            let chat_id = req_i64(params, "chat_id")?;
            let message_id = req_i64(params, "message_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .pin_message(chat_id, message_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "unpinMessage" => {
            let chat_id = req_i64(params, "chat_id")?;
            let message_id = req_i64(params, "message_id")?;
            host.run_cancellable(req_id, async move {
                client
                    .unpin_message(chat_id, message_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "unpinned": true }))
        }
        "draftGet" => {
            let chat_id = req_i64(params, "chat_id")?;
            if client.token().is_some() {
                let res = host.run_cancellable(req_id, async move {
                    client.get_draft(chat_id).await.map_err(HostError::from)
                })?;
                return to_val(res);
            }
            to_val(client.local_store.get_draft(ChatId(chat_id)))
        }
        "draftSave" => {
            let chat_id = req_i64(params, "chat_id")?;
            let draft: DraftDto = parse_req(params.get("draft").unwrap_or(params))?;
            let has_token = host.with_fenced_session_read(&client, || {
                client
                    .local_store
                    .set_draft(ChatId(chat_id), Some(draft.clone()));
                Ok(client.token().is_some())
            })?;
            if has_token {
                let res = host.run_cancellable(req_id, async move {
                    client
                        .save_draft(chat_id, &draft)
                        .await
                        .map_err(HostError::from)
                })?;
                return to_val(res);
            }
            to_val(draft)
        }
        "draftClear" => {
            let chat_id = req_i64(params, "chat_id")?;
            let has_token = host.with_fenced_session_read(&client, || {
                client.local_store.set_draft(ChatId(chat_id), None);
                Ok(client.token().is_some())
            })?;
            if has_token {
                host.run_cancellable(req_id, async move {
                    client.clear_draft(chat_id).await.map_err(HostError::from)
                })?;
            }
            Ok(json!({ "cleared": true }))
        }
        "sendAsPeers" => {
            let chat_id = req_i64(params, "chat_id")?;
            let token = host
                .fenced_client_token(&client)?
                .ok_or_else(|| HostError::unauthorized("No active session token"))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .http_client
                    .get_send_as_peers(&token, chat_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "statistics" => {
            let chat_id = req_i64(params, "chat_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_channel_statistics(chat_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "export" => {
            let chat_id = req_i64(params, "chat_id")?;
            let format = params
                .get("format")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let data = host.run_cancellable(req_id, async move {
                client
                    .export_chat_history(chat_id, format.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "chat_id": chat_id, "export": data }))
        }
        "subscribeWs" => {
            let chat_id = req_i64(params, "chat_id")?;
            Ok(json!({ "sent": client.subscribe_chat(chat_id) }))
        }
        "unsubscribeWs" => {
            let chat_id = req_i64(params, "chat_id")?;
            Ok(json!({ "sent": client.unsubscribe_chat(chat_id) }))
        }
        _ => Err(HostError::unknown_command(&format!("chats.{action}"))),
    }
}

pub fn handle_folders(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "list" => {
            if host.fenced_client_token(&client)?.is_some() {
                let c = std::sync::Arc::clone(&client);
                let res = host.run_cancellable(req_id, async move {
                    c.list_folders().await.map_err(HostError::from)
                })?;
                host.with_fenced_session_read(&client, || {
                    client.local_store.set_folders(res.clone());
                    Ok(())
                })?;
                return to_val(res);
            }
            to_val(client.local_store.get_folders())
        }
        "save" => {
            let folder: ChatFolderDto = parse_req(params)?;
            let has_token = host.with_fenced_session_read(&client, || {
                client.local_store.upsert_folder(folder.clone());
                Ok(client.token().is_some())
            })?;
            if has_token {
                let c = std::sync::Arc::clone(&client);
                let res = host.run_cancellable(req_id, async move {
                    c.save_folder(&folder).await.map_err(HostError::from)
                })?;
                host.with_fenced_session_read(&client, || {
                    client.local_store.upsert_folder(res.clone());
                    Ok(())
                })?;
                return to_val(res);
            }
            to_val(folder)
        }
        "update" => {
            let folder_id = req_i64(params, "folder_id")?;
            let folder: ChatFolderDto = parse_req(params.get("folder").unwrap_or(params))?;
            let has_token = host.with_fenced_session_read(&client, || {
                client.local_store.upsert_folder(folder.clone());
                Ok(client.token().is_some())
            })?;
            if has_token {
                let res = host.run_cancellable(req_id, async move {
                    client
                        .update_folder(folder_id, &folder)
                        .await
                        .map_err(HostError::from)
                })?;
                return to_val(res);
            }
            to_val(folder)
        }
        "delete" => {
            let folder_id = req_i64(params, "folder_id")?;
            let has_token = host.with_fenced_session_read(&client, || {
                client.local_store.remove_folder(folder_id);
                Ok(client.token().is_some())
            })?;
            if has_token {
                host.run_cancellable(req_id, async move {
                    client
                        .delete_folder(folder_id)
                        .await
                        .map_err(HostError::from)
                })?;
            }
            Ok(json!({ "deleted": true, "folder_id": folder_id }))
        }
        _ => Err(HostError::unknown_command(&format!("folders.{action}"))),
    }
}

pub fn handle_topics(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "list" => {
            let chat_id = req_i64(params, "chat_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_forum_topics(chat_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "create" => {
            let chat_id = req_i64(params, "chat_id")?;
            let title = req_str(params, "title")?.to_string();
            let icon_color = params
                .get("icon_color")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let icon_emoji_id = params.get("icon_emoji_id").and_then(|v| v.as_i64());
            let res = host.run_cancellable(req_id, async move {
                client
                    .create_forum_topic(chat_id, &title, icon_color, icon_emoji_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "update" => {
            let chat_id = req_i64(params, "chat_id")?;
            let topic_id = req_i64(params, "topic_id")?;
            let title = params.get("title").and_then(|v| v.as_str()).map(ToOwned::to_owned);
            let is_pinned = params.get("is_pinned").and_then(|v| v.as_bool());
            let is_closed = params.get("is_closed").and_then(|v| v.as_bool());
            let ok = host.run_cancellable(req_id, async move {
                client
                    .update_forum_topic(chat_id, topic_id, title.as_deref(), is_pinned, is_closed)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "updated": ok }))
        }
        "delete" => {
            let chat_id = req_i64(params, "chat_id")?;
            let topic_id = req_i64(params, "topic_id")?;
            let ok = host.run_cancellable(req_id, async move {
                client
                    .delete_forum_topic(chat_id, topic_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "deleted": ok }))
        }
        _ => Err(HostError::unknown_command(&format!("topics.{action}"))),
    }
}

pub fn handle_contacts(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "list" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_contacts().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "add" => {
            let req: AddContactHttpRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.add_contact(req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "delete" => {
            let user_id = req_u64(params, "user_id")?;
            host.run_cancellable(req_id, async move {
                client.delete_contact(user_id).await.map_err(HostError::from)
            })?;
            Ok(json!({ "deleted": true }))
        }
        "resolvePeer" => {
            let username = req_str(params, "username")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .resolve_peer(&username)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "userProfile" => {
            let user_id = req_u64(params, "user_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_user_profile(user_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "myProfile" => {
            let res = host.run_cancellable(req_id, async move {
                client.get_my_profile().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "updateProfile" => {
            let req: ProtoUpdateProfileRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.update_profile(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "blockedList" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_blocked_users().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "block" => {
            let user_id = req_u64(params, "user_id")?;
            let res = host.run_cancellable(req_id, async move {
                client.block_user(user_id).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "unblock" => {
            let user_id = req_u64(params, "user_id")?;
            host.run_cancellable(req_id, async move {
                client.unblock_user(user_id).await.map_err(HostError::from)
            })?;
            Ok(json!({ "unblocked": true }))
        }
        _ => Err(HostError::unknown_command(&format!("contacts.{action}"))),
    }
}

pub fn handle_messages(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "send" | "sendText" => {
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let text = params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let sender_id = params
                .get("sender_id")
                .and_then(|v| v.as_u64())
                .or_else(|| client.current_user_id())
                .unwrap_or(1);
            let reply_to = params.get("reply_to").and_then(|v| v.as_i64());
            let client_nonce = params.get("client_nonce").and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_i64().and_then(|n| (n >= 0).then_some(n as u64)))
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
            });

            let now = Utc::now();
            let mut leader_guard: Option<LeaderGuard> = None;

            if let Some(nonce) = client_nonce {
                loop {
                    let outcome = host.dedup_store.lock().reserve(sender_id, nonce, now);
                    match outcome {
                        ReservationOutcome::AlreadyExists(cached) => {
                            return Ok(json!({
                                "id": cached.message_id,
                                "chat_id": cached.chat_id,
                                "sender_id": cached.sender_id,
                                "text": cached.text,
                                "timestamp": cached.timestamp,
                                "reply_to": cached.reply_to,
                                "client_nonce": cached.client_nonce,
                                "status": cached.delivery_status,
                                "duplicate": true
                            }));
                        }
                        ReservationOutcome::Follower(slot) => match slot.wait() {
                            Ok(cached) => {
                                return Ok(json!({
                                    "id": cached.message_id,
                                    "chat_id": cached.chat_id,
                                    "sender_id": cached.sender_id,
                                    "text": cached.text,
                                    "timestamp": cached.timestamp,
                                    "reply_to": cached.reply_to,
                                    "client_nonce": cached.client_nonce,
                                    "status": cached.delivery_status,
                                    "duplicate": true
                                }));
                            }
                            Err(DedupWaitError::Aborted) => continue,
                        },
                        ReservationOutcome::Leader(slot) => {
                            leader_guard = Some(LeaderGuard {
                                host,
                                sender_id,
                                nonce,
                                slot,
                                completed: false,
                            });
                            break;
                        }
                    }
                }
            }

            // If authenticated and remote delivery requested (or active token + non-local_only),
            // attempt real durable server send via OutboxWorker/HttpClient!
            let local_only = params
                .get("local_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let remote_delivery = !local_only && host.fenced_client_token(&client)?.is_some();

            let timestamp = now.timestamp_millis();
            let mut assigned_id = host.next_message_id();
            let mut delivery_status = "sent";

            if remote_delivery {
                let effective_nonce = client_nonce.unwrap_or_else(|| client.next_client_nonce());
                let req = RichSendMessageRequest {
                    chat_id,
                    text: Some(text.clone()),
                    content: Some(MessageContent::plain_text(text.clone())),
                    entities: Vec::new(),
                    reply_to,
                    quote_text: None,
                    topic_id: params.get("topic_id").and_then(|v| v.as_i64()),
                    client_nonce: Some(effective_nonce),
                    forward_from_chat_id: None,
                    forward_from_msg_id: None,
                    silent: params.get("silent").and_then(|v| v.as_bool()).unwrap_or(false),
                    scheduled_at_ms: params.get("scheduled_at_ms").and_then(|v| v.as_i64()),
                    inline_keyboard: None,
                    send_as_peer_id: params.get("send_as_peer_id").and_then(|v| v.as_u64()),
                    skip_server_bot_reply: false,
                };
                let c = std::sync::Arc::clone(&client);
                match host.run_cancellable(req_id, async move {
                    c.send_message_durable(req).await.map_err(HostError::from)
                }) {
                    Ok(resp) => {
                        assigned_id = resp.message_id as i64;
                        if resp.pts > 0 {
                            host.with_fenced_session_read(&client, || {
                                client.sync_coordinator.set_pts(ChatId(chat_id), resp.pts);
                                let _ = host.host_db.save_pts(chat_id, resp.pts);
                                Ok(())
                            })?;
                        }
                    }
                    Err(e)
                        if matches!(
                            e.code.as_str(),
                            "cancelled" | "stale_generation" | "host_closed"
                        ) =>
                    {
                        return Err(e);
                    }
                    Err(e) => {
                        // Only treat genuinely retryable / queued-offline errors where the outbox
                        // holds a Pending/InFlight entry as "queued_outbox". Permanent HTTP 4xx
                        // (400/401/403/404/409/422) or OutboxStatus::Failed MUST fail closed so
                        // LeaderGuard::drop aborts the dedup slot without poisoning nonce_dedup or search!
                        let outbox_pending = host.with_fenced_session_read(&client, || {
                            let pending = client
                                .outbox
                                .store()
                                .get(effective_nonce)
                                .ok()
                                .flatten()
                                .is_some_and(|rec| {
                                    matches!(
                                        rec.status,
                                        OutboxStatus::Pending | OutboxStatus::InFlight
                                    )
                                });
                            if !(e.retryable && pending) {
                                let _ = client.outbox.store().delete_failed(effective_nonce);
                            }
                            Ok(pending)
                        })?;
                        if e.retryable && outbox_pending {
                            delivery_status = "queued_outbox";
                        } else {
                            return Err(e);
                        }
                    }
                }
            }

            let local_msg_opt = if !remote_delivery {
                let mut msg_item = MessageItem::new(
                    MessageId(assigned_id),
                    ChatId(chat_id),
                    UserId(sender_id),
                    PeerId::User(UserId(sender_id)),
                    MessageContent::plain_text(text.clone()),
                );
                if let Some(n) = client_nonce {
                    msg_item = msg_item.with_client_nonce(n);
                }
                if let Some(r) = reply_to {
                    msg_item.reply_to = Some(ReplyHeader::new(MessageId(r)));
                }
                Some(msg_item)
            } else {
                None
            };

            // Fence offline LocalStore insert, SQLite FTS5, Tantivy search index, and nonce dedup
            // persistence under session_lock.read() strictly before emit_event (NB5-02).
            host.with_fenced_session_read(&client, || {
                if let Some(msg_item) = local_msg_opt {
                    client.local_store.insert_message(msg_item);
                }
                if !text.is_empty() {
                    let _ = host.host_db.index_message_fts(
                        assigned_id,
                        chat_id,
                        sender_id,
                        timestamp,
                        &text,
                    );
                    let _ = host.search_engine.index_message(
                        assigned_id,
                        chat_id,
                        sender_id,
                        timestamp,
                        &text,
                    );
                }
                if let Some(mut guard) = leader_guard {
                    let cached = CachedMessage {
                        message_id: assigned_id,
                        chat_id,
                        sender_id,
                        text: text.clone(),
                        timestamp,
                        reply_to,
                        client_nonce: guard.nonce,
                        delivery_status: delivery_status.to_string(),
                        created_at: now,
                    };
                    let _ = host.host_db.persist_nonce(&cached);
                    host.dedup_store
                        .lock()
                        .complete_slot(sender_id, guard.nonce, &guard.slot, cached);
                    guard.completed = true;
                }
                Ok(())
            })?;

            let payload = json!({
                "id": assigned_id,
                "chat_id": chat_id,
                "sender_id": sender_id,
                "text": text,
                "timestamp": timestamp,
                "reply_to": reply_to,
                "client_nonce": client_nonce,
                "status": delivery_status,
            });
            host.emit_event("newMessage", payload.clone());
            Ok(payload)
        }
        "sendDurable" => {
            let req: RichSendMessageRequest = parse_req(params)?;
            let resp = host.run_cancellable(req_id, async move {
                client
                    .send_message_durable(req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(resp)
        }
        "localList" | "list" => {
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let msgs = client.get_messages(ChatId(chat_id))?;
            to_val(msgs)
        }
        "history" => {
            let chat_id = req_u64(params, "chat_id")?;
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let msgs = host.run_cancellable(req_id, async move {
                client
                    .get_chat_messages(chat_id, limit)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(msgs)
        }
        "edit" => {
            let chat_id = req_i64(params, "chat_id")?;
            let message_id = req_i64(params, "message_id")?;
            let text = req_raw_str(params, "text")?.to_string();
            let entities: Vec<MessageEntity> = params
                .get("entities")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            host.run_cancellable(req_id, async move {
                client
                    .edit_message(chat_id, message_id, text, entities)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "edited": true, "chat_id": chat_id, "message_id": message_id }))
        }
        "delete" => {
            let chat_id = req_i64(params, "chat_id")?;
            let message_id = req_i64(params, "message_id")?;
            let revoke = params
                .get("revoke")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            host.run_cancellable(req_id, async move {
                client
                    .delete_message(chat_id, message_id, revoke)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "deleted": true, "chat_id": chat_id, "message_id": message_id }))
        }
        "react" => {
            let chat_id = req_i64(params, "chat_id")?;
            let message_id = req_i64(params, "message_id")?;
            let emoji = req_str(params, "emoji")?.to_string();
            let react_action = params
                .get("action")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .react_to_message(chat_id, message_id, emoji, react_action.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "markRead" => {
            let chat_id = req_i64(params, "chat_id")?;
            let max_id = req_i64(params, "max_id")?;
            let topic_id = params.get("topic_id").and_then(|v| v.as_i64());
            let res = host.run_cancellable(req_id, async move {
                client
                    .mark_chat_read(chat_id, max_id, topic_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "pollCreate" => {
            let chat_id = req_i64(params, "chat_id")?;
            let req: CreatePollHttpRequest = parse_req(params.get("poll").unwrap_or(params))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .create_poll(chat_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "pollVote" => {
            let poll_id = req_u64(params, "poll_id")?;
            let option_ids: Vec<u8> = params
                .get("option_ids")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as u8)).collect())
                .ok_or_else(|| HostError::invalid_arguments("Missing array 'option_ids'"))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .vote_poll(poll_id, option_ids)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "pollClose" => {
            let poll_id = req_u64(params, "poll_id")?;
            let res = host.run_cancellable(req_id, async move {
                client.close_poll(poll_id).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "sendTyping" => {
            let chat_id = req_i64(params, "chat_id")?;
            let action_val: TypingAction = params
                .get("action")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or(TypingAction::Typing);
            Ok(json!({ "sent": client.send_ws_typing(chat_id, action_val) }))
        }
        "archiveLocalBotReply" => {
            let chat_id = req_i64(params, "chat_id")?;
            let req: gpui_telegram_proto::user_vault::ArchiveLocalBotReplyRequest =
                parse_req(params.get("request").unwrap_or(params))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .archive_local_bot_reply(chat_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        _ => Err(HostError::unknown_command(&format!("messages.{action}"))),
    }
}

pub fn handle_sync(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "getDifference" => {
            let chat_id = req_i64(params, "chat_id")?;
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(100) as u32;
            if let Some(explicit_pts) = params.get("pts").and_then(|v| v.as_u64()) {
                host.with_fenced_session_read(&client, || {
                    let current = client.sync_coordinator.last_pts(ChatId(chat_id));
                    if explicit_pts > current {
                        client.sync_coordinator.set_pts(ChatId(chat_id), explicit_pts);
                    }
                    Ok(())
                })?;
            }
            let c = std::sync::Arc::clone(&client);
            let diff = host.run_cancellable(req_id, async move {
                c.sync_chat_difference(ChatId(chat_id), limit)
                    .await
                    .map_err(HostError::from)
            })?;
            let updated_pts = host.with_fenced_session_read(&client, || {
                let pts = client.sync_coordinator.last_pts(ChatId(chat_id));
                let _ = host.host_db.save_pts(chat_id, pts);
                Ok(pts)
            })?;
            let val = to_val(&diff)?;
            host.emit_event(
                "syncDifference",
                json!({
                    "chat_id": chat_id,
                    "pts": updated_pts,
                    "has_more": diff.has_more,
                    "updates_count": diff.updates.len(),
                }),
            );
            Ok(val)
        }
        "syncAllGaps" => {
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(100) as u32;
            let diffs = host.run_cancellable(req_id, async move {
                client.sync_all_gaps(limit).await.map_err(HostError::from)
            })?;
            to_val(diffs)
        }
        "lastPts" => {
            let chat_id = req_i64(params, "chat_id")?;
            let pts = client.sync_coordinator.last_pts(ChatId(chat_id));
            Ok(json!({ "chat_id": chat_id, "pts": pts }))
        }
        "setPts" => {
            let chat_id = req_i64(params, "chat_id")?;
            let pts = req_u64(params, "pts")?;
            client.sync_coordinator.set_pts(ChatId(chat_id), pts);
            host.host_db.save_pts(chat_id, pts)?;
            Ok(json!({ "chat_id": chat_id, "pts": pts }))
        }
        "pendingGaps" => {
            let chats: Vec<i64> = client
                .pending_gap_chats()
                .into_iter()
                .map(|c| c.0)
                .collect();
            Ok(json!({ "gap_chats": chats }))
        }
        "evaluatePtsUpdate" => {
            let update: PtsUpdate = parse_req(params)?;
            let outcome = client.sync_coordinator.evaluate_update(&update);
            let pts_now = client.sync_coordinator.last_pts(update.chat_id);
            let _ = host.host_db.save_pts(update.chat_id.0, pts_now);
            match outcome {
                SyncOutcome::Contiguous => Ok(json!({
                    "outcome": "contiguous",
                    "last_pts": pts_now
                })),
                SyncOutcome::Duplicate => Ok(json!({
                    "outcome": "duplicate",
                    "last_pts": pts_now
                })),
                SyncOutcome::GapDetected {
                    last_seen,
                    incoming,
                    missing_range,
                } => {
                    client.sync_coordinator.buffer_gap_update(update);
                    Ok(json!({
                        "outcome": "gap_detected",
                        "last_seen": last_seen,
                        "incoming": incoming,
                        "missing_start": *missing_range.start(),
                        "missing_end": *missing_range.end(),
                    }))
                }
            }
        }
        "applyWsFrame" => {
            let frame: WsServerFrame = parse_req(params)?;
            let events = client.process_ws_frame(frame);
            let mut mapped = Vec::with_capacity(events.len());
            for ev in events {
                let (ev_type, payload) = domain_event_to_host_event(ev);
                host.emit_event(&ev_type, payload.clone());
                mapped.push(json!({ "type": ev_type, "payload": payload }));
            }
            Ok(json!({ "events": mapped }))
        }
        _ => Err(HostError::unknown_command(&format!("sync.{action}"))),
    }
}

pub fn handle_outbox(
    host: &FabushiAppHost,
    _req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "enqueue" => {
            let req: RichSendMessageRequest = parse_req(params)?;
            let rec = host.with_fenced_session_read(&client, || Ok(client.enqueue_message(req)?))?;
            to_val(rec)
        }
        "list" => {
            let status_filter = params.get("status").and_then(|v| v.as_str()).and_then(|s| {
                match s.to_ascii_lowercase().as_str() {
                    "pending" => Some(OutboxStatus::Pending),
                    "in_flight" | "inflight" => Some(OutboxStatus::InFlight),
                    "delivered" => Some(OutboxStatus::Delivered),
                    "failed" => Some(OutboxStatus::Failed),
                    _ => None,
                }
            });
            let list = client.outbox.store().list_by_status(status_filter)?;
            to_val(list)
        }
        "get" => {
            let nonce = req_u64(params, "client_nonce")?;
            let rec = client.outbox.store().get(nonce)?;
            to_val(rec)
        }
        "pendingCount" => Ok(json!({
            "pending_count": client.outbox.store().pending_count(),
            "network_online": client.network_listener.is_online(),
        })),
        "retryNow" => {
            let now_ms = Utc::now().timestamp_millis();
            let reset = host.with_fenced_session_read(&client, || {
                let count = client
                    .outbox
                    .store()
                    .reset_all_pending_retry_times(now_ms)?;
                client.outbox.wake();
                Ok(count)
            })?;
            Ok(json!({ "reset_count": reset }))
        }
        "setNetworkState" => {
            let online = params
                .get("online")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| HostError::invalid_arguments("Missing bool 'online'"))?;
            let current_online = host.with_fenced_session_read(&client, || {
                client.network_listener.set_online(online);
                if online {
                    client.outbox.wake();
                }
                Ok(client.network_listener.is_online())
            })?;
            Ok(json!({ "online": current_online }))
        }
        "recoverInFlight" => {
            let recovered =
                host.with_fenced_session_read(&client, || Ok(client.outbox.recover_in_flight()?))?;
            Ok(json!({ "recovered": recovered }))
        }
        _ => Err(HostError::unknown_command(&format!("outbox.{action}"))),
    }
}

pub fn handle_e2ee(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "identity" => {
            let (x25519_pub_b64, ed25519_pub_b64, device_id) =
                host.with_fenced_session_read(&client, || {
                    let id = client
                        .e2ee
                        .identity()
                        .map_err(|e| HostError::crypto(e.to_string()))?;
                    Ok((
                        b64_encode(&id.x25519_pub),
                        b64_encode(&id.ed25519_pub),
                        client.device_id(),
                    ))
                })?;
            Ok(json!({
                "x25519_pub_b64": x25519_pub_b64,
                "ed25519_pub_b64": ed25519_pub_b64,
                "device_id": device_id,
            }))
        }
        "initAndPublishPrekeys" => {
            let batch_size = params
                .get("otpk_batch_size")
                .and_then(|v| v.as_u64())
                .unwrap_or(20) as usize;
            let res = host.run_cancellable(req_id, async move {
                client
                    .e2ee_init_and_publish_prekeys(batch_size)
                    .await
                    .map_err(|e| HostError::crypto(e.to_string()))
            })?;
            to_val(res)
        }
        "replenishPrekeys" => {
            let min = params.get("min_threshold").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let batch = params.get("batch_size").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let count = host.run_cancellable(req_id, async move {
                client
                    .e2ee
                    .replenish_prekeys_if_low(min, batch)
                    .await
                    .map_err(|e| HostError::crypto(e.to_string()))
            })?;
            Ok(json!({ "available_otpks": count }))
        }
        "establishSession" => {
            let bundle: DevicePreKeyBundle = parse_req(params)?;
            let dh_self_pub_b64 = host.with_fenced_session_read(&client, || {
                let sess = client
                    .e2ee
                    .establish_session_from_bundle(&bundle)
                    .map_err(|e| HostError::crypto(e.to_string()))?;
                Ok(b64_encode(&sess.dh_self_pub))
            })?;
            Ok(json!({
                "established": true,
                "device_id": bundle.device_id,
                "dh_self_pub_b64": dh_self_pub_b64,
            }))
        }
        "encryptDm" => {
            let recipient_user_id = req_u64(params, "recipient_user_id")?;
            let plaintext = req_raw_str(params, "plaintext")?.as_bytes().to_vec();
            let env = host.run_cancellable(req_id, async move {
                client
                    .e2ee_encrypt_dm(recipient_user_id, &plaintext)
                    .await
                    .map_err(|e| HostError::crypto(e.to_string()))
            })?;
            to_val(env)
        }
        "encryptGroup" => {
            let chat_id = req_i64(params, "chat_id")?;
            let member_user_ids: Vec<u64> = params
                .get("member_user_ids")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
                .ok_or_else(|| HostError::invalid_arguments("Missing array 'member_user_ids'"))?;
            let plaintext = req_raw_str(params, "plaintext")?.as_bytes().to_vec();
            let (dist, group) = host.run_cancellable(req_id, async move {
                client
                    .e2ee_encrypt_group(chat_id, &member_user_ids, &plaintext)
                    .await
                    .map_err(|e| HostError::crypto(e.to_string()))
            })?;
            Ok(json!({ "distribution": dist, "group_envelope": group }))
        }
        "sendEncryptedDm" => {
            let chat_id = req_i64(params, "chat_id")?;
            let recipient_user_id = req_u64(params, "recipient_user_id")?;
            let plaintext = req_raw_str(params, "plaintext")?.as_bytes().to_vec();
            let resp = host.run_cancellable(req_id, async move {
                client
                    .send_encrypted_dm(chat_id, recipient_user_id, &plaintext)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(resp)
        }
        "sendEncryptedGroup" => {
            let chat_id = req_i64(params, "chat_id")?;
            let member_user_ids: Vec<u64> = params
                .get("member_user_ids")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
                .ok_or_else(|| HostError::invalid_arguments("Missing array 'member_user_ids'"))?;
            let plaintext = req_raw_str(params, "plaintext")?.as_bytes().to_vec();
            let resp = host.run_cancellable(req_id, async move {
                client
                    .send_encrypted_group(chat_id, &member_user_ids, &plaintext)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(resp)
        }
        "decryptEnvelope" => {
            let env: EncryptedEnvelope = parse_req(params)?;
            let plain = host.with_fenced_session_read(&client, || {
                client
                    .e2ee_decrypt_envelope(&env)
                    .map_err(|e| HostError::crypto(e.to_string()))
            })?;
            Ok(json!({
                "plaintext_b64": BASE64_STANDARD.encode(&plain),
                "plaintext_utf8": String::from_utf8(plain).ok(),
            }))
        }
        "safetyFingerprint" => {
            if let (Some(a_b64), Some(b_b64)) = (
                params.get("local_identity_key_b64").and_then(|v| v.as_str()),
                params.get("remote_identity_key_b64").and_then(|v| v.as_str()),
            ) {
                let a = b64_decode_32(a_b64).map_err(|e| HostError::crypto(e.to_string()))?;
                let b = b64_decode_32(b_b64).map_err(|e| HostError::crypto(e.to_string()))?;
                let emojis = compute_safety_number_emojis(&a, &b);
                return Ok(json!({ "emojis": emojis }));
            }
            let remote_b64 = req_str(params, "remote_identity_key_b64")?;
            let emojis = host.with_fenced_session_read(&client, || {
                client
                    .e2ee_safety_fingerprint(remote_b64)
                    .map_err(|e| HostError::crypto(e.to_string()))
            })?;
            Ok(json!({ "emojis": emojis }))
        }
        // Offline / local Double Ratchet & Sender Key session primitives for end-to-end verification
        "ratchetInitPair" => {
            let alice_id = req_str(params, "alice_session_id")?.to_string();
            let bob_id = req_str(params, "bob_session_id")?.to_string();
            let ad = params
                .get("associated_data")
                .and_then(|v| v.as_str())
                .unwrap_or("fabushi-dr-v1")
                .as_bytes()
                .to_vec();
            let shared_secret =
                Zeroizing::new(random_32().map_err(|e| HostError::crypto(e.to_string()))?);
            let bob_dh_sec =
                Zeroizing::new(random_32().map_err(|e| HostError::crypto(e.to_string()))?);
            let bob_dh_pub = x25519_pub_from_secret(&bob_dh_sec);
            let alice = DoubleRatchetSession::init_alice(*shared_secret, bob_dh_pub, ad.clone())
                .map_err(|e| HostError::crypto(e.to_string()))?;
            let bob = DoubleRatchetSession::init_bob(*shared_secret, *bob_dh_sec, ad);
            let mut guard = host.local_dr_sessions.lock();
            guard.insert(alice_id.clone(), alice);
            guard.insert(bob_id.clone(), bob);
            Ok(json!({ "alice_session_id": alice_id, "bob_session_id": bob_id }))
        }
        "ratchetEncrypt" => {
            let session_id = req_str(params, "session_id")?;
            let plaintext = req_raw_str(params, "plaintext")?.as_bytes();
            let mut guard = host.local_dr_sessions.lock();
            let sess = guard
                .get_mut(session_id)
                .ok_or_else(|| HostError::not_found(format!("Ratchet session '{session_id}' not found")))?;
            let (header, ciphertext_b64) = sess
                .encrypt(plaintext)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            Ok(json!({ "header": header, "ciphertext_b64": ciphertext_b64 }))
        }
        "ratchetDecrypt" => {
            let session_id = req_str(params, "session_id")?;
            let header: RatchetHeaderDto =
                parse_req(params.get("header").ok_or_else(|| HostError::invalid_arguments("Missing 'header'"))?)?;
            let ciphertext_b64 = req_str(params, "ciphertext_b64")?;
            let mut guard = host.local_dr_sessions.lock();
            let sess = guard
                .get_mut(session_id)
                .ok_or_else(|| HostError::not_found(format!("Ratchet session '{session_id}' not found")))?;
            let plain = sess
                .decrypt(&header, ciphertext_b64)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            Ok(json!({
                "plaintext_utf8": String::from_utf8(plain.clone()).ok(),
                "plaintext_b64": BASE64_STANDARD.encode(&plain),
            }))
        }
        "senderKeyCreate" => {
            let key_id = req_str(params, "key_id")?.to_string();
            let chat_id = req_i64(params, "chat_id")?;
            let sender_user_id = req_u64(params, "sender_user_id")?;
            let sender_device_id = req_str(params, "sender_device_id")?.to_string();
            let (state, dist) = SenderKeyState::create_own(chat_id, sender_user_id, sender_device_id)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            host.local_sender_keys.lock().insert(key_id.clone(), state);
            Ok(json!({ "key_id": key_id, "distribution": dist }))
        }
        "senderKeyImportDistribution" => {
            let key_id = req_str(params, "key_id")?.to_string();
            let dist: SenderKeyDistributionMessage = parse_req(
                params
                    .get("distribution")
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'distribution'"))?,
            )?;
            let state = SenderKeyState::from_distribution(&dist)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            host.local_sender_keys.lock().insert(key_id.clone(), state);
            Ok(json!({ "key_id": key_id, "imported": true }))
        }
        "senderKeyEncrypt" => {
            let key_id = req_str(params, "key_id")?;
            let plaintext = req_raw_str(params, "plaintext")?.as_bytes();
            let mut guard = host.local_sender_keys.lock();
            let state = guard
                .get_mut(key_id)
                .ok_or_else(|| HostError::not_found(format!("SenderKey '{key_id}' not found")))?;
            let ct = state
                .encrypt_group_message(plaintext)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            to_val(ct)
        }
        "senderKeyDecrypt" => {
            let key_id = req_str(params, "key_id")?;
            let ct: gpui_telegram_proto::e2ee::GroupCiphertext = parse_req(
                params
                    .get("ciphertext")
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'ciphertext'"))?,
            )?;
            let mut guard = host.local_sender_keys.lock();
            let state = guard
                .get_mut(key_id)
                .ok_or_else(|| HostError::not_found(format!("SenderKey '{key_id}' not found")))?;
            let plain = state
                .decrypt_group_message(&ct)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            Ok(json!({
                "plaintext_utf8": String::from_utf8(plain.clone()).ok(),
                "plaintext_b64": BASE64_STANDARD.encode(&plain),
            }))
        }
        _ => Err(HostError::unknown_command(&format!("e2ee.{action}"))),
    }
}

pub fn handle_search(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "messages" => {
            let query = req_str(params, "query")?.to_string();
            let filter = params
                .get("filter")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64());
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let res = host.run_cancellable(req_id, async move {
                client
                    .search_messages(&query, filter.as_deref(), chat_id, limit)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "globalCatalog" => {
            let query = req_str(params, "query")?.to_string();
            let category = params
                .get("category")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .search_global_catalog(&query, category.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "indexMessage" => {
            let message_id = req_i64(params, "message_id")?;
            let chat_id = req_i64(params, "chat_id")?;
            let sender_id = params.get("sender_id").and_then(|v| v.as_u64()).unwrap_or(1);
            let timestamp_ms = params
                .get("timestamp_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| Utc::now().timestamp_millis());
            let body = req_raw_str(params, "body")?;
            host.host_db
                .index_message_fts(message_id, chat_id, sender_id, timestamp_ms, body)?;
            host.search_engine
                .index_message(message_id, chat_id, sender_id, timestamp_ms, body)?;
            Ok(json!({
                "indexed": true,
                "message_id": message_id,
                "tantivy_docs": host.search_engine.doc_count(),
                "fts5_enabled": host.host_db.fts5_enabled(),
            }))
        }
        "localMessages" | "query" => {
            let query = req_str(params, "query")?;
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64());
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let tantivy_hits = host.search_engine.search(query, chat_id, limit)?;
            if !tantivy_hits.is_empty() {
                return Ok(json!({
                    "engine": "tantivy",
                    "query": query,
                    "total": tantivy_hits.len(),
                    "hits": tantivy_hits,
                }));
            }
            let fallback_hits = host.host_db.search_fts5_or_cjk(query, chat_id, limit)?;
            let engine_used = fallback_hits
                .first()
                .map(|h| h.engine.clone())
                .unwrap_or_else(|| "tantivy".to_string());
            Ok(json!({
                "engine": engine_used,
                "query": query,
                "total": fallback_hits.len(),
                "hits": fallback_hits,
            }))
        }
        "rebuildLocalIndex" => {
            let all = host.host_db.list_all_indexed_messages()?;
            let count = host.search_engine.rebuild(&all)?;
            Ok(json!({ "rebuilt": true, "indexed_docs": count }))
        }
        "sessions" => {
            let query = req_str(params, "query")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let hits = host
                .hermes
                .store()
                .session_search(query, None, limit)
                .map_err(|e| HostError::storage(e.to_string()))?;
            to_val(hits)
        }
        _ => Err(HostError::unknown_command(&format!("search.{action}"))),
    }
}

pub fn handle_media(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "encodeWaveform" => {
            if let Some(bars_arr) = params.get("bars").and_then(|v| v.as_array()) {
                let bars: Vec<u8> = bars_arr
                    .iter()
                    .filter_map(|x| x.as_u64().map(|n| (n as u8).min(31)))
                    .collect();
                let encoded = encode_waveform_5bit(&bars);
                return Ok(json!({
                    "waveform_bytes": encoded,
                    "waveform_b64": BASE64_STANDARD.encode(&encoded),
                    "byte_count": encoded.len(),
                }));
            }
            let samples: Vec<f32> = params
                .get("samples")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect())
                .ok_or_else(|| {
                    HostError::invalid_arguments("Provide either 'bars' (0..31) or 'samples' (f32)")
                })?;
            let encoded = encode_waveform_from_floats(&samples);
            Ok(json!({
                "waveform_bytes": encoded,
                "waveform_b64": BASE64_STANDARD.encode(&encoded),
                "byte_count": encoded.len(),
            }))
        }
        "decodeWaveform" => {
            let bytes: Vec<u8> = if let Some(b64) = params.get("waveform_b64").and_then(|v| v.as_str()) {
                BASE64_STANDARD
                    .decode(b64.trim())
                    .map_err(|e| HostError::invalid_arguments(format!("Invalid waveform_b64: {e}")))?
            } else {
                params
                    .get("waveform_bytes")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as u8)).collect())
                    .ok_or_else(|| {
                        HostError::invalid_arguments("Provide 'waveform_bytes' or 'waveform_b64'")
                    })?
            };
            let target_bars = params
                .get("target_bars")
                .and_then(|v| v.as_u64())
                .unwrap_or(TELEGRAM_WAVEFORM_BARS as u64) as usize;
            let bars = decode_waveform_5bit(&bytes);
            let floats = decode_waveform_to_floats(&bytes, target_bars);
            Ok(json!({
                "bars": bars,
                "samples": floats,
                "sample_count": floats.len(),
            }))
        }
        "waveformFromPcm" => {
            let pcm: Vec<i16> = params
                .get("pcm_i16")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_i64().map(|n| n as i16)).collect())
                .ok_or_else(|| HostError::invalid_arguments("Missing array 'pcm_i16'"))?;
            let chunk_size = (pcm.len() + TELEGRAM_WAVEFORM_BARS - 1) / TELEGRAM_WAVEFORM_BARS;
            let mut peaks = Vec::with_capacity(TELEGRAM_WAVEFORM_BARS);
            for chunk in pcm.chunks(chunk_size.max(1)) {
                let max_val = chunk.iter().map(|&s| s.unsigned_abs()).max().unwrap_or(0);
                peaks.push(max_val);
            }
            let normalized = normalize_pcm_peaks(&peaks);
            let encoded = encode_waveform_5bit(&normalized);
            Ok(json!({
                "bars": normalized,
                "waveform_bytes": encoded,
                "waveform_b64": BASE64_STANDARD.encode(&encoded),
                "byte_count": encoded.len(),
            }))
        }
        "casPut" => {
            let file_id = req_str(params, "file_id")?;
            let bytes = if let Some(b64) = params.get("data_b64").and_then(|v| v.as_str()) {
                BASE64_STANDARD
                    .decode(b64.trim())
                    .map_err(|e| HostError::invalid_arguments(format!("Invalid data_b64: {e}")))?
            } else {
                return Err(HostError::invalid_arguments("Missing base64 'data_b64'"));
            };
            let mime = params.get("mime_type").and_then(|v| v.as_str());
            let path = client.local_store.put_media_cas(file_id, &bytes, mime)?;
            let sha256 = {
                use sha2::{Digest, Sha256};
                hex::encode(Sha256::digest(&bytes))
            };
            Ok(json!({
                "file_id": file_id,
                "size_bytes": bytes.len(),
                "sha256": sha256,
                "local_path": path.map(|p| p.to_string_lossy().into_owned()),
            }))
        }
        "casGet" => {
            let file_id = req_str(params, "file_id")?;
            let bytes = client
                .local_store
                .get_media_cas(file_id)
                .ok_or_else(|| HostError::not_found(format!("Media '{file_id}' not in local CAS")))?;
            Ok(json!({
                "file_id": file_id,
                "size_bytes": bytes.len(),
                "data_b64": BASE64_STANDARD.encode(&bytes),
            }))
        }
        "localCasPath" => {
            let file_id = req_str(params, "file_id")?;
            let path = client.local_media_path(file_id);
            Ok(json!({
                "file_id": file_id,
                "local_path": path.map(|p| p.to_string_lossy().into_owned()),
            }))
        }
        "casStats" => {
            let metrics = compute_storage_metrics(&host.storage_dir);
            Ok(json!({
                "media_cas_bytes": metrics.media_cas_bytes,
                "media_cas_files": metrics.media_cas_files,
                "total_bytes": metrics.total_bytes,
            }))
        }
        "casPurge" => {
            let (freed_bytes, removed_files) = purge_media_cas_dir(&host.storage_dir)?;
            Ok(json!({ "freed_bytes": freed_bytes, "removed_files": removed_files }))
        }
        "recommendPartSize" => {
            let file_size = req_u64(params, "file_size")?;
            Ok(json!({ "part_size": recommend_part_size(file_size) }))
        }
        "chunkedSessionCreate" => {
            let file_id = req_str(params, "file_id")?.to_string();
            let total_size = req_u64(params, "total_size")?;
            let part_size = params
                .get("part_size")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .unwrap_or_else(|| recommend_part_size(total_size));
            let mut sess = ChunkedUploadSession::new(file_id.clone(), total_size, part_size)?;
            if let Some(sha) = params.get("expected_sha256").and_then(|v| v.as_str()) {
                sess = sess.with_expected_sha256(sha);
            }
            let total_parts = sess.total_parts;
            host.chunked_sessions.lock().insert(file_id.clone(), sess);
            Ok(json!({
                "file_id": file_id,
                "total_size": total_size,
                "part_size": part_size,
                "total_parts": total_parts,
            }))
        }
        "chunkedSessionReceivePart" => {
            let file_id = req_str(params, "file_id")?;
            let part_index = params
                .get("part_index")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| HostError::invalid_arguments("Missing u32 'part_index'"))?
                as u32;
            let data = BASE64_STANDARD
                .decode(req_str(params, "data_b64")?)
                .map_err(|e| HostError::invalid_arguments(format!("Invalid data_b64: {e}")))?;
            let mut guard = host.chunked_sessions.lock();
            let sess = guard
                .get_mut(file_id)
                .ok_or_else(|| HostError::not_found(format!("Chunked session '{file_id}' not found")))?;
            sess.receive_part(part_index, &data)?;
            let complete = sess.is_complete();
            let missing = sess.missing_parts();
            Ok(json!({
                "file_id": file_id,
                "part_index": part_index,
                "is_complete": complete,
                "missing_parts": missing,
            }))
        }
        "chunkedSessionFinalize" => {
            let file_id = req_str(params, "file_id")?;
            let mut guard = host.chunked_sessions.lock();
            let sess = guard
                .get(file_id)
                .ok_or_else(|| HostError::not_found(format!("Chunked session '{file_id}' not found")))?;
            let verified = sess.finalize_and_verify()?;
            if !verified {
                return Err(HostError::crypto(
                    "Chunked upload incomplete or SHA-256 verification failed",
                ));
            }
            let sha256 = sess.compute_sha256()?;
            let assembled = sess.assemble_payload()?;
            let mime = params.get("mime_type").and_then(|v| v.as_str());
            let path = client.local_store.put_media_cas(file_id, &assembled, mime)?;
            guard.remove(file_id);
            Ok(json!({
                "file_id": file_id,
                "verified": true,
                "sha256": sha256,
                "size_bytes": assembled.len(),
                "local_path": path.map(|p| p.to_string_lossy().into_owned()),
            }))
        }
        "upload" => {
            let filename = req_str(params, "filename")?.to_string();
            let mime_type = params
                .get("mime_type")
                .and_then(|v| v.as_str())
                .unwrap_or("application/octet-stream")
                .to_string();
            let data = BASE64_STANDARD
                .decode(req_str(params, "data_b64")?)
                .map_err(|e| HostError::invalid_arguments(format!("Invalid data_b64: {e}")))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .upload_media(&filename, &mime_type, data)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "uploadResumable" => {
            let file_path = req_str(params, "file_path")?.to_string();
            let mime_type = params
                .get("mime_type")
                .and_then(|v| v.as_str())
                .unwrap_or("application/octet-stream")
                .to_string();
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64());
            let transfer_id = params
                .get("transfer_id")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .upload_file_resumable(&file_path, &mime_type, chat_id, transfer_id.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "download" => {
            let file_id = req_str(params, "file_id")?.to_string();
            let bytes = host.run_cancellable(req_id, async move {
                client
                    .get_media_bytes(&file_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({
                "size_bytes": bytes.len(),
                "data_b64": BASE64_STANDARD.encode(&bytes),
            }))
        }
        "downloadRange" => {
            let file_id = req_str(params, "file_id")?.to_string();
            let start = req_u64(params, "start")?;
            let end_inclusive = params.get("end_inclusive").and_then(|v| v.as_u64());
            let (status, bytes, content_range) = host.run_cancellable(req_id, async move {
                client
                    .download_media_range(&file_id, start, end_inclusive)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({
                "status": status,
                "size_bytes": bytes.len(),
                "content_range": content_range,
                "data_b64": BASE64_STANDARD.encode(&bytes),
            }))
        }
        _ => Err(HostError::unknown_command(&format!("media.{action}"))),
    }
}

pub fn handle_stories(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "list" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_stories().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "create" => {
            let media_id = params
                .get("media_id")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let media_type = params
                .get("media_type")
                .and_then(|v| v.as_str())
                .unwrap_or("photo")
                .to_string();
            let caption = params
                .get("caption")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let privacy = params
                .get("privacy")
                .and_then(|v| v.as_str())
                .unwrap_or("everyone")
                .to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .create_story(
                        media_id.as_deref(),
                        &media_type,
                        caption.as_deref(),
                        &privacy,
                    )
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "delete" => {
            let story_id = req_str(params, "story_id")?.to_string();
            host.run_cancellable(req_id, async move {
                client.delete_story(&story_id).await.map_err(HostError::from)
            })?;
            Ok(json!({ "deleted": true }))
        }
        _ => Err(HostError::unknown_command(&format!("stories.{action}"))),
    }
}

pub fn handle_stickers(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "listSets" => {
            let query = params
                .get("query")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_sticker_sets(query.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "installSet" => {
            let set_id = req_str(params, "set_id")?.to_string();
            let sid_move = set_id.clone();
            let installed = params
                .get("installed")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            host.run_cancellable(req_id, async move {
                client
                    .install_sticker_set(&sid_move, installed)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "set_id": set_id, "installed": installed }))
        }
        "searchGifs" => {
            let query = req_str(params, "query")?.to_string();
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let res = host.run_cancellable(req_id, async move {
                client
                    .search_gifs(&query, limit)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        _ => Err(HostError::unknown_command(&format!("stickers.{action}"))),
    }
}
