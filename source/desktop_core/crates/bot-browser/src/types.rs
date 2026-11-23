use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const HIGH_SENSITIVITY_HOSTS: &[&str] = &[
    "stripe.com",
    "paypal.com",
    "checkout.stripe.com",
    "console.aws.amazon.com",
    "signin.aws.amazon.com",
    "console.cloud.google.com",
    "portal.azure.com",
    "dash.cloudflare.com",
    "namecheap.com",
    "godaddy.com",
    "porkbun.com",
    "vercel.com",
    "alipay.com",
    "pay.weixin.qq.com",
];

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BotBrowserError {
    #[error("invalid origin: {0}")]
    InvalidOrigin(String),

    #[error("origin mismatch: expected {expected}, actual {actual}")]
    OriginMismatch { expected: String, actual: String },

    #[error("frame origin mismatch: expected {expected}, actual {actual}")]
    FrameOriginMismatch { expected: String, actual: String },

    #[error("credential not found: {0}")]
    CredentialNotFound(String),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("totp error: {0}")]
    Totp(String),

    #[error("browser session locked for atomic secret fill")]
    SessionLockedForSecretFill,

    #[error("sidecar start failed: {0}")]
    SidecarStartFailed(String),

    #[error("sidecar timeout calling {method} after {timeout_ms}ms")]
    SidecarTimeout { method: String, timeout_ms: u64 },

    #[error("sidecar checksum mismatch: expected {expected}, actual {actual}")]
    SidecarChecksumMismatch { expected: String, actual: String },

    #[error("cdp error: {0}")]
    Cdp(String),

    #[error("assist card not found: {0}")]
    CardNotFound(String),

    #[error("approval required for credential {credential_id} on {origin} (card {card_id})")]
    ApprovalRequired {
        card_id: String,
        credential_id: String,
        origin: String,
    },

    #[error("approval denied: {0}")]
    ApprovalDenied(String),

    #[error("io error: {0}")]
    Io(String),
}

impl From<std::io::Error> for BotBrowserError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

/// Canonical `scheme://host[:port]` origin (only `http` or `https`, no userinfo,
/// lowercased host, default ports `:80`/`:443` and path/query/fragment stripped).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NormalizedOrigin {
    canonical: String,
}

impl NormalizedOrigin {
    pub fn parse(raw: &str) -> Result<Self, BotBrowserError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(BotBrowserError::InvalidOrigin(
                "origin cannot be empty".to_string(),
            ));
        }

        let after_scheme = trimmed
            .split_once("://")
            .map(|(_, rest)| rest)
            .ok_or_else(|| {
                BotBrowserError::InvalidOrigin(format!("missing scheme:// in `{raw}`"))
            })?;

        let raw_authority = after_scheme
            .split(['/', '\\', '?', '#'])
            .next()
            .unwrap_or("");
        if raw_authority.contains('@') {
            return Err(BotBrowserError::InvalidOrigin(
                "embedded userinfo is not allowed in origin".to_string(),
            ));
        }

        let parsed = Url::parse(trimmed).map_err(|e| {
            BotBrowserError::InvalidOrigin(format!("failed to parse `{raw}`: {e}"))
        })?;

        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(BotBrowserError::InvalidOrigin(format!(
                "unsupported scheme `{scheme}` (only http/https allowed)"
            )));
        }

        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(BotBrowserError::InvalidOrigin(
                "embedded userinfo is not allowed in origin".to_string(),
            ));
        }

        let host = parsed
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| {
                BotBrowserError::InvalidOrigin(format!("missing host in `{raw}`"))
            })?
            .to_ascii_lowercase();

        let canonical = match parsed.port() {
            Some(port) => format!("{scheme}://{host}:{port}"),
            None => format!("{scheme}://{host}"),
        };

        Ok(Self { canonical })
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    pub fn host(&self) -> &str {
        let authority = self
            .canonical
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(&self.canonical);

        if authority.starts_with('[') {
            if let Some(end_bracket) = authority.find(']') {
                return &authority[..=end_bracket];
            }
            return authority;
        }

        match authority.split_once(':') {
            Some((host, _)) => host,
            None => authority,
        }
    }

    pub fn is_high_sensitivity(&self) -> bool {
        let host = self.host();
        HIGH_SENSITIVITY_HOSTS.iter().any(|domain| {
            host == *domain
                || (host.len() > domain.len()
                    && host.ends_with(domain)
                    && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
        })
    }
}

impl std::fmt::Display for NormalizedOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.canonical)
    }
}

impl AsRef<str> for NormalizedOrigin {
    fn as_ref(&self) -> &str {
        &self.canonical
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReusePolicy {
    AutoReuse,
    ConfirmEveryTime,
}

impl ReusePolicy {
    pub fn default_for_origin(origin: &NormalizedOrigin) -> Self {
        if origin.is_high_sensitivity() {
            Self::ConfirmEveryTime
        } else {
            Self::AutoReuse
        }
    }
}

/// AI-visible credential metadata; intentionally holds zero secret fields.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CredentialMeta {
    pub credential_id: String,
    pub origin: String,
    pub account_hint: String,
    pub has_totp: bool,
    pub reuse_policy: ReusePolicy,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
}

/// Decrypted secret material strictly scoped in-process; never serializable and
/// redacted in `Debug`.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DecryptedSecret {
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
    pub totp_secret: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for DecryptedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DecryptedSecret {{ username: [REDACTED], password: [REDACTED], has_totp: {} }}",
            self.totp_secret.is_some()
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ElementRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub device_pixel_ratio: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DomNodeSnapshot {
    pub ref_id: String,
    pub tag: String,
    pub input_type: Option<String>,
    pub name: Option<String>,
    pub id: Option<String>,
    pub aria_label: Option<String>,
    pub placeholder: Option<String>,
    pub text: String,
    pub value: Option<String>,
    pub is_password: bool,
    pub selector: String,
    pub frame_origin: Option<String>,
    pub rect: Option<ElementRect>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BrowserSnapshot {
    pub url: String,
    pub origin: String,
    pub title: String,
    pub text_excerpt: String,
    pub interactive_elements: Vec<DomNodeSnapshot>,
    pub redacted_field_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistCardStatus {
    AwaitingUser,
    TimedOutResumable,
    SavedAndLoggedIn,
    ApprovedAndFilling,
    Completed,
    Denied,
    Expired,
    Cancelled,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AssistCardPayload {
    VaultLogin {
        card_id: String,
        task_id: String,
        session_id: String,
        chat_id: Option<i64>,
        origin: String,
        target_url: String,
        site_title: String,
        suggested_reuse_policy: ReusePolicy,
        is_high_sensitivity: bool,
        status: AssistCardStatus,
        status_note: Option<String>,
        saved_credential_id: Option<String>,
        saved_account_hint: Option<String>,
        created_at_ms: i64,
    },
    ReuseConfirm {
        card_id: String,
        task_id: String,
        session_id: String,
        chat_id: Option<i64>,
        credential_id: String,
        origin: String,
        target_url: String,
        account_hint: String,
        has_totp: bool,
        require_biometric_hint: bool,
        status: AssistCardStatus,
        status_note: Option<String>,
        created_at_ms: i64,
    },
    QrLogin {
        card_id: String,
        task_id: String,
        session_id: String,
        chat_id: Option<i64>,
        origin: String,
        target_url: String,
        png_bytes: Vec<u8>,
        countdown_secs: u32,
        refresh_interval_secs: u32,
        poll_attempt: u32,
        status: AssistCardStatus,
        status_note: Option<String>,
        redirected_url: Option<String>,
        created_at_ms: i64,
    },
    ClickCaptcha {
        card_id: String,
        task_id: String,
        session_id: String,
        chat_id: Option<i64>,
        origin: String,
        target_url: String,
        png_bytes: Vec<u8>,
        element_rect: ElementRect,
        instruction: String,
        max_clicks: u8,
        status: AssistCardStatus,
        status_note: Option<String>,
        created_at_ms: i64,
    },
}

impl AssistCardPayload {
    pub fn card_id(&self) -> &str {
        match self {
            Self::VaultLogin { card_id, .. }
            | Self::ReuseConfirm { card_id, .. }
            | Self::QrLogin { card_id, .. }
            | Self::ClickCaptcha { card_id, .. } => card_id,
        }
    }

    pub fn chat_id(&self) -> Option<i64> {
        match self {
            Self::VaultLogin { chat_id, .. }
            | Self::ReuseConfirm { chat_id, .. }
            | Self::QrLogin { chat_id, .. }
            | Self::ClickCaptcha { chat_id, .. } => *chat_id,
        }
    }

    pub fn status(&self) -> AssistCardStatus {
        match self {
            Self::VaultLogin { status, .. }
            | Self::ReuseConfirm { status, .. }
            | Self::QrLogin { status, .. }
            | Self::ClickCaptcha { status, .. } => *status,
        }
    }

    pub fn set_status(&mut self, new_status: AssistCardStatus, note: Option<String>) {
        match self {
            Self::VaultLogin {
                status,
                status_note,
                ..
            }
            | Self::ReuseConfirm {
                status,
                status_note,
                ..
            }
            | Self::QrLogin {
                status,
                status_note,
                ..
            }
            | Self::ClickCaptcha {
                status,
                status_note,
                ..
            } => {
                *status = new_status;
                *status_note = note;
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct NormalizedClickPoint {
    pub norm_x: f64,
    pub norm_y: f64,
}

impl NormalizedClickPoint {
    pub fn new(norm_x: f64, norm_y: f64) -> Self {
        Self {
            norm_x: if norm_x.is_nan() {
                0.0
            } else {
                norm_x.clamp(0.0, 1.0)
            },
            norm_y: if norm_y.is_nan() {
                0.0
            } else {
                norm_y.clamp(0.0, 1.0)
            },
        }
    }

    pub fn clamped(self) -> Self {
        Self::new(self.norm_x, self.norm_y)
    }
}

/// Local-only in-process login submission from GPUI UI to vault/orchestrator.
/// Never serializable and redacted in `Debug`.
pub struct LocalLoginSubmission {
    pub card_id: String,
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
    pub totp_secret: Option<Zeroizing<String>>,
    pub account_hint: Option<String>,
    pub allow_auto_reuse: bool,
}

impl std::fmt::Debug for LocalLoginSubmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalLoginSubmission")
            .field("card_id", &self.card_id)
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .field("has_totp", &self.totp_secret.is_some())
            .field("account_hint", &self.account_hint)
            .field("allow_auto_reuse", &self.allow_auto_reuse)
            .finish()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TaskWakeup {
    pub task_id: String,
    pub session_id: String,
    pub chat_id: Option<i64>,
    pub target_url: String,
    pub origin: String,
    pub credential_id: String,
    pub triggered_at_ms: i64,
}

#[derive(Clone, Debug)]
pub enum BotBrowserEvent {
    UpsertCard(AssistCardPayload),
    UpdateCardStatus {
        card_id: String,
        chat_id: Option<i64>,
        status: AssistCardStatus,
        status_note: Option<String>,
        saved_credential_id: Option<String>,
        saved_account_hint: Option<String>,
        redirected_url: Option<String>,
    },
    RefreshQrImage {
        card_id: String,
        chat_id: Option<i64>,
        png_bytes: Vec<u8>,
        countdown_secs: u32,
        poll_attempt: u32,
    },
    TaskWakeupTriggered(TaskWakeup),
    SidecarRestarted {
        session_id: String,
        restart_count: u64,
        restored_url: Option<String>,
    },
}
