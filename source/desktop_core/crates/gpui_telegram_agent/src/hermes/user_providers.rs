//! Per-user isolated AI Provider Router registry with AEAD encryption at rest.
//!
//! Each authenticated user owns an independent `ProviderRouter` instance stored
//! under `"user_providers_state"` / `<user_id>` in `HermesStateStore`.
//!
//! Security guarantees:
//! 1. **Strict tenant isolation**: User A's synced Vertex AI ADC, Codex OAuth,
//!    custom API keys, routing groups, masking rules, and active model selection
//!    are NEVER visible or callable by User B.
//! 2. **Authenticated encryption at rest**: Persisted `ProviderRouterState`
//!    blobs are encrypted with `ChaCha20Poly1305` using a per-user 256-bit key
//!    derived via `HKDF-SHA256(master_key, salt, user_id)` and bound with
//!    associated data (`AAD = "user_providers_state:<user_id>"`) so ciphertexts
//!    cannot be decrypted or swapped across users.
//! 3. **Safe platform default seeding**: New users start from a clean router
//!    seeded ONLY with non-subscription platform defaults (e.g. `local_hermes`
//!    and operator-configured `.env` providers, never another user's private
//!    subscriptions or synced OAuth credentials).
//! 4. **Seamless cross-device reuse**: Logging in as the same `user_id` from
//!    any device (macOS, Windows, mobile, or after a server restart) reloads
//!    the user's encrypted router state and rehydrates live `VertexTokenManager`
//!    and `CodexTokenManager` instances automatically.

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

use super::providers::{ProviderConfig, ProviderRouter, ProviderRouterState};
use super::state_db::HermesStateStore;

const KV_NAMESPACE: &str = "user_providers_state";
const ENVELOPE_PREFIX: &str = "enc:v1:";
const HKDF_SALT: &[u8] = b"fabushi-user-provider-vault-v1";
const VAULT_KEY_FILENAME: &str = "provider_vault.key";

/// Thread-safe, per-user isolated `ProviderRouter` registry backed by encrypted
/// `HermesStateStore` KV persistence.
pub struct UserProviderRegistry {
    store: HermesStateStore,
    master_key: [u8; 32],
    routers: RwLock<HashMap<String, ProviderRouter>>,
}

impl UserProviderRegistry {
    /// Creates a new registry backed by `store`.
    ///
    /// Key resolution order:
    /// 1. `FABUSHI_PROVIDER_MASTER_KEY` env var (hashed via SHA-256 to 32 bytes).
    /// 2. `<data_dir>/provider_vault.key` (created with `0o600` on first boot).
    /// 3. Ephemeral 256-bit random key for `:memory:` / test stores.
    pub fn new(store: HermesStateStore, data_dir: Option<&Path>) -> Self {
        let master_key = load_or_create_master_key(data_dir);
        Self {
            store,
            master_key,
            routers: RwLock::new(HashMap::new()),
        }
    }

    /// Normalizes a user ID key; returns `None` for blank/whitespace strings.
    pub fn normalize_user_id(user_id: &str) -> Option<String> {
        let trimmed = user_id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// Returns the isolated `ProviderRouter` for `user_id`, loading and
    /// decrypting persisted state from SQLite on first access, or seeding a
    /// clean router from `platform_base`'s non-subscription defaults.
    pub fn get_or_load(&self, user_id: &str, platform_base: &ProviderRouter) -> ProviderRouter {
        let Some(uid) = Self::normalize_user_id(user_id) else {
            return platform_base.clone();
        };

        if let Ok(guard) = self.routers.read() {
            if let Some(existing) = guard.get(&uid) {
                return existing.clone();
            }
        }

        let loaded = self.load_or_seed_router(&uid, platform_base);
        let mut guard = self.routers.write().unwrap_or_else(|e| e.into_inner());
        guard.entry(uid).or_insert(loaded).clone()
    }

    /// Persists the current state of `user_id`'s `ProviderRouter` (including
    /// `synced_vertex_adcs` and `synced_codex_oauths`) encrypted at rest into
    /// SQLite.
    pub fn persist(&self, user_id: &str) -> Result<(), String> {
        let Some(uid) = Self::normalize_user_id(user_id) else {
            return Ok(());
        };
        let router = {
            let guard = self.routers.read().unwrap_or_else(|e| e.into_inner());
            guard.get(&uid).cloned()
        };
        let Some(router) = router else {
            return Ok(());
        };
        let snap = router.snapshot();
        let plaintext = serde_json::to_vec(&snap)
            .map_err(|e| format!("Failed to serialize user provider state: {e}"))?;
        let sealed = self.seal_for_user(&uid, &plaintext)?;
        self.store
            .kv_set(KV_NAMESPACE, &uid, &sealed)
            .map_err(|e| format!("Failed to persist encrypted user provider state: {e}"))
    }

    /// Evicts `user_id`'s cached router and clears its persisted vault entry.
    pub fn reset_user(&self, user_id: &str) {
        let Some(uid) = Self::normalize_user_id(user_id) else {
            return;
        };
        if let Ok(mut guard) = self.routers.write() {
            guard.remove(&uid);
        }
        let _ = self.store.kv_set(KV_NAMESPACE, &uid, "");
    }

    /// Evicts in-memory cache only (forces reload from encrypted SQLite on next access).
    pub fn evict_cache(&self, user_id: &str) {
        if let Some(uid) = Self::normalize_user_id(user_id) {
            if let Ok(mut guard) = self.routers.write() {
                guard.remove(&uid);
            }
        }
    }

    fn load_or_seed_router(&self, uid: &str, platform_base: &ProviderRouter) -> ProviderRouter {
        let router = seed_from_platform_base(platform_base);
        if let Ok(Some(raw)) = self.store.kv_get(KV_NAMESPACE, uid) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match self.open_for_user(uid, trimmed) {
                    Ok(bytes) => match serde_json::from_slice::<ProviderRouterState>(&bytes) {
                        Ok(saved) => {
                            router.sync_from_state(&saved);
                        }
                        Err(e) => {
                            tracing::warn!(
                                user_id = %uid,
                                error = %e,
                                "Corrupt deserialized user provider state; starting from clean platform base"
                            );
                        }
                    },
                    Err(e) => {
                        tracing::warn!(
                            user_id = %uid,
                            error = %e,
                            "Failed to decrypt user provider state; starting from clean platform base"
                        );
                    }
                }
            }
        }
        router
    }

    fn derive_user_key(&self, uid: &str) -> [u8; 32] {
        let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), &self.master_key);
        let mut okm = [0u8; 32];
        hk.expand(format!("user:{uid}").as_bytes(), &mut okm)
            .expect("32-byte HKDF output is always valid");
        okm
    }

    pub fn seal_for_user(&self, uid: &str, plaintext: &[u8]) -> Result<String, String> {
        let user_key = self.derive_user_key(uid);
        let cipher = ChaCha20Poly1305::new_from_slice(&user_key)
            .map_err(|e| format!("cipher init failed: {e}"))?;
        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes)
            .map_err(|e| format!("CSPRNG failure generating nonce: {e}"))?;
        let aad = format!("{KV_NAMESPACE}:{uid}");
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|e| format!("AEAD encryption failed: {e}"))?;
        let mut combined = Vec::with_capacity(12 + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);
        let encoded = base64::engine::general_purpose::STANDARD.encode(combined);
        Ok(format!("{ENVELOPE_PREFIX}{encoded}"))
    }

    pub fn open_for_user(&self, uid: &str, sealed: &str) -> Result<Vec<u8>, String> {
        let payload_b64 = sealed
            .strip_prefix(ENVELOPE_PREFIX)
            .ok_or_else(|| "Unsupported user provider vault envelope format".to_string())?;
        let combined = base64::engine::general_purpose::STANDARD
            .decode(payload_b64)
            .map_err(|e| format!("Invalid base64 in user provider vault: {e}"))?;
        if combined.len() < 12 + 16 {
            return Err("Truncated AEAD ciphertext in user provider vault".to_string());
        }
        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let user_key = self.derive_user_key(uid);
        let cipher = ChaCha20Poly1305::new_from_slice(&user_key)
            .map_err(|e| format!("cipher init failed: {e}"))?;
        let aad = format!("{KV_NAMESPACE}:{uid}");
        cipher
            .decrypt(
                Nonce::from_slice(nonce_bytes),
                Payload {
                    msg: ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "AEAD authentication failed (wrong key or tampered user vault)".to_string())
    }
}

/// Seeds a fresh per-user `ProviderRouter` using ONLY non-subscription platform
/// defaults from `platform_base`. Never copies any user's synced OAuth tokens
/// (`synced_vertex_adcs`, `synced_codex_oauths`) or local subscriptions.
fn seed_from_platform_base(platform_base: &ProviderRouter) -> ProviderRouter {
    let base_snap = platform_base.snapshot();
    let router = ProviderRouter::new();
    router.set_allow_local_fallback(base_snap.allow_local_fallback);
    router.set_routing_strategy(base_snap.routing_strategy);

    // Copy only non-subscription platform providers (e.g., operator env keys or test mocks).
    for cfg in base_snap.providers.values() {
        if cfg.id == "local_hermes" || cfg.is_subscription {
            continue;
        }
        let clean: ProviderConfig = cfg.clone();
        router.register_provider(clean);
    }

    // Preserve active platform default if it is a non-subscription provider.
    if !base_snap.active_provider_id.is_empty()
        && base_snap.active_provider_id != "local_hermes"
        && base_snap
            .providers
            .get(&base_snap.active_provider_id)
            .is_some_and(|p| !p.is_subscription && p.enabled)
    {
        let model_opt = (!base_snap.active_model.trim().is_empty())
            .then_some(base_snap.active_model.as_str());
        let _ = router.set_active(&base_snap.active_provider_id, model_opt);
    }

    router
}

fn load_or_create_master_key(data_dir: Option<&Path>) -> [u8; 32] {
    if let Ok(env_secret) = std::env::var("FABUSHI_PROVIDER_MASTER_KEY") {
        let trimmed = env_secret.trim();
        if !trimmed.is_empty() {
            let mut hasher = Sha256::new();
            hasher.update(b"fabushi-provider-master-key-env-v1:");
            hasher.update(trimmed.as_bytes());
            return hasher.finalize().into();
        }
    }

    if let Some(dir) = data_dir {
        let key_path = dir.join(VAULT_KEY_FILENAME);
        if let Ok(raw) = std::fs::read_to_string(&key_path) {
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(raw.trim()) {
                if decoded.len() == 32 {
                    let mut key = [0u8; 32];
                    key.copy_from_slice(&decoded);
                    return key;
                }
            }
        }
        let mut key = [0u8; 32];
        if getrandom::getrandom(&mut key).is_ok() {
            let _ = std::fs::create_dir_all(dir);
            let encoded = base64::engine::general_purpose::STANDARD.encode(key);
            if write_secret_file(&key_path, &encoded).is_ok() {
                return key;
            }
        }
    }

    let mut ephemeral = [0u8; 32];
    let _ = getrandom::getrandom(&mut ephemeral);
    ephemeral
}

fn write_secret_file(path: &Path, content: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(content.as_bytes())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hermes::providers::CredentialEntry;

    #[test]
    fn user_provider_isolation_and_encrypted_persistence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("hermes.db");
        let store = HermesStateStore::open(&db_path).expect("open store");
        let base = ProviderRouter::new();
        base.set_allow_local_fallback(false);

        let reg = UserProviderRegistry::new(store.clone(), Some(dir.path()));

        // User A configures a secret provider
        let router_a = reg.get_or_load("100001", &base);
        let mut cfg_a = ProviderConfig::openai_compatible(
            "my_openai",
            "Alice OpenAI",
            "https://api.openai.com/v1",
            "gpt-4o",
            "sk-alice-super-secret-token-9999",
        );
        cfg_a.credentials.entries = vec![CredentialEntry::new(
            "k1",
            "sk-alice-super-secret-token-9999",
            "user",
        )];
        router_a.register_provider(cfg_a);
        router_a.set_active("my_openai", Some("gpt-4o")).unwrap();
        reg.persist("100001").expect("persist Alice");

        // Verify stored blob in SQLite is encrypted (no plaintext key!)
        let raw_blob = store
            .kv_get(KV_NAMESPACE, "100001")
            .unwrap()
            .expect("blob stored");
        assert!(raw_blob.starts_with(ENVELOPE_PREFIX));
        assert!(
            !raw_blob.contains("sk-alice-super-secret-token-9999"),
            "plaintext API key must NEVER appear in SQLite!"
        );

        // Verify cross-user AAD tampering fails decryption
        assert!(
            reg.open_for_user("100002", &raw_blob).is_err(),
            "User B must NOT be able to decrypt User A's vault blob"
        );

        // User B gets a completely clean router with ZERO of User A's providers
        let router_b = reg.get_or_load("100002", &base);
        assert!(
            !router_b.snapshot().providers.contains_key("my_openai"),
            "User B must NOT see User A's providers"
        );
        assert!(router_b.needs_provider_selection());

        // Simulate full server restart (reopen store & registry from same data_dir)
        drop(reg);
        let store2 = HermesStateStore::open(&db_path).expect("reopen store");
        let reg2 = UserProviderRegistry::new(store2, Some(dir.path()));
        let router_a2 = reg2.get_or_load("100001", &base);
        let snap_a2 = router_a2.snapshot();
        assert_eq!(snap_a2.active_provider_id, "my_openai");
        assert_eq!(snap_a2.active_model, "gpt-4o");
        assert!(snap_a2.providers.contains_key("my_openai"));
        assert_eq!(
            snap_a2.providers["my_openai"].credentials.entries[0].api_key,
            "sk-alice-super-secret-token-9999"
        );
    }
}
