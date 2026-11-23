use crate::net::http_client::{HttpClient, HttpClientError};
use crate::net::http_helpers::percent_encode_rfc3986;
use base64::Engine as _;
use fabushi_remote_engine::accessibility::AccessibilityEngine;
use fabushi_remote_engine::audio::AudioPipeline;
use fabushi_remote_engine::capture::{CaptureFormat, ScreenCaptureEngine};
use fabushi_remote_engine::clipboard::ClipboardSyncEngine;
use fabushi_remote_engine::control_lease::RemoteControlHub;
use fabushi_remote_engine::display_privacy::DisplayPrivacyManager;
use fabushi_remote_engine::file_transfer::DEFAULT_CHUNK_SIZE;
use fabushi_remote_engine::input::{KeyboardActionParams, MouseActionParams, NativeInputEngine};
use fabushi_remote_engine::power::PowerManager;
use fabushi_remote_engine::terminal::PtyMuxServer;
use gpui_telegram_proto::computer_contracts::{
    ApproveComputerCommandRequest, BotComputerProfileDto, ComputerListResponse, ComputerNodeDto,
    PendingComputerCommandDto, RegisterLocalComputerRequest, RemoteComputerCommandRequest,
    RemoteComputerCommandResult, RemoteComputerOp, UpdateBotComputerRequest,
    UpdateComputerPolicyRequest, UpsertCloudComputerRequest,
};
use gpui_telegram_proto::remote_control_contracts::{
    AccessibilityNodeDto, AssistAllowRequest, AssistSetCodeRequest, AssistStartRequest,
    AssistStartResponse, ClipboardPayloadDto, DisplayInfoDto, McpOAuthGrantsResponse,
    RemoteAssistConfigDto, RemoteSessionDisconnectRequest, RemoteSessionsResponse,
};
use gpui_telegram_proto::tailnet_contracts::{
    TailnetNetMap, TailnetRegisterRequest, TailnetStunResponse,
};
use reqwest::Method;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const MAX_LOCAL_OUTPUT_BYTES: usize = 64 * 1024;
const DEFAULT_SHELL_TIMEOUT_MS: u64 = 20_000;

/// Shared local remote-control engine instances powered by `fabushi_remote_engine`.
pub struct LocalRemoteEngine {
    pub hub: Arc<RemoteControlHub>,
    pub pty_mux: Arc<PtyMuxServer>,
    pub capture: Arc<ScreenCaptureEngine>,
    pub input: Arc<NativeInputEngine>,
    pub clipboard: Arc<ClipboardSyncEngine>,
    pub display_privacy: Arc<DisplayPrivacyManager>,
    pub accessibility: Arc<AccessibilityEngine>,
    pub audio: Arc<AudioPipeline>,
}

/// Returns the process-wide `LocalRemoteEngine` singleton shared across `gpui_telegram_client`
/// and `gpui_telegram_desktop` IPC / UI runtime.
pub fn shared_remote_engine() -> &'static LocalRemoteEngine {
    static ENGINE: OnceLock<LocalRemoteEngine> = OnceLock::new();
    ENGINE.get_or_init(|| LocalRemoteEngine {
        hub: RemoteControlHub::shared(),
        pty_mux: Arc::new(PtyMuxServer::new()),
        capture: Arc::new(ScreenCaptureEngine::new()),
        input: Arc::new(NativeInputEngine::new()),
        clipboard: Arc::new(ClipboardSyncEngine::new(format!(
            "host-{}",
            std::process::id()
        ))),
        display_privacy: Arc::new(DisplayPrivacyManager::new()),
        accessibility: Arc::new(AccessibilityEngine),
        audio: Arc::new(AudioPipeline::new()),
    })
}

fn sanitize_hostname_candidate(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Detects the current host's hostname cleanly via macOS `scutil`, `gethostname`, or env fallback,
/// rejecting raw IP strings as hostnames.
pub fn detect_local_hostname() -> String {
    #[cfg(target_os = "macos")]
    if std::env::var_os("GPUI_OFFLINE_TEST").is_none() && !cfg!(test) {
        for key in ["ComputerName", "LocalHostName"] {
            if let Ok(output) = std::process::Command::new("scutil")
                .args(["--get", key])
                .output()
            {
                if output.status.success() {
                    if let Ok(raw) = std::str::from_utf8(&output.stdout) {
                        if let Some(clean) = sanitize_hostname_candidate(raw) {
                            return clean;
                        }
                    }
                }
            }
        }
    }

    #[allow(unused_mut)]
    let mut saw_ip_hostname = false;
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            if let Ok(name) = std::str::from_utf8(&buf[..end]) {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    if trimmed.parse::<std::net::IpAddr>().is_ok() {
                        saw_ip_hostname = true;
                    } else {
                        return trimmed.to_string();
                    }
                }
            }
        }
    }

    for var in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(val) = std::env::var(var) {
            if let Some(clean) = sanitize_hostname_candidate(&val) {
                return clean;
            }
        }
    }

    if cfg!(target_os = "macos") && saw_ip_hostname {
        return format!("Mac ({})", std::env::consts::ARCH);
    }

    "localhost".to_string()
}

fn is_ignored_interface_name(if_name: &str) -> bool {
    let lower = if_name.to_ascii_lowercase();
    const IGNORED_PREFIXES: &[&str] = &[
        "lo", "utun", "tun", "tap", "docker", "br-", "veth", "bridge", "awdl", "llw", "vmnet",
        "vboxnet",
    ];
    IGNORED_PREFIXES.iter().any(|p| lower.starts_with(p))
}

fn is_invalid_or_virtual_lan_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_unspecified()
        || o[0] == 127
        || (o[0] == 169 && o[1] == 254)
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
        || o[0] >= 224
}

fn is_docker_bridge_172(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 172 && (o[1] == 17 || o[1] == 18)
}

fn is_cgnat_100(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..=127).contains(&o[1])
}

fn is_preferred_lan_rfc1918(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    (o[0] == 192 && o[1] == 168) || o[0] == 10
}

fn is_clean_rfc1918(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    is_preferred_lan_rfc1918(ip)
        || (o[0] == 172 && (16..=31).contains(&o[1]) && o[1] != 17 && o[1] != 18)
}

fn is_primary_physical_iface(if_name: &str) -> bool {
    let lower = if_name.to_ascii_lowercase();
    lower.starts_with("en")
        || lower.starts_with("eth")
        || lower.starts_with("wlan")
        || lower.starts_with("wl")
}

/// Detects real LAN IPv4 addresses sorted by physical NIC & RFC1918 priority,
/// filtering out down/loopback/tunnel/bridge interfaces and fake-IP/link-local ranges.
pub fn detect_real_lan_ips() -> Vec<String> {
    #[allow(unused_mut)]
    let mut candidates: Vec<(u8, String, std::net::Ipv4Addr)> = Vec::new();

    #[cfg(unix)]
    {
        struct IfAddrsGuard(*mut libc::ifaddrs);
        impl Drop for IfAddrsGuard {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe { libc::freeifaddrs(self.0) };
                }
            }
        }

        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        let rc = unsafe { libc::getifaddrs(&mut ifap) };
        if rc == 0 && !ifap.is_null() {
            let _guard = IfAddrsGuard(ifap);
            let mut cur = ifap;
            while !cur.is_null() {
                let ifa = unsafe { &*cur };
                cur = ifa.ifa_next;

                let is_up = (ifa.ifa_flags as u64) & (libc::IFF_UP as u64) != 0;
                let is_loopback = (ifa.ifa_flags as u64) & (libc::IFF_LOOPBACK as u64) != 0;
                if !is_up || is_loopback || ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
                    continue;
                }

                let if_name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }
                    .to_string_lossy()
                    .into_owned();
                if is_ignored_interface_name(&if_name) {
                    continue;
                }

                let sa = unsafe { std::ptr::read_unaligned(ifa.ifa_addr) };
                if sa.sa_family as i32 != libc::AF_INET {
                    continue;
                }

                let sin =
                    unsafe { std::ptr::read_unaligned(ifa.ifa_addr as *const libc::sockaddr_in) };
                let ipv4 = std::net::Ipv4Addr::from(sin.sin_addr.s_addr.to_ne_bytes());
                if is_invalid_or_virtual_lan_ipv4(ipv4) {
                    continue;
                }

                let primary_nic = is_primary_physical_iface(&if_name);
                let tier = if primary_nic && is_preferred_lan_rfc1918(ipv4) {
                    0
                } else if primary_nic && is_clean_rfc1918(ipv4) {
                    1
                } else if is_clean_rfc1918(ipv4) {
                    2
                } else if primary_nic && !is_docker_bridge_172(ipv4) && !is_cgnat_100(ipv4) {
                    3
                } else if !is_docker_bridge_172(ipv4) && !is_cgnat_100(ipv4) {
                    4
                } else if is_cgnat_100(ipv4) {
                    5
                } else {
                    6
                };

                candidates.push((tier, if_name, ipv4));
            }
        }
    }

    if candidates.iter().any(|(_, _, ip)| is_preferred_lan_rfc1918(*ip)) {
        candidates.retain(|(_, _, ip)| !is_docker_bridge_172(*ip));
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(&b.2)));

    let mut out = Vec::new();
    for (_, _, ip) in candidates {
        let s = ip.to_string();
        if !out.contains(&s) {
            out.push(s);
        }
    }

    if out.is_empty() {
        if let Ok(addr) = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|sock| {
            sock.connect("8.8.8.8:80")?;
            sock.local_addr()
        }) {
            if let std::net::IpAddr::V4(ipv4) = addr.ip() {
                if !is_invalid_or_virtual_lan_ipv4(ipv4) {
                    out.push(ipv4.to_string());
                }
            }
        }
    }

    out
}

/// Detects `(ip_address, os_name, resolution)` for the local host machine.
pub fn detect_local_host_info() -> (String, String, String) {
    let real_ips = detect_real_lan_ips();
    let has_non_bridge_lan = real_ips.iter().any(|ip| {
        ip.parse::<std::net::Ipv4Addr>()
            .map(|v4| !is_docker_bridge_172(v4) && !is_invalid_or_virtual_lan_ipv4(v4))
            .unwrap_or(false)
    });
    let ip = if has_non_bridge_lan {
        real_ips
            .into_iter()
            .find(|s| {
                s.parse::<std::net::Ipv4Addr>()
                    .map(|v4| !is_docker_bridge_172(v4) && !is_invalid_or_virtual_lan_ipv4(v4))
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        real_ips
            .into_iter()
            .find(|s| {
                s.parse::<std::net::Ipv4Addr>()
                    .map(|v4| !is_invalid_or_virtual_lan_ipv4(v4))
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| "127.0.0.1".to_string())
    };

    let os_pretty = match std::env::consts::OS {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    };
    let hostname = detect_local_hostname();
    let os_name = format!("{} {} ({})", os_pretty, std::env::consts::ARCH, hostname);
    let resolution = std::env::var("GPUI_SCREEN_RESOLUTION")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "1440x900".to_string());

    (ip, os_name, resolution)
}

pub(crate) fn build_local_register_request(device_id: String) -> RegisterLocalComputerRequest {
    let (ip_address, os_name, resolution) = detect_local_host_info();
    let display_name = detect_local_hostname();
    RegisterLocalComputerRequest {
        device_id,
        display_name,
        ip_address,
        os_name,
        resolution,
    }
}

fn truncate_bytes_lossy(bytes: &[u8], max_bytes: usize) -> String {
    if bytes.len() <= max_bytes {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let lossy = String::from_utf8_lossy(&bytes[..max_bytes]);
    format!("{}\n...[truncated]", lossy)
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let b0 = input[i] as usize;
        let b1 = input[i + 1] as usize;
        let b2 = input[i + 2] as usize;
        out.push(TABLE[b0 >> 2] as char);
        out.push(TABLE[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        out.push(TABLE[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        out.push(TABLE[b2 & 0x3f] as char);
        i += 3;
    }
    match input.len() - i {
        1 => {
            let b0 = input[i] as usize;
            out.push(TABLE[b0 >> 2] as char);
            out.push(TABLE[(b0 & 0x03) << 4] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b0 = input[i] as usize;
            let b1 = input[i + 1] as usize;
            out.push(TABLE[b0 >> 2] as char);
            out.push(TABLE[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
            out.push(TABLE[(b1 & 0x0f) << 2] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }
    (b << 16) | a
}

fn zlib_stored_compress(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 32);
    out.push(0x78);
    out.push(0x01);
    if raw.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    } else {
        let mut offset = 0;
        while offset < raw.len() {
            let end = (offset + 65535).min(raw.len());
            let chunk = &raw[offset..end];
            let is_last = end == raw.len();
            let len = chunk.len() as u16;
            let nlen = !len;
            out.push(if is_last { 0x01 } else { 0x00 });
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&nlen.to_le_bytes());
            out.extend_from_slice(chunk);
            offset = end;
        }
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

fn push_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = crc32_ieee(&out[crc_start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn generate_fallback_screen_png_bytes(width: u32, height: u32) -> Vec<u8> {
    let row_bytes = 1 + (width as usize) * 3;
    let mut raw = Vec::with_capacity(row_bytes * (height as usize));
    for y in 0..height {
        raw.push(0); // Filter type None
        for x in 0..width {
            let is_top_bar = y < 12;
            let is_window_card =
                x >= 14 && x < width.saturating_sub(14) && y >= 20 && y < height.saturating_sub(12);
            let is_border = is_window_card
                && (x == 14
                    || x + 1 == width.saturating_sub(14)
                    || y == 20
                    || y + 1 == height.saturating_sub(12));
            let (r, g, b) = if is_top_bar {
                (36u8, 129u8, 204u8)
            } else if is_border {
                (71u8, 85u8, 105u8)
            } else if is_window_card {
                (30u8, 41u8, 59u8)
            } else {
                let grad = ((y * 24) / height.max(1)) as u8;
                (15u8.saturating_add(grad / 2), 23u8.saturating_add(grad / 2), 42u8.saturating_add(grad))
            };
            raw.push(r);
            raw.push(g);
            raw.push(b);
        }
    }

    let mut png = Vec::with_capacity(raw.len() + 128);
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // color type: RGB
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    push_png_chunk(&mut png, b"IHDR", &ihdr);

    let idat = zlib_stored_compress(&raw);
    push_png_chunk(&mut png, b"IDAT", &idat);
    push_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn is_headless_or_unit_test() -> bool {
    cfg!(test)
        || std::env::var_os("GPUI_OFFLINE_TEST").is_some()
        || fabushi_remote_engine::is_headless_test_env()
}

fn headless_clipboard_slot() -> &'static parking_lot::RwLock<ClipboardPayloadDto> {
    static CLIP: OnceLock<parking_lot::RwLock<ClipboardPayloadDto>> = OnceLock::new();
    CLIP.get_or_init(|| {
        parking_lot::RwLock::new(ClipboardPayloadDto {
            mime_type: "text/plain".to_string(),
            text: Some(String::new()),
            image_b64: None,
            origin_id: "headless-init".to_string(),
            seq: 1,
        })
    })
}

async fn capture_screen_png_data_url(force_fallback: bool) -> (String, &'static str, Option<String>) {
    let mut capture_error: Option<(bool, String)> = None;
    if !force_fallback
        && !is_headless_or_unit_test()
        && std::env::var_os("GPUI_SKIP_NATIVE_SCREENCAPTURE").is_none()
    {
        let engine = shared_remote_engine();
        let display_idx = engine.display_privacy.active_display_index();
        let capture_res = tokio::task::spawn_blocking(move || {
            engine.capture.capture_screen(display_idx, CaptureFormat::Png)
        })
        .await;

        match capture_res {
            Ok(Ok(frame))
                if frame.data.len() > 8
                    && (frame.data.starts_with(b"\x89PNG\r\n\x1a\n")
                        || frame.data.starts_with(b"\xff\xd8\xff")) =>
            {
                return (frame.to_data_url(), "fabushi_remote_engine", None);
            }
            Ok(Ok(_)) => {
                capture_error = Some((false, "screen capture returned an empty or invalid frame".to_string()));
            }
            Ok(Err(err)) => {
                let is_perm = matches!(
                    err,
                    fabushi_remote_engine::RemoteEngineError::PermissionDenied(_)
                );
                capture_error = Some((is_perm, err.to_string()));
            }
            Err(join_err) => {
                capture_error = Some((false, format!("screen capture worker join error: {join_err}")));
            }
        }
    }

    let png_bytes = generate_fallback_screen_png_bytes(160, 90);
    let b64 = encode_base64(&png_bytes);
    let data_url = format!("data:image/png;base64,{}", b64);
    match capture_error {
        Some((true, msg)) => (data_url, "fallback_png_permission_denied", Some(msg)),
        Some((false, msg)) => (data_url, "fallback_png_error", Some(msg)),
        None => (data_url, "fallback_png", None),
    }
}

/// Executes a `RemoteComputerOp` directly on the local host machine via `fabushi_remote_engine`.
pub async fn execute_local_computer_op(
    node_id: &str,
    command_id: &str,
    op: &RemoteComputerOp,
) -> RemoteComputerCommandResult {
    execute_local_computer_op_inner(node_id, command_id, op, false).await
}

async fn execute_local_computer_op_inner(
    node_id: &str,
    command_id: &str,
    op: &RemoteComputerOp,
    force_fallback_capture: bool,
) -> RemoteComputerCommandResult {
    let started = Instant::now();
    let executed_by = format!("local-host:{}", detect_local_hostname());
    let engine = shared_remote_engine();

    let mut result = RemoteComputerCommandResult {
        command_id: command_id.to_string(),
        node_id: node_id.to_string(),
        ok: false,
        exit_code: -1,
        stdout: String::new(),
        stderr: String::new(),
        entries: Vec::new(),
        frame_data_url: None,
        executed_by,
        duration_ms: 0,
    };

    match op {
        RemoteComputerOp::ShellExec {
            command,
            cwd,
            timeout_ms,
        } => {
            #[cfg(windows)]
            let mut cmd = {
                let mut c = tokio::process::Command::new("cmd");
                c.arg("/C").arg(command);
                c
            };
            #[cfg(not(windows))]
            let mut cmd = {
                let mut c = tokio::process::Command::new("sh");
                c.arg("-c").arg(command);
                c
            };

            if let Some(dir) = cwd.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                cmd.current_dir(dir);
            }
            cmd.kill_on_drop(true);

            let wait_ms = timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS).max(100);
            match tokio::time::timeout(Duration::from_millis(wait_ms), cmd.output()).await {
                Ok(Ok(output)) => {
                    result.exit_code = output.status.code().unwrap_or(-1);
                    result.ok = output.status.success();
                    result.stdout = truncate_bytes_lossy(&output.stdout, MAX_LOCAL_OUTPUT_BYTES);
                    result.stderr = truncate_bytes_lossy(&output.stderr, MAX_LOCAL_OUTPUT_BYTES);
                }
                Ok(Err(err)) => {
                    result.stderr = format!("Failed to spawn command: {}", err);
                }
                Err(_) => {
                    result.stderr = format!("Command timed out after {} ms", wait_ms);
                }
            }
        }
        RemoteComputerOp::ReadFile { path, max_bytes } => {
            let limit = max_bytes
                .unwrap_or(MAX_LOCAL_OUTPUT_BYTES)
                .clamp(1, 1024 * 1024);
            match tokio::fs::read(path).await {
                Ok(bytes) => {
                    result.ok = true;
                    result.exit_code = 0;
                    result.stdout = truncate_bytes_lossy(&bytes, limit);
                }
                Err(err) => {
                    result.exit_code = 1;
                    result.stderr = format!("Failed to read '{}': {}", path, err);
                }
            }
        }
        RemoteComputerOp::WriteFile {
            path,
            content,
            create_dirs,
        } => {
            if *create_dirs {
                if let Some(parent) = Path::new(path).parent() {
                    if !parent.as_os_str().is_empty() {
                        if let Err(err) = tokio::fs::create_dir_all(parent).await {
                            result.exit_code = 1;
                            result.stderr =
                                format!("Failed to create parent directories for '{}': {}", path, err);
                            result.duration_ms = started.elapsed().as_millis() as u64;
                            return result;
                        }
                    }
                }
            }
            match tokio::fs::write(path, content.as_bytes()).await {
                Ok(()) => {
                    result.ok = true;
                    result.exit_code = 0;
                    result.stdout = format!("Wrote {} bytes to {}", content.len(), path);
                }
                Err(err) => {
                    result.exit_code = 1;
                    result.stderr = format!("Failed to write '{}': {}", path, err);
                }
            }
        }
        RemoteComputerOp::ListDir { path } => match tokio::fs::read_dir(path).await {
            Ok(mut reader) => {
                let mut entries = Vec::new();
                while let Ok(Some(entry)) = reader.next_entry().await {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let is_dir = entry
                        .file_type()
                        .await
                        .map(|ft| ft.is_dir())
                        .unwrap_or(false);
                    let tag = if is_dir { "[dir]" } else { "[file]" };
                    entries.push(format!("{} {}", tag, name));
                }
                entries.sort();
                result.ok = true;
                result.exit_code = 0;
                result.stdout = entries.join("\n");
                result.entries = entries;
            }
            Err(err) => {
                result.exit_code = 1;
                result.stderr = format!("Failed to list directory '{}': {}", path, err);
            }
        },
        RemoteComputerOp::CaptureScreen => {
            let (data_url, capture_mode, capture_err) =
                capture_screen_png_data_url(force_fallback_capture).await;
            result.ok = true;
            result.exit_code = 0;
            result.stdout = format!("Captured screen frame ({})", capture_mode);
            if let Some(err_msg) = capture_err {
                result.stderr = err_msg;
            }
            result.frame_data_url = Some(data_url);
        }
        RemoteComputerOp::GuiInput {
            action,
            x,
            y,
            text,
            key,
        } => {
            let action_lc = action.trim().to_ascii_lowercase();
            let engine_res = if is_headless_or_unit_test() {
                Ok(None)
            } else if matches!(
                action_lc.as_str(),
                "click" | "move" | "double_click" | "right_click" | "down" | "up" | "scroll"
            ) {
                engine
                    .input
                    .execute_mouse_action(&MouseActionParams {
                        action: action_lc.clone(),
                        x: *x,
                        y: *y,
                        dx: None,
                        dy: None,
                        button: Some("left".to_string()),
                        clicks: Some(1),
                        scroll_x: None,
                        scroll_y: None,
                        relative: false,
                    })
                    .map(|_| Some("fabushi_remote_engine"))
            } else {
                engine
                    .input
                    .execute_keyboard_action(&KeyboardActionParams {
                        action: action_lc.clone(),
                        text: text.clone(),
                        key: key.clone(),
                        modifiers: Vec::new(),
                    })
                    .map(|_| Some("fabushi_remote_engine"))
            };

            match engine_res {
                Ok(Some(driver)) => {
                    result.ok = true;
                    result.exit_code = 0;
                    result.stdout = format!(
                        "gui_input delivered ({}): action={} x={:?} y={:?} text_len={} key={:?}",
                        driver,
                        action,
                        x,
                        y,
                        text.as_ref().map(|t| t.len()).unwrap_or(0),
                        key
                    );
                }
                Ok(None) => {
                    result.ok = true;
                    result.exit_code = 0;
                    result.stdout = format!(
                        "gui_input acknowledged: action={} x={:?} y={:?} text_len={} key={:?}",
                        action,
                        x,
                        y,
                        text.as_ref().map(|t| t.len()).unwrap_or(0),
                        key
                    );
                }
                Err(err) => {
                    result.ok = false;
                    result.exit_code = 1;
                    result.stderr = err.to_string();
                }
            }
        }
        RemoteComputerOp::MouseAction {
            action,
            x,
            y,
            dx,
            dy,
            button,
            clicks,
            scroll_x,
            scroll_y,
            relative,
        } => {
            if is_headless_or_unit_test() {
                result.ok = true;
                result.exit_code = 0;
                result.stdout = format!(
                    "mouse_action acknowledged (headless): action={} x={:?} y={:?} relative={}",
                    action, x, y, relative
                );
            } else if let Err(err) = engine.input.set_relative_mouse(*relative) {
                result.exit_code = 1;
                result.stderr = err.to_string();
            } else if let Err(err) = engine.input.execute_mouse_action(&MouseActionParams {
                action: action.clone(),
                x: *x,
                y: *y,
                dx: *dx,
                dy: *dy,
                button: button.clone(),
                clicks: *clicks,
                scroll_x: *scroll_x,
                scroll_y: *scroll_y,
                relative: *relative,
            }) {
                result.exit_code = 1;
                result.stderr = err.to_string();
            } else {
                result.ok = true;
                result.exit_code = 0;
                result.stdout = format!(
                    "mouse_action acknowledged: action={} x={:?} y={:?} relative={}",
                    action, x, y, relative
                );
            }
        }
        RemoteComputerOp::KeyboardAction {
            action,
            text,
            key,
            modifiers,
        } => {
            if is_headless_or_unit_test() {
                result.ok = true;
                result.exit_code = 0;
                result.stdout = format!(
                    "keyboard_action acknowledged (headless): action={} key={:?} modifiers={:?}",
                    action, key, modifiers
                );
            } else {
                match engine.input.execute_keyboard_action(&KeyboardActionParams {
                    action: action.clone(),
                    text: text.clone(),
                    key: key.clone(),
                    modifiers: modifiers.clone(),
                }) {
                    Ok(()) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = format!(
                            "keyboard_action acknowledged: action={} key={:?} modifiers={:?}",
                            action, key, modifiers
                        );
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                }
            }
        }
        RemoteComputerOp::ClipboardGet => {
            if is_headless_or_unit_test() {
                let payload = headless_clipboard_slot().read().clone();
                result.ok = true;
                result.exit_code = 0;
                result.stdout = serde_json::to_string(&payload).unwrap_or_default();
            } else {
                match engine.clipboard.get_local() {
                    Ok(payload) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = serde_json::to_string(&payload).unwrap_or_default();
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                }
            }
        }
        RemoteComputerOp::ClipboardSet {
            text,
            image_b64,
            mime_type,
        } => {
            let payload = ClipboardPayloadDto {
                mime_type: mime_type
                    .clone()
                    .unwrap_or_else(|| "text/plain".to_string()),
                text: text.clone(),
                image_b64: image_b64.clone(),
                origin_id: format!("remote-op:{}", command_id),
                seq: chrono::Utc::now().timestamp_millis().max(1) as u64,
            };
            if is_headless_or_unit_test() {
                *headless_clipboard_slot().write() = payload;
                result.ok = true;
                result.exit_code = 0;
                result.stdout = serde_json::json!({ "applied": true }).to_string();
            } else {
                match engine.clipboard.apply_remote(&payload) {
                    Ok(applied) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = serde_json::json!({ "applied": applied }).to_string();
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                }
            }
        }
        RemoteComputerOp::PowerAction { action } => {
            if is_headless_or_unit_test() {
                result.ok = true;
                result.exit_code = 0;
                result.stdout = format!("headless power action acknowledged: {:?}", action);
            } else {
                match PowerManager::execute_power_action(action) {
                    Ok(msg) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = msg;
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                }
            }
        }
        RemoteComputerOp::DisplayList => {
            if is_headless_or_unit_test() {
                let mut displays = vec![DisplayInfoDto {
                    display_id: 0,
                    name: "Headless Virtual Display".to_string(),
                    width: 1920,
                    height: 1080,
                    scale_factor_x100: 100,
                    is_primary: true,
                    is_virtual: false,
                    refresh_hz: 60,
                }];
                displays.extend(engine.display_privacy.virtual_displays());
                result.ok = true;
                result.exit_code = 0;
                result.entries = displays
                    .iter()
                    .map(|d| format!("{}:{} ({}x{})", d.display_id, d.name, d.width, d.height))
                    .collect();
                result.stdout = serde_json::to_string(&displays).unwrap_or_default();
            } else {
                match engine.capture.list_displays() {
                    Ok(mut displays) => {
                        displays.extend(engine.display_privacy.virtual_displays());
                        result.ok = true;
                        result.exit_code = 0;
                        result.entries = displays
                            .iter()
                            .map(|d| format!("{}:{} ({}x{})", d.display_id, d.name, d.width, d.height))
                            .collect();
                        result.stdout = serde_json::to_string(&displays).unwrap_or_default();
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                }
            }
        }
        RemoteComputerOp::DisplaySwitch { display_index } => {
            match engine.display_privacy.switch_display(*display_index) {
                Ok(idx) => {
                    result.ok = true;
                    result.exit_code = 0;
                    result.stdout = format!("Switched active display to {idx}");
                }
                Err(err) => {
                    result.exit_code = 1;
                    result.stderr = err.to_string();
                }
            }
        }
        RemoteComputerOp::PrivacyScreenToggle { enabled, message } => {
            match engine
                .display_privacy
                .set_privacy_screen(*enabled, message.clone())
            {
                Ok(state) => {
                    result.ok = true;
                    result.exit_code = 0;
                    result.stdout = format!("Privacy screen enabled={state}");
                }
                Err(err) => {
                    result.exit_code = 1;
                    result.stderr = err.to_string();
                }
            }
        }
        RemoteComputerOp::TerminalOp {
            op: sub_op,
            session_name,
            shell,
            input,
            command,
            cols,
            rows,
            timeout_ms,
            new_name,
        } => {
            let target = session_name.as_deref().unwrap_or("default");
            match sub_op.as_str() {
                "ls" => {
                    let list = engine.pty_mux.list_sessions();
                    result.ok = true;
                    result.exit_code = 0;
                    result.entries = list.iter().map(|s| s.name.clone()).collect();
                    result.stdout = serde_json::to_string(&list).unwrap_or_default();
                }
                "has" => {
                    let exists = engine.pty_mux.has_session(target);
                    result.ok = exists;
                    result.exit_code = if exists { 0 } else { 1 };
                    result.stdout = serde_json::json!({ "exists": exists }).to_string();
                }
                "new" => match engine.pty_mux.new_session(
                    session_name.as_deref(),
                    shell.as_deref(),
                    cols.unwrap_or(100),
                    rows.unwrap_or(28),
                    None,
                ) {
                    Ok(dto) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = serde_json::to_string(&dto).unwrap_or_default();
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                },
                "kill" => match engine.pty_mux.kill_session(target) {
                    Ok(killed) => {
                        result.ok = killed;
                        result.exit_code = if killed { 0 } else { 1 };
                        result.stdout = serde_json::json!({ "killed": killed }).to_string();
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                },
                "rename" => {
                    let next = new_name.as_deref().unwrap_or("");
                    match engine.pty_mux.rename_session(target, next) {
                        Ok(dto) => {
                            result.ok = true;
                            result.exit_code = 0;
                            result.stdout = serde_json::to_string(&dto).unwrap_or_default();
                        }
                        Err(err) => {
                            result.exit_code = 1;
                            result.stderr = err.to_string();
                        }
                    }
                }
                "resize" => match engine.pty_mux.resize_session(
                    target,
                    cols.unwrap_or(100),
                    rows.unwrap_or(28),
                    true,
                    false,
                ) {
                    Ok(()) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = "resized".to_string();
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                },
                "write" | "send" => {
                    let data = input.as_deref().unwrap_or("");
                    match engine.pty_mux.write_input(target, data.as_bytes()) {
                        Ok(()) => {
                            result.ok = true;
                            result.exit_code = 0;
                            result.stdout = format!("wrote {} bytes", data.len());
                        }
                        Err(err) => {
                            result.exit_code = 1;
                            result.stderr = err.to_string();
                        }
                    }
                }
                "read" => match engine.pty_mux.read_scrollback(target, Some(400)) {
                    Ok(txt) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = txt;
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                },
                "exec" => {
                    let cmd_str = command.as_deref().unwrap_or("");
                    let needs_spawn = match engine.pty_mux.get_session(target) {
                        Some(dto) if !dto.alive && session_name.is_none() => {
                            let _ = engine.pty_mux.kill_session(target);
                            true
                        }
                        None => true,
                        Some(_) => false,
                    };
                    let init_res = if needs_spawn {
                        engine
                            .pty_mux
                            .new_session(
                                Some(target),
                                shell.as_deref(),
                                cols.unwrap_or(100),
                                rows.unwrap_or(28),
                                None,
                            )
                            .map(|_| ())
                    } else {
                        Ok(())
                    };
                    if let Err(err) = init_res {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    } else {
                        match engine
                            .pty_mux
                            .exec_command(target, cmd_str, timeout_ms.unwrap_or(15_000))
                            .await
                        {
                            Ok(exec_res) => {
                                result.ok = !exec_res.timed_out && exec_res.exit_code == 0;
                                result.exit_code = exec_res.exit_code;
                                result.stdout = serde_json::to_string(&exec_res).unwrap_or_default();
                            }
                            Err(err) => {
                                result.exit_code = 1;
                                result.stderr = err.to_string();
                            }
                        }
                    }
                }
                other => {
                    result.exit_code = 1;
                    result.stderr = format!("Unsupported terminal_op '{other}'");
                }
            }
        }
        RemoteComputerOp::AccessibilityTree { max_depth } => {
            if is_headless_or_unit_test() {
                let tree = AccessibilityNodeDto {
                    element_id: "root-0".to_string(),
                    role: "AXApplication".to_string(),
                    name: Some("Fabushi Desktop".to_string()),
                    value: None,
                    bounds: Some([0, 0, 1440, 900]),
                    actions: vec!["AXRaise".to_string()],
                    children: vec![AccessibilityNodeDto {
                        element_id: "win-1".to_string(),
                        role: "AXWindow".to_string(),
                        name: Some("Main".to_string()),
                        value: None,
                        bounds: Some([0, 0, 1280, 820]),
                        actions: vec!["AXPress".to_string()],
                        children: Vec::new(),
                    }],
                };
                result.ok = true;
                result.exit_code = 0;
                result.stdout = serde_json::to_string(&tree).unwrap_or_default();
            } else {
                match engine.accessibility.capture_tree(*max_depth) {
                    Ok(tree) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = serde_json::to_string(&tree).unwrap_or_default();
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                }
            }
        }
        RemoteComputerOp::SemanticAction {
            element_id,
            action,
            value,
        } => {
            if is_headless_or_unit_test() {
                result.ok = true;
                result.exit_code = 0;
                result.stdout = format!("semantic_action {action} on {element_id} (headless)");
            } else {
                match engine
                    .accessibility
                    .perform_semantic_action(element_id, action, value.as_deref())
                {
                    Ok(()) => {
                        result.ok = true;
                        result.exit_code = 0;
                        result.stdout = format!("semantic_action {action} on {element_id}");
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                }
            }
        }
        RemoteComputerOp::FileChunkOp {
            op: chunk_op,
            transfer_id,
            path,
            offset,
            data_b64,
            sha256,
            total_size: _,
        } => match chunk_op.as_str() {
            "read_chunk" => {
                use tokio::io::{AsyncReadExt, AsyncSeekExt};
                match tokio::fs::File::open(path).await {
                    Ok(mut file) => {
                        let meta_len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
                        if let Err(err) = file.seek(std::io::SeekFrom::Start(*offset)).await {
                            result.exit_code = 1;
                            result.stderr = err.to_string();
                        } else {
                            let mut buf = vec![0u8; DEFAULT_CHUNK_SIZE];
                            match file.read(&mut buf).await {
                                Ok(n) => {
                                    buf.truncate(n);
                                    let digest = hex_sha256(&buf);
                                    let b64 =
                                        base64::engine::general_purpose::STANDARD.encode(&buf);
                                    result.ok = true;
                                    result.exit_code = 0;
                                    result.stdout = serde_json::json!({
                                        "transfer_id": transfer_id,
                                        "path": path,
                                        "offset": offset,
                                        "size": n,
                                        "total_size": meta_len,
                                        "sha256": digest,
                                        "data_b64": b64,
                                    })
                                    .to_string();
                                }
                                Err(err) => {
                                    result.exit_code = 1;
                                    result.stderr = err.to_string();
                                }
                            }
                        }
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = err.to_string();
                    }
                }
            }
            "write_chunk" | "append" | "resume" => {
                use tokio::io::{AsyncSeekExt, AsyncWriteExt};
                let b64 = data_b64.as_deref().unwrap_or("");
                match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                    Ok(bytes) => {
                        let actual_sha = hex_sha256(&bytes);
                        if let Some(expected) = sha256.as_deref().filter(|s| !s.is_empty()) {
                            if !actual_sha.eq_ignore_ascii_case(expected) {
                                result.exit_code = 1;
                                result.stderr = format!(
                                    "SHA-256 mismatch at offset {offset}: expected {expected}, got {actual_sha}"
                                );
                                result.duration_ms = started.elapsed().as_millis() as u64;
                                return result;
                            }
                        }
                        if let Some(parent) = Path::new(path).parent() {
                            if !parent.as_os_str().is_empty() {
                                let _ = tokio::fs::create_dir_all(parent).await;
                            }
                        }
                        match tokio::fs::OpenOptions::new()
                            .create(true)
                            .write(true)
                            .read(true)
                            .open(path)
                            .await
                        {
                            Ok(mut file) => {
                                if let Err(err) = file.seek(std::io::SeekFrom::Start(*offset)).await
                                {
                                    result.exit_code = 1;
                                    result.stderr = err.to_string();
                                } else if let Err(err) = file.write_all(&bytes).await {
                                    result.exit_code = 1;
                                    result.stderr = err.to_string();
                                } else {
                                    let _ = file.flush().await;
                                    result.ok = true;
                                    result.exit_code = 0;
                                    result.stdout = serde_json::json!({
                                        "transfer_id": transfer_id,
                                        "path": path,
                                        "offset": offset,
                                        "bytes_written": bytes.len(),
                                        "sha256": actual_sha,
                                    })
                                    .to_string();
                                }
                            }
                            Err(err) => {
                                result.exit_code = 1;
                                result.stderr = err.to_string();
                            }
                        }
                    }
                    Err(err) => {
                        result.exit_code = 1;
                        result.stderr = format!("Invalid chunk base64: {err}");
                    }
                }
            }
            "stat" => match tokio::fs::metadata(path).await {
                Ok(meta) => {
                    result.ok = true;
                    result.exit_code = 0;
                    result.stdout = serde_json::json!({
                        "transfer_id": transfer_id,
                        "path": path,
                        "total_size": meta.len(),
                        "is_file": meta.is_file(),
                    })
                    .to_string();
                }
                Err(err) => {
                    result.exit_code = 1;
                    result.stderr = err.to_string();
                }
            },
            other => {
                result.exit_code = 1;
                result.stderr = format!("Unsupported file chunk_op '{other}'");
            }
        },
        RemoteComputerOp::TestConnection => {
            let (ip, os, _res) = detect_local_host_info();
            result.ok = true;
            result.exit_code = 0;
            result.stdout = format!("Local device reachable: {} ({})", ip, os);
        }
    }

    result.duration_ms = started.elapsed().as_millis() as u64;
    result
}

impl HttpClient {
    pub async fn list_computers(
        &self,
        token: &str,
    ) -> Result<ComputerListResponse, HttpClientError> {
        self.request_json(Method::GET, "/api/v1/computers", Some(token), None::<&()>)
            .await
    }

    pub async fn register_local_computer(
        &self,
        token: &str,
        req: &RegisterLocalComputerRequest,
    ) -> Result<ComputerNodeDto, HttpClientError> {
        self.request_json(
            Method::POST,
            "/api/v1/computers/local/register",
            Some(token),
            Some(req),
        )
        .await
    }

    pub async fn poll_local_computer_commands(
        &self,
        token: &str,
    ) -> Result<Vec<PendingComputerCommandDto>, HttpClientError> {
        self.request_vec_or_field(
            Method::GET,
            "/api/v1/computers/local/poll",
            Some(token),
            &["commands", "pending_commands", "items"],
        )
        .await
    }

    pub async fn approve_computer_command(
        &self,
        token: &str,
        command_id: &str,
        approved: bool,
    ) -> Result<PendingComputerCommandDto, HttpClientError> {
        let path = format!(
            "/api/v1/computers/commands/{}/approve",
            percent_encode_rfc3986(command_id)
        );
        let body = ApproveComputerCommandRequest { approved };
        self.request_json(Method::POST, &path, Some(token), Some(&body))
            .await
    }

    pub async fn complete_computer_command(
        &self,
        token: &str,
        command_id: &str,
        result: &RemoteComputerCommandResult,
    ) -> Result<(), HttpClientError> {
        let path = format!(
            "/api/v1/computers/commands/{}/complete",
            percent_encode_rfc3986(command_id)
        );
        self.request_empty(Method::POST, &path, Some(token), Some(result))
            .await
    }

    pub async fn upsert_cloud_computer(
        &self,
        token: &str,
        req: &UpsertCloudComputerRequest,
    ) -> Result<ComputerNodeDto, HttpClientError> {
        self.request_json(Method::POST, "/api/v1/computers/cloud", Some(token), Some(req))
            .await
    }

    pub async fn update_computer_policy(
        &self,
        token: &str,
        node_id: &str,
        req: &UpdateComputerPolicyRequest,
    ) -> Result<ComputerNodeDto, HttpClientError> {
        let path = format!(
            "/api/v1/computers/{}/policy",
            percent_encode_rfc3986(node_id)
        );
        self.request_json(Method::PATCH, &path, Some(token), Some(req))
            .await
    }

    pub async fn delete_computer(&self, token: &str, node_id: &str) -> Result<(), HttpClientError> {
        let path = format!("/api/v1/computers/{}", percent_encode_rfc3986(node_id));
        self.request_empty(Method::DELETE, &path, Some(token), None::<&()>)
            .await
    }

    pub async fn test_computer_connection(
        &self,
        token: &str,
        node_id: &str,
    ) -> Result<RemoteComputerCommandResult, HttpClientError> {
        let path = format!(
            "/api/v1/computers/{}/test",
            percent_encode_rfc3986(node_id)
        );
        self.request_json(Method::POST, &path, Some(token), Some(&serde_json::json!({})))
            .await
    }

    pub async fn execute_remote_computer_op(
        &self,
        token: &str,
        req: &RemoteComputerCommandRequest,
    ) -> Result<RemoteComputerCommandResult, HttpClientError> {
        self.request_json(Method::POST, "/api/v1/computers/exec", Some(token), Some(req))
            .await
    }

    pub async fn get_bot_computer_profile(
        &self,
        token: &str,
        bot_id: u64,
    ) -> Result<BotComputerProfileDto, HttpClientError> {
        let path = format!("/api/v1/bots/{}/computer", bot_id);
        self.request_json(Method::GET, &path, Some(token), None::<&()>)
            .await
    }

    pub async fn update_bot_computer_profile(
        &self,
        token: &str,
        bot_id: u64,
        req: &UpdateBotComputerRequest,
    ) -> Result<BotComputerProfileDto, HttpClientError> {
        let path = format!("/api/v1/bots/{}/computer", bot_id);
        self.request_json(Method::PUT, &path, Some(token), Some(req))
            .await
    }

    pub async fn register_tailnet(
        &self,
        token: &str,
        req: &TailnetRegisterRequest,
    ) -> Result<TailnetNetMap, HttpClientError> {
        self.request_json(
            Method::POST,
            "/api/v1/tailnet/register",
            Some(token),
            Some(req),
        )
        .await
    }

    pub async fn get_tailnet_netmap(
        &self,
        token: &str,
    ) -> Result<TailnetNetMap, HttpClientError> {
        self.request_json(
            Method::GET,
            "/api/v1/tailnet/netmap",
            Some(token),
            None::<&()>,
        )
        .await
    }

    pub async fn get_tailnet_stun(
        &self,
        token: &str,
    ) -> Result<TailnetStunResponse, HttpClientError> {
        self.request_json(
            Method::GET,
            "/api/v1/tailnet/stun",
            Some(token),
            None::<&()>,
        )
        .await
    }

    /// Registers or refreshes the current local host machine using `self.device_id()`.
    pub async fn register_current_host_computer(
        &self,
        token: &str,
    ) -> Result<ComputerNodeDto, HttpClientError> {
        let req = build_local_register_request(self.device_id());
        self.register_local_computer(token, &req).await
    }

    /// Polls pending local computer commands, executes those not blocked on approval,
    /// and posts their results back to `/api/v1/computers/commands/{command_id}/complete`.
    pub async fn process_ready_local_computer_commands(
        &self,
        token: &str,
    ) -> Result<Vec<PendingComputerCommandDto>, HttpClientError> {
        let pending = self.poll_local_computer_commands(token).await?;
        let mut processed = Vec::new();
        for cmd in pending {
            if cmd.requires_user_approval && !cmd.approved {
                continue;
            }
            let exec_res =
                execute_local_computer_op(&cmd.node_id, &cmd.command_id, &cmd.operation).await;
            self.complete_computer_command(token, &cmd.command_id, &exec_res)
                .await?;
            processed.push(cmd);
        }
        Ok(processed)
    }

    pub async fn get_remote_assist_info(
        &self,
        token: &str,
    ) -> Result<RemoteAssistConfigDto, HttpClientError> {
        self.request_json(
            Method::GET,
            "/api/v1/remote/assist/info",
            Some(token),
            None::<&()>,
        )
        .await
    }

    pub async fn reset_remote_assist_code(
        &self,
        token: &str,
    ) -> Result<RemoteAssistConfigDto, HttpClientError> {
        self.request_json(
            Method::POST,
            "/api/v1/remote/assist/reset-code",
            Some(token),
            Some(&serde_json::json!({})),
        )
        .await
    }

    pub async fn set_remote_assist_code(
        &self,
        token: &str,
        code: &str,
    ) -> Result<RemoteAssistConfigDto, HttpClientError> {
        let req = AssistSetCodeRequest {
            code: code.to_string(),
        };
        self.request_json(
            Method::POST,
            "/api/v1/remote/assist/set-code",
            Some(token),
            Some(&req),
        )
        .await
    }

    pub async fn set_remote_assist_allow(
        &self,
        token: &str,
        allow_assist: bool,
    ) -> Result<RemoteAssistConfigDto, HttpClientError> {
        let req = AssistAllowRequest { allow_assist };
        self.request_json(
            Method::POST,
            "/api/v1/remote/assist/allow",
            Some(token),
            Some(&req),
        )
        .await
    }

    pub async fn start_remote_assist(
        &self,
        token: &str,
        req: &AssistStartRequest,
    ) -> Result<AssistStartResponse, HttpClientError> {
        self.request_json(
            Method::POST,
            "/api/v1/remote/assist/start",
            Some(token),
            Some(req),
        )
        .await
    }

    pub async fn list_remote_sessions(
        &self,
        token: &str,
    ) -> Result<RemoteSessionsResponse, HttpClientError> {
        self.request_json(
            Method::GET,
            "/api/v1/remote/sessions",
            Some(token),
            None::<&()>,
        )
        .await
    }

    pub async fn disconnect_remote_session(
        &self,
        token: &str,
        session_id: &str,
    ) -> Result<(), HttpClientError> {
        let req = RemoteSessionDisconnectRequest {
            session_id: session_id.to_string(),
        };
        self.request_empty(
            Method::POST,
            "/api/v1/remote/sessions/disconnect",
            Some(token),
            Some(&req),
        )
        .await
    }

    pub async fn disconnect_all_remote_sessions(
        &self,
        token: &str,
    ) -> Result<serde_json::Value, HttpClientError> {
        self.request_json(
            Method::POST,
            "/api/v1/remote/sessions/disconnect-all",
            Some(token),
            Some(&serde_json::json!({})),
        )
        .await
    }

    pub async fn list_mcp_grants(
        &self,
        token: &str,
    ) -> Result<McpOAuthGrantsResponse, HttpClientError> {
        self.request_json(
            Method::GET,
            "/api/v1/mcp/grants",
            Some(token),
            None::<&()>,
        )
        .await
    }

    pub async fn revoke_mcp_grant(
        &self,
        token: &str,
        family_id: &str,
    ) -> Result<(), HttpClientError> {
        let path = format!("/api/v1/mcp/grants/{}", percent_encode_rfc3986(family_id));
        self.request_empty(Method::DELETE, &path, Some(token), None::<&()>)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::TelegramClient;
    use crate::config::ClientConfig;
    use gpui_telegram_proto::computer_contracts::{ComputerNodeKind, ComputerRunPolicy};
    use parking_lot::Mutex;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn decode_base64(input: &str) -> Vec<u8> {
        fn val(b: u8) -> u8 {
            match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => 0,
            }
        }
        let bytes = input.as_bytes();
        let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
        for chunk in bytes.chunks_exact(4) {
            let a = val(chunk[0]) as u32;
            let b = val(chunk[1]) as u32;
            let c = val(chunk[2]) as u32;
            let d = val(chunk[3]) as u32;
            let n = (a << 18) | (b << 12) | (c << 6) | d;
            out.push(((n >> 16) & 0xFF) as u8);
            if chunk[2] != b'=' {
                out.push(((n >> 8) & 0xFF) as u8);
            }
            if chunk[3] != b'=' {
                out.push((n & 0xFF) as u8);
            }
        }
        out
    }

    #[test]
    fn test_png_crc32_adler32_and_fallback_frame_validity() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_ieee(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);

        let png = generate_fallback_screen_png_bytes(64, 36);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.windows(4).any(|w| w == b"IHDR"));
        assert!(png.windows(4).any(|w| w == b"IDAT"));
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn test_local_executor_ops_and_raii_temp_cleanup() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file_path = temp_dir.path().join("nested/hello.txt");
        let file_path_str = file_path.to_string_lossy().to_string();
        let dir_path_str = temp_dir.path().to_string_lossy().to_string();

        rt.block_on(async {
            let write_res = execute_local_computer_op(
                "local:1:dev-a",
                "cmd-1",
                &RemoteComputerOp::WriteFile {
                    path: file_path_str.clone(),
                    content: "hello computer".to_string(),
                    create_dirs: true,
                },
            )
            .await;
            assert!(write_res.ok);
            assert_eq!(write_res.exit_code, 0);

            let read_res = execute_local_computer_op(
                "local:1:dev-a",
                "cmd-2",
                &RemoteComputerOp::ReadFile {
                    path: file_path_str.clone(),
                    max_bytes: Some(5),
                },
            )
            .await;
            assert!(read_res.ok);
            assert!(read_res.stdout.starts_with("hello"));
            assert!(read_res.stdout.contains("[truncated]"));

            let list_res = execute_local_computer_op(
                "local:1:dev-a",
                "cmd-3",
                &RemoteComputerOp::ListDir {
                    path: dir_path_str.clone(),
                },
            )
            .await;
            assert!(list_res.ok);
            assert!(list_res.entries.iter().any(|e| e == "[dir] nested"));

            let shell_res = execute_local_computer_op(
                "local:1:dev-a",
                "cmd-4",
                &RemoteComputerOp::ShellExec {
                    command: "printf 'ok-shell'".to_string(),
                    cwd: Some(dir_path_str.clone()),
                    timeout_ms: Some(5_000),
                },
            )
            .await;
            assert!(shell_res.ok);
            assert_eq!(shell_res.stdout, "ok-shell");

            let test_res = execute_local_computer_op(
                "local:1:dev-a",
                "cmd-5",
                &RemoteComputerOp::TestConnection,
            )
            .await;
            assert!(test_res.ok);
            assert!(test_res.stdout.contains("Local device reachable:"));

            let gui_res = execute_local_computer_op(
                "local:1:dev-a",
                "cmd-6",
                &RemoteComputerOp::GuiInput {
                    action: "click".to_string(),
                    x: Some(12),
                    y: Some(34),
                    text: None,
                    key: None,
                },
            )
            .await;
            assert!(gui_res.ok);
            assert!(gui_res.stdout.contains("gui_input acknowledged:"));

            let before_captures: Vec<_> = std::fs::read_dir(std::env::temp_dir())
                .unwrap()
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(&format!("gpui-telegram-capture-{}-", std::process::id()))
                })
                .collect();
            assert!(before_captures.is_empty());

            // Force fallback path in test to avoid macOS TCC permission dialogs in CI
            let cap_res = execute_local_computer_op_inner(
                "local:1:dev-a",
                "cmd-7",
                &RemoteComputerOp::CaptureScreen,
                true,
            )
            .await;

            assert!(cap_res.ok);
            let data_url = cap_res.frame_data_url.expect("frame_data_url present");
            let b64 = data_url
                .strip_prefix("data:image/png;base64,")
                .expect("png data url prefix");
            let decoded = decode_base64(b64);
            assert!(decoded.starts_with(b"\x89PNG\r\n\x1a\n"));

            let after_captures: Vec<_> = std::fs::read_dir(std::env::temp_dir())
                .unwrap()
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(&format!("gpui-telegram-capture-{}-", std::process::id()))
                })
                .collect();
            assert!(after_captures.is_empty(), "temp capture PNG must be cleaned up");
        });
    }

    #[test]
    fn test_device_id_header_url_encoding_and_client_wrappers() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let captured = Arc::new(Mutex::new(Vec::<(String, String, Option<String>, String)>::new()));
        let captured_srv = captured.clone();

        let addr = rt.block_on(async move {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    let mut header_end = None;
                    while header_end.is_none() {
                        let n = stream.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                        }
                    }
                    let hend = header_end.unwrap_or(buf.len());
                    let headers_str = String::from_utf8_lossy(&buf[..hend]).to_string();
                    let content_len = headers_str
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    while buf.len() < hend + content_len {
                        let n = stream.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let first_line = headers_str.lines().next().unwrap_or("");
                    let mut parts = first_line.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let path = parts.next().unwrap_or("").to_string();
                    let device_id_hdr = headers_str.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        if k.trim().eq_ignore_ascii_case("x-device-id") {
                            Some(v.trim().to_string())
                        } else {
                            None
                        }
                    });
                    let req_body = String::from_utf8_lossy(&buf[hend..]).to_string();
                    captured_srv.lock().push((
                        method.clone(),
                        path.clone(),
                        device_id_hdr,
                        req_body,
                    ));

                    let resp_body = match (method.as_str(), path.as_str()) {
                        ("POST", "/api/v1/computers/local/register") => serde_json::to_vec(
                            &ComputerNodeDto {
                                node_id: "local:1:dev-custom".to_string(),
                                owner_user_id: 1,
                                kind: ComputerNodeKind::LocalDevice,
                                display_name: "TestHost".to_string(),
                                ip_address: "127.0.0.1".to_string(),
                                os_name: "macOS".to_string(),
                                resolution: "1440x900".to_string(),
                                online: true,
                                is_current_device: true,
                                device_id: Some("dev-custom".to_string()),
                                tier_badge: "本地".to_string(),
                                run_policy: ComputerRunPolicy::AskEveryTime,
                                ..Default::default()
                            },
                        )
                        .unwrap(),
                        ("GET", "/api/v1/computers/local/poll") => serde_json::to_vec(&vec![
                            PendingComputerCommandDto {
                                command_id: "cmd-unapproved".to_string(),
                                node_id: "local:1:dev-custom".to_string(),
                                target_device_id: "dev-custom".to_string(),
                                caller_label: "Bot".to_string(),
                                requires_user_approval: true,
                                approved: false,
                                operation: RemoteComputerOp::TestConnection,
                                created_at_ms: 1,
                                ..Default::default()
                            },
                            PendingComputerCommandDto {
                                command_id: "cmd-ready".to_string(),
                                node_id: "local:1:dev-custom".to_string(),
                                target_device_id: "dev-custom".to_string(),
                                caller_label: "Bot".to_string(),
                                requires_user_approval: true,
                                approved: true,
                                operation: RemoteComputerOp::TestConnection,
                                created_at_ms: 2,
                                ..Default::default()
                            },
                        ])
                        .unwrap(),
                        ("POST", "/api/v1/computers/commands/cmd-ready/complete") => {
                            br#"{"ok":true}"#.to_vec()
                        }
                        ("PATCH", "/api/v1/computers/local%3A1%3Adev-custom/policy") => {
                            serde_json::to_vec(&ComputerNodeDto {
                                node_id: "local:1:dev-custom".to_string(),
                                run_policy: ComputerRunPolicy::AllowReadOnly,
                                ..Default::default()
                            })
                            .unwrap()
                        }
                        ("DELETE", "/api/v1/computers/local%3A1%3Adev-custom") => {
                            br#"{"ok":true}"#.to_vec()
                        }
                        _ => br#"{"ok":true}"#.to_vec(),
                    };

                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        resp_body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(&resp_body).await;
                }
            });
            addr
        });

        let mut cfg = ClientConfig::default();
        cfg.server_url = format!("http://{}", addr);
        let client = TelegramClient::new(cfg);
        let initial_id = client.device_id();
        assert!(initial_id.starts_with("dev-"));

        client.set_token(Some("tok-xyz".to_string()));
        client.set_device_id("dev-custom");
        assert_eq!(client.device_id(), "dev-custom");

        let registered = client
            .register_current_host_computer()
            .expect("register current host");
        assert_eq!(registered.node_id, "local:1:dev-custom");

        let processed = client
            .process_ready_local_computer_commands()
            .expect("process ready commands");
        assert_eq!(processed.len(), 1);
        assert_eq!(processed[0].command_id, "cmd-ready");

        let updated = client
            .update_computer_policy(
                "local:1:dev-custom",
                &UpdateComputerPolicyRequest {
                    run_policy: Some(ComputerRunPolicy::AllowReadOnly),
                    ..Default::default()
                },
            )
            .expect("update policy");
        assert_eq!(updated.run_policy, ComputerRunPolicy::AllowReadOnly);

        client
            .delete_computer("local:1:dev-custom")
            .expect("delete computer");

        let reqs = captured.lock();
        assert_eq!(reqs.len(), 5);
        for (method, path, did, _) in reqs.iter() {
            assert_eq!(
                did.as_deref(),
                Some("dev-custom"),
                "X-Device-Id missing on {} {}",
                method,
                path
            );
        }
        assert_eq!(
            reqs[2].1,
            "/api/v1/computers/commands/cmd-ready/complete"
        );
        assert!(reqs[2].3.contains("Local device reachable:"));
        assert_eq!(
            reqs[3].1,
            "/api/v1/computers/local%3A1%3Adev-custom/policy"
        );
        assert_eq!(reqs[4].1, "/api/v1/computers/local%3A1%3Adev-custom");
    }

    #[test]
    fn test_hostname_sanitization_and_real_lan_ip_filters() {
        assert_eq!(sanitize_hostname_candidate("  "), None);
        assert_eq!(sanitize_hostname_candidate("192.168.1.7"), None);
        assert_eq!(sanitize_hostname_candidate("::1"), None);
        assert_eq!(
            sanitize_hostname_candidate("  Bhrum-MacBook.local \n"),
            Some("Bhrum-MacBook.local".to_string())
        );

        let hostname = detect_local_hostname();
        assert!(!hostname.is_empty());
        assert!(
            hostname.parse::<std::net::IpAddr>().is_err(),
            "hostname must never resolve to a raw IP literal: {hostname}"
        );

        let ips = detect_real_lan_ips();
        for ip in &ips {
            let parsed: std::net::Ipv4Addr = ip.parse().expect("valid ipv4");
            assert!(!is_invalid_or_virtual_lan_ipv4(parsed));
        }

        let (primary_ip, os_name, res) = detect_local_host_info();
        assert!(!primary_ip.is_empty());
        assert!(!primary_ip.starts_with("198.18."));
        assert!(!primary_ip.starts_with("198.19."));
        assert!(!primary_ip.starts_with("169.254."));
        assert!(!os_name.is_empty());
        assert!(!res.is_empty());
    }

    #[test]
    fn test_extended_remote_ops_and_remote_control_http_endpoints() {
        use gpui_telegram_proto::remote_control_contracts::{
            AssistCodeMode, McpOAuthGrantSummaryDto, PowerActionKind, RemoteControlMode,
            RemoteControllerKind, RemoteSessionDto,
        };
        use gpui_telegram_proto::tailnet_contracts::TailnetRelayMode;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let tmp = tempfile::tempdir().expect("tempdir");
        let chunk_file = tmp.path().join("chunked.bin");
        let chunk_path = chunk_file.to_string_lossy().to_string();

        rt.block_on(async {
            // Mouse + Keyboard + Display + Privacy + Clipboard + Accessibility + Power
            let m = execute_local_computer_op(
                "local:1:d",
                "op-m",
                &RemoteComputerOp::MouseAction {
                    action: "move".into(),
                    x: Some(100),
                    y: Some(200),
                    dx: None,
                    dy: None,
                    button: None,
                    clicks: None,
                    scroll_x: None,
                    scroll_y: None,
                    relative: false,
                },
            )
            .await;
            assert!(m.ok);

            let k = execute_local_computer_op(
                "local:1:d",
                "op-k",
                &RemoteComputerOp::KeyboardAction {
                    action: "hotkey".into(),
                    text: None,
                    key: Some("c".into()),
                    modifiers: vec!["ctrl".into()],
                },
            )
            .await;
            assert!(k.ok);

            let cs = execute_local_computer_op(
                "local:1:d",
                "op-cs",
                &RemoteComputerOp::ClipboardSet {
                    text: Some("fabushi-clip".into()),
                    image_b64: None,
                    mime_type: Some("text/plain".into()),
                },
            )
            .await;
            assert!(cs.ok);
            let cg = execute_local_computer_op("local:1:d", "op-cg", &RemoteComputerOp::ClipboardGet)
                .await;
            assert!(cg.ok);
            assert!(cg.stdout.contains("fabushi-clip"));

            let dl = execute_local_computer_op("local:1:d", "op-dl", &RemoteComputerOp::DisplayList)
                .await;
            assert!(dl.ok);
            assert!(!dl.entries.is_empty());

            let ds = execute_local_computer_op(
                "local:1:d",
                "op-ds",
                &RemoteComputerOp::DisplaySwitch { display_index: 0 },
            )
            .await;
            assert!(ds.ok);

            let pv = execute_local_computer_op(
                "local:1:d",
                "op-pv",
                &RemoteComputerOp::PrivacyScreenToggle {
                    enabled: true,
                    message: Some("Locked".into()),
                },
            )
            .await;
            assert!(pv.ok);

            let ax = execute_local_computer_op(
                "local:1:d",
                "op-ax",
                &RemoteComputerOp::AccessibilityTree { max_depth: Some(2) },
            )
            .await;
            assert!(ax.ok);
            assert!(ax.stdout.contains("AXApplication"));

            let pw = execute_local_computer_op(
                "local:1:d",
                "op-pw",
                &RemoteComputerOp::PowerAction {
                    action: PowerActionKind::Lock,
                },
            )
            .await;
            assert!(pw.ok);

            // TerminalOp lifecycle
            let t_new = execute_local_computer_op(
                "local:1:d",
                "op-tn",
                &RemoteComputerOp::TerminalOp {
                    op: "new".into(),
                    session_name: Some("api-term-test".into()),
                    shell: Some("sh".into()),
                    input: None,
                    command: None,
                    cols: Some(80),
                    rows: Some(24),
                    timeout_ms: None,
                    new_name: None,
                },
            )
            .await;
            assert!(t_new.ok);

            let t_has = execute_local_computer_op(
                "local:1:d",
                "op-th",
                &RemoteComputerOp::TerminalOp {
                    op: "has".into(),
                    session_name: Some("api-term-test".into()),
                    shell: None,
                    input: None,
                    command: None,
                    cols: None,
                    rows: None,
                    timeout_ms: None,
                    new_name: None,
                },
            )
            .await;
            assert!(t_has.ok);
            assert_eq!(t_has.exit_code, 0);

            let t_kill = execute_local_computer_op(
                "local:1:d",
                "op-tk",
                &RemoteComputerOp::TerminalOp {
                    op: "kill".into(),
                    session_name: Some("api-term-test".into()),
                    shell: None,
                    input: None,
                    command: None,
                    cols: None,
                    rows: None,
                    timeout_ms: None,
                    new_name: None,
                },
            )
            .await;
            assert!(t_kill.ok);

            // FileChunkOp write + read + SHA-256 mismatch rejection
            let raw_payload = b"hello-fabushi-chunk";
            let good_sha = hex_sha256(raw_payload);
            let b64_payload = base64::engine::general_purpose::STANDARD.encode(raw_payload);

            let bad_write = execute_local_computer_op(
                "local:1:d",
                "op-fw-bad",
                &RemoteComputerOp::FileChunkOp {
                    op: "write_chunk".into(),
                    transfer_id: "tx-1".into(),
                    path: chunk_path.clone(),
                    offset: 0,
                    data_b64: Some(b64_payload.clone()),
                    sha256: Some("0000".into()),
                    total_size: Some(raw_payload.len() as u64),
                },
            )
            .await;
            assert!(!bad_write.ok);
            assert!(bad_write.stderr.contains("SHA-256 mismatch"));

            let good_write = execute_local_computer_op(
                "local:1:d",
                "op-fw-ok",
                &RemoteComputerOp::FileChunkOp {
                    op: "write_chunk".into(),
                    transfer_id: "tx-1".into(),
                    path: chunk_path.clone(),
                    offset: 0,
                    data_b64: Some(b64_payload),
                    sha256: Some(good_sha.clone()),
                    total_size: Some(raw_payload.len() as u64),
                },
            )
            .await;
            assert!(good_write.ok);

            let read_back = execute_local_computer_op(
                "local:1:d",
                "op-fr",
                &RemoteComputerOp::FileChunkOp {
                    op: "read_chunk".into(),
                    transfer_id: "tx-1".into(),
                    path: chunk_path,
                    offset: 0,
                    data_b64: None,
                    sha256: None,
                    total_size: None,
                },
            )
            .await;
            assert!(read_back.ok);
            assert!(read_back.stdout.contains(&good_sha));
        });

        // Verify new HttpClient endpoints (/api/v1/remote/assist/*, /api/v1/remote/sessions/*, /api/v1/mcp/grants*)
        let seen_paths = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let seen_srv = Arc::clone(&seen_paths);
        let addr = rt.block_on(async move {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let headers_str = String::from_utf8_lossy(&buf[..n]);
                    let first = headers_str.lines().next().unwrap_or("");
                    let mut parts = first.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let path = parts.next().unwrap_or("").to_string();
                    seen_srv.lock().push((method.clone(), path.clone()));

                    let body = match (method.as_str(), path.as_str()) {
                        ("GET", "/api/v1/remote/assist/info")
                        | ("POST", "/api/v1/remote/assist/reset-code")
                        | ("POST", "/api/v1/remote/assist/set-code")
                        | ("POST", "/api/v1/remote/assist/allow") => serde_json::to_vec(
                            &RemoteAssistConfigDto {
                                assist_id: "482910375".into(),
                                allow_assist: true,
                                code_mode: AssistCodeMode::Custom,
                                current_code: Some("Abc12345".into()),
                                expires_at_ms: None,
                            },
                        )
                        .unwrap(),
                        ("POST", "/api/v1/remote/assist/start") => serde_json::to_vec(
                            &AssistStartResponse {
                                session_id: "sess-1".into(),
                                target_node_id: "local:2:dev".into(),
                                target_device_id: "dev".into(),
                                temp_relay_token: "rtok".into(),
                                requires_host_confirm: false,
                                expires_at_ms: None,
                            },
                        )
                        .unwrap(),
                        ("GET", "/api/v1/remote/sessions") => serde_json::to_vec(
                            &RemoteSessionsResponse {
                                sessions: vec![RemoteSessionDto {
                                    session_id: "sess-1".into(),
                                    controller_kind: RemoteControllerKind::AiMcp,
                                    controller_name: "Antigravity".into(),
                                    controller_platform: "macos".into(),
                                    controller_id: "cli-1".into(),
                                    target_node_id: "local:1:d".into(),
                                    target_device_id: "d".into(),
                                    mode: RemoteControlMode::AiControl,
                                    started_at_ms: 1000,
                                    last_activity_ms: 2000,
                                    active_tool: Some("capture_screen".into()),
                                    relay_mode: TailnetRelayMode::DirectUdp,
                                    rtt_ms: Some(9),
                                    fps: Some(60),
                                }],
                            },
                        )
                        .unwrap(),
                        ("GET", "/api/v1/mcp/grants") => serde_json::to_vec(
                            &McpOAuthGrantsResponse {
                                grants: vec![McpOAuthGrantSummaryDto {
                                    family_id: "fam-1".into(),
                                    client_id: "cid-1".into(),
                                    client_name: "Antigravity".into(),
                                    scopes: vec!["devices:read".into()],
                                    allowed_node_ids: vec![],
                                    created_at_ms: 1000,
                                    last_used_ms: Some(2000),
                                    expires_at_ms: 9000,
                                }],
                            },
                        )
                        .unwrap(),
                        _ => br#"{"ok":true}"#.to_vec(),
                    };

                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                }
            });
            addr
        });

        let mut cfg = ClientConfig::default();
        cfg.server_url = format!("http://{addr}");
        let client = TelegramClient::new(cfg);
        client.set_token(Some("tok".into()));

        let assist = client.get_remote_assist_info().unwrap();
        assert_eq!(assist.assist_id, "482910375");
        let _ = client.reset_remote_assist_code().unwrap();
        let _ = client.set_remote_assist_code("Abc12345").unwrap();
        let _ = client.set_remote_assist_allow(true).unwrap();
        let start = client
            .start_remote_assist(&AssistStartRequest {
                assist_id: "482910375".into(),
                access_code: "Abc12345".into(),
                mode: RemoteControlMode::Desktop,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(start.session_id, "sess-1");

        let sessions = client.list_remote_sessions().unwrap();
        assert_eq!(sessions.sessions.len(), 1);
        client.disconnect_remote_session("sess-1").unwrap();
        let _ = client.disconnect_all_remote_sessions().unwrap();

        let grants = client.list_mcp_grants().unwrap();
        assert_eq!(grants.grants.len(), 1);
        client.revoke_mcp_grant("fam-1").unwrap();

        let recorded = seen_paths.lock();
        assert_eq!(recorded.len(), 10);
        assert_eq!(recorded[9], ("DELETE".into(), "/api/v1/mcp/grants/fam-1".into()));
    }
}
