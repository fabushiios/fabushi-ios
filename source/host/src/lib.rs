pub mod compat;
pub mod dedup;
pub mod dispatch;
pub mod error;
pub mod events;
pub mod ffi;
pub mod handlers_agent;
pub mod handlers_core;
pub mod handlers_messaging;
pub mod handlers_platform;
pub mod host;
pub mod search;
pub mod storage;

use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use serde_json::json;

pub use events::{HostEvent, TelegramBridgeEventCallback};
pub use host::{FabushiAppHost, HostConfig, TelegramAppHost};

use crate::error::{make_error_json_ptr, make_success_json};
use crate::ffi::{
    extract_panic_message, free_c_string, get_last_error_ptr, set_last_error, string_to_c_ptr,
};

// ============================================================================
// 1. Canonical Fabushi Host C-ABI Exports (Primary v1.6.7 API)
// ============================================================================

/// Creates a canonical `FabushiAppHost` instance from optional JSON configuration.
///
/// # Safety
/// If non-null, `config_json` must point to a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_create(config_json: *const c_char) -> *mut c_void {
    let result = catch_unwind(AssertUnwindSafe(|| {
        set_last_error(None);
        let config = if config_json.is_null() {
            HostConfig::default()
        } else {
            let raw = match unsafe { CStr::from_ptr(config_json) }.to_str() {
                Ok(s) => s.trim(),
                Err(e) => {
                    set_last_error(Some(format!("invalid_utf8: {e}")));
                    return std::ptr::null_mut();
                }
            };
            if raw.is_empty() {
                HostConfig::default()
            } else {
                match serde_json::from_str::<HostConfig>(raw) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        set_last_error(Some(format!("invalid_json: {e}")));
                        return std::ptr::null_mut();
                    }
                }
            }
        };

        match FabushiAppHost::from_config(config) {
            Ok(host) => Box::into_raw(Box::new(host)) as *mut c_void,
            Err(err) => {
                set_last_error(Some(err));
                std::ptr::null_mut()
            }
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(panic) => {
            let msg = extract_panic_message(panic);
            set_last_error(Some(format!("host_panic: {msg}")));
            std::ptr::null_mut()
        }
    }
}

/// Synchronously dispatches a fail-closed JSON-RPC request to the canonical host engine.
///
/// # Safety
/// `handle` must be a live pointer returned by `fabushi_host_create` (or legacy aliases).
/// `request_json` must point to a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_dispatch(
    handle: *mut c_void,
    request_json: *const c_char,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() || request_json.is_null() {
            return make_error_json_ptr(
                "",
                0,
                "invalid_arguments",
                "Null host handle or request JSON pointer",
                false,
            );
        }

        let input_str = match unsafe { CStr::from_ptr(request_json) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                return make_error_json_ptr(
                    "",
                    0,
                    "invalid_utf8",
                    &format!("UTF-8 decode error: {e}"),
                    false,
                );
            }
        };

        let host_ref = unsafe { &*(handle as *const FabushiAppHost) };
        let response_str = host_ref.dispatch(input_str);
        string_to_c_ptr(response_str)
    }));

    match result {
        Ok(ptr) => ptr,
        Err(panic) => {
            let msg = extract_panic_message(panic);
            make_error_json_ptr(
                "",
                0,
                "host_panic",
                &format!("Rust host panic during dispatch: {msg}"),
                false,
            )
        }
    }
}

/// Polls up to `max_count` events, blocking up to `timeout_ms` ms until an event is ready.
///
/// # Safety
/// `handle` must be a live pointer returned by `fabushi_host_create` (or legacy aliases).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_poll_events(
    handle: *mut c_void,
    max_count: u32,
    timeout_ms: u32,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return make_error_json_ptr(
                "",
                0,
                "invalid_arguments",
                "Null host handle passed to fabushi_host_poll_events",
                false,
            );
        }
        let host_ref = unsafe { &*(handle as *const FabushiAppHost) };
        let events = host_ref.poll_events(max_count, timeout_ms);
        let generation = host_ref.current_generation();
        let response = make_success_json("", generation, &json!({ "events": events }));
        string_to_c_ptr(response)
    }));

    match result {
        Ok(ptr) => ptr,
        Err(panic) => {
            let msg = extract_panic_message(panic);
            make_error_json_ptr(
                "",
                0,
                "host_panic",
                &format!("Rust host panic during poll_events: {msg}"),
                false,
            )
        }
    }
}

/// Cooperatively cancels an in-flight RPC matching `request_id`.
///
/// # Safety
/// `handle` must be a live host pointer and `request_id` a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_cancel(
    handle: *mut c_void,
    request_id: *const c_char,
) -> bool {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() || request_id.is_null() {
            return false;
        }
        let id = match unsafe { CStr::from_ptr(request_id) }.to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };
        let host_ref = unsafe { &*(handle as *const FabushiAppHost) };
        host_ref.cancel_request(id)
    }));
    result.unwrap_or(false)
}

/// Destroys a canonical host handle and reclaims all resources cleanly.
///
/// # Safety
/// `handle` must be NULL or a live pointer created by `fabushi_host_create` / legacy create aliases.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            unsafe {
                drop(Box::from_raw(handle as *mut FabushiAppHost));
            }
        }));
    }
}

/// Releases a UTF-8 C string returned by any host C-ABI entrypoint.
///
/// # Safety
/// `ptr` must be NULL or a pointer previously allocated by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_free_string(ptr: *mut c_char) {
    unsafe { free_c_string(ptr) };
}

/// Returns the last initialization error recorded on the calling thread, or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fabushi_host_last_error() -> *const c_char {
    catch_unwind(AssertUnwindSafe(get_last_error_ptr)).unwrap_or(std::ptr::null())
}

// ============================================================================
// 2. Unified Legacy `telegram_*` Compatibility Aliases (Same Canonical Engine)
// ============================================================================

/// # Safety
/// If non-null, `app_data_dir` and `storage_passphrase` must be valid NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_app_host_create(
    app_data_dir: *const c_char,
    storage_passphrase: *const c_char,
) -> *mut c_void {
    let result = catch_unwind(AssertUnwindSafe(|| {
        set_last_error(None);
        let path = if app_data_dir.is_null() {
            compat::default_app_data_dir()
        } else {
            let cstr = unsafe { CStr::from_ptr(app_data_dir) };
            PathBuf::from(cstr.to_string_lossy().into_owned())
        };

        let passphrase = if storage_passphrase.is_null() {
            None
        } else {
            let cstr = unsafe { CStr::from_ptr(storage_passphrase) };
            let s = cstr.to_string_lossy().into_owned();
            if s.is_empty() { None } else { Some(s) }
        };

        match FabushiAppHost::new(path, passphrase) {
            Ok(host) => Box::into_raw(Box::new(host)) as *mut c_void,
            Err(err) => {
                set_last_error(Some(err));
                std::ptr::null_mut()
            }
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(panic) => {
            let msg = extract_panic_message(panic);
            set_last_error(Some(format!("host_panic: {msg}")));
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// If non-null, `app_data_dir` must be a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_app_host_create_test(
    app_data_dir: *const c_char,
) -> *mut c_void {
    let result = catch_unwind(AssertUnwindSafe(|| {
        set_last_error(None);
        let path = if app_data_dir.is_null() {
            compat::default_app_data_dir().join("test_host")
        } else {
            let cstr = unsafe { CStr::from_ptr(app_data_dir) };
            PathBuf::from(cstr.to_string_lossy().into_owned())
        };

        match FabushiAppHost::new_test(path) {
            Ok(host) => Box::into_raw(Box::new(host)) as *mut c_void,
            Err(err) => {
                set_last_error(Some(err));
                std::ptr::null_mut()
            }
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(panic) => {
            let msg = extract_panic_message(panic);
            set_last_error(Some(format!("host_panic: {msg}")));
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_app_host_last_error() -> *const c_char {
    unsafe { fabushi_host_last_error() }
}

/// Registers or unregisters a push event callback. Passing NULL for `callback` safely unregisters.
///
/// # Safety
/// `host` must be NULL or a live handle returned by host creation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_bridge_register_event_callback(
    host: *mut c_void,
    callback: Option<TelegramBridgeEventCallback>,
    user_context: *mut c_void,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if host.is_null() {
            return;
        }
        let host_ref = unsafe { &*(host as *const FabushiAppHost) };
        host_ref.register_event_callback(callback, user_context);
    }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_app_host_dispatch(
    host: *mut c_void,
    request_json: *const c_char,
) -> *mut c_char {
    unsafe { fabushi_host_dispatch(host, request_json) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_app_host_dispatch_with_handle(
    host: *mut c_void,
    request_json: *const c_char,
) -> *mut c_char {
    unsafe { fabushi_host_dispatch(host, request_json) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_app_host_free_string(pointer: *mut c_char) {
    unsafe { fabushi_host_free_string(pointer) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_app_host_destroy(host: *mut c_void) {
    unsafe { fabushi_host_destroy(host) };
}

// ============================================================================
// 3. Unified Legacy `mahayana_*` Compatibility Aliases (Same Canonical Engine)
// ============================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_create(app_data_dir: *const c_char) -> *mut c_void {
    unsafe { telegram_app_host_create(app_data_dir, std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_create_with_storage_passphrase(
    app_data_dir: *const c_char,
    storage_passphrase: *const c_char,
) -> *mut c_void {
    unsafe { telegram_app_host_create(app_data_dir, storage_passphrase) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_create_test(
    app_data_dir: *const c_char,
) -> *mut c_void {
    unsafe { telegram_app_host_create_test(app_data_dir) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_last_error() -> *const c_char {
    unsafe { fabushi_host_last_error() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_dispatch_with_handle(
    host: *mut c_void,
    request_json: *const c_char,
) -> *mut c_char {
    unsafe { fabushi_host_dispatch(host, request_json) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_dispatch(request_json: *const c_char) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if request_json.is_null() {
            return make_error_json_ptr(
                "",
                0,
                "invalid_arguments",
                "Null request JSON pointer",
                false,
            );
        }
        make_error_json_ptr(
            "",
            0,
            "handle_required",
            "Global singleton dispatch removed; pass an explicit host handle",
            false,
        )
    }));
    match result {
        Ok(ptr) => ptr,
        Err(panic) => {
            let msg = extract_panic_message(panic);
            make_error_json_ptr("", 0, "host_panic", &msg, false)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_free_string(pointer: *mut c_char) {
    unsafe { fabushi_host_free_string(pointer) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_destroy(host: *mut c_void) {
    unsafe { fabushi_host_destroy(host) };
}

// ============================================================================
// 4. Sliding Replay Window & Protocol Limit C-ABI Exports (E2EE)
// ============================================================================

const REPLAY_WINDOW_BITS: u64 = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlidingReplayWindow128 {
    pub highest: u64,
    pub bitmap: u128,
}

impl SlidingReplayWindow128 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check_and_add(&mut self, seq: u64) -> bool {
        if seq == 0 {
            return false;
        }
        if self.highest == 0 {
            self.highest = seq;
            self.bitmap = 1;
            return true;
        }
        if seq > self.highest {
            let shift = seq - self.highest;
            self.bitmap = if shift >= REPLAY_WINDOW_BITS {
                1
            } else {
                (self.bitmap << shift) | 1
            };
            self.highest = seq;
            return true;
        }
        let distance = self.highest - seq;
        if distance >= REPLAY_WINDOW_BITS {
            return false;
        }
        if (self.bitmap & (1u128 << distance)) != 0 {
            return false;
        }
        self.bitmap |= 1u128 << distance;
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_replay_window_create() -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(SlidingReplayWindow128::new())) as *mut c_void
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_replay_window_check_and_add(
    window: *mut c_void,
    seq: u64,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        if window.is_null() {
            return false;
        }
        let win = unsafe { &mut *(window as *mut SlidingReplayWindow128) };
        win.check_and_add(seq)
    }))
    .unwrap_or(false)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_replay_window_destroy(window: *mut c_void) {
    if !window.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            unsafe { drop(Box::from_raw(window as *mut SlidingReplayWindow128)) };
        }));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn telegram_proto_max_packet_size() -> usize {
    catch_unwind(AssertUnwindSafe(|| 16 * 1024 * 1024)).unwrap_or(0)
}
