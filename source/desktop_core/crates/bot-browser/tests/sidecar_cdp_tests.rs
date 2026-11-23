use std::collections::HashMap;
use std::io::{Cursor, Write as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use flate2::Compression;
use flate2::write::GzEncoder;
use futures::{SinkExt, StreamExt};
use image::{GenericImageView, ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, broadcast};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use zeroize::Zeroizing;
use zip::write::SimpleFileOptions;

use bot_browser::cdp::{
    CdpBridge, FillSubmitResult, LoginFormSelectors, ObscuraSidecarTransport, WsCdpTransport,
};
use bot_browser::sidecar::{
    ArchiveKind, DEFAULT_OBSCURA_VERSION, ObscuraBinaryManifest, ObscuraDownloader,
    ObscuraReleaseArtifact, ObscuraSidecar, ProfileNamespace, SidecarConfig,
};
use bot_browser::{
    BotBrowserError, BotBrowserEvent, DecryptedSecret, ElementRect, NormalizedClickPoint,
    NormalizedOrigin,
};

#[derive(Clone, Debug)]
struct RecordedCdpCall {
    method: String,
    params: Value,
}

type MockHandler = Arc<
    dyn Fn(&str, &Value, usize) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, Value>> + Send>>
        + Send
        + Sync,
>;

struct MockCdpServer {
    http_endpoint: String,
    recorded_calls: Arc<Mutex<Vec<RecordedCdpCall>>>,
    _shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl MockCdpServer {
    async fn start<F, Fut>(handler: F) -> Self
    where
        F: Fn(String, Value, usize) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Value, Value>> + Send + 'static,
    {
        let handler: MockHandler = Arc::new(move |method, params, seq| {
            Box::pin(handler(method.to_string(), params.clone(), seq))
        });
        let recorded_calls = Arc::new(Mutex::new(Vec::new()));

        let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_port = ws_listener.local_addr().unwrap().port();
        let ws_url = format!("ws://127.0.0.1:{ws_port}/devtools/page/1");

        let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_port = http_listener.local_addr().unwrap().port();
        let http_endpoint = format!("http://127.0.0.1:{http_port}");

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let calls_ref = Arc::clone(&recorded_calls);

        tokio::spawn(async move {
            let seq = Arc::new(AtomicUsize::new(0));
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    Ok((mut sock, _)) = http_listener.accept() => {
                        let ws_url_clone = ws_url.clone();
                        tokio::spawn(async move {
                            let mut buf = [0u8; 2048];
                            let _ = sock.read(&mut buf).await;
                            let body = json!([{
                                "id": "page-1",
                                "type": "page",
                                "title": "Mock Page",
                                "url": "https://example.com/",
                                "webSocketDebuggerUrl": ws_url_clone
                            }]).to_string();
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = sock.write_all(resp.as_bytes()).await;
                        });
                    }
                    Ok((stream, _)) = ws_listener.accept() => {
                        let handler_clone = Arc::clone(&handler);
                        let calls_clone = Arc::clone(&calls_ref);
                        let seq_clone = Arc::clone(&seq);
                        tokio::spawn(async move {
                            let Ok(mut ws) = accept_async(stream).await else { return; };
                            // Emit an unrelated CDP event frame first to ensure WsCdpTransport ignores events.
                            let _ = ws.send(Message::Text(json!({
                                "method": "Page.frameStartedLoading",
                                "params": { "frameId": "main" }
                            }).to_string())).await;

                            while let Some(Ok(msg)) = ws.next().await {
                                match msg {
                                    Message::Text(txt) => {
                                        let Ok(req) = serde_json::from_str::<Value>(&txt) else { continue; };
                                        let id = req.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                                        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
                                        let call_idx = seq_clone.fetch_add(1, Ordering::SeqCst);
                                        calls_clone.lock().await.push(RecordedCdpCall {
                                            method: method.clone(),
                                            params: params.clone(),
                                        });
                                        let reply = match handler_clone(&method, &params, call_idx).await {
                                            Ok(result) => json!({ "id": id, "result": result }),
                                            Err(err) => json!({ "id": id, "error": err }),
                                        };
                                        if ws.send(Message::Text(reply.to_string())).await.is_err() {
                                            break;
                                        }
                                    }
                                    Message::Close(_) => break,
                                    _ => {}
                                }
                            }
                        });
                    }
                }
            }
        });

        Self {
            http_endpoint,
            recorded_calls,
            _shutdown_tx: shutdown_tx,
        }
    }

    async fn calls(&self) -> Vec<RecordedCdpCall> {
        self.recorded_calls.lock().await.clone()
    }
}

fn encode_png_rgba(img: &RgbaImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[tokio::test]
async fn browser_snapshot_redacts_password_inputs_in_js_and_rust() {
    let server = MockCdpServer::start(|method, params, _seq| async move {
        if method == "Runtime.evaluate" {
            let expr = params.get("expression").and_then(|v| v.as_str()).unwrap_or("");
            assert!(expr.contains("[REDACTED]"));
            assert!(expr.contains("one-time-code"));
            // Deliberately simulate leaked values coming back from a broken/adversarial JS layer
            // to prove Rust's post-pass forces "[REDACTED]" on all sensitive fields.
            let raw_snapshot = json!({
                "url": "https://Login.Example.COM:443/auth",
                "origin": "https://Login.Example.COM:443",
                "title": "Sign In",
                "text_excerpt": "Welcome back",
                "redacted_field_count": 0,
                "interactive_elements": [
                    {
                        "ref_id": "el_1",
                        "tag": "input",
                        "input_type": "text",
                        "name": "username",
                        "id": "user",
                        "text": "",
                        "value": "alice@example.com",
                        "is_password": false,
                        "selector": "#user"
                    },
                    {
                        "ref_id": "el_2",
                        "tag": "input",
                        "input_type": "password",
                        "name": "pw",
                        "id": "pass",
                        "text": "",
                        "value": "LEAKED_PASSWORD_1",
                        "is_password": false,
                        "selector": "#pass"
                    },
                    {
                        "ref_id": "el_3",
                        "tag": "input",
                        "input_type": "text",
                        "autocomplete": "current-password",
                        "name": "secret_box",
                        "id": "sb",
                        "text": "",
                        "value": "LEAKED_PASSWORD_2",
                        "is_password": false,
                        "selector": "#sb"
                    },
                    {
                        "ref_id": "el_4",
                        "tag": "input",
                        "input_type": "text",
                        "autocomplete": "one-time-code",
                        "name": "code",
                        "id": "code",
                        "text": "",
                        "value": "654321",
                        "is_password": false,
                        "selector": "#code"
                    },
                    {
                        "ref_id": "el_5",
                        "tag": "input",
                        "input_type": "text",
                        "name": "user_password",
                        "id": "upw",
                        "text": "",
                        "value": "LEAKED_PASSWORD_3",
                        "is_password": false,
                        "selector": "#upw"
                    }
                ]
            });
            return Ok(json!({ "result": { "value": raw_snapshot.to_string() } }));
        }
        Ok(json!({}))
    })
    .await;

    let transport = Arc::new(WsCdpTransport::new(&server.http_endpoint));
    let bridge = CdpBridge::new(transport);
    let snapshot = bridge.capture_redacted_snapshot(None).await.unwrap();

    assert_eq!(snapshot.origin, "https://login.example.com");
    assert_eq!(snapshot.redacted_field_count, 4);
    assert_eq!(snapshot.interactive_elements.len(), 5);

    assert_eq!(
        snapshot.interactive_elements[0].value.as_deref(),
        Some("alice@example.com")
    );
    assert!(!snapshot.interactive_elements[0].is_password);

    for idx in 1..=4 {
        assert!(snapshot.interactive_elements[idx].is_password);
        assert_eq!(
            snapshot.interactive_elements[idx].value.as_deref(),
            Some("[REDACTED]")
        );
    }

    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("LEAKED_PASSWORD_1"));
    assert!(!serialized.contains("LEAKED_PASSWORD_2"));
    assert!(!serialized.contains("654321"));
    assert!(!serialized.contains("LEAKED_PASSWORD_3"));
}

#[tokio::test]
async fn atomic_fill_writes_only_via_insert_text_and_blocks_origin_mismatch() {
    let node_ids: HashMap<&str, i64> = HashMap::from([
        ("#username", 101),
        ("#password", 102),
        ("#totp", 103),
    ]);
    let page_origin = Arc::new(Mutex::new("https://app.example.com".to_string()));
    let frame_origin = Arc::new(Mutex::new(Some("https://app.example.com".to_string())));

    let page_origin_ref = Arc::clone(&page_origin);
    let frame_origin_ref = Arc::clone(&frame_origin);

    let server = MockCdpServer::start(move |method, params, _seq| {
        let node_ids = node_ids.clone();
        let page_origin_ref = Arc::clone(&page_origin_ref);
        let frame_origin_ref = Arc::clone(&frame_origin_ref);
        async move {
            match method.as_str() {
                "Runtime.evaluate" => {
                    let expr = params.get("expression").and_then(|v| v.as_str()).unwrap_or("");
                    if expr.contains("page_origin") {
                        let po = page_origin_ref.lock().await.clone();
                        let fo = frame_origin_ref.lock().await.clone();
                        return Ok(json!({
                            "result": {
                                "value": json!({
                                    "page_origin": po,
                                    "frame_origin": fo,
                                    "href": format!("{po}/login")
                                }).to_string()
                            }
                        }));
                    }
                    Ok(json!({ "result": { "value": "CLICKED" } }))
                }
                "DOM.getDocument" => Ok(json!({ "root": { "nodeId": 1 } })),
                "DOM.querySelector" => {
                    let sel = params.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                    let id = node_ids.get(sel).copied().unwrap_or(0);
                    Ok(json!({ "nodeId": id }))
                }
                "DOM.focus" | "Input.dispatchKeyEvent" | "Input.insertText" => Ok(json!({})),
                other => Err(json!({ "code": -32601, "message": format!("unhandled {other}") })),
            }
        }
    })
    .await;

    let transport = Arc::new(WsCdpTransport::new(&server.http_endpoint));
    let bridge = CdpBridge::new(transport);

    let expected_origin = NormalizedOrigin::parse("https://app.example.com").unwrap();
    let secret = DecryptedSecret {
        username: Zeroizing::new("alice_secret_user".to_string()),
        password: Zeroizing::new("Hunter2!SuperSecret#99".to_string()),
        totp_secret: Some(Zeroizing::new("JBSWY3DPEHPK3PXP".to_string())),
    };
    let totp_code = Zeroizing::new("482910".to_string());
    let selectors = LoginFormSelectors {
        username_selector: "#username".to_string(),
        password_selector: "#password".to_string(),
        totp_selector: Some("#totp".to_string()),
        submit_selector: Some("#submit-btn".to_string()),
    };

    let res: FillSubmitResult = bridge
        .atomic_fill_and_submit(&expected_origin, &secret, Some(&totp_code), &selectors)
        .await
        .unwrap();
    assert_eq!(res.origin, "https://app.example.com");
    assert!(res.submitted);
    assert!(res.used_totp);

    let calls = server.calls().await;
    let insert_texts: Vec<String> = calls
        .iter()
        .filter(|c| c.method == "Input.insertText")
        .filter_map(|c| c.params.get("text").and_then(|v| v.as_str()).map(ToString::to_string))
        .collect();
    assert_eq!(
        insert_texts,
        vec![
            "alice_secret_user".to_string(),
            "Hunter2!SuperSecret#99".to_string(),
            "482910".to_string()
        ]
    );

    // Zero Runtime.evaluate call may ever contain username, password, or TOTP code.
    for call in &calls {
        if call.method == "Runtime.evaluate" {
            let expr = call.params.to_string();
            assert!(!expr.contains("alice_secret_user"));
            assert!(!expr.contains("Hunter2!SuperSecret#99"));
            assert!(!expr.contains("482910"));
        }
    }

    // Verify DOM.focus was used for all 3 fields.
    let focus_count = calls.iter().filter(|c| c.method == "DOM.focus").count();
    assert_eq!(focus_count, 3);

    // Cross-origin frame must fail with FrameOriginMismatch and send ZERO Input.insertText calls.
    let before_len = calls.len();
    *frame_origin.lock().await = Some("https://evil.attacker.com".to_string());
    let err = bridge
        .atomic_fill_and_submit(&expected_origin, &secret, Some(&totp_code), &selectors)
        .await
        .unwrap_err();
    assert!(matches!(err, BotBrowserError::FrameOriginMismatch { .. }));

    let after_calls = server.calls().await;
    let new_inserts = after_calls[before_len..]
        .iter()
        .filter(|c| c.method == "Input.insertText")
        .count();
    assert_eq!(new_inserts, 0);

    // Cross-origin page must fail with OriginMismatch.
    *page_origin.lock().await = "https://phishing.attacker.com".to_string();
    let err2 = bridge
        .atomic_fill_and_submit(&expected_origin, &secret, Some(&totp_code), &selectors)
        .await
        .unwrap_err();
    assert!(matches!(err2, BotBrowserError::OriginMismatch { .. }));
}

#[tokio::test]
async fn concurrent_evaluate_and_screenshot_rejected_while_atomic_fill_active() {
    let paused_notify = Arc::new(Notify::new());
    let resume_notify = Arc::new(Notify::new());

    let paused_ref = Arc::clone(&paused_notify);
    let resume_ref = Arc::clone(&resume_notify);

    let server = MockCdpServer::start(move |method, _params, _seq| {
        let paused_ref = Arc::clone(&paused_ref);
        let resume_ref = Arc::clone(&resume_ref);
        async move {
            match method.as_str() {
                "Runtime.evaluate" => Ok(json!({
                    "result": {
                        "value": json!({
                            "page_origin": "https://app.example.com",
                            "frame_origin": "https://app.example.com",
                            "href": "https://app.example.com/login"
                        }).to_string()
                    }
                })),
                "DOM.getDocument" => Ok(json!({ "root": { "nodeId": 1 } })),
                "DOM.querySelector" => Ok(json!({ "nodeId": 77 })),
                "DOM.focus" | "Input.dispatchKeyEvent" => Ok(json!({})),
                "Input.insertText" => {
                    paused_ref.notify_one();
                    resume_ref.notified().await;
                    Ok(json!({}))
                }
                _ => Ok(json!({})),
            }
        }
    })
    .await;

    let transport = Arc::new(WsCdpTransport::new(&server.http_endpoint));
    let bridge = Arc::new(CdpBridge::new(transport));

    let bridge_fill = Arc::clone(&bridge);
    let fill_task = tokio::spawn(async move {
        let origin = NormalizedOrigin::parse("https://app.example.com").unwrap();
        let secret = DecryptedSecret {
            username: Zeroizing::new("bob".to_string()),
            password: Zeroizing::new("secret_pw".to_string()),
            totp_secret: None,
        };
        let selectors = LoginFormSelectors {
            username_selector: "#user".to_string(),
            password_selector: "#pass".to_string(),
            totp_selector: None,
            submit_selector: None,
        };
        bridge_fill
            .atomic_fill_and_submit(&origin, &secret, None, &selectors)
            .await
    });

    // Wait until atomic_fill_and_submit is actively paused inside Input.insertText.
    paused_notify.notified().await;
    assert!(bridge.is_atomic_fill_active());

    let eval_err = bridge.evaluate_script("document.cookie").await.unwrap_err();
    assert_eq!(eval_err, BotBrowserError::SessionLockedForSecretFill);

    let shot_err = bridge.capture_screenshot_png(false).await.unwrap_err();
    assert_eq!(shot_err, BotBrowserError::SessionLockedForSecretFill);

    let snap_err = bridge.capture_redacted_snapshot(None).await.unwrap_err();
    assert_eq!(snap_err, BotBrowserError::SessionLockedForSecretFill);

    let elem_err = bridge.capture_element_png("#captcha").await.unwrap_err();
    assert_eq!(elem_err, BotBrowserError::SessionLockedForSecretFill);

    // Unblock fill (two Input.insertText calls: username + password).
    resume_notify.notify_one();
    tokio::task::yield_now().await;
    resume_notify.notify_one();

    let fill_res = fill_task.await.unwrap().unwrap();
    assert!(fill_res.submitted);
    assert!(!bridge.is_atomic_fill_active());

    // After atomic fill finishes, evaluate_script succeeds normally again.
    assert!(bridge.evaluate_script("1 + 1").await.is_ok());
}

#[tokio::test]
async fn capture_element_png_crops_full_viewport_fallback_accurately() {
    // Full viewport 640x480 blue image with a distinct green target box at (120, 90, 160x100).
    let mut full_img = RgbaImage::from_pixel(640, 480, Rgba([10, 20, 200, 255]));
    for y in 90..190 {
        for x in 120..280 {
            full_img.put_pixel(x, y, Rgba([20, 220, 40, 255]));
        }
    }
    let full_png_b64 = BASE64_STANDARD.encode(encode_png_rgba(&full_img));

    // Also test a 2x Retina DPR full-viewport screenshot (1280x960 for 640x480 CSS viewport).
    let mut retina_img = RgbaImage::from_pixel(1280, 960, Rgba([10, 20, 200, 255]));
    for y in 180..380 {
        for x in 240..560 {
            retina_img.put_pixel(x, y, Rgba([220, 40, 20, 255]));
        }
    }
    let retina_png_b64 = BASE64_STANDARD.encode(encode_png_rgba(&retina_img));

    let server = MockCdpServer::start(move |method, params, _seq| {
        let full_png_b64 = full_png_b64.clone();
        let retina_png_b64 = retina_png_b64.clone();
        async move {
            match method.as_str() {
                "Runtime.evaluate" => {
                    let expr = params.get("expression").and_then(|v| v.as_str()).unwrap_or("");
                    let dpr = if expr.contains("retina") { 2.0 } else { 1.0 };
                    Ok(json!({
                        "result": {
                            "value": json!({
                                "found": true,
                                "x": 120.0,
                                "y": 90.0,
                                "width": 160.0,
                                "height": 100.0,
                                "device_pixel_ratio": dpr,
                                "viewport_width": 640.0,
                                "viewport_height": 480.0
                            }).to_string()
                        }
                    }))
                }
                "Page.captureScreenshot" => {
                    // Simulate Obscura returning the uncropped full-viewport PNG despite `clip`.
                    let clip_w = params
                        .get("clip")
                        .and_then(|c| c.get("width"))
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    assert_eq!(clip_w, 160.0);
                    let data = if _seq >= 2 { retina_png_b64 } else { full_png_b64 };
                    Ok(json!({ "data": data }))
                }
                _ => Ok(json!({})),
            }
        }
    })
    .await;

    let transport = Arc::new(WsCdpTransport::new(&server.http_endpoint));
    let bridge = CdpBridge::new(transport);

    // Case 1: 1x full-viewport screenshot cropped accurately to 160x100 green square.
    let (cropped_bytes, rect) = bridge.capture_element_png("#qr-box").await.unwrap();
    assert_eq!(rect.width, 160.0);
    assert_eq!(rect.height, 100.0);
    let decoded = image::load_from_memory_with_format(&cropped_bytes, ImageFormat::Png).unwrap();
    assert_eq!(decoded.dimensions(), (160, 100));
    let rgba = decoded.to_rgba8();
    assert!(rgba.pixels().all(|p| *p == Rgba([20, 220, 40, 255])));

    // Case 2: 2x Retina full-viewport screenshot cropped accurately to 320x200 red square.
    let (cropped_retina, rect_retina) = bridge.capture_element_png("#retina-box").await.unwrap();
    assert_eq!(rect_retina.device_pixel_ratio, 2.0);
    let decoded_retina =
        image::load_from_memory_with_format(&cropped_retina, ImageFormat::Png).unwrap();
    assert_eq!(decoded_retina.dimensions(), (320, 200));
    assert!(
        decoded_retina
            .to_rgba8()
            .pixels()
            .all(|p| *p == Rgba([220, 40, 20, 255]))
    );
}

#[tokio::test]
async fn dispatch_element_relative_click_computes_exact_coordinates_and_clamps() {
    let server = MockCdpServer::start(|_method, _params, _seq| async move { Ok(json!({})) }).await;
    let transport = Arc::new(WsCdpTransport::new(&server.http_endpoint));
    let bridge = CdpBridge::new(transport);

    let rect = ElementRect {
        x: 100.0,
        y: 200.0,
        width: 300.0,
        height: 120.0,
        device_pixel_ratio: 2.0,
    };

    let (px, py) = bridge
        .dispatch_element_relative_click(&rect, NormalizedClickPoint::new(0.5, 0.25))
        .await
        .unwrap();
    assert!((px - 250.0).abs() < f64::EPSILON);
    assert!((py - 230.0).abs() < f64::EPSILON);

    let (clamped_x, clamped_y) = bridge
        .dispatch_element_relative_click(&rect, NormalizedClickPoint::new(1.5, -0.4))
        .await
        .unwrap();
    assert!((clamped_x - 400.0).abs() < f64::EPSILON);
    assert!((clamped_y - 200.0).abs() < f64::EPSILON);

    let calls = server.calls().await;
    let mouse_events: Vec<(&str, f64, f64)> = calls
        .iter()
        .filter(|c| c.method == "Input.dispatchMouseEvent")
        .map(|c| {
            (
                c.params.get("type").and_then(|v| v.as_str()).unwrap(),
                c.params.get("x").and_then(|v| v.as_f64()).unwrap(),
                c.params.get("y").and_then(|v| v.as_f64()).unwrap(),
            )
        })
        .collect();

    assert_eq!(
        mouse_events,
        vec![
            ("mouseMoved", 250.0, 230.0),
            ("mousePressed", 250.0, 230.0),
            ("mouseReleased", 250.0, 230.0),
            ("mouseMoved", 400.0, 200.0),
            ("mousePressed", 400.0, 200.0),
            ("mouseReleased", 400.0, 200.0),
        ]
    );
}

#[tokio::test]
async fn obscura_downloader_verifies_sha256_targz_and_rejects_tampered_checksum() {
    let bin_bytes = b"#!/bin/sh\necho obscura-test-binary\n";
    let mut tar_gz_buf = Vec::new();
    {
        let gz = GzEncoder::new(&mut tar_gz_buf, Compression::default());
        let mut tar = Builder::new(gz);
        let mut header = Header::new_gnu();
        header.set_size(bin_bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "bin/obscura", &bin_bytes[..])
            .unwrap();
        let gz = tar.into_inner().unwrap();
        gz.finish().unwrap();
    }

    let valid_sha256 = hex::encode(Sha256::digest(&tar_gz_buf));
    let serve_bytes = Arc::new(tar_gz_buf);
    let request_hits = Arc::new(AtomicUsize::new(0));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let bytes_ref = Arc::clone(&serve_bytes);
    let hits_ref = Arc::clone(&request_hits);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = listener.accept() => {
                    let payload = Arc::clone(&bytes_ref);
                    let hits = Arc::clone(&hits_ref);
                    tokio::spawn(async move {
                        let mut req_buf = [0u8; 1024];
                        let _ = sock.read(&mut req_buf).await;
                        hits.fetch_add(1, Ordering::SeqCst);
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            payload.len()
                        );
                        let _ = sock.write_all(hdr.as_bytes()).await;
                        let _ = sock.write_all(&payload).await;
                    });
                }
            }
        }
    });

    let tmp_dir = TempDir::new().unwrap();
    let data_dir = tmp_dir.path().to_path_buf();
    let target = ObscuraBinaryManifest::current_target().to_string();
    let url = format!("http://127.0.0.1:{port}/obscura.tar.gz");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // 1. Tampered SHA-256 must fail with SidecarChecksumMismatch and leave ZERO .tmp files.
    let bad_manifest = ObscuraBinaryManifest {
        version: "0.1.0-test".to_string(),
        artifacts: vec![ObscuraReleaseArtifact {
            target: target.clone(),
            url: url.clone(),
            sha256: "0".repeat(64),
            archive_kind: ArchiveKind::TarGz,
        }],
    };
    let bad_res = ObscuraDownloader::ensure_binary_with_overrides(
        &data_dir,
        &bad_manifest,
        &client,
        None,
        None,
        None,
    )
    .await;
    assert!(matches!(
        bad_res,
        Err(BotBrowserError::SidecarChecksumMismatch { .. })
    ));

    let bin_version_dir = data_dir.join("bin").join("0.1.0-test");
    for entry in std::fs::read_dir(&bin_version_dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(!name.ends_with(".tmp"), "leftover tmp file: {name}");
    }

    // 2. Valid SHA-256 downloads, extracts, chmods, and caches cleanly.
    let good_manifest = ObscuraBinaryManifest {
        version: "0.1.0-test".to_string(),
        artifacts: vec![ObscuraReleaseArtifact {
            target: target.clone(),
            url,
            sha256: valid_sha256,
            archive_kind: ArchiveKind::TarGz,
        }],
    };
    let bin_path = ObscuraDownloader::ensure_binary_with_overrides(
        &data_dir,
        &good_manifest,
        &client,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(bin_path.is_file());
    assert_eq!(std::fs::read(&bin_path).unwrap(), bin_bytes);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&bin_path).unwrap().permissions().mode();
        assert_ne!(mode & 0o111, 0, "binary must be executable");
    }

    for entry in std::fs::read_dir(&bin_version_dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(!name.ends_with(".tmp"), "leftover tmp file: {name}");
    }

    let hits_after_download = request_hits.load(Ordering::SeqCst);
    let bin_path_cached = ObscuraDownloader::ensure_binary_with_overrides(
        &data_dir,
        &good_manifest,
        &client,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(bin_path_cached, bin_path);
    assert_eq!(request_hits.load(Ordering::SeqCst), hits_after_download);

    // 3. Verify BOT_BROWSER_OBSCURA_BIN override precedence & ProfileNamespace determinism.
    let override_path = ObscuraDownloader::ensure_binary_with_overrides(
        &data_dir,
        &good_manifest,
        &client,
        Some(bin_path.to_str().unwrap()),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(override_path, bin_path);

    let origin = NormalizedOrigin::parse("https://Dashboard.Stripe.com:443/pay").unwrap();
    let p1 = ProfileNamespace::storage_dir_for(&data_dir, "Agent Session/1", Some(&origin));
    let p2 = ProfileNamespace::storage_dir_for(&data_dir, "Agent Session/1", Some(&origin));
    assert_eq!(p1, p2);
    assert!(p1.starts_with(data_dir.join("profiles")));

    let _ = shutdown_tx.send(());
}

#[test]
fn default_obscura_v027_manifest_pins_all_five_targets_with_sha256() {
    let manifest = ObscuraBinaryManifest::default_obscura();
    assert_eq!(DEFAULT_OBSCURA_VERSION, "0.2.7");
    assert_eq!(manifest.version, "0.2.7");

    let expected = [
        (
            "aarch64-macos",
            "https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.7/obscura-aarch64-macos-stealth.tar.gz",
            "3b05ebcd4755a325be224992fad6a038d86f7e5ab483e4c781f14de06cab973f",
            ArchiveKind::TarGz,
        ),
        (
            "x86_64-macos",
            "https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.7/obscura-x86_64-macos-stealth.tar.gz",
            "62bd094388a1f97480fd06be349299e721a3488c216c86a2fce9367310594ec3",
            ArchiveKind::TarGz,
        ),
        (
            "x86_64-linux",
            "https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.7/obscura-x86_64-linux-stealth.tar.gz",
            "1d6514beacef9fe75671f4bf7a66d3a7bceef9bde39f8156d1ca4a701eb1e216",
            ArchiveKind::TarGz,
        ),
        (
            "aarch64-linux",
            "https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.7/obscura-aarch64-linux-stealth.tar.gz",
            "f51167d7ae98be6c6327d3e7e015e74430e190ea6846589e3c131e10641a52b7",
            ArchiveKind::TarGz,
        ),
        (
            "x86_64-windows",
            "https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.7/obscura-x86_64-windows-stealth.zip",
            "ffae5e71cfca0108709b57ad83ce7d6b588eff6a15a43093b0cf867dbd18c867",
            ArchiveKind::Zip,
        ),
    ];

    assert_eq!(manifest.artifacts.len(), expected.len());
    for (target, url, sha, kind) in expected {
        let art = manifest
            .artifact_for_target(target)
            .unwrap_or_else(|| panic!("missing manifest target {target}"));
        assert_eq!(art.url, url);
        assert_eq!(art.sha256, sha);
        assert_eq!(art.sha256.len(), 64);
        assert!(art.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(art.archive_kind, kind);
    }
}

#[tokio::test]
async fn obscura_downloader_extracts_zip_and_prefixed_stealth_binaries() {
    let bin_bytes = b"#!/bin/sh\necho obscura-zip-stealth\n";

    // Build a ZIP archive containing README + an obscura*-prefixed binary.
    let mut zip_buf = Vec::new();
    {
        let cursor = Cursor::new(&mut zip_buf);
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file("README.md", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"ignore me").unwrap();
        writer
            .start_file(
                "obscura-stealth-dir/obscura-aarch64-macos-stealth",
                SimpleFileOptions::default().unix_permissions(0o755),
            )
            .unwrap();
        writer.write_all(bin_bytes).unwrap();
        writer.finish().unwrap();
    }
    let zip_sha256 = hex::encode(Sha256::digest(&zip_buf));

    // Also build a TarGz archive containing obscura.sha256 + ./obscura-aarch64-macos-stealth.
    let tgz_bytes = b"#!/bin/sh\necho obscura-targz-stealth\n";
    let mut tgz_buf = Vec::new();
    {
        let gz = GzEncoder::new(&mut tgz_buf, Compression::default());
        let mut tar = Builder::new(gz);

        let sha_meta = b"deadbeef  obscura\n";
        let mut sha_hdr = Header::new_gnu();
        sha_hdr.set_size(sha_meta.len() as u64);
        sha_hdr.set_mode(0o644);
        sha_hdr.set_cksum();
        tar.append_data(&mut sha_hdr, "obscura.sha256", &sha_meta[..])
            .unwrap();

        let mut bin_hdr = Header::new_gnu();
        bin_hdr.set_size(tgz_bytes.len() as u64);
        bin_hdr.set_mode(0o755);
        bin_hdr.set_cksum();
        tar.append_data(
            &mut bin_hdr,
            "./obscura-aarch64-macos-stealth",
            &tgz_bytes[..],
        )
        .unwrap();

        let gz = tar.into_inner().unwrap();
        gz.finish().unwrap();
    }
    let tgz_sha256 = hex::encode(Sha256::digest(&tgz_buf));

    let routes = Arc::new(HashMap::from([
        ("/obscura.zip".to_string(), zip_buf),
        ("/obscura.tar.gz".to_string(), tgz_buf),
    ]));
    let hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let routes_ref = Arc::clone(&routes);
    let hits_ref = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                Ok((mut sock, _)) = listener.accept() => {
                    let routes_c = Arc::clone(&routes_ref);
                    let hits_c = Arc::clone(&hits_ref);
                    tokio::spawn(async move {
                        let mut buf = [0u8; 2048];
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let path = req
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .unwrap_or("/");
                        if let Some(payload) = routes_c.get(path) {
                            hits_c.fetch_add(1, Ordering::SeqCst);
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                payload.len()
                            );
                            let _ = sock.write_all(hdr.as_bytes()).await;
                            let _ = sock.write_all(payload).await;
                        } else {
                            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
                        }
                    });
                }
            }
        }
    });

    let tmp_dir = TempDir::new().unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let target = ObscuraBinaryManifest::current_target().to_string();

    // 1. Tampered ZIP checksum must fail.
    let bad_zip_manifest = ObscuraBinaryManifest {
        version: "0.2.7-zip-test".to_string(),
        artifacts: vec![ObscuraReleaseArtifact {
            target: target.clone(),
            url: format!("http://127.0.0.1:{port}/obscura.zip"),
            sha256: "f".repeat(64),
            archive_kind: ArchiveKind::Zip,
        }],
    };
    let bad_err = ObscuraDownloader::ensure_binary_with_overrides(
        tmp_dir.path(),
        &bad_zip_manifest,
        &client,
        None,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(bad_err, BotBrowserError::SidecarChecksumMismatch { .. }));

    // 2. Valid ZIP extracts obscura-*-stealth binary, caches it on second call.
    let zip_manifest = ObscuraBinaryManifest {
        version: "0.2.7-zip-test".to_string(),
        artifacts: vec![ObscuraReleaseArtifact {
            target: target.clone(),
            url: format!("http://127.0.0.1:{port}/obscura.zip"),
            sha256: zip_sha256,
            archive_kind: ArchiveKind::Zip,
        }],
    };
    let zip_bin = ObscuraDownloader::ensure_binary_with_overrides(
        tmp_dir.path(),
        &zip_manifest,
        &client,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&zip_bin).unwrap(), bin_bytes);
    let hits_after_zip = hits.load(Ordering::SeqCst);
    let zip_bin_cached = ObscuraDownloader::ensure_binary_with_overrides(
        tmp_dir.path(),
        &zip_manifest,
        &client,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(zip_bin_cached, zip_bin);
    assert_eq!(hits.load(Ordering::SeqCst), hits_after_zip);

    // 3. Valid TarGz skips obscura.sha256 and extracts ./obscura-aarch64-macos-stealth.
    let tgz_manifest = ObscuraBinaryManifest {
        version: "0.2.7-tgz-stealth-test".to_string(),
        artifacts: vec![ObscuraReleaseArtifact {
            target,
            url: format!("http://127.0.0.1:{port}/obscura.tar.gz"),
            sha256: tgz_sha256,
            archive_kind: ArchiveKind::TarGz,
        }],
    };
    let tgz_bin = ObscuraDownloader::ensure_binary_with_overrides(
        tmp_dir.path(),
        &tgz_manifest,
        &client,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&tgz_bin).unwrap(), tgz_bytes);

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn obscura_sidecar_watchdog_kills_hung_process_restarts_and_renavigates() {
    let tmp_dir = TempDir::new().unwrap();
    let script_path = tmp_dir.path().join("mock_obscura.py");
    let storage_dir = ProfileNamespace::storage_dir_for(
        tmp_dir.path(),
        "watchdog-test",
        Some(&NormalizedOrigin::parse("https://app.example.com").unwrap()),
    );

    let mock_py = r####"#!/usr/bin/env python3
import argparse
import base64
import hashlib
import json
import os
import socket
import struct
import sys
import threading
import time

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

def recv_exact(conn, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk:
            return None
        buf.extend(chunk)
    return bytes(buf)

def read_ws_frame(conn):
    hdr = recv_exact(conn, 2)
    if not hdr:
        return None
    opcode = hdr[0] & 0x0F
    masked = (hdr[1] & 0x80) != 0
    length = hdr[1] & 0x7F
    if length == 126:
        ext = recv_exact(conn, 2)
        if not ext:
            return None
        length = struct.unpack("!H", ext)[0]
    elif length == 127:
        ext = recv_exact(conn, 8)
        if not ext:
            return None
        length = struct.unpack("!Q", ext)[0]
    mask = recv_exact(conn, 4) if masked else b""
    payload = recv_exact(conn, length) if length > 0 else b""
    if payload is None:
        return None
    if masked:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return opcode, payload

def send_ws_text(conn, text):
    data = text.encode("utf-8")
    frame = bytearray([0x81])
    if len(data) < 126:
        frame.append(len(data))
    elif len(data) <= 65535:
        frame.append(126)
        frame.extend(struct.pack("!H", len(data)))
    else:
        frame.append(127)
        frame.extend(struct.pack("!Q", len(data)))
    frame.extend(data)
    conn.sendall(bytes(frame))

def handle_conn(conn, port, storage_dir, run_id, log_lock):
    try:
        req = b""
        while b"\r\n\r\n" not in req:
            chunk = conn.recv(4096)
            if not chunk:
                return
            req += chunk
        header_text = req.decode("utf-8", errors="replace")
        first_line = header_text.splitlines()[0]
        if "Upgrade: websocket" in header_text or "upgrade: websocket" in header_text:
            ws_key = ""
            for line in header_text.splitlines():
                if line.lower().startswith("sec-websocket-key:"):
                    ws_key = line.split(":", 1)[1].strip()
                    break
            accept = base64.b64encode(hashlib.sha1((ws_key + GUID).encode()).digest()).decode()
            conn.sendall(
                (
                    "HTTP/1.1 101 Switching Protocols\r\n"
                    "Upgrade: websocket\r\n"
                    "Connection: Upgrade\r\n"
                    f"Sec-WebSocket-Accept: {accept}\r\n\r\n"
                ).encode()
            )
            while True:
                frame = read_ws_frame(conn)
                if frame is None:
                    break
                opcode, payload = frame
                if opcode == 0x8:
                    break
                if opcode != 0x1:
                    continue
                msg = json.loads(payload.decode("utf-8"))
                req_id = msg.get("id")
                method = msg.get("method", "")
                params = msg.get("params", {})
                with log_lock:
                    with open(os.path.join(storage_dir, "cdp_calls.jsonl"), "a") as lf:
                        lf.write(json.dumps({"run": run_id, "method": method, "params": params}) + "\n")
                if run_id == 1 and method == "Runtime.evaluate" and params.get("expression") == "HANG_ON_FIRST_RUN":
                    time.sleep(30)
                    return
                resp = json.dumps({"id": req_id, "result": {"run": run_id, "method": method}})
                send_ws_text(conn, resp)
        elif "/json/list" in first_line:
            body = json.dumps([{
                "id": "page-1",
                "type": "page",
                "webSocketDebuggerUrl": f"ws://127.0.0.1:{port}/devtools/page/1"
            }])
            conn.sendall(
                f"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n{body}".encode()
            )
        else:
            body = "{}"
            conn.sendall(
                f"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n{body}".encode()
            )
    except Exception:
        pass
    finally:
        try:
            conn.close()
        except Exception:
            pass

def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd")
    serve = sub.add_parser("serve")
    serve.add_argument("--host", required=True)
    serve.add_argument("--port", type=int, required=True)
    serve.add_argument("--stealth", action="store_true")
    serve.add_argument("--storage-dir", required=True)
    args = parser.parse_args()

    os.makedirs(args.storage_dir, exist_ok=True)
    run_file = os.path.join(args.storage_dir, "runs.txt")
    with open(run_file, "a+") as f:
        f.seek(0)
        lines = f.read().splitlines()
        run_id = len(lines) + 1
        f.write(json.dumps({"run": run_id, "pid": os.getpid(), "stealth": args.stealth, "storage_dir": args.storage_dir}) + "\n")

    log_lock = threading.Lock()
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((args.host, args.port))
    srv.listen(16)
    while True:
        conn, _ = srv.accept()
        threading.Thread(target=handle_conn, args=(conn, args.port, args.storage_dir, run_id, log_lock), daemon=True).start()

if __name__ == "__main__":
    main()
"####;

    std::fs::write(&script_path, mock_py).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let (event_tx, mut event_rx) = broadcast::channel::<BotBrowserEvent>(16);
    let config = SidecarConfig::new(&script_path, &storage_dir)
        .with_command_timeout(Duration::from_millis(300))
        .with_startup_timeout(Duration::from_secs(5))
        .with_heartbeat_interval(Duration::from_secs(30));

    let sidecar = Arc::new(
        ObscuraSidecar::start(config, "sess-watchdog-1", Some(event_tx))
            .await
            .unwrap(),
    );
    let transport = Arc::new(ObscuraSidecarTransport::new(Arc::clone(&sidecar)));
    let bridge = CdpBridge::new(transport);

    // First navigate succeeds and records `last_url`.
    bridge
        .navigate("https://app.example.com/checkout?step=2")
        .await
        .unwrap();
    assert_eq!(
        sidecar.last_url().await.as_deref(),
        Some("https://app.example.com/checkout?step=2")
    );

    // Trigger hung command on run #1 -> watchdog kills & restarts within short timeout.
    let started = Instant::now();
    let err = bridge.evaluate_script("HANG_ON_FIRST_RUN").await.unwrap_err();
    let elapsed = started.elapsed();
    assert!(
        matches!(err, BotBrowserError::SidecarTimeout { .. }),
        "expected SidecarTimeout, got {err:?}"
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "watchdog took too long: {elapsed:?}"
    );
    assert_eq!(sidecar.restart_count(), 1);

    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match event {
        BotBrowserEvent::SidecarRestarted {
            session_id,
            restart_count,
            restored_url,
        } => {
            assert_eq!(session_id, "sess-watchdog-1");
            assert_eq!(restart_count, 1);
            assert_eq!(
                restored_url.as_deref(),
                Some("https://app.example.com/checkout?step=2")
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }

    // Verify subsequent CDP command runs cleanly on run #2.
    let res = bridge.evaluate_script("HANG_ON_FIRST_RUN").await.unwrap();
    assert_eq!(res.get("run").and_then(|v| v.as_i64()), Some(2));

    // Verify both runs shared the SAME --storage-dir and --stealth, and run #2 re-navigated to last_url.
    let runs_raw = std::fs::read_to_string(storage_dir.join("runs.txt")).unwrap();
    let runs: Vec<Value> = runs_raw
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["stealth"], true);
    assert_eq!(runs[1]["stealth"], true);
    assert_eq!(runs[0]["storage_dir"], runs[1]["storage_dir"]);

    let cdp_log_raw = std::fs::read_to_string(storage_dir.join("cdp_calls.jsonl")).unwrap();
    let run2_navs: Vec<Value> = cdp_log_raw
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["run"] == 2 && v["method"] == "Page.navigate")
        .collect();
    assert_eq!(run2_navs.len(), 1);
    assert_eq!(
        run2_navs[0]["params"]["url"],
        "https://app.example.com/checkout?step=2"
    );

    sidecar.shutdown().await.unwrap();
    assert!(!sidecar.is_running().await);
}
