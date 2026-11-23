use crate::error::EngineResult;
use crate::is_headless_test_env;
use gpui_telegram_proto::remote_control_contracts::DisplayInfoDto;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[derive(Debug, Default)]
pub struct DisplayPrivacyManager {
    active_display_index: AtomicU32,
    privacy_enabled: AtomicBool,
    privacy_message: Mutex<Option<String>>,
    virtual_displays: Mutex<Vec<DisplayInfoDto>>,
}

impl DisplayPrivacyManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_display_index(&self) -> u32 {
        self.active_display_index.load(Ordering::SeqCst)
    }

    pub fn switch_display(&self, display_index: u32) -> EngineResult<u32> {
        self.active_display_index.store(display_index, Ordering::SeqCst);
        Ok(display_index)
    }

    pub fn set_privacy_screen(&self, enabled: bool, message: Option<String>) -> EngineResult<bool> {
        self.privacy_enabled.store(enabled, Ordering::SeqCst);
        *self.privacy_message.lock() = message;
        if !is_headless_test_env() {
            #[cfg(target_os = "macos")]
            {
                // On macOS, when privacy screen is requested in a live session,
                // display shielding state is synchronized with the GPUI overlay host.
                tracing::info!(enabled, "macOS privacy screen state updated");
            }
        }
        Ok(enabled)
    }

    pub fn is_privacy_screen_enabled(&self) -> bool {
        self.privacy_enabled.load(Ordering::SeqCst)
    }

    pub fn privacy_message(&self) -> Option<String> {
        self.privacy_message.lock().clone()
    }

    pub fn create_virtual_display(
        &self,
        width: u32,
        height: u32,
        refresh_hz: u32,
    ) -> EngineResult<DisplayInfoDto> {
        let mut list = self.virtual_displays.lock();
        let id = 1000 + list.len() as u32;
        let dto = DisplayInfoDto {
            display_id: id,
            name: format!("Fabushi Virtual Display {id}"),
            width: width.max(640),
            height: height.max(480),
            scale_factor_x100: 100,
            is_primary: false,
            is_virtual: true,
            refresh_hz: refresh_hz.max(30),
        };
        list.push(dto.clone());
        Ok(dto)
    }

    pub fn remove_virtual_display(&self, display_id: u32) -> bool {
        let mut list = self.virtual_displays.lock();
        let before = list.len();
        list.retain(|d| d.display_id != display_id);
        list.len() < before
    }

    pub fn virtual_displays(&self) -> Vec<DisplayInfoDto> {
        self.virtual_displays.lock().clone()
    }
}
