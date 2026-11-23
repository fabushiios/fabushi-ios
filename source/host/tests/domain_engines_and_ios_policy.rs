use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use mahayana_app_host::{
    fabushi_host_create, fabushi_host_destroy, fabushi_host_dispatch, fabushi_host_free_string,
    telegram_bridge_register_event_callback,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

unsafe fn dispatch(handle: *mut c_void, payload: &Value) -> Value {
    let req_c = CString::new(payload.to_string()).unwrap();
    let ptr = unsafe { fabushi_host_dispatch(handle, req_c.as_ptr()) };
    assert!(!ptr.is_null());
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
    unsafe { fabushi_host_free_string(ptr) };
    serde_json::from_str(&text).unwrap()
}

fn create_temp_host() -> (tempfile::TempDir, *mut c_void) {
    let dir = tempdir().unwrap();
    let cfg = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "ios_sandbox_strict": true
        })
        .to_string(),
    )
    .unwrap();
    let host = unsafe { fabushi_host_create(cfg.as_ptr()) };
    assert!(!host.is_null());
    (dir, host)
}

fn create_temp_test_host() -> (tempfile::TempDir, *mut c_void) {
    let dir = tempdir().unwrap();
    let cfg = CString::new(
        json!({
            "app_data_dir": dir.path().to_string_lossy(),
            "ios_sandbox_strict": true,
            "test_mode": true
        })
        .to_string(),
    )
    .unwrap();
    let host = unsafe { fabushi_host_create(cfg.as_ptr()) };
    assert!(!host.is_null());
    (dir, host)
}

#[test]
fn test_real_e2ee_double_ratchet_sender_keys_and_safety_numbers() {
    let (_dir, host) = create_temp_host();
    unsafe {
        // 1. Device identity + 4-emoji safety number symmetry
        let id = dispatch(host, &json!({"method": "e2ee.identity"}));
        assert_eq!(id["ok"], true);
        let key_a = id["result"]["x25519_pub_b64"].as_str().unwrap().to_string();
        let key_b = BASE64_STANDARD.encode([0x42u8; 32]);

        let fp_ab = dispatch(
            host,
            &json!({
                "method": "e2ee.safetyFingerprint",
                "params": {"local_identity_key_b64": key_a, "remote_identity_key_b64": key_b}
            }),
        );
        let fp_ba = dispatch(
            host,
            &json!({
                "method": "e2ee.safetyFingerprint",
                "params": {"local_identity_key_b64": key_b, "remote_identity_key_b64": key_a}
            }),
        );
        assert_eq!(fp_ab["ok"], true);
        assert_eq!(fp_ab["result"]["emojis"], fp_ba["result"]["emojis"]);
        assert!(fp_ab["result"]["emojis"].as_str().unwrap().chars().count() >= 4);

        // 2. Double Ratchet Alice <-> Bob bidirectional roundtrip
        assert_eq!(
            dispatch(
                host,
                &json!({
                    "method": "e2ee.ratchetInitPair",
                    "params": {"alice_session_id": "alice", "bob_session_id": "bob"}
                })
            )["ok"],
            true
        );
        let enc1 = dispatch(
            host,
            &json!({
                "method": "e2ee.ratchetEncrypt",
                "params": {"session_id": "alice", "plaintext": "Secret hello from Alice"}
            }),
        );
        assert_eq!(enc1["ok"], true);
        let dec1 = dispatch(
            host,
            &json!({
                "method": "e2ee.ratchetDecrypt",
                "params": {
                    "session_id": "bob",
                    "header": enc1["result"]["header"],
                    "ciphertext_b64": enc1["result"]["ciphertext_b64"]
                }
            }),
        );
        assert_eq!(dec1["ok"], true);
        assert_eq!(dec1["result"]["plaintext_utf8"], "Secret hello from Alice");

        // Reply Bob -> Alice
        let enc2 = dispatch(
            host,
            &json!({
                "method": "e2ee.ratchetEncrypt",
                "params": {"session_id": "bob", "plaintext": "Ratchet reply from Bob"}
            }),
        );
        let dec2 = dispatch(
            host,
            &json!({
                "method": "e2ee.ratchetDecrypt",
                "params": {
                    "session_id": "alice",
                    "header": enc2["result"]["header"],
                    "ciphertext_b64": enc2["result"]["ciphertext_b64"]
                }
            }),
        );
        assert_eq!(dec2["result"]["plaintext_utf8"], "Ratchet reply from Bob");

        // 3. Sender Keys group encryption roundtrip
        let sk = dispatch(
            host,
            &json!({
                "method": "e2ee.senderKeyCreate",
                "params": {
                    "key_id": "grp-sender",
                    "chat_id": 777,
                    "sender_user_id": 10,
                    "sender_device_id": "ios-dev-1"
                }
            }),
        );
        assert_eq!(sk["ok"], true);
        assert_eq!(
            dispatch(
                host,
                &json!({
                    "method": "e2ee.senderKeyImportDistribution",
                    "params": {
                        "key_id": "grp-receiver",
                        "distribution": sk["result"]["distribution"]
                    }
                })
            )["ok"],
            true
        );
        let grp_ct = dispatch(
            host,
            &json!({
                "method": "e2ee.senderKeyEncrypt",
                "params": {"key_id": "grp-sender", "plaintext": "Group broadcast payload"}
            }),
        );
        assert_eq!(grp_ct["ok"], true);
        let grp_pt = dispatch(
            host,
            &json!({
                "method": "e2ee.senderKeyDecrypt",
                "params": {"key_id": "grp-receiver", "ciphertext": grp_ct["result"]}
            }),
        );
        assert_eq!(grp_pt["result"]["plaintext_utf8"], "Group broadcast payload");

        // 4. Negative crypto integrity: tampered Double Ratchet & Sender Key ciphertexts MUST fail closed
        let enc_tamper = dispatch(
            host,
            &json!({
                "method": "e2ee.ratchetEncrypt",
                "params": {"session_id": "alice", "plaintext": "Authentic payload"}
            }),
        );
        assert_eq!(enc_tamper["ok"], true);
        let mut ct_bytes = BASE64_STANDARD
            .decode(enc_tamper["result"]["ciphertext_b64"].as_str().unwrap())
            .unwrap();
        ct_bytes[0] ^= 0xFF;
        let bad_dr = dispatch(
            host,
            &json!({
                "method": "e2ee.ratchetDecrypt",
                "params": {
                    "session_id": "bob",
                    "header": enc_tamper["result"]["header"],
                    "ciphertext_b64": BASE64_STANDARD.encode(&ct_bytes)
                }
            }),
        );
        assert_eq!(bad_dr["ok"], false, "Tampered DR ciphertext must be rejected: {bad_dr}");
        assert_eq!(bad_dr["error"]["code"], "crypto_error");

        let grp_tamper = dispatch(
            host,
            &json!({
                "method": "e2ee.senderKeyEncrypt",
                "params": {"key_id": "grp-sender", "plaintext": "Group secret"}
            }),
        );
        assert_eq!(grp_tamper["ok"], true);
        let mut bad_grp_obj = grp_tamper["result"].clone();
        let mut g_ct = BASE64_STANDARD
            .decode(bad_grp_obj["ciphertext"].as_str().unwrap())
            .unwrap();
        g_ct[0] ^= 0xAA;
        bad_grp_obj["ciphertext"] = json!(BASE64_STANDARD.encode(&g_ct));
        let bad_sk = dispatch(
            host,
            &json!({
                "method": "e2ee.senderKeyDecrypt",
                "params": {"key_id": "grp-receiver", "ciphertext": bad_grp_obj}
            }),
        );
        assert_eq!(bad_sk["ok"], false, "Tampered Sender Key ciphertext must be rejected: {bad_sk}");
        assert_eq!(bad_sk["error"]["code"], "crypto_error");

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_real_split_key_v2_argon2id_x25519_vault_and_rfc6238_totp() {
    // Production handle (test_mode: false) MUST reject fast_kdf: true and weak kdf_params!
    let (_prod_dir, prod_host) = create_temp_host();
    unsafe {
        let rej_fast = dispatch(
            prod_host,
            &json!({
                "method": "vault.deriveSplitKey",
                "params": {"password": "CorrectHorseBatteryStaple!", "fast_kdf": true}
            }),
        );
        assert_eq!(rej_fast["ok"], false, "Production handle must reject fast_kdf: {rej_fast}");
        assert_eq!(rej_fast["error"]["code"], "invalid_arguments");

        let rej_weak = dispatch(
            prod_host,
            &json!({
                "method": "vault.initSplitKey",
                "params": {
                    "user_id": 42,
                    "password": "CorrectHorseBatteryStaple!",
                    "kdf_params": {"m_cost_kib": 1024, "t_cost": 1, "p_cost": 1}
                }
            }),
        );
        assert_eq!(rej_weak["ok"], false, "Production handle must reject weak kdf_params: {rej_weak}");
        assert_eq!(rej_weak["error"]["code"], "invalid_arguments");
        fabushi_host_destroy(prod_host);
    }

    // Test-mode handle exercises full Split-Key V2, Vault, X25519 Device Trust Ring, and RFC 6238 TOTP
    let (_dir, host) = create_temp_test_host();
    unsafe {
        // 1. Split-Key V2 Argon2id derivation & vault initialization
        let derived = dispatch(
            host,
            &json!({
                "method": "vault.deriveSplitKey",
                "params": {"password": "CorrectHorseBatteryStaple!", "fast_kdf": true}
            }),
        );
        assert_eq!(derived["ok"], true);
        assert!(!derived["result"]["auth_verifier_b64"].as_str().unwrap().is_empty());

        let init = dispatch(
            host,
            &json!({
                "method": "vault.initSplitKey",
                "params": {"user_id": 42, "password": "CorrectHorseBatteryStaple!", "fast_kdf": true}
            }),
        );
        assert_eq!(init["ok"], true);
        let envelope = init["result"]["envelope"].clone();
        let root_key_b64 = init["result"]["root_key_b64"].as_str().unwrap().to_string();

        // Store & decrypt credential + verify origin binding
        let stored = dispatch(
            host,
            &json!({
                "method": "vault.storeCredential",
                "params": {
                    "origin": "https://github.com",
                    "username": "alice@example.com",
                    "password": "super-secret-password",
                    "totp_secret": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
                }
            }),
        );
        assert_eq!(stored["ok"], true);
        let cred_id = stored["result"]["credential_id"].as_str().unwrap().to_string();

        // Lock -> fail closed while locked -> unlock with split key
        assert_eq!(dispatch(host, &json!({"method": "vault.lock"}))["result"]["unlocked"], false);
        let locked_err = dispatch(
            host,
            &json!({"method": "vault.listForOrigin", "params": {"origin": "https://github.com"}}),
        );
        assert_eq!(locked_err["ok"], false);
        assert_eq!(locked_err["error"]["code"], "unauthorized");

        let unlocked = dispatch(
            host,
            &json!({
                "method": "vault.unlockWithSplitKey",
                "params": {
                    "user_id": 42,
                    "password": "CorrectHorseBatteryStaple!",
                    "envelope": envelope
                }
            }),
        );
        assert_eq!(unlocked["ok"], true);
        assert_eq!(unlocked["result"]["root_key_b64"], root_key_b64);

        let dec = dispatch(
            host,
            &json!({
                "method": "vault.decryptForOrigin",
                "params": {"credential_id": cred_id, "origin": "https://github.com"}
            }),
        );
        assert_eq!(dec["ok"], true);
        assert_eq!(dec["result"]["username"], "alice@example.com");
        assert_eq!(dec["result"]["password"], "super-secret-password");

        // Wrong origin MUST fail closed
        let wrong_origin = dispatch(
            host,
            &json!({
                "method": "vault.decryptForOrigin",
                "params": {"credential_id": cred_id, "origin": "https://evil.example"}
            }),
        );
        assert_eq!(wrong_origin["ok"], false);

        // 2. X25519 Device Trust Ring seal/open & 6-digit SAS code
        let kp = dispatch(
            host,
            &json!({"method": "vault.deviceSyncKeygen", "params": {"pair_id": "ipad-1"}}),
        );
        assert_eq!(kp["ok"], true);
        let pub_b64 = kp["result"]["public_key_b64"].as_str().unwrap();
        let sas_code = kp["result"]["verification_code"].as_str().unwrap();
        assert_eq!(sas_code.len(), 7); // NNN-NNN

        let sealed = dispatch(
            host,
            &json!({
                "method": "vault.deviceSyncSeal",
                "params": {
                    "request_id": "req-sync-001",
                    "user_id": 42,
                    "target_device_pub_b64": pub_b64
                }
            }),
        );
        assert_eq!(sealed["ok"], true);
        assert_eq!(sealed["result"]["verification_code"], sas_code);

        // Lock then open transfer on target device keypair
        let _ = dispatch(host, &json!({"method": "vault.lock"}));
        let opened = dispatch(
            host,
            &json!({
                "method": "vault.deviceSyncOpen",
                "params": {
                    "pair_id": "ipad-1",
                    "request_id": "req-sync-001",
                    "user_id": 42,
                    "transfer": sealed["result"]["transfer"]
                }
            }),
        );
        assert_eq!(opened["ok"], true);
        assert_eq!(opened["result"]["root_key_b64"], root_key_b64);

        // 3. RFC 6238 SHA-1 TOTP test vector (secret = "12345678901234567890" -> Base32 GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ, t=59 -> 94287082)
        let totp8 = dispatch(
            host,
            &json!({
                "method": "vault.totpGenerate",
                "params": {
                    "secret": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
                    "unix_secs": 59,
                    "digits": 8,
                    "period": 30
                }
            }),
        );
        assert_eq!(totp8["ok"], true);
        assert_eq!(totp8["result"]["code"], "94287082");

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_real_waveform_5bit_63byte_and_sha256_chunked_cas() {
    let (_dir, host) = create_temp_host();
    unsafe {
        // 100 bars (0..31) -> exactly 63 packed bytes -> exact decode roundtrip
        let bars: Vec<u8> = (0..100).map(|i| (i % 32) as u8).collect();
        let enc = dispatch(
            host,
            &json!({"method": "media.encodeWaveform", "params": {"bars": bars}}),
        );
        assert_eq!(enc["ok"], true);
        assert_eq!(enc["result"]["byte_count"], 63);

        let dec = dispatch(
            host,
            &json!({
                "method": "media.decodeWaveform",
                "params": {"waveform_b64": enc["result"]["waveform_b64"]}
            }),
        );
        assert_eq!(dec["ok"], true);
        let decoded_bars: Vec<u8> = dec["result"]["bars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        assert_eq!(&decoded_bars[..100], &bars[..]);

        // PCM i16 -> 63-byte 5-bit waveform
        let pcm: Vec<i16> = (0..2000).map(|i| ((i as f32 * 0.1).sin() * 20000.0) as i16).collect();
        let wf_pcm = dispatch(
            host,
            &json!({"method": "media.waveformFromPcm", "params": {"pcm_i16": pcm}}),
        );
        assert_eq!(wf_pcm["ok"], true);
        assert_eq!(wf_pcm["result"]["byte_count"], 63);

        // SHA-256 chunked upload session + out-of-order receive + CAS put/get/purge
        let payload = vec![0xABu8; 70 * 1024];
        let expected_sha = hex::encode(Sha256::digest(&payload));
        let create = dispatch(
            host,
            &json!({
                "method": "media.chunkedSessionCreate",
                "params": {
                    "file_id": "voice-001",
                    "total_size": payload.len(),
                    "part_size": 32768,
                    "expected_sha256": expected_sha
                }
            }),
        );
        assert_eq!(create["ok"], true);
        assert_eq!(create["result"]["total_parts"], 3);

        // Send parts out-of-order: 2, 0, 1
        for (idx, chunk) in [
            (2usize, &payload[65536..]),
            (0usize, &payload[0..32768]),
            (1usize, &payload[32768..65536]),
        ] {
            let part_res = dispatch(
                host,
                &json!({
                    "method": "media.chunkedSessionReceivePart",
                    "params": {
                        "file_id": "voice-001",
                        "part_index": idx,
                        "data_b64": BASE64_STANDARD.encode(chunk)
                    }
                }),
            );
            assert_eq!(part_res["ok"], true, "part {idx} failed: {part_res}");
        }

        let fin = dispatch(
            host,
            &json!({
                "method": "media.chunkedSessionFinalize",
                "params": {"file_id": "voice-001", "mime_type": "audio/ogg"}
            }),
        );
        assert_eq!(fin["ok"], true, "finalize failed: {fin}");
        assert_eq!(fin["result"]["verified"], true);
        assert_eq!(fin["result"]["sha256"], expected_sha);

        let cas_get = dispatch(
            host,
            &json!({"method": "media.casGet", "params": {"file_id": "voice-001"}}),
        );
        assert_eq!(cas_get["ok"], true);
        assert_eq!(cas_get["result"]["size_bytes"], payload.len());

        let stats = dispatch(host, &json!({"method": "media.casStats"}));
        assert!(stats["result"]["media_cas_bytes"].as_u64().unwrap() >= payload.len() as u64);

        let purge = dispatch(host, &json!({"method": "media.casPurge"}));
        assert_eq!(purge["ok"], true);
        assert!(purge["result"]["freed_bytes"].as_u64().unwrap() >= payload.len() as u64);

        // Negative integrity: SHA-256 mismatch MUST fail closed on media.chunkedSessionFinalize
        let bad_payload = b"corrupted chunk bytes";
        assert_eq!(
            dispatch(
                host,
                &json!({
                    "method": "media.chunkedSessionCreate",
                    "params": {
                        "file_id": "bad-sha-001",
                        "total_size": bad_payload.len(),
                        "part_size": 1024,
                        "expected_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
                    }
                })
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host,
                &json!({
                    "method": "media.chunkedSessionReceivePart",
                    "params": {
                        "file_id": "bad-sha-001",
                        "part_index": 0,
                        "data_b64": BASE64_STANDARD.encode(bad_payload)
                    }
                })
            )["ok"],
            true
        );
        let bad_fin = dispatch(
            host,
            &json!({
                "method": "media.chunkedSessionFinalize",
                "params": {"file_id": "bad-sha-001"}
            }),
        );
        assert_eq!(bad_fin["ok"], false, "SHA-256 mismatch must fail closed: {bad_fin}");
        assert_eq!(bad_fin["error"]["code"], "crypto_error");

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_real_magpie_masking_routing_and_presets() {
    let (_dir, host) = create_temp_host();
    unsafe {
        let presets = dispatch(host, &json!({"method": "magpie.listPresets"}));
        assert_eq!(presets["ok"], true);
        assert!(presets["result"]["count"].as_u64().unwrap() >= 40);

        let zhipu = dispatch(
            host,
            &json!({
                "method": "magpie.resolvePresetEndpoints",
                "params": {"preset_id": "zhipu", "region": "coding"}
            }),
        );
        assert_eq!(zhipu["ok"], true);
        assert_eq!(
            zhipu["result"]["base_url"],
            "https://open.bigmodel.cn/api/coding/paas/v4"
        );

        // PII Masking + Unmasking roundtrip
        let raw = "Key=sk-ant-api03-secret9988776655 email=alice@realcorp.io";
        let masked = dispatch(
            host,
            &json!({"method": "magpie.maskText", "params": {"text": raw}}),
        );
        assert_eq!(masked["ok"], true);
        let masked_str = masked["result"]["masked_text"].as_str().unwrap();
        assert!(!masked_str.contains("sk-ant-api03-secret9988776655"));
        assert!(!masked_str.contains("alice@realcorp.io"));

        let unmasked = dispatch(
            host,
            &json!({"method": "magpie.unmaskText", "params": {"text": masked_str}}),
        );
        assert_eq!(unmasked["result"]["text"], raw);

        // magpie.maskingPreview uses preview_text: works even when enabled=false and does NOT retain secrets in RAM vault!
        assert_eq!(
            dispatch(
                host,
                &json!({
                    "method": "magpie.maskingUpdate",
                    "params": {"enabled": false, "mask_secrets": true, "mask_personal": true}
                })
            )["ok"],
            true
        );
        let preview_raw = "PreviewKey=sk-ant-api03-previewOnly991122";
        let preview = dispatch(
            host,
            &json!({"method": "magpie.maskingPreview", "params": {"text": preview_raw}}),
        );
        assert_eq!(preview["ok"], true);
        let preview_masked = preview["result"]["masked_text"].as_str().unwrap();
        assert!(!preview_masked.contains("sk-ant-api03-previewOnly991122"));
        let preview_unmask = dispatch(
            host,
            &json!({"method": "magpie.unmaskText", "params": {"text": preview_masked}}),
        );
        assert!(
            !preview_unmask["result"]["text"]
                .as_str()
                .unwrap()
                .contains("sk-ant-api03-previewOnly991122"),
            "maskingPreview must not persist preview secrets into unmask vault"
        );

        // Routing group + circuit breaker cooldown
        let grp = dispatch(
            host,
            &json!({
                "method": "magpie.upsertGroup",
                "params": {
                    "id": "mix",
                    "name": "Mix",
                    "strategy": "drain",
                    "members": [
                        {"target": "openai/gpt-4o", "weight": 1, "enabled": true},
                        {"target": "deepseek/deepseek-chat", "weight": 1, "enabled": true}
                    ]
                }
            }),
        );
        assert_eq!(grp["ok"], true, "upsertGroup failed: {grp}");

        let r1 = dispatch(
            host,
            &json!({"method": "magpie.resolveRoute", "params": {"target": "group/mix", "prompt": "Explain TCP congestion control in detail.", "now_ms": 1000000}}),
        );
        assert_eq!(r1["ok"], true);
        assert_eq!(r1["result"][0]["provider_id"], "openai");

        let _ = dispatch(
            host,
            &json!({"method": "magpie.recordFailure", "params": {"target": "openai/gpt-4o", "status_code": 429, "retry_after_secs": 60, "now_ms": 1000000}}),
        );
        let r2 = dispatch(
            host,
            &json!({"method": "magpie.resolveRoute", "params": {"target": "group/mix", "prompt": "Explain TCP congestion control in detail.", "now_ms": 1000000}}),
        );
        assert_eq!(r2["result"][0]["provider_id"], "deepseek");

        // Import link & dotenv
        let imp = dispatch(
            host,
            &json!({"method": "magpie.importLink", "params": {"uri": "magpie://import?preset=zhipu&region=coding&key=sk-123"}}),
        );
        assert_eq!(imp["ok"], true);
        assert_eq!(imp["result"]["preset_id"], "zhipu");

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_real_multi_agent_governance_and_hermes_studio() {
    let (_dir, host) = create_temp_host();
    unsafe {
        // 1. 5-hop LoopBudget: hops 1..=5 succeed, 6th hop fails closed with loop_budget_exceeded
        for hop in 1..=5 {
            let r = dispatch(
                host,
                &json!({
                    "method": "multi_agent.loopBudgetRecordHop",
                    "params": {"chat_id": 900, "agent_id": format!("bot-{hop}")}
                }),
            );
            assert_eq!(r["ok"], true);
            assert_eq!(r["result"]["current_hop"], hop);
        }
        let hop6 = dispatch(
            host,
            &json!({
                "method": "multi_agent.loopBudgetRecordHop",
                "params": {"chat_id": 900, "agent_id": "bot-6"}
            }),
        );
        assert_eq!(hop6["ok"], false);
        assert_eq!(hop6["error"]["code"], "loop_budget_exceeded");

        // 2. FloorControlQueue: bot holds floor -> human UserInterruption preempts immediately
        let f1 = dispatch(
            host,
            &json!({
                "method": "multi_agent.floorRequest",
                "params": {"chat_id": 900, "agent_id": "bot-alpha", "priority": "general_contribution"}
            }),
        );
        assert_eq!(f1["result"]["granted"], true);
        assert_eq!(f1["result"]["current_speaker"], "bot-alpha");

        let preempt = dispatch(
            host,
            &json!({"method": "multi_agent.floorPreemptUser", "params": {"chat_id": 900}}),
        );
        assert_eq!(preempt["ok"], true);
        assert_eq!(preempt["result"]["preempted_agent"], "bot-alpha");
        assert!(
            preempt["result"]["preempted_nonce"]
                .as_str()
                .unwrap_or("")
                .starts_with("trn_")
        );
        assert!(preempt["result"]["current_speaker"].is_null());

        // 3. Revision-fenced SharedGroupWorkspace optimistic lock
        let ws1 = dispatch(
            host,
            &json!({
                "method": "multi_agent.workspaceSet",
                "params": {"chat_id": 900, "key": "plan", "value": {"v": 1}, "updated_by": "bot-a"}
            }),
        );
        assert_eq!(ws1["ok"], true);
        let rev1 = ws1["result"]["snapshot"]["revision"].as_u64().unwrap();
        let ws_conflict = dispatch(
            host,
            &json!({
                "method": "multi_agent.workspaceSet",
                "params": {
                    "chat_id": 900,
                    "key": "plan",
                    "value": {"v": 2},
                    "updated_by": "bot-b",
                    "expected_revision": rev1 + 99
                }
            }),
        );
        assert_eq!(ws_conflict["ok"], false);
        assert_eq!(ws_conflict["error"]["code"], "conflict");

        // 4. Hermes/Fabushi Studio: >=49 slash commands, 29 toolsets, Kanban, Cron, Value-Bound Approval
        let cmds = dispatch(host, &json!({"method": "fabushi.listCommands"}));
        assert_eq!(cmds["ok"], true);
        assert!(
            cmds["result"]["count"].as_u64().unwrap() >= 49,
            "Expected >=49 slash commands, got {}",
            cmds["result"]["count"]
        );

        let ts = dispatch(host, &json!({"method": "fabushi.listToolsets"}));
        assert_eq!(ts["ok"], true);
        assert_eq!(ts["result"]["count"], 29);

        let board = dispatch(
            host,
            &json!({"method": "fabushi.kanbanCreateBoard", "params": {"name": "Sprint 1"}}),
        );
        assert_eq!(board["ok"], true);
        let board_id = board["result"]["id"].as_str().unwrap();

        let card = dispatch(
            host,
            &json!({
                "method": "fabushi.kanbanCreateCard",
                "params": {"board_id": board_id, "title": "Ship v1.6.8"}
            }),
        );
        assert_eq!(card["ok"], true);
        let card_id = card["result"]["id"].as_str().unwrap();
        let moved = dispatch(
            host,
            &json!({
                "method": "fabushi.kanbanMoveCard",
                "params": {"card_id": card_id, "target_column": "In Progress"}
            }),
        );
        assert_eq!(moved["ok"], true);

        let cron = dispatch(
            host,
            &json!({"method": "fabushi.cronPreview", "params": {"schedule": "0 9 * * 1-5"}}),
        );
        assert_eq!(cron["ok"], true);
        assert_eq!(cron["result"]["valid"], true);

        // Value-Bound Approval: single-use consumption rejects replay!
        let issued = dispatch(
            host,
            &json!({
                "method": "fabushi.approvalIssueValueBound",
                "params": {"action": "deploy", "target": "prod", "content_sha256": "abc123"}
            }),
        );
        assert_eq!(issued["ok"], true);
        let aid = issued["result"]["approval_id"].as_str().unwrap();
        let consumed = dispatch(
            host,
            &json!({
                "method": "fabushi.approvalConsumeValueBound",
                "params": {"approval_id": aid, "action": "deploy", "target": "prod", "content_sha256": "abc123"}
            }),
        );
        assert_eq!(consumed["ok"], true);
        let replay = dispatch(
            host,
            &json!({
                "method": "fabushi.approvalConsumeValueBound",
                "params": {"approval_id": aid, "action": "deploy", "target": "prod", "content_sha256": "abc123"}
            }),
        );
        assert_eq!(replay["ok"], false);
        assert_eq!(replay["error"]["code"], "approval_rejected");

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_real_webrtc_talk_remote_hub_miniapp_and_action_bus() {
    let (_dir, host) = create_temp_host();
    unsafe {
        // 1. WebRtcCallEngine + 4-emoji fingerprint + AudioLevelMeter + SfuBridge
        let start = dispatch(
            host,
            &json!({
                "method": "calls.engineStartOutgoing",
                "params": {"call_id": 1001, "local_user_id": 1, "remote_user_id": 2, "is_video": false}
            }),
        );
        assert_eq!(start["ok"], true);
        assert!(start["result"]["local_sdp"].as_str().unwrap().contains("v=0"));

        let fp = dispatch(
            host,
            &json!({
                "method": "calls.engineEmojiFingerprint",
                "params": {"sha256_hex": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}
            }),
        );
        assert_eq!(fp["ok"], true);
        assert!(fp["result"]["emojis"].as_str().unwrap().chars().count() >= 4);

        let meter = dispatch(
            host,
            &json!({"method": "calls.enginePushAudioLevel", "params": {"pcm_i16": vec![12000i16; 320]}}),
        );
        assert_eq!(meter["ok"], true);
        assert!(meter["result"]["level"].as_f64().unwrap() > 0.1);

        let sfu = dispatch(
            host,
            &json!({"method": "calls.sfuJoinRoom", "params": {"chat_id": 50, "user_id": 1}}),
        );
        assert_eq!(sfu["result"]["subscribers"][0], 1);

        // 2. TalkCoordinator + <60ms VAD barge-in
        assert_eq!(dispatch(host, &json!({"method": "talk.createSession"}))["ok"], true);
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "talk.transition", "params": {"state": "speaking"}})
            )["ok"],
            true
        );
        assert_eq!(
            dispatch(
                host,
                &json!({"method": "talk.enqueueAudio", "params": {"chunk_b64": BASE64_STANDARD.encode([1u8; 64])}})
            )["result"]["buffered_chunks"],
            1
        );
        let loud = vec![15000i16; 320];
        let _ = dispatch(host, &json!({"method": "talk.processVadPcm", "params": {"pcm_i16": loud}}));
        let _ = dispatch(host, &json!({"method": "talk.processVadPcm", "params": {"pcm_i16": loud}}));
        let third = dispatch(host, &json!({"method": "talk.processVadPcm", "params": {"pcm_i16": loud}}));
        assert_eq!(third["result"]["barge_in_triggered"], true);
        assert_eq!(third["result"]["flushed_chunks"], 1);

        // 3. RemoteControlHub exclusive lease + <10ms emergency takeover
        let ai_lease = dispatch(
            host,
            &json!({
                "method": "remote.hubAcquireLease",
                "params": {
                    "session_id": "ai-sess-1",
                    "controller_kind": "ai_mcp",
                    "controller_name": "Agent",
                    "controller_platform": "ios",
                    "controller_id": "ctrl-agent-1",
                    "target_node_id": "node-mac-1",
                    "target_device_id": "dev-mac-1",
                    "mode": "desktop",
                    "started_at_ms": 0,
                    "last_activity_ms": 0,
                    "relay_mode": "direct_udp"
                }
            }),
        );
        assert_eq!(ai_lease["ok"], true, "ai_lease failed: {ai_lease}");

        let takeover = dispatch(host, &json!({"method": "remote.hubEmergencyTakeover"}));
        assert_eq!(takeover["ok"], true);
        assert_eq!(takeover["result"]["disconnected_sessions"], 1);
        assert!(
            takeover["result"]["elapsed_us"].as_u64().unwrap() < 10_000,
            "Emergency takeover must complete in <10ms"
        );

        // 4. MiniApp 全球法布施 pure-Rust state/storage/assets + WebMCP
        let boot = dispatch(host, &json!({"method": "miniapp.dharmaBootstrap", "params": {"user_id": 7}}));
        assert_eq!(boot["ok"], true);
        let prac = dispatch(
            host,
            &json!({
                "method": "miniapp.dharmaRecordPractice",
                "params": {"user_id": 7, "sutra": "心经", "duration_minutes": 15, "chant_count": 3}
            }),
        );
        assert_eq!(prac["ok"], true);
        let wasm = dispatch(
            host,
            &json!({"method": "miniapp.dharmaAssetBytes", "params": {"name": "fabushi_miniapp.wasm"}}),
        );
        assert_eq!(wasm["ok"], true);
        assert_eq!(wasm["result"]["has_wasm_magic"], true);

        assert_eq!(
            dispatch(
                host,
                &json!({"method": "webmcp.setControlMode", "params": {"app_key": "bot/app", "mode": "ai_controlling"}})
            )["ok"],
            true
        );

        // 5. Action Bus (>=37 actions + all 4 exporters with non-empty verified payloads)
        let actions = dispatch(host, &json!({"method": "actions.list"}));
        assert_eq!(actions["ok"], true);
        assert!(actions["result"]["count"].as_u64().unwrap() >= 37);

        let mcp_exp = dispatch(host, &json!({"method": "actions.exportMcp"}));
        assert_eq!(mcp_exp["ok"], true);
        assert!(
            mcp_exp["result"]
                .get("tools")
                .and_then(|v| v.as_array())
                .map_or(false, |a| a.len() >= 37)
                || mcp_exp["result"].as_array().map_or(false, |a| a.len() >= 37),
            "Unexpected exportMcp payload: {mcp_exp}"
        );

        let openapi_exp = dispatch(host, &json!({"method": "actions.exportOpenApi"}));
        assert_eq!(openapi_exp["ok"], true);
        assert!(openapi_exp["result"]["openapi"].as_str().is_some());
        assert!(
            openapi_exp["result"]["paths"]
                .as_object()
                .map_or(false, |m| m.len() >= 37),
            "Expected >=37 OpenAPI paths: {openapi_exp}"
        );

        let hermes_exp = dispatch(host, &json!({"method": "actions.exportHermes"}));
        assert_eq!(hermes_exp["ok"], true);
        assert!(hermes_exp["result"]["count"].as_u64().unwrap() >= 37);
        assert!(hermes_exp["result"]["openai_tools"].as_array().unwrap().len() >= 37);
        assert!(hermes_exp["result"]["anthropic_tools"].as_array().unwrap().len() >= 37);

        let cli_exp = dispatch(host, &json!({"method": "actions.exportCli"}));
        assert_eq!(cli_exp["ok"], true);
        assert!(cli_exp["result"].to_string().len() > 500);

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_fail_closed_ios_capability_policy_and_honest_blockers() {
    let (_dir, host) = create_temp_host();
    unsafe {
        // 1. Default iOS Hermes toolsets MUST use Safe preset and disable desktop-only toolsets (B-06)
        let ts_list = dispatch(host, &json!({"method": "fabushi.listToolsets"}));
        assert_eq!(ts_list["ok"], true);
        assert_eq!(ts_list["result"]["summary"]["active_preset"], "safe");
        let toolsets_arr = ts_list["result"]["summary"]["toolsets"]
            .as_array()
            .expect("summary.toolsets must be an array");
        assert_eq!(toolsets_arr.len(), 29);
        let core_entry = toolsets_arr
            .iter()
            .find(|t| t["id"] == "core")
            .expect("core toolset must exist");
        assert_eq!(core_entry["enabled"], true);
        for forbidden_ts in ["terminal", "code_execution", "desktop_ui", "browser"] {
            let entry = toolsets_arr
                .iter()
                .find(|t| t["id"] == forbidden_ts)
                .unwrap_or_else(|| panic!("missing toolset entry '{forbidden_ts}'"));
            assert_eq!(
                entry["enabled"], false,
                "Default iOS Hermes toolsets must NOT enable '{forbidden_ts}': {entry}"
            );
        }

        // 2. fabushi.updateToolsets MUST reject presets & alias/whitespace bypasses on iOS (B-06)
        for bypass in [
            "full",
            "all",
            "hermes-cli",
            "cli",
            "hermes-acp",
            "acp",
            "code-execution",
            "codeexecution",
            " terminal ",
            "shell",
            "desktop_ui",
            "browser",
        ] {
            let res = dispatch(
                host,
                &json!({
                    "method": "fabushi.updateToolsets",
                    "params": {"enable": [bypass]}
                }),
            );
            assert_eq!(
                res["ok"], false,
                "fabushi.updateToolsets({bypass:?}) must fail closed on iOS: {res}"
            );
            assert_eq!(
                res["error"]["code"], "capability_unavailable",
                "fabushi.updateToolsets({bypass:?}) must return capability_unavailable: {res}"
            );
        }

        // 3. Every desktop host-control / subprocess / dotfile scan RPC MUST fail closed on iOS!
        let forbidden_on_ios = [
            "remote.localPower",
            "remote.localAccessibility",
            "remote.localInput",
            "remote.localCapture",
            "remote.localPtySpawn",
            "remote.localPrivacyScreen",
            "remote.localAudioMute",
            "remote.localClipboard",
            "coding.probeLocal",
            "coding.startLocalTurn",
            "coding.localControl",
            "computers.registerLocalHost",
            "computers.pollLocalCommands",
            "computers.executeLocalOp",
            "magpie.scanLocalSubscriptions",
            "fabushi.executeLocalTerminalTool",
        ];
        for method in forbidden_on_ios {
            let res = dispatch(host, &json!({"id": "ios-cap", "method": method, "params": {}}));
            assert_eq!(
                res["ok"], false,
                "{method} must fail closed under iOS sandbox policy: {res}"
            );
            assert_eq!(
                res["error"]["code"], "capability_unavailable",
                "{method} must return capability_unavailable, got {res}"
            );
        }

        // 4. mcp.registerServer(transport="stdio") fails closed on iOS
        let mcp_stdio = dispatch(
            host,
            &json!({
                "method": "mcp.registerServer",
                "params": {"name": "local-stdio", "transport": "stdio", "command": "/bin/sh"}
            }),
        );
        assert_eq!(mcp_stdio["ok"], false);
        assert_eq!(mcp_stdio["error"]["code"], "capability_unavailable");

        // 5. tailnet.decideApproval(approved=true) fails closed on iOS (B-01)
        let tailnet_approve = dispatch(
            host,
            &json!({
                "method": "tailnet.decideApproval",
                "params": {"command_id": "cmd-inbound-1", "approved": true}
            }),
        );
        assert_eq!(tailnet_approve["ok"], false);
        assert_eq!(tailnet_approve["error"]["code"], "capability_unavailable");

        // 6. Desktop host-control actions via actions.execute fail closed on iOS (NB-08)
        for act_name in ["device_power_action", "terminal_exec", "remote_capture_screen"] {
            let act_res = dispatch(
                host,
                &json!({
                    "method": "actions.execute",
                    "params": {"name": act_name, "input": {"node_id": "local", "action": "shutdown", "session_name": "s", "command": "id"}, "approved": true}
                }),
            );
            assert_eq!(
                act_res["ok"], false,
                "actions.execute({act_name}) must fail closed on iOS: {act_res}"
            );
            assert_eq!(act_res["error"]["code"], "capability_unavailable");
        }

        // 7. Honest blocker BLK-01: APNs push token persists locally and reports upstream_stub_blocked
        let push = dispatch(
            host,
            &json!({
                "method": "push.registerToken",
                "params": {"token": "aabbccddeeff00112233445566778899", "environment": "production"}
            }),
        );
        assert_eq!(push["ok"], true);
        assert_eq!(push["result"]["persisted"], true);
        assert_eq!(push["result"]["server_apns_dispatch"], "upstream_stub_blocked");

        // 8. Honest blocker BLK-03: TON signing fails closed with upstream_not_implemented
        let ton = dispatch(host, &json!({"method": "wallet.tonSignBroadcast"}));
        assert_eq!(ton["ok"], false);
        assert_eq!(ton["error"]["code"], "upstream_not_implemented");

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_transition_task_reentrant_callback_no_deadlock() {
    let (_dir, host) = create_temp_host();

    struct ReentrantTaskCtx {
        host: usize,
        list_calls: AtomicUsize,
    }
    let ctx = Arc::new(ReentrantTaskCtx {
        host: host as usize,
        list_calls: AtomicUsize::new(0),
    });

    unsafe extern "C" fn on_task_event(
        event_type: *const c_char,
        _payload: *const c_char,
        user_ctx: *mut c_void,
    ) {
        if !event_type.is_null() && !user_ctx.is_null() {
            let ev = unsafe { CStr::from_ptr(event_type) }.to_string_lossy();
            if ev == "taskStateUpdated" {
                let st = unsafe { &*(user_ctx as *const ReentrantTaskCtx) };
                // Re-entrantly locks host.task_registry via fabushi.listTasks — MUST NOT DEADLOCK (B-03)!
                let list = unsafe {
                    dispatch(st.host as *mut c_void, &json!({"method": "fabushi.listTasks"}))
                };
                assert_eq!(list["ok"], true);
                st.list_calls.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    unsafe {
        let raw_ctx = Arc::into_raw(Arc::clone(&ctx)) as *mut c_void;
        telegram_bridge_register_event_callback(host, Some(on_task_event), raw_ctx);

        let created = dispatch(
            host,
            &json!({"method": "fabushi.createTask", "params": {"task_id": "t-reent-1"}}),
        );
        assert_eq!(created["ok"], true);

        let transitioned = dispatch(
            host,
            &json!({
                "method": "fabushi.transitionTask",
                "params": {"task_id": "t-reent-1", "state": "running"}
            }),
        );
        assert_eq!(transitioned["ok"], true);
        assert!(ctx.list_calls.load(Ordering::SeqCst) >= 2);

        fabushi_host_destroy(host);
        drop(Arc::from_raw(raw_ctx as *const ReentrantTaskCtx));
    }
}
