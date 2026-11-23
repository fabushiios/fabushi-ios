//! Strict JSON-RPC envelope validation and pure-Rust domain & boundary oracles
//! grounded in `bhrumom/gpui-telegram@6141456e2a11` (`Fabushi v1.6.7`) and `PROJECT.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};
use std::collections::HashSet;

pub const DEFAULT_SERVER_URL_V167: &str = "http://34.46.77.133:8080";
pub const MAX_VAULT_CIPHERTEXT_BYTES: usize = 24 * 1024 * 1024; // 25_165_824
pub const MAX_VAULT_NONCE_BYTES: usize = 128;
pub const MAX_VAULT_ITEM_KEY_LEN: usize = 128;
pub const DEFAULT_MAX_LOOP_HOPS: u32 = 5;
pub const TELEGRAM_WAVEFORM_BARS: usize = 100;
pub const TELEGRAM_WAVEFORM_BYTES: usize = 63;
pub const WAVEFORM_MAX_VALUE: u8 = 31;
pub const BUBBLE_GROUPING_DELTA_SECS: i64 = 900;
pub const IME_COMMIT_FENCE_WINDOW_MS: u64 = 80;
pub const MAX_AUDIO_BUFFER_CHUNKS: usize = 320;
pub const MCP_AUDIT_MAX_CHARS: usize = 280;

pub const CANONICAL_29_TOOLSETS: [&str; 29] = [
    "core",
    "web",
    "terminal",
    "file",
    "memory",
    "skills",
    "vision",
    "image_gen",
    "video_gen",
    "browser",
    "audio_tts",
    "audio_stt",
    "cronjob",
    "subagents",
    "code_execution",
    "clarify",
    "todo",
    "planning",
    "reasoning",
    "mcp",
    "kanban",
    "desktop_ui",
    "computer_use",
    "connections",
    "honcho",
    "session_search",
    "workspace",
    "voice",
    "platform_im",
];

pub const CANONICAL_26_SETTINGS_SECTIONS: [&str; 26] = [
    "main",
    "information",
    "notifications",
    "notifications_type",
    "notifications_reactions",
    "privacy",
    "blocked_peers",
    "active_sessions",
    "chat",
    "folders",
    "advanced",
    "calls",
    "credits",
    "premium",
    "shortcuts",
    "websites",
    "passkeys",
    "local_passcode",
    "global_ttl",
    "power_saving",
    "local_storage",
    "business",
    "computers",
    "ai_providers",
    "language",
    "search",
];

pub const REQUIRED_SLASH_CATEGORIES: [&str; 8] = [
    "Session",
    "Safety & Control",
    "Inspection & Export",
    "Model & Provider",
    "Tools & Skills",
    "Memory & Persona",
    "Workflow & Automation",
    "Modes & UI",
];

pub const HONEST_TRACKED_CAPABILITY_IDS: [&str; 7] = [
    "OCRM-001",
    "FBCP-001",
    "TON_ONCHAIN_SIGNING",
    "TELEGRAM_PASSPORT",
    "APNS_HTTP2_SERVER_DELIVERY",
    "WEBRTC_NAT_SFU_TRANSPORT",
    "STOREKIT_SERVER_BINDING",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: String,
    pub method: String,
    pub generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub id: String,
    pub ok: bool,
    pub generation: u64,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEventEnvelope {
    pub seq: u64,
    #[serde(default)]
    pub account_id: Option<String>,
    pub generation: u64,
    pub kind: String,
    pub payload: Value,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPollEnvelope {
    pub ok: bool,
    pub generation: u64,
    #[serde(default)]
    pub events: Vec<DomainEventEnvelope>,
    #[serde(default)]
    pub error: Option<RpcError>,
}

/// Validates strict JSON-RPC envelope compliance per `PROJECT.md § Interface Contracts #1`:
/// - Must be valid JSON object
/// - Must contain string `id` matching `expected_id` (when `expected_id` is non-empty)
/// - Must contain boolean `ok` and u64 `generation`
/// - When `ok == true`: must contain `result` and MUST NOT be a fail-open `{"status": "acknowledged"}` stub
/// - When `ok == false`: must contain `error.{code: String, message: String, retryable: bool}`
pub fn validate_envelope(raw_json: &str, expected_id: &str) -> Result<RpcResponse, String> {
    let val: Value = serde_json::from_str(raw_json)
        .map_err(|err| format!("response is not valid JSON: {err}; raw={raw_json}"))?;

    let obj = val
        .as_object()
        .ok_or_else(|| format!("response must be a JSON object; raw={raw_json}"))?;

    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("envelope missing string `id`; raw={raw_json}"))?;
    if !expected_id.is_empty() && id != expected_id {
        return Err(format!(
            "envelope `id` mismatch: expected `{expected_id}`, got `{id}`"
        ));
    }

    let ok = obj
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("envelope missing boolean `ok`; raw={raw_json}"))?;

    let generation = obj
        .get("generation")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("envelope missing u64 `generation`; raw={raw_json}"))?;

    if ok {
        let result = obj
            .get("result")
            .cloned()
            .ok_or_else(|| format!("success envelope missing `result`; raw={raw_json}"))?;
        assert_not_acknowledged_stub(&result)?;
        Ok(RpcResponse {
            id: id.to_owned(),
            ok: true,
            generation,
            result: Some(result),
            error: None,
        })
    } else {
        let err_obj = obj
            .get("error")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("error envelope missing `error` object; raw={raw_json}"))?;
        let code = err_obj
            .get("code")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("error envelope missing non-empty `error.code`; raw={raw_json}"))?;
        let message = err_obj
            .get("message")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                format!("error envelope missing non-empty `error.message`; raw={raw_json}")
            })?;
        let retryable = err_obj
            .get("retryable")
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("error envelope missing boolean `error.retryable`; raw={raw_json}"))?;

        Ok(RpcResponse {
            id: id.to_owned(),
            ok: false,
            generation,
            result: None,
            error: Some(RpcError {
                code: code.to_owned(),
                message: message.to_owned(),
                retryable,
            }),
        })
    }
}

pub fn validate_success_envelope(
    raw_json: &str,
    expected_id: &str,
) -> Result<(RpcResponse, Value), String> {
    let resp = validate_envelope(raw_json, expected_id)?;
    if !resp.ok {
        return Err(format!(
            "expected ok=true envelope, got error: {:?}",
            resp.error
        ));
    }
    let result = resp
        .result
        .clone()
        .ok_or_else(|| "validated success envelope missing result".to_owned())?;
    assert_not_acknowledged_stub(&result)?;
    Ok((resp, result))
}

pub fn validate_error_envelope(
    raw_json: &str,
    expected_id: &str,
    allowed_codes: &[&str],
) -> Result<(RpcResponse, RpcError), String> {
    let resp = validate_envelope(raw_json, expected_id)?;
    if resp.ok {
        return Err(format!(
            "expected fail-closed ok=false envelope, got ok=true result={:?}",
            resp.result
        ));
    }
    let err = resp
        .error
        .clone()
        .ok_or_else(|| "validated error envelope missing error object".to_owned())?;
    if !allowed_codes.is_empty() && !allowed_codes.iter().any(|c| *c == err.code) {
        return Err(format!(
            "unexpected error.code `{}` (allowed: {:?}); message=`{}`",
            err.code, allowed_codes, err.message
        ));
    }
    Ok((resp, err))
}

pub fn assert_not_acknowledged_stub(result: &Value) -> Result<(), String> {
    if let Some(status) = result.get("status").and_then(Value::as_str) {
        if status.eq_ignore_ascii_case("acknowledged") {
            return Err(format!(
                "STUB VIOLATION: fail-open `status: \"acknowledged\"` detected in result: {result}"
            ));
        }
    }
    Ok(())
}

/// Detects hardcoded synthetic `sample_chats` from baseline `8128951` (`dispatch.rs:223-244`).
pub fn assert_no_sample_chats(payload: &Value) -> Result<(), String> {
    let items: &[Value] = if let Some(arr) = payload.as_array() {
        arr
    } else if let Some(arr) = payload.get("chats").and_then(Value::as_array) {
        arr
    } else {
        return Ok(());
    };

    for item in items {
        let title = item.get("title").and_then(Value::as_str).unwrap_or("");
        let last_msg = item
            .get("last_message")
            .and_then(Value::as_str)
            .unwrap_or("");
        let id = item
            .get("chat_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_i64)
            .unwrap_or(-1);

        if (id == 100 && title == "Saved Messages")
            || (id == 200 && title == "Fabushi Community")
            || title == "Fabushi Community"
            || last_msg.contains("Welcome to Fabushi iOS Telegram Core")
            || last_msg.contains("Native iOS 1:1 Telegram UI is live!")
        {
            return Err(format!(
                "STUB VIOLATION: hardcoded synthetic sample_chats detected: {item}"
            ));
        }
    }
    Ok(())
}

pub fn assert_not_hardcoded_localhost(server_url: &str) -> Result<(), String> {
    let trimmed = server_url.trim().trim_end_matches('/');
    if trimmed.eq_ignore_ascii_case("https://127.0.0.1:8443")
        || trimmed.eq_ignore_ascii_case("http://127.0.0.1:8443")
    {
        return Err(format!(
            "STUB VIOLATION: legacy hardcoded localhost default URL detected: {server_url}"
        ));
    }
    Ok(())
}

/// Pure-Rust FIPS 180-4 SHA-1 implementation (required for RFC 6455 WebSocket handshake
/// `Sec-WebSocket-Accept` and RFC 6238 HMAC-SHA1 TOTP verification).
pub fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut msg = input.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

fn hmac_compute(algo: TotpAlgorithm, key: &[u8], message: &[u8]) -> Vec<u8> {
    let block_size = match algo {
        TotpAlgorithm::Sha1 | TotpAlgorithm::Sha256 => 64,
        TotpAlgorithm::Sha512 => 128,
    };

    let hash_once = |data: &[u8]| -> Vec<u8> {
        match algo {
            TotpAlgorithm::Sha1 => sha1_digest(data).to_vec(),
            TotpAlgorithm::Sha256 => Sha256::digest(data).to_vec(),
            TotpAlgorithm::Sha512 => Sha512::digest(data).to_vec(),
        }
    };

    let mut key_block = vec![0u8; block_size];
    if key.len() > block_size {
        let hashed = hash_once(key);
        key_block[..hashed.len()].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = Vec::with_capacity(block_size + message.len());
    let mut opad = Vec::with_capacity(block_size + 64);
    for &b in &key_block {
        ipad.push(b ^ 0x36);
        opad.push(b ^ 0x5C);
    }
    ipad.extend_from_slice(message);
    let inner_hash = hash_once(&ipad);
    opad.extend_from_slice(&inner_hash);
    hash_once(&opad)
}

pub fn decode_base32_rfc4648(raw: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '=')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.is_empty() {
        return Err("empty base32 secret".to_owned());
    }
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits_left: u32 = 0;
    for ch in cleaned.chars() {
        let val = match ch {
            'A'..='Z' => (ch as u8) - b'A',
            '2'..='7' => (ch as u8) - b'2' + 26,
            _ => return Err(format!("invalid base32 char `{ch}`")),
        };
        buffer = (buffer << 5) | u32::from(val);
        bits_left += 5;
        if bits_left >= 8 {
            bits_left -= 8;
            out.push(((buffer >> bits_left) & 0xFF) as u8);
        }
    }
    Ok(out)
}

/// Pure-Rust RFC 6238 TOTP reference calculator (SHA1 / SHA256 / SHA512).
pub fn compute_rfc6238_totp(
    secret: &[u8],
    unix_secs: u64,
    period_secs: u64,
    digits: u32,
    algo: TotpAlgorithm,
) -> Result<String, String> {
    if secret.is_empty() {
        return Err("TOTP secret must be non-empty".to_owned());
    }
    if period_secs == 0 {
        return Err("TOTP period_secs must be > 0".to_owned());
    }
    if !(1..=9).contains(&digits) {
        return Err(format!("TOTP digits must be in 1..=9, got {digits}"));
    }
    let counter = unix_secs / period_secs;
    let mac = hmac_compute(algo, secret, &counter.to_be_bytes());
    let offset = (mac[mac.len() - 1] & 0x0F) as usize;
    let binary = ((u32::from(mac[offset] & 0x7F)) << 24)
        | ((u32::from(mac[offset + 1])) << 16)
        | ((u32::from(mac[offset + 2])) << 8)
        | u32::from(mac[offset + 3]);
    let modulo = 10u32.pow(digits);
    Ok(format!("{:0width$}", binary % modulo, width = digits as usize))
}

pub fn verify_rfc6238_totp(
    base32_secret: &str,
    unix_secs: u64,
    period_secs: u64,
    digits: u32,
    algo: TotpAlgorithm,
    observed_code: &str,
) -> Result<(), String> {
    let secret = decode_base32_rfc4648(base32_secret)?;
    let expected = compute_rfc6238_totp(&secret, unix_secs, period_secs, digits, algo)?;
    if observed_code != expected {
        return Err(format!(
            "RFC 6238 TOTP mismatch: expected `{expected}`, got `{observed_code}`"
        ));
    }
    Ok(())
}

/// Verifies Split-Key V2 Argon2id parameter bounds (`routes/user_vault.rs:98-180`).
pub fn verify_argon2id_params(
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
    salt_len: usize,
    verifier_len: usize,
) -> Result<(), String> {
    if !(1024..=1_048_576).contains(&m_cost_kib) {
        return Err(format!("invalid m_cost_kib={m_cost_kib} (expected 1024..=1048576)"));
    }
    if !(1..=64).contains(&t_cost) {
        return Err(format!("invalid t_cost={t_cost} (expected 1..=64)"));
    }
    if !(1..=16).contains(&p_cost) {
        return Err(format!("invalid p_cost={p_cost} (expected 1..=16)"));
    }
    if salt_len != 16 {
        return Err(format!("invalid salt_len={salt_len} (expected exact 16 bytes)"));
    }
    if !(32..=128).contains(&verifier_len) {
        return Err(format!("invalid verifier_len={verifier_len} (expected 32..=128 bytes)"));
    }
    Ok(())
}

pub fn verify_otp_code(code: &str) -> Result<(), String> {
    if code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(format!("invalid 6-digit OTP code: `{code}`"))
    }
}

pub fn normalize_phone(raw: &str) -> Result<String, String> {
    let normalized: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    if (7..=20).contains(&normalized.len()) {
        Ok(normalized)
    } else {
        Err(format!("invalid normalized phone length: `{normalized}`"))
    }
}

/// Telegram 5-bit 100-bar waveform encoder (`(100 * 5 + 7) / 8 = 63` bytes).
pub fn encode_waveform_5bit(samples: &[u8]) -> Vec<u8> {
    if samples.is_empty() {
        return Vec::new();
    }
    let count = samples.len().min(TELEGRAM_WAVEFORM_BARS);
    let byte_len = (count * 5).div_ceil(8);
    let mut out = vec![0u8; byte_len];
    for (idx, &raw_sample) in samples.iter().take(count).enumerate() {
        let clamped = raw_sample.min(WAVEFORM_MAX_VALUE);
        let bit_pos = idx * 5;
        let byte_idx = bit_pos / 8;
        let bit_offset = bit_pos % 8;
        out[byte_idx] |= clamped << bit_offset;
        if bit_offset > 3 && byte_idx + 1 < byte_len {
            out[byte_idx + 1] |= clamped >> (8 - bit_offset);
        }
    }
    out
}

pub fn decode_waveform_5bit(bytes: &[u8], sample_count: usize) -> Vec<u8> {
    if bytes.is_empty() || sample_count == 0 {
        return Vec::new();
    }
    let count = sample_count.min(TELEGRAM_WAVEFORM_BARS);
    let mut out = Vec::with_capacity(count);
    for idx in 0..count {
        let bit_pos = idx * 5;
        let byte_idx = bit_pos / 8;
        let bit_offset = bit_pos % 8;
        if byte_idx >= bytes.len() {
            break;
        }
        let mut val = (bytes[byte_idx] >> bit_offset) & 0x1F;
        if bit_offset > 3 && byte_idx + 1 < bytes.len() {
            val |= (bytes[byte_idx + 1] << (8 - bit_offset)) & 0x1F;
        }
        out.push(val & 0x1F);
    }
    out
}

pub fn verify_waveform_5bit_63bytes(samples: &[u8], encoded: &[u8]) -> Result<(), String> {
    if samples.len() == TELEGRAM_WAVEFORM_BARS && encoded.len() != TELEGRAM_WAVEFORM_BYTES {
        return Err(format!(
            "expected exact 63 bytes for 100 waveform bars, got {}",
            encoded.len()
        ));
    }
    let expected = encode_waveform_5bit(samples);
    if encoded != expected.as_slice() {
        return Err("encoded 5-bit waveform does not match canonical packing".to_owned());
    }
    let decoded = decode_waveform_5bit(encoded, samples.len().min(TELEGRAM_WAVEFORM_BARS));
    for (i, (&orig, &dec)) in samples
        .iter()
        .take(TELEGRAM_WAVEFORM_BARS)
        .zip(decoded.iter())
        .enumerate()
    {
        if orig.min(WAVEFORM_MAX_VALUE) != dec {
            return Err(format!("waveform lossless roundtrip mismatch at bar {i}"));
        }
    }
    Ok(())
}

fn parse_hex_rgb(hex: &str) -> Result<(u8, u8, u8), String> {
    let s = hex.strip_prefix('#').unwrap_or(hex);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid #RRGGBB hex color: `{hex}`"));
    }
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|e| e.to_string())?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|e| e.to_string())?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|e| e.to_string())?;
    Ok((r, g, b))
}

/// Exact v1.6.7 WCAG white-text accent contrast guard (`palette_presets.rs:276-287`).
pub fn compute_wcag_accent_fill(hex: &str) -> Result<(String, f64, bool), String> {
    let (r, g, b) = parse_hex_rgb(hex)?;
    let luma = 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b);
    if luma <= 138.0 {
        return Ok((format!("#{r:02X}{g:02X}{b:02X}"), luma, false));
    }
    let darken_t = ((luma - 138.0) / 145.0).clamp(0.0, 0.36);
    let mix = |c: u8| -> u8 {
        let blended = f64::from(c) * (1.0 - darken_t) + f64::from(0x11u8) * darken_t;
        blended.round().clamp(0.0, 255.0) as u8
    };
    Ok((
        format!("#{:02X}{:02X}{:02X}", mix(r), mix(g), mix(b)),
        luma,
        true,
    ))
}

pub fn verify_wcag_accent_luma_guard(
    input_hex: &str,
    observed_effective_hex: &str,
    observed_adjusted: bool,
) -> Result<(), String> {
    let (expected_hex, _luma, expected_adjusted) = compute_wcag_accent_fill(input_hex)?;
    if !observed_effective_hex.eq_ignore_ascii_case(&expected_hex)
        || observed_adjusted != expected_adjusted
    {
        return Err(format!(
            "WCAG accent guard mismatch for {input_hex}: expected ({expected_hex}, adjusted={expected_adjusted}), got ({observed_effective_hex}, adjusted={observed_adjusted})"
        ));
    }
    Ok(())
}

pub fn verify_vault_size_cap(
    category: &str,
    item_key: &str,
    ciphertext_bytes: usize,
    nonce_bytes: usize,
) -> Result<(), String> {
    if !matches!(category, "password" | "browser_cookie" | "ai_oauth") {
        return Err(format!("invalid_category: `{category}`"));
    }
    if item_key.is_empty()
        || item_key.len() > MAX_VAULT_ITEM_KEY_LEN
        || item_key.contains("..")
        || !item_key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!("invalid_item_key: `{item_key}`"));
    }
    if ciphertext_bytes > MAX_VAULT_CIPHERTEXT_BYTES {
        return Err(format!(
            "invalid_payload_size: ciphertext {ciphertext_bytes} > {MAX_VAULT_CIPHERTEXT_BYTES}"
        ));
    }
    if nonce_bytes > MAX_VAULT_NONCE_BYTES {
        return Err(format!(
            "invalid_payload_size: nonce {nonce_bytes} > {MAX_VAULT_NONCE_BYTES}"
        ));
    }
    Ok(())
}

pub fn verify_pts_monotonic_and_snapshot_fence(
    req_pts: u64,
    floor_pts: u64,
    update_pts: &[u64],
    snapshot_required: bool,
) -> Result<(), String> {
    let expected_snapshot = req_pts.saturating_add(1) < floor_pts;
    if snapshot_required != expected_snapshot {
        return Err(format!(
            "snapshot_required mismatch: req_pts={req_pts}, floor_pts={floor_pts}, expected={expected_snapshot}, got={snapshot_required}"
        ));
    }
    if !snapshot_required {
        let mut prev = req_pts;
        for &pts in update_pts {
            if pts != prev + 1 {
                return Err(format!(
                    "non-contiguous PTS update: prev={prev}, next={pts}"
                ));
            }
            prev = pts;
        }
    }
    Ok(())
}

pub fn verify_outbox_backoff_bounds(retry_count: u32, observed_ms: u64) -> Result<(), String> {
    let shift = retry_count.min(6);
    let base = (500u64 << shift).min(30_000);
    let min_ms = ((base as f64) * 0.75).floor().max(250.0) as u64;
    let max_ms = ((base as f64) * 1.25).ceil().min(30_000.0) as u64;
    if observed_ms < min_ms || observed_ms > max_ms {
        return Err(format!(
            "outbox backoff out of bounds for retry {retry_count}: got {observed_ms}ms, expected [{min_ms}..={max_ms}]"
        ));
    }
    Ok(())
}

pub fn verify_loop_budget_5_hops(
    current_hop: u32,
    max_hops: u32,
    exhausted: bool,
) -> Result<(), String> {
    if max_hops != DEFAULT_MAX_LOOP_HOPS {
        return Err(format!("expected default max_hops=5, got {max_hops}"));
    }
    let should_exhaust = current_hop > max_hops;
    if exhausted != should_exhaust {
        return Err(format!(
            "loop budget exhaustion mismatch: hop={current_hop}/{max_hops}, exhausted={exhausted}"
        ));
    }
    Ok(())
}

pub fn verify_mcp_audit_redaction(raw_args: &Value, redacted_summary: &str) -> Result<(), String> {
    if redacted_summary.chars().count() > MCP_AUDIT_MAX_CHARS {
        return Err(format!(
            "MCP audit summary exceeded 280 chars: len={}",
            redacted_summary.chars().count()
        ));
    }
    if let Some(obj) = raw_args.as_object() {
        for sensitive_key in ["code", "text", "content", "data", "password"] {
            if let Some(secret_val) = obj.get(sensitive_key).and_then(Value::as_str) {
                if !secret_val.is_empty() && redacted_summary.contains(secret_val) {
                    return Err(format!(
                        "MCP audit redaction leaked raw value for key `{sensitive_key}`"
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn verify_config_4tier_cascade(
    cli_arg: Option<&str>,
    env_url: Option<&str>,
    config_file_url: Option<&str>,
) -> (String, &'static str) {
    let clean = |s: &str| s.trim().trim_end_matches('/').to_owned();
    if let Some(v) = cli_arg.filter(|s| !s.trim().is_empty()) {
        return (clean(v), "cli_argument");
    }
    if let Some(v) = env_url.filter(|s| !s.trim().is_empty()) {
        return (clean(v), "environment_variable");
    }
    if let Some(v) = config_file_url.filter(|s| !s.trim().is_empty()) {
        return (clean(v), "config_file");
    }
    (DEFAULT_SERVER_URL_V167.to_owned(), "default_fallback")
}

pub fn verify_honest_capability_disposition(items: &[Value]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for item in items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("capability ledger item missing `id`: {item}"))?;
        let disp = item
            .get("disposition")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("capability ledger item `{id}` missing `disposition`"))?;
        if !matches!(
            disp,
            "upstream_planned" | "server_blocked" | "desktop_only_sandbox"
        ) {
            return Err(format!(
                "capability `{id}` has non-honest disposition `{disp}`"
            ));
        }
        seen.insert(id);
    }
    for required in HONEST_TRACKED_CAPABILITY_IDS {
        if !seen.contains(required) {
            return Err(format!(
                "capability ledger missing required honest entry `{required}`"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct TranscriptBubbleInput {
    pub sender_id: u64,
    pub outgoing: bool,
    pub timestamp_secs: i64,
    pub is_service: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BubbleGroupLayout {
    pub attached_prev: bool,
    pub attached_next: bool,
    pub inner_radius: f32,
    pub has_tail: bool,
}

pub fn verify_bubble_grouping_900s(messages: &[TranscriptBubbleInput]) -> Vec<BubbleGroupLayout> {
    let can_attach = |a: &TranscriptBubbleInput, b: &TranscriptBubbleInput| -> bool {
        !a.is_service
            && !b.is_service
            && a.sender_id == b.sender_id
            && a.outgoing == b.outgoing
            && (b.timestamp_secs - a.timestamp_secs).abs() <= BUBBLE_GROUPING_DELTA_SECS
    };

    let mut out = Vec::with_capacity(messages.len());
    for i in 0..messages.len() {
        let attached_prev = i > 0 && can_attach(&messages[i - 1], &messages[i]);
        let attached_next = i + 1 < messages.len() && can_attach(&messages[i], &messages[i + 1]);
        let inner_radius = if attached_prev || attached_next {
            4.0
        } else {
            16.0
        };
        out.push(BubbleGroupLayout {
            attached_prev,
            attached_next,
            inner_radius,
            has_tail: !attached_next && !messages[i].is_service,
        });
    }
    out
}

pub fn verify_utf16_entity_bounds(text: &str, offset: usize, length: usize) -> Result<(), String> {
    let utf16_len = text.encode_utf16().count();
    let end = offset
        .checked_add(length)
        .ok_or_else(|| "UTF-16 entity offset+length overflow".to_owned())?;
    if end > utf16_len {
        return Err(format!(
            "UTF-16 entity out of bounds: offset={offset}, length={length}, utf16_len={utf16_len}"
        ));
    }
    Ok(())
}

pub fn verify_slash_command_catalog(commands: &[Value]) -> Result<(), String> {
    if commands.len() < 49 {
        return Err(format!(
            "expected >= 49 slash commands, got {}",
            commands.len()
        ));
    }
    let categories: HashSet<&str> = commands
        .iter()
        .filter_map(|c| c.get("category").and_then(Value::as_str))
        .collect();
    for req in REQUIRED_SLASH_CATEGORIES {
        if !categories.contains(req) {
            return Err(format!("slash command catalog missing category `{req}`"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBadge {
    pub kind: &'static str,
    pub token: String,
    pub color_hex: &'static str,
}

pub fn extract_context_reference_badges(text: &str) -> Vec<ContextBadge> {
    let trim_chars: &[char] = &[',', ';', ')', ']', '"', '\'', '('];
    let mut badges = Vec::new();
    for raw_word in text.split_whitespace() {
        let token = raw_word.trim_matches(trim_chars);
        if !token.starts_with('@') {
            continue;
        }
        if token == "@file" || token.starts_with("@file:") {
            badges.push(ContextBadge {
                kind: "FILE",
                token: token.to_owned(),
                color_hex: "#2AABEE",
            });
        } else if token == "@dir" || token.starts_with("@dir:") {
            badges.push(ContextBadge {
                kind: "DIR",
                token: token.to_owned(),
                color_hex: "#27AE60",
            });
        } else if token == "@diff" || token == "@staged" || token.starts_with("@git:") {
            badges.push(ContextBadge {
                kind: "GIT",
                token: token.to_owned(),
                color_hex: "#E67E22",
            });
        } else if token == "@url" || token.starts_with("@url:") {
            badges.push(ContextBadge {
                kind: "URL",
                token: token.to_owned(),
                color_hex: "#9B51E0",
            });
        }
    }
    badges
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeState {
    Idle,
    Composing,
    Committed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeEvent {
    Start,
    Commit,
    KeyEnter,
    KeyShiftEnter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeDecision {
    pub next_state: ImeState,
    pub action: &'static str,
}

pub fn verify_ime_commit_fence(
    state: ImeState,
    event: ImeEvent,
    elapsed_since_commit_ms: u64,
) -> ImeDecision {
    match (state, event) {
        (_, ImeEvent::Start) => ImeDecision {
            next_state: ImeState::Composing,
            action: "default_edit",
        },
        (_, ImeEvent::Commit) => ImeDecision {
            next_state: ImeState::Committed,
            action: "default_edit",
        },
        (_, ImeEvent::KeyShiftEnter) => ImeDecision {
            next_state: ImeState::Idle,
            action: "insert_newline",
        },
        (ImeState::Composing, ImeEvent::KeyEnter) => ImeDecision {
            next_state: ImeState::Composing,
            action: "ime_candidate_chosen",
        },
        (ImeState::Committed, ImeEvent::KeyEnter) => {
            if elapsed_since_commit_ms < IME_COMMIT_FENCE_WINDOW_MS {
                ImeDecision {
                    next_state: ImeState::Idle,
                    action: "none",
                }
            } else {
                ImeDecision {
                    next_state: ImeState::Idle,
                    action: "send_message",
                }
            }
        }
        (ImeState::Idle, ImeEvent::KeyEnter) => ImeDecision {
            next_state: ImeState::Idle,
            action: "send_message",
        },
    }
}

pub fn verify_voice_gesture(dx: f32, dy: f32, elapsed_secs: f32) -> &'static str {
    if dx < -80.0 {
        "slide_canceling"
    } else if dy < -50.0 {
        "locked"
    } else if elapsed_secs < 0.5 {
        "discarded"
    } else {
        "completed"
    }
}

pub fn verify_sliding_replay_window_128(highest: &mut u64, bitmap: &mut u128, seq: u64) -> bool {
    if seq == 0 {
        return false;
    }
    if *bitmap == 0 && *highest == 0 {
        *highest = seq;
        *bitmap = 1;
        return true;
    }
    if seq > *highest {
        let shift = seq - *highest;
        if shift >= 128 {
            *bitmap = 1;
        } else {
            *bitmap = (*bitmap << shift) | 1;
        }
        *highest = seq;
        true
    } else {
        let diff = *highest - seq;
        if diff >= 128 {
            return false;
        }
        let mask = 1u128 << diff;
        if (*bitmap & mask) != 0 {
            return false;
        }
        *bitmap |= mask;
        true
    }
}

pub fn verify_controlled_banner_hex(variant: &str, hex: &str) -> Result<(), String> {
    let expected = match variant {
        "human" => "#1E6BFF",
        "ai_mcp" => "#0F766E",
        "mixed" => "#4338CA",
        "none" => "",
        other => return Err(format!("unknown banner variant `{other}`")),
    };
    if !hex.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "controlled banner hex mismatch for `{variant}`: expected `{expected}`, got `{hex}`"
        ));
    }
    Ok(())
}

pub fn verify_29_toolsets(toolset_ids: &[String]) -> Result<(), String> {
    let actual: HashSet<&str> = toolset_ids.iter().map(String::as_str).collect();
    if actual.len() != CANONICAL_29_TOOLSETS.len() {
        return Err(format!(
            "expected exactly 29 toolsets, got {}",
            actual.len()
        ));
    }
    for req in CANONICAL_29_TOOLSETS {
        if !actual.contains(req) {
            return Err(format!("missing canonical toolset `{req}`"));
        }
    }
    Ok(())
}

pub fn verify_26_settings_sections(section_ids: &[String]) -> Result<(), String> {
    let actual: HashSet<&str> = section_ids.iter().map(String::as_str).collect();
    if actual.len() != CANONICAL_26_SETTINGS_SECTIONS.len() {
        return Err(format!(
            "expected exactly 26 settings sections, got {}",
            actual.len()
        ));
    }
    for req in CANONICAL_26_SETTINGS_SECTIONS {
        if !actual.contains(req) {
            return Err(format!("missing canonical settings section `{req}`"));
        }
    }
    Ok(())
}

pub fn verify_magpie_circuit_rest_ms(
    kind: &str,
    attempt: u32,
    observed_ms: u64,
) -> Result<(), String> {
    let expected = match kind {
        "rate_limit" => (5_000u64.saturating_mul(1u64 << attempt.min(9))).min(1_800_000),
        "overload" => 30_000,
        "credit" => {
            const LADDER: [u64; 4] = [1_800_000, 3_600_000, 7_200_000, 14_400_000];
            LADDER[(attempt as usize).min(LADDER.len() - 1)]
        }
        other => return Err(format!("unknown Magpie circuit rest kind `{other}`")),
    };
    if observed_ms != expected {
        return Err(format!(
            "Magpie circuit rest mismatch ({kind}, attempt={attempt}): expected {expected}ms, got {observed_ms}ms"
        ));
    }
    Ok(())
}

pub fn verify_talk_vad_bargein(
    consecutive_loud_frames: u32,
    consecutive_silent_frames: u32,
    buffered_chunks: usize,
) -> Result<(bool, bool), String> {
    if buffered_chunks > MAX_AUDIO_BUFFER_CHUNKS {
        return Err(format!(
            "Talk Mode audio buffer exceeded cap {MAX_AUDIO_BUFFER_CHUNKS}: got {buffered_chunks}"
        ));
    }
    let barge_in = consecutive_loud_frames >= 3;
    let end_of_speech = consecutive_silent_frames >= 25;
    Ok((barge_in, end_of_speech))
}
