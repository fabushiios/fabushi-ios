//! Tests for Telegram-style startup cache persistence (`LocalStore` + `SqliteClientCache` +
//! directory-scoped `ClientConfig` session storage) and strict account/logout isolation.

use gpui_telegram_client::net::hermes_client::{HermesRichChatDto, HermesRichMessageDto};
use gpui_telegram_client::{ChatSummary, ClientConfig, HistoricalMessage, LocalStore, StoredSession};
use gpui_telegram_proto::contracts::auth::ContactDto;

fn sample_chat(chat_id: u64, title: &str, last_message: &str, unread: u32) -> ChatSummary {
    ChatSummary {
        chat_id,
        title: title.to_string(),
        chat_type: 0,
        last_message: Some(last_message.to_string()),
        unread_count: unread,
        is_group: Some(false),
        is_bot: Some(false),
        avatar_color: Some(0x2AABEE),
        updated_at: Some(1_730_000_000_000),
    }
}

fn sample_hist_msg(id: u64, sender_id: u64, text: &str, is_outgoing: bool) -> HistoricalMessage {
    HistoricalMessage {
        message_id: id,
        sender_id,
        text: text.to_string(),
        content: None,
        date: Some(1_730_000_000_000 + (id as i64) * 1000),
        reply_to: None,
        is_outgoing: Some(is_outgoing),
        reply_to_info: None,
        reactions: Vec::new(),
        ttl_seconds: None,
        expires_at_ms: None,
        send_as_peer_id: None,
        reactions_authoritative: false,
    }
}

#[test]
fn sqlite_persists_dialogs_rich_chats_histories_contacts_and_meta_across_reopen() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    {
        let store = LocalStore::open_with_dir(dir.path()).expect("open store #1");
        store.bind_account_scope("https://api.fabushi.example", 42);

        store.replace_cached_dialog_summaries(&[
            sample_chat(101, "Alice Chen", "See you soon", 2),
            sample_chat(202, "Engineering", "Build is green", 0),
        ]);

        let rich_101: HermesRichChatDto = serde_json::from_value(serde_json::json!({
            "chat_id": 101,
            "title": "Alice Chen",
            "is_pinned": true,
            "is_muted": false,
            "is_archived": false,
            "peer_user_id": 888,
            "peer_display_name": "Alice Chen",
            "peer_username": "alice_c"
        }))
        .expect("rich chat dto");
        store.replace_cached_rich_chats(&[rich_101]);

        // Seed 115 messages -> store must cap persisted snapshot to latest 100 messages.
        let bulk_msgs: Vec<HistoricalMessage> = (1..=115)
            .map(|i| sample_hist_msg(i, if i % 2 == 0 { 42 } else { 888 }, &format!("msg #{i}"), i % 2 == 0))
            .collect();
        let rich_msgs: Vec<HermesRichMessageDto> = vec![
            serde_json::from_value(serde_json::json!({
                "message_id": 10,
                "chat_id": 101,
                "sender_id": 888,
                "sender_name": "Old Dropped"
            }))
            .unwrap(),
            serde_json::from_value(serde_json::json!({
                "message_id": 115,
                "chat_id": 101,
                "sender_id": 888,
                "sender_name": "Alice Chen",
                "sender_initials": "AC"
            }))
            .unwrap(),
        ];
        store.replace_cached_chat_history(101, &bulk_msgs, &rich_msgs);

        // Live write-through: append, edit, delete, preview bump
        store.upsert_cached_historical_message(101, sample_hist_msg(116, 42, "draft live", true));
        store.update_cached_historical_message_text(101, 116, "edited live".to_string());
        store.delete_cached_historical_messages(101, &[114]);
        store.update_cached_dialog_preview(202, "Hotfix merged!".to_string(), 1_730_000_500_000, Some(1));

        let contact: ContactDto = serde_json::from_value(serde_json::json!({
            "user_id": 888,
            "first_name": "Alice",
            "last_name": "Chen",
            "username": "alice_c",
            "phone": "+10000000000",
            "is_bot": false,
            "chat_id": 101
        }))
        .expect("contact dto");
        store.replace_cached_contacts(&[contact]);

        store.set_last_active_chat_id(Some(101));
        store.set_cached_user_display_name(Some("Bob Founder".to_string()));
    }

    // Reopen from SQLite on disk and verify Frame-0 hydration fidelity
    let reopened = LocalStore::open_with_dir(dir.path()).expect("open store #2");
    let dialogs = reopened.get_cached_dialog_summaries();
    assert_eq!(dialogs.len(), 2);
    // update_cached_dialog_preview promoted chat 202 to index 0 with unread +1
    assert_eq!(dialogs[0].chat_id, 202);
    assert_eq!(dialogs[0].last_message.as_deref(), Some("Hotfix merged!"));
    assert_eq!(dialogs[0].unread_count, 1);
    assert_eq!(dialogs[1].chat_id, 101);

    let rich = reopened.get_cached_rich_chats();
    assert!(rich.get(&101).is_some_and(|r| r.is_pinned && r.peer_user_id == Some(888)));

    let hist = reopened
        .get_cached_chat_history(101)
        .expect("chat 101 history persisted");
    assert_eq!(hist.messages.len(), 99, "100 capped -> +1 push capped to 100 -> -1 delete = 99");
    assert!(!hist.messages.iter().any(|m| m.message_id == 114));
    assert_eq!(
        hist.messages.last().map(|m| (m.message_id, m.text.as_str())),
        Some((116, "edited live"))
    );
    // Rich metadata for dropped message #10 was pruned; #115 survived
    assert_eq!(hist.rich_messages.len(), 1);
    assert_eq!(hist.rich_messages[0].message_id, 115);

    let contacts = reopened.get_cached_contacts();
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].user_id, 888);

    assert_eq!(reopened.get_last_active_chat_id(), Some(101));
    assert_eq!(
        reopened.get_cached_user_display_name().as_deref(),
        Some("Bob Founder")
    );

    // Account switch MUST wipe every cached table and metadata key!
    reopened.bind_account_scope("https://api.fabushi.example", 99);
    assert!(reopened.get_cached_dialog_summaries().is_empty());
    assert!(reopened.get_cached_rich_chats().is_empty());
    assert!(reopened.list_cached_chat_histories().is_empty());
    assert!(reopened.get_cached_contacts().is_empty());
    assert_eq!(reopened.get_last_active_chat_id(), None);
    assert_eq!(reopened.get_cached_user_display_name(), None);

    // Confirm wipe persisted to SQLite as well
    let after_switch = LocalStore::open_with_dir(dir.path()).expect("open store #3");
    assert!(after_switch.get_cached_dialog_summaries().is_empty());
    assert!(after_switch.list_cached_chat_histories().is_empty());
    assert_eq!(after_switch.get_last_active_chat_id(), None);
}

#[test]
fn config_session_in_dir_round_trip_and_server_url_matching() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let sess = StoredSession {
        server_url: "https://chat.fabushi.ai/".to_string(),
        user_id: 77,
        username: "founder".to_string(),
        token: "tok_secret_77".to_string(),
        session_id: "sid_77".to_string(),
        created_at_ms: 1_730_000_000_000,
    };

    ClientConfig::save_session_in_dir(dir.path(), &sess).expect("save session in dir");
    let loaded = ClientConfig::load_session_in_dir(dir.path()).expect("load session in dir");
    assert_eq!(loaded.user_id, 77);
    assert_eq!(loaded.username, "founder");
    assert_eq!(loaded.token, "tok_secret_77");

    assert!(ClientConfig::server_urls_match("https://chat.fabushi.ai/", "https://chat.fabushi.ai"));
    assert!(ClientConfig::server_urls_match("HTTPS://CHAT.FABUSHI.AI", "https://chat.fabushi.ai/"));
    assert!(ClientConfig::server_urls_match("", "https://chat.fabushi.ai"));
    assert!(!ClientConfig::server_urls_match("http://127.0.0.1:8080", "http://127.0.0.1:9090"));

    ClientConfig::clear_session_in_dir(dir.path());
    assert!(ClientConfig::load_session_in_dir(dir.path()).is_none());
}
