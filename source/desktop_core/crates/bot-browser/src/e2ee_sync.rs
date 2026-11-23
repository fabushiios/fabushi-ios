use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use argon2::{Algorithm, Argon2, Params as Argon2Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use chrono::Utc;
use data_encoding::BASE32_NOPAD;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::{Archive, Builder, EntryType, Header};
use uuid::Uuid;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::types::BotBrowserError;
use crate::vault::MasterKeyStore;

pub const USER_VAULT_KEYRING_SERVICE: &str = "com.gpui-telegram.user-vault";
pub const USER_VAULT_KDF_ALG_ARGON2ID: &str = "argon2id";
pub const USER_VAULT_KDF_ALG_SPLIT_KEY_V2: &str = "split_key_v2";
pub const USER_VAULT_KDF_ALG_DEVICE_KEYCHAIN_V2: &str = "device_keychain_v2";
const USER_VAULT_KDF_ALG: &str = USER_VAULT_KDF_ALG_ARGON2ID;
const USER_VAULT_KEY_VERSION: u32 = 1;
const USER_VAULT_KEY_VERSION_V2: u32 = 2;
const USER_VAULT_RECOVERY_HKDF_SALT: &[u8] = b"gpui-user-vault-recovery-hkdf-v1";
const USER_VAULT_ITEM_HKDF_SALT: &[u8] = b"gpui-user-vault-item-hkdf-v1";
const SPLIT_KEY_AUTH_VERIFIER_INFO: &[u8] = b"gpui-telegram/auth-verifier/v2";
const SPLIT_KEY_VAULT_KEK_INFO: &[u8] = b"gpui-telegram/vault-kek/v2";
const DEVICE_SYNC_CODE_DOMAIN: &[u8] = b"gpui-telegram/device-sync-code/v1";
const DEVICE_KEY_TRANSFER_INFO_PREFIX: &str = "gpui-telegram/device-key-transfer/v1:";
const PROFILE_ARCHIVE_COOKIES_ENTRY: &str = "cookies.json";
const PROFILE_ARCHIVE_STORAGE_PREFIX: &str = "storage";
const MAX_UNPACK_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_COOKIES_JSON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserVaultCategory {
    Password,
    BrowserCookie,
    AiOauth,
}

impl UserVaultCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::BrowserCookie => "browser_cookie",
            Self::AiOauth => "ai_oauth",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(raw: &str) -> Result<Self, BotBrowserError> {
        match raw.trim() {
            "password" => Ok(Self::Password),
            "browser_cookie" => Ok(Self::BrowserCookie),
            "ai_oauth" => Ok(Self::AiOauth),
            other => Err(BotBrowserError::Storage(format!(
                "unknown user vault category `{other}`"
            ))),
        }
    }
}

impl std::str::FromStr for UserVaultCategory {
    type Err = BotBrowserError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Argon2KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for Argon2KdfParams {
    fn default() -> Self {
        Self {
            m_cost_kib: 65_536,
            t_cost: 3,
            p_cost: 1,
        }
    }
}

impl Argon2KdfParams {
    pub fn fast_for_tests() -> Self {
        Self {
            m_cost_kib: 1_024,
            t_cost: 1,
            p_cost: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserVaultKeyEnvelope {
    pub key_version: u32,
    pub kdf_alg: String,
    pub kdf_salt_b64: String,
    pub kdf_params: Argon2KdfParams,
    pub wrapped_root_key_b64: String,
    pub recovery_wrapped_root_key_b64: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedVaultBlob {
    pub category: UserVaultCategory,
    pub item_key: String,
    pub revision: i64,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
    pub deleted: bool,
    pub updated_at_ms: i64,
}

/// Unlocked 256-bit `VaultRootKey` kept strictly in-process (never serializable, redacted in `Debug`).
/// Implements `MasterKeyStore` so `CredentialVault::open(db_path, &unlocked_key)` uses the
/// user's E2EE `VaultRootKey` directly.
#[derive(Clone)]
pub struct UnlockedUserVaultKey {
    root_key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for UnlockedUserVaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("UnlockedUserVaultKey")
            .field(&"[REDACTED]")
            .finish()
    }
}

impl MasterKeyStore for UnlockedUserVaultKey {
    fn load_or_create_key(&self) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
        Ok(self.root_key.clone())
    }
}

impl UnlockedUserVaultKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            root_key: Zeroizing::new(bytes),
        }
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.root_key
    }

    pub fn save_to_keyring(&self, user_id: u64) -> Result<(), BotBrowserError> {
        // iOS target_os adaptation: desktop `keyring` crate is not built on iOS; Swift Keychain
        // runner persists `UnlockedUserVaultKey::as_bytes()` via SecItem APIs.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            let user_account = keyring_user_for(user_id);
            let entry = keyring::Entry::new(USER_VAULT_KEYRING_SERVICE, &user_account).map_err(|e| {
                BotBrowserError::Keyring(format!("failed to open user vault keyring entry: {e}"))
            })?;
            let encoded = Zeroizing::new(hex::encode(&*self.root_key));
            entry.set_password(&encoded).map_err(|e| {
                BotBrowserError::Keyring(format!("failed to store user vault root key in keyring: {e}"))
            })
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = user_id;
            Err(BotBrowserError::Keyring(
                "desktop OS keyring unavailable on this target; use iOS Keychain runner with UnlockedUserVaultKey::as_bytes/from_bytes".into(),
            ))
        }
    }

    pub fn save_to_keychain(&self, user_id: u64) -> Result<(), BotBrowserError> {
        self.save_to_keyring(user_id)
    }

    pub fn load_from_keyring(user_id: u64) -> Result<Option<Self>, BotBrowserError> {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            let user_account = keyring_user_for(user_id);
            let entry = keyring::Entry::new(USER_VAULT_KEYRING_SERVICE, &user_account).map_err(|e| {
                BotBrowserError::Keyring(format!("failed to open user vault keyring entry: {e}"))
            })?;

            match entry.get_password() {
                Ok(encoded) => {
                    let encoded = Zeroizing::new(encoded);
                    let decoded = Zeroizing::new(hex::decode(encoded.trim()).map_err(|e| {
                        BotBrowserError::Keyring(format!(
                            "corrupt user vault keyring root key encoding: {e}"
                        ))
                    })?);
                    if decoded.len() != 32 {
                        return Err(BotBrowserError::Keyring(format!(
                            "unexpected user vault keyring root key length: {} bytes",
                            decoded.len()
                        )));
                    }
                    let mut key = Zeroizing::new([0u8; 32]);
                    key.copy_from_slice(&decoded);
                    Ok(Some(Self { root_key: key }))
                }
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(BotBrowserError::Keyring(format!(
                    "failed to read user vault root key from keyring: {e}"
                ))),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = user_id;
            Err(BotBrowserError::Keyring(
                "desktop OS keyring unavailable on this target; use iOS Keychain runner with UnlockedUserVaultKey::from_bytes".into(),
            ))
        }
    }

    pub fn load_from_keychain(user_id: u64) -> Result<Option<Self>, BotBrowserError> {
        Self::load_from_keyring(user_id)
    }

    pub fn delete_from_keyring(user_id: u64) -> Result<(), BotBrowserError> {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            let user_account = keyring_user_for(user_id);
            let entry = keyring::Entry::new(USER_VAULT_KEYRING_SERVICE, &user_account).map_err(|e| {
                BotBrowserError::Keyring(format!("failed to open user vault keyring entry: {e}"))
            })?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(BotBrowserError::Keyring(format!(
                    "failed to delete user vault root key from keyring: {e}"
                ))),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = user_id;
            Err(BotBrowserError::Keyring(
                "desktop OS keyring unavailable on this target; delete key via iOS Keychain runner".into(),
            ))
        }
    }

    pub fn delete_from_keychain(user_id: u64) -> Result<(), BotBrowserError> {
        Self::delete_from_keyring(user_id)
    }

    /// Persists this 256-bit root key atomically into a `0o600` (on Unix) local key file.
    pub fn save_to_file(&self, path: &Path) -> Result<(), BotBrowserError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
                }
            }
        }

        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let tmp_path = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("user-vault-root.key"),
            Uuid::new_v4()
        ));
        let encoded = Zeroizing::new(hex::encode(&*self.root_key));

        let write_result: Result<(), BotBrowserError> = (|| {
            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut file = opts.open(&tmp_path)?;
            file.write_all(encoded.as_bytes())?;
            file.sync_all()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
            }
            fs::rename(&tmp_path, path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            }
            Ok(())
        })();

        if let Err(err) = write_result {
            let _ = fs::remove_file(&tmp_path);
            return Err(err);
        }
        Ok(())
    }

    /// Loads a 256-bit root key from a local key file (`32` raw bytes or trimmed hex).
    pub fn load_from_file(path: &Path) -> Result<Option<Self>, BotBrowserError> {
        if !path.exists() {
            return Ok(None);
        }
        let mut file = OpenOptions::new().read(true).open(path)?;
        let mut raw = Zeroizing::new(Vec::new());
        file.read_to_end(&mut raw)?;

        if raw.len() == 32 {
            let mut key = Zeroizing::new([0u8; 32]);
            key.copy_from_slice(&raw);
            return Ok(Some(Self { root_key: key }));
        }

        if let Ok(as_str) = std::str::from_utf8(&raw) {
            let decoded = Zeroizing::new(hex::decode(as_str.trim()).map_err(|e| {
                BotBrowserError::Keyring(format!("corrupt user vault file root key encoding: {e}"))
            })?);
            if decoded.len() == 32 {
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&decoded);
                return Ok(Some(Self { root_key: key }));
            }
            return Err(BotBrowserError::Keyring(format!(
                "unexpected user vault file root key length: {} bytes",
                decoded.len()
            )));
        }

        Err(BotBrowserError::Keyring(format!(
            "invalid user vault root key file size ({} bytes) at {}",
            raw.len(),
            path.display()
        )))
    }

    pub fn delete_file(path: &Path) -> Result<(), BotBrowserError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(BotBrowserError::Io(err.to_string())),
        }
    }

    pub fn seal_item(
        &self,
        user_id: u64,
        category: UserVaultCategory,
        item_key: &str,
        revision: i64,
        plaintext: &[u8],
    ) -> Result<EncryptedVaultBlob, BotBrowserError> {
        validate_item_key(item_key)?;
        let subkey = derive_item_subkey(&self.root_key, user_id, category, item_key)?;
        let aad = build_item_aad(user_id, category, item_key, revision);

        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes).map_err(|e| {
            BotBrowserError::Crypto(format!("failed to generate vault item nonce: {e}"))
        })?;

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*subkey));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|e| {
                BotBrowserError::Crypto(format!("vault item encryption failed: {e}"))
            })?;

        Ok(EncryptedVaultBlob {
            category,
            item_key: item_key.to_string(),
            revision,
            nonce_b64: BASE64_STANDARD.encode(nonce_bytes),
            ciphertext_b64: BASE64_STANDARD.encode(ciphertext),
            deleted: false,
            updated_at_ms: Utc::now().timestamp_millis(),
        })
    }

    pub fn open_item(
        &self,
        user_id: u64,
        blob: &EncryptedVaultBlob,
    ) -> Result<Zeroizing<Vec<u8>>, BotBrowserError> {
        validate_item_key(&blob.item_key)?;
        let nonce_bytes = BASE64_STANDARD.decode(blob.nonce_b64.trim()).map_err(|e| {
            BotBrowserError::Crypto(format!("invalid vault item nonce base64: {e}"))
        })?;
        if nonce_bytes.len() != 12 {
            return Err(BotBrowserError::Crypto(format!(
                "invalid vault item nonce length: {} bytes",
                nonce_bytes.len()
            )));
        }
        let ciphertext = BASE64_STANDARD
            .decode(blob.ciphertext_b64.trim())
            .map_err(|e| {
                BotBrowserError::Crypto(format!("invalid vault item ciphertext base64: {e}"))
            })?;

        let subkey = derive_item_subkey(&self.root_key, user_id, blob.category, &blob.item_key)?;
        let aad = build_item_aad(user_id, blob.category, &blob.item_key, blob.revision);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*subkey));

        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| {
                BotBrowserError::Crypto(
                    "vault item AEAD authentication failed (wrong key, user, category, item_key, or revision)"
                        .to_string(),
                )
            })?;

        Ok(Zeroizing::new(plaintext))
    }
}

/// 256-bit vault key-encryption key derived locally via 1Password-style Split-Key V2.
/// Strictly zeroized on drop, never serialized, and redacted in `Debug`.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SplitKeyVaultKek([u8; 32]);

impl std::fmt::Debug for SplitKeyVaultKek {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SplitKeyVaultKek")
            .field(&"[REDACTED]")
            .finish()
    }
}

impl SplitKeyVaultKek {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Output of Split-Key V2 local password derivation: separates `auth_verifier_b64` (sent to server)
/// from `vault_kek` (strictly client-side KEK wrapping `VaultRootKey`).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SplitKeyBundle {
    #[zeroize(skip)]
    pub auth_salt_b64: String,
    #[zeroize(skip)]
    pub kdf_params: Argon2KdfParams,
    pub auth_verifier_b64: Zeroizing<String>,
    pub vault_kek: SplitKeyVaultKek,
}

impl std::fmt::Debug for SplitKeyBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplitKeyBundle")
            .field("auth_salt_b64", &self.auth_salt_b64)
            .field("kdf_params", &self.kdf_params)
            .field("auth_verifier_b64", &"[REDACTED]")
            .field("vault_kek", &self.vault_kek)
            .finish()
    }
}

/// Generates a fresh random 16-byte salt encoded as standard base64 for Split-Key V2.
pub fn generate_split_key_salt_b64() -> String {
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).expect("OS CSPRNG unavailable for split-key salt");
    BASE64_STANDARD.encode(salt)
}

/// Derives domain-separated `auth_verifier_b64` and `vault_kek` from `password` and a 16-byte
/// base64 salt using Argon2id + HKDF-SHA256.
pub fn derive_split_key(
    password: &str,
    auth_salt_b64: &str,
    params: &Argon2KdfParams,
) -> Result<SplitKeyBundle, BotBrowserError> {
    if password.is_empty() {
        return Err(BotBrowserError::Crypto(
            "password cannot be empty for split-key derivation".to_string(),
        ));
    }
    let salt = decode_split_key_salt_16(auth_salt_b64)?;
    let master_key = derive_passphrase_wrap_key(password, &salt, *params)?;

    let hk = Hkdf::<Sha256>::new(None, &*master_key);
    let mut auth_verifier_bytes = Zeroizing::new([0u8; 32]);
    hk.expand(SPLIT_KEY_AUTH_VERIFIER_INFO, &mut *auth_verifier_bytes)
        .map_err(|e| {
            BotBrowserError::Crypto(format!("split-key auth verifier HKDF failed: {e}"))
        })?;

    let mut kek_bytes = Zeroizing::new([0u8; 32]);
    hk.expand(SPLIT_KEY_VAULT_KEK_INFO, &mut *kek_bytes)
        .map_err(|e| BotBrowserError::Crypto(format!("split-key vault KEK HKDF failed: {e}")))?;

    Ok(SplitKeyBundle {
        auth_salt_b64: auth_salt_b64.trim().to_string(),
        kdf_params: *params,
        auth_verifier_b64: Zeroizing::new(BASE64_STANDARD.encode(&*auth_verifier_bytes)),
        vault_kek: SplitKeyVaultKek(*kek_bytes),
    })
}

/// Ephemeral X25519-sealed `VaultRootKey` payload relayed between trusted user devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceKeyTransfer {
    pub approver_ephemeral_pub_b64: String,
    pub encrypted_root_key_b64: String,
    pub nonce_b64: String,
}

/// Ephemeral X25519 keypair for Apple Passwords-style Device Trust Ring approval.
/// Secret scalar bytes are strictly zeroized on drop and redacted in `Debug`.
pub struct DeviceSyncKeypair {
    secret: Zeroizing<[u8; 32]>,
    public: PublicKey,
}

impl std::fmt::Debug for DeviceSyncKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceSyncKeypair")
            .field("public_key_b64", &self.public_key_b64())
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl DeviceSyncKeypair {
    pub fn generate() -> Self {
        let mut secret = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(&mut *secret).expect("OS CSPRNG unavailable for X25519 keygen");
        let static_secret = StaticSecret::from(*secret);
        let public = PublicKey::from(&static_secret);
        Self { secret, public }
    }

    pub fn public_key_b64(&self) -> String {
        BASE64_STANDARD.encode(self.public.as_bytes())
    }

    /// Computes canonical deterministic 6-digit verification code (`NNN-NNN`) from an X25519 public key.
    pub fn verification_code(public_key_b64: &str) -> Result<String, BotBrowserError> {
        let pubkey = decode_x25519_public_key(public_key_b64)?;
        let mut hasher = Sha256::new();
        hasher.update(DEVICE_SYNC_CODE_DOMAIN);
        hasher.update(pubkey.as_bytes());
        let digest = hasher.finalize();
        let code = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
        Ok(format!("{:03}-{:03}", code / 1_000, code % 1_000))
    }

    /// Seals `root_key` with an ephemeral X25519 ECDH + HKDF-SHA256 + ChaCha20-Poly1305 key for `target_device_pub_b64`.
    pub fn seal_root_key_for_device(
        request_id: &str,
        user_id: u64,
        root_key: &UnlockedUserVaultKey,
        target_device_pub_b64: &str,
    ) -> Result<DeviceKeyTransfer, BotBrowserError> {
        validate_device_sync_request_id(request_id)?;
        let target_pub = decode_x25519_public_key(target_device_pub_b64)?;

        let mut eph_bytes = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(&mut *eph_bytes).map_err(|e| {
            BotBrowserError::Crypto(format!("failed generating ephemeral X25519 secret: {e}"))
        })?;
        let eph_secret = StaticSecret::from(*eph_bytes);
        let eph_pub = PublicKey::from(&eph_secret);
        let shared = eph_secret.diffie_hellman(&target_pub);
        if !shared.was_contributory() {
            return Err(BotBrowserError::Crypto(
                "rejected non-contributory X25519 target public key".to_string(),
            ));
        }

        let wrap_key = derive_device_transfer_wrap_key(shared.as_bytes(), request_id)?;
        let aad = build_device_transfer_aad(request_id, user_id);
        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes).map_err(|e| {
            BotBrowserError::Crypto(format!("failed generating device transfer nonce: {e}"))
        })?;

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*wrap_key));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: root_key.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|e| {
                BotBrowserError::Crypto(format!("device root key transfer encryption failed: {e}"))
            })?;

        Ok(DeviceKeyTransfer {
            approver_ephemeral_pub_b64: BASE64_STANDARD.encode(eph_pub.as_bytes()),
            encrypted_root_key_b64: BASE64_STANDARD.encode(ciphertext),
            nonce_b64: BASE64_STANDARD.encode(nonce_bytes),
        })
    }

    /// Consumes this ephemeral keypair to decrypt a `DeviceKeyTransfer` bound to `(request_id, user_id)`.
    pub fn open_root_key_transfer(
        self,
        request_id: &str,
        user_id: u64,
        transfer: &DeviceKeyTransfer,
    ) -> Result<UnlockedUserVaultKey, BotBrowserError> {
        validate_device_sync_request_id(request_id)?;
        let eph_pub = decode_x25519_public_key(&transfer.approver_ephemeral_pub_b64)?;
        let nonce_bytes = BASE64_STANDARD
            .decode(transfer.nonce_b64.trim())
            .map_err(|e| {
                BotBrowserError::Crypto(format!("invalid device transfer nonce base64: {e}"))
            })?;
        if nonce_bytes.len() != 12 {
            return Err(BotBrowserError::Crypto(format!(
                "invalid device transfer nonce length: {} bytes",
                nonce_bytes.len()
            )));
        }
        let ciphertext = BASE64_STANDARD
            .decode(transfer.encrypted_root_key_b64.trim())
            .map_err(|e| {
                BotBrowserError::Crypto(format!("invalid device transfer ciphertext base64: {e}"))
            })?;
        if ciphertext.len() != 32 + 16 {
            return Err(BotBrowserError::Crypto(format!(
                "unexpected device transfer ciphertext length: {} bytes",
                ciphertext.len()
            )));
        }

        let static_secret = StaticSecret::from(*self.secret);
        let shared = static_secret.diffie_hellman(&eph_pub);
        if !shared.was_contributory() {
            return Err(BotBrowserError::Crypto(
                "rejected non-contributory X25519 ephemeral public key".to_string(),
            ));
        }

        let wrap_key = derive_device_transfer_wrap_key(shared.as_bytes(), request_id)?;
        let aad = build_device_transfer_aad(request_id, user_id);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*wrap_key));
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&nonce_bytes),
                    Payload {
                        msg: &ciphertext,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| {
                    BotBrowserError::Crypto(
                        "device root key transfer AEAD verification failed".to_string(),
                    )
                })?,
        );
        if plaintext.len() != 32 {
            return Err(BotBrowserError::Crypto(format!(
                "unwrapped device transfer root key has unexpected length: {} bytes",
                plaintext.len()
            )));
        }
        let mut root_bytes = Zeroizing::new([0u8; 32]);
        root_bytes.copy_from_slice(&plaintext);
        Ok(UnlockedUserVaultKey::from_bytes(*root_bytes))
    }
}

pub struct UserVaultCrypto;

impl UserVaultCrypto {
    pub fn initialize_new(
        user_id: u64,
        passphrase: &str,
        params: Argon2KdfParams,
    ) -> Result<(UnlockedUserVaultKey, UserVaultKeyEnvelope, Zeroizing<String>), BotBrowserError> {
        if passphrase.is_empty() {
            return Err(BotBrowserError::Crypto(
                "vault passphrase cannot be empty".to_string(),
            ));
        }

        let mut salt = [0u8; 16];
        getrandom::getrandom(&mut salt).map_err(|e| {
            BotBrowserError::Crypto(format!("failed generating Argon2id salt entropy: {e}"))
        })?;

        let (root_key, recovery_wrapped_root_key_b64, recovery_raw) =
            generate_root_and_recovery_wrapping(user_id)?;

        let pass_wrap_key = derive_passphrase_wrap_key(passphrase, &salt, params)?;
        let wrapped_root_key_b64 = wrap_root_key_with_aad(
            &pass_wrap_key,
            &root_key,
            &build_wrapped_root_aad(user_id),
        )?;

        let recovery_key = format_recovery_key(&*recovery_raw);
        let envelope = UserVaultKeyEnvelope {
            key_version: USER_VAULT_KEY_VERSION,
            kdf_alg: USER_VAULT_KDF_ALG.to_string(),
            kdf_salt_b64: BASE64_STANDARD.encode(salt),
            kdf_params: params,
            wrapped_root_key_b64,
            recovery_wrapped_root_key_b64: Some(recovery_wrapped_root_key_b64),
            updated_at_ms: Utc::now().timestamp_millis(),
        };

        Ok((
            UnlockedUserVaultKey { root_key },
            envelope,
            recovery_key,
        ))
    }

    /// Initializes a new E2EE vault wrapped with a Split-Key V2 `VaultKEK` plus an `EK-...` Emergency Kit.
    pub fn initialize_with_split_key(
        user_id: u64,
        split_kek: &SplitKeyVaultKek,
        auth_salt_b64: &str,
        kdf_params: Argon2KdfParams,
    ) -> Result<(UserVaultKeyEnvelope, UnlockedUserVaultKey, Zeroizing<String>), BotBrowserError> {
        let _ = decode_split_key_salt_16(auth_salt_b64)?;
        let _ = validate_argon2_params(kdf_params)?;

        let (root_key, recovery_wrapped_root_key_b64, recovery_raw) =
            generate_root_and_recovery_wrapping(user_id)?;

        let wrapped_root_key_b64 = wrap_root_key_with_aad(
            split_kek.as_bytes(),
            &root_key,
            &build_wrapped_root_aad(user_id),
        )?;
        let recovery_key = format_emergency_kit_key(&*recovery_raw);
        let envelope = UserVaultKeyEnvelope {
            key_version: USER_VAULT_KEY_VERSION_V2,
            kdf_alg: USER_VAULT_KDF_ALG_SPLIT_KEY_V2.to_string(),
            kdf_salt_b64: auth_salt_b64.trim().to_string(),
            kdf_params,
            wrapped_root_key_b64,
            recovery_wrapped_root_key_b64: Some(recovery_wrapped_root_key_b64),
            updated_at_ms: Utc::now().timestamp_millis(),
        };

        Ok((
            envelope,
            UnlockedUserVaultKey { root_key },
            recovery_key,
        ))
    }

    /// Initializes a passwordless (`device_keychain_v2`) E2EE vault with no password wrap and an `EK-...` Emergency Kit.
    pub fn initialize_keychain_only(
        user_id: u64,
    ) -> Result<(UserVaultKeyEnvelope, UnlockedUserVaultKey, Zeroizing<String>), BotBrowserError> {
        let (root_key, recovery_wrapped_root_key_b64, recovery_raw) =
            generate_root_and_recovery_wrapping(user_id)?;
        let recovery_key = format_emergency_kit_key(&*recovery_raw);
        let envelope = UserVaultKeyEnvelope {
            key_version: USER_VAULT_KEY_VERSION_V2,
            kdf_alg: USER_VAULT_KDF_ALG_DEVICE_KEYCHAIN_V2.to_string(),
            kdf_salt_b64: String::new(),
            kdf_params: Argon2KdfParams::default(),
            wrapped_root_key_b64: String::new(),
            recovery_wrapped_root_key_b64: Some(recovery_wrapped_root_key_b64),
            updated_at_ms: Utc::now().timestamp_millis(),
        };

        Ok((
            envelope,
            UnlockedUserVaultKey { root_key },
            recovery_key,
        ))
    }

    pub fn unlock_with_passphrase(
        user_id: u64,
        passphrase: &str,
        envelope: &UserVaultKeyEnvelope,
    ) -> Result<UnlockedUserVaultKey, BotBrowserError> {
        if passphrase.is_empty() {
            return Err(BotBrowserError::Crypto(
                "vault passphrase cannot be empty".to_string(),
            ));
        }
        if envelope.kdf_alg != USER_VAULT_KDF_ALG {
            return Err(BotBrowserError::Crypto(format!(
                "unsupported envelope kdf_alg `{}`",
                envelope.kdf_alg
            )));
        }

        let salt = BASE64_STANDARD
            .decode(envelope.kdf_salt_b64.trim())
            .map_err(|e| BotBrowserError::Crypto(format!("invalid kdf_salt_b64: {e}")))?;
        if salt.len() < 8 {
            return Err(BotBrowserError::Crypto(format!(
                "invalid kdf salt length: {} bytes",
                salt.len()
            )));
        }

        let pass_wrap_key = derive_passphrase_wrap_key(passphrase, &salt, envelope.kdf_params)?;
        let root_key = unwrap_root_key_with_aad(
            &pass_wrap_key,
            &envelope.wrapped_root_key_b64,
            &build_wrapped_root_aad(user_id),
        )?;
        Ok(UnlockedUserVaultKey { root_key })
    }

    /// Unwraps `VaultRootKey` from a `split_key_v2` (or compatible passphrase-wrapped) envelope using `split_kek`.
    pub fn unlock_with_split_key(
        user_id: u64,
        envelope: &UserVaultKeyEnvelope,
        split_kek: &SplitKeyVaultKek,
    ) -> Result<UnlockedUserVaultKey, BotBrowserError> {
        match envelope.kdf_alg.trim() {
            USER_VAULT_KDF_ALG_SPLIT_KEY_V2 | USER_VAULT_KDF_ALG_ARGON2ID => {}
            USER_VAULT_KDF_ALG_DEVICE_KEYCHAIN_V2 => {
                return Err(BotBrowserError::Crypto(
                    "device_keychain_v2 envelope has no password-wrapped root key; unlock via Keychain, Device Sync, or Emergency Kit".to_string(),
                ));
            }
            other => {
                return Err(BotBrowserError::Crypto(format!(
                    "unsupported envelope kdf_alg `{other}` for split-key unlock"
                )));
            }
        }
        let root_key = unwrap_root_key_with_aad(
            split_kek.as_bytes(),
            &envelope.wrapped_root_key_b64,
            &build_wrapped_root_aad(user_id),
        )?;
        Ok(UnlockedUserVaultKey { root_key })
    }

    /// Re-wraps an already-unlocked `VaultRootKey` under `new_kek` (for password changes or v1->v2 silent
    /// upgrades) without re-encrypting any vault items and preserving the existing Emergency Kit wrapping.
    pub fn rewrap_root_key_with_kek(
        user_id: u64,
        existing: &UnlockedUserVaultKey,
        envelope: &UserVaultKeyEnvelope,
        new_kek: &SplitKeyVaultKek,
        new_salt_b64: &str,
        new_kdf_params: Argon2KdfParams,
    ) -> Result<UserVaultKeyEnvelope, BotBrowserError> {
        let _ = decode_split_key_salt_16(new_salt_b64)?;
        let _ = validate_argon2_params(new_kdf_params)?;

        let wrapped_root_key_b64 = wrap_root_key_with_aad(
            new_kek.as_bytes(),
            existing.as_bytes(),
            &build_wrapped_root_aad(user_id),
        )?;
        Ok(UserVaultKeyEnvelope {
            key_version: USER_VAULT_KEY_VERSION_V2,
            kdf_alg: USER_VAULT_KDF_ALG_SPLIT_KEY_V2.to_string(),
            kdf_salt_b64: new_salt_b64.trim().to_string(),
            kdf_params: new_kdf_params,
            wrapped_root_key_b64,
            recovery_wrapped_root_key_b64: envelope.recovery_wrapped_root_key_b64.clone(),
            updated_at_ms: Utc::now().timestamp_millis(),
        })
    }

    pub fn unlock_with_recovery_key(
        user_id: u64,
        recovery_key: &str,
        envelope: &UserVaultKeyEnvelope,
    ) -> Result<UnlockedUserVaultKey, BotBrowserError> {
        match envelope.kdf_alg.trim() {
            USER_VAULT_KDF_ALG_ARGON2ID
            | USER_VAULT_KDF_ALG_SPLIT_KEY_V2
            | USER_VAULT_KDF_ALG_DEVICE_KEYCHAIN_V2 => {}
            other => {
                return Err(BotBrowserError::Crypto(format!(
                    "unsupported envelope kdf_alg `{other}` for recovery unlock"
                )));
            }
        }
        let wrapped_b64 = envelope
            .recovery_wrapped_root_key_b64
            .as_deref()
            .ok_or_else(|| {
                BotBrowserError::Crypto(
                    "vault key envelope does not contain a recovery-wrapped root key".to_string(),
                )
            })?;

        let recovery_raw = parse_recovery_key(recovery_key)?;
        let rec_wrap_key = derive_recovery_wrap_key(user_id, &recovery_raw)?;
        let root_key = unwrap_root_key_with_aad(
            &rec_wrap_key,
            wrapped_b64,
            &build_recovery_root_aad(user_id),
        )?;
        Ok(UnlockedUserVaultKey { root_key })
    }
}

pub struct BrowserProfileSnapshot;

impl BrowserProfileSnapshot {
    pub fn pack_storage_dir(
        storage_dir: &Path,
        cookies_json: Option<&[serde_json::Value]>,
        max_bytes: usize,
    ) -> Result<Zeroizing<Vec<u8>>, BotBrowserError> {
        if storage_dir.exists() && !storage_dir.is_dir() {
            return Err(BotBrowserError::Storage(format!(
                "browser storage_dir '{}' is not a directory",
                storage_dir.display()
            )));
        }

        let mut tar_gz_buf = Zeroizing::new(Vec::new());
        {
            let gz = GzEncoder::new(&mut *tar_gz_buf, Compression::default());
            let mut tar = Builder::new(gz);

            if let Some(cookies) = cookies_json {
                let cookies_bytes = Zeroizing::new(serde_json::to_vec(cookies).map_err(|e| {
                    BotBrowserError::Storage(format!("failed serializing cookies_json: {e}"))
                })?);
                let mut header = Header::new_gnu();
                header.set_entry_type(EntryType::Regular);
                header.set_size(cookies_bytes.len() as u64);
                header.set_mode(0o600);
                header.set_cksum();
                tar.append_data(&mut header, PROFILE_ARCHIVE_COOKIES_ENTRY, &cookies_bytes[..])?;
            }

            if storage_dir.exists() {
                append_storage_dir_recursive(&mut tar, storage_dir, storage_dir)?;
            }

            let gz = tar.into_inner()?;
            gz.finish()?;
        }

        if tar_gz_buf.len() > max_bytes {
            return Err(BotBrowserError::Storage(format!(
                "packed browser profile ({} bytes) exceeds max_bytes ({max_bytes})",
                tar_gz_buf.len()
            )));
        }

        Ok(tar_gz_buf)
    }

    pub fn unpack_storage_dir(
        bundle: &[u8],
        target_storage_dir: &Path,
    ) -> Result<Option<Vec<serde_json::Value>>, BotBrowserError> {
        if target_storage_dir.exists() && !target_storage_dir.is_dir() {
            return Err(BotBrowserError::Storage(format!(
                "target_storage_dir '{}' is not a directory",
                target_storage_dir.display()
            )));
        }
        fs::create_dir_all(target_storage_dir)?;

        let gz = GzDecoder::new(Cursor::new(bundle));
        let mut archive = Archive::new(gz);
        let mut total_unpacked_bytes: u64 = 0;
        let mut extracted_cookies: Option<Vec<serde_json::Value>> = None;

        for entry_res in archive.entries()? {
            let mut entry = entry_res?;
            let entry_type = entry.header().entry_type();

            if entry_type.is_symlink() || entry_type.is_hard_link() {
                return Err(BotBrowserError::Storage(
                    "rejected symlink/hardlink entry in browser profile archive".to_string(),
                ));
            }
            if !(entry_type.is_file() || entry_type.is_dir()) {
                return Err(BotBrowserError::Storage(format!(
                    "rejected unsupported tar entry type {:?} in browser profile archive",
                    entry_type
                )));
            }

            let raw_bytes = entry.path_bytes();
            let raw_lossy = String::from_utf8_lossy(&raw_bytes);
            validate_raw_archive_path_bytes(&raw_bytes, &raw_lossy)?;

            let entry_path = entry.path()?.into_owned();
            let clean_rel = validate_and_normalize_archive_path(&entry_path)?;
            if clean_rel.as_os_str().is_empty() {
                continue;
            }

            if clean_rel == Path::new(PROFILE_ARCHIVE_COOKIES_ENTRY) {
                if !entry_type.is_file() {
                    return Err(BotBrowserError::Storage(
                        "cookies.json entry in archive must be a regular file".to_string(),
                    ));
                }
                let entry_size = entry.header().size()?;
                if entry_size > MAX_COOKIES_JSON_BYTES {
                    return Err(BotBrowserError::Storage(format!(
                        "cookies.json exceeds maximum allowed size ({entry_size} bytes)"
                    )));
                }
                total_unpacked_bytes = total_unpacked_bytes.saturating_add(entry_size);
                if total_unpacked_bytes > MAX_UNPACK_TOTAL_BYTES {
                    return Err(BotBrowserError::Storage(
                        "browser profile archive exceeds maximum unpacked byte limit".to_string(),
                    ));
                }

                let mut raw_cookies = Zeroizing::new(Vec::new());
                (&mut entry)
                    .take(MAX_COOKIES_JSON_BYTES + 1)
                    .read_to_end(&mut raw_cookies)?;
                if raw_cookies.len() as u64 > MAX_COOKIES_JSON_BYTES {
                    return Err(BotBrowserError::Storage(
                        "cookies.json stream exceeds maximum allowed size".to_string(),
                    ));
                }
                let parsed: Vec<serde_json::Value> = serde_json::from_slice(&raw_cookies)
                    .map_err(|e| {
                        BotBrowserError::Storage(format!("invalid cookies.json in archive: {e}"))
                    })?;
                extracted_cookies = Some(parsed);
                continue;
            }

            let storage_rel = match clean_rel.strip_prefix(PROFILE_ARCHIVE_STORAGE_PREFIX) {
                Ok(stripped) => stripped,
                Err(_) => clean_rel.as_path(),
            };
            if storage_rel.as_os_str().is_empty() {
                continue;
            }

            let dest_path = safe_join_under(target_storage_dir, storage_rel)?;

            if entry_type.is_dir() {
                fs::create_dir_all(&dest_path)?;
                continue;
            }

            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let declared_size = entry.header().size()?;
            if total_unpacked_bytes.saturating_add(declared_size) > MAX_UNPACK_TOTAL_BYTES {
                return Err(BotBrowserError::Storage(
                    "browser profile archive exceeds maximum unpacked byte limit".to_string(),
                ));
            }

            let mut out_opts = OpenOptions::new();
            out_opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                out_opts.mode(0o600);
            }
            let mut out_file = out_opts.open(&dest_path)?;
            let mut buf = Zeroizing::new([0u8; 32 * 1024]);
            let mut entry_written: u64 = 0;
            loop {
                let n = entry.read(&mut *buf)?;
                if n == 0 {
                    break;
                }
                entry_written = entry_written.saturating_add(n as u64);
                total_unpacked_bytes = total_unpacked_bytes.saturating_add(n as u64);
                if total_unpacked_bytes > MAX_UNPACK_TOTAL_BYTES {
                    drop(out_file);
                    let _ = fs::remove_file(&dest_path);
                    return Err(BotBrowserError::Storage(
                        "browser profile archive exceeds maximum unpacked byte limit".to_string(),
                    ));
                }
                out_file.write_all(&buf[..n])?;
            }
            out_file.flush()?;
        }

        Ok(extracted_cookies)
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiOauthVaultBundle {
    pub providers_json: serde_json::Value,
    pub subscriptions_json: serde_json::Value,
    pub updated_at_ms: i64,
}

impl Zeroize for AiOauthVaultBundle {
    fn zeroize(&mut self) {
        zeroize_json_value(&mut self.providers_json);
        zeroize_json_value(&mut self.subscriptions_json);
        self.updated_at_ms.zeroize();
    }
}

impl Drop for AiOauthVaultBundle {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for AiOauthVaultBundle {}

impl std::fmt::Debug for AiOauthVaultBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiOauthVaultBundle")
            .field("providers_json", &"[REDACTED]")
            .field("subscriptions_json", &"[REDACTED]")
            .field("updated_at_ms", &self.updated_at_ms)
            .finish()
    }
}

impl AiOauthVaultBundle {
    pub fn to_json_bytes(&self) -> Result<Zeroizing<Vec<u8>>, BotBrowserError> {
        serde_json::to_vec(self)
            .map(Zeroizing::new)
            .map_err(|e| BotBrowserError::Crypto(format!("failed serializing AiOauthVaultBundle: {e}")))
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, BotBrowserError> {
        serde_json::from_slice(bytes).map_err(|e| {
            BotBrowserError::Crypto(format!("failed deserializing AiOauthVaultBundle: {e}"))
        })
    }
}

fn zeroize_json_value(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::String(s) => s.zeroize(),
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                zeroize_json_value(item);
            }
            arr.clear();
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                zeroize_json_value(v);
            }
            map.clear();
        }
        _ => {}
    }
    *val = serde_json::Value::Null;
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn keyring_user_for(user_id: u64) -> String {
    format!("user-{user_id}-root-key-v1")
}

fn validate_item_key(item_key: &str) -> Result<(), BotBrowserError> {
    if item_key.is_empty() || item_key.len() > 128 {
        return Err(BotBrowserError::Storage(format!(
            "invalid vault item_key length ({}): must be 1..=128",
            item_key.len()
        )));
    }
    if item_key.contains('|') {
        return Err(BotBrowserError::Storage(
            "vault item_key cannot contain '|'".to_string(),
        ));
    }
    Ok(())
}

fn derive_item_subkey(
    root_key: &[u8; 32],
    user_id: u64,
    category: UserVaultCategory,
    item_key: &str,
) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
    let hk = Hkdf::<Sha256>::new(Some(USER_VAULT_ITEM_HKDF_SALT), root_key);
    let info = format!(
        "gpui-user-vault:subkey:v1|{user_id}|{}|{item_key}",
        category.as_str()
    );
    let mut subkey = Zeroizing::new([0u8; 32]);
    hk.expand(info.as_bytes(), &mut *subkey)
        .map_err(|e| BotBrowserError::Crypto(format!("vault item HKDF derivation failed: {e}")))?;
    Ok(subkey)
}

fn build_item_aad(
    user_id: u64,
    category: UserVaultCategory,
    item_key: &str,
    revision: i64,
) -> String {
    format!(
        "gpui-user-vault:item:v1|{user_id}|{}|{item_key}|{revision}",
        category.as_str()
    )
}

fn build_wrapped_root_aad(user_id: u64) -> String {
    format!("gpui-user-vault:wrapped-root-key:v1|{user_id}")
}

fn build_recovery_root_aad(user_id: u64) -> String {
    format!("gpui-user-vault:recovery-root-key:v1|{user_id}")
}

fn validate_argon2_params(params: Argon2KdfParams) -> Result<Argon2Params, BotBrowserError> {
    if params.m_cost_kib < 8 || params.m_cost_kib > 4 * 1024 * 1024 {
        return Err(BotBrowserError::Crypto(format!(
            "Argon2id m_cost_kib out of safe bounds: {}",
            params.m_cost_kib
        )));
    }
    if params.t_cost == 0 || params.p_cost == 0 {
        return Err(BotBrowserError::Crypto(
            "Argon2id t_cost and p_cost must be >= 1".to_string(),
        ));
    }

    Argon2Params::new(params.m_cost_kib, params.t_cost, params.p_cost, Some(32))
        .map_err(|e| BotBrowserError::Crypto(format!("invalid Argon2id params: {e}")))
}

fn decode_split_key_salt_16(auth_salt_b64: &str) -> Result<[u8; 16], BotBrowserError> {
    let salt_vec = BASE64_STANDARD
        .decode(auth_salt_b64.trim())
        .map_err(|e| BotBrowserError::Crypto(format!("invalid split-key auth_salt_b64: {e}")))?;
    if salt_vec.len() != 16 {
        return Err(BotBrowserError::Crypto(format!(
            "invalid split-key salt length: {} bytes (expected 16)",
            salt_vec.len()
        )));
    }
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&salt_vec);
    Ok(salt)
}

fn generate_root_and_recovery_wrapping(
    user_id: u64,
) -> Result<(Zeroizing<[u8; 32]>, String, Zeroizing<[u8; 32]>), BotBrowserError> {
    let mut root_key = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(&mut *root_key).map_err(|e| {
        BotBrowserError::Crypto(format!("failed generating VaultRootKey entropy: {e}"))
    })?;

    let mut recovery_raw = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(&mut *recovery_raw).map_err(|e| {
        BotBrowserError::Crypto(format!("failed generating Recovery Key entropy: {e}"))
    })?;

    let rec_wrap_key = derive_recovery_wrap_key(user_id, &recovery_raw)?;
    let recovery_wrapped_root_key_b64 = wrap_root_key_with_aad(
        &rec_wrap_key,
        &root_key,
        &build_recovery_root_aad(user_id),
    )?;
    Ok((root_key, recovery_wrapped_root_key_b64, recovery_raw))
}

fn validate_device_sync_request_id(request_id: &str) -> Result<(), BotBrowserError> {
    if request_id.trim().is_empty() || request_id.len() > 128 {
        return Err(BotBrowserError::Crypto(
            "invalid device sync request_id".to_string(),
        ));
    }
    Ok(())
}

fn decode_x25519_public_key(public_key_b64: &str) -> Result<PublicKey, BotBrowserError> {
    let bytes = BASE64_STANDARD
        .decode(public_key_b64.trim())
        .map_err(|e| BotBrowserError::Crypto(format!("invalid X25519 public key base64: {e}")))?;
    if bytes.len() != 32 {
        return Err(BotBrowserError::Crypto(format!(
            "invalid X25519 public key length: {} bytes (expected 32)",
            bytes.len()
        )));
    }
    if bytes.iter().all(|&b| b == 0) {
        return Err(BotBrowserError::Crypto(
            "rejected all-zero X25519 public key".to_string(),
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(PublicKey::from(arr))
}

fn derive_device_transfer_wrap_key(
    shared_secret_bytes: &[u8; 32],
    request_id: &str,
) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
    let hk = Hkdf::<Sha256>::new(None, shared_secret_bytes);
    let info = format!("{DEVICE_KEY_TRANSFER_INFO_PREFIX}{request_id}");
    let mut wrap_key = Zeroizing::new([0u8; 32]);
    hk.expand(info.as_bytes(), &mut *wrap_key)
        .map_err(|e| BotBrowserError::Crypto(format!("device key transfer HKDF failed: {e}")))?;
    Ok(wrap_key)
}

fn build_device_transfer_aad(request_id: &str, user_id: u64) -> String {
    format!("device-sync:{request_id}:{user_id}")
}

fn derive_passphrase_wrap_key(
    passphrase: &str,
    salt: &[u8],
    params: Argon2KdfParams,
) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
    let argon2_params = validate_argon2_params(params)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);
    let mut wrap_key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut *wrap_key)
        .map_err(|e| BotBrowserError::Crypto(format!("Argon2id derivation failed: {e}")))?;
    Ok(wrap_key)
}

fn derive_recovery_wrap_key(
    user_id: u64,
    recovery_raw: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
    let hk = Hkdf::<Sha256>::new(Some(USER_VAULT_RECOVERY_HKDF_SALT), recovery_raw);
    let info = build_recovery_root_aad(user_id);
    let mut wrap_key = Zeroizing::new([0u8; 32]);
    hk.expand(info.as_bytes(), &mut *wrap_key).map_err(|e| {
        BotBrowserError::Crypto(format!("recovery key HKDF derivation failed: {e}"))
    })?;
    Ok(wrap_key)
}

fn wrap_root_key_with_aad(
    wrap_key: &[u8; 32],
    root_key: &[u8; 32],
    aad: &str,
) -> Result<String, BotBrowserError> {
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| {
        BotBrowserError::Crypto(format!("failed to generate key-wrap nonce: {e}"))
    })?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(wrap_key));
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: root_key,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|e| BotBrowserError::Crypto(format!("root key wrapping failed: {e}")))?;

    let mut combined = Zeroizing::new(Vec::with_capacity(12 + ciphertext.len()));
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64_STANDARD.encode(&*combined))
}

fn unwrap_root_key_with_aad(
    wrap_key: &[u8; 32],
    wrapped_b64: &str,
    aad: &str,
) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
    let combined = BASE64_STANDARD
        .decode(wrapped_b64.trim())
        .map_err(|e| BotBrowserError::Crypto(format!("invalid wrapped root key base64: {e}")))?;
    if combined.len() != 12 + 32 + 16 {
        return Err(BotBrowserError::Crypto(format!(
            "unexpected wrapped root key length: {} bytes",
            combined.len()
        )));
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(wrap_key));
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
                    "failed to unwrap VaultRootKey (wrong passphrase, recovery key, or user)"
                        .to_string(),
                )
            })?,
    );
    if plaintext.len() != 32 {
        return Err(BotBrowserError::Crypto(format!(
            "unwrapped VaultRootKey has unexpected length: {} bytes",
            plaintext.len()
        )));
    }
    let mut root_key = Zeroizing::new([0u8; 32]);
    root_key.copy_from_slice(&plaintext);
    Ok(root_key)
}

fn format_recovery_key(raw: &[u8; 32]) -> Zeroizing<String> {
    let encoded = Zeroizing::new(BASE32_NOPAD.encode(raw));
    let mut grouped = Zeroizing::new(String::with_capacity(encoded.len() + encoded.len() / 4));
    for (idx, ch) in encoded.chars().enumerate() {
        if idx > 0 && idx % 4 == 0 {
            grouped.push('-');
        }
        grouped.push(ch);
    }
    grouped
}

fn format_emergency_kit_key(raw: &[u8; 32]) -> Zeroizing<String> {
    let grouped = format_recovery_key(raw);
    let mut prefixed = Zeroizing::new(String::with_capacity(3 + grouped.len()));
    prefixed.push_str("EK-");
    prefixed.push_str(&grouped);
    prefixed
}

fn parse_recovery_key(recovery_key: &str) -> Result<Zeroizing<[u8; 32]>, BotBrowserError> {
    let cleaned: Zeroizing<String> = Zeroizing::new(
        recovery_key
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect(),
    );
    if cleaned.is_empty() {
        return Err(BotBrowserError::Crypto(
            "recovery key cannot be empty".to_string(),
        ));
    }
    let payload = if cleaned.len() == 54 && cleaned.starts_with("EK") {
        &cleaned[2..]
    } else {
        &cleaned[..]
    };
    let decoded = Zeroizing::new(BASE32_NOPAD.decode(payload.as_bytes()).map_err(|e| {
        BotBrowserError::Crypto(format!("invalid recovery key format: {e}"))
    })?);
    if decoded.len() != 32 {
        return Err(BotBrowserError::Crypto(format!(
            "invalid recovery key decoded length: {} bytes (expected 32)",
            decoded.len()
        )));
    }
    let mut raw = Zeroizing::new([0u8; 32]);
    raw.copy_from_slice(&decoded);
    Ok(raw)
}

fn should_skip_profile_entry(file_name: &str, is_dir: bool) -> bool {
    if is_dir {
        return matches!(
            file_name,
            "Cache"
                | "Code Cache"
                | "GPUCache"
                | "ShaderCache"
                | "GrShaderCache"
                | "DawnCache"
                | "Crashpad"
                | "BrowserMetrics"
                | "blob_storage"
        );
    }
    matches!(
        file_name,
        "SingletonLock" | "SingletonCookie" | "SingletonSocket" | "LOCK" | "lock" | ".DS_Store"
    ) || file_name.ends_with(".lock")
        || file_name.ends_with(".lck")
        || file_name.ends_with(".tmp")
}

fn append_storage_dir_recursive<W: Write>(
    tar: &mut Builder<W>,
    root_dir: &Path,
    current_dir: &Path,
) -> Result<(), BotBrowserError> {
    let mut entries: Vec<_> = fs::read_dir(current_dir)?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip_profile_entry(&name, file_type.is_dir()) {
            continue;
        }

        let rel = path.strip_prefix(root_dir).map_err(|e| {
            BotBrowserError::Storage(format!("failed computing relative profile path: {e}"))
        })?;
        let mut tar_rel = PathBuf::from(PROFILE_ARCHIVE_STORAGE_PREFIX);
        for comp in rel.components() {
            if let Component::Normal(seg) = comp {
                tar_rel.push(seg);
            }
        }

        if file_type.is_dir() {
            tar.append_dir(&tar_rel, &path)?;
            append_storage_dir_recursive(tar, root_dir, &path)?;
        } else if file_type.is_file() {
            tar.append_path_with_name(&path, &tar_rel)?;
        }
    }
    Ok(())
}

fn validate_raw_archive_path_bytes(raw_bytes: &[u8], raw_lossy: &str) -> Result<(), BotBrowserError> {
    if raw_bytes.is_empty() {
        return Err(BotBrowserError::Storage(
            "empty path in browser profile archive".to_string(),
        ));
    }
    if raw_bytes.contains(&0) {
        return Err(BotBrowserError::Storage(
            "null byte in browser profile archive path".to_string(),
        ));
    }
    if raw_bytes.starts_with(b"/") || raw_bytes.starts_with(b"\\") {
        return Err(BotBrowserError::Storage(format!(
            "rejected absolute path `{raw_lossy}` in browser profile archive"
        )));
    }
    if raw_bytes.len() >= 2 && raw_bytes[0].is_ascii_alphabetic() && raw_bytes[1] == b':' {
        return Err(BotBrowserError::Storage(format!(
            "rejected drive-prefixed path `{raw_lossy}` in browser profile archive"
        )));
    }
    if raw_lossy.contains('\\') {
        return Err(BotBrowserError::Storage(format!(
            "rejected backslash path `{raw_lossy}` in browser profile archive"
        )));
    }
    Ok(())
}

fn validate_and_normalize_archive_path(path: &Path) -> Result<PathBuf, BotBrowserError> {
    let mut normalized = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(seg) => normalized.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BotBrowserError::Storage(format!(
                    "rejected path traversal `{}` in browser profile archive",
                    path.display()
                )));
            }
        }
    }
    Ok(normalized)
}

fn safe_join_under(base_dir: &Path, rel: &Path) -> Result<PathBuf, BotBrowserError> {
    let mut current = base_dir.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(seg) => {
                current.push(seg);
                if let Ok(meta) = fs::symlink_metadata(&current) {
                    if meta.file_type().is_symlink() {
                        return Err(BotBrowserError::Storage(format!(
                            "refusing to traverse symlink `{}` inside target storage dir",
                            current.display()
                        )));
                    }
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BotBrowserError::Storage(format!(
                    "rejected unsafe relative path `{}`",
                    rel.display()
                )));
            }
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unlocked_user_vault_key_file_roundtrip_and_permissions() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let key_path = tmp.path().join("nested").join("user_vault_root.key");

        assert!(
            UnlockedUserVaultKey::load_from_file(&key_path)
                .expect("load missing")
                .is_none()
        );

        let raw_bytes = [0x42u8; 32];
        let key = UnlockedUserVaultKey::from_bytes(raw_bytes);
        key.save_to_file(&key_path).expect("save key file");

        let loaded = UnlockedUserVaultKey::load_from_file(&key_path)
            .expect("load saved")
            .expect("key present");
        assert_eq!(loaded.as_bytes(), &raw_bytes);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&key_path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        UnlockedUserVaultKey::delete_file(&key_path).expect("delete key file");
        assert!(!key_path.exists());
        UnlockedUserVaultKey::delete_file(&key_path).expect("idempotent delete");
    }
}
