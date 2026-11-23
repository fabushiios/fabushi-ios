use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use axum::routing::get;
use axum::Router;
use mahayana_app_host::{
    compat::{mahayana_cloud_agent_trace_rows, mahayana_spreadsheet_parse_file},
    fabushi_host_cancel, fabushi_host_create, fabushi_host_destroy, fabushi_host_dispatch,
    fabushi_host_free_string, fabushi_host_last_error, fabushi_host_poll_events,
    mahayana_app_host_create, mahayana_app_host_create_test,
    mahayana_app_host_create_with_storage_passphrase, mahayana_app_host_destroy,
    mahayana_app_host_dispatch, mahayana_app_host_dispatch_with_handle,
    mahayana_app_host_free_string, mahayana_app_host_last_error, telegram_app_host_create,
    telegram_app_host_create_test, telegram_app_host_destroy, telegram_app_host_dispatch,
    telegram_app_host_dispatch_with_handle, telegram_app_host_free_string,
    telegram_app_host_last_error, telegram_bridge_register_event_callback,
    telegram_proto_max_packet_size, telegram_replay_window_check_and_add,
    telegram_replay_window_create, telegram_replay_window_destroy,
};
use serde_json::{json, Value};
use tempfile::tempdir;

unsafe fn call_dispatch(handle: *mut c_void, payload: &Value) -> Value {
    let req_c = CString::new(payload.to_string()).expect("valid JSON CString");
    let ptr = unsafe { fabushi_host_dispatch(handle, req_c.as_ptr()) };
    assert!(!ptr.is_null(), "dispatch must never return null");
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("valid UTF-8 response")
        .to_string();
    unsafe { fabushi_host_free_string(ptr) };
    serde_json::from_str(&text).expect("response must be valid JSON")
}

#[test]
fn test_c_abi_lifecycle_aliases_replay_window_and_null_callback_unregister() {
    let dir = tempdir().expect("tempdir");
    let cfg = json!({
        "app_data_dir": dir.path().to_string_lossy(),
        "test_mode": false
    });
    let cfg_c = CString::new(cfg.to_string()).unwrap();

    struct CallbackCounter {
        count: AtomicUsize,
    }
    let counter = Arc::new(CallbackCounter {
        count: AtomicUsize::new(0),
    });

    unsafe extern "C" fn on_event(
        event_type: *const c_char,
        _payload: *const c_char,
        ctx: *mut c_void,
    ) {
        if !event_type.is_null() && !ctx.is_null() {
            let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
            if ev == "newMessage" {
                let c = unsafe { &*(ctx as *const CallbackCounter) };
                c.count.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    unsafe {
        // 1. Canonical fabushi_host_* lifecycle
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null(), "fabushi_host_create failed");

        // Register callback -> emit -> unregister via None (NULL) without UB -> emit again
        let raw_ctx = Arc::into_raw(Arc::clone(&counter)) as *mut c_void;
        telegram_bridge_register_event_callback(host, Some(on_event), raw_ctx);

        let send1 = call_dispatch(
            host,
            &json!({
                "id": "req-1",
                "method": "messages.send",
                "params": {"chat_id": 101, "text": "hello callback", "local_only": true}
            }),
        );
        assert_eq!(send1["ok"], true);
        assert_eq!(send1["id"], "req-1");
        assert_eq!(counter.count.load(Ordering::SeqCst), 1);

        // Unregister by passing None (C NULL) — must NOT cause UB or crash
        telegram_bridge_register_event_callback(host, None, std::ptr::null_mut());
        let send2 = call_dispatch(
            host,
            &json!({
                "id": "req-2",
                "method": "messages.send",
                "params": {"chat_id": 101, "text": "after unregister", "local_only": true}
            }),
        );
        assert_eq!(send2["ok"], true);
        assert_eq!(
            counter.count.load(Ordering::SeqCst),
            1,
            "Callback must not fire after NULL unregister"
        );

        // Canonical fabushi_host_poll_events should still have both events
        let poll_ptr = fabushi_host_poll_events(host, 16, 100);
        let poll_val: Value =
            serde_json::from_str(CStr::from_ptr(poll_ptr).to_str().unwrap()).unwrap();
        fabushi_host_free_string(poll_ptr);
        assert_eq!(poll_val["ok"], true);
        let events = poll_val["result"]["events"].as_array().unwrap();
        assert!(events.len() >= 2, "Expected >=2 polled events, got {:?}", events);

        fabushi_host_destroy(host);
        drop(Arc::from_raw(raw_ctx as *const CallbackCounter));

        // 2. Legacy telegram_* and mahayana_* aliases route into exact same real engine
        let dir_c = CString::new(dir.path().to_str().unwrap()).unwrap();
        let pass_c = CString::new("keychain-secret-passphrase").unwrap();

        let tg_host = telegram_app_host_create(dir_c.as_ptr(), pass_c.as_ptr());
        assert!(!tg_host.is_null());
        let ping_c = CString::new(r#"{"command":"telegram.ping","params":{"nonce":99}}"#).unwrap();
        let r1 = telegram_app_host_dispatch(tg_host, ping_c.as_ptr());
        let v1: Value = serde_json::from_str(CStr::from_ptr(r1).to_str().unwrap()).unwrap();
        telegram_app_host_free_string(r1);
        assert_eq!(v1["ok"], true);
        assert_eq!(v1["result"]["nonce"], 99);

        let r2 = telegram_app_host_dispatch_with_handle(tg_host, ping_c.as_ptr());
        let v2: Value = serde_json::from_str(CStr::from_ptr(r2).to_str().unwrap()).unwrap();
        telegram_app_host_free_string(r2);
        assert_eq!(v2["ok"], true);
        telegram_app_host_destroy(tg_host);

        let mh_dir = tempdir().unwrap();
        let mh_dir_c = CString::new(mh_dir.path().to_str().unwrap()).unwrap();
        let mh1 = mahayana_app_host_create(mh_dir_c.as_ptr());
        assert!(!mh1.is_null());
        let mr = mahayana_app_host_dispatch_with_handle(mh1, ping_c.as_ptr());
        let mv: Value = serde_json::from_str(CStr::from_ptr(mr).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(mr);
        assert_eq!(mv["ok"], true);
        mahayana_app_host_destroy(mh1);

        let mh2 = mahayana_app_host_create_with_storage_passphrase(mh_dir_c.as_ptr(), pass_c.as_ptr());
        assert!(!mh2.is_null());
        let mr2 = mahayana_app_host_dispatch_with_handle(mh2, ping_c.as_ptr());
        let mv2: Value = serde_json::from_str(CStr::from_ptr(mr2).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(mr2);
        assert_eq!(mv2["ok"], true);
        assert_eq!(mv2["result"]["nonce"], 99);
        mahayana_app_host_destroy(mh2);

        let mh3 = mahayana_app_host_create_test(mh_dir_c.as_ptr());
        assert!(!mh3.is_null());
        let mr3 = mahayana_app_host_dispatch_with_handle(mh3, ping_c.as_ptr());
        let mv3: Value = serde_json::from_str(CStr::from_ptr(mr3).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(mr3);
        assert_eq!(mv3["ok"], true);
        mahayana_app_host_destroy(mh3);

        // 3. Sliding 128-bit replay window & protocol packet size
        assert_eq!(telegram_proto_max_packet_size(), 16_777_216);
        let win = telegram_replay_window_create();
        assert!(!win.is_null());
        assert!(!telegram_replay_window_check_and_add(win, 0));
        assert!(telegram_replay_window_check_and_add(win, 10));
        assert!(!telegram_replay_window_check_and_add(win, 10));
        assert!(telegram_replay_window_check_and_add(win, 5));
        assert!(!telegram_replay_window_check_and_add(win, 5));
        assert!(telegram_replay_window_check_and_add(win, 200));
        assert!(!telegram_replay_window_check_and_add(win, 50)); // >128 behind
        assert!(!telegram_replay_window_check_and_add(std::ptr::null_mut(), 1));
        telegram_replay_window_destroy(win);
        telegram_replay_window_destroy(std::ptr::null_mut());
    }
}

#[test]
fn test_null_utf8_malformed_json_and_panic_isolation() {
    let dir = tempdir().unwrap();

    unsafe {
        // Invalid config JSON -> returns NULL and records fabushi_host_last_error
        let bad_cfg = CString::new("{not-json").unwrap();
        let bad_host = fabushi_host_create(bad_cfg.as_ptr());
        assert!(bad_host.is_null());
        let err_ptr = fabushi_host_last_error();
        assert!(!err_ptr.is_null());
        let err_msg = CStr::from_ptr(err_ptr).to_str().unwrap();
        assert!(err_msg.contains("invalid_json"), "unexpected err: {err_msg}");
        let tg_err_ptr = telegram_app_host_last_error();
        assert!(!tg_err_ptr.is_null());
        assert!(CStr::from_ptr(tg_err_ptr).to_str().unwrap().contains("invalid_json"));
        let mh_err_ptr = mahayana_app_host_last_error();
        assert!(!mh_err_ptr.is_null());
        assert!(CStr::from_ptr(mh_err_ptr).to_str().unwrap().contains("invalid_json"));

        // Invalid UTF-8 in telegram_app_host_create fails closed (never lossy U+FFFD!)
        let bad_utf8_dir: [u8; 3] = [0xFF, 0xFE, 0x00];
        let bad_tg_host = telegram_app_host_create(bad_utf8_dir.as_ptr() as *const c_char, std::ptr::null());
        assert!(bad_tg_host.is_null());
        let tg_utf8_err = CStr::from_ptr(telegram_app_host_last_error()).to_str().unwrap();
        assert!(tg_utf8_err.contains("invalid_utf8"), "unexpected err: {tg_utf8_err}");

        // Null pointers to dispatch / poll / cancel / destroy / free must never crash
        let null_res = fabushi_host_dispatch(std::ptr::null_mut(), std::ptr::null());
        let null_val: Value =
            serde_json::from_str(CStr::from_ptr(null_res).to_str().unwrap()).unwrap();
        fabushi_host_free_string(null_res);
        assert_eq!(null_val["ok"], false);
        assert_eq!(null_val["error"]["code"], "invalid_arguments");

        let poll_null = fabushi_host_poll_events(std::ptr::null_mut(), 10, 0);
        let poll_null_val: Value =
            serde_json::from_str(CStr::from_ptr(poll_null).to_str().unwrap()).unwrap();
        fabushi_host_free_string(poll_null);
        assert_eq!(poll_null_val["ok"], false);
        assert_eq!(poll_null_val["error"]["code"], "invalid_arguments");

        assert!(!fabushi_host_cancel(std::ptr::null_mut(), std::ptr::null()));
        fabushi_host_destroy(std::ptr::null_mut());
        fabushi_host_free_string(std::ptr::null_mut());

        // Test-mode host: panic trigger caught cleanly by catch_unwind & handle remains usable
        let dir_c = CString::new(dir.path().to_str().unwrap()).unwrap();
        let test_host = telegram_app_host_create_test(dir_c.as_ptr());
        assert!(!test_host.is_null());

        // Invalid UTF-8 bytes
        let bad_utf8: [u8; 3] = [0xFF, 0xFE, 0x00];
        let utf8_ptr = fabushi_host_dispatch(test_host, bad_utf8.as_ptr() as *const c_char);
        let utf8_val: Value =
            serde_json::from_str(CStr::from_ptr(utf8_ptr).to_str().unwrap()).unwrap();
        fabushi_host_free_string(utf8_ptr);
        assert_eq!(utf8_val["ok"], false);
        assert_eq!(utf8_val["error"]["code"], "invalid_utf8");

        // Malformed JSON
        let bad_json = CString::new("{broken").unwrap();
        let bj_ptr = fabushi_host_dispatch(test_host, bad_json.as_ptr());
        let bj_val: Value =
            serde_json::from_str(CStr::from_ptr(bj_ptr).to_str().unwrap()).unwrap();
        fabushi_host_free_string(bj_ptr);
        assert_eq!(bj_val["ok"], false);
        assert_eq!(bj_val["error"]["code"], "invalid_json");

        // Trigger panic in test_mode -> returns host_panic, does NOT crash process
        let panic_res = call_dispatch(
            test_host,
            &json!({"id": "p-1", "method": "debug.trigger_panic"}),
        );
        assert_eq!(panic_res["ok"], false);
        assert_eq!(panic_res["error"]["code"], "host_panic");

        // Handle remains healthy after caught panic
        let after_panic = call_dispatch(
            test_host,
            &json!({"id": "p-2", "method": "session.ping", "params": {"nonce": 7}}),
        );
        assert_eq!(after_panic["ok"], true);
        assert_eq!(after_panic["result"]["nonce"], 7);
        fabushi_host_destroy(test_host);

        // Production handle (test_mode: false) MUST reject debug.trigger_panic with unknown_command
        let prod_dir = tempdir().unwrap();
        let prod_cfg = CString::new(
            json!({
                "app_data_dir": prod_dir.path().to_string_lossy(),
                "test_mode": false
            })
            .to_string(),
        )
        .unwrap();
        let prod_host = fabushi_host_create(prod_cfg.as_ptr());
        assert!(!prod_host.is_null());
        let prod_panic = call_dispatch(
            prod_host,
            &json!({"id": "prod-1", "method": "debug.trigger_panic"}),
        );
        assert_eq!(prod_panic["ok"], false);
        assert_eq!(prod_panic["error"]["code"], "unknown_command");
        fabushi_host_destroy(prod_host);
    }
}

#[test]
fn test_anti_stub_fail_closed_contracts_and_compat_exports() {
    let dir = tempdir().unwrap();
    let cfg_c = CString::new(
        json!({ "app_data_dir": dir.path().to_string_lossy() }).to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        // 1. Arbitrary unknown commands MUST fail closed with unknown_command (zero fake ack!)
        for bogus in ["nonexistent.method", "totally_fake", "chats.doesNotExist", "remote.unknownOp"] {
            let resp = call_dispatch(host, &json!({"id": "u-1", "method": bogus, "params": {}}));
            let raw = resp.to_string();
            assert_eq!(resp["ok"], false, "Bogus method {bogus} must fail closed: {raw}");
            assert_eq!(resp["error"]["code"], "unknown_command");
            assert!(!raw.contains(r#""status":"acknowledged""#));
            assert!(!raw.contains("acknowledged"));
            assert!(!raw.contains("sample_chats"));
        }

        // 2. Default server URL MUST be http://34.46.77.133:8080 (never localhost:8443)
        let url_resp = call_dispatch(host, &json!({"id": "u-2", "method": "session.getServerUrl"}));
        assert_eq!(url_resp["ok"], true);
        assert_eq!(url_resp["result"]["server_url"], "http://34.46.77.133:8080");
        assert!(!url_resp.to_string().contains("127.0.0.1:8443"));

        // 3. Fresh chats.list returns real empty array (ZERO fake sample chats!)
        let chats = call_dispatch(host, &json!({"id": "u-3", "method": "chats.list"}));
        assert_eq!(chats["ok"], true);
        assert_eq!(chats["result"].as_array().unwrap().len(), 0);
        assert!(!chats.to_string().contains("Saved Messages"));
        assert!(!chats.to_string().contains("Fabushi Community"));

        // 4. Legacy singleton mahayana_app_host_dispatch fails closed
        let mh_null = mahayana_app_host_dispatch(std::ptr::null());
        let mh_null_v: Value =
            serde_json::from_str(CStr::from_ptr(mh_null).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(mh_null);
        assert_eq!(mh_null_v["ok"], false);
        assert_eq!(mh_null_v["error"]["code"], "invalid_arguments");

        let req_c = CString::new(r#"{"method":"session.ping"}"#).unwrap();
        let mh_nohandle = mahayana_app_host_dispatch(req_c.as_ptr());
        let mh_nh_v: Value =
            serde_json::from_str(CStr::from_ptr(mh_nohandle).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(mh_nohandle);
        assert_eq!(mh_nh_v["ok"], false);
        assert_eq!(mh_nh_v["error"]["code"], "handle_required");

        // 5. Compat exports fail closed instead of faking empty ok:true
        let bin_bytes: [u8; 4] = [0x0A, 0xFF, 0xFE, 0x01];
        let tr_ptr = mahayana_cloud_agent_trace_rows(bin_bytes.as_ptr(), bin_bytes.len());
        let tr_v: Value = serde_json::from_str(CStr::from_ptr(tr_ptr).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(tr_ptr);
        assert_eq!(tr_v["ok"], false);
        assert!(tr_v["error"].as_str().unwrap().contains("capability_unavailable"));

        // Real JSONL trace parses cleanly
        let jsonl = b"{\"step\":1,\"tool\":\"read\"}\n{\"step\":2,\"tool\":\"edit\"}\n";
        let tr_ok = mahayana_cloud_agent_trace_rows(jsonl.as_ptr(), jsonl.len());
        let tr_ok_v: Value = serde_json::from_str(CStr::from_ptr(tr_ok).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(tr_ok);
        assert_eq!(tr_ok_v["ok"], true);
        assert_eq!(tr_ok_v["rows"].as_array().unwrap().len(), 2);

        // Unsupported .xlsx fails closed
        let fake_xlsx = CString::new("/nonexistent/book.xlsx").unwrap();
        let sp_fail = mahayana_spreadsheet_parse_file(fake_xlsx.as_ptr(), 1024 * 1024, 100);
        let sp_fail_v: Value =
            serde_json::from_str(CStr::from_ptr(sp_fail).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(sp_fail);
        assert_eq!(sp_fail_v["ok"], false);
        assert!(sp_fail_v["error"].as_str().unwrap().contains("capability_unavailable"));

        // Real bounded CSV parses cleanly with BOTH rowCount and totalRows (including truncation!)
        let csv_path = dir.path().join("ledger.csv");
        std::fs::write(&csv_path, "name,stars\nalice,100\nbob,250\ncarol,500\n").unwrap();
        let csv_c = CString::new(csv_path.to_str().unwrap()).unwrap();
        let sp_ok = mahayana_spreadsheet_parse_file(csv_c.as_ptr(), 1024 * 1024, 2);
        let sp_ok_v: Value = serde_json::from_str(CStr::from_ptr(sp_ok).to_str().unwrap()).unwrap();
        mahayana_app_host_free_string(sp_ok);
        assert_eq!(sp_ok_v["ok"], true);
        assert_eq!(sp_ok_v["result"]["sheets"][0]["rowCount"], 2);
        assert_eq!(sp_ok_v["result"]["sheets"][0]["totalRows"], 4);
        assert_eq!(sp_ok_v["result"]["sheets"][0]["truncated"], true);
        assert_eq!(sp_ok_v["result"]["sheets"][0]["rows"][1][0], "alice");

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_generation_fencing_persistence_and_real_async_cancellation() {
    let dir = tempdir().unwrap();

    // Start a real slow local TCP loopback HTTP server (sleeps 5s on /health)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let addr = rt.block_on(async {
        let app = Router::new().route(
            "/health",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                "ok"
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_str = json!({
        "app_data_dir": dir.path().to_string_lossy(),
        "server_url": format!("http://{addr}"),
        "http_timeout_secs": 15
    })
    .to_string();
    let cfg_c = CString::new(cfg_str.clone()).unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        // 1. Generation fencing
        let g1 = call_dispatch(host, &json!({"id": "g1", "method": "generation.current"}));
        assert_eq!(g1["result"]["generation"], 1);

        let g2 = call_dispatch(host, &json!({"id": "g2", "method": "generation.bump"}));
        assert_eq!(g2["result"]["generation"], 2);

        // Stale generation=1 MUST fail closed with stale_generation
        let stale = call_dispatch(
            host,
            &json!({
                "id": "g3",
                "method": "chats.list",
                "generation": 1
            }),
        );
        assert_eq!(stale["ok"], false);
        assert_eq!(stale["error"]["code"], "stale_generation");
        assert_eq!(stale["error"]["retryable"], false);
        assert_eq!(stale["generation"], 2);

        // Matching generation=2 succeeds
        let fresh = call_dispatch(
            host,
            &json!({
                "id": "g4",
                "method": "chats.list",
                "generation": 2
            }),
        );
        assert_eq!(fresh["ok"], true);

        // 2. Real in-flight async cancellation via fabushi_host_cancel
        let host_addr = host as usize;
        let cancel_req_id = "cancel-me-42";
        let start = Instant::now();

        let dispatch_thread = thread::spawn(move || {
            let req = CString::new(
                json!({
                    "id": cancel_req_id,
                    "method": "auth.health",
                    "generation": 2
                })
                .to_string(),
            )
            .unwrap();
            let ptr = fabushi_host_dispatch(host_addr as *mut c_void, req.as_ptr());
            let s = CStr::from_ptr(ptr).to_str().unwrap().to_string();
            fabushi_host_free_string(ptr);
            serde_json::from_str::<Value>(&s).unwrap()
        });

        // Wait briefly for request to enter in-flight map, then cancel
        let req_id_c = CString::new(cancel_req_id).unwrap();
        let mut cancelled_hit = false;
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(20));
            if fabushi_host_cancel(host, req_id_c.as_ptr()) {
                cancelled_hit = true;
                break;
            }
        }
        assert!(cancelled_hit, "fabushi_host_cancel must find and cancel in-flight request");

        let resp = dispatch_thread.join().unwrap();
        let elapsed = start.elapsed();
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["id"], cancel_req_id);
        assert_eq!(resp["error"]["code"], "cancelled");
        assert_eq!(resp["error"]["retryable"], false);
        assert!(
            elapsed < Duration::from_secs(3),
            "Cancellation should abort well before 5s server delay (took {elapsed:?})"
        );

        fabushi_host_destroy(host);

        // 3. Reopen on same storage_dir -> generation=2 persisted in SQLite WAL!
        let host2 = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host2.is_null());
        let g_reopen = call_dispatch(host2, &json!({"id": "g5", "method": "generation.current"}));
        assert_eq!(g_reopen["result"]["generation"], 2);
        fabushi_host_destroy(host2);
    }
}

#[test]
fn test_concurrent_poll_events_and_destroy_no_uaf_and_destroyed_handle_fails_closed() {
    let dir = tempdir().unwrap();
    let cfg_c = CString::new(
        json!({ "app_data_dir": dir.path().to_string_lossy() }).to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        let host_addr = host as usize;
        let started = Instant::now();
        let poller = thread::spawn(move || {
            let ptr = fabushi_host_poll_events(host_addr as *mut c_void, 16, 5_000);
            assert!(!ptr.is_null());
            let text = CStr::from_ptr(ptr).to_str().unwrap().to_string();
            fabushi_host_free_string(ptr);
            serde_json::from_str::<Value>(&text).unwrap()
        });

        // Let poller block inside EventQueue::poll Condvar::wait_timeout, then destroy concurrently!
        thread::sleep(Duration::from_millis(80));
        fabushi_host_destroy(host);
        // Double-destroy must also be a safe no-op
        fabushi_host_destroy(host);

        let poll_resp = poller.join().expect("poller thread must not panic/crash");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "fabushi_host_destroy must wake blocked poll_events immediately (took {elapsed:?})"
        );
        assert_eq!(poll_resp["ok"], true);

        // Subsequent calls using destroyed handle MUST fail closed (zero UAF!)
        let after_dispatch = call_dispatch(host, &json!({"id": "dead-1", "method": "session.ping"}));
        assert_eq!(after_dispatch["ok"], false);
        assert_eq!(after_dispatch["error"]["code"], "host_closed");

        let after_poll_ptr = fabushi_host_poll_events(host, 4, 10);
        let after_poll: Value =
            serde_json::from_str(CStr::from_ptr(after_poll_ptr).to_str().unwrap()).unwrap();
        fabushi_host_free_string(after_poll_ptr);
        assert_eq!(after_poll["ok"], false);
        assert_eq!(after_poll["error"]["code"], "host_closed");

        let req_id_c = CString::new("any-req").unwrap();
        assert!(!fabushi_host_cancel(host, req_id_c.as_ptr()));
    }
}

#[test]
fn test_in_flight_generation_bump_toctou_and_monotonic_generation_set() {
    let dir = tempdir().unwrap();

    // Slow loopback server: /health takes 350ms so we deterministically bump generation while in-flight
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let addr = rt.block_on(async {
        let app = Router::new().route(
            "/health",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(350)).await;
                "ok"
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}")
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        // 1. Advance generation monotonically to 5, then verify downward generation.set fails closed
        let up = call_dispatch(
            host,
            &json!({"id": "m1", "method": "generation.set", "params": {"generation": 5}}),
        );
        assert_eq!(up["ok"], true);
        assert_eq!(up["result"]["generation"], 5);

        let down = call_dispatch(
            host,
            &json!({"id": "m2", "method": "generation.set", "params": {"generation": 3}}),
        );
        assert_eq!(down["ok"], false, "Downward generation.set must fail closed: {down}");
        assert_eq!(down["error"]["code"], "invalid_arguments");
        assert_eq!(
            call_dispatch(host, &json!({"id": "m3", "method": "generation.current"}))["result"]["generation"],
            5
        );

        // 2. Start slow HTTP request at generation 5, bump generation to 6 while in-flight ->
        //    in-flight RPC MUST fail closed with stale_generation stamped with 6 (NEVER ok:true!)
        let host_addr = host as usize;
        let inflight = thread::spawn(move || {
            call_dispatch(
                host_addr as *mut c_void,
                &json!({"id": "toctou-1", "method": "auth.health", "generation": 5}),
            )
        });

        thread::sleep(Duration::from_millis(80));
        let bumped = call_dispatch(host, &json!({"id": "bump-6", "method": "generation.bump"}));
        assert_eq!(bumped["ok"], true);
        assert_eq!(bumped["result"]["generation"], 6);

        let stale_resp = inflight.join().unwrap();
        assert_eq!(
            stale_resp["ok"], false,
            "In-flight RPC after concurrent generation bump must fail closed: {stale_resp}"
        );
        assert_eq!(stale_resp["error"]["code"], "stale_generation");
        assert_eq!(stale_resp["generation"], 6);

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_b2_01_concurrent_emit_event_and_destroy_drains_callback_zero_post_destroy_uaf() {
    let dir = tempdir().unwrap();
    let cfg_c = CString::new(
        json!({ "app_data_dir": dir.path().to_string_lossy() }).to_string(),
    )
    .unwrap();

    struct DrainCtx {
        entered: AtomicBool,
        release: AtomicBool,
        exited: AtomicBool,
        destroy_returned: AtomicBool,
        post_destroy_callbacks: AtomicUsize,
    }

    let ctx = Arc::new(DrainCtx {
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        destroy_returned: AtomicBool::new(false),
        post_destroy_callbacks: AtomicUsize::new(0),
    });

    unsafe extern "C" fn on_blocking_event(
        event_type: *const c_char,
        _payload: *const c_char,
        user_ctx: *mut c_void,
    ) {
        if event_type.is_null() || user_ctx.is_null() {
            return;
        }
        let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
        let st = unsafe { &*(user_ctx as *const DrainCtx) };
        if st.destroy_returned.load(Ordering::SeqCst) {
            st.post_destroy_callbacks.fetch_add(1, Ordering::SeqCst);
        }
        if ev == "taskStateUpdated" {
            st.entered.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !st.release.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            st.exited.store(true, Ordering::SeqCst);
        }
    }

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());

        let raw_ctx = Arc::into_raw(Arc::clone(&ctx)) as *mut c_void;
        telegram_bridge_register_event_callback(host, Some(on_blocking_event), raw_ctx);

        let host_addr = host as usize;
        let emitter = thread::spawn(move || {
            call_dispatch(
                host_addr as *mut c_void,
                &json!({"id": "emit-1", "method": "fabushi.createTask", "params": {"task_id": "drain-task"}}),
            )
        });

        // Wait deterministically until the C callback is actively running on Thread 1
        let enter_deadline = Instant::now() + Duration::from_secs(3);
        while !ctx.entered.load(Ordering::SeqCst) && Instant::now() < enter_deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(ctx.entered.load(Ordering::SeqCst), "C callback must enter");

        // Thread 2 invokes fabushi_host_destroy while Thread 1 is blocked inside C callback
        let ctx_destroyer = Arc::clone(&ctx);
        let destroyer = thread::spawn(move || {
            fabushi_host_destroy(host_addr as *mut c_void);
            ctx_destroyer.destroy_returned.store(true, Ordering::SeqCst);
        });

        // Give destroyer plenty of time: fabushi_host_destroy MUST NOT return while C callback is active!
        thread::sleep(Duration::from_millis(120));
        assert!(
            !ctx.destroy_returned.load(Ordering::SeqCst),
            "fabushi_host_destroy MUST block until in-flight emit_event C callback finishes (B2-01)"
        );

        // Release the C callback -> both emitter and destroyer must now complete cleanly
        ctx.release.store(true, Ordering::SeqCst);
        let emit_resp = emitter.join().expect("emitter must not panic");
        assert_eq!(emit_resp["ok"], true);
        destroyer.join().expect("destroyer must not panic");

        assert!(ctx.exited.load(Ordering::SeqCst));
        assert!(ctx.destroy_returned.load(Ordering::SeqCst));
        assert_eq!(ctx.post_destroy_callbacks.load(Ordering::SeqCst), 0);

        // Re-registering callback or dispatching on destroyed host must fail closed with zero UAF
        telegram_bridge_register_event_callback(host, Some(on_blocking_event), raw_ctx);
        let after = call_dispatch(host, &json!({"id": "after-1", "method": "session.ping"}));
        assert_eq!(after["ok"], false);
        assert_eq!(after["error"]["code"], "host_closed");

        // Safe to free caller-owned user_context immediately after fabushi_host_destroy returns
        drop(Arc::from_raw(raw_ctx as *const DrainCtx));
    }
}

#[test]
fn test_b2_01_concurrent_set_server_url_and_destroy_no_orphaned_pump_and_clean_error() {
    for round in 0..4 {
        let dir = tempdir().unwrap();
        let cfg_c = CString::new(
            json!({
                "app_data_dir": dir.path().to_string_lossy(),
                "server_url": "http://127.0.0.1:18080"
            })
            .to_string(),
        )
        .unwrap();

        unsafe {
            let host = fabushi_host_create(cfg_c.as_ptr());
            assert!(!host.is_null());
            let host_addr = host as usize;
            let next_url = format!("http://127.0.0.1:{}", 18081 + round);

            let reconf = thread::spawn(move || {
                call_dispatch(
                    host_addr as *mut c_void,
                    &json!({
                        "id": format!("reconf-{round}"),
                        "method": "session.setServerUrl",
                        "params": {"server_url": next_url}
                    }),
                )
            });

            if round % 2 == 1 {
                thread::sleep(Duration::from_millis(5));
            }
            fabushi_host_destroy(host);

            let resp = reconf.join().expect("concurrent setServerUrl must not panic or deadlock");
            if resp["ok"] == false {
                assert_eq!(
                    resp["error"]["code"], "host_closed",
                    "Failed concurrent setServerUrl during destroy must fail closed with host_closed: {resp}"
                );
            }

            // Post-destroy setServerUrl MUST fail closed with host_closed
            let post = call_dispatch(
                host,
                &json!({
                    "id": "post-reconf",
                    "method": "session.setServerUrl",
                    "params": {"server_url": "http://127.0.0.1:18099"}
                }),
            );
            assert_eq!(post["ok"], false);
            assert_eq!(post["error"]["code"], "host_closed");
        }
    }
}

#[test]
fn test_nb2_01_anonymous_rpc_cancelled_on_destroy_and_nb2_06_replay_window_registry_safety() {
    let dir = tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let addr = rt.block_on(async {
        let app = Router::new().route(
            "/health",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                "ok"
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        local_addr
    });

    let cfg_c = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "server_url": format!("http://{addr}")
        })
        .to_string(),
    )
    .unwrap();

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());
        let host_addr = host as usize;

        // Dispatch an anonymous RPC (omitting "id") against the 5s slow endpoint
        let start = Instant::now();
        let anon_thread = thread::spawn(move || {
            call_dispatch(
                host_addr as *mut c_void,
                &json!({"method": "auth.health"}),
            )
        });

        thread::sleep(Duration::from_millis(80));
        // External callers MUST NOT be able to cancel anonymous RPCs by guessing internal keys
        let empty_c = CString::new("").unwrap();
        let guessed_c = CString::new("__anon_1").unwrap();
        assert!(!fabushi_host_cancel(host, empty_c.as_ptr()));
        assert!(!fabushi_host_cancel(host, guessed_c.as_ptr()));

        // Destroying the host MUST cancel even anonymous in-flight RPCs promptly (NB2-01)
        fabushi_host_destroy(host);
        let anon_resp = anon_thread.join().expect("anon RPC thread must not panic");
        let elapsed = start.elapsed();
        assert_eq!(anon_resp["ok"], false, "Anonymous RPC must abort on shutdown: {anon_resp}");
        assert_eq!(anon_resp["error"]["code"], "cancelled");
        assert!(
            elapsed < Duration::from_secs(3),
            "Anonymous RPC must cancel promptly on shutdown (took {elapsed:?})"
        );

        // NB2-06: SlidingReplayWindow refcounted registry prevents double-destroy & post-destroy UAF
        let win = telegram_replay_window_create();
        assert!(!win.is_null());
        assert!(telegram_replay_window_check_and_add(win, 10));
        assert!(!telegram_replay_window_check_and_add(win, 10));
        telegram_replay_window_destroy(win);
        telegram_replay_window_destroy(win); // idempotent double-destroy no-op
        assert!(!telegram_replay_window_check_and_add(win, 11)); // post-destroy fails closed safely
    }
}

#[test]
fn test_b3_03_unregister_and_replace_callback_drains_inflight_callbacks_zero_uaf_and_reentrant_safe() {
    let dir = tempdir().unwrap();
    let cfg_c = CString::new(
        json!({ "app_data_dir": dir.path().to_string_lossy() }).to_string(),
    )
    .unwrap();

    struct UnregDrainCtx {
        entered: AtomicBool,
        release: AtomicBool,
        exited: AtomicBool,
        unregister_returned: AtomicBool,
        post_unregister_callbacks: AtomicUsize,
    }

    unsafe extern "C" fn on_blocking_cb(
        event_type: *const c_char,
        _payload: *const c_char,
        user_ctx: *mut c_void,
    ) {
        if event_type.is_null() || user_ctx.is_null() {
            return;
        }
        let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
        let st = unsafe { &*(user_ctx as *const UnregDrainCtx) };
        if st.unregister_returned.load(Ordering::SeqCst) {
            st.post_unregister_callbacks.fetch_add(1, Ordering::SeqCst);
        }
        if ev == "taskStateUpdated" {
            st.entered.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !st.release.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            st.exited.store(true, Ordering::SeqCst);
        }
    }

    struct CounterCtx {
        count: AtomicUsize,
    }

    unsafe extern "C" fn on_replacement_cb(
        event_type: *const c_char,
        _payload: *const c_char,
        user_ctx: *mut c_void,
    ) {
        if event_type.is_null() || user_ctx.is_null() {
            return;
        }
        let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
        if ev == "taskStateUpdated" {
            let st = unsafe { &*(user_ctx as *const CounterCtx) };
            st.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct ReentrantUnregCtx {
        host_addr: usize,
        count: AtomicUsize,
    }

    unsafe extern "C" fn on_reentrant_unregister_cb(
        event_type: *const c_char,
        _payload: *const c_char,
        user_ctx: *mut c_void,
    ) {
        if event_type.is_null() || user_ctx.is_null() {
            return;
        }
        let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
        if ev == "taskStateUpdated" {
            let st = unsafe { &*(user_ctx as *const ReentrantUnregCtx) };
            st.count.fetch_add(1, Ordering::SeqCst);
            // Re-entrant unregister from inside the callback on its own thread MUST NOT deadlock!
            unsafe {
                telegram_bridge_register_event_callback(
                    st.host_addr as *mut c_void,
                    None,
                    std::ptr::null_mut(),
                );
            }
        }
    }

    unsafe {
        let host = fabushi_host_create(cfg_c.as_ptr());
        assert!(!host.is_null());
        let host_addr = host as usize;

        // 1. Unregister (NULL callback) while host remains ALIVE MUST block until in-flight
        //    C callback on Thread 1 completes before caller frees ctx_1 (B3-03).
        let ctx_1 = Arc::new(UnregDrainCtx {
            entered: AtomicBool::new(false),
            release: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            unregister_returned: AtomicBool::new(false),
            post_unregister_callbacks: AtomicUsize::new(0),
        });
        let raw_ctx_1 = Arc::into_raw(Arc::clone(&ctx_1)) as *mut c_void;
        telegram_bridge_register_event_callback(host, Some(on_blocking_cb), raw_ctx_1);

        let emitter1 = thread::spawn(move || {
            call_dispatch(
                host_addr as *mut c_void,
                &json!({"id": "b303-emit-1", "method": "fabushi.createTask", "params": {"task_id": "b303-t1"}}),
            )
        });

        let enter_deadline = Instant::now() + Duration::from_secs(3);
        while !ctx_1.entered.load(Ordering::SeqCst) && Instant::now() < enter_deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(ctx_1.entered.load(Ordering::SeqCst), "Callback 1 must enter");

        let ctx_unreg = Arc::clone(&ctx_1);
        let unregistrator = thread::spawn(move || {
            telegram_bridge_register_event_callback(
                host_addr as *mut c_void,
                None,
                std::ptr::null_mut(),
            );
            ctx_unreg.unregister_returned.store(true, Ordering::SeqCst);
        });

        thread::sleep(Duration::from_millis(120));
        assert!(
            !ctx_1.unregister_returned.load(Ordering::SeqCst),
            "telegram_bridge_register_event_callback(NULL) MUST block until in-flight C callback exits (B3-03)"
        );

        ctx_1.release.store(true, Ordering::SeqCst);
        let emit1_resp = emitter1.join().expect("emitter1 must not panic");
        assert_eq!(emit1_resp["ok"], true);
        unregistrator.join().expect("unregistrator must not panic");

        assert!(ctx_1.exited.load(Ordering::SeqCst));
        assert!(ctx_1.unregister_returned.load(Ordering::SeqCst));
        assert_eq!(ctx_1.post_unregister_callbacks.load(Ordering::SeqCst), 0);

        // Immediately free ctx_1 while host stays ALIVE and dispatch again -> ZERO UAF!
        drop(Arc::from_raw(raw_ctx_1 as *const UnregDrainCtx));
        let alive_after_unreg = call_dispatch(
            host,
            &json!({"id": "b303-alive-1", "method": "fabushi.createTask", "params": {"task_id": "b303-t2"}}),
        );
        assert_eq!(alive_after_unreg["ok"], true);
        assert_eq!(
            call_dispatch(host, &json!({"id": "b303-ping", "method": "session.ping"}))["ok"],
            true
        );

        // 2. Replacing callback (cb_1b -> cb_2) ALSO drains cb_1b before returning and routes
        //    subsequent events strictly to cb_2!
        let ctx_1b = Arc::new(UnregDrainCtx {
            entered: AtomicBool::new(false),
            release: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            unregister_returned: AtomicBool::new(false),
            post_unregister_callbacks: AtomicUsize::new(0),
        });
        let raw_ctx_1b = Arc::into_raw(Arc::clone(&ctx_1b)) as *mut c_void;
        let ctx_2 = Arc::new(CounterCtx {
            count: AtomicUsize::new(0),
        });
        let raw_ctx_2 = Arc::into_raw(Arc::clone(&ctx_2)) as *mut c_void;
        let raw_ctx_2_addr = raw_ctx_2 as usize;

        telegram_bridge_register_event_callback(host, Some(on_blocking_cb), raw_ctx_1b);
        let emitter2 = thread::spawn(move || {
            call_dispatch(
                host_addr as *mut c_void,
                &json!({"id": "b303-emit-2", "method": "fabushi.createTask", "params": {"task_id": "b303-t3"}}),
            )
        });

        let enter_deadline2 = Instant::now() + Duration::from_secs(3);
        while !ctx_1b.entered.load(Ordering::SeqCst) && Instant::now() < enter_deadline2 {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(ctx_1b.entered.load(Ordering::SeqCst));

        let ctx_replace = Arc::clone(&ctx_1b);
        let replacer = thread::spawn(move || {
            telegram_bridge_register_event_callback(
                host_addr as *mut c_void,
                Some(on_replacement_cb),
                raw_ctx_2_addr as *mut c_void,
            );
            ctx_replace.unregister_returned.store(true, Ordering::SeqCst);
        });

        thread::sleep(Duration::from_millis(120));
        assert!(
            !ctx_1b.unregister_returned.load(Ordering::SeqCst),
            "Callback replacement MUST drain prior callback on other threads before returning (B3-03)"
        );

        ctx_1b.release.store(true, Ordering::SeqCst);
        assert_eq!(emitter2.join().expect("emitter2 must not panic")["ok"], true);
        replacer.join().expect("replacer must not panic");
        assert!(ctx_1b.exited.load(Ordering::SeqCst));
        assert_eq!(ctx_1b.post_unregister_callbacks.load(Ordering::SeqCst), 0);

        // Free ctx_1b immediately and verify replacement cb_2 receives new events cleanly
        drop(Arc::from_raw(raw_ctx_1b as *const UnregDrainCtx));
        assert_eq!(
            call_dispatch(
                host,
                &json!({"id": "b303-emit-3", "method": "fabushi.createTask", "params": {"task_id": "b303-t4"}}),
            )["ok"],
            true
        );
        assert_eq!(ctx_2.count.load(Ordering::SeqCst), 1);

        telegram_bridge_register_event_callback(host, None, std::ptr::null_mut());
        drop(Arc::from_raw(raw_ctx_2 as *const CounterCtx));

        // 3. Re-entrant unregister from inside the callback itself MUST succeed with zero deadlock
        let ctx_reent = Arc::new(ReentrantUnregCtx {
            host_addr,
            count: AtomicUsize::new(0),
        });
        let raw_ctx_reent = Arc::into_raw(Arc::clone(&ctx_reent)) as *mut c_void;
        telegram_bridge_register_event_callback(
            host,
            Some(on_reentrant_unregister_cb),
            raw_ctx_reent,
        );

        assert_eq!(
            call_dispatch(
                host,
                &json!({"id": "b303-reent-1", "method": "fabushi.createTask", "params": {"task_id": "b303-r1"}}),
            )["ok"],
            true
        );
        assert_eq!(ctx_reent.count.load(Ordering::SeqCst), 1);

        // Second task creation MUST NOT invoke the re-entrantly unregistered callback
        assert_eq!(
            call_dispatch(
                host,
                &json!({"id": "b303-reent-2", "method": "fabushi.createTask", "params": {"task_id": "b303-r2"}}),
            )["ok"],
            true
        );
        assert_eq!(ctx_reent.count.load(Ordering::SeqCst), 1);

        fabushi_host_destroy(host);
        drop(Arc::from_raw(raw_ctx_reent as *const ReentrantUnregCtx));
    }
}

