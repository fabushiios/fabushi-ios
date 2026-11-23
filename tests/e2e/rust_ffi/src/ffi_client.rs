//! Safe dynamic `libloading` C-ABI client for the canonical 6 `fabushi_host_*` symbols
//! (`PROJECT.md § Interface Contracts #1`) plus an isolated `LegacyHost8128951Probe` used
//! exclusively by `tests/self_validate_antistub.rs` to prove negative detection of baseline `8128951`.

use crate::contracts::validate_error_envelope;
use libloading::Library;
use serde_json::json;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use uuid::Uuid;

pub const REQUIRED_HOST_SYMBOLS: [&str; 6] = [
    "fabushi_host_create",
    "fabushi_host_dispatch",
    "fabushi_host_poll_events",
    "fabushi_host_cancel",
    "fabushi_host_destroy",
    "fabushi_host_free_string",
];

pub const MUTANT_STUB_SENTINEL_SYMBOL: &str = "fabushi_e2e_is_mutant_stub";
pub const STARTUP_PROBE_UNKNOWN_METHOD: &str = "__fabushi_e2e_probe__.unknown_command_guard";

pub type FabushiHostCreateFn = unsafe extern "C" fn(config_json: *const c_char) -> *mut c_void;
pub type FabushiHostDispatchFn =
    unsafe extern "C" fn(handle: *mut c_void, request_json: *const c_char) -> *mut c_char;
pub type FabushiHostPollEventsFn =
    unsafe extern "C" fn(handle: *mut c_void, max_count: u32, timeout_ms: u32) -> *mut c_char;
pub type FabushiHostCancelFn =
    unsafe extern "C" fn(handle: *mut c_void, request_id: *const c_char) -> bool;
pub type FabushiHostDestroyFn = unsafe extern "C" fn(handle: *mut c_void);
pub type FabushiHostFreeStringFn = unsafe extern "C" fn(ptr: *mut c_char);
pub type MutantStubSentinelFn = unsafe extern "C" fn() -> u32;

type LegacyHostCreateTestFn = unsafe extern "C" fn(app_data_dir: *const c_char) -> *mut c_void;
type LegacyHostDispatchWithHandleFn =
    unsafe extern "C" fn(host: *mut c_void, request_json: *const c_char) -> *mut c_char;
type LegacyHostFreeStringFn = unsafe extern "C" fn(pointer: *mut c_char);
type LegacyHostDestroyFn = unsafe extern "C" fn(host: *mut c_void);

/// Panic-safe RAII guard ensuring every `*mut c_char` returned by a host cdylib
/// is freed by that exact cdylib's `free_string` function (never crossing allocators).
struct OwnedHostString {
    ptr: *mut c_char,
    free_fn: FabushiHostFreeStringFn,
}

impl OwnedHostString {
    fn from_raw(ptr: *mut c_char, free_fn: FabushiHostFreeStringFn) -> Result<Self, String> {
        if ptr.is_null() {
            return Err("FFI host returned NULL string pointer".to_owned());
        }
        Ok(Self { ptr, free_fn })
    }

    fn to_string_checked(&self) -> Result<String, String> {
        // SAFETY: `self.ptr` is non-null and points to a NUL-terminated C string allocated by the host.
        let cstr = unsafe { CStr::from_ptr(self.ptr) };
        cstr.to_str()
            .map(str::to_owned)
            .map_err(|err| format!("FFI host returned invalid UTF-8: {err}"))
    }
}

impl Drop for OwnedHostString {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: `self.ptr` was allocated by the same loaded cdylib whose `free_fn` we call.
            unsafe {
                (self.free_fn)(self.ptr);
            }
            self.ptr = std::ptr::null_mut();
        }
    }
}

/// Inspects a shared library and returns the list of missing required `fabushi_host_*` symbols.
pub fn probe_required_symbols(lib_path: &Path) -> Result<Vec<&'static str>, String> {
    if !lib_path.exists() {
        return Err(format!("host library not found at {}", lib_path.display()));
    }
    // SAFETY: Caller supplies path to a native shared library to inspect via `dlsym`.
    let library = unsafe { Library::new(lib_path) }
        .map_err(|err| format!("failed to load {}: {err}", lib_path.display()))?;

    let mut missing = Vec::new();
    for sym_name in REQUIRED_HOST_SYMBOLS {
        let nul_name = format!("{sym_name}\0");
        // SAFETY: Checking symbol existence without invoking it.
        let found = unsafe { library.get::<*const c_void>(nul_name.as_bytes()) }.is_ok();
        if !found {
            missing.push(sym_name);
        }
    }
    Ok(missing)
}

fn path_contains_mutant_marker(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    lower.contains("mutant_stub") || lower.contains("mutant-stub")
}

pub fn is_mutant_stub_path(lib_path: &Path) -> bool {
    if path_contains_mutant_marker(lib_path) {
        return true;
    }
    if let Ok(canonical) = std::fs::canonicalize(lib_path) {
        if path_contains_mutant_marker(&canonical) {
            return true;
        }
    }
    false
}

/// Safe dynamic wrapper around the 6 canonical `fabushi_host_*` C-ABI symbols.
///
/// Field ordering invariant: `handle` is explicitly destroyed in `Drop::drop` BEFORE `_library`
/// is dropped by Rust, preventing `dlclose` while host worker threads are active.
pub struct FfiHostClient {
    handle: *mut c_void,
    initial_generation: u64,
    fn_dispatch: FabushiHostDispatchFn,
    fn_poll_events: FabushiHostPollEventsFn,
    fn_cancel: FabushiHostCancelFn,
    fn_destroy: FabushiHostDestroyFn,
    fn_free_string: FabushiHostFreeStringFn,
    _library: Library,
}

// SAFETY: The opaque host handle is thread-safe across FFI calls and is only destroyed via `&mut self` in `Drop`.
unsafe impl Send for FfiHostClient {}
// SAFETY: Concurrent `fabushi_host_dispatch` / `poll_events` / `cancel` calls are supported by the host C-ABI contract.
unsafe impl Sync for FfiHostClient {}

impl FfiHostClient {
    /// Production loader with 3-layer fail-closed anti-stub gate:
    /// 1. Canonicalized path check rejecting `mutant_stub`/`mutant-stub` (including symlinks).
    /// 2. `dlsym` probe refusing any cdylib exporting `fabushi_e2e_is_mutant_stub` (even if copied/renamed).
    /// 3. Startup behavioral probe verifying unknown methods fail closed with `unknown_command` + `retryable == false`.
    pub fn load_production(lib_path: &Path, config_json: &str) -> Result<Self, String> {
        if is_mutant_stub_path(lib_path) {
            return Err(format!(
                "FAIL-CLOSED: connect_production() refuses mutant_stub library: {}",
                lib_path.display()
            ));
        }
        let mut client = Self::load_internal(lib_path, config_json, true)?;
        let authoritative_gen = client.verify_startup_antistub_probe()?;
        client.initial_generation = authoritative_gen;
        Ok(client)
    }

    /// Anti-stub oracle loader: strictly for `tests/self_validate_antistub.rs` to validate
    /// that our contract assertions catch synthetic `mutant_stub` defects.
    pub fn load_for_antistub_oracle(lib_path: &Path, config_json: &str) -> Result<Self, String> {
        Self::load_internal(lib_path, config_json, false)
    }

    fn load_internal(
        lib_path: &Path,
        config_json: &str,
        production_mode: bool,
    ) -> Result<Self, String> {
        if !lib_path.exists() {
            return Err(format!(
                "FAIL-CLOSED: host shared library does not exist at {}",
                lib_path.display()
            ));
        }
        let resolved_path =
            std::fs::canonicalize(lib_path).unwrap_or_else(|_| lib_path.to_path_buf());
        if production_mode && path_contains_mutant_marker(&resolved_path) {
            return Err(format!(
                "FAIL-CLOSED: connect_production() refuses mutant_stub library: {}",
                resolved_path.display()
            ));
        }

        // SAFETY: Loading a native shared library from an explicitly verified path.
        let library = unsafe { Library::new(&resolved_path) }
            .map_err(|err| format!("failed to dlopen {}: {err}", resolved_path.display()))?;

        if production_mode {
            // SAFETY: Probing `dlsym` for the negative-oracle sentinel symbol without invoking it.
            // Any cdylib exporting `fabushi_e2e_is_mutant_stub` is rejected immediately even if copied/renamed.
            let has_sentinel =
                unsafe { library.get::<MutantStubSentinelFn>(b"fabushi_e2e_is_mutant_stub\0") }
                    .is_ok();
            if has_sentinel {
                return Err(format!(
                    "FAIL-CLOSED: Refusing to load negative-oracle mutant_stub cdylib in production mode \
                     (detected sentinel symbol `{MUTANT_STUB_SENTINEL_SYMBOL}` in {})",
                    resolved_path.display()
                ));
            }
        }

        // SAFETY: Resolving the 6 canonical C-ABI symbol signatures defined in PROJECT.md.
        let fn_create: FabushiHostCreateFn = unsafe {
            *library
                .get::<FabushiHostCreateFn>(b"fabushi_host_create\0")
                .map_err(|err| format!("missing symbol fabushi_host_create: {err}"))?
        };
        let fn_dispatch: FabushiHostDispatchFn = unsafe {
            *library
                .get::<FabushiHostDispatchFn>(b"fabushi_host_dispatch\0")
                .map_err(|err| format!("missing symbol fabushi_host_dispatch: {err}"))?
        };
        let fn_poll_events: FabushiHostPollEventsFn = unsafe {
            *library
                .get::<FabushiHostPollEventsFn>(b"fabushi_host_poll_events\0")
                .map_err(|err| format!("missing symbol fabushi_host_poll_events: {err}"))?
        };
        let fn_cancel: FabushiHostCancelFn = unsafe {
            *library
                .get::<FabushiHostCancelFn>(b"fabushi_host_cancel\0")
                .map_err(|err| format!("missing symbol fabushi_host_cancel: {err}"))?
        };
        let fn_destroy: FabushiHostDestroyFn = unsafe {
            *library
                .get::<FabushiHostDestroyFn>(b"fabushi_host_destroy\0")
                .map_err(|err| format!("missing symbol fabushi_host_destroy: {err}"))?
        };
        let fn_free_string: FabushiHostFreeStringFn = unsafe {
            *library
                .get::<FabushiHostFreeStringFn>(b"fabushi_host_free_string\0")
                .map_err(|err| format!("missing symbol fabushi_host_free_string: {err}"))?
        };

        let c_config = CString::new(config_json)
            .map_err(|err| format!("config_json contains interior NUL byte: {err}"))?;

        // SAFETY: Invoking `fabushi_host_create` with valid NUL-terminated UTF-8 config JSON.
        let handle = unsafe { fn_create(c_config.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "fabushi_host_create returned NULL handle for {}",
                resolved_path.display()
            ));
        }

        Ok(Self {
            handle,
            initial_generation: 1,
            fn_dispatch,
            fn_poll_events,
            fn_cancel,
            fn_destroy,
            fn_free_string,
            _library: library,
        })
    }

    pub fn initial_generation(&self) -> u64 {
        self.initial_generation
    }

    /// Startup fail-closed behavioral anti-stub gate.
    ///
    /// Dispatches a synthetic unknown method with random UUID suffix under
    /// `__fabushi_e2e_probe__.unknown_command_guard` and asserts the host fails closed with:
    /// - `ok == false`
    /// - `error.code == "unknown_command"`
    /// - `error.retryable == false`
    /// - zero `"acknowledged"` stub token in the raw response
    ///
    /// Returns the host's authoritative `generation` (handles persisted `generation > 1` on `restart_cold()`
    /// via a single bounded retry if attempt 0 receives fail-closed `stale_generation`).
    pub fn verify_startup_antistub_probe(&self) -> Result<u64, String> {
        let mut probe_gen = 1u64;
        for attempt in 0..2 {
            let probe_id = format!("e2e-antistub-probe-{}", Uuid::new_v4());
            let probe_method = format!(
                "{STARTUP_PROBE_UNKNOWN_METHOD}.{}",
                Uuid::new_v4().simple()
            );
            let req = json!({
                "id": probe_id,
                "method": probe_method,
                "generation": probe_gen,
                "params": {}
            });
            let raw = self.dispatch_raw(&req.to_string()).map_err(|err| {
                format!("FAIL-CLOSED: startup anti-stub probe dispatch failed: {err}")
            })?;

            if raw.to_ascii_lowercase().contains("\"acknowledged\"") {
                return Err(format!(
                    "FAIL-CLOSED: startup anti-stub probe detected fail-open `acknowledged` stub response: {raw}"
                ));
            }

            // If a cold-restarted host persisted generation > 1 and checks generation before method dispatch,
            // retry once at the host's authoritative generation.
            if attempt == 0 {
                if let Ok((resp, err)) =
                    validate_error_envelope(&raw, &probe_id, &["stale_generation"])
                {
                    if !err.retryable && resp.generation > probe_gen {
                        probe_gen = resp.generation;
                        continue;
                    }
                }
            }

            let (resp, err) =
                validate_error_envelope(&raw, &probe_id, &["unknown_command"]).map_err(|e| {
                    format!(
                        "FAIL-CLOSED: startup anti-stub probe failed for `{STARTUP_PROBE_UNKNOWN_METHOD}`: {e}"
                    )
                })?;

            if err.retryable {
                return Err(format!(
                    "FAIL-CLOSED: startup anti-stub probe requires `unknown_command` error.retryable == false, got true (raw={raw})"
                ));
            }
            return Ok(resp.generation.max(1));
        }
        Err("FAIL-CLOSED: startup anti-stub probe exhausted retry after stale_generation".to_owned())
    }

    pub fn dispatch_raw(&self, request_json: &str) -> Result<String, String> {
        let c_req = CString::new(request_json)
            .map_err(|err| format!("request_json contains interior NUL byte: {err}"))?;
        // SAFETY: `self.handle` is live and `c_req` is a valid NUL-terminated C string.
        let raw_ptr = unsafe { (self.fn_dispatch)(self.handle, c_req.as_ptr()) };
        let owned = OwnedHostString::from_raw(raw_ptr, self.fn_free_string)?;
        owned.to_string_checked()
    }

    pub fn dispatch_null_handle(&self, request_json: &str) -> Result<String, String> {
        let c_req = CString::new(request_json)
            .map_err(|err| format!("request_json contains interior NUL byte: {err}"))?;
        // SAFETY: Testing C-ABI null-handle fail-closed handling.
        let raw_ptr = unsafe { (self.fn_dispatch)(std::ptr::null_mut(), c_req.as_ptr()) };
        let owned = OwnedHostString::from_raw(raw_ptr, self.fn_free_string)?;
        owned.to_string_checked()
    }

    pub fn dispatch_null_request(&self) -> Result<String, String> {
        // SAFETY: Testing C-ABI null-request fail-closed handling.
        let raw_ptr = unsafe { (self.fn_dispatch)(self.handle, std::ptr::null()) };
        let owned = OwnedHostString::from_raw(raw_ptr, self.fn_free_string)?;
        owned.to_string_checked()
    }

    pub fn dispatch_invalid_utf8(&self) -> Result<String, String> {
        let bad_bytes: [u8; 4] = [0xFF, 0xFE, 0xFD, 0x00];
        // SAFETY: `bad_bytes` is NUL-terminated invalid UTF-8 used to verify fail-closed UTF-8 decoding.
        let raw_ptr = unsafe { (self.fn_dispatch)(self.handle, bad_bytes.as_ptr().cast::<c_char>()) };
        let owned = OwnedHostString::from_raw(raw_ptr, self.fn_free_string)?;
        owned.to_string_checked()
    }

    pub fn poll_events_raw(&self, max_count: u32, timeout_ms: u32) -> Result<String, String> {
        // SAFETY: `self.handle` is live and valid.
        let raw_ptr = unsafe { (self.fn_poll_events)(self.handle, max_count, timeout_ms) };
        let owned = OwnedHostString::from_raw(raw_ptr, self.fn_free_string)?;
        owned.to_string_checked()
    }

    pub fn poll_events_null_handle(&self) -> Result<String, String> {
        // SAFETY: Testing C-ABI null-handle fail-closed handling on poll_events.
        let raw_ptr = unsafe { (self.fn_poll_events)(std::ptr::null_mut(), 4, 1) };
        let owned = OwnedHostString::from_raw(raw_ptr, self.fn_free_string)?;
        owned.to_string_checked()
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        let Ok(c_id) = CString::new(request_id) else {
            return false;
        };
        // SAFETY: `self.handle` is live and `c_id` is a valid NUL-terminated C string.
        unsafe { (self.fn_cancel)(self.handle, c_id.as_ptr()) }
    }

    pub fn cancel_null_handle(&self, request_id: &str) -> bool {
        let Ok(c_id) = CString::new(request_id) else {
            return false;
        };
        // SAFETY: Testing null-handle fail-closed safety on cancel.
        unsafe { (self.fn_cancel)(std::ptr::null_mut(), c_id.as_ptr()) }
    }

    pub fn free_null_string(&self) {
        // SAFETY: C-ABI contract requires `fabushi_host_free_string(NULL)` to be a safe no-op.
        unsafe { (self.fn_free_string)(std::ptr::null_mut()) };
    }
}

impl Drop for FfiHostClient {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: Destroy host handle cleanly BEFORE `_library` drops (`dlclose`).
            unsafe {
                (self.fn_destroy)(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

/// Dedicated probe for baseline `8128951` `libmahayana_app_host.so` used ONLY by
/// `tests/self_validate_antistub.rs` to empirically prove the baseline stub is caught.
pub struct LegacyHost8128951Probe {
    handle: *mut c_void,
    fn_dispatch: LegacyHostDispatchWithHandleFn,
    fn_free_string: LegacyHostFreeStringFn,
    fn_destroy: LegacyHostDestroyFn,
    _library: Library,
}

impl LegacyHost8128951Probe {
    pub fn load(lib_path: &Path, app_data_dir: &Path) -> Result<Self, String> {
        if !lib_path.exists() {
            return Err(format!(
                "baseline 8128951 library does not exist at {}",
                lib_path.display()
            ));
        }
        // SAFETY: Loading the compiled baseline `libmahayana_app_host.so` for negative-proof verification.
        let library = unsafe { Library::new(lib_path) }
            .map_err(|err| format!("failed to dlopen {}: {err}", lib_path.display()))?;

        // SAFETY: Binding exact legacy 8128951 C-ABI signatures from `source/host/src/lib.rs`.
        let fn_create_test: LegacyHostCreateTestFn = unsafe {
            *library
                .get::<LegacyHostCreateTestFn>(b"telegram_app_host_create_test\0")
                .map_err(|err| format!("missing legacy symbol telegram_app_host_create_test: {err}"))?
        };
        let fn_dispatch: LegacyHostDispatchWithHandleFn = unsafe {
            *library
                .get::<LegacyHostDispatchWithHandleFn>(b"telegram_app_host_dispatch_with_handle\0")
                .map_err(|err| {
                    format!("missing legacy symbol telegram_app_host_dispatch_with_handle: {err}")
                })?
        };
        let fn_free_string: LegacyHostFreeStringFn = unsafe {
            *library
                .get::<LegacyHostFreeStringFn>(b"telegram_app_host_free_string\0")
                .map_err(|err| format!("missing legacy symbol telegram_app_host_free_string: {err}"))?
        };
        let fn_destroy: LegacyHostDestroyFn = unsafe {
            *library
                .get::<LegacyHostDestroyFn>(b"telegram_app_host_destroy\0")
                .map_err(|err| format!("missing legacy symbol telegram_app_host_destroy: {err}"))?
        };

        let dir_str = app_data_dir.to_string_lossy().into_owned();
        let c_dir = CString::new(dir_str)
            .map_err(|err| format!("app_data_dir contains NUL: {err}"))?;

        // SAFETY: Creating baseline test host with isolated temp dir.
        let handle = unsafe { fn_create_test(c_dir.as_ptr()) };
        if handle.is_null() {
            return Err("telegram_app_host_create_test returned NULL handle".to_owned());
        }

        Ok(Self {
            handle,
            fn_dispatch,
            fn_free_string,
            fn_destroy,
            _library: library,
        })
    }

    pub fn dispatch(&self, request_json: &str) -> Result<String, String> {
        let c_req = CString::new(request_json)
            .map_err(|err| format!("request_json contains NUL: {err}"))?;
        // SAFETY: `self.handle` is live and `c_req` is valid NUL-terminated UTF-8.
        let raw_ptr = unsafe { (self.fn_dispatch)(self.handle, c_req.as_ptr()) };
        let owned = OwnedHostString::from_raw(raw_ptr, self.fn_free_string)?;
        owned.to_string_checked()
    }
}

impl Drop for LegacyHost8128951Probe {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: Must destroy baseline Tokio host BEFORE `_library` unmaps the `.so`.
            unsafe {
                (self.fn_destroy)(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}
