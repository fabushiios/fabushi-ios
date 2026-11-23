//! Authoritative macOS privacy permission diagnostics, signing identity introspection,
//! and native ScreenCaptureKit capture bridge shared across `fabushi_remote_engine` and
//! `gpui_telegram_desktop`.

use crate::capture::{CaptureFormat, CapturedFrame};
use crate::error::{EngineResult, RemoteEngineError};
use crate::is_headless_test_env;
use std::sync::atomic::{AtomicBool, Ordering};

/// Process-wide flag flipped to `true` as soon as a real native ScreenCaptureKit
/// frame or shareable content query succeeds in this process.
static SCK_LIVE_VERIFIED: AtomicBool = AtomicBool::new(false);

pub fn mark_sck_live_verified() {
    SCK_LIVE_VERIFIED.store(true, Ordering::Release);
}

pub fn clear_sck_live_verified() {
    SCK_LIVE_VERIFIED.store(false, Ordering::Release);
}

pub fn is_sck_live_verified() -> bool {
    SCK_LIVE_VERIFIED.load(Ordering::Acquire)
}

/// Code-signing and bundle identity metadata for the currently running process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MacSigningIdentity {
    pub is_adhoc: bool,
    pub is_app_bundle: bool,
    pub cdhash_hex: String,
    pub bundle_id: String,
    pub bundle_path: String,
    pub executable_path: String,
    pub team_id: String,
}

impl MacSigningIdentity {
    /// Returns the best filesystem target to drag onto macOS System Settings
    /// (the enclosing `.app` bundle when running inside an app bundle, otherwise the executable).
    pub fn drag_target_path(&self) -> &str {
        if self.is_app_bundle && !self.bundle_path.is_empty() {
            &self.bundle_path
        } else if !self.executable_path.is_empty() {
            &self.executable_path
        } else {
            "/Applications/Fabushi.app"
        }
    }

    /// Returns a human-friendly display name for the running app.
    pub fn display_app_name(&self) -> String {
        if self.is_app_bundle {
            if let Some(stem) = std::path::Path::new(&self.bundle_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
            {
                return stem.to_string();
            }
        }
        "Fabushi".to_string()
    }
}

/// Outcome of a live `SCShareableContent` probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SckProbeOutcome {
    GrantedActive,
    PermissionDenied(String),
    Unavailable,
    Error(String),
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::{CStr, c_char};

    const FABUSHI_SCK_OK: i32 = 0;
    const FABUSHI_SCK_PERMISSION_DENIED: i32 = 1;
    const FABUSHI_SCK_UNAVAILABLE: i32 = 2;
    const FABUSHI_SCK_NOT_FOUND: i32 = 3;

    const FABUSHI_FMT_PNG: i32 = 0;
    const FABUSHI_FMT_JPEG: i32 = 1;
    const FABUSHI_FMT_BGRA8: i32 = 2;

    #[repr(C)]
    struct FabushiSckCaptureResultC {
        status: i32,
        width: u32,
        height: u32,
        data: *mut u8,
        data_len: usize,
        error_msg: [c_char; 512],
    }

    #[repr(C)]
    struct FabushiMacSigningInfoC {
        is_adhoc: bool,
        is_app_bundle: bool,
        cdhash_hex: [c_char; 128],
        bundle_id: [c_char; 256],
        bundle_path: [c_char; 1024],
        executable_path: [c_char; 1024],
        team_id: [c_char; 128],
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn AXIsProcessTrusted() -> bool;
    }

    unsafe extern "C" {
        fn fabushi_sck_free_buffer(ptr: *mut u8);
        fn fabushi_sck_probe_access(err_buf: *mut c_char, err_buf_len: usize) -> i32;
        fn fabushi_sck_capture_display(
            target_display_id: u32,
            display_index: u32,
            format: i32,
            jpeg_quality: u8,
        ) -> FabushiSckCaptureResultC;
        fn fabushi_cg_can_read_other_window_names() -> bool;
        fn fabushi_copy_basic_bundle_info() -> FabushiMacSigningInfoC;
        fn fabushi_copy_signing_info() -> FabushiMacSigningInfoC;
    }

    fn cstr_array_to_string(buf: &[c_char]) -> String {
        let nul_pos = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if nul_pos == 0 {
            return String::new();
        }
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    fn signing_identity_from_raw(raw: FabushiMacSigningInfoC) -> MacSigningIdentity {
        MacSigningIdentity {
            is_adhoc: raw.is_adhoc,
            is_app_bundle: raw.is_app_bundle,
            cdhash_hex: cstr_array_to_string(&raw.cdhash_hex),
            bundle_id: cstr_array_to_string(&raw.bundle_id),
            bundle_path: cstr_array_to_string(&raw.bundle_path),
            executable_path: cstr_array_to_string(&raw.executable_path),
            team_id: cstr_array_to_string(&raw.team_id),
        }
    }

    pub fn preflight_screen_capture_access() -> bool {
        if is_headless_test_env() {
            return true;
        }
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn is_accessibility_trusted() -> bool {
        if is_headless_test_env() {
            return true;
        }
        unsafe { AXIsProcessTrusted() }
    }

    pub fn can_read_other_window_names() -> bool {
        if is_headless_test_env() {
            return true;
        }
        unsafe { fabushi_cg_can_read_other_window_names() }
    }

    /// Fast, main-thread-safe bundle/executable lookup without invoking Security.framework.
    pub fn current_bundle_identity() -> MacSigningIdentity {
        signing_identity_from_raw(unsafe { fabushi_copy_basic_bundle_info() })
    }

    pub fn current_signing_identity() -> MacSigningIdentity {
        signing_identity_from_raw(unsafe { fabushi_copy_signing_info() })
    }

    pub fn probe_sck_screen_access() -> SckProbeOutcome {
        if is_headless_test_env() {
            return SckProbeOutcome::GrantedActive;
        }
        let mut err_buf = [0 as c_char; 512];
        let rc = unsafe { fabushi_sck_probe_access(err_buf.as_mut_ptr(), err_buf.len()) };
        let err_msg = cstr_array_to_string(&err_buf);
        match rc {
            FABUSHI_SCK_OK => {
                mark_sck_live_verified();
                SckProbeOutcome::GrantedActive
            }
            FABUSHI_SCK_PERMISSION_DENIED => {
                clear_sck_live_verified();
                SckProbeOutcome::PermissionDenied(if err_msg.is_empty() {
                    "ScreenCaptureKit TCC permission denied (-3801)".to_string()
                } else {
                    err_msg
                })
            }
            FABUSHI_SCK_UNAVAILABLE => SckProbeOutcome::Unavailable,
            _ => SckProbeOutcome::Error(if err_msg.is_empty() {
                format!("SCShareableContent probe failed with code {rc}")
            } else {
                err_msg
            }),
        }
    }

    pub fn sck_capture_display(
        target_display_id: u32,
        display_index: u32,
        format: CaptureFormat,
        seq: u64,
    ) -> Result<CapturedFrame, (i32, RemoteEngineError)> {
        let (fmt_code, quality, mime) = match format {
            CaptureFormat::Png => (FABUSHI_FMT_PNG, 100u8, "image/png"),
            CaptureFormat::Jpeg { quality } => (FABUSHI_FMT_JPEG, quality, "image/jpeg"),
            CaptureFormat::Bgra8 => (FABUSHI_FMT_BGRA8, 100u8, "application/octet-stream"),
        };

        let raw = unsafe {
            fabushi_sck_capture_display(target_display_id, display_index, fmt_code, quality)
        };
        let err_msg = cstr_array_to_string(&raw.error_msg);

        if raw.status == FABUSHI_SCK_OK && !raw.data.is_null() && raw.data_len > 0 {
            let bytes = unsafe { std::slice::from_raw_parts(raw.data, raw.data_len) }.to_vec();
            unsafe { fabushi_sck_free_buffer(raw.data) };
            mark_sck_live_verified();
            return Ok(CapturedFrame {
                width: raw.width,
                height: raw.height,
                mime_type: mime.to_string(),
                data: bytes,
                captured_at_ms: chrono::Utc::now().timestamp_millis(),
                display_id: display_index,
                seq,
            });
        }

        if !raw.data.is_null() {
            unsafe { fabushi_sck_free_buffer(raw.data) };
        }

        let err = match raw.status {
            FABUSHI_SCK_PERMISSION_DENIED => {
                clear_sck_live_verified();
                RemoteEngineError::PermissionDenied(if err_msg.is_empty() {
                    "Screen Recording permission required in System Settings > Privacy & Security"
                        .to_string()
                } else {
                    err_msg
                })
            }
            FABUSHI_SCK_NOT_FOUND => RemoteEngineError::NotFound(if err_msg.is_empty() {
                format!("display index {display_index} not found")
            } else {
                err_msg
            }),
            FABUSHI_SCK_UNAVAILABLE => RemoteEngineError::Unavailable(if err_msg.is_empty() {
                "ScreenCaptureKit SCScreenshotManager unavailable".to_string()
            } else {
                err_msg
            }),
            _ => RemoteEngineError::Other(if err_msg.is_empty() {
                format!("ScreenCaptureKit capture failed (status {})", raw.status)
            } else {
                err_msg
            }),
        };

        Err((raw.status, err))
    }

    pub const STATUS_UNAVAILABLE: i32 = FABUSHI_SCK_UNAVAILABLE;
    pub const STATUS_PERMISSION_DENIED: i32 = FABUSHI_SCK_PERMISSION_DENIED;
    pub const STATUS_NOT_FOUND: i32 = FABUSHI_SCK_NOT_FOUND;
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    pub fn preflight_screen_capture_access() -> bool {
        !cfg!(target_os = "ios")
    }

    pub fn is_accessibility_trusted() -> bool {
        !cfg!(target_os = "ios")
    }

    pub fn can_read_other_window_names() -> bool {
        !cfg!(target_os = "ios")
    }

    pub fn current_bundle_identity() -> MacSigningIdentity {
        MacSigningIdentity::default()
    }

    pub fn current_signing_identity() -> MacSigningIdentity {
        MacSigningIdentity::default()
    }

    pub fn probe_sck_screen_access() -> SckProbeOutcome {
        if cfg!(target_os = "ios") {
            SckProbeOutcome::Unavailable
        } else {
            SckProbeOutcome::GrantedActive
        }
    }

    pub fn sck_capture_display(
        _target_display_id: u32,
        _display_index: u32,
        _format: CaptureFormat,
        _seq: u64,
    ) -> Result<CapturedFrame, (i32, RemoteEngineError)> {
        Err((
            2,
            RemoteEngineError::Unavailable("ScreenCaptureKit is macOS-only".to_string()),
        ))
    }

    pub const STATUS_UNAVAILABLE: i32 = 2;
    pub const STATUS_PERMISSION_DENIED: i32 = 1;
    pub const STATUS_NOT_FOUND: i32 = 3;
}

pub use imp::{
    STATUS_NOT_FOUND, STATUS_PERMISSION_DENIED, STATUS_UNAVAILABLE, can_read_other_window_names,
    current_bundle_identity, current_signing_identity, is_accessibility_trusted,
    preflight_screen_capture_access, probe_sck_screen_access, sck_capture_display,
};

/// Helper returning `Ok(())` when Accessibility permission is active on the current host.
pub fn ensure_accessibility_permission() -> EngineResult<()> {
    if is_accessibility_trusted() {
        Ok(())
    } else {
        Err(RemoteEngineError::PermissionDenied(
            "Accessibility permission required in System Settings > Privacy & Security".to_string(),
        ))
    }
}
