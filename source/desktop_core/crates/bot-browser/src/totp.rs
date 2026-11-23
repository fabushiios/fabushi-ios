use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use url::Url;
use zeroize::Zeroizing;

use crate::types::BotBrowserError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TotpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

struct ParsedTotpInput {
    secret_b32: Zeroizing<String>,
    algorithm: TotpAlgorithm,
    digits: Option<u32>,
    period_secs: Option<u64>,
}

fn parse_totp_input(raw: &str) -> Result<ParsedTotpInput, BotBrowserError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BotBrowserError::Totp("TOTP secret cannot be empty".into()));
    }

    if trimmed
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("otpauth://"))
    {
        let parsed = Url::parse(trimmed)
            .map_err(|e| BotBrowserError::Totp(format!("invalid otpauth URI: {e}")))?;

        let otp_kind = parsed.host_str().unwrap_or("");
        if !otp_kind.eq_ignore_ascii_case("totp") {
            return Err(BotBrowserError::Totp(format!(
                "unsupported otpauth type `{otp_kind}` (expected `totp`)"
            )));
        }

        let mut secret: Option<Zeroizing<String>> = None;
        let mut algorithm = TotpAlgorithm::Sha1;
        let mut digits: Option<u32> = None;
        let mut period_secs: Option<u64> = None;

        for (key, value) in parsed.query_pairs() {
            if key.eq_ignore_ascii_case("secret") {
                secret = Some(Zeroizing::new(value.into_owned()));
            } else if key.eq_ignore_ascii_case("algorithm") {
                if value.eq_ignore_ascii_case("SHA1") {
                    algorithm = TotpAlgorithm::Sha1;
                } else if value.eq_ignore_ascii_case("SHA256") {
                    algorithm = TotpAlgorithm::Sha256;
                } else if value.eq_ignore_ascii_case("SHA512") {
                    algorithm = TotpAlgorithm::Sha512;
                } else {
                    return Err(BotBrowserError::Totp(format!(
                        "unsupported otpauth TOTP algorithm `{value}`"
                    )));
                }
            } else if key.eq_ignore_ascii_case("digits") {
                let d: u32 = value.parse().map_err(|_| {
                    BotBrowserError::Totp(format!("invalid otpauth digits `{value}`"))
                })?;
                digits = Some(d);
            } else if key.eq_ignore_ascii_case("period") {
                let p: u64 = value.parse().map_err(|_| {
                    BotBrowserError::Totp(format!("invalid otpauth period `{value}`"))
                })?;
                period_secs = Some(p);
            }
        }

        let secret_b32 = secret
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| BotBrowserError::Totp("otpauth URI missing `secret` parameter".into()))?;

        return Ok(ParsedTotpInput {
            secret_b32,
            algorithm,
            digits,
            period_secs,
        });
    }

    Ok(ParsedTotpInput {
        secret_b32: Zeroizing::new(trimmed.to_string()),
        algorithm: TotpAlgorithm::Sha1,
        digits: None,
        period_secs: None,
    })
}

fn decode_base32_secret(raw_b32: &str) -> Result<Zeroizing<Vec<u8>>, BotBrowserError> {
    let mut normalized = Zeroizing::new(String::with_capacity(raw_b32.len()));
    for ch in raw_b32.chars() {
        if ch.is_whitespace() || ch == '-' {
            continue;
        }
        normalized.push(ch.to_ascii_uppercase());
    }

    let stripped = normalized.trim_end_matches('=');
    if stripped.is_empty() {
        return Err(BotBrowserError::Totp("TOTP secret is empty".into()));
    }

    let decoded = BASE32_NOPAD
        .decode(stripped.as_bytes())
        .map_err(|e| BotBrowserError::Totp(format!("invalid Base32 TOTP secret: {e}")))?;
    if decoded.is_empty() {
        return Err(BotBrowserError::Totp(
            "decoded TOTP secret cannot be empty".into(),
        ));
    }

    Ok(Zeroizing::new(decoded))
}

fn compute_hmac_truncation(
    algorithm: TotpAlgorithm,
    key: &[u8],
    counter_bytes: &[u8; 8],
) -> Result<u32, BotBrowserError> {
    let digest_bytes: Zeroizing<Vec<u8>> = match algorithm {
        TotpAlgorithm::Sha1 => {
            let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(key)
                .map_err(|e| BotBrowserError::Totp(format!("HMAC-SHA1 init failed: {e}")))?;
            mac.update(counter_bytes);
            Zeroizing::new(mac.finalize().into_bytes().to_vec())
        }
        TotpAlgorithm::Sha256 => {
            let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key)
                .map_err(|e| BotBrowserError::Totp(format!("HMAC-SHA256 init failed: {e}")))?;
            mac.update(counter_bytes);
            Zeroizing::new(mac.finalize().into_bytes().to_vec())
        }
        TotpAlgorithm::Sha512 => {
            let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(key)
                .map_err(|e| BotBrowserError::Totp(format!("HMAC-SHA512 init failed: {e}")))?;
            mac.update(counter_bytes);
            Zeroizing::new(mac.finalize().into_bytes().to_vec())
        }
    };

    let offset = (digest_bytes[digest_bytes.len() - 1] & 0x0f) as usize;
    let bin_code = (((digest_bytes[offset] & 0x7f) as u32) << 24)
        | ((digest_bytes[offset + 1] as u32) << 16)
        | ((digest_bytes[offset + 2] as u32) << 8)
        | (digest_bytes[offset + 3] as u32);
    Ok(bin_code)
}

fn generate_totp_internal(
    parsed: &ParsedTotpInput,
    unix_secs: u64,
    digits: u32,
    period_secs: u64,
) -> Result<Zeroizing<String>, BotBrowserError> {
    if period_secs == 0 {
        return Err(BotBrowserError::Totp(
            "TOTP period must be greater than 0".into(),
        ));
    }
    if !(1..=9).contains(&digits) {
        return Err(BotBrowserError::Totp(format!(
            "unsupported TOTP digit count `{digits}` (expected 1..=9)"
        )));
    }

    let key = decode_base32_secret(&parsed.secret_b32)?;
    let counter = unix_secs / period_secs;
    let counter_bytes = counter.to_be_bytes();
    let bin_code = compute_hmac_truncation(parsed.algorithm, &key, &counter_bytes)?;
    let modulus = 10u64.pow(digits);
    let otp = (bin_code as u64) % modulus;

    Ok(Zeroizing::new(format!(
        "{otp:0width$}",
        width = digits as usize
    )))
}

/// Generate a TOTP code for `secret` (raw Base32 or `otpauth://totp/...` URI) at `unix_secs`
/// with explicit `digits` and `period_secs`.
pub fn generate_totp_at(
    secret: &str,
    unix_secs: u64,
    digits: u32,
    period_secs: u64,
) -> Result<Zeroizing<String>, BotBrowserError> {
    let parsed = parse_totp_input(secret)?;
    generate_totp_internal(&parsed, unix_secs, digits, period_secs)
}

/// Generate a TOTP code for `secret` (raw Base32 or `otpauth://totp/...` URI) at `unix_secs`,
/// honoring `digits`/`period` embedded in `otpauth://` URIs or defaulting to 6 digits / 30s.
pub fn generate_totp_with_defaults_at(
    secret: &str,
    unix_secs: u64,
) -> Result<Zeroizing<String>, BotBrowserError> {
    let parsed = parse_totp_input(secret)?;
    let digits = parsed.digits.unwrap_or(6);
    let period_secs = parsed.period_secs.unwrap_or(30);
    generate_totp_internal(&parsed, unix_secs, digits, period_secs)
}

/// Generate a TOTP code for the current system UTC timestamp (honoring `otpauth://` defaults
/// or 6 digits / 30s).
pub fn generate_totp_now(secret: &str) -> Result<Zeroizing<String>, BotBrowserError> {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| BotBrowserError::Totp(format!("system clock before UNIX_EPOCH: {e}")))?
        .as_secs();
    generate_totp_with_defaults_at(secret, unix_secs)
}
