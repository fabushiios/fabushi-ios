use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::error::HostError;

thread_local! {
    static LAST_INIT_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

pub fn set_last_error(err: Option<String>) {
    LAST_INIT_ERROR.with(|slot| {
        *slot.borrow_mut() = err.and_then(|s| {
            let sanitized = s.replace('\0', " ");
            CString::new(sanitized).ok()
        });
    });
}

pub fn get_last_error_ptr() -> *const c_char {
    LAST_INIT_ERROR.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

pub fn extract_panic_message(panic_payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic_payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown Rust runtime panic".to_string()
    }
}

pub fn catch_ffi<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> R,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(res) => Ok(res),
        Err(panic) => Err(extract_panic_message(panic)),
    }
}

/// Converts a raw C-string pointer to a Rust `&str`.
///
/// # Safety
/// If non-null, `ptr` must point to a valid NUL-terminated C string.
pub unsafe fn ptr_to_str<'a>(ptr: *const c_char) -> Result<&'a str, HostError> {
    if ptr.is_null() {
        return Err(HostError::invalid_arguments("Null string pointer"));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|e| HostError::invalid_utf8(format!("UTF-8 decode error: {e}")))
}

pub fn string_to_c_ptr(s: String) -> *mut c_char {
    let sanitized = if s.as_bytes().contains(&0) {
        s.replace('\0', "")
    } else {
        s
    };
    CString::new(sanitized)
        .unwrap_or_else(|_| {
            CString::new(
                "{\"id\":\"\",\"ok\":false,\"generation\":0,\"error\":{\"code\":\"serialization_error\",\"message\":\"Output encoding error\",\"retryable\":false}}",
            )
            .expect("static CString is valid")
        })
        .into_raw()
}

/// Frees a C-string allocated by `string_to_c_ptr`.
///
/// # Safety
/// `ptr` must be null or a pointer previously allocated by Rust `CString::into_raw`.
pub unsafe fn free_c_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            unsafe { drop(CString::from_raw(ptr)) };
        }));
    }
}
