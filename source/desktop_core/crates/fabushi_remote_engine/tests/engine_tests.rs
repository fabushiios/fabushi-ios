use fabushi_remote_engine::accessibility::AccessibilityEngine;
use fabushi_remote_engine::audio::AudioPipeline;
use fabushi_remote_engine::capture::{CaptureFormat, ScreenCaptureEngine};
use fabushi_remote_engine::clipboard::ClipboardSyncEngine;
use fabushi_remote_engine::codec::{AbrController, NetworkMetricsSample, VideoCodecEngine};
use fabushi_remote_engine::control_lease::{RemoteControlHub, USER_OVERRIDE_EPOCH};
use fabushi_remote_engine::display_privacy::DisplayPrivacyManager;
use fabushi_remote_engine::file_transfer::{
    enumerate_directory_files, sha256_file_hex, FileTransferManager, DEFAULT_CHUNK_SIZE,
};
use fabushi_remote_engine::input::NativeInputEngine;
use fabushi_remote_engine::ipc::{LocalIpcClient, LocalIpcHandler, LocalIpcServer};
use fabushi_remote_engine::power::{build_wol_magic_packet, PowerManager};
use fabushi_remote_engine::terminal::client_tty::{
    run_local_tty_attach, DetachEscapeParser, EscapeAction, TtyAttachOptions,
};
use fabushi_remote_engine::terminal::{
    detect_osc52_sequences, handle_terminal_ipc_method, normalize_pty_output, Osc52StreamScanner,
    PtyMuxServer, FABUSHI_IN_MUX_ENV,
};
use fabushi_remote_engine::RemoteEngineError;
use gpui_telegram_proto::remote_control_contracts::{
    ClipboardPayloadDto, ColorSubsampling, FileConflictPolicy, FileTransferStatus, LocalIpcRequest,
    LocalIpcResponse, PowerActionKind, RemoteControlMode, RemoteControllerKind, RemoteSessionDto,
    RemoteStreamSettingsDto, StreamCodec, StreamQuality,
};
use gpui_telegram_proto::tailnet_contracts::TailnetRelayMode;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct TestIpcHandler {
    mux: Arc<PtyMuxServer>,
}

impl LocalIpcHandler for TestIpcHandler {
    fn handle(
        &self,
        req: LocalIpcRequest,
    ) -> fabushi_remote_engine::ipc::BoxFuture<'_, LocalIpcResponse> {
        Box::pin(async move {
            if let Some(resp) = handle_terminal_ipc_method(&self.mux, &req).await {
                return resp;
            }
            match req.method.as_str() {
                "status" => LocalIpcResponse::ok(req.id, serde_json::json!({ "ok": true })),
                "echo" => LocalIpcResponse::ok(req.id, req.params),
                other => LocalIpcResponse::err(req.id, "not_found", other),
            }
        })
    }
}

fn test_ipc_endpoint(tmp: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    #[cfg(unix)]
    {
        tmp.path().join(format!("{name}.sock"))
    }
    #[cfg(windows)]
    {
        let _ = tmp;
        std::path::PathBuf::from(format!(
            r"\\.\pipe\fabushi-test-{}-{name}",
            uuid::Uuid::new_v4().simple()
        ))
    }
}

#[tokio::test]
async fn test_ipc_roundtrip_permissions_and_uid_check() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let sock = test_ipc_endpoint(&tmp, "ipc");
    let mux = Arc::new(PtyMuxServer::new());
    let server = LocalIpcServer::bind(sock.clone(), Arc::new(TestIpcHandler { mux }))
        .await
        .expect("bind ipc");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::io::AsRawFd;
        let mode = std::fs::metadata(&sock)
            .expect("sock metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "UDS socket must have strict 0600 permissions");

        let stream = std::os::unix::net::UnixStream::connect(&sock).expect("connect raw unix");
        let my_uid = unsafe { libc::geteuid() };
        assert_eq!(
            fabushi_remote_engine::ipc::verify_unix_peer_uid(stream.as_raw_fd(), my_uid)
                .expect("matching uid passes"),
            my_uid
        );
        let err = fabushi_remote_engine::ipc::verify_unix_peer_uid(
            stream.as_raw_fd(),
            my_uid.wrapping_add(1),
        )
        .unwrap_err();
        assert!(matches!(err, RemoteEngineError::PermissionDenied(_)));
    }

    let client = LocalIpcClient::new(sock.clone());
    let resp = client
        .call("echo", serde_json::json!({ "hello": "fabushi" }))
        .await
        .expect("ipc echo");
    assert!(resp.success);
    assert_eq!(resp.data.unwrap()["hello"], "fabushi");

    server.shutdown();
    server.wait_stopped().await;
}

#[tokio::test]
async fn test_control_hub_kill_switch_preempts_under_50ms() {
    let hub = RemoteControlHub::new();
    let before_global = USER_OVERRIDE_EPOCH.load(Ordering::SeqCst);
    let session = RemoteSessionDto {
        session_id: "ai-sess-1".into(),
        controller_kind: RemoteControllerKind::AiMcp,
        controller_name: "Antigravity".into(),
        controller_platform: "macos".into(),
        controller_id: "oauth-client-1".into(),
        target_node_id: "local:1:mac".into(),
        target_device_id: "mac-1".into(),
        mode: RemoteControlMode::AiControl,
        started_at_ms: 0,
        last_activity_ms: 0,
        active_tool: Some("capture_screen".into()),
        relay_mode: TailnetRelayMode::DirectUdp,
        rtt_ms: Some(5),
        fps: None,
    };
    hub.acquire_exclusive_lease(session, true)
        .expect("acquire AI lease");
    assert_eq!(hub.active_count(), 1);

    let mut token = hub.preemption_token();
    assert!(!token.is_cancelled());

    let task = tokio::spawn(async move {
        let start = Instant::now();
        token.cancelled().await;
        (start.elapsed(), token.check())
    });

    tokio::time::sleep(Duration::from_millis(5)).await;
    let t0 = Instant::now();
    let disconnected = hub.disconnect_all();
    assert_eq!(disconnected, 1);
    assert_eq!(hub.active_count(), 0);
    assert!(USER_OVERRIDE_EPOCH.load(Ordering::SeqCst) > before_global);

    let (elapsed, check_res) = tokio::time::timeout(Duration::from_millis(50), task)
        .await
        .expect("kill switch must preempt within 50ms")
        .expect("join");
    assert!(t0.elapsed() < Duration::from_millis(50));
    let _ = elapsed;
    assert!(matches!(check_res, Err(RemoteEngineError::Preempted(_))));
}

#[tokio::test]
async fn test_pty_mux_lifecycle_persistence_exec_env_and_nested_loop_prevention() {
    let mux = Arc::new(PtyMuxServer::new());
    let created = mux
        .new_session(Some("build"), None, 100, 30, None)
        .expect("create pty session");
    assert_eq!(created.name, "build");
    assert!(mux.has_session("build"));
    assert_eq!(mux.list_sessions().len(), 1);

    let renamed = mux
        .rename_session("build", "dev-shell")
        .expect("rename session");
    assert_eq!(renamed.name, "dev-shell");
    assert!(!mux.has_session("build"));
    assert!(mux.has_session("dev-shell"));

    // Attach and detach; destroy_unattached = false keeps PTY alive!
    let att = mux
        .attach_session("dev-shell", "client-local", false, true)
        .expect("attach");
    assert_eq!(att.session.attached_clients, 1);
    mux.resize_client("dev-shell", "client-remote", 132, 42, true, false)
        .expect("remote resize");
    mux.resize_client("dev-shell", "client-local", 80, 24, false, true)
        .expect("local mirror ignored resize");
    assert_eq!(mux.get_session("dev-shell").unwrap().cols, 132);
    assert_eq!(mux.get_session("dev-shell").unwrap().rows, 42);

    mux.detach_client("dev-shell", "client-local").expect("detach");
    mux.detach_client("dev-shell", "client-remote").expect("detach");
    assert!(mux.get_session("dev-shell").unwrap().alive);

    // Execute commands in the unattached persistent PTY
    let res1 = mux
        .exec_command("dev-shell", "echo fabushi_pty_ok", 10_000)
        .await
        .expect("exec echo");
    assert!(!res1.timed_out, "exec echo timed out: {:?}", res1.output);
    assert_eq!(res1.exit_code, 0);
    assert!(
        res1.output.contains("fabushi_pty_ok"),
        "unexpected output: {:?}",
        res1.output
    );

    #[cfg(unix)]
    {
        let res_fail = mux
            .exec_command("dev-shell", "false", 10_000)
            .await
            .expect("exec false");
        assert!(!res_fail.timed_out);
        assert_eq!(res_fail.exit_code, 1);

        let res_env = mux
            .exec_command(
                "dev-shell",
                "echo MUX=$FABUSHI_IN_MUX TMUX_SET=${TMUX:-none}",
                10_000,
            )
            .await
            .expect("exec env");
        assert_eq!(res_env.exit_code, 0);
        assert!(res_env.output.contains("MUX=1"));
        assert!(res_env.output.contains("TMUX_SET=none"));
    }

    #[cfg(windows)]
    {
        let res_fail = mux
            .exec_command("dev-shell", "cmd /c exit 7", 10_000)
            .await
            .expect("exec non-zero");
        assert!(
            !res_fail.timed_out,
            "cmd non-zero exec timed out: {:?}",
            res_fail.output
        );
        assert_eq!(res_fail.exit_code, 7);

        // Subsequent command must observe ERRORLEVEL reset back to 0 and FABUSHI_IN_MUX=1.
        let res_env = mux
            .exec_command("dev-shell", "echo MUX=%FABUSHI_IN_MUX%", 10_000)
            .await
            .expect("exec env after error");
        assert!(
            !res_env.timed_out,
            "cmd env exec timed out: {:?}",
            res_env.output
        );
        assert_eq!(res_env.exit_code, 0);
        assert!(
            res_env.output.contains("MUX=1"),
            "unexpected env output: {:?}",
            res_env.output
        );
    }

    // Nested FABUSHI_IN_MUX=1 protection
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let sock = test_ipc_endpoint(&tmp, "mux");
    let server = LocalIpcServer::bind(sock.clone(), Arc::new(TestIpcHandler { mux: Arc::clone(&mux) }))
        .await
        .expect("bind");
    let client = LocalIpcClient::new(sock);

    let prev = std::env::var_os(FABUSHI_IN_MUX_ENV);
    unsafe {
        std::env::set_var(FABUSHI_IN_MUX_ENV, "1");
    }
    let nested_err = run_local_tty_attach(
        &client,
        TtyAttachOptions {
            session_name: Some("dev-shell".into()),
            shell: None,
            create_if_missing: false,
            ignore_size: true,
        },
    )
    .await
    .unwrap_err();
    match prev {
        Some(v) => unsafe { std::env::set_var(FABUSHI_IN_MUX_ENV, v) },
        None => unsafe { std::env::remove_var(FABUSHI_IN_MUX_ENV) },
    }
    assert!(matches!(nested_err, RemoteEngineError::NestedMuxBlocked(_)));

    assert!(mux.kill_session("dev-shell").expect("kill"));
    assert!(!mux.has_session("dev-shell"));
    server.shutdown();
    server.wait_stopped().await;
}

#[test]
fn test_osc52_detection_and_tty_detach_escape() {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode("secret-clip-123");
    let seq_bel = format!("hello\x1b]52;c;{b64}\x07world");
    let detected = detect_osc52_sequences(seq_bel.as_bytes());
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0].text.as_deref(), Some("secret-clip-123"));

    // Split chunk across ST terminator
    let mut scanner = Osc52StreamScanner::default();
    let part1 = format!("\x1b]52;c;{}", &b64[..4]);
    let part2 = format!("{}\x1b\\", &b64[4..]);
    assert!(scanner.feed(part1.as_bytes()).is_empty());
    let out = scanner.feed(part2.as_bytes());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].text.as_deref(), Some("secret-clip-123"));

    let mut parser = DetachEscapeParser::default();
    assert_eq!(parser.process(b"ls\n"), EscapeAction::Forward(b"ls\n".to_vec()));
    assert_eq!(parser.process(&[0x1d, b'x']), EscapeAction::Forward(vec![0x1d, b'x']));
    assert_eq!(parser.process(&[0x1d]), EscapeAction::Forward(vec![]));
    assert_eq!(parser.process(b"d"), EscapeAction::Detach);
}

#[test]
fn test_codec_profiles_and_abr_downgrade_recovery() {
    let engine = VideoCodecEngine::new();
    assert!(!engine.profiles().is_empty());
    assert_eq!(
        engine.negotiate_codec(
            StreamCodec::H265,
            Some("amd"),
            "windows",
            true,
            ColorSubsampling::TrueColor444
        ),
        StreamCodec::H264
    );
    assert_eq!(
        engine.negotiate_codec(
            StreamCodec::Auto,
            Some("apple"),
            "macos",
            true,
            ColorSubsampling::TrueColor444
        ),
        StreamCodec::H265
    );

    let mut abr = AbrController::new(RemoteStreamSettingsDto {
        target_fps: 120,
        quality: StreamQuality::Ultra4K,
        color_mode: ColorSubsampling::TrueColor444,
        ..Default::default()
    });
    let degraded = abr.observe(NetworkMetricsSample {
        rtt_ms: 210,
        packet_loss_pct: 4.5,
        decode_time_ms: 40,
        queue_depth: 8,
    });
    assert!(degraded.downgraded);
    assert_eq!(degraded.target_fps, 30);
    assert_eq!(degraded.quality, StreamQuality::Smooth);
    assert_eq!(degraded.color_mode, ColorSubsampling::Yuv420);

    for _ in 0..3 {
        abr.observe(NetworkMetricsSample {
            rtt_ms: 12,
            packet_loss_pct: 0.0,
            decode_time_ms: 4,
            queue_depth: 0,
        });
    }
    let recovered = abr.current_decision();
    assert!(!recovered.downgraded);
    assert_eq!(recovered.target_fps, 120);
}

#[test]
fn test_file_transfer_chunking_resume_sha256_mismatch_and_conflicts() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let src_dir = tmp.path().join("src");
    let dst_dir = tmp.path().join("dst");
    std::fs::create_dir_all(src_dir.join("nested")).unwrap();
    std::fs::create_dir_all(&dst_dir).unwrap();

    let src_file = src_dir.join("nested").join("payload.bin");
    let payload: Vec<u8> = (0..(DEFAULT_CHUNK_SIZE * 2 + 12345))
        .map(|i| (i % 251) as u8)
        .collect();
    std::fs::write(&src_file, &payload).unwrap();

    let dir_entries = enumerate_directory_files(&src_dir).expect("enumerate");
    assert_eq!(dir_entries.len(), 1);
    assert_eq!(dir_entries[0].0, "nested/payload.bin");

    let mgr1 = FileTransferManager::new(state_dir.clone()).expect("mgr1");
    let up = mgr1
        .create_upload(&src_file, "/remote/payload.bin", FileConflictPolicy::Overwrite)
        .expect("create upload");
    let dst_file = dst_dir.join("payload.bin");
    let dl = mgr1
        .create_download(
            "/remote/payload.bin",
            &dst_file,
            up.total_bytes,
            up.sha256.clone(),
            FileConflictPolicy::Overwrite,
        )
        .expect("create download");

    let chunk0 = mgr1.read_next_chunk(&up.transfer_id).unwrap().unwrap();
    assert_eq!(chunk0.offset, 0);
    assert_eq!(chunk0.data.len(), DEFAULT_CHUNK_SIZE);
    let st0 = mgr1.write_chunk(&chunk0_for(&dl.transfer_id, &chunk0)).unwrap();
    assert_eq!(st0.status, FileTransferStatus::Running);

    // Corrupted chunk SHA-256 must be rejected
    let bad = chunk0_for(&dl.transfer_id, &mgr1.read_next_chunk(&up.transfer_id).unwrap().unwrap());
    let mut bad_clone = bad.clone();
    bad_clone.data[0] ^= 0xff;
    let err = mgr1.write_chunk(&bad_clone).unwrap_err();
    assert!(matches!(err, RemoteEngineError::ChecksumMismatch { .. }));

    // Simulate restart by creating a new FileTransferManager on the same state_dir
    let mgr2 = FileTransferManager::new(state_dir.clone()).expect("mgr2");
    assert_eq!(
        mgr2.missing_offsets(&dl.transfer_id).unwrap(),
        vec![DEFAULT_CHUNK_SIZE as u64, (DEFAULT_CHUNK_SIZE * 2) as u64]
    );
    mgr2.write_chunk(&bad).unwrap();
    let chunk2 = mgr2.read_next_chunk(&up.transfer_id).unwrap().unwrap();
    let final_dto = mgr2.write_chunk(&chunk0_for(&dl.transfer_id, &chunk2)).unwrap();
    assert_eq!(final_dto.status, FileTransferStatus::Completed);
    assert_eq!(
        sha256_file_hex(&dst_file).unwrap(),
        sha256_file_hex(&src_file).unwrap()
    );

    // Conflict policy Rename
    let renamed_path =
        FileTransferManager::resolve_conflict_path(&dst_file, FileConflictPolicy::Rename)
            .unwrap()
            .unwrap();
    assert!(renamed_path.ends_with("payload (1).bin"));
    assert!(
        FileTransferManager::resolve_conflict_path(&dst_file, FileConflictPolicy::Skip)
            .unwrap()
            .is_none()
    );

    // Source modification invalidates upload resume
    let up2 = mgr2
        .create_upload(&src_file, "/remote/payload2.bin", FileConflictPolicy::Overwrite)
        .unwrap();
    std::fs::write(&src_file, b"shortened").unwrap();
    assert!(!mgr2.check_source_unchanged(&up2.transfer_id).unwrap());
    assert!(mgr2.read_chunk_at(&up2.transfer_id, 0).is_err());
}

fn chunk0_for(
    transfer_id: &str,
    src: &fabushi_remote_engine::file_transfer::FileChunkFrame,
) -> fabushi_remote_engine::file_transfer::FileChunkFrame {
    let mut c = src.clone();
    c.transfer_id = transfer_id.to_string();
    c
}

#[test]
fn test_clipboard_loop_prevention_and_wol_and_headless_subsystems() {
    fabushi_remote_engine::set_headless_test_mode(true);
    let clip = ClipboardSyncEngine::headless("node-a");
    let same_origin = ClipboardPayloadDto {
        mime_type: "text/plain".into(),
        text: Some("loop".into()),
        image_b64: None,
        origin_id: "node-a".into(),
        seq: 1,
    };
    assert!(!clip.apply_remote(&same_origin).unwrap());

    let from_b_1 = ClipboardPayloadDto {
        mime_type: "text/plain".into(),
        text: Some("hello from b".into()),
        image_b64: None,
        origin_id: "node-b".into(),
        seq: 1,
    };
    assert!(clip.apply_remote(&from_b_1).unwrap());
    assert_eq!(clip.get_local().unwrap().text.as_deref(), Some("hello from b"));
    // Replay seq 1 is suppressed
    assert!(!clip.apply_remote(&from_b_1).unwrap());
    // Same digest even with bumped seq is suppressed
    let mut dup = from_b_1.clone();
    dup.seq = 2;
    assert!(!clip.apply_remote(&dup).unwrap());

    // Wake-on-LAN magic packet
    let pkt = build_wol_magic_packet("AA:BB:CC:DD:EE:FF").expect("wol packet");
    assert_eq!(&pkt[..6], &[0xff; 6]);
    for chunk in pkt[6..].chunks_exact(6) {
        assert_eq!(chunk, &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }
    assert!(build_wol_magic_packet("bad-mac").is_err());

    // Headless screen capture & cache
    let cap = ScreenCaptureEngine::new();
    assert!(!cap.list_displays().unwrap().is_empty());
    let f1 = cap.capture_or_cached(0, 5_000, CaptureFormat::Png).unwrap();
    let f2 = cap.capture_or_cached(0, 5_000, CaptureFormat::Png).unwrap();
    assert_eq!(f1.seq, f2.seq);
    assert!(f1.data.starts_with(b"\x89PNG"));

    // Audio RAII mute guard
    let audio = AudioPipeline::new();
    assert!(!audio.is_remote_speaker_muted());
    {
        let _guard = audio.mute_remote_speaker().unwrap();
        assert!(audio.is_remote_speaker_muted());
    }
    assert!(!audio.is_remote_speaker_muted());

    // Display/privacy, input remap, accessibility, power headless dry-run
    let dp = DisplayPrivacyManager::new();
    dp.set_privacy_screen(true, Some("Locked".into())).unwrap();
    assert!(dp.is_privacy_screen_enabled());
    assert_eq!(
        NativeInputEngine::remap_modifiers(&["ctrl".into()], "windows", "macos", true),
        vec!["cmd".to_string()]
    );
    let ax = AccessibilityEngine;
    assert_eq!(ax.capture_tree(Some(2)).unwrap().element_id, "headless-root");
    assert!(PowerManager::verify_touch_id("test").unwrap());
    assert!(PowerManager::execute_power_action(&PowerActionKind::Lock)
        .unwrap()
        .contains("headless"));
}

#[test]
fn test_normalize_pty_output_conpty_vt100_sequences() {
    let raw = concat!(
        "\x1b]0;cmd.exe\x07",
        "\x1b[2J\x1b[m\x1b[H",
        "\x1b[2;1HC:\\Users\\Admin>whoami\x1b[1Cand\x1b[1Chost",
        "\x1b[3;1H\x1b[32m0cadhjae-vmejbe\\administrator\x1b[0m",
        "\x1b[4;1H__FABUSHI_EXEC_0123456789abcdef__:0__FB_END__",
        "\x1b[?25h"
    );
    let norm = normalize_pty_output(raw);
    let lines: Vec<&str> = norm.lines().collect();
    assert_eq!(lines[0], "C:\\Users\\Admin>whoami and host");
    assert_eq!(lines[1], "0cadhjae-vmejbe\\administrator");
    assert_eq!(lines[2], "__FABUSHI_EXEC_0123456789abcdef__:0__FB_END__");
}

#[cfg(windows)]
#[tokio::test]
async fn test_windows_powershell_pty_exec() {
    let mux = Arc::new(PtyMuxServer::new());
    let created = mux
        .new_session(Some("ps-ci"), Some("powershell.exe"), 120, 30, None)
        .expect("spawn powershell ConPTY");
    assert!(created.alive);

    let res1 = mux
        .exec_command("ps-ci", "Write-Output fabushi_ps_ok", 20_000)
        .await
        .expect("exec ps Write-Output");
    assert!(
        !res1.timed_out,
        "powershell Write-Output timed out: {:?}",
        res1.output
    );
    assert_eq!(res1.exit_code, 0);
    assert!(
        res1.output.contains("fabushi_ps_ok"),
        "unexpected ps output: {:?}",
        res1.output
    );

    let res_err = mux
        .exec_command("ps-ci", "cmd /c exit 7", 15_000)
        .await
        .expect("exec ps non-zero");
    assert!(
        !res_err.timed_out,
        "powershell non-zero exec timed out: {:?}",
        res_err.output
    );
    assert_eq!(res_err.exit_code, 7);

    let res_rec = mux
        .exec_command("ps-ci", "Write-Output after_err_ok", 15_000)
        .await
        .expect("exec ps recovery");
    assert!(
        !res_rec.timed_out,
        "powershell recovery timed out: {:?}",
        res_rec.output
    );
    assert_eq!(res_rec.exit_code, 0);
    assert!(
        res_rec.output.contains("after_err_ok"),
        "unexpected ps recovery output: {:?}",
        res_rec.output
    );

    assert!(mux.kill_session("ps-ci").expect("kill ps session"));
}
