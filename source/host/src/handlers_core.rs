use argon2::{Algorithm, Argon2, Params as Argon2Params, Version};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use chrono::Utc;
use gpui_telegram_client::client::TelegramClient;
use gpui_telegram_client::config::{ClientConfig, DEFAULT_SERVER_URL};
use gpui_telegram_client::net::proxy::ClientProxyConfig;
use gpui_telegram_client::net::{
    AuthResponse, ChangePasswordRequest, EmailRegisterRequest, LoginRequest, PhoneVerifyRequest,
    QrPairPollResponse, RegisterRequest, SplitKeyEmailRegisterRequest, SplitKeyLoginRequest,
    SplitKeyRegisterRequest, StarGiftRequest, TwoFactorSetRequest, TwoFactorVerifyRequest,
    UpgradeSplitKeyRequest, UserPrivacySettingsDto,
};
use gpui_telegram_core::action_bus::{
    ActionCategory, ActionContext, CliExporter, HermesToolExporter, McpExporter, OpenApiExporter,
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use subtle::ConstantTimeEq as _;
use zeroize::Zeroizing;

use crate::error::{HostError, HostResult};
use crate::host::{validate_argon2_ios_ceiling, FabushiAppHost, RamAccountToken};
use crate::storage::{compute_storage_metrics, purge_media_cas_dir};

const ACCOUNT_TOKEN_HKDF_SALT: &[u8] = b"fabushi-ios:host-kv:account-token-salt:v1";
const ACCOUNT_TOKEN_HKDF_INFO: &[u8] = b"fabushi-ios:host-kv:account-token-aead:v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccountRecord {
    user_id: u64,
    username: String,
    session_id: String,
    first_name: Option<String>,
    created_at: Option<String>,
    expires_at: Option<String>,
    server_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    encrypted_token_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce_b64: Option<String>,
}

fn derive_account_aead_key(passphrase: &str) -> HostResult<Zeroizing<[u8; 32]>> {
    let hk = Hkdf::<Sha256>::new(Some(ACCOUNT_TOKEN_HKDF_SALT), passphrase.as_bytes());
    let mut key = Zeroizing::new([0u8; 32]);
    hk.expand(ACCOUNT_TOKEN_HKDF_INFO, &mut *key)
        .map_err(|e| HostError::crypto(format!("HKDF derivation failed: {e}")))?;
    Ok(key)
}

fn persist_account_record(host: &FabushiAppHost, auth: &AuthResponse) -> HostResult<()> {
    let server_url = host.current_server_url();
    let account_id = auth.user_id.to_string();

    let (encrypted_token_b64, nonce_b64) = if let Some(ref pass) = host.storage_passphrase {
        if !auth.token.trim().is_empty() {
            let key = derive_account_aead_key(pass.as_str())?;
            let cipher = ChaCha20Poly1305::new_from_slice(&*key)
                .map_err(|e| HostError::crypto(format!("AEAD init failed: {e}")))?;
            let mut nonce_bytes = [0u8; 12];
            getrandom::getrandom(&mut nonce_bytes)
                .map_err(|e| HostError::crypto(format!("CSPRNG failed: {e}")))?;
            let aad = format!("fabushi-account-token:v1|{}|{}", auth.user_id, server_url);
            let ct = cipher
                .encrypt(
                    Nonce::from_slice(&nonce_bytes),
                    Payload {
                        msg: auth.token.as_bytes(),
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|e| HostError::crypto(format!("Token encryption failed: {e}")))?;
            (
                Some(BASE64_STANDARD.encode(ct)),
                Some(BASE64_STANDARD.encode(nonce_bytes)),
            )
        } else {
            (None, None)
        }
    } else {
        // Without a Keychain storage_passphrase, NEVER persist raw bearer tokens in SQLite:
        // store strictly non-secret account metadata on disk while token stays RAM-only.
        (None, None)
    };

    let record = StoredAccountRecord {
        user_id: auth.user_id,
        username: auth.username.clone(),
        session_id: auth.session_id.clone(),
        first_name: auth.first_name.clone(),
        created_at: auth.created_at.clone(),
        expires_at: auth.expires_at.clone(),
        server_url: server_url.clone(),
        encrypted_token_b64,
        nonce_b64,
    };
    let encoded = serde_json::to_string(&record)
        .map_err(|e| HostError::internal(format!("Account record serialization failed: {e}")))?;
    // Persist SQLite account record STRICTLY BEFORE caching token in RAM so a SQLite write
    // failure never leaves a stale RAM bearer token behind (B4-02).
    host.host_db.kv_set("accounts", &account_id, &encoded)?;

    if !auth.token.trim().is_empty() {
        host.account_tokens.lock().insert(
            account_id,
            RamAccountToken {
                server_url,
                token: Zeroizing::new(auth.token.clone()),
            },
        );
    }
    Ok(())
}

fn restore_account_auth(
    host: &FabushiAppHost,
    account_id: &str,
    raw: &str,
) -> HostResult<AuthResponse> {
    let rec: StoredAccountRecord = serde_json::from_str(raw)
        .map_err(|e| HostError::storage(format!("Corrupt stored account record: {e}")))?;
    let current_server_url = host.current_server_url();
    if rec.server_url != current_server_url {
        return Err(HostError::unauthorized(
            "Stored account belongs to a different server URL; re-authentication required",
        ));
    }

    let token = if let (Some(ct_b64), Some(nonce_b64)) =
        (rec.encrypted_token_b64.as_deref(), rec.nonce_b64.as_deref())
    {
        if let Some(ref pass) = host.storage_passphrase {
            let key = derive_account_aead_key(pass.as_str())?;
            let cipher = ChaCha20Poly1305::new_from_slice(&*key)
                .map_err(|e| HostError::crypto(format!("AEAD init failed: {e}")))?;
            let ct = BASE64_STANDARD
                .decode(ct_b64.trim())
                .map_err(|e| HostError::crypto(format!("Invalid encrypted_token_b64: {e}")))?;
            let nonce_vec = BASE64_STANDARD
                .decode(nonce_b64.trim())
                .map_err(|e| HostError::crypto(format!("Invalid nonce_b64: {e}")))?;
            if nonce_vec.len() != 12 {
                return Err(HostError::crypto("Corrupt account token nonce length"));
            }
            let aad = format!("fabushi-account-token:v1|{}|{}", rec.user_id, current_server_url);
            let plain = Zeroizing::new(
                cipher
                    .decrypt(
                        Nonce::from_slice(&nonce_vec),
                        Payload {
                            msg: &ct,
                            aad: aad.as_bytes(),
                        },
                    )
                    .map_err(|_| HostError::crypto("Failed to decrypt stored account token"))?,
            );
            String::from_utf8(plain.to_vec())
                .map_err(|e| HostError::crypto(format!("Decrypted token is invalid UTF-8: {e}")))?
        } else if let Some(mem_tok) = host
            .account_tokens
            .lock()
            .get(account_id)
            .filter(|entry| entry.server_url == current_server_url)
        {
            mem_tok.token.as_str().to_string()
        } else {
            return Err(HostError::unauthorized(
                "Account token is encrypted and storage_passphrase is unavailable",
            ));
        }
    } else if let Some(mem_tok) = host
        .account_tokens
        .lock()
        .get(account_id)
        .filter(|entry| entry.server_url == current_server_url)
    {
        mem_tok.token.as_str().to_string()
    } else {
        return Err(HostError::unauthorized(
            "Stored account has no recoverable session token; re-authentication required",
        ));
    };

    if token.trim().is_empty() {
        return Err(HostError::unauthorized(
            "Stored account has no recoverable session token; re-authentication required",
        ));
    }

    Ok(AuthResponse {
        user_id: rec.user_id,
        username: rec.username,
        session_id: rec.session_id,
        token,
        first_name: rec.first_name,
        created_at: rec.created_at,
        expires_at: rec.expires_at,
    })
}

/// Canonical account-boundary commit for ALL session-establishing authentication RPCs.
/// Fences against concurrent generation changes / server switches, enforces the strict 5-step
/// commit sequence inside `session_lock.write()` (clear -> reset_account_scope -> SQLite persist ->
/// bump_generation -> install live credentials LAST) so zero Account A RPCs can ever observe
/// Account B's bearer token during SQLite I/O or across OS preemption (B4-02).
fn finalize_authenticated_session(
    host: &FabushiAppHost,
    client: &std::sync::Arc<TelegramClient>,
    start_gen: u64,
    auth: &AuthResponse,
    generation_advanced: &mut bool,
) -> HostResult<u64> {
    if auth.user_id == 0 {
        return Err(HostError::invalid_arguments(
            "Authenticated session requires a non-zero user_id",
        ));
    }
    let _sess_guard = host.session_lock.write();
    if host.is_shutdown() {
        return Err(HostError::host_closed());
    }
    let current_gen = host.current_generation();
    let live_client = host.client();
    if current_gen != start_gen || !std::sync::Arc::ptr_eq(client, &live_client) {
        return Err(HostError::stale_generation(start_gen, current_gen));
    }
    let target_owner = format!("{}|{}", host.current_server_url(), auth.user_id);
    let prev_owner = host
        .host_db
        .get_meta("local_store_owner_scope")
        .ok()
        .flatten();
    let needs_local_wipe = prev_owner.as_deref() != Some(target_owner.as_str());

    // Step 1: Clear or suspend live client session (stops & drains OutboxWorker/WsTransport/Tailnet; token == None)
    if needs_local_wipe {
        live_client.clear_session();
    } else {
        live_client.suspend_session_preserving_stores();
    }
    // Step 2: Drain stale WS frames, wipe runtime account scope, and if local_store_owner_scope
    // does not match "{server_url}|{user_id}" (covering cross-user switch, token-only cold restart,
    // iOS Jetsam crash, and unauthenticated staging), wipe all account-scoped local data BEFORE
    // Step 3 SQLite I/O while preserving same-user re-login caches (B6-01, NB6-01, NB6-02).
    let _ = live_client.poll_events();
    host.reset_account_scope();
    if needs_local_wipe {
        let _ = host.host_db.delete_meta("local_store_owner_scope");
        host.clear_account_local_data(&live_client);
    }
    // Step 3: Execute ALL fallible SQLite persistence while live_client.token() is STILL None
    if let Err(err) = persist_account_record(host, auth) {
        let _ = host.host_db.delete_meta("active_account_id");
        if needs_local_wipe {
            let _ = host.host_db.delete_meta("local_store_owner_scope");
            host.clear_account_local_data(&live_client);
        }
        host.bump_generation();
        *generation_advanced = true;
        return Err(err);
    }
    if let Err(err) = host
        .host_db
        .set_meta("active_account_id", &auth.user_id.to_string())
    {
        let _ = host.host_db.delete_meta("active_account_id");
        if needs_local_wipe {
            let _ = host.host_db.delete_meta("local_store_owner_scope");
            host.clear_account_local_data(&live_client);
        }
        host.bump_generation();
        *generation_advanced = true;
        return Err(err);
    }
    if let Err(err) = host
        .host_db
        .set_meta("local_store_owner_scope", &target_owner)
    {
        let _ = host.host_db.delete_meta("active_account_id");
        if needs_local_wipe {
            let _ = host.host_db.delete_meta("local_store_owner_scope");
            host.clear_account_local_data(&live_client);
        }
        host.bump_generation();
        *generation_advanced = true;
        return Err(err);
    }
    // Step 4: Advance generation while live_client.token() is STILL None
    let new_gen = host.bump_generation();
    *generation_advanced = true;
    // Step 5: Infallible live credential installation LAST strictly inside session_lock.write()
    // (calls bind_account_scope, preserving same-user cache and starting OutboxWorker)
    live_client.set_auth_session(auth.clone());
    Ok(new_gen)
}

fn finalize_token_only_session(
    host: &FabushiAppHost,
    client: &std::sync::Arc<TelegramClient>,
    start_gen: u64,
    token: &str,
    generation_advanced: &mut bool,
) -> HostResult<u64> {
    let _sess_guard = host.session_lock.write();
    if host.is_shutdown() {
        return Err(HostError::host_closed());
    }
    let current_gen = host.current_generation();
    let live_client = host.client();
    if current_gen != start_gen || !std::sync::Arc::ptr_eq(client, &live_client) {
        return Err(HostError::stale_generation(start_gen, current_gen));
    }
    // Step 1: Clear live client session (stops & drains OutboxWorker/WsTransport/Tailnet; token == None)
    live_client.clear_session();
    // Step 2: Wipe all account-scoped cryptographic keys, runtime state, owner scope marker,
    // and account-scoped local data because a token-only session has no user_id to verify cache
    // ownership (B5-02, B6-01, NB6-01, NB6-02).
    host.reset_account_scope();
    let _ = host.host_db.delete_meta("local_store_owner_scope");
    host.clear_account_local_data(&live_client);
    // Step 3: Execute fallible SQLite persistence while live_client.token() is STILL None
    if let Err(err) = host.host_db.delete_meta("active_account_id") {
        host.bump_generation();
        *generation_advanced = true;
        return Err(err);
    }
    // Step 4: Advance generation while live_client.token() is STILL None
    let new_gen = host.bump_generation();
    *generation_advanced = true;
    // Step 5: Infallible live token installation LAST strictly inside session_lock.write()
    live_client.set_token(Some(token.to_string()));
    Ok(new_gen)
}

fn finalize_qr_pair_response(
    host: &FabushiAppHost,
    client: &std::sync::Arc<TelegramClient>,
    start_gen: u64,
    res: &QrPairPollResponse,
    generation_advanced: &mut bool,
) -> HostResult<()> {
    if let Some(ref sess) = res.session {
        let auth = AuthResponse::from(sess.clone());
        finalize_authenticated_session(host, client, start_gen, &auth, generation_advanced)?;
    } else if let Some(tok) = res
        .token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(ref u) = res.user {
            let auth = AuthResponse {
                user_id: u.user_id,
                username: u.username.clone().unwrap_or_default(),
                session_id: String::new(),
                token: tok.to_string(),
                first_name: Some(u.first_name.clone()),
                created_at: None,
                expires_at: None,
            };
            finalize_authenticated_session(host, client, start_gen, &auth, generation_advanced)?;
        } else {
            finalize_token_only_session(host, client, start_gen, tok, generation_advanced)?;
        }
    }
    Ok(())
}

fn parse_req<T: serde::de::DeserializeOwned>(params: &Value) -> HostResult<T> {
    serde_json::from_value(params.clone())
        .map_err(|e| HostError::invalid_arguments(format!("Invalid request parameters: {e}")))
}

fn req_str<'a>(params: &'a Value, key: &str) -> HostResult<&'a str> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required string '{key}'")))
}

/// Non-trimming string extractor for secrets, passwords, and passcodes so leading/trailing spaces are preserved.
fn req_raw_str<'a>(params: &'a Value, key: &str) -> HostResult<&'a str> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required string '{key}'")))
}

fn to_val<T: serde::Serialize>(v: T) -> HostResult<Value> {
    serde_json::to_value(v).map_err(|e| HostError::internal(format!("Serialization error: {e}")))
}

pub fn handle_auth(
    host: &FabushiAppHost,
    req_id: &str,
    start_gen: u64,
    action: &str,
    params: &Value,
    generation_advanced: &mut bool,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "health" => {
            let ok = host.run_cancellable(req_id, async move {
                client.check_health().await.map_err(HostError::from)
            })?;
            Ok(json!({ "healthy": ok, "server_url": host.current_server_url() }))
        }
        "status" => {
            let st = host.run_cancellable(req_id, async move {
                client.fetch_server_status().await.map_err(HostError::from)
            })?;
            to_val(st)
        }
        "login" => {
            let req: LoginRequest = if params.get("device_name").is_some() {
                parse_req(params)?
            } else {
                LoginRequest {
                    username: req_str(params, "username")?.to_string(),
                    password: req_raw_str(params, "password")?.to_string(),
                    device_name: params
                        .get("device_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Fabushi iOS")
                        .to_string(),
                }
            };
            let c = std::sync::Arc::clone(&client);
            let auth = host.run_cancellable(req_id, async move {
                c.http_client.login(&req).await.map_err(HostError::from)
            })?;
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(auth)
        }
        "register" => {
            let req: RegisterRequest = parse_req(params)?;
            let c = std::sync::Arc::clone(&client);
            let auth = host.run_cancellable(req_id, async move {
                c.http_client.register(&req).await.map_err(HostError::from)
            })?;
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(auth)
        }
        "logout" => {
            let token_opt = host.fenced_client_token(&client)?;
            let pre_uid = client.current_user_id().map(|id| id.to_string());
            let c_logout = std::sync::Arc::clone(&client);
            let res = if let Some(tok) = token_opt {
                host.run_cancellable(req_id, async move {
                    c_logout
                        .http_client
                        .logout(&tok)
                        .await
                        .map_err(HostError::from)
                })
            } else {
                Err(HostError::unauthorized("No active session token"))
            };
            let new_gen = {
                let _sess_guard = host.session_lock.write();
                if host.is_shutdown() {
                    return Err(HostError::host_closed());
                }
                let current_gen = host.current_generation();
                let live_client = host.client();
                if current_gen != start_gen || !std::sync::Arc::ptr_eq(&client, &live_client) {
                    return Err(HostError::stale_generation(start_gen, current_gen));
                }
                if let Err(ref err) = res {
                    if err.code == "cancelled" {
                        return Err(err.clone());
                    }
                }
                let target_uid = pre_uid
                    .or_else(|| live_client.current_user_id().map(|id| id.to_string()))
                    .or_else(|| {
                        host.host_db
                            .get_meta("active_account_id")
                            .ok()
                            .flatten()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                    });
                live_client.clear_session();
                host.reset_account_scope();
                let _ = host.host_db.delete_meta("local_store_owner_scope");
                host.clear_account_local_data(&live_client);
                let mut delete_err = None;
                if let Some(uid) = target_uid {
                    host.account_tokens.lock().remove(&uid);
                    if let Err(err) = host.host_db.kv_delete("accounts", &uid) {
                        delete_err = Some(err);
                    }
                }
                let _ = host.host_db.delete_meta("active_account_id");
                let next_gen = host.bump_generation();
                *generation_advanced = true;
                if let Some(err) = delete_err {
                    return Err(err);
                }
                next_gen
            };
            match res {
                Ok(()) => Ok(json!({ "logged_out": true, "generation": new_gen })),
                Err(e) => Err(e),
            }
        }
        "kdfParams" => {
            let ident = req_str(params, "username_or_email")
                .or_else(|_| req_str(params, "username"))?
                .to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_auth_kdf_params(&ident)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "registerSplitKey" => {
            let req: SplitKeyRegisterRequest = parse_req(params)?;
            let c = std::sync::Arc::clone(&client);
            let res = host.run_cancellable(req_id, async move {
                c.http_client
                    .register_split_key(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            let auth = AuthResponse::from(res.clone());
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(res)
        }
        "emailRegisterSplitKey" => {
            let req: SplitKeyEmailRegisterRequest = parse_req(params)?;
            let c = std::sync::Arc::clone(&client);
            let res = host.run_cancellable(req_id, async move {
                c.http_client
                    .email_register_split_key(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            let auth = AuthResponse::from(res.clone());
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(res)
        }
        "loginSplitKey" => {
            let req: SplitKeyLoginRequest = parse_req(params)?;
            let c = std::sync::Arc::clone(&client);
            let res = host.run_cancellable(req_id, async move {
                c.http_client
                    .login_split_key(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            let auth = AuthResponse::from(res.clone());
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(res)
        }
        "upgradeSplitKey" => {
            let req: UpgradeSplitKeyRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .upgrade_split_key(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "changePassword" => {
            let req: ChangePasswordRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.change_password(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "phoneSendCode" => {
            let phone = req_str(params, "phone")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client.phone_send_code(&phone).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "phoneVerify" => {
            let req: PhoneVerifyRequest = parse_req(params)?;
            let c = std::sync::Arc::clone(&client);
            let proto_auth = host.run_cancellable(req_id, async move {
                c.http_client
                    .phone_verify(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            let auth = AuthResponse::from(proto_auth);
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(auth)
        }
        "emailSendCode" => {
            let email = req_str(params, "email")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client.email_send_code(&email).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "emailRegister" => {
            let req: EmailRegisterRequest = parse_req(params)?;
            let c = std::sync::Arc::clone(&client);
            let auth = host.run_cancellable(req_id, async move {
                c.http_client
                    .email_register(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(auth)
        }
        "emailVerify" => {
            let email = req_str(params, "email")?.to_string();
            let code = req_str(params, "code")?.to_string();
            let c = std::sync::Arc::clone(&client);
            let proto_auth = host.run_cancellable(req_id, async move {
                c.http_client
                    .email_verify(&email, &code)
                    .await
                    .map_err(HostError::from)
            })?;
            let auth = AuthResponse::from(proto_auth);
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(auth)
        }
        "alipayStart" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .http_client
                    .alipay_start()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "alipayPoll" => {
            let attempt_id = req_str(params, "attempt_id")?.to_string();
            let c = std::sync::Arc::clone(&client);
            let poll = host.run_cancellable(req_id, async move {
                c.http_client
                    .alipay_poll(&attempt_id)
                    .await
                    .map_err(HostError::from)
            })?;
            if let Some(ref sess) = poll.session {
                finalize_authenticated_session(host, &client, start_gen, sess, generation_advanced)?;
            } else if let Some(tok) = poll
                .token
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Some(uid) = poll
                    .user_id
                    .or_else(|| poll.user.as_ref().map(|u| u.user_id))
                {
                    let auth = AuthResponse {
                        user_id: uid,
                        username: poll
                            .username
                            .clone()
                            .or_else(|| poll.user.as_ref().and_then(|u| u.username.clone()))
                            .unwrap_or_default(),
                        session_id: String::new(),
                        token: tok.to_string(),
                        first_name: poll.user.as_ref().map(|u| u.first_name.clone()),
                        created_at: None,
                        expires_at: None,
                    };
                    finalize_authenticated_session(
                        host,
                        &client,
                        start_gen,
                        &auth,
                        generation_advanced,
                    )?;
                } else {
                    finalize_token_only_session(
                        host,
                        &client,
                        start_gen,
                        tok,
                        generation_advanced,
                    )?;
                }
            }
            to_val(poll)
        }
        "qrPairInit" => {
            let res = host.run_cancellable(req_id, async move {
                client.qr_pair_init().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "qrPairPoll" => {
            let pair_token = req_str(params, "pair_token")?.to_string();
            let c = std::sync::Arc::clone(&client);
            let res = host.run_cancellable(req_id, async move {
                c.http_client
                    .qr_pair_poll(&pair_token)
                    .await
                    .map_err(HostError::from)
            })?;
            finalize_qr_pair_response(host, &client, start_gen, &res, generation_advanced)?;
            to_val(res)
        }
        "qrPairAccept" => {
            let pair_token = req_str(params, "pair_token")?.to_string();
            host.run_cancellable(req_id, async move {
                client
                    .qr_pair_accept(&pair_token)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "accepted": true }))
        }
        "qrPairVerify" => {
            let pair_token = req_str(params, "pair_token")?.to_string();
            let cloud_password = params
                .get("cloud_password")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let c = std::sync::Arc::clone(&client);
            let res = host.run_cancellable(req_id, async move {
                c.http_client
                    .qr_pair_verify(&pair_token, cloud_password.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            finalize_qr_pair_response(host, &client, start_gen, &res, generation_advanced)?;
            to_val(res)
        }
        "passkeyChallenge" => {
            let username = params
                .get("username")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            host.run_cancellable(req_id, async move {
                client
                    .passkey_challenge(username.as_deref())
                    .await
                    .map_err(HostError::from)
            })
        }
        "passkeyVerify" => {
            let body = params.clone();
            let c = std::sync::Arc::clone(&client);
            let proto_auth = host.run_cancellable(req_id, async move {
                c.http_client
                    .passkey_verify(&body)
                    .await
                    .map_err(HostError::from)
            })?;
            let auth = AuthResponse::from(proto_auth);
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(auth)
        }
        "passkeyList" => {
            let list = host.run_cancellable(req_id, async move {
                client.list_passkeys().await.map_err(HostError::from)
            })?;
            to_val(list)
        }
        "passkeyRegister" => {
            let body = params.clone();
            host.run_cancellable(req_id, async move {
                client
                    .register_passkey(&body)
                    .await
                    .map_err(HostError::from)
            })
        }
        "passkeyDelete" => {
            let passkey_id = req_str(params, "passkey_id")?.to_string();
            host.run_cancellable(req_id, async move {
                client
                    .delete_passkey(&passkey_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "deleted": true }))
        }
        "twoFactorStatus" => {
            let res = host.run_cancellable(req_id, async move {
                client.get_2fa_status().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "twoFactorSet" => {
            let req: TwoFactorSetRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.set_2fa(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "twoFactorDisable" => {
            let pwd = params
                .get("current_password")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            host.run_cancellable(req_id, async move {
                client
                    .disable_2fa(pwd.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "disabled": true }))
        }
        "twoFactorVerify" => {
            let req: TwoFactorVerifyRequest = parse_req(params)?;
            let c = std::sync::Arc::clone(&client);
            let proto_auth = host.run_cancellable(req_id, async move {
                c.http_client
                    .verify_2fa(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            let auth = AuthResponse::from(proto_auth);
            finalize_authenticated_session(host, &client, start_gen, &auth, generation_advanced)?;
            to_val(auth)
        }
        "me" => {
            let res = host.run_cancellable(req_id, async move {
                client.get_me().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        _ => Err(HostError::unknown_command(&format!("auth.{action}"))),
    }
}

pub fn handle_session(
    host: &FabushiAppHost,
    req_id: &str,
    start_gen: u64,
    action: &str,
    params: &Value,
    generation_advanced: &mut bool,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "ping" => {
            let nonce = params.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0);
            Ok(json!({
                "pong": true,
                "nonce": nonce,
                "timestamp": Utc::now().timestamp_millis(),
                "server_url": host.current_server_url(),
                "generation": host.current_generation(),
            }))
        }
        "list" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_sessions().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "revoke" => {
            let session_id = req_str(params, "session_id")?.to_string();
            let revoked = host.run_cancellable(req_id, async move {
                client
                    .revoke_session(&session_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "revoked": revoked }))
        }
        "revokeOther" => {
            host.run_cancellable(req_id, async move {
                client
                    .revoke_other_sessions()
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "revoked_others": true }))
        }
        "connectWs" => {
            let (status, server_url) = host.with_fenced_session_read(&client, || {
                client.connect_ws();
                Ok((format!("{:?}", client.status()), host.current_server_url()))
            })?;
            host.emit_event(
                "connectionStateChanged",
                json!({ "status": status, "server_url": server_url }),
            );
            Ok(json!({
                "connecting": true,
                "status": status,
                "server_url": server_url,
            }))
        }
        "disconnectWs" => {
            let (status, server_url) = host.with_fenced_session_read(&client, || {
                client.disconnect_ws();
                Ok((format!("{:?}", client.status()), host.current_server_url()))
            })?;
            host.emit_event(
                "connectionStateChanged",
                json!({ "status": status, "server_url": server_url }),
            );
            Ok(json!({ "disconnected": true, "status": status }))
        }
        "wsStatus" => Ok(json!({
            "connected": client.is_connected(),
            "status": format!("{:?}", client.status()),
            "server_url": host.current_server_url(),
            "device_id": client.device_id(),
            "user_id": client.current_user_id(),
        })),
        "setToken" => {
            let token = params
                .get("token")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            match token {
                Some(tok) => {
                    let new_gen = finalize_token_only_session(
                        host,
                        &client,
                        start_gen,
                        &tok,
                        generation_advanced,
                    )?;
                    Ok(json!({ "has_token": true, "generation": new_gen }))
                }
                None => {
                    let new_gen = {
                        let _sess_guard = host.session_lock.write();
                        if host.is_shutdown() {
                            return Err(HostError::host_closed());
                        }
                        let current_gen = host.current_generation();
                        let live_client = host.client();
                        if current_gen != start_gen
                            || !std::sync::Arc::ptr_eq(&client, &live_client)
                        {
                            return Err(HostError::stale_generation(start_gen, current_gen));
                        }
                        live_client.clear_session();
                        let _ = host.host_db.delete_meta("active_account_id");
                        let _ = host.host_db.delete_meta("local_store_owner_scope");
                        host.reset_account_scope();
                        host.clear_account_local_data(&live_client);
                        let next_gen = host.bump_generation();
                        *generation_advanced = true;
                        next_gen
                    };
                    Ok(json!({ "has_token": false, "generation": new_gen }))
                }
            }
        }
        "setAuthSession" => {
            let auth: AuthResponse = parse_req(params)?;
            if auth.user_id == 0 {
                return Err(HostError::invalid_arguments(
                    "session.setAuthSession requires a non-zero user_id",
                ));
            }
            let new_gen = finalize_authenticated_session(
                host,
                &client,
                start_gen,
                &auth,
                generation_advanced,
            )?;
            Ok(json!({ "user_id": auth.user_id, "generation": new_gen }))
        }
        "clearSession" => {
            let generation = {
                let _sess_guard = host.session_lock.write();
                if host.is_shutdown() {
                    return Err(HostError::host_closed());
                }
                let current_gen = host.current_generation();
                let live_client = host.client();
                if current_gen != start_gen || !std::sync::Arc::ptr_eq(&client, &live_client) {
                    return Err(HostError::stale_generation(start_gen, current_gen));
                }
                live_client.clear_session();
                let _ = host.host_db.delete_meta("active_account_id");
                let _ = host.host_db.delete_meta("local_store_owner_scope");
                host.reset_account_scope();
                host.clear_account_local_data(&live_client);
                let next_gen = host.bump_generation();
                *generation_advanced = true;
                next_gen
            };
            Ok(json!({ "cleared": true, "generation": generation }))
        }
        "switchAccount" => {
            let account_id = req_str(params, "account_id")?;
            let _sess_guard = host.session_lock.write();
            if host.is_shutdown() {
                return Err(HostError::host_closed());
            }
            let current_gen = host.current_generation();
            let live_client = host.client();
            if current_gen != start_gen || !std::sync::Arc::ptr_eq(&client, &live_client) {
                return Err(HostError::stale_generation(start_gen, current_gen));
            }
            let raw = host
                .host_db
                .kv_get("accounts", account_id)?
                .ok_or_else(|| HostError::not_found(format!("account not found: {account_id}")))?;
            // Validate server binding, AEAD integrity, and non-empty recoverable token BEFORE
            // clearing the current live session, wiping account scope, or bumping generation!
            let auth = restore_account_auth(host, account_id, &raw)?;
            if auth.user_id == 0 {
                return Err(HostError::invalid_arguments(
                    "Stored account has invalid zero user_id",
                ));
            }
            let target_owner = format!("{}|{}", host.current_server_url(), auth.user_id);
            let prev_owner = host
                .host_db
                .get_meta("local_store_owner_scope")
                .ok()
                .flatten();
            let needs_local_wipe = prev_owner.as_deref() != Some(target_owner.as_str());
            // Strict 5-step commit order (B4-02, B5-02, B6-01, NB6-01, NB6-02):
            // Step 1: Clear or suspend live client session (stops & drains OutboxWorker/WsTransport/Tailnet)
            if needs_local_wipe {
                live_client.clear_session();
            } else {
                live_client.suspend_session_preserving_stores();
            }
            // Step 2: Drain stale WS frames, wipe runtime account scope & cross-account local data
            let _ = live_client.poll_events();
            host.reset_account_scope();
            if needs_local_wipe {
                let _ = host.host_db.delete_meta("local_store_owner_scope");
                host.clear_account_local_data(&live_client);
            }
            // Step 3: Persist active_account_id & local_store_owner_scope while token() is STILL None
            if let Err(err) = host.host_db.set_meta("active_account_id", account_id) {
                let _ = host.host_db.delete_meta("active_account_id");
                if needs_local_wipe {
                    let _ = host.host_db.delete_meta("local_store_owner_scope");
                    host.clear_account_local_data(&live_client);
                }
                host.bump_generation();
                *generation_advanced = true;
                return Err(err);
            }
            if let Err(err) = host
                .host_db
                .set_meta("local_store_owner_scope", &target_owner)
            {
                let _ = host.host_db.delete_meta("active_account_id");
                if needs_local_wipe {
                    let _ = host.host_db.delete_meta("local_store_owner_scope");
                    host.clear_account_local_data(&live_client);
                }
                host.bump_generation();
                *generation_advanced = true;
                return Err(err);
            }
            // Step 4: Advance generation while live_client.token() is STILL None
            let new_gen = host.bump_generation();
            *generation_advanced = true;
            // Step 5: Infallible live credential installation LAST strictly inside session_lock.write()
            live_client.set_auth_session(auth.clone());
            Ok(json!({
                "switched": true,
                "account_id": account_id,
                "user_id": auth.user_id,
                "generation": new_gen,
            }))
        }
        "listAccounts" => {
            let map = host.host_db.kv_list("accounts")?;
            let active = host.host_db.get_meta("active_account_id")?;
            let accounts: Vec<Value> = map
                .into_iter()
                .map(|(k, v)| {
                    if let Ok(rec) = serde_json::from_str::<StoredAccountRecord>(&v) {
                        json!({
                            "account_id": k,
                            "user_id": rec.user_id,
                            "username": rec.username,
                            "first_name": rec.first_name,
                            "active": active.as_deref() == Some(k.as_str()),
                        })
                    } else {
                        json!({
                            "account_id": k,
                            "active": active.as_deref() == Some(k.as_str()),
                        })
                    }
                })
                .collect();
            Ok(json!({ "accounts": accounts, "active_account_id": active }))
        }
        "setServerUrl" => {
            let url = req_str(params, "server_url").or_else(|_| req_str(params, "url"))?;
            let normalized = host.set_server_url(url)?;
            *generation_advanced = true;
            Ok(json!({
                "server_url": normalized,
                "generation": host.current_generation(),
            }))
        }
        "getServerUrl" => Ok(json!({
            "server_url": host.current_server_url(),
            "default_server_url": DEFAULT_SERVER_URL,
        })),
        "checkpointWal" => {
            let (busy, log, checkpointed) =
                host.with_fenced_session_read(&client, || host.checkpoint_all_wals())?;
            Ok(json!({
                "checkpointed": true,
                "busy": busy,
                "log_frames": log,
                "checkpointed_frames": checkpointed,
            }))
        }
        _ => Err(HostError::unknown_command(&format!("session.{action}"))),
    }
}

pub fn handle_generation(
    host: &FabushiAppHost,
    _req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    match action {
        "current" => Ok(json!({ "generation": host.current_generation() })),
        "bump" => {
            let next = host.bump_generation();
            Ok(json!({ "generation": next }))
        }
        "set" => {
            let val = params
                .get("generation")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| HostError::invalid_arguments("Missing u64 'generation'"))?;
            let applied = host.set_generation(val)?;
            Ok(json!({ "generation": applied }))
        }
        "validate" => {
            let expected = params
                .get("generation")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| HostError::invalid_arguments("Missing u64 'generation'"))?;
            let current = host.current_generation();
            Ok(json!({
                "valid": expected >= current,
                "requested": expected,
                "current": current,
            }))
        }
        _ => Err(HostError::unknown_command(&format!("generation.{action}"))),
    }
}

pub fn handle_settings(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
    generation_advanced: &mut bool,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "privacyGet" => {
            let res = host.run_cancellable(req_id, async move {
                client.get_my_settings().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "privacySave" => {
            let req: UserPrivacySettingsDto = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .save_my_settings(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "localGet" => {
            let key = req_str(params, "key")?;
            let value = host.host_db.kv_get("settings", key)?;
            let parsed = value.map(|raw| {
                serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| Value::String(raw))
            });
            Ok(json!({ "key": key, "value": parsed }))
        }
        "localSet" => {
            let key = req_str(params, "key")?;
            let val = params
                .get("value")
                .ok_or_else(|| HostError::invalid_arguments("Missing 'value'"))?;
            let encoded = serde_json::to_string(val)
                .map_err(|e| HostError::invalid_json(e.to_string()))?;
            host.with_fenced_session_read(&client, || host.host_db.kv_set("settings", key, &encoded))?;
            Ok(json!({ "key": key, "saved": true }))
        }
        "localAll" => {
            let map = host.host_db.kv_list("settings")?;
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let parsed = serde_json::from_str::<Value>(&v).unwrap_or(Value::String(v));
                out.insert(k, parsed);
            }
            Ok(Value::Object(out))
        }
        "serverUrlGet" => Ok(json!({
            "server_url": host.current_server_url(),
            "default_server_url": DEFAULT_SERVER_URL,
        })),
        "serverUrlSet" => {
            let url = req_str(params, "server_url").or_else(|_| req_str(params, "url"))?;
            let normalized = host.set_server_url(url)?;
            *generation_advanced = true;
            Ok(json!({
                "server_url": normalized,
                "generation": host.current_generation(),
            }))
        }
        "storageMetrics" => {
            let metrics = compute_storage_metrics(&host.storage_dir);
            to_val(metrics)
        }
        "storagePurge" => {
            let (freed_bytes, removed_files) = host.with_fenced_session_read(&client, || {
                purge_media_cas_dir(&host.storage_dir)
            })?;
            let metrics = compute_storage_metrics(&host.storage_dir);
            Ok(json!({
                "freed_bytes": freed_bytes,
                "removed_files": removed_files,
                "metrics": metrics,
            }))
        }
        "passcodeSet" => {
            let passcode = req_raw_str(params, "passcode")?;
            if passcode.len() < 4 {
                return Err(HostError::invalid_arguments(
                    "Passcode must be at least 4 characters",
                ));
            }
            let mut salt = [0u8; 16];
            getrandom::getrandom(&mut salt)
                .map_err(|e| HostError::crypto(format!("CSPRNG failed: {e}")))?;
            let (m_cost, t_cost, p_cost) = if host.test_mode {
                (1_024u32, 1u32, 1u32)
            } else {
                (65_536u32, 3u32, 1u32)
            };
            let argon_params = Argon2Params::new(m_cost, t_cost, p_cost, Some(32))
                .map_err(|e| HostError::crypto(format!("Invalid Argon2id params: {e}")))?;
            let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);
            let mut digest = Zeroizing::new([0u8; 32]);
            argon2
                .hash_password_into(passcode.as_bytes(), &salt, &mut *digest)
                .map_err(|e| HostError::crypto(format!("Argon2id passcode hashing failed: {e}")))?;
            let salt_b64 = BASE64_STANDARD.encode(salt);
            let hash_hex = hex::encode(&*digest);
            let encoded_hash = format!("argon2id$m={m_cost},t={t_cost},p={p_cost}${salt_b64}${hash_hex}");
            host.with_fenced_session_read(&client, || {
                host.host_db.set_meta("passcode_salt", &salt_b64)?;
                host.host_db.set_meta("passcode_hash", &encoded_hash)
            })?;
            Ok(json!({ "configured": true, "algorithm": "argon2id" }))
        }
        "passcodeVerify" => {
            let passcode = req_raw_str(params, "passcode")?;
            let hash_opt = host.host_db.get_meta("passcode_hash")?;
            match hash_opt {
                Some(stored) => {
                    let parts: Vec<&str> = stored.split('$').collect();
                    if parts.len() != 4 || parts[0] != "argon2id" {
                        return Err(HostError::crypto(
                            "Unsupported or corrupt passcode hash format",
                        ));
                    }
                    let mut m_cost: Option<u32> = None;
                    let mut t_cost: Option<u32> = None;
                    let mut p_cost: Option<u32> = None;
                    for kv in parts[1].split(',') {
                        if let Some((k, v)) = kv.split_once('=') {
                            match k {
                                "m" => m_cost = v.parse().ok(),
                                "t" => t_cost = v.parse().ok(),
                                "p" => p_cost = v.parse().ok(),
                                _ => {}
                            }
                        }
                    }
                    let (Some(m), Some(t), Some(p)) = (m_cost, t_cost, p_cost) else {
                        return Err(HostError::crypto("Corrupt Argon2id passcode parameters"));
                    };
                    validate_argon2_ios_ceiling(m, t, p)?;
                    let salt = BASE64_STANDARD
                        .decode(parts[2].trim())
                        .map_err(|e| HostError::crypto(format!("Corrupt passcode salt: {e}")))?;
                    let expected = hex::decode(parts[3].trim())
                        .map_err(|e| HostError::crypto(format!("Corrupt passcode digest: {e}")))?;
                    if expected.len() != 32 {
                        return Err(HostError::crypto("Corrupt passcode digest length"));
                    }
                    let argon_params = Argon2Params::new(m, t, p, Some(32))
                        .map_err(|e| HostError::crypto(format!("Invalid Argon2id params: {e}")))?;
                    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);
                    let mut actual = Zeroizing::new([0u8; 32]);
                    argon2
                        .hash_password_into(passcode.as_bytes(), &salt, &mut *actual)
                        .map_err(|e| HostError::crypto(format!("Argon2id verify failed: {e}")))?;
                    let valid = bool::from(actual.as_slice().ct_eq(expected.as_slice()));
                    Ok(json!({ "valid": valid, "configured": true }))
                }
                None => Ok(json!({ "valid": false, "configured": false })),
            }
        }
        "passcodeStatus" => {
            let configured = host.with_fenced_session_read(&client, || {
                Ok(host
                    .host_db
                    .get_meta("passcode_hash")?
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false))
            })?;
            Ok(json!({ "configured": configured }))
        }
        "passcodeClear" => {
            host.with_fenced_session_read(&client, || {
                host.host_db.delete_meta("passcode_salt")?;
                host.host_db.delete_meta("passcode_hash")
            })?;
            Ok(json!({ "cleared": true }))
        }
        "updateCheck" => Ok(json!({
            "current_version": env!("CARGO_PKG_VERSION"),
            "distribution": "app_store_testflight",
            "self_update_supported": false,
            "note": "iOS updates are delivered strictly through App Store / TestFlight"
        })),
        "checkpointWal" => {
            let (busy, log, checkpointed) =
                host.with_fenced_session_read(&client, || host.checkpoint_all_wals())?;
            Ok(json!({
                "checkpointed": true,
                "busy": busy,
                "log_frames": log,
                "checkpointed_frames": checkpointed,
            }))
        }
        _ => Err(HostError::unknown_command(&format!("settings.{action}"))),
    }
}

pub fn handle_proxy(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "list" => {
            let map = host.host_db.kv_list("proxies")?;
            let active_id = host.host_db.get_meta("active_proxy_id")?;
            let mut items = Vec::new();
            for (id, raw) in map {
                let mut val = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({}));
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("id".to_string(), json!(id));
                    obj.insert(
                        "active".to_string(),
                        json!(active_id.as_deref() == Some(id.as_str())),
                    );
                }
                items.push(val);
            }
            Ok(json!({ "proxies": items, "active_proxy_id": active_id }))
        }
        "upsert" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("proxy_{}", uuid::Uuid::new_v4().simple()));
            let cfg: ClientProxyConfig = if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
                ClientProxyConfig::from_url(url)
                    .map_err(|e| HostError::invalid_arguments(format!("Invalid proxy URL: {e}")))?
            } else {
                parse_req(params)?
            };
            cfg.validate()
                .map_err(|e| HostError::invalid_arguments(format!("Invalid proxy config: {e}")))?;
            let encoded = serde_json::to_string(&cfg)
                .map_err(|e| HostError::internal(e.to_string()))?;
            let activate = params
                .get("activate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            host.with_fenced_session_read(&client, || {
                host.host_db.kv_set("proxies", &id, &encoded)?;
                if activate {
                    client.apply_proxy(Some(cfg.clone()))?;
                    host.host_db.set_meta("active_proxy_id", &id)?;
                }
                Ok(())
            })?;
            Ok(json!({ "id": id, "proxy": cfg }))
        }
        "delete" => {
            let id = req_str(params, "id")?;
            let deleted = host.with_fenced_session_read(&client, || {
                let removed = host.host_db.kv_delete("proxies", id)?;
                if host.host_db.get_meta("active_proxy_id")?.as_deref() == Some(id) {
                    host.host_db.delete_meta("active_proxy_id")?;
                    client.apply_proxy(None)?;
                }
                Ok(removed)
            })?;
            Ok(json!({ "deleted": deleted, "id": id }))
        }
        "setActive" => {
            let id_opt = params
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match id_opt {
                None => {
                    host.with_fenced_session_read(&client, || {
                        client.apply_proxy(None)?;
                        host.host_db.delete_meta("active_proxy_id")
                    })?;
                    Ok(json!({ "active_proxy_id": Value::Null }))
                }
                Some(id) => {
                    let cfg = host.with_fenced_session_read(&client, || {
                        let raw = host
                            .host_db
                            .kv_get("proxies", id)?
                            .ok_or_else(|| HostError::not_found(format!("Proxy '{id}' not found")))?;
                        let parsed: ClientProxyConfig = serde_json::from_str(&raw)
                            .map_err(|e| HostError::storage(format!("Corrupt proxy record: {e}")))?;
                        parsed
                            .validate()
                            .map_err(|e| HostError::invalid_arguments(format!("Invalid proxy config: {e}")))?;
                        client.apply_proxy(Some(parsed.clone()))?;
                        host.host_db.set_meta("active_proxy_id", id)?;
                        Ok(parsed)
                    })?;
                    Ok(json!({ "active_proxy_id": id, "proxy": cfg }))
                }
            }
        }
        "getActive" => {
            let active_id = host.host_db.get_meta("active_proxy_id")?;
            let active_cfg = if let Some(ref id) = active_id {
                host.host_db
                    .kv_get("proxies", id)?
                    .and_then(|raw| serde_json::from_str::<ClientProxyConfig>(&raw).ok())
            } else {
                None
            };
            Ok(json!({ "active_proxy_id": active_id, "proxy": active_cfg }))
        }
        "parseUrl" => {
            let url = req_str(params, "url")?;
            let cfg = ClientProxyConfig::from_url(url)
                .map_err(|e| HostError::invalid_arguments(format!("Invalid proxy URL: {e}")))?;
            to_val(cfg)
        }
        "probeHealth" => {
            let cfg: ClientProxyConfig = if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
                ClientProxyConfig::from_url(url)
                    .map_err(|e| HostError::invalid_arguments(format!("Invalid proxy URL: {e}")))?
            } else {
                parse_req(params)?
            };
            let latency_ms = host.run_cancellable(req_id, async move {
                client
                    .http_client
                    .probe_proxy(&cfg)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "reachable": true, "latency_ms": latency_ms }))
        }
        _ => Err(HostError::unknown_command(&format!("proxy.{action}"))),
    }
}

pub fn handle_push(
    host: &FabushiAppHost,
    _req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    match action {
        "registerToken" => {
            let token = req_str(params, "token")?;
            let environment = params
                .get("environment")
                .and_then(|v| v.as_str())
                .unwrap_or("production");
            if environment != "sandbox" && environment != "production" {
                return Err(HostError::invalid_arguments(
                    "environment must be 'sandbox' or 'production'",
                ));
            }
            let now_ms = Utc::now().timestamp_millis();
            let client = host.client();
            let payload = json!({
                "token": token,
                "environment": environment,
                "updated_at_ms": now_ms,
            })
            .to_string();
            host.with_fenced_session_read(&client, || {
                host.host_db.kv_set("push", "apns_registration", &payload)
            })?;
            Ok(json!({
                "persisted": true,
                "token_length": token.len(),
                "environment": environment,
                "updated_at_ms": now_ms,
                "server_apns_dispatch": "upstream_stub_blocked",
                "blocker_id": "BLK-01"
            }))
        }
        "unregisterToken" => {
            let client = host.client();
            let removed = host.with_fenced_session_read(&client, || {
                host.host_db.kv_delete("push", "apns_registration")
            })?;
            Ok(json!({
                "unregistered": removed,
                "server_apns_dispatch": "upstream_stub_blocked",
                "blocker_id": "BLK-01"
            }))
        }
        "status" => {
            let reg = host
                .host_db
                .kv_get("push", "apns_registration")?
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
            Ok(json!({
                "registered": reg.is_some(),
                "registration": reg,
                "server_apns_dispatch": "upstream_stub_blocked",
                "blocker_id": "BLK-01"
            }))
        }
        _ => Err(HostError::unknown_command(&format!("push.{action}"))),
    }
}

fn parse_action_category(raw: &str) -> Option<ActionCategory> {
    let normalized = raw.trim().to_ascii_lowercase().replace(['-', '_'], "");
    match normalized.as_str() {
        "messaging" => Some(ActionCategory::Messaging),
        "dialogs" => Some(ActionCategory::Dialogs),
        "chat" => Some(ActionCategory::Chat),
        "media" => Some(ActionCategory::Media),
        "search" => Some(ActionCategory::Search),
        "user" => Some(ActionCategory::User),
        "call" => Some(ActionCategory::Call),
        "miniapp" => Some(ActionCategory::MiniApp),
        "admin" => Some(ActionCategory::Admin),
        "system" => Some(ActionCategory::System),
        "device" => Some(ActionCategory::Device),
        "remote" => Some(ActionCategory::Remote),
        "terminal" => Some(ActionCategory::Terminal),
        "assist" => Some(ActionCategory::Assist),
        "cloudpc" => Some(ActionCategory::CloudPc),
        "mcp" => Some(ActionCategory::Mcp),
        "bot" => Some(ActionCategory::Bot),
        "task" => Some(ActionCategory::Task),
        "vault" => Some(ActionCategory::Vault),
        _ => None,
    }
}

pub fn handle_actions(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    match action {
        "list" => {
            let items: Vec<Value> = host
                .action_registry
                .list()
                .into_iter()
                .map(|a| {
                    json!({
                        "name": a.name(),
                        "description": a.description(),
                        "category": format!("{:?}", a.category()),
                        "risk_level": format!("{:?}", a.risk_level()),
                    })
                })
                .collect();
            Ok(json!({ "count": items.len(), "actions": items }))
        }
        "listByCategory" => {
            let cat_str = req_str(params, "category")?;
            let cat = parse_action_category(cat_str).ok_or_else(|| {
                HostError::invalid_arguments(format!("Unknown ActionCategory '{cat_str}'"))
            })?;
            let items: Vec<Value> = host
                .action_registry
                .list_by_category(cat)
                .into_iter()
                .map(|a| {
                    json!({
                        "name": a.name(),
                        "description": a.description(),
                        "category": format!("{:?}", a.category()),
                    })
                })
                .collect();
            Ok(json!({ "category": cat_str, "count": items.len(), "actions": items }))
        }
        "get" => {
            let name = req_str(params, "name")?;
            let found = host
                .action_registry
                .get(name)
                .ok_or_else(|| HostError::not_found(format!("Action '{name}' not found")))?;
            Ok(json!({
                "name": found.name(),
                "description": found.description(),
                "category": format!("{:?}", found.category()),
                "risk_level": format!("{:?}", found.risk_level()),
                "input_schema": found.input_schema(),
            }))
        }
        "execute" => {
            let name = req_str(params, "name")?.to_string();
            if host.is_ios_sandbox() {
                let norm = name.trim().to_ascii_lowercase();
                if matches!(
                    norm.as_str(),
                    "device_power_action"
                        | "device_connect"
                        | "remote_capture_screen"
                        | "remote_mouse"
                        | "remote_mouse_action"
                        | "remote_keyboard"
                        | "remote_keyboard_action"
                        | "remote_disconnect_all"
                        | "terminal_exec"
                        | "terminal_list"
                ) {
                    return Err(HostError::capability_unavailable(format!(
                        "Action '{name}' requires desktop host-control capabilities unavailable in the iOS sandbox"
                    )));
                }
            }
            let input = params.get("input").cloned().unwrap_or_else(|| json!({}));
            let approved = params
                .get("approved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut ctx = ActionContext::new();
            ctx.user_id = host.client().current_user_id();
            ctx.approved = approved;
            let reg = std::sync::Arc::clone(&host.action_registry);
            host.run_cancellable(req_id, async move {
                reg.execute(&name, ctx, input)
                    .await
                    .map_err(|e| HostError::new("action_execution_error", e.to_string(), false))
            })
        }
        "exportMcp" => {
            let list = McpExporter::export_tools_list(&host.action_registry);
            to_val(list)
        }
        "exportOpenApi" => {
            let title = params
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Fabushi iOS Action Bus");
            let version = params
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or(env!("CARGO_PKG_VERSION"));
            let base_url = params
                .get("base_url")
                .and_then(|v| v.as_str())
                .map(ClientConfig::normalize_url)
                .unwrap_or_else(|| host.current_server_url());
            Ok(OpenApiExporter::export_spec(
                &host.action_registry,
                title,
                version,
                &base_url,
            ))
        }
        "exportHermes" => {
            let tools = HermesToolExporter::export_tools(&host.action_registry);
            let openai = HermesToolExporter::export_openai_tools(&host.action_registry);
            let anthropic = HermesToolExporter::export_anthropic_tools(&host.action_registry);
            Ok(json!({
                "count": tools.len(),
                "openai_tools": openai,
                "anthropic_tools": anthropic,
            }))
        }
        "exportCli" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("fabushi");
            let version = params
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or(env!("CARGO_PKG_VERSION"));
            let schema = CliExporter::export_cli_schema(&host.action_registry, name, version);
            to_val(schema)
        }
        _ => Err(HostError::unknown_command(&format!("actions.{action}"))),
    }
}

pub fn handle_wallet(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "balance" => {
            let res = host.run_cancellable(req_id, async move {
                client.wallet_balance().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "transactions" => {
            let res = host.run_cancellable(req_id, async move {
                client.wallet_transactions().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "topup" => {
            let amount_stars = params
                .get("amount_stars")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| HostError::invalid_arguments("Missing i64 'amount_stars'"))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .wallet_topup(amount_stars)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "giftStars" => {
            let req: StarGiftRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .wallet_gift_stars(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "createAlipayOrder" => {
            let plan = req_str(params, "plan")?.to_string();
            let platform = params
                .get("platform")
                .and_then(|v| v.as_str())
                .unwrap_or("ios")
                .to_string();
            let token = host
                .fenced_client_token(&client)?
                .ok_or_else(|| HostError::unauthorized("No active session token"))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .http_client
                    .create_alipay_order(&token, &plan, &platform)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "queryAlipayOrder" => {
            let order_id = req_str(params, "order_id")?.to_string();
            let token = host
                .fenced_client_token(&client)?
                .ok_or_else(|| HostError::unauthorized("No active session token"))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .http_client
                    .query_alipay_order(&token, &order_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "redeemCode" => {
            let code = req_str(params, "code")?.to_string();
            let token = host
                .fenced_client_token(&client)?
                .ok_or_else(|| HostError::unauthorized("No active session token"))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .http_client
                    .use_redeem_code(&token, &code)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "checkMembership" => {
            let token = host
                .fenced_client_token(&client)?
                .ok_or_else(|| HostError::unauthorized("No active session token"))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .http_client
                    .check_membership(&token)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "tonStatus" => Ok(json!({
            "on_chain_signing": "upstream_not_implemented",
            "blocker_id": "BLK-03",
            "storekit2_runner": "native_ios_swift"
        })),
        "tonSignBroadcast" => Err(HostError::upstream_not_implemented(
            "TON on-chain signing & broadcast is not implemented upstream (BLK-03)",
        )),
        _ => Err(HostError::unknown_command(&format!("wallet.{action}"))),
    }
}
