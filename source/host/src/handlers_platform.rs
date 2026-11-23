use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use bot_browser::e2ee_sync::{
    derive_split_key, generate_split_key_salt_b64, Argon2KdfParams, DeviceKeyTransfer,
    DeviceSyncKeypair, EncryptedVaultBlob, SplitKeyVaultKek, UnlockedUserVaultKey,
    UserVaultCategory, UserVaultCrypto, UserVaultKeyEnvelope,
};
use bot_browser::totp::{generate_totp_at, generate_totp_now};
use bot_browser::types::ReusePolicy;
use bot_browser::vault::CredentialVault;
use fabushi_miniapp::state::UiCommand;
use fabushi_remote_engine::input::NativeInputEngine;
use fabushi_remote_engine::power::PowerManager;
use gpui_telegram_client::net::api_miniapps::verify_telegram_init_data_hmac;
use gpui_telegram_client::net::{
    ApproveDeviceSyncRequest, AssistStartRequest, CallInviteRequest, CallSignalRequest,
    DeviceSyncRequestBody, HermesCallInviteRequest, HermesTalkTurnRequest,
    RemoteComputerCommandRequest, RemoteComputerCommandResult, UpdateBotComputerRequest,
    UpdateComputerPolicyRequest, UpsertCloudComputerRequest, UpsertUserVaultItemRequest,
    UserVaultKeyEnvelopeDto,
};
use gpui_telegram_core::types::{ChatId, UserId};
use gpui_telegram_proto::miniapp_contracts::{
    parse_miniapp_uri, sanitize_webapp_custom_url, AcceptMiniAppTosRequest, AgentManifest,
    AllowSendMessageRequest, MiniAppActionRequest, MiniAppControlMode, MiniAppLaunchMode,
    MiniAppTakeoverEvent, MiniAppWebMcpResponse,
};
use gpui_telegram_proto::remote_control_contracts::{
    PowerActionKind, RemoteSessionDto,
};
use gpui_telegram_webrtc::calls::{CallsEmojiFingerprint, SignalingFrame};
use gpui_telegram_webrtc::media::AiMediaTap;
use gpui_telegram_agent::talk::TalkSessionState;
use serde_json::{json, Value};

use crate::error::{HostError, HostResult};
use crate::host::FabushiAppHost;

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

fn req_i64(params: &Value, key: &str) -> HostResult<i64> {
    params
        .get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required i64 '{key}'")))
}

fn req_u64(params: &Value, key: &str) -> HostResult<u64> {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required u64 '{key}'")))
}

fn to_val<T: serde::Serialize>(v: T) -> HostResult<Value> {
    serde_json::to_value(v).map_err(|e| HostError::internal(format!("Serialization error: {e}")))
}

fn parse_kdf_params(params: &Value) -> Argon2KdfParams {
    if let Some(p) = params.get("kdf_params") {
        if let Ok(parsed) = serde_json::from_value::<Argon2KdfParams>(p.clone()) {
            return parsed;
        }
    }
    if params
        .get("fast_kdf")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        Argon2KdfParams::fast_for_tests()
    } else {
        Argon2KdfParams::default()
    }
}

fn open_unlocked_vault(host: &FabushiAppHost) -> HostResult<CredentialVault> {
    let key = host
        .vault_unlocked_key
        .lock()
        .clone()
        .ok_or_else(|| HostError::unauthorized("Vault is locked; unlock before accessing credentials"))?;
    let vault_path = host.storage_dir.join("vault.db");
    CredentialVault::open(&vault_path, &key).map_err(|e| HostError::crypto(e.to_string()))
}

pub fn handle_vault(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "deriveSplitKey" => {
            let password = req_str(params, "password")?;
            let salt_b64 = params
                .get("salt_b64")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(generate_split_key_salt_b64);
            let kdf = parse_kdf_params(params);
            let bundle = derive_split_key(password, &salt_b64, &kdf)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            Ok(json!({
                "auth_salt_b64": bundle.auth_salt_b64,
                "kdf_params": bundle.kdf_params,
                "auth_verifier_b64": &*bundle.auth_verifier_b64,
                "vault_kek_b64": BASE64_STANDARD.encode(bundle.vault_kek.as_bytes()),
            }))
        }
        "initNew" => {
            let user_id = req_u64(params, "user_id")?;
            let passphrase = req_str(params, "passphrase")?;
            let kdf = parse_kdf_params(params);
            let (unlocked, envelope, recovery_key) =
                UserVaultCrypto::initialize_new(user_id, passphrase, kdf)
                    .map_err(|e| HostError::crypto(e.to_string()))?;
            let root_key_b64 = BASE64_STANDARD.encode(unlocked.as_bytes());
            *host.vault_unlocked_key.lock() = Some(unlocked);
            *host.vault_envelope.lock() = Some(envelope.clone());
            Ok(json!({
                "unlocked": true,
                "envelope": envelope,
                "recovery_key": &*recovery_key,
                "root_key_b64": root_key_b64,
            }))
        }
        "initSplitKey" => {
            let user_id = req_u64(params, "user_id")?;
            let password = req_str(params, "password")?;
            let salt_b64 = params
                .get("salt_b64")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(generate_split_key_salt_b64);
            let kdf = parse_kdf_params(params);
            let bundle = derive_split_key(password, &salt_b64, &kdf)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            let (envelope, unlocked, emergency_kit) = UserVaultCrypto::initialize_with_split_key(
                user_id,
                &bundle.vault_kek,
                &bundle.auth_salt_b64,
                bundle.kdf_params,
            )
            .map_err(|e| HostError::crypto(e.to_string()))?;
            let root_key_b64 = BASE64_STANDARD.encode(unlocked.as_bytes());
            *host.vault_unlocked_key.lock() = Some(unlocked);
            *host.vault_envelope.lock() = Some(envelope.clone());
            Ok(json!({
                "unlocked": true,
                "auth_salt_b64": bundle.auth_salt_b64,
                "auth_verifier_b64": &*bundle.auth_verifier_b64,
                "envelope": envelope,
                "emergency_kit": &*emergency_kit,
                "root_key_b64": root_key_b64,
            }))
        }
        "unlockWithPassphrase" => {
            let user_id = req_u64(params, "user_id")?;
            let passphrase = req_str(params, "passphrase")?;
            let envelope: UserVaultKeyEnvelope = if let Some(env_val) = params.get("envelope") {
                parse_req(env_val)?
            } else {
                host.vault_envelope
                    .lock()
                    .clone()
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'envelope'"))?
            };
            let unlocked = UserVaultCrypto::unlock_with_passphrase(user_id, passphrase, &envelope)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            *host.vault_unlocked_key.lock() = Some(unlocked);
            *host.vault_envelope.lock() = Some(envelope);
            Ok(json!({ "unlocked": true }))
        }
        "unlockWithSplitKey" => {
            let user_id = req_u64(params, "user_id")?;
            let envelope: UserVaultKeyEnvelope = if let Some(env_val) = params.get("envelope") {
                parse_req(env_val)?
            } else {
                host.vault_envelope
                    .lock()
                    .clone()
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'envelope'"))?
            };
            let kek = if let Some(kek_b64) = params.get("vault_kek_b64").and_then(|v| v.as_str()) {
                let raw = BASE64_STANDARD
                    .decode(kek_b64.trim())
                    .map_err(|e| HostError::invalid_arguments(format!("Invalid vault_kek_b64: {e}")))?;
                let arr: [u8; 32] = raw
                    .try_into()
                    .map_err(|_| HostError::invalid_arguments("vault_kek_b64 must decode to 32 bytes"))?;
                SplitKeyVaultKek::from_bytes(arr)
            } else {
                let password = req_str(params, "password")?;
                let bundle = derive_split_key(password, &envelope.kdf_salt_b64, &envelope.kdf_params)
                    .map_err(|e| HostError::crypto(e.to_string()))?;
                bundle.vault_kek
            };
            let unlocked = UserVaultCrypto::unlock_with_split_key(user_id, &envelope, &kek)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            let root_key_b64 = BASE64_STANDARD.encode(unlocked.as_bytes());
            *host.vault_unlocked_key.lock() = Some(unlocked);
            *host.vault_envelope.lock() = Some(envelope);
            Ok(json!({ "unlocked": true, "root_key_b64": root_key_b64 }))
        }
        "unlockWithRecoveryKey" => {
            let user_id = req_u64(params, "user_id")?;
            let recovery_key = req_str(params, "recovery_key")?;
            let envelope: UserVaultKeyEnvelope = if let Some(env_val) = params.get("envelope") {
                parse_req(env_val)?
            } else {
                host.vault_envelope
                    .lock()
                    .clone()
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'envelope'"))?
            };
            let unlocked =
                UserVaultCrypto::unlock_with_recovery_key(user_id, recovery_key, &envelope)
                    .map_err(|e| HostError::crypto(e.to_string()))?;
            *host.vault_unlocked_key.lock() = Some(unlocked);
            *host.vault_envelope.lock() = Some(envelope);
            Ok(json!({ "unlocked": true }))
        }
        "unlockWithKeyBytes" => {
            let raw = BASE64_STANDARD
                .decode(req_str(params, "root_key_b64")?)
                .map_err(|e| HostError::invalid_arguments(format!("Invalid root_key_b64: {e}")))?;
            let arr: [u8; 32] = raw
                .try_into()
                .map_err(|_| HostError::invalid_arguments("root_key_b64 must be exactly 32 bytes"))?;
            *host.vault_unlocked_key.lock() = Some(UnlockedUserVaultKey::from_bytes(arr));
            Ok(json!({ "unlocked": true }))
        }
        "lock" => {
            *host.vault_unlocked_key.lock() = None;
            Ok(json!({ "unlocked": false }))
        }
        "status" => Ok(json!({
            "unlocked": host.vault_unlocked_key.lock().is_some(),
            "has_envelope": host.vault_envelope.lock().is_some(),
        })),
        "storeCredential" => {
            let vault = open_unlocked_vault(host)?;
            let origin = req_str(params, "origin")?;
            let username = req_str(params, "username")?;
            let password = req_str(params, "password")?;
            let totp_secret = params.get("totp_secret").and_then(|v| v.as_str());
            let account_hint = params.get("account_hint").and_then(|v| v.as_str());
            let reuse_policy: Option<ReusePolicy> = params
                .get("reuse_policy")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok());
            let meta = vault
                .store_credential(
                    origin,
                    username,
                    password,
                    totp_secret,
                    account_hint,
                    reuse_policy,
                )
                .map_err(|e| HostError::crypto(e.to_string()))?;
            to_val(meta)
        }
        "listForOrigin" => {
            let vault = open_unlocked_vault(host)?;
            let origin = req_str(params, "origin")?;
            let list = vault
                .list_for_origin(origin)
                .map_err(|e| HostError::storage(e.to_string()))?;
            to_val(list)
        }
        "getMeta" => {
            let vault = open_unlocked_vault(host)?;
            let cred_id = req_str(params, "credential_id")?;
            let meta = vault
                .get_meta(cred_id)
                .map_err(|e| HostError::not_found(e.to_string()))?;
            to_val(meta)
        }
        "decryptForOrigin" => {
            let vault = open_unlocked_vault(host)?;
            let cred_id = req_str(params, "credential_id")?;
            let origin = req_str(params, "origin")?;
            let frame_origin = params.get("frame_origin").and_then(|v| v.as_str());
            let secret = vault
                .decrypt_for_origin(cred_id, origin, frame_origin)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            let totp_code = secret
                .totp_secret
                .as_deref()
                .and_then(|s| generate_totp_now(s).ok())
                .map(|c| c.to_string());
            Ok(json!({
                "credential_id": cred_id,
                "origin": origin,
                "username": &*secret.username,
                "password": &*secret.password,
                "has_totp": secret.totp_secret.is_some(),
                "totp_code": totp_code,
            }))
        }
        "deleteCredential" => {
            let vault = open_unlocked_vault(host)?;
            let cred_id = req_str(params, "credential_id")?;
            vault
                .delete_credential(cred_id)
                .map_err(|e| HostError::not_found(e.to_string()))?;
            Ok(json!({ "deleted": true, "credential_id": cred_id }))
        }
        "sealItem" => {
            let key = host
                .vault_unlocked_key
                .lock()
                .clone()
                .ok_or_else(|| HostError::unauthorized("Vault is locked"))?;
            let user_id = req_u64(params, "user_id")?;
            let cat = UserVaultCategory::from_str(req_str(params, "category")?)
                .map_err(|e| HostError::invalid_arguments(e.to_string()))?;
            let item_key = req_str(params, "item_key")?;
            let revision = params.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let plaintext = req_str(params, "plaintext")?.as_bytes();
            let blob = key
                .seal_item(user_id, cat, item_key, revision, plaintext)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            to_val(blob)
        }
        "openItem" => {
            let key = host
                .vault_unlocked_key
                .lock()
                .clone()
                .ok_or_else(|| HostError::unauthorized("Vault is locked"))?;
            let user_id = req_u64(params, "user_id")?;
            let blob: EncryptedVaultBlob = parse_req(
                params
                    .get("blob")
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'blob'"))?,
            )?;
            let plain = key
                .open_item(user_id, &blob)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            Ok(json!({
                "plaintext_utf8": String::from_utf8(plain.to_vec()).ok(),
                "plaintext_b64": BASE64_STANDARD.encode(&*plain),
            }))
        }
        // X25519 Device Trust Ring local crypto
        "deviceSyncKeygen" => {
            let pair_id = params
                .get("pair_id")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("sync_{}", uuid::Uuid::new_v4().simple()));
            let kp = DeviceSyncKeypair::generate();
            let pub_b64 = kp.public_key_b64();
            let code = DeviceSyncKeypair::verification_code(&pub_b64)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            host.vault_device_keypairs.lock().insert(pair_id.clone(), kp);
            Ok(json!({
                "pair_id": pair_id,
                "public_key_b64": pub_b64,
                "verification_code": code,
            }))
        }
        "deviceSyncSeal" => {
            let key = host
                .vault_unlocked_key
                .lock()
                .clone()
                .ok_or_else(|| HostError::unauthorized("Vault is locked"))?;
            let request_id = req_str(params, "request_id")?;
            let user_id = req_u64(params, "user_id")?;
            let target_pub = req_str(params, "target_device_pub_b64")?;
            let code = DeviceSyncKeypair::verification_code(target_pub)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            let transfer = DeviceSyncKeypair::seal_root_key_for_device(
                request_id, user_id, &key, target_pub,
            )
            .map_err(|e| HostError::crypto(e.to_string()))?;
            Ok(json!({ "verification_code": code, "transfer": transfer }))
        }
        "deviceSyncOpen" => {
            let pair_id = req_str(params, "pair_id")?;
            let request_id = req_str(params, "request_id")?;
            let user_id = req_u64(params, "user_id")?;
            let transfer: DeviceKeyTransfer = parse_req(
                params
                    .get("transfer")
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'transfer'"))?,
            )?;
            let kp = host
                .vault_device_keypairs
                .lock()
                .remove(pair_id)
                .ok_or_else(|| HostError::not_found(format!("Sync keypair '{pair_id}' not found")))?;
            let unlocked = kp
                .open_root_key_transfer(request_id, user_id, &transfer)
                .map_err(|e| HostError::crypto(e.to_string()))?;
            let root_key_b64 = BASE64_STANDARD.encode(unlocked.as_bytes());
            *host.vault_unlocked_key.lock() = Some(unlocked);
            Ok(json!({ "unlocked": true, "root_key_b64": root_key_b64 }))
        }
        "totpGenerate" => {
            let secret = req_str(params, "secret")?;
            if let Some(unix_secs) = params.get("unix_secs").and_then(|v| v.as_u64()) {
                let digits = params.get("digits").and_then(|v| v.as_u64()).unwrap_or(6) as u32;
                let period = params.get("period").and_then(|v| v.as_u64()).unwrap_or(30);
                let code = generate_totp_at(secret, unix_secs, digits, period)
                    .map_err(|e| HostError::crypto(e.to_string()))?;
                return Ok(json!({ "code": &*code, "unix_secs": unix_secs, "digits": digits, "period": period }));
            }
            let code = generate_totp_now(secret).map_err(|e| HostError::crypto(e.to_string()))?;
            Ok(json!({ "code": &*code }))
        }
        // Server Vault Sync endpoints
        "envelopeGet" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_user_vault_envelope()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "envelopePut" => {
            let env: UserVaultKeyEnvelopeDto = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .put_user_vault_envelope(&env)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "syncPull" => {
            let since_revision = params.get("since_revision").and_then(|v| v.as_i64());
            let category = params
                .get("category")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .pull_user_vault_items(since_revision, category.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "syncPush" => {
            let category = req_str(params, "category")?.to_string();
            let item_key = req_str(params, "item_key")?.to_string();
            let req: UpsertUserVaultItemRequest =
                parse_req(params.get("item").unwrap_or(params))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .put_user_vault_item(&category, &item_key, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "syncDelete" => {
            let category = req_str(params, "category")?.to_string();
            let item_key = req_str(params, "item_key")?.to_string();
            let expected_revision = params.get("expected_revision").and_then(|v| v.as_i64());
            let res = host.run_cancellable(req_id, async move {
                client
                    .delete_user_vault_item(&category, &item_key, expected_revision)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "deviceSyncRequest" => {
            let req: DeviceSyncRequestBody = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .device_sync_request(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "deviceSyncPending" => {
            let res = host.run_cancellable(req_id, async move {
                client.device_sync_pending().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "deviceSyncApprove" => {
            let request_id = req_str(params, "request_id")?.to_string();
            let req: ApproveDeviceSyncRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .device_sync_approve(&request_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "deviceSyncDeny" => {
            let request_id = req_str(params, "request_id")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .device_sync_deny(&request_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "deviceSyncStatus" => {
            let request_id = req_str(params, "request_id")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .device_sync_status(&request_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        _ => Err(HostError::unknown_command(&format!("vault.{action}"))),
    }
}

pub fn handle_computers(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "list" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_computers_async().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "upsertCloud" => {
            let req: UpsertCloudComputerRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .upsert_cloud_computer_async(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "updatePolicy" => {
            let node_id = req_str(params, "node_id")?.to_string();
            let req: UpdateComputerPolicyRequest =
                parse_req(params.get("policy").unwrap_or(params))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .update_computer_policy_async(&node_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "delete" => {
            let node_id = req_str(params, "node_id")?.to_string();
            host.run_cancellable(req_id, async move {
                client
                    .delete_computer_async(&node_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "deleted": true, "node_id": node_id }))
        }
        "testConnection" => {
            let node_id = req_str(params, "node_id")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .test_computer_connection_async(&node_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "executeRemoteOp" => {
            let req: RemoteComputerCommandRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .execute_remote_computer_op_async(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "approveCommand" => {
            let command_id = req_str(params, "command_id")?.to_string();
            let approved = params
                .get("approved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let res = host.run_cancellable(req_id, async move {
                client
                    .approve_computer_command_async(&command_id, approved)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "completeCommand" => {
            let command_id = req_str(params, "command_id")?.to_string();
            let result: RemoteComputerCommandResult =
                parse_req(params.get("result").unwrap_or(params))?;
            host.run_cancellable(req_id, async move {
                client
                    .complete_computer_command_async(&command_id, &result)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "completed": true }))
        }
        "botProfileGet" => {
            let bot_id = req_u64(params, "bot_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_bot_computer_profile_async(bot_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "botProfileUpdate" => {
            let bot_id = req_u64(params, "bot_id")?;
            let req: UpdateBotComputerRequest = parse_req(params.get("profile").unwrap_or(params))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .update_bot_computer_profile_async(bot_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "acquireLease" => {
            let payload = params.clone();
            host.run_cancellable(req_id, async move {
                client
                    .acquire_computer_lease(&payload)
                    .await
                    .map_err(HostError::from)
            })
        }
        "executeAction" => {
            let payload = params.clone();
            host.run_cancellable(req_id, async move {
                client
                    .execute_computer_action(&payload)
                    .await
                    .map_err(HostError::from)
            })
        }
        "registerLocalHost" | "pollLocalCommands" | "executeLocalOp" => {
            host.require_non_ios_host_capability(&format!("computers.{action}"))?;
            Err(HostError::capability_unavailable(
                "Unattended local computer daemon hosting is desktop-only; iOS operates as a remote controller",
            ))
        }
        _ => Err(HostError::unknown_command(&format!("computers.{action}"))),
    }
}

pub fn handle_remote(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        // CRITICAL M1a NB-03 FAIL-CLOSED GUARD:
        // Never invoke fabushi_remote_engine desktop host-control modules locally on iOS!
        "localPower"
        | "localAccessibility"
        | "localInput"
        | "localCapture"
        | "localPtySpawn"
        | "localPrivacyScreen"
        | "localAudioMute"
        | "localClipboard" => {
            host.require_non_ios_host_capability(&format!("remote.{action}"))?;
            Err(HostError::capability_unavailable(format!(
                "remote.{action} is disabled in mobile host; route remote operations via computers.executeRemoteOp"
            )))
        }
        // Safe cross-platform remote assist & sessions APIs
        "assistInfo" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_remote_assist_info_async()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "assistResetCode" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .reset_remote_assist_code_async()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "assistSetCode" => {
            let code = req_str(params, "code")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .set_remote_assist_code_async(&code)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "assistAllow" => {
            let allow = params
                .get("allow_assist")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| HostError::invalid_arguments("Missing bool 'allow_assist'"))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .set_remote_assist_allow_async(allow)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "assistStart" => {
            let req: AssistStartRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .start_remote_assist_async(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "sessionsList" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_remote_sessions_async()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "sessionDisconnect" => {
            let session_id = req_str(params, "session_id")?.to_string();
            host.run_cancellable(req_id, async move {
                client
                    .disconnect_remote_session_async(&session_id)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "disconnected": true }))
        }
        "sessionsDisconnectAll" => {
            host.run_cancellable(req_id, async move {
                client
                    .disconnect_all_remote_sessions_async()
                    .await
                    .map_err(HostError::from)
            })
        }
        // Safe local RemoteControlHub lease & <10ms emergency takeover
        "hubRegisterSession" => {
            let sess: RemoteSessionDto = parse_req(params)?;
            let registered = host.remote_hub.register_session(sess);
            to_val(registered)
        }
        "hubAcquireLease" => {
            let sess: RemoteSessionDto = parse_req(params.get("session").unwrap_or(params))?;
            let allow_preempt_ai = params
                .get("allow_preempt_ai")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let granted = host
                .remote_hub
                .acquire_exclusive_lease(sess, allow_preempt_ai)
                .map_err(|e| HostError::conflict(e.to_string()))?;
            to_val(granted)
        }
        "hubEmergencyTakeover" => {
            let start = std::time::Instant::now();
            let disconnected = host.remote_hub.disconnect_all();
            let epoch = host.remote_hub.override_epoch();
            let elapsed_us = start.elapsed().as_micros() as u64;
            host.emit_event(
                "remoteEmergencyTakeover",
                json!({
                    "disconnected_sessions": disconnected,
                    "override_epoch": epoch,
                    "elapsed_us": elapsed_us,
                }),
            );
            Ok(json!({
                "disconnected_sessions": disconnected,
                "override_epoch": epoch,
                "elapsed_us": elapsed_us,
            }))
        }
        "hubPreemptionStatus" => Ok(json!({
            "override_epoch": host.remote_hub.override_epoch(),
            "active_count": host.remote_hub.active_count(),
            "sessions": host.remote_hub.list_sessions(),
        })),
        "ptyParseOsc52" => {
            let data = if let Some(b64) = params.get("data_b64").and_then(|v| v.as_str()) {
                BASE64_STANDARD
                    .decode(b64.trim())
                    .map_err(|e| HostError::invalid_arguments(format!("Invalid data_b64: {e}")))?
            } else {
                req_str(params, "data")?.as_bytes().to_vec()
            };
            let payloads = host.osc52_scanner.lock().feed(&data);
            to_val(payloads)
        }
        "remapModifiers" => {
            let modifiers: Vec<String> = params
                .get("modifiers")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let client_os = params
                .get("client_os")
                .and_then(|v| v.as_str())
                .unwrap_or("ios");
            let host_os = params
                .get("host_os")
                .and_then(|v| v.as_str())
                .unwrap_or("macos");
            let enabled = params
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let remapped =
                NativeInputEngine::remap_modifiers(&modifiers, client_os, host_os, enabled);
            Ok(json!({ "modifiers": remapped }))
        }
        "sendWakeOnLan" => {
            let mac = req_str(params, "mac_address")?.to_string();
            let broadcast_ip = params
                .get("broadcast_ip")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let msg = PowerManager::execute_power_action(&PowerActionKind::WakeOnLan {
                mac_address: mac,
                broadcast_ip,
            })
            .map_err(|e| HostError::network(e.to_string()))?;
            Ok(json!({ "sent": true, "detail": msg }))
        }
        _ => Err(HostError::unknown_command(&format!("remote.{action}"))),
    }
}

pub fn handle_tailnet(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "status" => to_val(client.tailnet_status()),
        "start" => {
            client.start_tailnet_mesh();
            to_val(client.tailnet_status())
        }
        "stop" => {
            client.stop_tailnet_mesh();
            to_val(client.tailnet_status())
        }
        "register" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .register_tailnet_async()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "netmap" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_tailnet_netmap_async()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "pendingApprovals" => to_val(client.tailnet_pending_approvals()),
        "decideApproval" => {
            let command_id = req_str(params, "command_id")?.to_string();
            let approved = params
                .get("approved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let outcome = host.run_cancellable(req_id, async move {
                Ok(client
                    .decide_tailnet_approval_async(&command_id, approved)
                    .await)
            })?;
            match outcome {
                Some(Ok(())) => Ok(json!({ "decided": true, "approved": approved })),
                Some(Err(e)) => Err(HostError::internal(e)),
                None => Err(HostError::not_found("Pending tailnet approval not found")),
            }
        }
        "setForceDerpOnly" => {
            let force = params
                .get("force")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| HostError::invalid_arguments("Missing bool 'force'"))?;
            client.set_tailnet_force_derp_only(force);
            Ok(json!({ "force_derp_only": force }))
        }
        _ => Err(HostError::unknown_command(&format!("tailnet.{action}"))),
    }
}

pub fn handle_miniapp(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "list" => {
            let query = params
                .get("query")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let category = params
                .get("category")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_mini_apps(query.as_deref(), category.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "listInstalled" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_installed_mini_apps()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "listRecent" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_recent_mini_apps()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "listAttachMenuBots" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_attach_menu_bots()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "install" => {
            let bot = req_str(params, "bot_username")?.to_string();
            let short = req_str(params, "short_name")?.to_string();
            let write = params
                .get("allow_write_access")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let res = host.run_cancellable(req_id, async move {
                client
                    .install_mini_app(&bot, &short, write)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "uninstall" => {
            let bot = req_str(params, "bot_username")?.to_string();
            let short = req_str(params, "short_name")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .uninstall_mini_app(&bot, &short)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "launch" | "launchWithUrl" => {
            let bot = req_str(params, "bot_username")?.to_string();
            let short = req_str(params, "short_name")?.to_string();
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64());
            let launch_mode: MiniAppLaunchMode = params
                .get("launch_mode")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or(MiniAppLaunchMode::FullSize);
            let start_param = params
                .get("start_param")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let custom_url = params
                .get("custom_url")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .launch_mini_app_with_url(
                        &bot,
                        &short,
                        chat_id,
                        launch_mode,
                        start_param.as_deref(),
                        custom_url.as_deref(),
                    )
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "validateInitData" => {
            let bot = req_str(params, "bot_username")?.to_string();
            let short = req_str(params, "short_name")?.to_string();
            let init_data = req_str(params, "init_data")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .validate_mini_app_init_data(&bot, &short, &init_data)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "verifyInitDataLocal" => {
            let init_data = req_str(params, "init_data")?;
            let bot_token = req_str(params, "bot_token")?;
            let valid = verify_telegram_init_data_hmac(init_data, bot_token)
                .map_err(HostError::crypto)?;
            Ok(json!({ "valid": valid }))
        }
        "sendAction" => {
            let bot = req_str(params, "bot_username")?.to_string();
            let short = req_str(params, "short_name")?.to_string();
            let req: MiniAppActionRequest = parse_req(params.get("request").unwrap_or(params))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .send_mini_app_action(&bot, &short, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "cloudStorageGet" => {
            let bot = req_str(params, "bot_username")?.to_string();
            let short = req_str(params, "short_name")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_cloud_storage(&bot, &short)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "cloudStorageSet" => {
            let bot = req_str(params, "bot_username")?.to_string();
            let short = req_str(params, "short_name")?.to_string();
            let key = req_str(params, "key")?.to_string();
            let value = params
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .set_cloud_storage_item(&bot, &short, &key, &value)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "cloudStorageDelete" => {
            let bot = req_str(params, "bot_username")?.to_string();
            let short = req_str(params, "short_name")?.to_string();
            let key = req_str(params, "key")?.to_string();
            let deleted = host.run_cancellable(req_id, async move {
                client
                    .delete_cloud_storage_item(&bot, &short, &key)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "deleted": deleted }))
        }
        "botProfile" => {
            let target = req_str(params, "target")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_bot_public_profile(&target)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "canBotSendMessage" => {
            let bot_id = req_u64(params, "bot_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .can_bot_send_message(bot_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "allowBotSendMessage" => {
            let bot_id = req_u64(params, "bot_id")?;
            let req: AllowSendMessageRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .allow_bot_send_message(bot_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "acceptTos" => {
            let bot_id = req_u64(params, "bot_id")?;
            let req: AcceptMiniAppTosRequest = parse_req(params)?;
            let ok = host.run_cancellable(req_id, async move {
                client
                    .accept_mini_app_tos(bot_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "accepted": ok }))
        }
        "setBotBlocked" => {
            let bot_id = req_u64(params, "bot_id")?;
            let blocked = params
                .get("blocked")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let ok = host.run_cancellable(req_id, async move {
                client
                    .set_bot_blocked(bot_id, blocked)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "blocked": blocked, "ok": ok }))
        }
        "parseUri" => {
            let uri = req_str(params, "uri")?;
            match parse_miniapp_uri(uri) {
                Some((bot, short, start_param)) => Ok(json!({
                    "valid": true,
                    "bot_username": bot,
                    "short_name": short,
                    "start_param": start_param,
                })),
                None => Ok(json!({ "valid": false })),
            }
        }
        "sanitizeCustomUrl" => {
            let raw = req_str(params, "url")?;
            Ok(json!({ "sanitized_url": sanitize_webapp_custom_url(raw) }))
        }
        "agentManifestValidate" => {
            let raw = req_str(params, "manifest_json")?;
            let manifest = AgentManifest::from_json(raw)
                .map_err(|e| HostError::invalid_arguments(format!("Invalid AgentManifest: {e}")))?;
            to_val(manifest)
        }
        // --- 全球法布施 (fabushi_miniapp pure-Rust core & SQLite storage) ---
        "dharmaStateSnapshot" => {
            let snap = host.dharma_state.lock().snapshot();
            to_val(snap)
        }
        "dharmaDispatch" => {
            let cmd: UiCommand = parse_req(params.get("command").unwrap_or(params))?;
            let mut state = host.dharma_state.lock();
            let result = state.dispatch(cmd);
            let snap = state.snapshot();
            Ok(json!({
                "effects_count": result.effects.len(),
                "snapshot": snap,
            }))
        }
        "dharmaBootstrap" => {
            let user_id = params.get("user_id").and_then(|v| v.as_i64()).unwrap_or(1);
            let username = params
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or("practitioner");
            let display_name = params
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Practitioner");
            let conn = host.miniapp_db.lock();
            let bundle = fabushi_miniapp::storage::get_bootstrap_bundle(
                &conn,
                user_id,
                username,
                display_name,
                "",
                false,
            )
            .map_err(|e| HostError::storage(e.to_string()))?;
            to_val(bundle)
        }
        "dharmaRecordPractice" => {
            let user_id = params.get("user_id").and_then(|v| v.as_i64()).unwrap_or(1);
            let sutra = req_str(params, "sutra")?;
            let duration_minutes = params
                .get("duration_minutes")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as u32;
            let chant_count = params
                .get("chant_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as u32;
            let notes = params.get("notes").and_then(|v| v.as_str());
            let conn = host.miniapp_db.lock();
            let _ = fabushi_miniapp::storage::ensure_profile(
                &conn,
                user_id,
                "practitioner",
                "Practitioner",
                "",
                false,
            );
            let res = fabushi_miniapp::storage::record_practice_session(
                &conn,
                user_id,
                sutra,
                duration_minutes,
                chant_count,
                None,
                None,
                notes,
            )
            .map_err(|e| HostError::storage(e.to_string()))?;
            to_val(res)
        }
        "dharmaLeaderboard" => {
            let viewer_id = params.get("user_id").and_then(|v| v.as_i64()).unwrap_or(1);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let conn = host.miniapp_db.lock();
            let board = fabushi_miniapp::storage::get_global_leaderboard(&conn, viewer_id, limit)
                .map_err(|e| HostError::storage(e.to_string()))?;
            to_val(board)
        }
        "dharmaAssetBytes" => {
            let name = req_str(params, "name")?;
            let (bytes, mime) = fabushi_miniapp::asset_bytes(name)
                .ok_or_else(|| HostError::not_found(format!("MiniApp asset '{name}' not found")))?;
            Ok(json!({
                "name": name,
                "mime_type": mime,
                "size_bytes": bytes.len(),
                "has_wasm_magic": bytes.starts_with(b"\0asm"),
            }))
        }
        _ => Err(HostError::unknown_command(&format!("miniapp.{action}"))),
    }
}

pub fn handle_webmcp(
    host: &FabushiAppHost,
    _req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "sendResult" => {
            let resp: MiniAppWebMcpResponse = parse_req(params)?;
            Ok(json!({ "sent": client.send_webmcp_result(resp) }))
        }
        "sendTakeoverState" => {
            let evt: MiniAppTakeoverEvent = parse_req(params)?;
            let key = format!("{}/{}", evt.bot_username, evt.short_name);
            host.webmcp_control_modes.lock().insert(key, evt.mode);
            Ok(json!({ "sent": client.send_miniapp_takeover_state(evt) }))
        }
        "setControlMode" => {
            let key = req_str(params, "app_key")?.to_string();
            let mode: MiniAppControlMode = parse_req(
                params
                    .get("mode")
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'mode'"))?,
            )?;
            host.webmcp_control_modes.lock().insert(key.clone(), mode);
            Ok(json!({ "app_key": key, "mode": mode }))
        }
        "getControlState" => {
            let key = req_str(params, "app_key")?;
            let mode = host
                .webmcp_control_modes
                .lock()
                .get(key)
                .copied()
                .unwrap_or(MiniAppControlMode::Idle);
            Ok(json!({ "app_key": key, "mode": mode }))
        }
        "validateToolCall" => {
            let manifest_raw = req_str(params, "manifest_json")?;
            let action_name = req_str(params, "action")?;
            let args = params.get("args").cloned().unwrap_or_else(|| json!({}));
            let manifest = AgentManifest::from_json(manifest_raw)
                .map_err(|e| HostError::invalid_arguments(e.to_string()))?;
            manifest
                .validate_action_call(action_name, &args)
                .map_err(|e| HostError::invalid_arguments(e.to_string()))?;
            Ok(json!({ "valid": true, "action": action_name }))
        }
        _ => Err(HostError::unknown_command(&format!("webmcp.{action}"))),
    }
}

pub fn handle_calls(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        // Server call signaling endpoints
        "list" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_calls().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "invite" => {
            let req: CallInviteRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.call_invite(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "accept" => {
            let call_id = req_str(params, "call_id")?.to_string();
            let res = host.run_cancellable(req_id, async move {
                client.call_accept(&call_id).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "decline" => {
            let call_id = req_str(params, "call_id")?.to_string();
            host.run_cancellable(req_id, async move {
                client.call_decline(&call_id).await.map_err(HostError::from)
            })?;
            Ok(json!({ "declined": true }))
        }
        "hangup" => {
            let call_id = req_str(params, "call_id")?.to_string();
            host.run_cancellable(req_id, async move {
                client.call_hangup(&call_id).await.map_err(HostError::from)
            })?;
            Ok(json!({ "hung_up": true }))
        }
        "rate" => {
            let call_id = req_str(params, "call_id")?.to_string();
            let rating = params
                .get("rating")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| HostError::invalid_arguments("Missing u8 'rating'"))?
                as u8;
            let feedback = params
                .get("feedback")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .rate_call_with_feedback(&call_id, rating, feedback.as_deref())
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "signal" => {
            let target_uid = params.get("target_user_id").and_then(|v| v.as_u64());
            let req: CallSignalRequest = parse_req(params)?;
            host.run_cancellable(req_id, async move {
                client
                    .send_call_signal(req, target_uid)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "sent": true }))
        }
        "turnCredentials" => host.run_cancellable(req_id, async move {
            client
                .get_turn_credentials()
                .await
                .map_err(HostError::from)
        }),
        // Real cross-platform WebRtcCallEngine (ZERO peer_transport references!)
        "engineStartOutgoing" => {
            let call_id = req_i64(params, "call_id")?;
            let local_user = req_u64(params, "local_user_id")?;
            let remote_user = req_u64(params, "remote_user_id")?;
            let is_video = params
                .get("is_video")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut engine = host.call_engine.lock();
            engine.start_outgoing(call_id, UserId(local_user), UserId(remote_user), is_video);
            let outbound = engine.drain_outbound_frames();
            Ok(json!({
                "call_id": call_id,
                "local_sdp": engine.local_sdp,
                "outbound_frames": outbound,
            }))
        }
        "engineReceiveIncoming" => {
            let call_id = req_i64(params, "call_id")?;
            let caller = req_u64(params, "caller_user_id")?;
            let callee = req_u64(params, "callee_user_id")?;
            let is_video = params
                .get("is_video")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut engine = host.call_engine.lock();
            engine.receive_incoming(call_id, UserId(caller), UserId(callee), is_video);
            let outbound = engine.drain_outbound_frames();
            Ok(json!({ "call_id": call_id, "outbound_frames": outbound }))
        }
        "engineAcceptIncoming" => {
            let mut engine = host.call_engine.lock();
            let answer = engine.accept_incoming();
            let outbound = engine.drain_outbound_frames();
            let fp = engine.emoji_fingerprint().map(|f| f.as_emoji_string());
            Ok(json!({
                "answer": answer,
                "outbound_frames": outbound,
                "emoji_fingerprint": fp,
            }))
        }
        "engineApplySignal" => {
            let frame: SignalingFrame = parse_req(params)?;
            let mut engine = host.call_engine.lock();
            engine.apply_signaling_frame(frame);
            let fp = engine.emoji_fingerprint().map(|f| f.as_emoji_string());
            let outbound = engine.drain_outbound_frames();
            Ok(json!({
                "applied": true,
                "emoji_fingerprint": fp,
                "outbound_frames": outbound,
            }))
        }
        "engineHangup" => {
            let reason = params
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("local_hangup");
            let mut engine = host.call_engine.lock();
            engine.hangup_local(reason);
            let outbound = engine.drain_outbound_frames();
            Ok(json!({ "hung_up": true, "outbound_frames": outbound }))
        }
        "engineEmojiFingerprint" => {
            if let Some(hex_sha) = params.get("sha256_hex").and_then(|v| v.as_str()) {
                let raw = hex::decode(hex_sha.trim())
                    .map_err(|e| HostError::invalid_arguments(format!("Invalid sha256_hex: {e}")))?;
                let arr: [u8; 32] = raw
                    .try_into()
                    .map_err(|_| HostError::invalid_arguments("sha256_hex must be 32 bytes"))?;
                let fp = CallsEmojiFingerprint::from_sha256(arr);
                return Ok(json!({ "emojis": fp.as_emoji_string() }));
            }
            let engine = host.call_engine.lock();
            let fp = engine.emoji_fingerprint().map(|f| f.as_emoji_string());
            Ok(json!({ "emojis": fp }))
        }
        "enginePushAudioLevel" => {
            let pcm: Vec<i16> = params
                .get("pcm_i16")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_i64().map(|n| n as i16)).collect())
                .ok_or_else(|| HostError::invalid_arguments("Missing array 'pcm_i16'"))?;
            let mut engine = host.call_engine.lock();
            let level = engine.push_audio_level_pcm(&pcm);
            Ok(json!({
                "level": level,
                "is_speaking": engine.meter.is_speaking(),
            }))
        }
        "sfuJoinRoom" => {
            let chat_id = req_i64(params, "chat_id")?;
            let user_id = req_u64(params, "user_id")?;
            let mut sfu = host.sfu_bridge.lock();
            sfu.join_room(ChatId(chat_id), UserId(user_id));
            let subs: Vec<u64> = sfu
                .get_subscribers(ChatId(chat_id))
                .iter()
                .map(|u| u.0)
                .collect();
            Ok(json!({ "chat_id": chat_id, "subscribers": subs }))
        }
        "sfuLeaveRoom" => {
            let chat_id = req_i64(params, "chat_id")?;
            let user_id = req_u64(params, "user_id")?;
            let mut sfu = host.sfu_bridge.lock();
            sfu.leave_room(ChatId(chat_id), UserId(user_id));
            let subs: Vec<u64> = sfu
                .get_subscribers(ChatId(chat_id))
                .iter()
                .map(|u| u.0)
                .collect();
            Ok(json!({ "chat_id": chat_id, "subscribers": subs }))
        }
        _ => Err(HostError::unknown_command(&format!("calls.{action}"))),
    }
}

pub fn handle_talk(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "createSession" => {
            if client.token().is_some()
                && params
                    .get("remote")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                let payload = params.clone();
                return host.run_cancellable(req_id, async move {
                    client
                        .create_talk_session(&payload)
                        .await
                        .map_err(HostError::from)
                });
            }
            let mut tc = host.talk_coordinator.lock();
            *tc = gpui_telegram_agent::talk::TalkCoordinator::new();
            let _ = tc.transition(TalkSessionState::Listening);
            Ok(json!({
                "session_id": tc.session_id,
                "state": tc.state,
                "generation": tc.generation,
            }))
        }
        "transition" => {
            let next: TalkSessionState = parse_req(
                params
                    .get("state")
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'state'"))?,
            )?;
            let mut tc = host.talk_coordinator.lock();
            tc.transition(next)
                .map_err(HostError::invalid_arguments)?;
            Ok(json!({ "session_id": tc.session_id, "state": tc.state }))
        }
        "bargeIn" => {
            let mut tc = host.talk_coordinator.lock();
            let flushed = tc.handle_barge_in();
            Ok(json!({
                "session_id": tc.session_id,
                "state": tc.state,
                "flushed_chunks": flushed,
            }))
        }
        "resumeListening" => {
            let mut tc = host.talk_coordinator.lock();
            tc.resume_listening();
            Ok(json!({ "session_id": tc.session_id, "state": tc.state }))
        }
        "enqueueAudio" => {
            let chunk = BASE64_STANDARD
                .decode(req_str(params, "chunk_b64")?)
                .map_err(|e| HostError::invalid_arguments(format!("Invalid chunk_b64: {e}")))?;
            let mut tc = host.talk_coordinator.lock();
            tc.enqueue_audio(chunk)
                .map_err(HostError::invalid_arguments)?;
            Ok(json!({
                "buffered_chunks": tc.buffered_chunks_count(),
                "state": tc.state,
            }))
        }
        "dequeueAudio" => {
            let mut tc = host.talk_coordinator.lock();
            let chunk = tc.dequeue_audio();
            Ok(json!({
                "chunk_b64": chunk.map(|b| BASE64_STANDARD.encode(b)),
                "buffered_chunks": tc.buffered_chunks_count(),
            }))
        }
        "close" => {
            let mut tc = host.talk_coordinator.lock();
            tc.close();
            Ok(json!({ "session_id": tc.session_id, "state": tc.state }))
        }
        "processVadPcm" => {
            let pcm: Vec<i16> = params
                .get("pcm_i16")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_i64().map(|n| n as i16)).collect())
                .ok_or_else(|| HostError::invalid_arguments("Missing array 'pcm_i16'"))?;
            let mut vad = host.vad_detector.lock();
            let barge_in_triggered = vad.process_pcm_frame(&pcm);
            let silence_detected = vad.is_silence_detected();
            let tap_barge_in = {
                host.ai_media_tap.on_incoming_pcm_frame(&pcm);
                host.ai_media_tap.trigger_vad_barge_in()
            };
            let mut flushed_chunks = 0usize;
            if barge_in_triggered {
                let mut tc = host.talk_coordinator.lock();
                flushed_chunks = tc.handle_barge_in();
            }
            Ok(json!({
                "barge_in_triggered": barge_in_triggered,
                "ai_media_tap_barge_in": tap_barge_in,
                "silence_detected": silence_detected,
                "consecutive_speech_frames": vad.consecutive_speech_frames,
                "flushed_chunks": flushed_chunks,
            }))
        }
        "turn" => {
            let req: HermesTalkTurnRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.hermes_talk_turn(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "callInvite" => {
            let req: HermesCallInviteRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .hermes_call_invite(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        _ => Err(HostError::unknown_command(&format!("talk.{action}"))),
    }
}
