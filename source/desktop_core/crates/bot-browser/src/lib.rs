pub mod backend;
pub mod cdp;
pub mod e2ee_sync;
pub mod orchestrator;
pub mod script_app;
pub mod session_import;
pub mod sidecar;
pub mod totp;
pub mod types;
pub mod vault;

pub use backend::{
    ActionOutcome, BrowserPerformanceMetrics, BrowserSessionBackend, CdpBrowserBackend,
    ChromeLaunchConfig, KeyEventKind, KeyInputEvent, NavigationResult, OffscreenFrameBuffer,
    PointerButton, PointerEventKind, PointerInputEvent,
};
pub use e2ee_sync::*;
pub use script_app::{
    AutomationScriptApp, BrowserActionTrace, BrowserControlGate, BrowserControlMode,
    CredentialVaultSecretResolver, DeterministicScriptRunner, InMemoryVaultSecretResolver,
    RecordedActionKind, RecordedBrowserAction, SCRIPT_APP_SCHEMA_VERSION, ScriptAppMetadata,
    ScriptCompiler, ScriptParameter, ScriptReplayReport, ScriptStep, TraceParameterBinding,
    VaultSecretResolver, scan_json_value_for_secret_leaks, scan_text_for_secret_leaks,
    validate_script_app_safe_for_publish,
};
pub use session_import::{
    BrowserProfileData, BrowserProfileStore, ChromeImportOptions, ChromeImportProbe,
    ChromeImportReport, ChromeKeyProvider, LocalStorageMap, NoopChromeKeyProvider,
    StaticChromeKeyProvider, SystemChromeKeyProvider, classify_probe_io_error,
    default_chrome_cookie_candidates, import_cookies_from_chrome_sqlite, import_cookies_from_json,
    import_cookies_from_netscape, import_cookies_from_text, probe_chrome_cookies_at,
    probe_local_chrome_cookies,
};
pub use sidecar::ProfileNamespace;
pub use totp::{generate_totp_at, generate_totp_now, generate_totp_with_defaults_at};
pub use types::*;
pub use vault::{
    CredentialVault, FileMasterKeyStore, MasterKeyStore, MemoryMasterKeyStore,
    SystemKeyringMasterKeyStore, VaultSyncCredentialPayload, mask_account_hint,
};
