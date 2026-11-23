use std::collections::HashSet;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mahayana_app_host::{
    fabushi_host_create, fabushi_host_destroy, fabushi_host_dispatch, fabushi_host_free_string,
    telegram_app_host_dispatch, telegram_app_host_free_string,
    telegram_bridge_register_event_callback,
};
use serde_json::{json, Value};
use tempfile::tempdir;

unsafe fn dispatch(handle: *mut c_void, payload: &Value) -> Value {
    let req_c = CString::new(payload.to_string()).unwrap();
    let ptr = unsafe { fabushi_host_dispatch(handle, req_c.as_ptr()) };
    assert!(!ptr.is_null());
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
    unsafe { fabushi_host_free_string(ptr) };
    serde_json::from_str(&text).unwrap()
}

#[test]
fn test_100_thread_concurrent_nonce_dedup_and_restart_persistence() {
    let dir = tempdir().unwrap();
    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();

    struct EventCounter {
        new_msg_events: AtomicUsize,
    }
    let counter = Arc::new(EventCounter {
        new_msg_events: AtomicUsize::new(0),
    });

    unsafe extern "C" fn on_event(
        event_type: *const c_char,
        _payload: *const c_char,
        user_ctx: *mut c_void,
    ) {
        if !event_type.is_null() && !user_ctx.is_null() {
            if unsafe { CStr::from_ptr(event_type) }.to_string_lossy() == "newMessage" {
                let c = unsafe { &*(user_ctx as *const EventCounter) };
                c.new_msg_events.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    let winning_id: i64;
    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        let raw_ctx = Arc::into_raw(Arc::clone(&counter)) as *mut c_void;
        telegram_bridge_register_event_callback(host, Some(on_event), raw_ctx);

        let thread_count = 100usize;
        let barrier = Arc::new(Barrier::new(thread_count));
        let host_addr = host as usize;
        let results = Arc::new(Mutex::new(Vec::with_capacity(thread_count)));
        let start = Instant::now();

        let mut handles = Vec::with_capacity(thread_count);
        for idx in 0..thread_count {
            let b = Arc::clone(&barrier);
            let res = Arc::clone(&results);
            handles.push(thread::spawn(move || {
                let req = CString::new(
                    json!({
                        "command": "telegram.sendMessage",
                        "params": {
                            "chat_id": 2020,
                            "sender_id": 88,
                            "client_nonce": 55555,
                            "text": format!("Concurrent message {idx}"),
                            "local_only": true
                        }
                    })
                    .to_string(),
                )
                .unwrap();
                b.wait();
                let ptr = telegram_app_host_dispatch(host_addr as *mut c_void, req.as_ptr());
                assert!(!ptr.is_null());
                let s = CStr::from_ptr(ptr).to_str().unwrap().to_string();
                telegram_app_host_free_string(ptr);
                res.lock().unwrap().push(s);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "100-thread nonce dedup exceeded 5s CI bound: {elapsed:?}"
        );

        let responses = results.lock().unwrap().clone();
        assert_eq!(responses.len(), thread_count);

        let mut new_count = 0usize;
        let mut dup_count = 0usize;
        let mut ids = HashSet::new();
        for r in &responses {
            let v: Value = serde_json::from_str(r).unwrap();
            assert_eq!(v["ok"], true, "Unexpected error: {r}");
            if v["result"]["duplicate"].as_bool().unwrap_or(false) {
                dup_count += 1;
            } else {
                new_count += 1;
            }
            ids.insert(v["result"]["id"].as_i64().unwrap());
        }

        assert_eq!(new_count, 1);
        assert_eq!(dup_count, 99);
        assert_eq!(ids.len(), 1);
        assert_eq!(counter.new_msg_events.load(Ordering::SeqCst), 1);
        winning_id = *ids.iter().next().unwrap();

        let stored = dispatch(
            host,
            &json!({"method": "messages.localList", "params": {"chat_id": 2020}}),
        );
        let arr = stored["result"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], winning_id);

        fabushi_host_destroy(host);
        drop(Arc::from_raw(raw_ctx as *const EventCounter));

        // Restart persistence: destroy & reopen on exact same storage_dir
        let host2 = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host2.is_null());

        let replay = dispatch(
            host2,
            &json!({
                "method": "messages.send",
                "params": {
                    "chat_id": 2020,
                    "sender_id": 88,
                    "client_nonce": 55555,
                    "text": "Replayed after restart",
                    "local_only": true
                }
            }),
        );
        assert_eq!(replay["ok"], true);
        assert_eq!(replay["result"]["duplicate"], true);
        assert_eq!(replay["result"]["id"], winning_id);

        // Different sender_id with same nonce creates a fresh message
        let diff_sender = dispatch(
            host2,
            &json!({
                "method": "messages.send",
                "params": {
                    "chat_id": 2020,
                    "sender_id": 99,
                    "client_nonce": 55555,
                    "text": "Different sender",
                    "local_only": true
                }
            }),
        );
        assert_eq!(diff_sender["ok"], true);
        assert_ne!(diff_sender["result"]["duplicate"], true);
        assert_ne!(diff_sender["result"]["id"].as_i64().unwrap(), winning_id);

        fabushi_host_destroy(host2);
    }
}

#[test]
fn test_sqlite_wal_outbox_settings_passcode_proxy_and_persistence() {
    let dir = tempdir().unwrap();
    let cfg_c = CString::new(
        json!({ "app_data_dir": dir.path().to_string_lossy() }).to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        // Outbox durable enqueue while offline
        let off = dispatch(
            host,
            &json!({"method": "outbox.setNetworkState", "params": {"online": false}}),
        );
        assert_eq!(off["ok"], true);
        assert_eq!(off["result"]["online"], false);

        let enq = dispatch(
            host,
            &json!({
                "method": "outbox.enqueue",
                "params": {"chat_id": 500, "text": "Offline durable message", "client_nonce": 9001}
            }),
        );
        assert_eq!(enq["ok"], true);

        let cnt = dispatch(host, &json!({"method": "outbox.pendingCount"}));
        assert_eq!(cnt["result"]["pending_count"], 1);

        // Settings KV, Passcode, WAL checkpoint, real storage metrics, and Proxy CRUD
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "settings.localSet", "params": {"key": "theme", "value": "Night"}})
            )["ok"],
            true
        );
        let set_pw = dispatch(
            host,
            &json!({"method": "settings.passcodeSet", "params": {"passcode": " 246810 "}}),
        );
        assert_eq!(set_pw["ok"], true);
        assert_eq!(set_pw["result"]["algorithm"], "argon2id");
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "settings.passcodeVerify", "params": {"passcode": " 246810 "}})
            )["result"]["valid"],
            true
        );
        // Trimmed version MUST NOT match (verifies req_raw_str preserves leading/trailing whitespace!)
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "settings.passcodeVerify", "params": {"passcode": "246810"}})
            )["result"]["valid"],
            false
        );
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "settings.passcodeVerify", "params": {"passcode": "000000"}})
            )["result"]["valid"],
            false
        );

        // Verify stored passcode_hash in host_state.db is genuinely Argon2id (never raw SHA-256!)
        let conn = rusqlite::Connection::open(dir.path().join("host_state.db")).unwrap();
        let stored_hash: String = conn
            .query_row(
                "SELECT value FROM host_meta WHERE key = 'passcode_hash'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            stored_hash.starts_with("argon2id$m=65536,t=3,p=1$"),
            "Expected production Argon2id passcode hash, got: {stored_hash}"
        );
        drop(conn);

        let proxy_up = dispatch(
            host,
            &json!({
                "method": "proxy.upsert",
                "params": {"id": "socks-1", "url": "socks5://127.0.0.1:1080", "activate": true}
            }),
        );
        assert_eq!(proxy_up["ok"], true, "proxy upsert failed: {proxy_up}");

        let ckpt = dispatch(host, &json!({"method": "session.checkpointWal"}));
        assert_eq!(ckpt["ok"], true);
        assert_eq!(ckpt["result"]["checkpointed"], true);

        let metrics = dispatch(host, &json!({"method": "settings.storageMetrics"}));
        assert_eq!(metrics["ok"], true);
        assert!(metrics["result"]["total_bytes"].as_u64().unwrap() > 0);
        assert!(metrics["result"]["sqlite_bytes"].as_u64().unwrap() > 0);

        fabushi_host_destroy(host);

        // Reopen on same storage_dir and verify persistence!
        let host2 = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host2.is_null());

        let cnt2 = dispatch(host2, &json!({"method": "outbox.pendingCount"}));
        assert_eq!(cnt2["result"]["pending_count"], 1);

        let theme = dispatch(
            host2,
            &json!({"method": "settings.localGet", "params": {"key": "theme"}}),
        );
        assert_eq!(theme["result"]["value"], "Night");

        let active_proxy = dispatch(host2, &json!({"method": "proxy.getActive"}));
        assert_eq!(active_proxy["result"]["active_proxy_id"], "socks-1");

        let verify_reopen = dispatch(
            host2,
            &json!({"method": "settings.passcodeVerify", "params": {"passcode": " 246810 "}}),
        );
        assert_eq!(verify_reopen["result"]["valid"], true);

        fabushi_host_destroy(host2);
    }
}

#[test]
fn test_real_tantivy_full_text_search_ranking_persistence_and_cjk_fallback() {
    let dir = tempdir().unwrap();
    let cfg_c = CString::new(
        json!({ "app_data_dir": dir.path().to_string_lossy() }).to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        // Index 4 messages (English BM25 relevance contrast + chat scoping + Chinese CJK)
        let docs = [
            (1001, 10, "rust tantivy search engine architecture rust tantivy bm25"),
            (1002, 10, "intro to rust programming language"),
            (1003, 20, "rust tantivy isolated in another chat"),
            (1004, 10, "欢迎来到全球法布施禅堂一起共修金刚经"),
        ];
        for (mid, cid, body) in docs {
            let res = dispatch(
                host,
                &json!({
                    "method": "search.indexMessage",
                    "params": {
                        "message_id": mid,
                        "chat_id": cid,
                        "sender_id": 7,
                        "body": body
                    }
                }),
            );
            assert_eq!(res["ok"], true, "indexMessage failed: {res}");
        }

        // 1. Tantivy BM25 search + relevance ranking (doc 1001 mentions rust & tantivy twice)
        let hit_res = dispatch(
            host,
            &json!({
                "method": "search.localMessages",
                "params": {"query": "rust tantivy", "chat_id": 10, "limit": 10}
            }),
        );
        assert_eq!(hit_res["ok"], true);
        assert_eq!(hit_res["result"]["engine"], "tantivy");
        let hits = hit_res["result"]["hits"].as_array().unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0]["message_id"], 1001);
        assert!(hits[0]["score"].as_f64().unwrap() > 0.0);
        // Chat filter excludes chat 20 doc 1003
        assert!(hits.iter().all(|h| h["chat_id"] == 10));

        // 2. Unsegmented CJK query hits bundled SQLite FTS5 / CJK fallback seamlessly
        let cjk = dispatch(
            host,
            &json!({
                "method": "search.localMessages",
                "params": {"query": "法布施禅堂", "chat_id": 10}
            }),
        );
        assert_eq!(cjk["ok"], true);
        let cjk_hits = cjk["result"]["hits"].as_array().unwrap();
        assert_eq!(cjk_hits.len(), 1);
        assert_eq!(cjk_hits[0]["message_id"], 1004);
        assert!(
            cjk["result"]["engine"]
                .as_str()
                .unwrap()
                .contains("sqlite_fts5"),
            "Expected sqlite_fts5 fallback engine, got {:?}",
            cjk["result"]["engine"]
        );

        fabushi_host_destroy(host);

        // 3. Reopen host -> Tantivy index persisted cleanly on disk in <storage_dir>/tantivy_index!
        let host2 = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host2.is_null());
        let reopened = dispatch(
            host2,
            &json!({
                "method": "search.localMessages",
                "params": {"query": "bm25"}
            }),
        );
        assert_eq!(reopened["ok"], true);
        assert_eq!(reopened["result"]["engine"], "tantivy");
        let r_hits = reopened["result"]["hits"].as_array().unwrap();
        assert_eq!(r_hits.len(), 1);
        assert_eq!(r_hits[0]["message_id"], 1001);

        let rebuild = dispatch(host2, &json!({"method": "search.rebuildLocalIndex"}));
        assert_eq!(rebuild["ok"], true);
        assert_eq!(rebuild["result"]["indexed_docs"], 4);

        // Trailing backslash in search query must not trigger SQLite LIKE ESCAPE error (NB-06)
        let bs = dispatch(
            host2,
            &json!({
                "method": "search.localMessages",
                "params": {"query": "path\\"}
            }),
        );
        assert_eq!(bs["ok"], true, "Backslash query must not error: {bs}");

        fabushi_host_destroy(host2);
    }
}
