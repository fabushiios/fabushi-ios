use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use aes::Aes128;
use cbc::cipher::generic_array::GenericArray;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use chrono::Utc;
use pbkdf2::pbkdf2_hmac;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tempfile::{NamedTempFile, TempDir};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::e2ee_sync::{
    BrowserProfileSnapshot, EncryptedVaultBlob, UnlockedUserVaultKey, UserVaultCategory,
    validate_item_key,
};
use crate::sidecar::ProfileNamespace;
use crate::types::{BotBrowserError, NormalizedOrigin};
use crate::vault::CredentialVault;

pub use crate::types::{BrowserCookie, SameSitePolicy};

pub type LocalStorageMap = BTreeMap<String, BTreeMap<String, String>>;

const CHROME_PBKDF2_SALT: &[u8] = b"saltysalt";
const CHROME_PBKDF2_ITERATIONS: u32 = 1003;
const CHROME_V10_IV: [u8; 16] = [0x20u8; 16];
const WEBKIT_UNIX_EPOCH_OFFSET_SECS: i64 = 11_644_473_600;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserProfileData {
    #[serde(default)]
    pub cookies: Vec<BrowserCookie>,
    #[serde(default)]
    pub local_storage: LocalStorageMap,
    #[serde(default)]
    pub updated_at_ms: i64,
}

impl std::fmt::Debug for BrowserProfileData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserProfileData")
            .field("cookie_count", &self.cookies.len())
            .field("origin_count", &self.local_storage.len())
            .field("updated_at_ms", &self.updated_at_ms)
            .finish()
    }
}

impl Zeroize for BrowserProfileData {
    fn zeroize(&mut self) {
        for cookie in &mut self.cookies {
            cookie.zeroize();
        }
        self.cookies.clear();
        for (_origin, kv) in &mut self.local_storage {
            for (_k, v) in kv.iter_mut() {
                v.zeroize();
            }
            kv.clear();
        }
        self.local_storage.clear();
        self.updated_at_ms.zeroize();
    }
}

impl Drop for BrowserProfileData {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for BrowserProfileData {}

#[derive(Debug, Clone)]
pub struct BrowserProfileStore {
    base_dir: PathBuf,
}

impl BrowserProfileStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn profile_key(namespace: &str, origin: Option<&NormalizedOrigin>) -> String {
        ProfileNamespace::profile_key_for(namespace, origin)
    }

    pub fn profile_dir(&self, namespace: &str, origin: Option<&NormalizedOrigin>) -> PathBuf {
        ProfileNamespace::storage_dir_for(&self.base_dir, namespace, origin)
    }

    pub fn profile_dir_by_key(&self, item_key: &str) -> Result<PathBuf, BotBrowserError> {
        validate_item_key(item_key)?;
        if item_key == "."
            || item_key.contains('/')
            || item_key.contains('\\')
            || item_key.contains("..")
        {
            return Err(BotBrowserError::Storage(format!(
                "unsafe profile item_key `{item_key}`"
            )));
        }
        Ok(self.base_dir.join("profiles").join(item_key))
    }

    pub fn save_cookies(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
        cookies: &[BrowserCookie],
    ) -> Result<PathBuf, BotBrowserError> {
        let dir = self.profile_dir(namespace, origin);
        save_cookies_to_dir(&dir, cookies)
    }

    pub fn merge_cookies(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
        incoming: &[BrowserCookie],
    ) -> Result<usize, BotBrowserError> {
        let mut existing = self.load_cookies(namespace, origin)?;
        for cookie in incoming {
            let mut normalized = cookie.clone();
            validate_and_normalize_cookie(&mut normalized)?;
            let key = cookie_dedup_key(&normalized);
            if let Some(pos) = existing.iter().position(|c| cookie_dedup_key(c) == key) {
                existing[pos] = normalized;
            } else {
                existing.push(normalized);
            }
        }
        self.save_cookies(namespace, origin, &existing)?;
        Ok(existing.len())
    }

    pub fn load_cookies(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> Result<Vec<BrowserCookie>, BotBrowserError> {
        let dir = self.profile_dir(namespace, origin);
        load_cookies_from_dir(&dir)
    }

    pub fn load_active_cookies(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
        now_unix_secs: i64,
    ) -> Result<Vec<BrowserCookie>, BotBrowserError> {
        let cookies = self.load_cookies(namespace, origin)?;
        Ok(cookies
            .into_iter()
            .filter(|c| !c.is_expired_at(now_unix_secs))
            .collect())
    }

    pub fn save_local_storage(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
        storage: &LocalStorageMap,
    ) -> Result<PathBuf, BotBrowserError> {
        let dir = self.profile_dir(namespace, origin);
        save_local_storage_to_dir(&dir, storage)
    }

    pub fn load_local_storage(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> Result<LocalStorageMap, BotBrowserError> {
        let dir = self.profile_dir(namespace, origin);
        load_local_storage_from_dir(&dir)
    }

    pub fn save_profile(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
        data: &BrowserProfileData,
    ) -> Result<PathBuf, BotBrowserError> {
        let dir = self.profile_dir(namespace, origin);
        save_cookies_to_dir(&dir, &data.cookies)?;
        save_local_storage_to_dir(&dir, &data.local_storage)?;
        Ok(dir)
    }

    pub fn load_profile(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> Result<BrowserProfileData, BotBrowserError> {
        let dir = self.profile_dir(namespace, origin);
        load_profile_from_dir(&dir)
    }

    pub fn clear_profile(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> Result<(), BotBrowserError> {
        let dir = self.profile_dir(namespace, origin);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    pub fn pack_profile_bundle(
        &self,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
        max_bytes: usize,
    ) -> Result<Zeroizing<Vec<u8>>, BotBrowserError> {
        let dir = self.profile_dir(namespace, origin);
        BrowserProfileSnapshot::pack_storage_dir(&dir, None, max_bytes)
    }

    pub fn seal_to_vault_blob(
        &self,
        key: &UnlockedUserVaultKey,
        user_id: u64,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
        revision: i64,
        max_bytes: usize,
    ) -> Result<EncryptedVaultBlob, BotBrowserError> {
        let item_key = Self::profile_key(namespace, origin);
        let bundle = self.pack_profile_bundle(namespace, origin, max_bytes)?;
        key.seal_item(
            user_id,
            UserVaultCategory::BrowserCookie,
            &item_key,
            revision,
            &bundle,
        )
    }

    pub fn restore_from_vault_blob(
        &self,
        key: &UnlockedUserVaultKey,
        user_id: u64,
        blob: &EncryptedVaultBlob,
    ) -> Result<(String, BrowserProfileData), BotBrowserError> {
        if blob.category != UserVaultCategory::BrowserCookie {
            return Err(BotBrowserError::Storage(format!(
                "expected browser_cookie vault blob, got {}",
                blob.category.as_str()
            )));
        }
        let target_dir = self.profile_dir_by_key(&blob.item_key)?;
        let bundle = key.open_item(user_id, blob)?;
        let root_cookies = BrowserProfileSnapshot::unpack_storage_dir(&bundle, &target_dir)?;
        if let Some(cookie_vals) = root_cookies {
            let cookies_path = target_dir.join("cookies.json");
            if !cookies_path.exists() {
                let mut parsed = Vec::with_capacity(cookie_vals.len());
                for (idx, val) in cookie_vals.into_iter().enumerate() {
                    let mut cookie: BrowserCookie = serde_json::from_value(val).map_err(|e| {
                        BotBrowserError::CookieImport(format!(
                            "invalid cookie object at index {idx} in vault bundle: {e}"
                        ))
                    })?;
                    validate_and_normalize_cookie(&mut cookie)?;
                    parsed.push(cookie);
                }
                save_cookies_to_dir(&target_dir, &parsed)?;
            }
        }
        let data = load_profile_from_dir(&target_dir)?;
        Ok((blob.item_key.clone(), data))
    }

    pub fn backup_to_credential_vault(
        &self,
        vault: &CredentialVault,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
        max_bytes: usize,
    ) -> Result<(), BotBrowserError> {
        let profile_key = Self::profile_key(namespace, origin);
        let bundle = self.pack_profile_bundle(namespace, origin, max_bytes)?;
        vault.store_profile_snapshot(&profile_key, origin, &bundle)
    }

    pub fn restore_from_credential_vault(
        &self,
        vault: &CredentialVault,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> Result<bool, BotBrowserError> {
        let profile_key = Self::profile_key(namespace, origin);
        let Some(bundle) = vault.load_profile_snapshot(&profile_key, origin)? else {
            return Ok(false);
        };
        let target_dir = self.profile_dir(namespace, origin);
        let root_cookies = BrowserProfileSnapshot::unpack_storage_dir(&bundle, &target_dir)?;
        if let Some(cookie_vals) = root_cookies {
            let cookies_path = target_dir.join("cookies.json");
            if !cookies_path.exists() {
                let mut parsed = Vec::with_capacity(cookie_vals.len());
                for (idx, val) in cookie_vals.into_iter().enumerate() {
                    let mut cookie: BrowserCookie = serde_json::from_value(val).map_err(|e| {
                        BotBrowserError::CookieImport(format!(
                            "invalid cookie object at index {idx} in local vault snapshot: {e}"
                        ))
                    })?;
                    validate_and_normalize_cookie(&mut cookie)?;
                    parsed.push(cookie);
                }
                save_cookies_to_dir(&target_dir, &parsed)?;
            }
        }
        Ok(true)
    }
}

fn ensure_private_dir(dir: &Path) -> Result<(), BotBrowserError> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), BotBrowserError> {
    let parent = path.parent().ok_or_else(|| {
        BotBrowserError::Storage("target file has no parent directory".to_string())
    })?;
    ensure_private_dir(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| BotBrowserError::Io(e.error.to_string()))?;
    Ok(())
}

fn save_cookies_to_dir(dir: &Path, cookies: &[BrowserCookie]) -> Result<PathBuf, BotBrowserError> {
    ensure_private_dir(dir)?;
    let mut normalized = Vec::with_capacity(cookies.len());
    for cookie in cookies {
        let mut c = cookie.clone();
        validate_and_normalize_cookie(&mut c)?;
        normalized.push(c);
    }
    let json_bytes = Zeroizing::new(serde_json::to_vec_pretty(&normalized).map_err(|e| {
        BotBrowserError::Storage(format!("failed serializing cookies: {e}"))
    })?);
    let path = dir.join("cookies.json");
    write_private_atomic(&path, &json_bytes)?;
    Ok(path)
}

fn load_cookies_from_dir(dir: &Path) -> Result<Vec<BrowserCookie>, BotBrowserError> {
    let path = dir.join("cookies.json");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = Zeroizing::new(fs::read_to_string(&path)?);
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    import_cookies_from_json(&raw)
}

fn save_local_storage_to_dir(
    dir: &Path,
    storage: &LocalStorageMap,
) -> Result<PathBuf, BotBrowserError> {
    let mut canonical_map: LocalStorageMap = BTreeMap::new();
    for (raw_origin, entries) in storage {
        let canonical = NormalizedOrigin::parse(raw_origin)?.as_str().to_string();
        let target = canonical_map.entry(canonical).or_default();
        for (k, v) in entries {
            target.insert(k.clone(), v.clone());
        }
    }
    let json_bytes = Zeroizing::new(serde_json::to_vec_pretty(&canonical_map).map_err(|e| {
        BotBrowserError::Storage(format!("failed serializing localStorage: {e}"))
    })?);
    let path = dir.join("storage").join("local_storage.json");
    write_private_atomic(&path, &json_bytes)?;
    Ok(path)
}

fn load_local_storage_from_dir(dir: &Path) -> Result<LocalStorageMap, BotBrowserError> {
    let path = dir.join("storage").join("local_storage.json");
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let raw = Zeroizing::new(fs::read_to_string(&path)?);
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let map: LocalStorageMap = serde_json::from_str(&raw).map_err(|_| {
        BotBrowserError::Storage("corrupted local_storage.json payload".to_string())
    })?;
    Ok(map)
}

fn load_profile_from_dir(dir: &Path) -> Result<BrowserProfileData, BotBrowserError> {
    let cookies = load_cookies_from_dir(dir)?;
    let local_storage = load_local_storage_from_dir(dir)?;
    Ok(BrowserProfileData {
        cookies,
        local_storage,
        updated_at_ms: Utc::now().timestamp_millis(),
    })
}

fn cookie_dedup_key(cookie: &BrowserCookie) -> (String, String, String) {
    (
        cookie.domain.trim_start_matches('.').to_ascii_lowercase(),
        cookie.path.clone(),
        cookie.name.clone(),
    )
}

pub fn import_cookies_from_text(input: &str) -> Result<Vec<BrowserCookie>, BotBrowserError> {
    import_cookies_from_json(input)
}

pub fn import_cookies_from_json(json_str: &str) -> Result<Vec<BrowserCookie>, BotBrowserError> {
    let stripped = json_str.trim_start_matches('\u{feff}').trim();
    if stripped.is_empty() {
        return Err(BotBrowserError::CookieImport(
            "cookie input is empty".to_string(),
        ));
    }

    if !stripped.starts_with('[') && !stripped.starts_with('{') {
        return import_cookies_from_netscape(stripped);
    }

    let root: serde_json::Value = serde_json::from_str(stripped)
        .map_err(|_| BotBrowserError::CookieImport("invalid cookie JSON syntax".to_string()))?;

    let items = match root {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(mut obj) => {
            if let Some(serde_json::Value::Array(arr)) = obj.remove("cookies") {
                arr
            } else if let Some(serde_json::Value::Object(mut res)) = obj.remove("result") {
                if let Some(serde_json::Value::Array(arr)) = res.remove("cookies") {
                    arr
                } else {
                    return Err(BotBrowserError::CookieImport(
                        "unsupported cookie JSON result shape (expected result.cookies array)"
                            .to_string(),
                    ));
                }
            } else {
                return Err(BotBrowserError::CookieImport(
                    "unsupported cookie JSON object shape (expected array or {\"cookies\":[...]})"
                        .to_string(),
                ));
            }
        }
        _ => {
            return Err(BotBrowserError::CookieImport(
                "unsupported cookie JSON root type".to_string(),
            ));
        }
    };

    let mut cookies = Vec::with_capacity(items.len());
    for (idx, item) in items.into_iter().enumerate() {
        let mut cookie: BrowserCookie = serde_json::from_value(item).map_err(|_| {
            BotBrowserError::CookieImport(format!(
                "invalid cookie object at index {idx}"
            ))
        })?;
        validate_and_normalize_cookie(&mut cookie)?;
        cookies.push(cookie);
    }
    Ok(cookies)
}

pub fn import_cookies_from_netscape(text: &str) -> Result<Vec<BrowserCookie>, BotBrowserError> {
    let stripped = text.trim_start_matches('\u{feff}');
    if stripped.trim().is_empty() {
        return Err(BotBrowserError::CookieImport(
            "Netscape cookie input is empty".to_string(),
        ));
    }

    let mut cookies = Vec::new();
    for (idx, raw_line) in stripped.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }

        let (http_only, body) = if line
            .as_bytes()
            .get(..10)
            .is_some_and(|p| p.eq_ignore_ascii_case(b"#HttpOnly_"))
        {
            (true, &line[10..])
        } else if line.trim_start().starts_with('#') {
            continue;
        } else {
            (false, line)
        };

        let fields: Vec<&str> = body.splitn(7, '\t').collect();
        if fields.len() != 7 {
            return Err(BotBrowserError::CookieImport(format!(
                "invalid Netscape cookie line {line_no}: expected 7 tab-separated fields"
            )));
        }

        let domain = fields[0].trim().to_string();
        let include_subdomains = parse_netscape_bool(fields[1], line_no, "include_subdomains")?;
        let path = fields[2].trim().to_string();
        let secure = parse_netscape_bool(fields[3], line_no, "secure")?;
        let exp_raw = fields[4].trim();
        let expires = if exp_raw.is_empty() {
            None
        } else {
            let parsed = exp_raw.parse::<f64>().map_err(|_| {
                BotBrowserError::CookieImport(format!(
                    "invalid Netscape cookie expiration on line {line_no}"
                ))
            })?;
            if !parsed.is_finite() || parsed <= 0.0 {
                None
            } else {
                Some(parsed.round() as i64)
            }
        };
        let name = fields[5].to_string();
        let value = fields[6].to_string();

        let mut cookie = BrowserCookie {
            name,
            value,
            domain,
            path,
            secure,
            http_only,
            same_site: None,
            expires,
            host_only: !include_subdomains,
            session: expires.is_none(),
        };
        validate_and_normalize_cookie(&mut cookie)?;
        cookies.push(cookie);
    }

    if cookies.is_empty() {
        return Err(BotBrowserError::CookieImport(
            "Netscape cookie file contained no cookie entries".to_string(),
        ));
    }
    Ok(cookies)
}

fn parse_netscape_bool(raw: &str, line_no: usize, field: &str) -> Result<bool, BotBrowserError> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "TRUE" => Ok(true),
        "FALSE" => Ok(false),
        _ => Err(BotBrowserError::CookieImport(format!(
            "invalid Netscape boolean for {field} on line {line_no}"
        ))),
    }
}

pub(crate) fn validate_and_normalize_cookie(
    cookie: &mut BrowserCookie,
) -> Result<(), BotBrowserError> {
    if cookie
        .name
        .chars()
        .any(|c| c.is_ascii_control() || matches!(c, '\r' | '\n' | '\0' | ';' | '='))
    {
        return Err(BotBrowserError::CookieImport(
            "cookie name contains disallowed control characters".to_string(),
        ));
    }
    let trimmed_name = cookie.name.trim();
    if trimmed_name.is_empty() {
        return Err(BotBrowserError::CookieImport(
            "cookie name cannot be empty".to_string(),
        ));
    }
    cookie.name = trimmed_name.to_string();

    if cookie
        .value
        .chars()
        .any(|c| c.is_ascii_control() || matches!(c, '\r' | '\n' | '\0' | ';'))
    {
        return Err(BotBrowserError::CookieImport(format!(
            "cookie '{}' value contains disallowed control characters",
            cookie.name
        )));
    }

    let trimmed_domain = cookie.domain.trim().to_ascii_lowercase();
    if trimmed_domain.is_empty()
        || trimmed_domain
            .chars()
            .any(|c| c.is_whitespace() || c.is_ascii_control() || matches!(c, '\0' | ';'))
    {
        return Err(BotBrowserError::CookieImport(format!(
            "cookie '{}' has invalid domain",
            cookie.name
        )));
    }
    cookie.domain = trimmed_domain;

    if cookie
        .path
        .chars()
        .any(|c| c.is_ascii_control() || matches!(c, '\r' | '\n' | '\0' | ';'))
    {
        return Err(BotBrowserError::CookieImport(format!(
            "cookie '{}' path contains disallowed control characters or ';'",
            cookie.name
        )));
    }
    let trimmed_path = cookie.path.trim();
    if trimmed_path.is_empty() {
        cookie.path = "/".to_string();
    } else if !trimmed_path.starts_with('/') {
        cookie.path = format!("/{trimmed_path}");
    } else {
        cookie.path = trimmed_path.to_string();
    }

    if cookie.expires.is_some_and(|e| e <= 0) {
        cookie.expires = None;
    }
    cookie.session = cookie.expires.is_none();

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChromeImportProbe {
    Available {
        path: PathBuf,
        profile_name: String,
        size_bytes: u64,
        encrypted_v10: bool,
    },
    PermissionDeniedTcc {
        path: PathBuf,
        guidance: String,
    },
    NotFound,
}

fn tcc_guidance_message() -> String {
    if cfg!(target_os = "macos") {
        "macOS TCC denied read access to Chrome's Cookies database. Grant Full Disk Access in System Settings > Privacy & Security > Full Disk Access, or paste/import cookies JSON/Netscape manually."
            .to_string()
    } else {
        "Permission denied reading Chrome's Cookies database. Adjust file permissions or paste/import cookies JSON/Netscape manually."
            .to_string()
    }
}

pub fn classify_probe_io_error(path: &Path, err: &std::io::Error) -> ChromeImportProbe {
    if err.kind() == ErrorKind::PermissionDenied || matches!(err.raw_os_error(), Some(1) | Some(13))
    {
        ChromeImportProbe::PermissionDeniedTcc {
            path: path.to_path_buf(),
            guidance: tcc_guidance_message(),
        }
    } else {
        ChromeImportProbe::NotFound
    }
}

pub fn probe_chrome_cookies_at(path: &Path) -> ChromeImportProbe {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(err) => return classify_probe_io_error(path, &err),
    };

    let Ok(meta) = file.metadata() else {
        return ChromeImportProbe::NotFound;
    };
    if !meta.is_file() {
        return ChromeImportProbe::NotFound;
    }

    let mut header = [0u8; 16];
    if let Err(err) = file.read(&mut header) {
        return classify_probe_io_error(path, &err);
    }

    let profile_name = derive_chrome_profile_name(path);
    ChromeImportProbe::Available {
        path: path.to_path_buf(),
        profile_name,
        size_bytes: meta.len(),
        encrypted_v10: cfg!(target_os = "macos") || cfg!(target_os = "linux"),
    }
}

fn derive_chrome_profile_name(path: &Path) -> String {
    let Some(parent) = path.parent() else {
        return "Default".to_string();
    };
    let parent_name = parent
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if parent_name.eq_ignore_ascii_case("Network") {
        parent
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Default".to_string())
    } else if !parent_name.is_empty() {
        parent_name
    } else {
        "Default".to_string()
    }
}

pub fn default_chrome_cookie_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let profiles = ["Default", "Profile 1", "Profile 2", "Profile 3"];
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let base = home.join("Library/Application Support/Google/Chrome");
            for prof in profiles {
                candidates.push(base.join(prof).join("Cookies"));
            }
        }
    } else if cfg!(target_os = "windows") {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            let base = local_app_data.join("Google/Chrome/User Data");
            for prof in profiles {
                candidates.push(base.join(prof).join("Network/Cookies"));
                candidates.push(base.join(prof).join("Cookies"));
            }
        }
    } else if cfg!(target_os = "ios") {
        // iOS App Sandbox has no desktop Chrome cookie stores.
    } else if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let chrome_base = home.join(".config/google-chrome");
        for prof in profiles {
            candidates.push(chrome_base.join(prof).join("Cookies"));
        }
        candidates.push(home.join(".config/chromium/Default/Cookies"));
    }
    candidates
}

pub fn probe_local_chrome_cookies() -> ChromeImportProbe {
    let mut first_denied = None;
    for candidate in default_chrome_cookie_candidates() {
        match probe_chrome_cookies_at(&candidate) {
            avail @ ChromeImportProbe::Available { .. } => return avail,
            denied @ ChromeImportProbe::PermissionDeniedTcc { .. } => {
                if first_denied.is_none() {
                    first_denied = Some(denied);
                }
            }
            ChromeImportProbe::NotFound => {}
        }
    }
    first_denied.unwrap_or(ChromeImportProbe::NotFound)
}

pub trait ChromeKeyProvider: Send + Sync {
    fn chrome_safe_storage_password(&self) -> Result<Zeroizing<String>, BotBrowserError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopChromeKeyProvider;

impl ChromeKeyProvider for NoopChromeKeyProvider {
    fn chrome_safe_storage_password(&self) -> Result<Zeroizing<String>, BotBrowserError> {
        Err(BotBrowserError::Keyring(
            "Chrome Safe Storage key provider disabled (NoopChromeKeyProvider)".to_string(),
        ))
    }
}

#[derive(Clone)]
pub struct StaticChromeKeyProvider(Zeroizing<String>);

impl StaticChromeKeyProvider {
    pub fn new(password: impl Into<String>) -> Self {
        Self(Zeroizing::new(password.into()))
    }
}

impl std::fmt::Debug for StaticChromeKeyProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StaticChromeKeyProvider")
            .field(&"[REDACTED]")
            .finish()
    }
}

impl ChromeKeyProvider for StaticChromeKeyProvider {
    fn chrome_safe_storage_password(&self) -> Result<Zeroizing<String>, BotBrowserError> {
        Ok(self.0.clone())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemChromeKeyProvider;

impl ChromeKeyProvider for SystemChromeKeyProvider {
    fn chrome_safe_storage_password(&self) -> Result<Zeroizing<String>, BotBrowserError> {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            if std::env::var("GPUI_OFFLINE_TEST").ok().as_deref() == Some("1") {
                return Err(BotBrowserError::Keyring(
                    "SystemChromeKeyProvider disabled when GPUI_OFFLINE_TEST=1".to_string(),
                ));
            }
            let candidates = [
                ("Chrome Safe Storage", "Chrome"),
                ("Chromium Safe Storage", "Chromium"),
            ];
            let mut last_err = None;
            for (service, user) in candidates {
                match keyring::Entry::new(service, user) {
                    Ok(entry) => match entry.get_password() {
                        Ok(pw) if !pw.is_empty() => return Ok(Zeroizing::new(pw)),
                        Ok(_) => {}
                        Err(e) => last_err = Some(e.to_string()),
                    },
                    Err(e) => last_err = Some(e.to_string()),
                }
            }
            Err(BotBrowserError::Keyring(format!(
                "failed retrieving Chrome Safe Storage key: {}",
                last_err.unwrap_or_else(|| "entry not found".to_string())
            )))
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Err(BotBrowserError::Keyring(
                "Chrome Safe Storage keyring unavailable inside the iOS App Sandbox; use JSON/Netscape cookie import or E2EE Vault profile sync".to_string(),
            ))
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChromeImportOptions {
    pub domain_filter: Option<String>,
    pub include_expired: bool,
    pub skip_undecryptable: bool,
}

#[derive(Clone)]
pub struct ChromeImportReport {
    pub cookies: Vec<BrowserCookie>,
    pub probe: ChromeImportProbe,
    pub imported_count: usize,
    pub skipped_encrypted_count: usize,
    pub skipped_expired_count: usize,
    pub used_key_provider: bool,
}

impl std::fmt::Debug for ChromeImportReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChromeImportReport")
            .field("cookies_count", &self.cookies.len())
            .field("probe", &self.probe)
            .field("imported_count", &self.imported_count)
            .field("skipped_encrypted_count", &self.skipped_encrypted_count)
            .field("skipped_expired_count", &self.skipped_expired_count)
            .field("used_key_provider", &self.used_key_provider)
            .finish()
    }
}

impl Zeroize for ChromeImportReport {
    fn zeroize(&mut self) {
        for c in &mut self.cookies {
            c.zeroize();
        }
        self.cookies.clear();
        self.imported_count.zeroize();
        self.skipped_encrypted_count.zeroize();
        self.skipped_expired_count.zeroize();
        self.used_key_provider.zeroize();
    }
}

impl Drop for ChromeImportReport {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for ChromeImportReport {}

pub fn import_cookies_from_chrome_sqlite(
    db_path: &Path,
    key_provider: &dyn ChromeKeyProvider,
    options: &ChromeImportOptions,
) -> Result<ChromeImportReport, BotBrowserError> {
    let probe = probe_chrome_cookies_at(db_path);
    match &probe {
        ChromeImportProbe::Available { .. } => {}
        ChromeImportProbe::PermissionDeniedTcc { guidance, .. } => {
            return Err(BotBrowserError::PermissionDenied(guidance.clone()));
        }
        ChromeImportProbe::NotFound => {
            return Ok(ChromeImportReport {
                cookies: Vec::new(),
                probe,
                imported_count: 0,
                skipped_encrypted_count: 0,
                skipped_expired_count: 0,
                used_key_provider: false,
            });
        }
    }

    let tmp_dir = TempDir::new()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tmp_dir.path(), fs::Permissions::from_mode(0o700))?;
    }

    let copied_db = tmp_dir.path().join("Cookies");
    fs::copy(db_path, &copied_db)?;
    for suffix in ["-wal", "-shm"] {
        let side_src = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        if side_src.is_file() {
            let side_dst = tmp_dir.path().join(format!("Cookies{suffix}"));
            let _ = fs::copy(&side_src, &side_dst);
        }
    }

    let conn = Connection::open_with_flags(
        &copied_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| {
        BotBrowserError::CookieImport(format!("failed opening Chrome Cookies sqlite: {e}"))
    })?;

    let meta_version: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);

    let mut stmt = conn
        .prepare(
            "SELECT host_key, name, value, encrypted_value, path, expires_utc, is_secure, is_httponly, samesite
             FROM cookies",
        )
        .map_err(|e| {
            BotBrowserError::CookieImport(format!("failed querying Chrome cookies table: {e}"))
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ChromeCookieRow {
                host_key: row.get(0)?,
                name: row.get(1)?,
                value: row.get(2)?,
                encrypted_value: row.get(3)?,
                path: row.get(4)?,
                expires_utc: row.get(5)?,
                is_secure: row.get::<_, i64>(6)? != 0,
                is_httponly: row.get::<_, i64>(7)? != 0,
                samesite: row.get(8)?,
            })
        })
        .map_err(|e| {
            BotBrowserError::CookieImport(format!("failed reading Chrome cookies rows: {e}"))
        })?;

    let now_unix_secs = Utc::now().timestamp();
    let mut cached_aes_key: Option<Zeroizing<[u8; 16]>> = None;
    let mut used_key_provider = false;
    let mut skipped_encrypted_count = 0usize;
    let mut skipped_expired_count = 0usize;
    let mut cookies = Vec::new();

    for row_res in rows {
        let row = row_res.map_err(|e| {
            BotBrowserError::CookieImport(format!("failed decoding Chrome cookie row: {e}"))
        })?;

        if let Some(filter) = options
            .domain_filter
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let norm_filter = filter.trim_start_matches('.').to_ascii_lowercase();
            let norm_host = row.host_key.trim().trim_start_matches('.').to_ascii_lowercase();
            if norm_host != norm_filter && !norm_host.ends_with(&format!(".{norm_filter}")) {
                continue;
            }
        }

        let expires = convert_chrome_expires_utc(row.expires_utc);
        if !options.include_expired && expires.is_some_and(|e| e <= now_unix_secs) {
            skipped_expired_count += 1;
            continue;
        }

        let resolved_value: Zeroizing<String> = if !row.value.is_empty() {
            Zeroizing::new(row.value)
        } else if row.encrypted_value.starts_with(b"v10") || row.encrypted_value.starts_with(b"v11")
        {
            if cached_aes_key.is_none() {
                match key_provider.chrome_safe_storage_password() {
                    Ok(pw) => {
                        used_key_provider = true;
                        let mut derived = Zeroizing::new([0u8; 16]);
                        pbkdf2_hmac::<Sha1>(
                            pw.as_bytes(),
                            CHROME_PBKDF2_SALT,
                            CHROME_PBKDF2_ITERATIONS,
                            &mut *derived,
                        );
                        cached_aes_key = Some(derived);
                    }
                    Err(e) => {
                        if options.skip_undecryptable {
                            skipped_encrypted_count += 1;
                            continue;
                        }
                        return Err(e);
                    }
                }
            }

            let aes_key = cached_aes_key.as_ref().expect("just initialized");
            match decrypt_chrome_v10_cookie(
                aes_key,
                &row.encrypted_value[3..],
                &row.host_key,
                meta_version,
            ) {
                Ok(plain) => plain,
                Err(e) => {
                    if options.skip_undecryptable {
                        skipped_encrypted_count += 1;
                        continue;
                    }
                    return Err(e);
                }
            }
        } else if !row.encrypted_value.is_empty() {
            match String::from_utf8(row.encrypted_value) {
                Ok(s) => Zeroizing::new(s),
                Err(_) if options.skip_undecryptable => {
                    skipped_encrypted_count += 1;
                    continue;
                }
                Err(_) => {
                    return Err(BotBrowserError::CookieImport(format!(
                        "undecryptable non-v10 cookie payload for '{}'",
                        row.name
                    )));
                }
            }
        } else {
            Zeroizing::new(String::new())
        };

        let same_site = match row.samesite {
            0 => Some(SameSitePolicy::None),
            1 => Some(SameSitePolicy::Lax),
            2 => Some(SameSitePolicy::Strict),
            _ => None,
        };

        let mut cookie = BrowserCookie {
            name: row.name,
            value: resolved_value.to_string(),
            domain: row.host_key.clone(),
            path: row.path,
            secure: row.is_secure,
            http_only: row.is_httponly,
            same_site,
            expires,
            host_only: !row.host_key.starts_with('.'),
            session: expires.is_none(),
        };
        validate_and_normalize_cookie(&mut cookie)?;
        cookies.push(cookie);
    }

    let imported_count = cookies.len();
    Ok(ChromeImportReport {
        cookies,
        probe,
        imported_count,
        skipped_encrypted_count,
        skipped_expired_count,
        used_key_provider,
    })
}

struct ChromeCookieRow {
    host_key: String,
    name: String,
    value: String,
    encrypted_value: Vec<u8>,
    path: String,
    expires_utc: i64,
    is_secure: bool,
    is_httponly: bool,
    samesite: i64,
}

fn convert_chrome_expires_utc(expires_utc: i64) -> Option<i64> {
    if expires_utc <= 0 {
        return None;
    }
    if expires_utc > 10_000_000_000_000 {
        let unix_secs = (expires_utc / 1_000_000) - WEBKIT_UNIX_EPOCH_OFFSET_SECS;
        if unix_secs <= 0 {
            None
        } else {
            Some(unix_secs)
        }
    } else {
        Some(expires_utc)
    }
}

fn decrypt_chrome_v10_cookie(
    aes_key: &[u8; 16],
    ciphertext: &[u8],
    host_key: &str,
    meta_version: i64,
) -> Result<Zeroizing<String>, BotBrowserError> {
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return Err(BotBrowserError::Crypto(
            "invalid Chrome v10 cookie ciphertext block length".to_string(),
        ));
    }

    let mut buf = Zeroizing::new(ciphertext.to_vec());
    let mut dec = cbc::Decryptor::<Aes128>::new_from_slices(aes_key, &CHROME_V10_IV).map_err(|e| {
        BotBrowserError::Crypto(format!("failed initializing AES-128-CBC decryptor: {e}"))
    })?;
    for chunk in buf.chunks_exact_mut(16) {
        dec.decrypt_block_mut(GenericArray::from_mut_slice(chunk));
    }

    let pad_len = *buf.last().unwrap_or(&0) as usize;
    if pad_len == 0 || pad_len > 16 || pad_len > buf.len() {
        return Err(BotBrowserError::Crypto(
            "invalid PKCS#7 padding in decrypted Chrome cookie".to_string(),
        ));
    }
    let unpadded_len = buf.len() - pad_len;
    if !buf[unpadded_len..].iter().all(|&b| b as usize == pad_len) {
        return Err(BotBrowserError::Crypto(
            "corrupted PKCS#7 padding in decrypted Chrome cookie".to_string(),
        ));
    }
    buf.truncate(unpadded_len);

    let mut payload_slice = buf.as_slice();
    if meta_version >= 24 && payload_slice.len() >= 32 {
        let host_hash = Sha256::digest(host_key.as_bytes());
        if payload_slice[..32] == host_hash[..] {
            payload_slice = &payload_slice[32..];
        }
    }

    let decoded = std::str::from_utf8(payload_slice).map_err(|_| {
        BotBrowserError::Crypto("decrypted Chrome cookie value is not valid UTF-8".to_string())
    })?;
    Ok(Zeroizing::new(decoded.to_string()))
}
