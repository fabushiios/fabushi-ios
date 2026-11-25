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

    #[error("locator not found: {0}")]
    LocatorNotFound(String),

    #[error("unsupported backend operation: {0}")]
    Unsupported(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("cookie import error: {0}")]
    CookieImport(String),

    #[error("script compilation error: {0}")]
    ScriptCompilation(String),

    #[error("script validation error: {0}")]
    ScriptValidation(String),

    #[error("script replay error: {0}")]
    ScriptReplay(String),

    #[error("secret leak detected in script manifest: {0}")]
    SecretLeakDetected(String),

    #[error("browser automation cancelled by user: {0}")]
    ControlCancelled(String),

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserBackendKind {
    WkWebView,
    ObscuraCdp,
    ChromeCdp,
    MockCdp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SemanticLocator {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aria_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_anchor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css_selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_attr: Option<String>,
}

impl SemanticLocator {
    pub fn from_test_id(id: impl Into<String>) -> Self {
        Self {
            test_id: Some(id.into()),
            ..Default::default()
        }
    }

    pub fn by_test_id(id: impl Into<String>) -> Self {
        Self::from_test_id(id)
    }

    pub fn from_aria_label(label: impl Into<String>) -> Self {
        Self {
            aria_label: Some(label.into()),
            ..Default::default()
        }
    }

    pub fn by_aria_label(label: impl Into<String>) -> Self {
        Self::from_aria_label(label)
    }

    pub fn by_role_and_text(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: Some(role.into()),
            text_anchor: Some(text.into()),
            ..Default::default()
        }
    }

    pub fn by_text(text: impl Into<String>) -> Self {
        Self {
            text_anchor: Some(text.into()),
            ..Default::default()
        }
    }

    pub fn from_css(sel: impl Into<String>) -> Self {
        Self {
            css_selector: Some(sel.into()),
            ..Default::default()
        }
    }

    pub fn by_css(sel: impl Into<String>) -> Self {
        Self::from_css(sel)
    }

    pub fn by_name(name: impl Into<String>) -> Self {
        Self {
            name_attr: Some(name.into()),
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        [
            &self.test_id,
            &self.aria_label,
            &self.role,
            &self.text_anchor,
            &self.css_selector,
            &self.name_attr,
        ]
        .iter()
        .all(|opt| opt.as_deref().is_none_or(|s| s.trim().is_empty()))
    }

    pub fn display_label(&self) -> String {
        if let Some(tid) = self.test_id.as_deref().filter(|s| !s.trim().is_empty()) {
            return format!("test_id={tid}");
        }
        if let Some(aria) = self.aria_label.as_deref().filter(|s| !s.trim().is_empty()) {
            return format!("aria_label={aria}");
        }
        if let Some(txt) = self.text_anchor.as_deref().filter(|s| !s.trim().is_empty()) {
            if let Some(role) = self.role.as_deref().filter(|s| !s.trim().is_empty()) {
                return format!("role={role}:text={txt}");
            }
            return format!("text={txt}");
        }
        if let Some(nm) = self.name_attr.as_deref().filter(|s| !s.trim().is_empty()) {
            return format!("name={nm}");
        }
        if let Some(css) = self.css_selector.as_deref().filter(|s| !s.trim().is_empty()) {
            return format!("css={css}");
        }
        if let Some(role) = self.role.as_deref().filter(|s| !s.trim().is_empty()) {
            return format!("role={role}");
        }
        "empty".to_string()
    }

    pub fn describe(&self) -> String {
        self.display_label()
    }

    pub fn to_css_candidates(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(tid) = self.test_id.as_deref().filter(|s| !s.trim().is_empty()) {
            out.push(format!(r#"[data-testid="{}"]"#, escape_css_attr(tid)));
        }
        if let Some(aria) = self.aria_label.as_deref().filter(|s| !s.trim().is_empty()) {
            out.push(format!(r#"[aria-label="{}"]"#, escape_css_attr(aria)));
        }
        if let Some(nm) = self.name_attr.as_deref().filter(|s| !s.trim().is_empty()) {
            out.push(format!(r#"[name="{}"]"#, escape_css_attr(nm)));
        }
        if let Some(css) = self.css_selector.as_deref().filter(|s| !s.trim().is_empty()) {
            out.push(css.trim().to_string());
        }
        if let Some(role) = self.role.as_deref().filter(|s| !s.trim().is_empty()) {
            out.push(format!(r#"[role="{}"]"#, escape_css_attr(role)));
        }
        out
    }

    pub fn is_sensitive_hint(&self) -> bool {
        const MARKERS: &[&str] = &[
            "password",
            "passwd",
            "passcode",
            "pwd",
            "one-time-code",
            "totp",
            "secret",
            "api_key",
            "apikey",
            "auth_token",
            "access_token",
            "ct0",
        ];
        for candidate in [
            self.test_id.as_deref(),
            self.aria_label.as_deref(),
            self.css_selector.as_deref(),
            self.name_attr.as_deref(),
            self.text_anchor.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let lower = candidate.to_ascii_lowercase();
            if MARKERS.iter().any(|m| lower.contains(m)) {
                return true;
            }
            if lower
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|tok| tok == "otp" || tok == "token" || tok == "pin")
            {
                return true;
            }
        }
        false
    }
}

fn escape_css_attr(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SameSitePolicy {
    Strict,
    Lax,
    None,
}

impl<'de> Deserialize<'de> for SameSitePolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        match raw.trim().to_ascii_lowercase().as_str() {
            "strict" => Ok(Self::Strict),
            "lax" => Ok(Self::Lax),
            "none" | "no_restriction" => Ok(Self::None),
            other => Err(serde::de::Error::custom(format!(
                "unsupported sameSite value: {other}"
            ))),
        }
    }
}

fn default_cookie_path() -> String {
    "/".to_string()
}

fn deserialize_same_site_opt<'de, D>(deserializer: D) -> Result<Option<SameSitePolicy>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(val) = opt else {
        return Ok(None);
    };
    let Some(raw) = val.as_str() else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "unspecified" | "default" => Ok(None),
        "strict" => Ok(Some(SameSitePolicy::Strict)),
        "lax" => Ok(Some(SameSitePolicy::Lax)),
        "none" | "no_restriction" => Ok(Some(SameSitePolicy::None)),
        other => Err(serde::de::Error::custom(format!(
            "unsupported sameSite value: {other}"
        ))),
    }
}

fn deserialize_cookie_expires_opt<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(val) = opt else {
        return Ok(None);
    };
    match val {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(num) => {
            if let Some(i) = num.as_i64() {
                if i <= 0 {
                    Ok(None)
                } else {
                    Ok(Some(i))
                }
            } else if let Some(u) = num.as_u64() {
                if u == 0 {
                    Ok(None)
                } else {
                    Ok(Some(i64::try_from(u).unwrap_or(i64::MAX)))
                }
            } else if let Some(f) = num.as_f64() {
                if !f.is_finite() || f <= 0.0 {
                    Ok(None)
                } else {
                    Ok(Some(f.round() as i64))
                }
            } else {
                Ok(None)
            }
        }
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let parsed = trimmed
                .parse::<f64>()
                .map_err(|_| serde::de::Error::custom("invalid cookie expiration string"))?;
            if !parsed.is_finite() || parsed <= 0.0 {
                Ok(None)
            } else {
                Ok(Some(parsed.round() as i64))
            }
        }
        _ => Err(serde::de::Error::custom("invalid cookie expiration type")),
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default = "default_cookie_path")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default, rename = "httpOnly", alias = "http_only")]
    pub http_only: bool,
    #[serde(
        default,
        rename = "sameSite",
        alias = "same_site",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_same_site_opt"
    )]
    #[zeroize(skip)]
    pub same_site: Option<SameSitePolicy>,
    #[serde(
        default,
        alias = "expires_unix_secs",
        alias = "expirationDate",
        alias = "expiry",
        alias = "expires_epoch_secs",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_cookie_expires_opt"
    )]
    pub expires: Option<i64>,
    #[serde(default, rename = "hostOnly", alias = "host_only")]
    pub host_only: bool,
    #[serde(default)]
    pub session: bool,
}

impl std::fmt::Debug for BrowserCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserCookie")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("same_site", &self.same_site)
            .field("expires", &self.expires)
            .field("host_only", &self.host_only)
            .field("session", &self.session)
            .finish()
    }
}

impl BrowserCookie {
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".to_string(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
            host_only: false,
            session: true,
        }
    }

    pub fn is_expired_at(&self, now_unix_secs: i64) -> bool {
        self.expires.is_some_and(|exp| exp <= now_unix_secs)
    }

    pub fn matches_host(&self, host: &str) -> bool {
        let host_norm = host.trim().trim_start_matches('.').to_ascii_lowercase();
        let domain_norm = self.domain.trim().trim_start_matches('.').to_ascii_lowercase();
        if host_norm.is_empty() || domain_norm.is_empty() {
            return false;
        }
        if self.host_only {
            host_norm == domain_norm
        } else {
            host_norm == domain_norm || host_norm.ends_with(&format!(".{domain_norm}"))
        }
    }

    pub fn to_cdp_cookie_param(&self) -> serde_json::Value {
        let path = if self.path.trim().is_empty() {
            "/"
        } else if self.path.starts_with('/') {
            self.path.as_str()
        } else {
            "/"
        };
        let mut obj = serde_json::Map::new();
        obj.insert("name".to_string(), serde_json::Value::String(self.name.clone()));
        obj.insert("value".to_string(), serde_json::Value::String(self.value.clone()));
        obj.insert("domain".to_string(), serde_json::Value::String(self.domain.clone()));
        obj.insert("path".to_string(), serde_json::Value::String(path.to_string()));
        obj.insert("secure".to_string(), serde_json::Value::Bool(self.secure));
        obj.insert("httpOnly".to_string(), serde_json::Value::Bool(self.http_only));
        if let Some(ss) = self.same_site {
            let ss_str = match ss {
                SameSitePolicy::Strict => "Strict",
                SameSitePolicy::Lax => "Lax",
                SameSitePolicy::None => "None",
            };
            obj.insert("sameSite".to_string(), serde_json::Value::String(ss_str.to_string()));
        }
        if !self.session {
            if let Some(exp) = self.expires.filter(|&e| e > 0) {
                obj.insert("expires".to_string(), serde_json::json!(exp));
            }
        }
        let clean_domain = self.domain.trim().trim_start_matches('.');
        if !clean_domain.is_empty() {
            let scheme = if self.secure { "https" } else { "http" };
            obj.insert(
                "url".to_string(),
                serde_json::Value::String(format!("{scheme}://{clean_domain}{path}")),
            );
        }
        serde_json::Value::Object(obj)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepTimingMetric {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_index: Option<usize>,
    pub operation: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BrowserBackendKind>,
    pub duration_ms: u64,
    pub success: bool,
    #[serde(default)]
    pub timestamp_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
