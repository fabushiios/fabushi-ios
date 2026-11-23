use crate::error::{EngineResult, RemoteEngineError};
use crate::is_headless_test_env;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GamepadStateFrame {
    pub controller_index: u8,
    pub buttons: u32,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseActionParams {
    pub action: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub dx: Option<i32>,
    pub dy: Option<i32>,
    pub button: Option<String>,
    pub clicks: Option<u8>,
    pub scroll_x: Option<i32>,
    pub scroll_y: Option<i32>,
    pub relative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardActionParams {
    pub action: String,
    pub text: Option<String>,
    pub key: Option<String>,
    pub modifiers: Vec<String>,
}

pub trait NativeInputBackend: Send + Sync {
    fn execute_mouse(&self, params: &MouseActionParams) -> EngineResult<()>;
    fn execute_keyboard(&self, params: &KeyboardActionParams) -> EngineResult<()>;
    fn set_relative_mouse(&self, enabled: bool) -> EngineResult<()>;
    fn inject_gamepad(&self, frame: &GamepadStateFrame) -> EngineResult<()>;
}

#[derive(Debug, Default)]
pub struct HeadlessInputBackend;

impl NativeInputBackend for HeadlessInputBackend {
    fn execute_mouse(&self, params: &MouseActionParams) -> EngineResult<()> {
        if params.action.trim().is_empty() {
            return Err(RemoteEngineError::InvalidArgument("mouse action empty".into()));
        }
        Ok(())
    }

    fn execute_keyboard(&self, params: &KeyboardActionParams) -> EngineResult<()> {
        if params.action.trim().is_empty() {
            return Err(RemoteEngineError::InvalidArgument("keyboard action empty".into()));
        }
        Ok(())
    }

    fn set_relative_mouse(&self, _enabled: bool) -> EngineResult<()> {
        Ok(())
    }

    fn inject_gamepad(&self, _frame: &GamepadStateFrame) -> EngineResult<()> {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos_input {
    use super::*;
    use core_graphics::event::{
        CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField, KeyCode,
        ScrollEventUnit,
    };
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use core_graphics::geometry::CGPoint;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn CGAssociateMouseAndMouseCursorPosition(connected: i32) -> i32;
    }

    pub struct MacOsCgEventBackend;

    impl MacOsCgEventBackend {
        fn ensure_ax() -> EngineResult<()> {
            if !unsafe { AXIsProcessTrusted() } {
                return Err(RemoteEngineError::PermissionDenied(
                    "Accessibility permission required in System Settings > Privacy & Security"
                        .to_string(),
                ));
            }
            Ok(())
        }

        fn source() -> EngineResult<CGEventSource> {
            CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|_| {
                RemoteEngineError::Other("failed to create CGEventSource".to_string())
            })
        }

        fn current_cursor() -> EngineResult<CGPoint> {
            CGEvent::new(Self::source()?)
                .map(|ev| ev.location())
                .map_err(|_| RemoteEngineError::Other("failed to query cursor location".into()))
        }

        fn parse_button(btn: Option<&str>) -> CGMouseButton {
            match btn.unwrap_or("left").to_ascii_lowercase().as_str() {
                "right" => CGMouseButton::Right,
                "middle" | "center" => CGMouseButton::Center,
                _ => CGMouseButton::Left,
            }
        }

        fn mouse_event_types(btn: CGMouseButton) -> (CGEventType, CGEventType, CGEventType) {
            match btn {
                CGMouseButton::Left => (
                    CGEventType::LeftMouseDown,
                    CGEventType::LeftMouseDragged,
                    CGEventType::LeftMouseUp,
                ),
                CGMouseButton::Right => (
                    CGEventType::RightMouseDown,
                    CGEventType::RightMouseDragged,
                    CGEventType::RightMouseUp,
                ),
                CGMouseButton::Center => (
                    CGEventType::OtherMouseDown,
                    CGEventType::OtherMouseDragged,
                    CGEventType::OtherMouseUp,
                ),
            }
        }

        fn post_mouse(
            ev_type: CGEventType,
            pos: CGPoint,
            btn: CGMouseButton,
            clicks: Option<u8>,
            delta: Option<(i32, i32)>,
        ) -> EngineResult<()> {
            let ev = CGEvent::new_mouse_event(Self::source()?, ev_type, pos, btn)
                .map_err(|_| RemoteEngineError::Other("failed to create CGEvent mouse".into()))?;
            if let Some(c) = clicks {
                ev.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, i64::from(c));
            }
            if let Some((dx, dy)) = delta {
                ev.set_integer_value_field(EventField::MOUSE_EVENT_DELTA_X, i64::from(dx));
                ev.set_integer_value_field(EventField::MOUSE_EVENT_DELTA_Y, i64::from(dy));
            }
            ev.post(CGEventTapLocation::HID);
            Ok(())
        }

        fn modifier_flags(mods: &[String]) -> CGEventFlags {
            let mut flags = CGEventFlags::empty();
            for m in mods {
                match m.trim().to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
                    "alt" | "option" => flags |= CGEventFlags::CGEventFlagAlternate,
                    "shift" => flags |= CGEventFlags::CGEventFlagShift,
                    "cmd" | "command" | "meta" | "super" => {
                        flags |= CGEventFlags::CGEventFlagCommand
                    }
                    _ => {}
                }
            }
            flags
        }

        fn key_code(raw: &str) -> Option<u16> {
            Some(match raw.trim().to_ascii_lowercase().as_str() {
                "a" => KeyCode::ANSI_A,
                "b" => KeyCode::ANSI_B,
                "c" => KeyCode::ANSI_C,
                "d" => KeyCode::ANSI_D,
                "e" => KeyCode::ANSI_E,
                "f" => KeyCode::ANSI_F,
                "g" => KeyCode::ANSI_G,
                "h" => KeyCode::ANSI_H,
                "i" => KeyCode::ANSI_I,
                "j" => KeyCode::ANSI_J,
                "k" => KeyCode::ANSI_K,
                "l" => KeyCode::ANSI_L,
                "m" => KeyCode::ANSI_M,
                "n" => KeyCode::ANSI_N,
                "o" => KeyCode::ANSI_O,
                "p" => KeyCode::ANSI_P,
                "q" => KeyCode::ANSI_Q,
                "r" => KeyCode::ANSI_R,
                "s" => KeyCode::ANSI_S,
                "t" => KeyCode::ANSI_T,
                "u" => KeyCode::ANSI_U,
                "v" => KeyCode::ANSI_V,
                "w" => KeyCode::ANSI_W,
                "x" => KeyCode::ANSI_X,
                "y" => KeyCode::ANSI_Y,
                "z" => KeyCode::ANSI_Z,
                "0" => KeyCode::ANSI_0,
                "1" => KeyCode::ANSI_1,
                "2" => KeyCode::ANSI_2,
                "3" => KeyCode::ANSI_3,
                "4" => KeyCode::ANSI_4,
                "5" => KeyCode::ANSI_5,
                "6" => KeyCode::ANSI_6,
                "7" => KeyCode::ANSI_7,
                "8" => KeyCode::ANSI_8,
                "9" => KeyCode::ANSI_9,
                "return" | "enter" => KeyCode::RETURN,
                "tab" => KeyCode::TAB,
                "space" | " " => KeyCode::SPACE,
                "backspace" => KeyCode::DELETE,
                "delete" | "forwarddelete" => KeyCode::FORWARD_DELETE,
                "escape" | "esc" => KeyCode::ESCAPE,
                "left" | "arrowleft" => KeyCode::LEFT_ARROW,
                "right" | "arrowright" => KeyCode::RIGHT_ARROW,
                "up" | "arrowup" => KeyCode::UP_ARROW,
                "down" | "arrowdown" => KeyCode::DOWN_ARROW,
                "home" => KeyCode::HOME,
                "end" => KeyCode::END,
                "pageup" => KeyCode::PAGE_UP,
                "pagedown" => KeyCode::PAGE_DOWN,
                "f1" => KeyCode::F1,
                "f2" => KeyCode::F2,
                "f3" => KeyCode::F3,
                "f4" => KeyCode::F4,
                "f5" => KeyCode::F5,
                "f6" => KeyCode::F6,
                "f7" => KeyCode::F7,
                "f8" => KeyCode::F8,
                "f9" => KeyCode::F9,
                "f10" => KeyCode::F10,
                "f11" => KeyCode::F11,
                "f12" => KeyCode::F12,
                _ => return None,
            })
        }
    }

    impl NativeInputBackend for MacOsCgEventBackend {
        fn execute_mouse(&self, params: &MouseActionParams) -> EngineResult<()> {
            Self::ensure_ax()?;
            let btn = Self::parse_button(params.button.as_deref());
            let (down_ev, drag_ev, up_ev) = Self::mouse_event_types(btn);
            let cur = Self::current_cursor()?;
            let pos = if params.relative {
                CGPoint::new(
                    cur.x + f64::from(params.dx.unwrap_or(0)),
                    cur.y + f64::from(params.dy.unwrap_or(0)),
                )
            } else {
                match (params.x, params.y) {
                    (Some(x), Some(y)) => CGPoint::new(f64::from(x), f64::from(y)),
                    _ => cur,
                }
            };
            let delta = if params.relative {
                Some((params.dx.unwrap_or(0), params.dy.unwrap_or(0)))
            } else {
                None
            };

            match params.action.to_ascii_lowercase().as_str() {
                "move" => Self::post_mouse(CGEventType::MouseMoved, pos, btn, None, delta),
                "down" => Self::post_mouse(down_ev, pos, btn, Some(1), delta),
                "up" => Self::post_mouse(up_ev, pos, btn, Some(1), delta),
                "drag" => Self::post_mouse(drag_ev, pos, btn, Some(1), delta),
                "click" | "double_click" => {
                    let count = if params.action == "double_click" {
                        2
                    } else {
                        params.clicks.unwrap_or(1).clamp(1, 3)
                    };
                    Self::post_mouse(CGEventType::MouseMoved, pos, btn, None, delta)?;
                    for idx in 1..=count {
                        Self::post_mouse(down_ev, pos, btn, Some(idx), None)?;
                        Self::post_mouse(up_ev, pos, btn, Some(idx), None)?;
                    }
                    Ok(())
                }
                "scroll" => {
                    let ev = CGEvent::new_scroll_event(
                        Self::source()?,
                        ScrollEventUnit::LINE,
                        2,
                        params.scroll_y.unwrap_or(0),
                        params.scroll_x.unwrap_or(0),
                        0,
                    )
                    .map_err(|_| RemoteEngineError::Other("failed to create scroll event".into()))?;
                    ev.post(CGEventTapLocation::HID);
                    Ok(())
                }
                other => Err(RemoteEngineError::InvalidArgument(format!(
                    "unsupported mouse action '{other}'"
                ))),
            }
        }

        fn execute_keyboard(&self, params: &KeyboardActionParams) -> EngineResult<()> {
            Self::ensure_ax()?;
            match params.action.to_ascii_lowercase().as_str() {
                "type" => {
                    let text = params.text.as_deref().unwrap_or("");
                    for chunk in text.chars().collect::<Vec<_>>().chunks(20) {
                        let s: String = chunk.iter().collect();
                        let down = CGEvent::new_keyboard_event(Self::source()?, 0, true)
                            .map_err(|_| RemoteEngineError::Other("CGEvent keyboard failed".into()))?;
                        down.set_string(&s);
                        down.post(CGEventTapLocation::HID);
                        let up = CGEvent::new_keyboard_event(Self::source()?, 0, false)
                            .map_err(|_| RemoteEngineError::Other("CGEvent keyboard failed".into()))?;
                        up.post(CGEventTapLocation::HID);
                    }
                    Ok(())
                }
                act @ ("down" | "up" | "press" | "hotkey") => {
                    let key_str = params.key.as_deref().ok_or_else(|| {
                        RemoteEngineError::InvalidArgument("keyboard key is required".into())
                    })?;
                    let code = Self::key_code(key_str).ok_or_else(|| {
                        RemoteEngineError::InvalidArgument(format!("unsupported key '{key_str}'"))
                    })?;
                    let flags = Self::modifier_flags(&params.modifiers);
                    if act != "up" {
                        let down = CGEvent::new_keyboard_event(Self::source()?, code, true)
                            .map_err(|_| RemoteEngineError::Other("CGEvent key down failed".into()))?;
                        down.set_flags(flags);
                        down.post(CGEventTapLocation::HID);
                    }
                    if act != "down" {
                        let up = CGEvent::new_keyboard_event(Self::source()?, code, false)
                            .map_err(|_| RemoteEngineError::Other("CGEvent key up failed".into()))?;
                        up.set_flags(flags);
                        up.post(CGEventTapLocation::HID);
                    }
                    Ok(())
                }
                other => Err(RemoteEngineError::InvalidArgument(format!(
                    "unsupported keyboard action '{other}'"
                ))),
            }
        }

        fn set_relative_mouse(&self, enabled: bool) -> EngineResult<()> {
            Self::ensure_ax()?;
            let connected = if enabled { 0 } else { 1 };
            let rc = unsafe { CGAssociateMouseAndMouseCursorPosition(connected) };
            if rc != 0 {
                return Err(RemoteEngineError::Other(format!(
                    "CGAssociateMouseAndMouseCursorPosition failed: {rc}"
                )));
            }
            Ok(())
        }

        fn inject_gamepad(&self, _frame: &GamepadStateFrame) -> EngineResult<()> {
            Ok(())
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod enigo_input {
    use super::*;
    use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
    use parking_lot::Mutex;

    pub struct EnigoInputBackend {
        enigo: Mutex<Enigo>,
    }

    impl EnigoInputBackend {
        pub fn new() -> EngineResult<Self> {
            let instance = Enigo::new(&Settings::default())
                .map_err(|e| RemoteEngineError::Unavailable(format!("enigo init failed: {e}")))?;
            Ok(Self {
                enigo: Mutex::new(instance),
            })
        }

        fn parse_button(btn: Option<&str>) -> Button {
            match btn.unwrap_or("left").to_ascii_lowercase().as_str() {
                "right" => Button::Right,
                "middle" | "center" => Button::Middle,
                _ => Button::Left,
            }
        }

        fn parse_key(raw: &str) -> Option<Key> {
            let low = raw.trim().to_ascii_lowercase();
            Some(match low.as_str() {
                "return" | "enter" => Key::Return,
                "tab" => Key::Tab,
                "space" | " " => Key::Space,
                "backspace" => Key::Backspace,
                "delete" => Key::Delete,
                "escape" | "esc" => Key::Escape,
                "left" | "arrowleft" => Key::LeftArrow,
                "right" | "arrowright" => Key::RightArrow,
                "up" | "arrowup" => Key::UpArrow,
                "down" | "arrowdown" => Key::DownArrow,
                "home" => Key::Home,
                "end" => Key::End,
                "pageup" => Key::PageUp,
                "pagedown" => Key::PageDown,
                "ctrl" | "control" => Key::Control,
                "alt" | "option" => Key::Alt,
                "shift" => Key::Shift,
                "cmd" | "command" | "meta" | "super" => Key::Meta,
                s if s.chars().count() == 1 => Key::Unicode(s.chars().next().unwrap()),
                _ => return None,
            })
        }
    }

    impl NativeInputBackend for EnigoInputBackend {
        fn execute_mouse(&self, params: &MouseActionParams) -> EngineResult<()> {
            let mut enigo = self.enigo.lock();
            let btn = Self::parse_button(params.button.as_deref());
            match params.action.to_ascii_lowercase().as_str() {
                "move" | "drag" => {
                    if params.relative {
                        enigo
                            .move_mouse(params.dx.unwrap_or(0), params.dy.unwrap_or(0), Coordinate::Rel)
                            .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                    } else if let (Some(x), Some(y)) = (params.x, params.y) {
                        enigo
                            .move_mouse(x, y, Coordinate::Abs)
                            .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                    }
                }
                "down" => {
                    enigo
                        .button(btn, Direction::Press)
                        .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                }
                "up" => {
                    enigo
                        .button(btn, Direction::Release)
                        .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                }
                "click" | "double_click" => {
                    if let (Some(x), Some(y)) = (params.x, params.y) {
                        enigo
                            .move_mouse(x, y, Coordinate::Abs)
                            .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                    }
                    let count = if params.action == "double_click" {
                        2
                    } else {
                        params.clicks.unwrap_or(1).clamp(1, 3)
                    };
                    for _ in 0..count {
                        enigo
                            .button(btn, Direction::Click)
                            .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                    }
                }
                "scroll" => {
                    if let Some(sy) = params.scroll_y.filter(|v| *v != 0) {
                        enigo
                            .scroll(sy, enigo::Axis::Vertical)
                            .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                    }
                    if let Some(sx) = params.scroll_x.filter(|v| *v != 0) {
                        enigo
                            .scroll(sx, enigo::Axis::Horizontal)
                            .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                    }
                }
                other => {
                    return Err(RemoteEngineError::InvalidArgument(format!(
                        "unsupported mouse action '{other}'"
                    )));
                }
            }
            Ok(())
        }

        fn execute_keyboard(&self, params: &KeyboardActionParams) -> EngineResult<()> {
            let mut enigo = self.enigo.lock();
            match params.action.to_ascii_lowercase().as_str() {
                "type" => {
                    enigo
                        .text(params.text.as_deref().unwrap_or(""))
                        .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                }
                act @ ("down" | "up" | "press" | "hotkey") => {
                    let key_str = params.key.as_deref().ok_or_else(|| {
                        RemoteEngineError::InvalidArgument("keyboard key required".into())
                    })?;
                    let key = Self::parse_key(key_str).ok_or_else(|| {
                        RemoteEngineError::InvalidArgument(format!("unsupported key '{key_str}'"))
                    })?;
                    let mod_keys: Vec<Key> = params
                        .modifiers
                        .iter()
                        .filter_map(|m| Self::parse_key(m))
                        .collect();
                    for mk in &mod_keys {
                        let _ = enigo.key(*mk, Direction::Press);
                    }
                    let dir = match act {
                        "down" => Direction::Press,
                        "up" => Direction::Release,
                        _ => Direction::Click,
                    };
                    enigo
                        .key(key, dir)
                        .map_err(|e| RemoteEngineError::Other(e.to_string()))?;
                    for mk in mod_keys.iter().rev() {
                        let _ = enigo.key(*mk, Direction::Release);
                    }
                }
                other => {
                    return Err(RemoteEngineError::InvalidArgument(format!(
                        "unsupported keyboard action '{other}'"
                    )));
                }
            }
            Ok(())
        }

        fn set_relative_mouse(&self, _enabled: bool) -> EngineResult<()> {
            Ok(())
        }

        fn inject_gamepad(&self, _frame: &GamepadStateFrame) -> EngineResult<()> {
            Ok(())
        }
    }
}

pub struct NativeInputEngine {
    backend: Arc<dyn NativeInputBackend>,
}

impl Default for NativeInputEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeInputEngine {
    pub fn new() -> Self {
        if is_headless_test_env() {
            return Self::with_backend(Arc::new(HeadlessInputBackend));
        }
        #[cfg(target_os = "macos")]
        {
            Self::with_backend(Arc::new(macos_input::MacOsCgEventBackend))
        }
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        {
            match enigo_input::EnigoInputBackend::new() {
                Ok(b) => Self::with_backend(Arc::new(b)),
                Err(_) => Self::with_backend(Arc::new(HeadlessInputBackend)),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::with_backend(Arc::new(HeadlessInputBackend))
        }
    }

    pub fn with_backend(backend: Arc<dyn NativeInputBackend>) -> Self {
        Self { backend }
    }

    pub fn headless() -> Self {
        Self::with_backend(Arc::new(HeadlessInputBackend))
    }

    pub fn remap_modifiers(
        modifiers: &[String],
        client_os: &str,
        host_os: &str,
        enabled: bool,
    ) -> Vec<String> {
        if !enabled || client_os.eq_ignore_ascii_case(host_os) {
            return modifiers.to_vec();
        }
        modifiers
            .iter()
            .map(|m| {
                let low = m.trim().to_ascii_lowercase();
                if host_os.eq_ignore_ascii_case("macos") && (low == "ctrl" || low == "control") {
                    "cmd".to_string()
                } else if !host_os.eq_ignore_ascii_case("macos")
                    && (low == "cmd" || low == "command" || low == "meta")
                {
                    "ctrl".to_string()
                } else {
                    m.clone()
                }
            })
            .collect()
    }

    pub fn execute_mouse_action(&self, params: &MouseActionParams) -> EngineResult<()> {
        self.backend.execute_mouse(params)
    }

    pub fn execute_keyboard_action(&self, params: &KeyboardActionParams) -> EngineResult<()> {
        self.backend.execute_keyboard(params)
    }

    pub fn set_relative_mouse(&self, enabled: bool) -> EngineResult<()> {
        self.backend.set_relative_mouse(enabled)
    }

    pub fn inject_gamepad(&self, frame: &GamepadStateFrame) -> EngineResult<()> {
        self.backend.inject_gamepad(frame)
    }
}
