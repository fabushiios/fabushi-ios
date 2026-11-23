use crate::error::{EngineResult, RemoteEngineError};
use crate::is_headless_test_env;
use gpui_telegram_proto::remote_control_contracts::PowerActionKind;
use std::net::UdpSocket;

/// Builds a standard 102-byte Wake-on-LAN magic packet (`6 x 0xFF` + `16 x 6-byte MAC`).
pub fn build_wol_magic_packet(mac_address: &str) -> EngineResult<[u8; 102]> {
    let cleaned: String = mac_address
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    if cleaned.len() != 12 {
        return Err(RemoteEngineError::InvalidArgument(format!(
            "invalid MAC address '{mac_address}'"
        )));
    }
    let mut mac = [0u8; 6];
    for (idx, chunk) in cleaned.as_bytes().chunks_exact(2).enumerate() {
        let s = std::str::from_utf8(chunk)
            .map_err(|_| RemoteEngineError::InvalidArgument("invalid MAC bytes".into()))?;
        mac[idx] = u8::from_str_radix(s, 16)
            .map_err(|_| RemoteEngineError::InvalidArgument("invalid MAC hex".into()))?;
    }
    let mut packet = [0xffu8; 102];
    for i in 0..16 {
        let start = 6 + i * 6;
        packet[start..start + 6].copy_from_slice(&mac);
    }
    Ok(packet)
}

pub fn send_wake_on_lan(mac_address: &str, broadcast_ip: Option<&str>) -> EngineResult<()> {
    let packet = build_wol_magic_packet(mac_address)?;
    let target = format!("{}:9", broadcast_ip.unwrap_or("255.255.255.255"));
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_broadcast(true)?;
    socket.send_to(&packet, &target)?;
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos_power {
    use super::*;

    pub fn lock_screen() -> EngineResult<()> {
        unsafe {
            let path = c"/System/Library/PrivateFrameworks/login.framework/login";
            let handle = libc::dlopen(path.as_ptr(), libc::RTLD_LAZY);
            if !handle.is_null() {
                let sym = libc::dlsym(handle, c"SACLockScreenImmediate".as_ptr());
                if !sym.is_null() {
                    let func: extern "C" fn() = std::mem::transmute(sym);
                    func();
                    let _ = libc::dlclose(handle);
                    return Ok(());
                }
                let _ = libc::dlclose(handle);
            }
        }
        let status = std::process::Command::new("pmset")
            .arg("displaysleepnow")
            .status()?;
        if !status.success() {
            return Err(RemoteEngineError::Other(format!(
                "lock screen failed with {status}"
            )));
        }
        Ok(())
    }

    pub fn sleep_system() -> EngineResult<()> {
        let status = std::process::Command::new("pmset")
            .arg("sleepnow")
            .status()?;
        if !status.success() {
            return Err(RemoteEngineError::Other(format!(
                "pmset sleepnow exited with {status}"
            )));
        }
        Ok(())
    }

    pub fn reboot_system() -> EngineResult<()> {
        let status = std::process::Command::new("shutdown")
            .args(["-r", "now"])
            .status()?;
        if !status.success() {
            return Err(RemoteEngineError::PermissionDenied(format!(
                "shutdown -r now exited with {status}"
            )));
        }
        Ok(())
    }

    pub fn shutdown_system(delay_minutes: Option<u32>) -> EngineResult<()> {
        let arg = match delay_minutes {
            Some(m) if m > 0 => format!("+{m}"),
            _ => "now".to_string(),
        };
        let status = std::process::Command::new("shutdown")
            .args(["-h", &arg])
            .status()?;
        if !status.success() {
            return Err(RemoteEngineError::PermissionDenied(format!(
                "shutdown -h {arg} exited with {status}"
            )));
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod win_power {
    use super::*;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn LockWorkStation() -> i32;
        fn ExitWindowsEx(u_flags: u32, dw_reason: u32) -> i32;
    }

    pub fn lock_screen() -> EngineResult<()> {
        let ok = unsafe { LockWorkStation() };
        if ok == 0 {
            return Err(RemoteEngineError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    pub fn sleep_system() -> EngineResult<()> {
        let status = std::process::Command::new("rundll32.exe")
            .args(["powrprof.dll,SetSuspendState", "0,1,0"])
            .status()?;
        if !status.success() {
            return Err(RemoteEngineError::Other("SetSuspendState failed".into()));
        }
        Ok(())
    }

    pub fn reboot_system() -> EngineResult<()> {
        let ok = unsafe { ExitWindowsEx(0x0000_0002, 0) };
        if ok == 0 {
            return Err(RemoteEngineError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    pub fn shutdown_system(delay_minutes: Option<u32>) -> EngineResult<()> {
        let secs = delay_minutes.unwrap_or(0).saturating_mul(60);
        let status = std::process::Command::new("shutdown.exe")
            .args(["/s", "/t", &secs.to_string()])
            .status()?;
        if !status.success() {
            return Err(RemoteEngineError::Other("shutdown.exe failed".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct PowerManager;

impl PowerManager {
    pub fn execute_power_action(action: &PowerActionKind) -> EngineResult<String> {
        match action {
            PowerActionKind::WakeOnLan {
                mac_address,
                broadcast_ip,
            } => {
                send_wake_on_lan(mac_address, broadcast_ip.as_deref())?;
                Ok(format!("Wake-on-LAN packet sent to {mac_address}"))
            }
            other if is_headless_test_env() => Ok(format!("headless power dry-run: {other:?}")),
            PowerActionKind::Lock => {
                #[cfg(target_os = "macos")]
                {
                    macos_power::lock_screen()?;
                }
                #[cfg(target_os = "windows")]
                {
                    win_power::lock_screen()?;
                }
                #[cfg(target_os = "linux")]
                {
                    let _ = std::process::Command::new("loginctl")
                        .arg("lock-session")
                        .status()?;
                }
                Ok("lock dispatched".to_string())
            }
            PowerActionKind::Sleep => {
                #[cfg(target_os = "macos")]
                macos_power::sleep_system()?;
                #[cfg(target_os = "windows")]
                win_power::sleep_system()?;
                #[cfg(target_os = "linux")]
                {
                    let _ = std::process::Command::new("systemctl")
                        .arg("suspend")
                        .status()?;
                }
                Ok("sleep dispatched".to_string())
            }
            PowerActionKind::Reboot => {
                #[cfg(target_os = "macos")]
                macos_power::reboot_system()?;
                #[cfg(target_os = "windows")]
                win_power::reboot_system()?;
                #[cfg(target_os = "linux")]
                {
                    let _ = std::process::Command::new("systemctl")
                        .arg("reboot")
                        .status()?;
                }
                Ok("reboot dispatched".to_string())
            }
            PowerActionKind::Shutdown => {
                #[cfg(target_os = "macos")]
                macos_power::shutdown_system(None)?;
                #[cfg(target_os = "windows")]
                win_power::shutdown_system(None)?;
                #[cfg(target_os = "linux")]
                {
                    let _ = std::process::Command::new("systemctl")
                        .arg("poweroff")
                        .status()?;
                }
                Ok("shutdown dispatched".to_string())
            }
            PowerActionKind::ScheduleShutdown { delay_minutes } => {
                #[cfg(target_os = "macos")]
                macos_power::shutdown_system(Some(*delay_minutes))?;
                #[cfg(target_os = "windows")]
                win_power::shutdown_system(Some(*delay_minutes))?;
                #[cfg(target_os = "linux")]
                {
                    let arg = format!("+{delay_minutes}");
                    let _ = std::process::Command::new("shutdown")
                        .args(["-h", &arg])
                        .status()?;
                }
                Ok(format!("scheduled shutdown in {delay_minutes}m"))
            }
        }
    }

    pub fn verify_touch_id(_reason: &str) -> EngineResult<bool> {
        if is_headless_test_env() {
            return Ok(true);
        }
        #[cfg(target_os = "macos")]
        #[allow(unexpected_cfgs)]
        {
            #[link(name = "LocalAuthentication", kind = "framework")]
            unsafe extern "C" {}
            use objc::runtime::{Class, Object};
            use objc::{msg_send, sel, sel_impl};
            unsafe {
                if let Some(ctx_cls) = Class::get("LAContext") {
                    let ctx: *mut Object = msg_send![ctx_cls, new];
                    if !ctx.is_null() {
                        let mut err: *mut Object = std::ptr::null_mut();
                        // LAPolicyDeviceOwnerAuthenticationWithBiometrics = 1
                        let can_eval: bool =
                            msg_send![ctx, canEvaluatePolicy:1i64 error:&mut err];
                        let _: () = msg_send![ctx, release];
                        return Ok(can_eval);
                    }
                }
            }
        }
        Ok(false)
    }
}
