use std::ffi::{c_void, CStr, CString};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use mahayana_app_host::{
    fabushi_host_create, fabushi_host_destroy, fabushi_host_dispatch, fabushi_host_free_string,
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

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_real_split_key_v2_argon2id_x25519_vault_and_rfc6238_totp() {
    let (_dir, host) = create_temp_host();
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
                "params": {"board_id": board_id, "title": "Ship v1.6.7"}
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

        // 5. Action Bus (>=37 actions + all 4 exporters)
        let actions = dispatch(host, &json!({"method": "actions.list"}));
        assert_eq!(actions["ok"], true);
        assert!(actions["result"]["count"].as_u64().unwrap() >= 37);
        assert_eq!(dispatch(host, &json!({"method": "actions.exportMcp"}))["ok"], true);
        assert_eq!(dispatch(host, &json!({"method": "actions.exportOpenApi"}))["ok"], true);
        assert_eq!(dispatch(host, &json!({"method": "actions.exportHermes"}))["ok"], true);
        assert_eq!(dispatch(host, &json!({"method": "actions.exportCli"}))["ok"], true);

        fabushi_host_destroy(host);
    }
}

#[test]
fn test_fail_closed_ios_capability_policy_and_honest_blockers() {
    let (_dir, host) = create_temp_host();
    unsafe {
        // Every desktop host-control / subprocess / dotfile scan RPC MUST fail closed on iOS!
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

        // Honest blocker BLK-01: APNs push token persists locally and reports upstream_stub_blocked
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

        // Honest blocker BLK-03: TON signing fails closed with upstream_not_implemented
        let ton = dispatch(host, &json!({"method": "wallet.tonSignBroadcast"}));
        assert_eq!(ton["ok"], false);
        assert_eq!(ton["error"]["code"], "upstream_not_implemented");

        fabushi_host_destroy(host);
    }
}
