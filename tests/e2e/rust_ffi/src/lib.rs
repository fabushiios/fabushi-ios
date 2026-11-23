//! Standalone opaque-box Rust E2E harness for `Fabushi v1.6.7` (`bhrumom/gpui-telegram@6141456e`).
//!
//! Zero internal Cargo path dependencies on `source/host` or `source/desktop_core`.
//! Every production Tier 1–4 test calls `FabushiE2EHarness::connect_production()`, which
//! STRICTLY requires a real compiled `FABUSHI_HOST_LIB` cdylib exporting all 6 canonical
//! `fabushi_host_*` symbols and FAILS CLOSED with zero mock fallback.

pub mod contracts;
pub mod ffi_client;
pub mod loopback_server;

use contracts::{
    EventPollEnvelope, RpcError, RpcRequest, RpcResponse, validate_envelope,
    validate_error_envelope, validate_success_envelope,
};
use ffi_client::FfiHostClient;
use loopback_server::LoopbackServer;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::TempDir;
use uuid::Uuid;

pub const ENV_FABUSHI_HOST_LIB: &str = "FABUSHI_HOST_LIB";
pub const ENV_FABUSHI_MUTANT_STUB_LIB: &str = "FABUSHI_MUTANT_STUB_LIB";
pub const ENV_FABUSHI_BASELINE_HOST_LIB: &str = "FABUSHI_BASELINE_HOST_LIB";
pub const ENV_FABUSHI_REQUIRE_BASELINE_PROOF: &str = "FABUSHI_REQUIRE_BASELINE_PROOF";
pub const ENV_FABUSHI_ALLOW_SKIP_BASELINE_PROOF: &str = "FABUSHI_ALLOW_SKIP_BASELINE_PROOF";

/// Secondary isolated host handle connected to the same `LoopbackServer` (for cross-device
/// QR pairing, X25519 vault sync, E2EE 1:1/group ratchet, and session revocation tests).
pub struct SecondaryHostHandle {
    client: Option<FfiHostClient>,
    _temp_dir: TempDir,
    generation: AtomicU64,
    account_id: Mutex<Option<String>>,
}

impl SecondaryHostHandle {
    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn dispatch(&self, method: &str, params: Value) -> Result<RpcResponse, String> {
        let current_gen = self.generation.load(Ordering::SeqCst);
        let acct = self.account_id.lock().ok().and_then(|g| g.clone());
        let id = Uuid::new_v4().to_string();
        let req = RpcRequest {
            id: id.clone(),
            method: method.to_owned(),
            generation: current_gen,
            account_id: acct,
            params,
        };
        let raw = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        let resp_raw = self
            .client
            .as_ref()
            .ok_or_else(|| "secondary host destroyed".to_owned())?
            .dispatch_raw(&raw)?;
        let parsed = validate_envelope(&resp_raw, &id)?;
        self.generation.fetch_max(parsed.generation, Ordering::SeqCst);
        Ok(parsed)
    }

    pub fn expect_ok(&self, method: &str, params: Value) -> Value {
        let current_gen = self.generation.load(Ordering::SeqCst);
        let acct = self.account_id.lock().ok().and_then(|g| g.clone());
        let id = Uuid::new_v4().to_string();
        let req = RpcRequest {
            id: id.clone(),
            method: method.to_owned(),
            generation: current_gen,
            account_id: acct,
            params,
        };
        let raw = serde_json::to_string(&req).expect("serialize request");
        let resp_raw = self
            .client
            .as_ref()
            .expect("secondary host live")
            .dispatch_raw(&raw)
            .expect("secondary FFI dispatch");
        let (resp, result) = validate_success_envelope(&resp_raw, &id)
            .unwrap_or_else(|e| panic!("secondary `{method}` failed: {e}"));
        self.generation.fetch_max(resp.generation, Ordering::SeqCst);
        result
    }

    pub fn expect_err(&self, method: &str, params: Value, allowed_codes: &[&str]) -> RpcError {
        let current_gen = self.generation.load(Ordering::SeqCst);
        let acct = self.account_id.lock().ok().and_then(|g| g.clone());
        let id = Uuid::new_v4().to_string();
        let req = RpcRequest {
            id: id.clone(),
            method: method.to_owned(),
            generation: current_gen,
            account_id: acct,
            params,
        };
        let raw = serde_json::to_string(&req).expect("serialize request");
        let resp_raw = self
            .client
            .as_ref()
            .expect("secondary host live")
            .dispatch_raw(&raw)
            .expect("secondary FFI dispatch");
        let (resp, err) = validate_error_envelope(&resp_raw, &id, allowed_codes)
            .unwrap_or_else(|e| panic!("secondary `{method}` expected error: {e}"));
        self.generation.fetch_max(resp.generation, Ordering::SeqCst);
        err
    }

    pub fn poll_events(&self, max_count: u32, timeout_ms: u32) -> EventPollEnvelope {
        let raw = self
            .client
            .as_ref()
            .expect("secondary host live")
            .poll_events_raw(max_count, timeout_ms)
            .expect("secondary poll_events");
        let env: EventPollEnvelope = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("secondary poll_events invalid JSON ({e}): {raw}"));
        self.generation.fetch_max(env.generation, Ordering::SeqCst);
        env
    }
}

impl Drop for SecondaryHostHandle {
    fn drop(&mut self) {
        // Explicitly destroy FFI host BEFORE `_temp_dir` cleans up.
        let _ = self.client.take();
    }
}

/// Primary opaque-box E2E harness owning an isolated `FfiHostClient`, real ephemeral
/// `LoopbackServer` (`127.0.0.1:0`), and isolated `TempDir` (`app_data_dir`).
///
/// Drop ordering invariant: `client` is explicitly dropped BEFORE `server` and `temp_dir`
/// so the host finishes clean SQLite WAL flush and shuts down worker threads while the
/// loopback server and data directory are still alive.
pub struct FabushiE2EHarness {
    client: Option<FfiHostClient>,
    lib_path: PathBuf,
    server: LoopbackServer,
    temp_dir: TempDir,
    generation: AtomicU64,
    account_id: Mutex<Option<String>>,
}

impl FabushiE2EHarness {
    /// Production opaque-box entrypoint required by ALL 394 Tier 1–4 tests.
    ///
    /// Strictly reads `FABUSHI_HOST_LIB`, refuses any `mutant_stub` library (via canonical path,
    /// `fabushi_e2e_is_mutant_stub` sentinel symbol, and startup behavioral unknown-command probe),
    /// requires all 6 `fabushi_host_*` symbols, boots a real loopback server and temp dir,
    /// and FAILS CLOSED (panics) if any precondition is unmet. ZERO mock fallback!
    pub fn connect_production() -> Self {
        Self::try_connect_production().unwrap_or_else(|err| {
            panic!("FabushiE2EHarness::connect_production() FAILED CLOSED: {err}")
        })
    }

    pub fn try_connect_production() -> Result<Self, String> {
        let raw_path = std::env::var(ENV_FABUSHI_HOST_LIB).ok();
        Self::try_connect_production_from_path_opt(raw_path.as_deref())
    }

    /// Pure env-value parser and production loader enabling thread-safe fail-closed tests
    /// without any `unsafe { std::env::set_var / remove_var }` process environment mutation.
    pub fn try_connect_production_from_path_opt(raw_path: Option<&str>) -> Result<Self, String> {
        let raw = raw_path.ok_or_else(|| {
            format!(
                "{ENV_FABUSHI_HOST_LIB} is not set; production E2E tests require a real compiled libmahayana_app_host.{{so,dylib}} (zero mock fallback)"
            )
        })?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(format!("{ENV_FABUSHI_HOST_LIB} is empty"));
        }
        Self::try_connect_production_at(Path::new(trimmed))
    }

    pub fn try_connect_production_at(lib_path: &Path) -> Result<Self, String> {
        let server = LoopbackServer::start()?;
        let temp_dir = TempDir::new()
            .map_err(|err| format!("failed to create isolated E2E TempDir: {err}"))?;
        let config_json =
            build_host_config_json(temp_dir.path(), &server.base_url(), "iOS E2E Primary");
        let client = FfiHostClient::load_production(lib_path, &config_json)?;
        let initial_gen = client.initial_generation();

        Ok(Self {
            client: Some(client),
            lib_path: lib_path.to_path_buf(),
            server,
            temp_dir,
            generation: AtomicU64::new(initial_gen),
            account_id: Mutex::new(None),
        })
    }

    pub fn client(&self) -> &FfiHostClient {
        self.client
            .as_ref()
            .expect("FabushiE2EHarness FFI client is active")
    }

    pub fn server(&self) -> &LoopbackServer {
        &self.server
    }

    pub fn server_url(&self) -> String {
        self.server.base_url()
    }

    pub fn app_data_dir(&self) -> &Path {
        self.temp_dir.path()
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn set_generation(&self, generation: u64) {
        self.generation.store(generation, Ordering::SeqCst);
    }

    pub fn set_account_id(&self, account_id: Option<String>) {
        if let Ok(mut guard) = self.account_id.lock() {
            *guard = account_id;
        }
    }

    pub fn dispatch(&self, method: &str, params: Value) -> Result<RpcResponse, String> {
        let current_gen = self.generation.load(Ordering::SeqCst);
        let acct = self.account_id.lock().ok().and_then(|g| g.clone());
        self.dispatch_with_gen(method, current_gen, acct.as_deref(), params)
    }

    pub fn dispatch_with_gen(
        &self,
        method: &str,
        generation: u64,
        account_id: Option<&str>,
        params: Value,
    ) -> Result<RpcResponse, String> {
        let id = Uuid::new_v4().to_string();
        let req = RpcRequest {
            id: id.clone(),
            method: method.to_owned(),
            generation,
            account_id: account_id.map(str::to_owned),
            params,
        };
        let raw = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        let resp_raw = self.client().dispatch_raw(&raw)?;
        let parsed = validate_envelope(&resp_raw, &id)?;
        self.generation.fetch_max(parsed.generation, Ordering::SeqCst);
        Ok(parsed)
    }

    pub fn dispatch_raw(&self, raw_json: &str) -> Result<String, String> {
        self.client().dispatch_raw(raw_json)
    }

    pub fn expect_ok(&self, method: &str, params: Value) -> Value {
        let (_id, _resp, result) = self.expect_ok_with_meta(method, params);
        result
    }

    pub fn expect_ok_with_meta(&self, method: &str, params: Value) -> (String, RpcResponse, Value) {
        let current_gen = self.generation.load(Ordering::SeqCst);
        let acct = self.account_id.lock().ok().and_then(|g| g.clone());
        let id = Uuid::new_v4().to_string();
        let req = RpcRequest {
            id: id.clone(),
            method: method.to_owned(),
            generation: current_gen,
            account_id: acct,
            params,
        };
        let raw = serde_json::to_string(&req).expect("serialize RpcRequest");
        let resp_raw = self
            .client()
            .dispatch_raw(&raw)
            .unwrap_or_else(|e| panic!("FFI dispatch failed for `{method}`: {e}"));
        let (resp, result) = validate_success_envelope(&resp_raw, &id)
            .unwrap_or_else(|e| panic!("expected ok=true for `{method}`: {e}"));
        self.generation.fetch_max(resp.generation, Ordering::SeqCst);
        (id, resp, result)
    }

    pub fn expect_ok_with_gen(
        &self,
        method: &str,
        generation: u64,
        account_id: Option<&str>,
        params: Value,
    ) -> Value {
        let id = Uuid::new_v4().to_string();
        let req = RpcRequest {
            id: id.clone(),
            method: method.to_owned(),
            generation,
            account_id: account_id.map(str::to_owned),
            params,
        };
        let raw = serde_json::to_string(&req).expect("serialize RpcRequest");
        let resp_raw = self
            .client()
            .dispatch_raw(&raw)
            .unwrap_or_else(|e| panic!("FFI dispatch failed for `{method}`: {e}"));
        let (resp, result) = validate_success_envelope(&resp_raw, &id)
            .unwrap_or_else(|e| panic!("expected ok=true for `{method}`: {e}"));
        self.generation.fetch_max(resp.generation, Ordering::SeqCst);
        result
    }

    pub fn expect_err(&self, method: &str, params: Value, allowed_codes: &[&str]) -> RpcError {
        let current_gen = self.generation.load(Ordering::SeqCst);
        let acct = self.account_id.lock().ok().and_then(|g| g.clone());
        self.expect_err_with_gen(method, current_gen, acct.as_deref(), params, allowed_codes)
    }

    pub fn expect_err_with_gen(
        &self,
        method: &str,
        generation: u64,
        account_id: Option<&str>,
        params: Value,
        allowed_codes: &[&str],
    ) -> RpcError {
        let id = Uuid::new_v4().to_string();
        let req = RpcRequest {
            id: id.clone(),
            method: method.to_owned(),
            generation,
            account_id: account_id.map(str::to_owned),
            params,
        };
        let raw = serde_json::to_string(&req).expect("serialize RpcRequest");
        let resp_raw = self
            .client()
            .dispatch_raw(&raw)
            .unwrap_or_else(|e| panic!("FFI dispatch failed for `{method}`: {e}"));
        let (resp, err) = validate_error_envelope(&resp_raw, &id, allowed_codes)
            .unwrap_or_else(|e| panic!("expected fail-closed error for `{method}`: {e}"));
        self.generation.fetch_max(resp.generation, Ordering::SeqCst);
        err
    }

    pub fn poll_events(&self, max_count: u32, timeout_ms: u32) -> EventPollEnvelope {
        let raw = self
            .client()
            .poll_events_raw(max_count, timeout_ms)
            .unwrap_or_else(|e| panic!("fabushi_host_poll_events failed: {e}"));
        let env: EventPollEnvelope = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("fabushi_host_poll_events invalid JSON ({e}): {raw}"));
        self.generation.fetch_max(env.generation, Ordering::SeqCst);
        env
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        self.client().cancel(request_id)
    }

    /// Destroys the active host handle and recreates a fresh host handle pointing at the
    /// EXACT SAME `app_data_dir` and `server_url` to verify real SQLite WAL / Tantivy /
    /// session / vault persistence across cold restart.
    pub fn restart_cold(&mut self) {
        let _ = self.client.take();
        let config_json = build_host_config_json(
            self.temp_dir.path(),
            &self.server.base_url(),
            "iOS E2E Primary",
        );
        let new_client = FfiHostClient::load_production(&self.lib_path, &config_json)
            .unwrap_or_else(|e| panic!("restart_cold failed to reload host: {e}"));
        let next_gen = new_client.initial_generation();
        self.client = Some(new_client);
        self.generation.store(next_gen, Ordering::SeqCst);
    }

    /// Spawns a second isolated host instance connected to the SAME `LoopbackServer`
    /// for dual-device E2E flows (QR pairing, X25519 vault sync, E2EE ratchet, session revoke).
    pub fn spawn_secondary_host(&self, device_name: &str) -> SecondaryHostHandle {
        let temp_dir = TempDir::new().expect("create secondary TempDir");
        let config_json =
            build_host_config_json(temp_dir.path(), &self.server.base_url(), device_name);
        let client = FfiHostClient::load_production(&self.lib_path, &config_json)
            .unwrap_or_else(|e| panic!("spawn_secondary_host failed: {e}"));
        let initial_gen = client.initial_generation();
        SecondaryHostHandle {
            client: Some(client),
            _temp_dir: temp_dir,
            generation: AtomicU64::new(initial_gen),
            account_id: Mutex::new(None),
        }
    }
}

impl Drop for FabushiE2EHarness {
    fn drop(&mut self) {
        // Destroy FFI host BEFORE `server` and `temp_dir` are dropped.
        let _ = self.client.take();
    }
}

pub fn build_host_config_json(app_data_dir: &Path, server_url: &str, device_name: &str) -> String {
    json!({
        "app_data_dir": app_data_dir.to_string_lossy(),
        "server_url": server_url,
        "device_name": device_name,
        "platform": "ios",
        "ws_ping_interval_secs": 25,
        "http_timeout_secs": 10
    })
    .to_string()
}

/// Checks recursively that no regular file under `dir` contains `needle` in plaintext
/// (used to verify passwords, PINs, and vault secrets are never persisted unencrypted).
pub fn assert_dir_excludes_plaintext_secret(dir: &Path, needle: &str) {
    fn visit(path: &Path, needle_bytes: &[u8]) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                visit(&p, needle_bytes);
            } else if p.is_file() {
                if let Ok(bytes) = std::fs::read(&p) {
                    if bytes.windows(needle_bytes.len()).any(|w| w == needle_bytes) {
                        panic!(
                            "SECURITY VIOLATION: plaintext secret leaked on disk at {}",
                            p.display()
                        );
                    }
                }
            }
        }
    }
    if !needle.is_empty() {
        visit(dir, needle.as_bytes());
    }
}
