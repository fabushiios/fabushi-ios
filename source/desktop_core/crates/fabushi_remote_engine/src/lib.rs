//! Fabushi Remote Engine — cross-platform low-latency remote desktop, persistent PTY mux,
//! local IPC, hardware-accelerated codec/ABR, HID input, audio, clipboard, file transfer,
//! accessibility, display/privacy, and power management engine.

pub mod accessibility;
pub mod audio;
pub mod capture;
pub mod clipboard;
pub mod codec;
pub mod control_lease;
pub mod display_privacy;
pub mod error;
pub mod file_transfer;
pub mod input;
pub mod ipc;
pub mod macos_permissions;
pub mod power;
pub mod terminal;

pub use error::{EngineResult, RemoteEngineError};

use std::sync::atomic::{AtomicBool, Ordering};

static HEADLESS_TEST_MODE: AtomicBool = AtomicBool::new(false);

/// Enables process-wide headless test mode (used by external integration tests and offline runners).
pub fn set_headless_test_mode(enabled: bool) {
    HEADLESS_TEST_MODE.store(enabled, Ordering::SeqCst);
}

/// Returns `true` when running inside unit/integration tests or with `GPUI_OFFLINE_TEST` set,
/// so native macOS/Windows GUI, audio, display, and HID FFI calls stay deterministically headless.
#[inline]
pub fn is_headless_test_env() -> bool {
    cfg!(test)
        || HEADLESS_TEST_MODE.load(Ordering::Relaxed)
        || std::env::var_os("GPUI_OFFLINE_TEST").is_some()
}
