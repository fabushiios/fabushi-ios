use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use chrono::Utc;
use gpui_telegram_client::config::{ClientConfig, DEFAULT_SERVER_URL};
use gpui_telegram_client::net::proxy::ClientProxyConfig;
use gpui_telegram_client::net::{
    AuthResponse, ChangePasswordRequest, EmailRegisterRequest, LoginRequest, PhoneVerifyRequest,
    RegisterRequest, SplitKeyEmailRegisterRequest, SplitKeyLoginRequest, SplitKeyRegisterRequest,
    StarGiftRequest, TwoFactorSetRequest, TwoFactorVerifyRequest, UpgradeSplitKeyRequest,
    UserPrivacySettingsDto,
};
use gpui_telegram_core::action_bus::{
    ActionCategory, ActionContext, CliExporter, HermesToolExporter, McpExporter, OpenApiExporter,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{HostError, HostResult};
use crate::host::FabushiAppHost;
use crate::storage::{compute_storage_metrics, purge_media_cas_dir};

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

fn to_val<T: serde::Serialize>(v: T) -> HostResult<Value> {
    serde_json::to_value(v).map_err(|e| HostError::internal(format!("Serialization error: {e}")))
}

pub fn handle_auth(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
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
                    password: req_str(params, "password")?.to_string(),
                    device_name: params
                        .get("device_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Fabushi iOS")
                        .to_string(),
                }
            };
            let auth = host.run_cancellable(req_id, async move {
                client.login(&req).await.map_err(HostError::from)
            })?;
            let _ = host.host_db.kv_set(
                "accounts",
                &auth.user_id.to_string(),
                &serde_json::to_string(&auth).unwrap_or_default(),
            );
            let _ = host
                .host_db
                .set_meta("active_account_id", &auth.user_id.to_string());
            to_val(auth)
        }
        "register" => {
            let req: RegisterRequest = parse_req(params)?;
            let auth = host.run_cancellable(req_id, async move {
                client.register(&req).await.map_err(HostError::from)
            })?;
            let _ = host.host_db.kv_set(
                "accounts",
                &auth.user_id.to_string(),
                &serde_json::to_string(&auth).unwrap_or_default(),
            );
            to_val(auth)
        }
        "logout" => {
            let res = host.run_cancellable(req_id, async move {
                client.logout().await.map_err(HostError::from)
            });
            host.bump_generation();
            match res {
                Ok(()) => Ok(json!({ "logged_out": true, "generation": host.current_generation() })),
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
            let res = host.run_cancellable(req_id, async move {
                client
                    .register_split_key(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "emailRegisterSplitKey" => {
            let req: SplitKeyEmailRegisterRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .email_register_split_key(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "loginSplitKey" => {
            let req: SplitKeyLoginRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.login_split_key(&req).await.map_err(HostError::from)
            })?;
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
            let res = host.run_cancellable(req_id, async move {
                client.phone_verify(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
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
            let res = host.run_cancellable(req_id, async move {
                client.email_register(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "emailVerify" => {
            let email = req_str(params, "email")?.to_string();
            let code = req_str(params, "code")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .email_verify(&email, &code)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
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
            let res = host.run_cancellable(req_id, async move {
                let poll = client
                    .http_client
                    .alipay_poll(&attempt_id)
                    .await
                    .map_err(HostError::from)?;
                if let Some(ref sess) = poll.session {
                    client.set_auth_session(sess.clone());
                } else if let Some(ref tok) = poll.token {
                    client.set_token(Some(tok.clone()));
                }
                Ok(poll)
            })?;
            to_val(res)
        }
        "qrPairInit" => {
            let res = host.run_cancellable(req_id, async move {
                client.qr_pair_init().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "qrPairPoll" => {
            let pair_token = req_str(params, "pair_token")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .qr_pair_poll(&pair_token)
                    .await
                    .map_err(HostError::from)
            })?;
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
            let res = host.run_cancellable(req_id, async move {
                client
                    .qr_pair_verify(&pair_token, cloud_password.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
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
            let res = host.run_cancellable(req_id, async move {
                client.passkey_verify(&body).await.map_err(HostError::from)
            })?;
            to_val(res)
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
            let res = host.run_cancellable(req_id, async move {
                client.verify_2fa(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
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
    action: &str,
    params: &Value,
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
            client.connect_ws();
            let status = format!("{:?}", client.status());
            host.emit_event(
                "connectionStateChanged",
                json!({ "status": status, "server_url": host.current_server_url() }),
            );
            Ok(json!({
                "connecting": true,
                "status": format!("{:?}", client.status()),
                "server_url": host.current_server_url(),
            }))
        }
        "disconnectWs" => {
            client.disconnect_ws();
            let status = format!("{:?}", client.status());
            host.emit_event(
                "connectionStateChanged",
                json!({ "status": status, "server_url": host.current_server_url() }),
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
            client.set_token(token.clone());
            Ok(json!({ "has_token": token.is_some() }))
        }
        "setAuthSession" => {
            let auth: AuthResponse = parse_req(params)?;
            client.set_auth_session(auth.clone());
            let _ = host.host_db.kv_set(
                "accounts",
                &auth.user_id.to_string(),
                &serde_json::to_string(&auth).unwrap_or_default(),
            );
            let _ = host
                .host_db
                .set_meta("active_account_id", &auth.user_id.to_string());
            Ok(json!({ "user_id": auth.user_id, "generation": host.current_generation() }))
        }
        "clearSession" => {
            client.clear_session();
            let generation = host.bump_generation();
            Ok(json!({ "cleared": true, "generation": generation }))
        }
        "switchAccount" => {
            let account_id = req_str(params, "account_id")?;
            let saved = host.host_db.kv_get("accounts", account_id)?;
            client.clear_session();
            let new_gen = host.bump_generation();
            if let Some(raw) = saved {
                if let Ok(auth) = serde_json::from_str::<AuthResponse>(&raw) {
                    client.set_auth_session(auth.clone());
                    host.host_db.set_meta("active_account_id", account_id)?;
                    return Ok(json!({
                        "switched": true,
                        "account_id": account_id,
                        "user_id": auth.user_id,
                        "generation": new_gen,
                    }));
                }
            }
            host.host_db.set_meta("active_account_id", account_id)?;
            Ok(json!({
                "switched": true,
                "account_id": account_id,
                "generation": new_gen,
            }))
        }
        "listAccounts" => {
            let map = host.host_db.kv_list("accounts")?;
            let active = host.host_db.get_meta("active_account_id")?;
            let accounts: Vec<Value> = map
                .into_iter()
                .map(|(k, v)| {
                    if let Ok(auth) = serde_json::from_str::<AuthResponse>(&v) {
                        json!({
                            "account_id": k,
                            "user_id": auth.user_id,
                            "username": auth.username,
                            "first_name": auth.first_name,
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
            let url = req_str(params, "server_url")
                .or_else(|_| req_str(params, "url"))?;
            let normalized = host.set_server_url(url)?;
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
            let (busy, log, checkpointed) = host.host_db.wal_checkpoint_truncate()?;
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
            let applied = host.set_generation(val);
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
            host.host_db.kv_set("settings", key, &encoded)?;
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
            let (freed_bytes, removed_files) = purge_media_cas_dir(&host.storage_dir)?;
            let metrics = compute_storage_metrics(&host.storage_dir);
            Ok(json!({
                "freed_bytes": freed_bytes,
                "removed_files": removed_files,
                "metrics": metrics,
            }))
        }
        "passcodeSet" => {
            let passcode = req_str(params, "passcode")?;
            if passcode.len() < 4 {
                return Err(HostError::invalid_arguments(
                    "Passcode must be at least 4 characters",
                ));
            }
            let mut salt = [0u8; 16];
            getrandom::getrandom(&mut salt)
                .map_err(|e| HostError::crypto(format!("CSPRNG failed: {e}")))?;
            let salt_b64 = BASE64_STANDARD.encode(salt);
            let mut hasher = Sha256::new();
            hasher.update(salt);
            hasher.update(passcode.as_bytes());
            let hash_hex = hex::encode(hasher.finalize());
            host.host_db.set_meta("passcode_salt", &salt_b64)?;
            host.host_db.set_meta("passcode_hash", &hash_hex)?;
            Ok(json!({ "configured": true }))
        }
        "passcodeVerify" => {
            let passcode = req_str(params, "passcode")?;
            let salt_opt = host.host_db.get_meta("passcode_salt")?;
            let hash_opt = host.host_db.get_meta("passcode_hash")?;
            match (salt_opt, hash_opt) {
                (Some(salt_b64), Some(expected_hash)) => {
                    let salt = BASE64_STANDARD
                        .decode(salt_b64.trim())
                        .map_err(|e| HostError::crypto(format!("Corrupt passcode salt: {e}")))?;
                    let mut hasher = Sha256::new();
                    hasher.update(salt);
                    hasher.update(passcode.as_bytes());
                    let actual_hash = hex::encode(hasher.finalize());
                    Ok(json!({ "valid": actual_hash == expected_hash, "configured": true }))
                }
                _ => Ok(json!({ "valid": false, "configured": false })),
            }
        }
        "passcodeClear" => {
            host.host_db.delete_meta("passcode_salt")?;
            host.host_db.delete_meta("passcode_hash")?;
            Ok(json!({ "cleared": true }))
        }
        "updateCheck" => Ok(json!({
            "current_version": env!("CARGO_PKG_VERSION"),
            "distribution": "app_store_testflight",
            "self_update_supported": false,
            "note": "iOS updates are delivered strictly through App Store / TestFlight"
        })),
        "checkpointWal" => {
            let (busy, log, checkpointed) = host.host_db.wal_checkpoint_truncate()?;
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
            host.host_db.kv_set("proxies", &id, &encoded)?;
            if params
                .get("activate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                client.apply_proxy(Some(cfg.clone()))?;
                host.host_db.set_meta("active_proxy_id", &id)?;
            }
            Ok(json!({ "id": id, "proxy": cfg }))
        }
        "delete" => {
            let id = req_str(params, "id")?;
            let deleted = host.host_db.kv_delete("proxies", id)?;
            if host.host_db.get_meta("active_proxy_id")?.as_deref() == Some(id) {
                host.host_db.delete_meta("active_proxy_id")?;
                client.apply_proxy(None)?;
            }
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
                    client.apply_proxy(None)?;
                    host.host_db.delete_meta("active_proxy_id")?;
                    Ok(json!({ "active_proxy_id": Value::Null }))
                }
                Some(id) => {
                    let raw = host
                        .host_db
                        .kv_get("proxies", id)?
                        .ok_or_else(|| HostError::not_found(format!("Proxy '{id}' not found")))?;
                    let cfg: ClientProxyConfig = serde_json::from_str(&raw)
                        .map_err(|e| HostError::storage(format!("Corrupt proxy record: {e}")))?;
                    client.apply_proxy(Some(cfg.clone()))?;
                    host.host_db.set_meta("active_proxy_id", id)?;
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
            host.host_db.kv_set(
                "push",
                "apns_registration",
                &json!({
                    "token": token,
                    "environment": environment,
                    "updated_at_ms": now_ms,
                })
                .to_string(),
            )?;
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
            let removed = host.host_db.kv_delete("push", "apns_registration")?;
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
            let token = client
                .token()
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
            let token = client
                .token()
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
            let token = client
                .token()
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
            let token = client
                .token()
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
