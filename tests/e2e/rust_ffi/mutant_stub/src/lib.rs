//! Configurable negative-oracle `cdylib` used EXCLUSIVELY by `tests/self_validate_antistub.rs`
//! to prove that our opaque-box contract assertions catch stub/facade defects.
//!
//! NEVER loaded by `FabushiE2EHarness::connect_production()` (which explicitly rejects any
//! library path containing `mutant_stub`).

use serde_json::{Value, json};
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::atomic::{AtomicU64, Ordering};

struct MutantHost {
    mode: String,
    generation: AtomicU64,
    mutated_on_stale_count: AtomicU64,
}

fn to_raw_c_string(payload: &Value) -> *mut c_char {
    let text = payload.to_string();
    match CString::new(text) {
        Ok(cstr) => cstr.into_raw(),
        Err(_) => CString::new(
            r#"{"id":"","ok":false,"generation":0,"error":{"code":"serialization_error","message":"nul byte in response","retryable":false}}"#,
        )
        .expect("static JSON has no NUL")
        .into_raw(),
    }
}

fn read_c_str(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Caller passes a valid NUL-terminated C string pointer.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str().ok().map(str::to_owned)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_create(config_json: *const c_char) -> *mut c_void {
    let env_mode = std::env::var("FABUSHI_MUTANT_MODE").ok();
    let cfg_mode = read_c_str(config_json)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|val| {
            val.get("mutant_mode")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let mode = env_mode
        .or(cfg_mode)
        .unwrap_or_else(|| "acknowledged_wildcard".to_owned());

    let host = Box::new(MutantHost {
        mode,
        generation: AtomicU64::new(1),
        mutated_on_stale_count: AtomicU64::new(0),
    });
    Box::into_raw(host).cast::<c_void>()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_dispatch(
    handle: *mut c_void,
    request_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return to_raw_c_string(&json!({
            "id": "",
            "ok": false,
            "generation": 0,
            "error": {
                "code": "invalid_handle",
                "message": "null host handle",
                "retryable": false
            }
        }));
    }

    // SAFETY: `handle` was created by `fabushi_host_create` and not yet destroyed.
    let host = unsafe { &*handle.cast::<MutantHost>() };
    let raw = match read_c_str(request_json) {
        Some(s) => s,
        None => {
            return to_raw_c_string(&json!({
                "id": "",
                "ok": false,
                "generation": host.generation.load(Ordering::SeqCst),
                "error": {
                    "code": "invalid_json",
                    "message": "null or non-UTF8 request",
                    "retryable": false
                }
            }));
        }
    };

    let req: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(err) => {
            return to_raw_c_string(&json!({
                "id": "",
                "ok": false,
                "generation": host.generation.load(Ordering::SeqCst),
                "error": {
                    "code": "invalid_json",
                    "message": format!("parse error: {err}"),
                    "retryable": false
                }
            }));
        }
    };

    let id = req.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let req_gen = req.get("generation").and_then(Value::as_u64).unwrap_or(1);
    let current_gen = host.generation.load(Ordering::SeqCst);

    if method == "generation.bump" {
        let next = host.generation.fetch_add(1, Ordering::SeqCst) + 1;
        return to_raw_c_string(&json!({
            "id": id,
            "ok": true,
            "generation": next,
            "result": {
                "previous_generation": next - 1,
                "generation": next
            }
        }));
    }

    match host.mode.as_str() {
        "acknowledged_wildcard" => {
            // Deliberate bug: fail-open wildcard acknowledging unknown commands.
            to_raw_c_string(&json!({
                "id": id,
                "ok": true,
                "generation": current_gen,
                "result": {
                    "status": "acknowledged",
                    "command": method
                }
            }))
        }
        "sample_chats" => {
            // Deliberate bug: injects synthetic sample chats on a fresh unauthenticated host.
            to_raw_c_string(&json!({
                "id": id,
                "ok": true,
                "generation": current_gen,
                "result": {
                    "chats": [
                        {
                            "chat_id": 100,
                            "title": "Saved Messages",
                            "kind": "saved",
                            "last_message": "Welcome to Fabushi iOS Telegram Core"
                        },
                        {
                            "chat_id": 200,
                            "title": "Fabushi Community",
                            "kind": "group",
                            "last_message": "Native iOS 1:1 Telegram UI is live!"
                        }
                    ],
                    "total": 2
                }
            }))
        }
        "missing_retryable" => {
            // Deliberate bug: error envelope omits mandatory boolean `error.retryable`.
            to_raw_c_string(&json!({
                "id": id,
                "ok": false,
                "generation": current_gen,
                "error": {
                    "code": "unknown_command",
                    "message": format!("unrecognized method: {method}")
                }
            }))
        }
        "stale_generation_mutation" => {
            // Deliberate bug: mutates state when `req_gen < current_gen` instead of returning `stale_generation`.
            let mutated = if req_gen < current_gen {
                host.mutated_on_stale_count.fetch_add(1, Ordering::SeqCst) + 1
            } else {
                host.mutated_on_stale_count.load(Ordering::SeqCst)
            };
            to_raw_c_string(&json!({
                "id": id,
                "ok": true,
                "generation": current_gen,
                "result": {
                    "accepted_stale": req_gen < current_gen,
                    "mutated_count": mutated
                }
            }))
        }
        "hardcoded_localhost" => {
            // Deliberate bug: returns legacy hardcoded https://127.0.0.1:8443 instead of configurable URL.
            to_raw_c_string(&json!({
                "id": id,
                "ok": true,
                "generation": current_gen,
                "result": {
                    "server_url": "https://127.0.0.1:8443",
                    "config_source": "default_fallback"
                }
            }))
        }
        "missing_id_or_generation" => {
            // Deliberate bug: omits both `id` and `generation` from envelope.
            to_raw_c_string(&json!({
                "ok": true,
                "result": {
                    "method": method
                }
            }))
        }
        "missing_symbols_probe" => {
            // Deliberate defect mode used with poll/cancel probes returning unsupported markers.
            to_raw_c_string(&json!({
                "id": id,
                "ok": false,
                "generation": current_gen,
                "error": {
                    "code": "symbol_not_implemented",
                    "message": "poll_events/cancel missing in mutant",
                    "retryable": false
                }
            }))
        }
        "unhandled_panic_marker" => {
            // Deliberate bug: returns malformed non-JSON / non-compliant payload on panic fault probe.
            CString::new("PANIC_UNCAUGHT_NON_JSON_ENVELOPE")
                .expect("static ascii")
                .into_raw()
        }
        _ => to_raw_c_string(&json!({
            "id": id,
            "ok": false,
            "generation": current_gen,
            "error": {
                "code": "unknown_command",
                "message": format!("unknown mutant mode: {}", host.mode),
                "retryable": false
            }
        })),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_poll_events(
    handle: *mut c_void,
    _max_count: u32,
    _timeout_ms: u32,
) -> *mut c_char {
    if handle.is_null() {
        return to_raw_c_string(&json!({
            "ok": false,
            "generation": 0,
            "error": {
                "code": "invalid_handle",
                "message": "null handle",
                "retryable": false
            }
        }));
    }
    // SAFETY: `handle` was created by `fabushi_host_create`.
    let host = unsafe { &*handle.cast::<MutantHost>() };
    if host.mode == "missing_symbols_probe" {
        return std::ptr::null_mut();
    }
    to_raw_c_string(&json!({
        "ok": true,
        "generation": host.generation.load(Ordering::SeqCst),
        "events": []
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_cancel(
    handle: *mut c_void,
    request_id: *const c_char,
) -> bool {
    if handle.is_null() || request_id.is_null() {
        return false;
    }
    // SAFETY: `handle` was created by `fabushi_host_create`.
    let host = unsafe { &*handle.cast::<MutantHost>() };
    if host.mode == "missing_symbols_probe" {
        return false;
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        // SAFETY: `handle` was allocated by `Box::into_raw` in `fabushi_host_create`.
        unsafe {
            let _ = Box::from_raw(handle.cast::<MutantHost>());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        // SAFETY: `ptr` was allocated by `CString::into_raw` in this cdylib.
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}
