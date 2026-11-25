use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use chrono::Utc;
use hkdf::Hkdf;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::totp::generate_totp_with_defaults_at;
use crate::types::{
    BotBrowserError, CredentialMeta, DecryptedSecret, NormalizedOrigin, ReusePolicy,
};

pub const DEFAULT_KEYRING_SERVICE: &str = "com.gpui-telegram.bot-browser";
pub const DEFAULT_KEYRING_USER: &str = "vault-master-key-v1";
const VAULT_HKDF_SALT: &[u8] = b"bot-browser-vault-hkdf-v1";

pub trait MasterKeyStore: Send + Sync {
    fn load_or_create_key(&self) -> Result<Zeroizing<[u8; 32]>, BotBrowserError>;
}

/// OS Keychain / Credential Manager backed 256-bit master key store.
pub struct SystemKeyringMasterKeyStore {
    service: String,
    user: String,
    lock: Mutex<()>,
}

impl SystemKeyringMasterKeyStore {
    pub fn new(service: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            user: user.into(),
            lock: Mutex::new(()),
        }
    }
}

impl Default for SystemKeyringMasterKeyStore {
    fn default() -> Self {
        Self::new(DEFAULT_KEYRING_SERVICE, DEFAULT_KEYRING_USER)
    }
}

impl MasterKeyStore for SystemKeyringMasterKeyStore {
    fn load_or_create_key(&self) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
        // iOS target_os adaptation: `keyring` crate is only declared for macOS/Windows/Linux
        // in Cargo.toml; on iOS, Swift's native SecItem Keychain runner owns master key storage
        // and injects keys via MasterKeyStore / FileMasterKeyStore.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            let _guard = self.lock.lock();
            let entry = keyring::Entry::new(&self.service, &self.user)
                .map_err(|e| BotBrowserError::Keyring(format!("failed to open keyring entry: {e}")))?;

            match entry.get_password() {
                Ok(encoded) => {
                    let encoded = Zeroizing::new(encoded);
                    let decoded = Zeroizing::new(hex::decode(encoded.trim()).map_err(|e| {
                        BotBrowserError::Keyring(format!("corrupt keyring master key encoding: {e}"))
                    })?);
                    if decoded.len() != 32 {
                        return Err(BotBrowserError::Keyring(format!(
                            "unexpected keyring master key length: {} bytes",
                            decoded.len()
                        )));
                    }
                    let mut key = Zeroizing::new([0u8; 32]);
                    key.copy_from_slice(&decoded);
                    Ok(key)
                }
                Err(keyring::Error::NoEntry) => {
                    let mut key = Zeroizing::new([0u8; 32]);
                    getrandom::getrandom(&mut *key).map_err(|e| {
                        BotBrowserError::Crypto(format!("failed to generate master key entropy: {e}"))
                    })?;
                    let encoded = Zeroizing::new(hex::encode(&*key));
                    entry.set_password(&encoded).map_err(|e| {
                        BotBrowserError::Keyring(format!("failed to store master key in keyring: {e}"))
                    })?;
                    Ok(key)
                }
                Err(e) => Err(BotBrowserError::Keyring(format!(
                    "failed to read keyring master key: {e}"
                ))),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = (&self.service, &self.user, &self.lock);
            Err(BotBrowserError::Keyring(
                "desktop OS keyring unavailable on this target; inject key via iOS Keychain runner (MasterKeyStore / FileMasterKeyStore)".into(),
            ))
        }
    }
}

/// File-backed 256-bit master key store with strict `0o600` permissions on Unix.
pub struct FileMasterKeyStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileMasterKeyStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl MasterKeyStore for FileMasterKeyStore {
    fn load_or_create_key(&self) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
        let _guard = self.lock.lock();
        if self.path.exists() {
            let mut file = OpenOptions::new().read(true).open(&self.path)?;
            let mut raw = Zeroizing::new(Vec::new());
            file.read_to_end(&mut raw)?;

            if raw.len() == 32 {
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&raw);
                return Ok(key);
            }

            if let Ok(as_str) = std::str::from_utf8(&raw) {
                let decoded = Zeroizing::new(hex::decode(as_str.trim()).map_err(|e| {
                    BotBrowserError::Keyring(format!("corrupt file master key: {e}"))
                })?);
                if decoded.len() == 32 {
                    let mut key = Zeroizing::new([0u8; 32]);
                    key.copy_from_slice(&decoded);
                    return Ok(key);
                }
            }

            return Err(BotBrowserError::Keyring(format!(
                "invalid master key file size ({} bytes) at {}",
                raw.len(),
                self.path.display()
            )));
        }

        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let mut key = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(&mut *key).map_err(|e| {
            BotBrowserError::Crypto(format!("failed to generate master key entropy: {e}"))
        })?;

        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let tmp_path = parent.join(format!(
            ".{}.{}.tmp",
            self.path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("vault-master.key"),
            Uuid::new_v4()
        ));

        let write_result: Result<(), BotBrowserError> = (|| {
            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut file = opts.open(&tmp_path)?;
            file.write_all(&*key)?;
            file.sync_all()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
            }
            fs::rename(&tmp_path, &self.path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
            }
            Ok(())
        })();

        if let Err(err) = write_result {
            let _ = fs::remove_file(&tmp_path);
            return Err(err);
        }

        Ok(key)
    }
}

/// In-memory master key store for unit & integration tests.
pub struct MemoryMasterKeyStore {
    key: Mutex<Option<Zeroizing<[u8; 32]>>>,
}

impl MemoryMasterKeyStore {
    pub fn new() -> Self {
        Self {
            key: Mutex::new(None),
        }
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            key: Mutex::new(Some(Zeroizing::new(bytes))),
        }
    }
}

impl Default for MemoryMasterKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MasterKeyStore for MemoryMasterKeyStore {
    fn load_or_create_key(&self) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
        let mut guard = self.key.lock();
        if let Some(existing) = guard.as_ref() {
            return Ok(existing.clone());
        }
        let mut key = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(&mut *key).map_err(|e| {
            BotBrowserError::Crypto(format!("failed to generate master key entropy: {e}"))
        })?;
        *guard = Some(key.clone());
        Ok(key)
    }
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct SecretPayload {
    username: String,
    password: String,
    totp_secret: Option<String>,
}

/// Decrypted credential payload for client-side E2EE sync export/import.
/// Never serialized to AI tool responses and redacted in `Debug`.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct VaultSyncCredentialPayload {
    pub credential_id: String,
    pub origin: String,
    pub account_hint: String,
    #[zeroize(skip)]
    pub reuse_policy: ReusePolicy,
    pub username: String,
    pub password: String,
    pub totp_secret: Option<String>,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    #[serde(default)]
    pub deleted: bool,
}

impl std::fmt::Debug for VaultSyncCredentialPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultSyncCredentialPayload")
            .field("credential_id", &self.credential_id)
            .field("origin", &self.origin)
            .field("account_hint", &self.account_hint)
            .field("reuse_policy", &self.reuse_policy)
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .field("has_totp", &self.totp_secret.is_some())
            .field("created_at_ms", &self.created_at_ms)
            .field("last_used_at_ms", &self.last_used_at_ms)
            .field("deleted", &self.deleted)
            .finish()
    }
}

pub struct CredentialVault {
    conn: Mutex<Connection>,
    master_key: Zeroizing<[u8; 32]>,
}

impl CredentialVault {
    pub fn open(
        db_path: impl AsRef<Path>,
        key_store: &dyn MasterKeyStore,
    ) -> Result<Self, BotBrowserError> {
        let db_path = db_path.as_ref();
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(db_path)
            .map_err(|e| BotBrowserError::Storage(format!("failed to open vault db: {e}")))?;
        Self::init_with_connection(conn, key_store)
    }

    pub fn open_in_memory(key_store: &dyn MasterKeyStore) -> Result<Self, BotBrowserError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| BotBrowserError::Storage(format!("failed to open memory db: {e}")))?;
        Self::init_with_connection(conn, key_store)
    }

    fn init_with_connection(
        conn: Connection,
        key_store: &dyn MasterKeyStore,
    ) -> Result<Self, BotBrowserError> {
        let master_key = key_store.load_or_create_key()?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS vault_credentials (
                 credential_id TEXT PRIMARY KEY NOT NULL,
                 origin TEXT NOT NULL,
                 account_hint TEXT NOT NULL,
                 has_totp INTEGER NOT NULL,
                 reuse_policy TEXT NOT NULL,
                 nonce BLOB NOT NULL,
                 ciphertext BLOB NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 last_used_at_ms INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_vault_credentials_origin
                 ON vault_credentials(origin);
             CREATE TABLE IF NOT EXISTS vault_browser_profiles (
                 profile_key TEXT PRIMARY KEY NOT NULL,
                 origin TEXT,
                 nonce BLOB NOT NULL,
                 ciphertext BLOB NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );",
        )
        .map_err(|e| BotBrowserError::Storage(format!("failed to initialize vault schema: {e}")))?;

        Ok(Self {
            conn: Mutex::new(conn),
            master_key,
        })
    }

    fn encrypt_secret_payload(
        &self,
        credential_id: &str,
        origin: &NormalizedOrigin,
        payload: &SecretPayload,
    ) -> Result<([u8; 12], Vec<u8>), BotBrowserError> {
        let plaintext = Zeroizing::new(serde_json::to_vec(payload).map_err(|e| {
            BotBrowserError::Crypto(format!("failed to serialize credential payload: {e}"))
        })?);
        let subkey = derive_credential_key(&self.master_key, credential_id)?;
        let aad = build_aad(credential_id, origin.as_str());

        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes).map_err(|e| {
            BotBrowserError::Crypto(format!("failed to generate AEAD nonce: {e}"))
        })?;

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*subkey));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|e| BotBrowserError::Crypto(format!("credential encryption failed: {e}")))?;
        Ok((nonce_bytes, ciphertext))
    }

    fn decrypt_row_payload(
        &self,
        credential_id: &str,
        canonical_origin: &str,
        nonce_bytes: &[u8],
        ciphertext: &[u8],
    ) -> Result<SecretPayload, BotBrowserError> {
        if nonce_bytes.len() != 12 {
            return Err(BotBrowserError::Crypto(format!(
                "invalid stored nonce length: {} bytes",
                nonce_bytes.len()
            )));
        }

        let subkey = derive_credential_key(&self.master_key, credential_id)?;
        let aad = build_aad(credential_id, canonical_origin);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*subkey));

        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(nonce_bytes),
                    Payload {
                        msg: ciphertext,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| {
                    BotBrowserError::Crypto(
                        "AEAD authentication failed (origin binding or ciphertext tampered)"
                            .to_string(),
                    )
                })?,
        );
        drop(subkey);

        serde_json::from_slice(&plaintext).map_err(|e| {
            BotBrowserError::Crypto(format!("failed to decode decrypted credential payload: {e}"))
        })
    }

    pub fn store_credential(
        &self,
        origin_raw: &str,
        username: &str,
        password: &str,
        totp_secret: Option<&str>,
        account_hint: Option<&str>,
        reuse_policy: Option<ReusePolicy>,
    ) -> Result<CredentialMeta, BotBrowserError> {
        let origin = NormalizedOrigin::parse(origin_raw)?;
        let policy = reuse_policy.unwrap_or_else(|| ReusePolicy::default_for_origin(&origin));
        let credential_id = format!("cred_{}", Uuid::new_v4().simple());
        let created_at_ms = Utc::now().timestamp_millis();

        let normalized_totp = totp_secret
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        let has_totp = normalized_totp.is_some();

        let resolved_hint = account_hint
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| mask_account_hint(username));

        let payload = SecretPayload {
            username: username.to_string(),
            password: password.to_string(),
            totp_secret: normalized_totp,
        };
        let (nonce_bytes, ciphertext) =
            self.encrypt_secret_payload(&credential_id, &origin, &payload)?;
        drop(payload);

        let policy_str = policy_to_db_str(policy);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO vault_credentials (
                credential_id, origin, account_hint, has_totp, reuse_policy,
                nonce, ciphertext, created_at_ms, last_used_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
            params![
                credential_id,
                origin.as_str(),
                resolved_hint,
                if has_totp { 1i64 } else { 0i64 },
                policy_str,
                nonce_bytes.as_slice(),
                ciphertext,
                created_at_ms,
            ],
        )
        .map_err(|e| BotBrowserError::Storage(format!("failed to insert credential: {e}")))?;

        Ok(CredentialMeta {
            credential_id,
            origin: origin.as_str().to_string(),
            account_hint: resolved_hint,
            has_totp,
            reuse_policy: policy,
            created_at_ms,
            last_used_at_ms: None,
        })
    }

    pub fn list_for_origin(&self, origin_raw: &str) -> Result<Vec<CredentialMeta>, BotBrowserError> {
        let origin = NormalizedOrigin::parse(origin_raw)?;
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT credential_id, origin, account_hint, has_totp, reuse_policy, created_at_ms, last_used_at_ms
                 FROM vault_credentials
                 WHERE origin = ?1
                 ORDER BY created_at_ms DESC, credential_id ASC",
            )
            .map_err(|e| BotBrowserError::Storage(format!("failed to prepare list query: {e}")))?;

        let rows = stmt
            .query_map(params![origin.as_str()], row_to_meta)
            .map_err(|e| BotBrowserError::Storage(format!("failed to query credentials: {e}")))?;

        let mut metas = Vec::new();
        for row in rows {
            metas.push(row.map_err(|e| {
                BotBrowserError::Storage(format!("failed to decode credential row: {e}"))
            })?);
        }
        Ok(metas)
    }

    pub fn get_meta(&self, credential_id: &str) -> Result<CredentialMeta, BotBrowserError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT credential_id, origin, account_hint, has_totp, reuse_policy, created_at_ms, last_used_at_ms
             FROM vault_credentials
             WHERE credential_id = ?1",
            params![credential_id],
            row_to_meta,
        )
        .optional()
        .map_err(|e| BotBrowserError::Storage(format!("failed to query credential meta: {e}")))?
        .ok_or_else(|| BotBrowserError::CredentialNotFound(credential_id.to_string()))
    }

    pub fn decrypt_for_origin(
        &self,
        credential_id: &str,
        actual_page_origin: &str,
        actual_frame_origin: Option<&str>,
    ) -> Result<DecryptedSecret, BotBrowserError> {
        let parsed_page_origin = NormalizedOrigin::parse(actual_page_origin)?;
        let parsed_frame_origin = actual_frame_origin
            .map(NormalizedOrigin::parse)
            .transpose()?;

        let (stored_origin, nonce_bytes, ciphertext): (String, Vec<u8>, Vec<u8>) = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT origin, nonce, ciphertext
                 FROM vault_credentials
                 WHERE credential_id = ?1",
                params![credential_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| BotBrowserError::Storage(format!("failed to load encrypted row: {e}")))?
            .ok_or_else(|| BotBrowserError::CredentialNotFound(credential_id.to_string()))?
        };

        if stored_origin != parsed_page_origin.as_str() {
            return Err(BotBrowserError::OriginMismatch {
                expected: stored_origin,
                actual: parsed_page_origin.as_str().to_string(),
            });
        }

        if let Some(frame_origin) = parsed_frame_origin.as_ref() {
            if frame_origin.as_str() != stored_origin {
                return Err(BotBrowserError::FrameOriginMismatch {
                    expected: stored_origin,
                    actual: frame_origin.as_str().to_string(),
                });
            }
        }

        let mut payload = self.decrypt_row_payload(
            credential_id,
            parsed_page_origin.as_str(),
            &nonce_bytes,
            &ciphertext,
        )?;

        let secret = DecryptedSecret {
            username: Zeroizing::new(std::mem::take(&mut payload.username)),
            password: Zeroizing::new(std::mem::take(&mut payload.password)),
            totp_secret: payload.totp_secret.take().map(Zeroizing::new),
        };
        drop(payload);

        let now_ms = Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE vault_credentials SET last_used_at_ms = ?1 WHERE credential_id = ?2",
            params![now_ms, credential_id],
        )
        .map_err(|e| BotBrowserError::Storage(format!("failed to update last_used_at_ms: {e}")))?;

        Ok(secret)
    }

    pub fn compute_totp_for_origin(
        &self,
        credential_id: &str,
        actual_page_origin: &str,
        actual_frame_origin: Option<&str>,
        now_unix_secs: u64,
    ) -> Result<Zeroizing<String>, BotBrowserError> {
        let secret =
            self.decrypt_for_origin(credential_id, actual_page_origin, actual_frame_origin)?;
        let totp_secret = secret.totp_secret.as_deref().ok_or_else(|| {
            BotBrowserError::Totp(format!(
                "credential `{credential_id}` does not have a TOTP secret"
            ))
        })?;
        generate_totp_with_defaults_at(totp_secret, now_unix_secs)
    }

    pub fn update_reuse_policy(
        &self,
        credential_id: &str,
        policy: ReusePolicy,
    ) -> Result<(), BotBrowserError> {
        let policy_str = policy_to_db_str(policy);
        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "UPDATE vault_credentials SET reuse_policy = ?1 WHERE credential_id = ?2",
                params![policy_str, credential_id],
            )
            .map_err(|e| {
                BotBrowserError::Storage(format!("failed to update reuse_policy: {e}"))
            })?;
        if affected == 0 {
            return Err(BotBrowserError::CredentialNotFound(
                credential_id.to_string(),
            ));
        }
        Ok(())
    }

    pub fn delete_credential(&self, credential_id: &str) -> Result<(), BotBrowserError> {
        if !self.delete_credential_if_exists(credential_id)? {
            return Err(BotBrowserError::CredentialNotFound(
                credential_id.to_string(),
            ));
        }
        Ok(())
    }

    pub fn delete_credential_if_exists(
        &self,
        credential_id: &str,
    ) -> Result<bool, BotBrowserError> {
        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "DELETE FROM vault_credentials WHERE credential_id = ?1",
                params![credential_id],
            )
            .map_err(|e| {
                BotBrowserError::Storage(format!("failed to delete credential: {e}"))
            })?;
        Ok(affected > 0)
    }

    /// Exports one credential decrypted for client-side E2EE sync without mutating `last_used_at_ms`.
    pub fn export_for_sync(
        &self,
        credential_id: &str,
    ) -> Result<VaultSyncCredentialPayload, BotBrowserError> {
        let row: (String, String, String, Vec<u8>, Vec<u8>, i64, Option<i64>) = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT origin, account_hint, reuse_policy, nonce, ciphertext, created_at_ms, last_used_at_ms
                 FROM vault_credentials
                 WHERE credential_id = ?1",
                params![credential_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| BotBrowserError::Storage(format!("failed to load credential for sync: {e}")))?
            .ok_or_else(|| BotBrowserError::CredentialNotFound(credential_id.to_string()))?
        };

        let (origin, account_hint, policy_raw, nonce, ciphertext, created_at_ms, last_used_at_ms) =
            row;
        let reuse_policy = db_str_to_policy(&policy_raw).map_err(|e| {
            BotBrowserError::Storage(format!("invalid stored reuse_policy: {e}"))
        })?;
        let mut payload =
            self.decrypt_row_payload(credential_id, &origin, &nonce, &ciphertext)?;

        Ok(VaultSyncCredentialPayload {
            credential_id: credential_id.to_string(),
            origin,
            account_hint,
            reuse_policy,
            username: std::mem::take(&mut payload.username),
            password: std::mem::take(&mut payload.password),
            totp_secret: payload.totp_secret.take(),
            created_at_ms,
            last_used_at_ms,
            deleted: false,
        })
    }

    /// Exports all stored credentials decrypted for client-side E2EE sync without mutating `last_used_at_ms`.
    pub fn export_all_for_sync(&self) -> Result<Vec<VaultSyncCredentialPayload>, BotBrowserError> {
        let rows: Vec<(String, String, String, String, Vec<u8>, Vec<u8>, i64, Option<i64>)> = {
            let conn = self.conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT credential_id, origin, account_hint, reuse_policy, nonce, ciphertext, created_at_ms, last_used_at_ms
                     FROM vault_credentials
                     ORDER BY created_at_ms ASC, credential_id ASC",
                )
                .map_err(|e| {
                    BotBrowserError::Storage(format!("failed to prepare sync export query: {e}"))
                })?;

            let mapped = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                })
                .map_err(|e| {
                    BotBrowserError::Storage(format!("failed to execute sync export query: {e}"))
                })?;

            let mut collected = Vec::new();
            for row in mapped {
                collected.push(row.map_err(|e| {
                    BotBrowserError::Storage(format!("failed to decode sync export row: {e}"))
                })?);
            }
            collected
        };

        let mut out = Vec::with_capacity(rows.len());
        for (
            credential_id,
            origin,
            account_hint,
            policy_raw,
            nonce,
            ciphertext,
            created_at_ms,
            last_used_at_ms,
        ) in rows
        {
            let reuse_policy = db_str_to_policy(&policy_raw).map_err(|e| {
                BotBrowserError::Storage(format!("invalid stored reuse_policy: {e}"))
            })?;
            let mut payload =
                self.decrypt_row_payload(&credential_id, &origin, &nonce, &ciphertext)?;
            out.push(VaultSyncCredentialPayload {
                credential_id,
                origin,
                account_hint,
                reuse_policy,
                username: std::mem::take(&mut payload.username),
                password: std::mem::take(&mut payload.password),
                totp_secret: payload.totp_secret.take(),
                created_at_ms,
                last_used_at_ms,
                deleted: false,
            });
        }
        Ok(out)
    }

    /// Merges/imports a decrypted E2EE sync record by `credential_id`, re-encrypting locally
    /// using the active `VaultRootKey` and canonical `origin` AAD binding.
    /// Returns `Ok(true)` if the local vault state changed, or `Ok(false)` if unchanged.
    pub fn import_from_sync(
        &self,
        record: &VaultSyncCredentialPayload,
    ) -> Result<bool, BotBrowserError> {
        let credential_id = record.credential_id.trim();
        if credential_id.is_empty() {
            return Err(BotBrowserError::Storage(
                "sync credential_id cannot be empty".to_string(),
            ));
        }

        if record.deleted {
            return self.delete_credential_if_exists(credential_id);
        }

        let origin = NormalizedOrigin::parse(&record.origin)?;
        let normalized_totp = record
            .totp_secret
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        let has_totp = normalized_totp.is_some();
        let resolved_hint = if record.account_hint.trim().is_empty() {
            mask_account_hint(&record.username)
        } else {
            record.account_hint.trim().to_string()
        };

        let existing: Option<(String, String, String, Vec<u8>, Vec<u8>, i64, Option<i64>)> = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT origin, account_hint, reuse_policy, nonce, ciphertext, created_at_ms, last_used_at_ms
                 FROM vault_credentials
                 WHERE credential_id = ?1",
                params![credential_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| BotBrowserError::Storage(format!("failed to check existing credential: {e}")))?
        };

        if let Some((
            existing_origin,
            existing_hint,
            existing_policy_raw,
            existing_nonce,
            existing_ct,
            existing_created_at_ms,
            existing_last_used_at_ms,
        )) = existing
        {
            let existing_policy = db_str_to_policy(&existing_policy_raw).map_err(|e| {
                BotBrowserError::Storage(format!("invalid stored reuse_policy: {e}"))
            })?;
            let existing_payload = self.decrypt_row_payload(
                credential_id,
                &existing_origin,
                &existing_nonce,
                &existing_ct,
            )?;

            let merged_last_used = match (existing_last_used_at_ms, record.last_used_at_ms) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };

            let identical = existing_origin == origin.as_str()
                && existing_hint == resolved_hint
                && existing_policy == record.reuse_policy
                && existing_payload.username == record.username
                && existing_payload.password == record.password
                && existing_payload.totp_secret == normalized_totp
                && existing_created_at_ms == record.created_at_ms
                && existing_last_used_at_ms == merged_last_used;
            drop(existing_payload);

            if identical {
                return Ok(false);
            }

            let payload = SecretPayload {
                username: record.username.clone(),
                password: record.password.clone(),
                totp_secret: normalized_totp,
            };
            let (nonce_bytes, ciphertext) =
                self.encrypt_secret_payload(credential_id, &origin, &payload)?;
            drop(payload);

            let policy_str = policy_to_db_str(record.reuse_policy);
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE vault_credentials
                 SET origin = ?1, account_hint = ?2, has_totp = ?3, reuse_policy = ?4,
                     nonce = ?5, ciphertext = ?6, created_at_ms = ?7, last_used_at_ms = ?8
                 WHERE credential_id = ?9",
                params![
                    origin.as_str(),
                    resolved_hint,
                    if has_totp { 1i64 } else { 0i64 },
                    policy_str,
                    nonce_bytes.as_slice(),
                    ciphertext,
                    record.created_at_ms,
                    merged_last_used,
                    credential_id,
                ],
            )
            .map_err(|e| BotBrowserError::Storage(format!("failed to merge sync credential: {e}")))?;
            return Ok(true);
        }

        let payload = SecretPayload {
            username: record.username.clone(),
            password: record.password.clone(),
            totp_secret: normalized_totp,
        };
        let (nonce_bytes, ciphertext) =
            self.encrypt_secret_payload(credential_id, &origin, &payload)?;
        drop(payload);

        let policy_str = policy_to_db_str(record.reuse_policy);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO vault_credentials (
                credential_id, origin, account_hint, has_totp, reuse_policy,
                nonce, ciphertext, created_at_ms, last_used_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                credential_id,
                origin.as_str(),
                resolved_hint,
                if has_totp { 1i64 } else { 0i64 },
                policy_str,
                nonce_bytes.as_slice(),
                ciphertext,
                record.created_at_ms,
                record.last_used_at_ms,
            ],
        )
        .map_err(|e| BotBrowserError::Storage(format!("failed to insert sync credential: {e}")))?;

        Ok(true)
    }

    pub fn store_profile_snapshot(
        &self,
        profile_key: &str,
        origin: Option<&NormalizedOrigin>,
        bundle_bytes: &[u8],
    ) -> Result<(), BotBrowserError> {
        let key_trimmed = profile_key.trim();
        if key_trimmed.is_empty() {
            return Err(BotBrowserError::Storage(
                "profile_key cannot be empty".to_string(),
            ));
        }
        let origin_str = origin.map(|o| o.as_str()).unwrap_or("");
        let subkey = derive_profile_key(&self.master_key, key_trimmed)?;
        let aad = build_profile_aad(key_trimmed, origin_str);

        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes).map_err(|e| {
            BotBrowserError::Crypto(format!("failed to generate profile AEAD nonce: {e}"))
        })?;

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*subkey));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: bundle_bytes,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|e| {
                BotBrowserError::Crypto(format!("profile snapshot encryption failed: {e}"))
            })?;

        let now_ms = Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO vault_browser_profiles (profile_key, origin, nonce, ciphertext, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(profile_key) DO UPDATE SET
                 origin = excluded.origin,
                 nonce = excluded.nonce,
                 ciphertext = excluded.ciphertext,
                 updated_at_ms = excluded.updated_at_ms",
            params![
                key_trimmed,
                origin.map(|o| o.as_str()),
                nonce_bytes.as_slice(),
                ciphertext,
                now_ms,
            ],
        )
        .map_err(|e| {
            BotBrowserError::Storage(format!("failed to store profile snapshot: {e}"))
        })?;
        Ok(())
    }

    pub fn load_profile_snapshot(
        &self,
        profile_key: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, BotBrowserError> {
        let key_trimmed = profile_key.trim();
        if key_trimmed.is_empty() {
            return Err(BotBrowserError::Storage(
                "profile_key cannot be empty".to_string(),
            ));
        }
        let row: Option<(Option<String>, Vec<u8>, Vec<u8>)> = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT origin, nonce, ciphertext
                 FROM vault_browser_profiles
                 WHERE profile_key = ?1",
                params![key_trimmed],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| {
                BotBrowserError::Storage(format!("failed to query profile snapshot: {e}"))
            })?
        };

        let Some((stored_origin, nonce_bytes, ciphertext)) = row else {
            return Ok(None);
        };
        if let Some(expected_origin) = origin {
            if stored_origin.as_deref() != Some(expected_origin.as_str()) {
                return Err(BotBrowserError::OriginMismatch {
                    expected: expected_origin.as_str().to_string(),
                    actual: stored_origin.unwrap_or_default(),
                });
            }
        }
        if nonce_bytes.len() != 12 {
            return Err(BotBrowserError::Crypto(format!(
                "invalid profile nonce length {}, expected 12",
                nonce_bytes.len()
            )));
        }
        let origin_str = stored_origin.as_deref().unwrap_or("");
        let subkey = derive_profile_key(&self.master_key, key_trimmed)?;
        let aad = build_profile_aad(key_trimmed, origin_str);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*subkey));
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|e| {
                BotBrowserError::Crypto(format!("profile snapshot decryption failed: {e}"))
            })?;
        Ok(Some(Zeroizing::new(plaintext)))
    }

    pub fn delete_profile_snapshot(&self, profile_key: &str) -> Result<bool, BotBrowserError> {
        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "DELETE FROM vault_browser_profiles WHERE profile_key = ?1",
                params![profile_key.trim()],
            )
            .map_err(|e| {
                BotBrowserError::Storage(format!("failed to delete profile snapshot: {e}"))
            })?;
        Ok(affected > 0)
    }

    pub fn list_profile_keys(&self) -> Result<Vec<String>, BotBrowserError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT profile_key FROM vault_browser_profiles ORDER BY updated_at_ms DESC, profile_key ASC",
            )
            .map_err(|e| {
                BotBrowserError::Storage(format!("failed to prepare profile keys query: {e}"))
            })?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| {
                BotBrowserError::Storage(format!("failed to list profile keys: {e}"))
            })?;
        let mut keys = Vec::new();
        for row in rows {
            keys.push(row.map_err(|e| {
                BotBrowserError::Storage(format!("failed reading profile key row: {e}"))
            })?);
        }
        Ok(keys)
    }
}

fn derive_profile_key(
    master_key: &[u8; 32],
    profile_key: &str,
) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
    let hk = Hkdf::<Sha256>::new(Some(VAULT_HKDF_SALT), master_key);
    let mut subkey = Zeroizing::new([0u8; 32]);
    let info = format!("profile:{profile_key}");
    hk.expand(info.as_bytes(), &mut *subkey)
        .map_err(|e| BotBrowserError::Crypto(format!("HKDF profile key derivation failed: {e}")))?;
    Ok(subkey)
}

fn build_profile_aad(profile_key: &str, canonical_origin: &str) -> String {
    format!("bot-browser-vault:profile:v1|{profile_key}|{canonical_origin}")
}

fn derive_credential_key(
    master_key: &[u8; 32],
    credential_id: &str,
) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
    let hk = Hkdf::<Sha256>::new(Some(VAULT_HKDF_SALT), master_key);
    let mut subkey = Zeroizing::new([0u8; 32]);
    hk.expand(credential_id.as_bytes(), &mut *subkey)
        .map_err(|e| BotBrowserError::Crypto(format!("HKDF key derivation failed: {e}")))?;
    Ok(subkey)
}

fn build_aad(credential_id: &str, canonical_origin: &str) -> String {
    format!("bot-browser-vault:v1:{credential_id}|{canonical_origin}")
}

fn policy_to_db_str(policy: ReusePolicy) -> &'static str {
    match policy {
        ReusePolicy::AutoReuse => "auto_reuse",
        ReusePolicy::ConfirmEveryTime => "confirm_every_time",
    }
}

fn db_str_to_policy(raw: &str) -> Result<ReusePolicy, rusqlite::Error> {
    match raw {
        "auto_reuse" => Ok(ReusePolicy::AutoReuse),
        "confirm_every_time" => Ok(ReusePolicy::ConfirmEveryTime),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            format!("unknown reuse_policy `{other}`").into(),
        )),
    }
}

fn row_to_meta(row: &rusqlite::Row<'_>) -> Result<CredentialMeta, rusqlite::Error> {
    let has_totp_int: i64 = row.get(3)?;
    let policy_raw: String = row.get(4)?;
    Ok(CredentialMeta {
        credential_id: row.get(0)?,
        origin: row.get(1)?,
        account_hint: row.get(2)?,
        has_totp: has_totp_int != 0,
        reuse_policy: db_str_to_policy(&policy_raw)?,
        created_at_ms: row.get(5)?,
        last_used_at_ms: row.get(6)?,
    })
}

pub fn mask_account_hint(username: &str) -> String {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        return "***".to_string();
    }

    if let Some((local, domain)) = trimmed.split_once('@') {
        if !domain.is_empty() {
            let local_chars: usize = local.chars().count();
            let prefix_len = if local_chars > 2 {
                2
            } else if local_chars >= 1 {
                1
            } else {
                0
            };
            let prefix: String = local.chars().take(prefix_len).collect();
            return format!("{prefix}***@{domain}");
        }
    }

    let char_count = trimmed.chars().count();
    let prefix_len = if char_count > 4 { 2 } else { 1 };
    let prefix: String = trimmed.chars().take(prefix_len).collect();
    format!("{prefix}***")
}
