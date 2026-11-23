use crate::error::{EngineResult, RemoteEngineError};
use crate::is_headless_test_env;
use gpui_telegram_proto::remote_control_contracts::AccessibilityNodeDto;

#[derive(Debug, Default)]
pub struct AccessibilityEngine;

#[cfg(target_os = "macos")]
mod macos_ax {
    use super::*;
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef};
    use core_foundation_sys::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef};
    use core_foundation_sys::string::{CFStringGetTypeID, CFStringRef};
    use std::ffi::c_void;
    use std::ptr;

    const AX_OK: i32 = 0;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXUIElementCreateSystemWide() -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementPerformAction(element: *const c_void, action: CFStringRef) -> i32;
        fn AXUIElementSetAttributeValue(
            element: *const c_void,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> i32;
        fn AXUIElementGetPid(element: *const c_void, pid: *mut i32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: *const c_void;
    }

    fn ax_copy_value(element: *const c_void, attribute: &str) -> Option<CFTypeRef> {
        let attr = CFString::new(attribute);
        let mut val: CFTypeRef = ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut val)
        };
        if status == AX_OK && !val.is_null() {
            Some(val)
        } else {
            None
        }
    }

    fn ax_string(element: *const c_void, attribute: &str) -> Option<String> {
        let val = ax_copy_value(element, attribute)?;
        if unsafe { CFGetTypeID(val) } != unsafe { CFStringGetTypeID() } {
            unsafe { CFRelease(val) };
            return None;
        }
        let s = unsafe { CFString::wrap_under_create_rule(val as CFStringRef) };
        Some(s.to_string())
    }

    fn focused_window() -> EngineResult<(*const c_void, i32)> {
        if !unsafe { AXIsProcessTrusted() } {
            return Err(RemoteEngineError::PermissionDenied(
                "Accessibility permission required".into(),
            ));
        }
        let sys = unsafe { AXUIElementCreateSystemWide() };
        if sys.is_null() {
            return Err(RemoteEngineError::Other("AXUIElementCreateSystemWide failed".into()));
        }
        let app = ax_copy_value(sys, "AXFocusedApplication");
        unsafe { CFRelease(sys as CFTypeRef) };
        let Some(app) = app else {
            return Err(RemoteEngineError::NotFound("no focused application".into()));
        };
        let mut pid: i32 = 0;
        let _ = unsafe { AXUIElementGetPid(app as *const c_void, &mut pid) };
        let win = ax_copy_value(app as *const c_void, "AXFocusedWindow");
        unsafe { CFRelease(app) };
        let Some(win) = win else {
            return Err(RemoteEngineError::NotFound("no focused window".into()));
        };
        Ok((win as *const c_void, pid))
    }

    fn walk_node(
        el: *const c_void,
        path: &str,
        pid: i32,
        depth: u32,
        max_depth: u32,
        visited: &mut u32,
    ) -> AccessibilityNodeDto {
        *visited += 1;
        let role = ax_string(el, "AXRole").unwrap_or_else(|| "unknown".into());
        let name = ax_string(el, "AXTitle").or_else(|| ax_string(el, "AXDescription"));
        let value = ax_string(el, "AXValue");
        let element_id = format!("macos-ax:{pid}:{path}");
        let mut children = Vec::new();
        if depth < max_depth && *visited < 256 {
            if let Some(arr_val) = ax_copy_value(el, "AXChildren") {
                if unsafe { CFGetTypeID(arr_val) } == unsafe { CFArrayGetTypeID() } {
                    let arr = arr_val as CFArrayRef;
                    let count = unsafe { CFArrayGetCount(arr) }.max(0);
                    for i in 0..count {
                        if *visited >= 256 {
                            break;
                        }
                        let child = unsafe { CFArrayGetValueAtIndex(arr, i) };
                        if child.is_null() {
                            continue;
                        }
                        let child_path = if path == "root" {
                            i.to_string()
                        } else {
                            format!("{path}.{i}")
                        };
                        children.push(walk_node(
                            child,
                            &child_path,
                            pid,
                            depth + 1,
                            max_depth,
                            visited,
                        ));
                    }
                }
                unsafe { CFRelease(arr_val) };
            }
        }
        AccessibilityNodeDto {
            element_id,
            role,
            name,
            value,
            bounds: None,
            actions: vec!["press".into(), "focus".into(), "set_value".into()],
            children,
        }
    }

    pub fn capture_tree(max_depth: Option<u32>) -> EngineResult<AccessibilityNodeDto> {
        let (win, pid) = focused_window()?;
        let mut visited = 0;
        let dto = walk_node(win, "root", pid, 0, max_depth.unwrap_or(6).clamp(1, 12), &mut visited);
        unsafe { CFRelease(win as CFTypeRef) };
        Ok(dto)
    }

    pub fn perform_semantic_action(
        element_id: &str,
        action: &str,
        value: Option<&str>,
    ) -> EngineResult<()> {
        let mut parts = element_id.splitn(3, ':');
        if parts.next() != Some("macos-ax") {
            return Err(RemoteEngineError::InvalidArgument("invalid macOS AX element_id".into()));
        }
        let _pid_str = parts.next();
        let path = parts
            .next()
            .ok_or_else(|| RemoteEngineError::InvalidArgument("missing AX path".into()))?;
        let (win, _) = focused_window()?;
        let mut cur = win;
        if path != "root" {
            for seg in path.split('.') {
                let idx: isize = seg
                    .parse()
                    .map_err(|_| RemoteEngineError::InvalidArgument("invalid AX index".into()))?;
                let Some(arr_val) = ax_copy_value(cur, "AXChildren") else {
                    unsafe { CFRelease(cur as CFTypeRef) };
                    return Err(RemoteEngineError::NotFound("AX element disappeared".into()));
                };
                let arr = arr_val as CFArrayRef;
                let count = unsafe { CFArrayGetCount(arr) };
                if idx < 0 || idx >= count {
                    unsafe {
                        CFRelease(arr_val);
                        CFRelease(cur as CFTypeRef);
                    }
                    return Err(RemoteEngineError::NotFound("AX child index out of bounds".into()));
                }
                let child = unsafe { CFArrayGetValueAtIndex(arr, idx) };
                unsafe {
                    CFRetain(child as CFTypeRef);
                    CFRelease(arr_val);
                    CFRelease(cur as CFTypeRef);
                }
                cur = child;
            }
        }
        let rc = match action.to_ascii_lowercase().as_str() {
            "press" => {
                let a = CFString::new("AXPress");
                unsafe { AXUIElementPerformAction(cur, a.as_concrete_TypeRef()) }
            }
            "focus" => {
                let attr = CFString::new("AXFocused");
                unsafe {
                    AXUIElementSetAttributeValue(
                        cur,
                        attr.as_concrete_TypeRef(),
                        kCFBooleanTrue as CFTypeRef,
                    )
                }
            }
            "set_value" => {
                let v = CFString::new(value.unwrap_or(""));
                let attr = CFString::new("AXValue");
                unsafe {
                    AXUIElementSetAttributeValue(cur, attr.as_concrete_TypeRef(), v.as_CFTypeRef())
                }
            }
            other => {
                unsafe { CFRelease(cur as CFTypeRef) };
                return Err(RemoteEngineError::InvalidArgument(format!(
                    "unsupported semantic action '{other}'"
                )));
            }
        };
        unsafe { CFRelease(cur as CFTypeRef) };
        if rc != AX_OK {
            return Err(RemoteEngineError::Other(format!(
                "AXUIElement action failed with status {rc}"
            )));
        }
        Ok(())
    }
}

impl AccessibilityEngine {
    pub fn capture_tree(&self, max_depth: Option<u32>) -> EngineResult<AccessibilityNodeDto> {
        if is_headless_test_env() {
            return Ok(AccessibilityNodeDto {
                element_id: "headless-root".to_string(),
                role: "AXApplication".to_string(),
                name: Some("Fabushi".to_string()),
                value: None,
                bounds: Some([0, 0, 1920, 1080]),
                actions: vec!["press".to_string(), "focus".to_string()],
                children: Vec::new(),
            });
        }
        #[cfg(target_os = "macos")]
        {
            macos_ax::capture_tree(max_depth)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = max_depth;
            Ok(AccessibilityNodeDto {
                element_id: "root".to_string(),
                role: "Application".to_string(),
                name: Some("Desktop".to_string()),
                value: None,
                bounds: None,
                actions: Vec::new(),
                children: Vec::new(),
            })
        }
    }

    pub fn perform_semantic_action(
        &self,
        element_id: &str,
        action: &str,
        value: Option<&str>,
    ) -> EngineResult<()> {
        if is_headless_test_env() {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            macos_ax::perform_semantic_action(element_id, action, value)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (element_id, action, value);
            Ok(())
        }
    }
}
