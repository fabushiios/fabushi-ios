use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures::{SinkExt, StreamExt};
use image::{GenericImageView, ImageFormat};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use zeroize::Zeroizing;

use crate::sidecar::ObscuraSidecar;
use crate::types::{
    BotBrowserError, BrowserCookie, BrowserSnapshot, DecryptedSecret, DomNodeSnapshot, ElementRect,
    NormalizedClickPoint, NormalizedOrigin,
};

const DEFAULT_CDP_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const SENSITIVE_FIELD_MARKERS: &[&str] = &[
    "password",
    "passwd",
    "passcode",
    "pwd",
    "one-time-code",
    "totp",
    "secret",
    "api_key",
    "apikey",
    "auth_token",
    "access_token",
    "ct0",
];

#[async_trait]
pub trait CdpTransport: Send + Sync {
    async fn send_command(&self, method: &str, params: Value) -> Result<Value, BotBrowserError>;
    async fn current_endpoint(&self) -> Result<String, BotBrowserError>;
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct WsConn {
    stream: WsStream,
    session_id: Option<String>,
}

struct ResolvedWsTarget {
    ws_url: String,
    needs_target_attach: bool,
}

pub struct WsCdpTransport {
    endpoint: String,
    http_client: reqwest::Client,
    command_timeout: Duration,
    next_id: AtomicU64,
    ws_conn: Mutex<Option<WsConn>>,
}

impl WsCdpTransport {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self::with_timeout(endpoint, DEFAULT_CDP_COMMAND_TIMEOUT)
    }

    pub fn with_timeout(endpoint: impl Into<String>, command_timeout: Duration) -> Self {
        let http_client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self::with_client(endpoint, http_client, command_timeout)
    }

    pub fn with_client(
        endpoint: impl Into<String>,
        http_client: reqwest::Client,
        command_timeout: Duration,
    ) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            http_client,
            command_timeout,
            next_id: AtomicU64::new(1),
            ws_conn: Mutex::new(None),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    async fn resolve_ws_target(&self) -> Result<ResolvedWsTarget, BotBrowserError> {
        if self.endpoint.starts_with("ws://") || self.endpoint.starts_with("wss://") {
            let needs_target_attach = self.endpoint.contains("/devtools/browser");
            return Ok(ResolvedWsTarget {
                ws_url: self.endpoint.clone(),
                needs_target_attach,
            });
        }

        let discovery_timeout = self.command_timeout.min(Duration::from_secs(3));
        let version_url = format!("{}/json/version", self.endpoint);
        if let Ok(version_resp) = self
            .http_client
            .get(&version_url)
            .timeout(discovery_timeout)
            .send()
            .await
        {
            if version_resp.status().is_success() {
                if let Ok(body) = version_resp.json::<Value>().await {
                    if let Some(ws_url) = body
                        .as_object()
                        .and_then(|obj| obj.get("webSocketDebuggerUrl"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        return Ok(ResolvedWsTarget {
                            ws_url: ws_url.to_string(),
                            needs_target_attach: ws_url.contains("/devtools/browser"),
                        });
                    }
                }
            }
        }

        let list_url = format!("{}/json/list", self.endpoint);
        let list_resp = self
            .http_client
            .get(&list_url)
            .timeout(discovery_timeout)
            .send()
            .await
            .map_err(|e| {
                BotBrowserError::Io(format!("CDP discovery request failed at {list_url}: {e}"))
            })?;

        if list_resp.status().is_success() {
            let targets: Value = list_resp.json().await.map_err(|e| {
                BotBrowserError::Io(format!("failed to parse CDP /json/list JSON: {e}"))
            })?;
            if let Some(ws_url) = extract_ws_url_from_list(&targets) {
                return Ok(ResolvedWsTarget {
                    ws_url,
                    needs_target_attach: false,
                });
            }
        }

        Err(BotBrowserError::Cdp(format!(
            "no page target with webSocketDebuggerUrl found at {}",
            self.endpoint
        )))
    }

    async fn connect_ws(&self) -> Result<WsConn, BotBrowserError> {
        let resolved = self.resolve_ws_target().await?;
        let (mut stream, _) =
            tokio::time::timeout(self.command_timeout, connect_async(&resolved.ws_url))
                .await
                .map_err(|_| BotBrowserError::SidecarTimeout {
                    method: "ws_connect".to_string(),
                    timeout_ms: self.command_timeout.as_millis() as u64,
                })?
                .map_err(|e| {
                    BotBrowserError::Io(format!(
                        "CDP WebSocket connect failed at {}: {e}",
                        resolved.ws_url
                    ))
                })?;

        let session_id = if resolved.needs_target_attach {
            Some(
                attach_or_create_obscura_session(&mut stream, &self.next_id, self.command_timeout)
                    .await?,
            )
        } else {
            None
        };

        Ok(WsConn { stream, session_id })
    }
}

fn extract_ws_url_from_list(targets: &Value) -> Option<String> {
    let arr = targets.as_array()?;
    let preferred = arr
        .iter()
        .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        .or_else(|| arr.first())?;
    preferred
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

async fn attach_or_create_obscura_session(
    stream: &mut WsStream,
    next_id: &AtomicU64,
    command_timeout: Duration,
) -> Result<String, BotBrowserError> {
    let targets = send_raw_cdp_on_stream(
        stream,
        next_id,
        command_timeout,
        "Target.getTargets",
        json!({}),
        None,
    )
    .await?;

    let existing_target_id = targets
        .get("targetInfos")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
                .or_else(|| arr.first())
        })
        .and_then(|t| t.get("targetId"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    let target_id = match existing_target_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            let created = send_raw_cdp_on_stream(
                stream,
                next_id,
                command_timeout,
                "Target.createTarget",
                json!({ "url": "about:blank" }),
                None,
            )
            .await?;
            created
                .get("targetId")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .ok_or_else(|| {
                    BotBrowserError::Cdp("Target.createTarget missing targetId".to_string())
                })?
        }
    };

    let attached = send_raw_cdp_on_stream(
        stream,
        next_id,
        command_timeout,
        "Target.attachToTarget",
        json!({ "targetId": target_id, "flatten": true }),
        None,
    )
    .await?;

    attached
        .get("sessionId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| BotBrowserError::Cdp("Target.attachToTarget missing sessionId".to_string()))
}

async fn send_raw_cdp_on_stream(
    stream: &mut WsStream,
    next_id: &AtomicU64,
    command_timeout: Duration,
    method: &str,
    params: Value,
    session_id: Option<&str>,
) -> Result<Value, BotBrowserError> {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let mut payload_obj = json!({
        "id": id,
        "method": method,
        "params": params,
    });
    if let (Some(sid), Some(map)) = (session_id, payload_obj.as_object_mut()) {
        if !method.starts_with("Target.") {
            map.insert("sessionId".to_string(), Value::String(sid.to_string()));
        }
    }
    let payload = payload_obj.to_string();

    stream.send(Message::Text(payload)).await.map_err(|err| {
        BotBrowserError::Io(format!("CDP WebSocket send failed for '{method}': {err}"))
    })?;

    let timeout_ms = command_timeout.as_millis() as u64;
    let read_res = tokio::time::timeout(command_timeout, async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(txt))) => {
                    if let Some(parsed) = parse_cdp_response_frame(txt.as_bytes(), id, method)? {
                        return Ok(parsed);
                    }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if let Some(parsed) = parse_cdp_response_frame(&bytes, id, method)? {
                        return Ok(parsed);
                    }
                }
                Some(Ok(Message::Ping(data))) => {
                    stream.send(Message::Pong(data)).await.map_err(|e| {
                        BotBrowserError::Io(format!("CDP WebSocket pong send failed: {e}"))
                    })?;
                }
                Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                Some(Ok(Message::Close(_))) => {
                    return Err(BotBrowserError::Io(format!(
                        "CDP WebSocket closed while waiting for '{method}'"
                    )));
                }
                Some(Err(e)) => {
                    return Err(BotBrowserError::Io(format!(
                        "CDP WebSocket read error for '{method}': {e}"
                    )));
                }
                None => {
                    return Err(BotBrowserError::Io(format!(
                        "CDP WebSocket EOF while waiting for '{method}'"
                    )));
                }
            }
        }
    })
    .await;

    match read_res {
        Ok(res) => res,
        Err(_) => Err(BotBrowserError::SidecarTimeout {
            method: method.to_string(),
            timeout_ms,
        }),
    }
}

#[async_trait]
impl CdpTransport for WsCdpTransport {
    async fn send_command(&self, method: &str, params: Value) -> Result<Value, BotBrowserError> {
        let mut guard = self.ws_conn.lock().await;

        // Reconnect once if a previously pooled WebSocket closed while idle, or lazy-attach if
        // a direct `/devtools/page/...` endpoint reports missing session page.
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(self.connect_ws().await?);
            }

            let conn = guard.as_mut().expect("ws_conn just initialized");
            let session_id = conn.session_id.clone();
            let res = send_raw_cdp_on_stream(
                &mut conn.stream,
                &self.next_id,
                self.command_timeout,
                method,
                params.clone(),
                session_id.as_deref(),
            )
            .await;

            match res {
                Ok(val) => return Ok(val),
                Err(BotBrowserError::Cdp(msg))
                    if attempt == 0 && conn.session_id.is_none() && msg.contains("No page") =>
                {
                    let sid = attach_or_create_obscura_session(
                        &mut conn.stream,
                        &self.next_id,
                        self.command_timeout,
                    )
                    .await?;
                    conn.session_id = Some(sid);
                }
                Err(err @ BotBrowserError::Io(_)) | Err(err @ BotBrowserError::SidecarTimeout { .. }) => {
                    *guard = None;
                    if attempt == 1 || matches!(err, BotBrowserError::SidecarTimeout { .. }) {
                        return Err(err);
                    }
                }
                Err(err) => return Err(err),
            }
        }

        Err(BotBrowserError::Io(format!(
            "CDP WebSocket send failed after reconnect for '{method}'"
        )))
    }

    async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
        Ok(self.endpoint.clone())
    }
}

fn parse_cdp_response_frame(
    raw: &[u8],
    expected_id: u64,
    method: &str,
) -> Result<Option<Value>, BotBrowserError> {
    let Ok(msg) = serde_json::from_slice::<Value>(raw) else {
        return Ok(None);
    };
    if msg.get("id").and_then(|v| v.as_u64()) != Some(expected_id) {
        return Ok(None);
    }
    if let Some(err) = msg.get("error").filter(|v| !v.is_null()) {
        return Err(BotBrowserError::Cdp(format!(
            "CDP error in '{method}': {err}"
        )));
    }
    Ok(Some(msg.get("result").cloned().unwrap_or_else(|| json!({}))))
}

pub struct ObscuraSidecarTransport {
    sidecar: Arc<ObscuraSidecar>,
}

impl ObscuraSidecarTransport {
    pub fn new(sidecar: Arc<ObscuraSidecar>) -> Self {
        Self { sidecar }
    }

    pub fn sidecar(&self) -> &Arc<ObscuraSidecar> {
        &self.sidecar
    }
}

#[async_trait]
impl CdpTransport for ObscuraSidecarTransport {
    async fn send_command(&self, method: &str, params: Value) -> Result<Value, BotBrowserError> {
        self.sidecar.send_cdp_with_watchdog(method, params).await
    }

    async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
        self.sidecar.current_endpoint().await
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LoginFormSelectors {
    pub username_selector: String,
    pub password_selector: String,
    pub totp_selector: Option<String>,
    pub submit_selector: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FillSubmitResult {
    pub origin: String,
    pub submitted: bool,
    pub used_totp: bool,
}

struct AtomicFillGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> AtomicFillGuard<'a> {
    fn new(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::Release);
        Self { flag }
    }
}

impl Drop for AtomicFillGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

pub struct CdpBridge {
    transport: Arc<dyn CdpTransport>,
    secret_fill_lock: Arc<RwLock<()>>,
    atomic_fill_active: Arc<AtomicBool>,
}

impl CdpBridge {
    pub fn new(transport: Arc<dyn CdpTransport>) -> Self {
        Self {
            transport,
            secret_fill_lock: Arc::new(RwLock::new(())),
            atomic_fill_active: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn transport(&self) -> &Arc<dyn CdpTransport> {
        &self.transport
    }

    pub fn is_atomic_fill_active(&self) -> bool {
        self.atomic_fill_active.load(Ordering::Acquire)
    }

    fn acquire_public_guard(
        &self,
    ) -> Result<tokio::sync::RwLockReadGuard<'_, ()>, BotBrowserError> {
        if self.atomic_fill_active.load(Ordering::Acquire) {
            return Err(BotBrowserError::SessionLockedForSecretFill);
        }
        self.secret_fill_lock
            .try_read()
            .map_err(|_| BotBrowserError::SessionLockedForSecretFill)
    }

    pub async fn navigate(&self, url: &str) -> Result<Value, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        let res = self
            .transport
            .send_command("Page.navigate", json!({ "url": url }))
            .await?;
        if let Some(err_text) = res
            .get("errorText")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let is_download = res
                .get("isDownload")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !(is_download && err_text == "net::ERR_ABORTED") {
                return Err(BotBrowserError::Cdp(format!(
                    "Page.navigate failed: {err_text}"
                )));
            }
        }
        Ok(res)
    }

    pub async fn click_selector(&self, selector: &str) -> Result<Value, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        self.click_selector_internal(selector).await
    }

    pub async fn click_coords(&self, x: f64, y: f64) -> Result<Value, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        self.dispatch_mouse_click(x, y).await
    }

    pub async fn insert_text(&self, text: &str) -> Result<Value, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        self.transport
            .send_command("Input.insertText", json!({ "text": text }))
            .await
    }

    pub async fn evaluate_script(&self, expression: &str) -> Result<Value, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        self.transport
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await
    }

    pub async fn send_guarded_command(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        self.transport.send_command(method, params).await
    }

    pub async fn capture_screenshot_png(&self, full_page: bool) -> Result<Vec<u8>, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        let res = self
            .transport
            .send_command(
                "Page.captureScreenshot",
                json!({ "format": "png", "captureBeyondViewport": full_page }),
            )
            .await?;
        decode_screenshot_png(&res)
    }

    pub async fn inspect_true_origin(
        &self,
        target_selector: Option<&str>,
    ) -> Result<(NormalizedOrigin, Option<NormalizedOrigin>, String), BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        self.inspect_true_origin_internal(target_selector).await
    }

    pub async fn capture_redacted_snapshot(
        &self,
        root_selector: Option<&str>,
    ) -> Result<BrowserSnapshot, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;

        let root_json = match root_selector {
            Some(sel) => serde_json::to_string(sel)
                .map_err(|e| BotBrowserError::Cdp(format!("invalid root selector: {e}")))?,
            None => "null".to_string(),
        };

        let script_template = r####"(() => {
  const rootSel = __ROOT_SEL__;
  const root = rootSel ? document.querySelector(rootSel) : (document.body || document.documentElement);
  const pageUrl = String(window.location.href || "");
  const pageOrigin = String(window.location.origin || "");
  const title = String(document.title || "");

  function cssPath(el) {
    if (el.id) {
      try { return "#" + CSS.escape(el.id); } catch (_) { return "#" + el.id; }
    }
    const tid = el.getAttribute("data-testid");
    if (tid) {
      return '[data-testid="' + String(tid).replace(/"/g, '\\"') + '"]';
    }
    const tag = (el.tagName || "div").toLowerCase();
    const nm = el.getAttribute("name");
    if (nm) {
      return tag + '[name="' + String(nm).replace(/"/g, '\\"') + '"]';
    }
    const parts = [];
    let cur = el;
    while (cur && cur.nodeType === 1 && parts.length < 5) {
      const t = cur.tagName.toLowerCase();
      let idx = 1;
      let sib = cur.previousElementSibling;
      while (sib) {
        if (sib.tagName === cur.tagName) idx++;
        sib = sib.previousElementSibling;
      }
      parts.unshift(t + ":nth-of-type(" + idx + ")");
      cur = cur.parentElement;
    }
    return parts.join(" > ");
  }

  function isSensitiveField(el) {
    const inputType = String(el.getAttribute("type") || el.type || "").toLowerCase();
    const ac = String(el.getAttribute("autocomplete") || "").toLowerCase();
    const nm = String(el.getAttribute("name") || "").toLowerCase();
    const eid = String(el.getAttribute("id") || "").toLowerCase();
    const aria = String(el.getAttribute("aria-label") || "").toLowerCase();
    const ph = String(el.getAttribute("placeholder") || "").toLowerCase();
    const tid = String(el.getAttribute("data-testid") || "").toLowerCase();
    const hay = [ac, nm, eid, aria, ph, tid].join(" ");
    return (
      inputType === "password" ||
      ac.includes("password") ||
      ac === "one-time-code" ||
      hay.includes("password") ||
      hay.includes("passwd") ||
      hay.includes("passcode") ||
      hay.includes("pwd") ||
      hay.includes("totp") ||
      hay.includes("one-time-code") ||
      hay.includes("secret") ||
      hay.includes("api_key") ||
      hay.includes("apikey") ||
      hay.includes("auth_token") ||
      hay.includes("access_token") ||
      hay.includes("ct0") ||
      /(?:^|[^a-z0-9])(otp|token|pin)(?:[^a-z0-9]|$)/.test(hay)
    );
  }

  let textExcerpt = "";
  if (root) {
    if (isSensitiveField(root)) {
      textExcerpt = "[REDACTED]";
    } else {
      const hasTreeWalker =
        typeof document.createTreeWalker === "function" &&
        typeof NodeFilter !== "undefined";
      const minFallbackLen = hasTreeWalker ? 4 : 1;
      const restorations = [];
      const leakedStrings = [];
      try {
        const sensitiveTargets = Array.from(
          root.querySelectorAll(
            'textarea, [contenteditable="true"], [type="password"], [autocomplete], [name], [id], [aria-label], [placeholder], [data-testid]'
          )
        );
        for (const el of sensitiveTargets) {
          if (!isSensitiveField(el)) continue;
          const rawInner = typeof el.innerText === "string" ? el.innerText.trim() : "";
          const rawContent = typeof el.textContent === "string" ? el.textContent.trim() : "";
          if (rawInner.length >= minFallbackLen) leakedStrings.push(rawInner);
          if (rawContent.length >= minFallbackLen && rawContent !== rawInner) {
            leakedStrings.push(rawContent);
          }
          if (hasTreeWalker) {
            const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, null);
            let tNode;
            let replacedFirst = false;
            while ((tNode = walker.nextNode())) {
              const orig = tNode.nodeValue;
              if (orig && orig.trim().length > 0) {
                restorations.push({ node: tNode, value: orig });
                tNode.nodeValue = replacedFirst ? "" : "[REDACTED]";
                replacedFirst = true;
              }
            }
          }
        }
        textExcerpt = typeof root.innerText === "string" ? root.innerText : "";
      } finally {
        for (let i = restorations.length - 1; i >= 0; i--) {
          restorations[i].node.nodeValue = restorations[i].value;
        }
      }
      for (const s of leakedStrings) {
        if (s && textExcerpt.includes(s)) {
          textExcerpt = textExcerpt.split(s).join("[REDACTED]");
        }
      }
      textExcerpt = textExcerpt.slice(0, 12000);
    }
  }

  const candidates = root
    ? Array.from(root.querySelectorAll('a, button, input, select, textarea, [data-testid], [role="button"], [role="link"], [role="textbox"], [role="tab"], [role="menuitem"], [role="checkbox"], [role="switch"], [contenteditable="true"]')).slice(0, 200)
    : [];
  let redactedCount = 0;
  const interactive_elements = candidates.map((el, idx) => {
    const sensitive = isSensitiveField(el);
    if (sensitive) redactedCount += 1;
    const rectRaw = typeof el.getBoundingClientRect === "function" ? el.getBoundingClientRect() : null;
    const dpr = Number(window.devicePixelRatio || 1);
    const rect = rectRaw ? {
      x: Number(rectRaw.x || 0),
      y: Number(rectRaw.y || 0),
      width: Number(rectRaw.width || 0),
      height: Number(rectRaw.height || 0),
      device_pixel_ratio: dpr
    } : null;
    const frameOrigin = (el.ownerDocument && el.ownerDocument.location && el.ownerDocument.location.origin) || pageOrigin || null;
    const rawVal = typeof el.value === "string" ? el.value : null;
    const value = sensitive ? "[REDACTED]" : (rawVal !== null ? rawVal.slice(0, 500) : null);
    const text = sensitive
      ? "[REDACTED]"
      : String(el.innerText || el.textContent || "").trim().slice(0, 200);
    return {
      ref_id: "el_" + (idx + 1),
      tag: String(el.tagName || "").toLowerCase(),
      input_type: el.getAttribute("type") || (typeof el.type === "string" ? el.type : null),
      name: el.getAttribute("name"),
      id: el.getAttribute("id"),
      aria_label: el.getAttribute("aria-label"),
      placeholder: el.getAttribute("placeholder"),
      test_id: el.getAttribute("data-testid"),
      role: el.getAttribute("role"),
      autocomplete: el.getAttribute("autocomplete"),
      text,
      value,
      is_password: sensitive,
      selector: cssPath(el),
      frame_origin: frameOrigin,
      rect
    };
  });

  return JSON.stringify({
    url: pageUrl,
    origin: pageOrigin,
    title,
    text_excerpt: textExcerpt,
    interactive_elements,
    redacted_field_count: redactedCount
  });
})()"####;
        let script = script_template.replace("__ROOT_SEL__", &root_json);

        let res = self
            .transport
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": script, "returnByValue": true }),
            )
            .await?;

        let raw: RawBrowserSnapshot = parse_eval_json(&res, "capture_redacted_snapshot")?;
        Ok(sanitize_snapshot(raw))
    }

    pub async fn atomic_fill_and_submit(
        &self,
        expected_origin: &NormalizedOrigin,
        secret: &DecryptedSecret,
        totp_code: Option<&Zeroizing<String>>,
        selectors: &LoginFormSelectors,
    ) -> Result<FillSubmitResult, BotBrowserError> {
        let _write_guard = self.secret_fill_lock.write().await;
        let _active_guard = AtomicFillGuard::new(&self.atomic_fill_active);

        let (actual_page_origin, actual_frame_origin, _href) = self
            .inspect_true_origin_internal(Some(&selectors.password_selector))
            .await?;

        if actual_page_origin != *expected_origin {
            return Err(BotBrowserError::OriginMismatch {
                expected: expected_origin.as_str().to_string(),
                actual: actual_page_origin.as_str().to_string(),
            });
        }

        if let Some(frame_origin) = actual_frame_origin {
            if frame_origin != *expected_origin {
                return Err(BotBrowserError::FrameOriginMismatch {
                    expected: expected_origin.as_str().to_string(),
                    actual: frame_origin.as_str().to_string(),
                });
            }
        }

        self.fill_input_via_cdp(&selectors.username_selector, secret.username.as_str())
            .await?;
        self.fill_input_via_cdp(&selectors.password_selector, secret.password.as_str())
            .await?;

        let mut used_totp = false;
        if let (Some(totp), Some(totp_selector)) = (totp_code, selectors.totp_selector.as_deref()) {
            self.fill_input_via_cdp(totp_selector, totp.as_str())
                .await?;
            used_totp = true;
        }

        if let Some(submit_selector) = selectors.submit_selector.as_deref() {
            self.click_selector_internal(submit_selector).await?;
        } else {
            self.dispatch_enter_key().await?;
        }

        Ok(FillSubmitResult {
            origin: actual_page_origin.as_str().to_string(),
            submitted: true,
            used_totp,
        })
    }

    pub async fn capture_element_png(
        &self,
        selector: &str,
    ) -> Result<(Vec<u8>, ElementRect), BotBrowserError> {
        let _guard = self.acquire_public_guard()?;

        let sel_json = serde_json::to_string(selector)
            .map_err(|e| BotBrowserError::Cdp(format!("invalid selector: {e}")))?;
        let script = format!(
            r#"(() => {{
  const el = document.querySelector({sel_json});
  if (!el) return JSON.stringify({{ found: false }});
  if (typeof el.scrollIntoView === "function") {{
    try {{ el.scrollIntoView({{ block: "center", inline: "center" }}); }} catch (_) {{}}
  }}
  const r = el.getBoundingClientRect();
  return JSON.stringify({{
    found: true,
    x: Number(r.x || 0),
    y: Number(r.y || 0),
    width: Number(r.width || 0),
    height: Number(r.height || 0),
    device_pixel_ratio: Number(window.devicePixelRatio || 1),
    viewport_width: Number(window.innerWidth || 0),
    viewport_height: Number(window.innerHeight || 0)
  }});
}})()"#
        );

        let eval_res = self
            .transport
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": script, "returnByValue": true }),
            )
            .await?;
        let info: ElementBoundsPayload = parse_eval_json(&eval_res, "capture_element_png")?;
        if !info.found || info.width <= 0.0 || info.height <= 0.0 {
            return Err(BotBrowserError::Cdp(format!(
                "element '{selector}' not found or has zero bounds"
            )));
        }

        let dpr = if info.device_pixel_ratio.is_finite() && info.device_pixel_ratio > 0.0 {
            info.device_pixel_ratio
        } else {
            1.0
        };
        let rect = ElementRect {
            x: info.x,
            y: info.y,
            width: info.width,
            height: info.height,
            device_pixel_ratio: dpr,
        };

        let shot = self
            .transport
            .send_command(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "clip": {
                        "x": rect.x.max(0.0),
                        "y": rect.y.max(0.0),
                        "width": rect.width.max(1.0),
                        "height": rect.height.max(1.0),
                        "scale": 1
                    }
                }),
            )
            .await?;

        let raw_png = decode_screenshot_png(&shot)?;
        let cropped_png = crop_element_png_if_needed(
            raw_png,
            &rect,
            info.viewport_width,
            info.viewport_height,
        )?;
        Ok((cropped_png, rect))
    }

    pub async fn dispatch_element_relative_click(
        &self,
        rect: &ElementRect,
        point: NormalizedClickPoint,
    ) -> Result<(f64, f64), BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        let clamped = point.clamped();
        let page_x = rect.x + rect.width * clamped.norm_x;
        let page_y = rect.y + rect.height * clamped.norm_y;
        self.dispatch_mouse_click(page_x, page_y).await?;
        Ok((page_x, page_y))
    }

    pub async fn get_cookies(&self) -> Result<Vec<Value>, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        let res = match self
            .transport
            .send_command("Network.getCookies", json!({}))
            .await
        {
            Ok(v) => v,
            Err(BotBrowserError::Cdp(_)) => {
                self.transport
                    .send_command("Storage.getCookies", json!({}))
                    .await?
            }
            Err(e) => return Err(e),
        };
        Ok(res
            .get("cookies")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub async fn set_cookies(&self, cookies: &[Value]) -> Result<usize, BotBrowserError> {
        let _guard = self.acquire_public_guard()?;
        if cookies.is_empty() {
            return Ok(0);
        }
        match self
            .transport
            .send_command("Network.setCookies", json!({ "cookies": cookies }))
            .await
        {
            Ok(_) => Ok(cookies.len()),
            Err(BotBrowserError::Cdp(_)) => {
                if self
                    .transport
                    .send_command("Storage.setCookies", json!({ "cookies": cookies }))
                    .await
                    .is_ok()
                {
                    return Ok(cookies.len());
                }
                let mut count = 0usize;
                for cookie in cookies {
                    let res = self
                        .transport
                        .send_command("Network.setCookie", cookie.clone())
                        .await?;
                    if res.get("success").and_then(|v| v.as_bool()) == Some(false) {
                        return Err(BotBrowserError::Cdp(
                            "Network.setCookie rejected cookie".to_string(),
                        ));
                    }
                    count += 1;
                }
                Ok(count)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn get_browser_cookies(
        &self,
        domain_filter: Option<&str>,
    ) -> Result<Vec<BrowserCookie>, BotBrowserError> {
        let raw = self.get_cookies().await?;
        let mut out = Vec::with_capacity(raw.len());
        for item in raw {
            let Ok(cookie) = serde_json::from_value::<BrowserCookie>(item) else {
                continue;
            };
            if let Some(filter) = domain_filter.map(str::trim).filter(|s| !s.is_empty()) {
                let norm_filter = filter.trim_start_matches('.').to_ascii_lowercase();
                let norm_domain = cookie
                    .domain
                    .trim()
                    .trim_start_matches('.')
                    .to_ascii_lowercase();
                if norm_domain != norm_filter && !norm_domain.ends_with(&format!(".{norm_filter}"))
                {
                    continue;
                }
            }
            out.push(cookie);
        }
        Ok(out)
    }

    pub async fn set_browser_cookies(
        &self,
        cookies: &[BrowserCookie],
    ) -> Result<usize, BotBrowserError> {
        let params: Vec<Value> = cookies.iter().map(|c| c.to_cdp_cookie_param()).collect();
        self.set_cookies(&params).await
    }

    async fn inspect_true_origin_internal(
        &self,
        target_selector: Option<&str>,
    ) -> Result<(NormalizedOrigin, Option<NormalizedOrigin>, String), BotBrowserError> {
        let sel_json = match target_selector {
            Some(sel) => serde_json::to_string(sel)
                .map_err(|e| BotBrowserError::Cdp(format!("invalid selector: {e}")))?,
            None => "null".to_string(),
        };

        let script = format!(
            r#"(() => {{
  const pageOrigin = String(window.location.origin || "");
  const href = String(window.location.href || "");
  const sel = {sel_json};
  let frameOrigin = null;
  if (sel !== null) {{
    const el = document.querySelector(sel);
    if (el) {{
      if (el.tagName && el.tagName.toLowerCase() === "iframe") {{
        try {{
          frameOrigin = el.contentWindow.location.origin;
        }} catch (_) {{
          try {{ frameOrigin = new URL(el.src, href).origin; }} catch (__) {{ frameOrigin = "null"; }}
        }}
      }} else {{
        frameOrigin = (el.ownerDocument && el.ownerDocument.location && el.ownerDocument.location.origin) || pageOrigin;
      }}
    }} else {{
      const frames = document.querySelectorAll("iframe");
      for (const f of frames) {{
        try {{
          const doc = f.contentDocument;
          if (doc && doc.querySelector(sel)) {{
            frameOrigin = (doc.location && doc.location.origin) || new URL(f.src, href).origin;
            break;
          }}
        }} catch (_) {{
          try {{ frameOrigin = new URL(f.src, href).origin; }} catch (__) {{}}
        }}
      }}
    }}
  }}
  return JSON.stringify({{ page_origin: pageOrigin, frame_origin: frameOrigin, href }});
}})()"#
        );

        let res = self
            .transport
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": script, "returnByValue": true }),
            )
            .await?;

        let payload: InspectOriginPayload = parse_eval_json(&res, "inspect_true_origin")?;
        let page_origin_src = if !payload.page_origin.trim().is_empty() {
            payload.page_origin.as_str()
        } else {
            payload.href.as_str()
        };
        let page_origin = NormalizedOrigin::parse(page_origin_src)?;

        let frame_origin = match payload.frame_origin {
            Some(raw_frame) if !raw_frame.trim().is_empty() => {
                if raw_frame.trim() == "null" {
                    return Err(BotBrowserError::FrameOriginMismatch {
                        expected: page_origin.as_str().to_string(),
                        actual: "null".to_string(),
                    });
                }
                Some(NormalizedOrigin::parse(&raw_frame)?)
            }
            _ => None,
        };

        let href = if payload.href.is_empty() {
            page_origin.as_str().to_string()
        } else {
            payload.href
        };
        Ok((page_origin, frame_origin, href))
    }

    async fn click_selector_internal(&self, selector: &str) -> Result<Value, BotBrowserError> {
        let sel_json = serde_json::to_string(selector)
            .map_err(|e| BotBrowserError::Cdp(format!("invalid selector: {e}")))?;
        let expr = format!(
            "(() => {{ const el = document.querySelector({sel_json}); if (!el) return 'NOT_FOUND'; el.click(); return 'CLICKED'; }})()"
        );
        let res = self
            .transport
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": expr, "returnByValue": true }),
            )
            .await?;
        let val = extract_eval_value(&res).and_then(|v| v.as_str().map(str::to_string));
        if val.as_deref() == Some("NOT_FOUND") {
            return Err(BotBrowserError::Cdp(format!(
                "element not found for selector '{selector}'"
            )));
        }
        Ok(res)
    }

    async fn dispatch_mouse_click(&self, x: f64, y: f64) -> Result<Value, BotBrowserError> {
        self.transport
            .send_command(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": x, "y": y, "button": "none" }),
            )
            .await?;
        self.transport
            .send_command(
                "Input.dispatchMouseEvent",
                json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
            )
            .await?;
        self.transport
            .send_command(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
            )
            .await
    }

    async fn fill_input_via_cdp(
        &self,
        selector: &str,
        secret_text: &str,
    ) -> Result<(), BotBrowserError> {
        self.focus_selector_via_cdp(selector).await?;
        self.clear_focused_input().await?;
        // Secrets must ONLY travel via CDP Input.insertText, never Runtime.evaluate.
        self.transport
            .send_command("Input.insertText", json!({ "text": secret_text }))
            .await?;
        Ok(())
    }

    async fn focus_selector_via_cdp(&self, selector: &str) -> Result<(), BotBrowserError> {
        if let Ok(doc) = self
            .transport
            .send_command("DOM.getDocument", json!({ "depth": 1, "pierce": true }))
            .await
        {
            if let Some(root_node_id) = doc
                .get("root")
                .and_then(|r| r.get("nodeId"))
                .and_then(|id| id.as_i64())
                .filter(|&id| id >= 0)
            {
                if let Ok(queried) = self
                    .transport
                    .send_command(
                        "DOM.querySelector",
                        json!({ "nodeId": root_node_id, "selector": selector }),
                    )
                    .await
                {
                    if let Some(node_id) = queried
                        .get("nodeId")
                        .and_then(|id| id.as_i64())
                        .filter(|&id| id > 0)
                    {
                        if self
                            .transport
                            .send_command("DOM.focus", json!({ "nodeId": node_id }))
                            .await
                            .is_ok()
                        {
                            return Ok(());
                        }
                    }
                }
            }
        }

        // Fallback focuses only the selector without touching any credential value.
        let sel_json = serde_json::to_string(selector)
            .map_err(|e| BotBrowserError::Cdp(format!("invalid selector: {e}")))?;
        let focus_expr = format!(
            "(() => {{ const el = document.querySelector({sel_json}); if (!el) return false; el.focus(); return true; }})()"
        );
        let res = self
            .transport
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": focus_expr, "returnByValue": true }),
            )
            .await?;
        let focused = extract_eval_value(&res).and_then(|v| v.as_bool());
        if focused == Some(false) {
            return Err(BotBrowserError::Cdp(format!(
                "input selector not found: '{selector}'"
            )));
        }
        Ok(())
    }

    async fn clear_focused_input(&self) -> Result<(), BotBrowserError> {
        // Clear via secret-free activeElement value reset so prefilled fields never leak or retain
        // trailing characters when Input.insertText runs on Obscura v0.2.7.
        let clear_expr = r#"(() => {
  const el = document.activeElement;
  if (el && typeof el.value === "string" && el.value.length > 0) {
    el.value = "";
    try { el.dispatchEvent(new Event("input", { bubbles: true })); } catch (_) {}
    try { el.dispatchEvent(new Event("change", { bubbles: true })); } catch (_) {}
  }
  return true;
})()"#;
        let _ = self
            .transport
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": clear_expr, "returnByValue": true }),
            )
            .await?;
        Ok(())
    }

    async fn dispatch_enter_key(&self) -> Result<(), BotBrowserError> {
        self.transport
            .send_command(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyDown",
                    "key": "Enter",
                    "code": "Enter",
                    "text": "\r",
                    "windowsVirtualKeyCode": 13
                }),
            )
            .await?;
        self.transport
            .send_command(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyUp",
                    "key": "Enter",
                    "code": "Enter",
                    "windowsVirtualKeyCode": 13
                }),
            )
            .await?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct InspectOriginPayload {
    #[serde(default)]
    page_origin: String,
    #[serde(default)]
    frame_origin: Option<String>,
    #[serde(default)]
    href: String,
}

#[derive(Deserialize)]
struct ElementBoundsPayload {
    #[serde(default)]
    found: bool,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    height: f64,
    #[serde(default = "default_dpr")]
    device_pixel_ratio: f64,
    #[serde(default)]
    viewport_width: f64,
    #[serde(default)]
    viewport_height: f64,
}

fn default_dpr() -> f64 {
    1.0
}

#[derive(Deserialize)]
struct RawBrowserSnapshot {
    #[serde(default)]
    url: String,
    #[serde(default)]
    origin: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    text_excerpt: String,
    #[serde(default)]
    interactive_elements: Vec<RawDomNodeSnapshot>,
    #[serde(default)]
    redacted_field_count: usize,
}

#[derive(Deserialize)]
struct RawDomNodeSnapshot {
    ref_id: String,
    tag: String,
    #[serde(default)]
    input_type: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    aria_label: Option<String>,
    #[serde(default)]
    placeholder: Option<String>,
    #[serde(default)]
    test_id: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    autocomplete: Option<String>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    is_password: bool,
    selector: String,
    #[serde(default)]
    frame_origin: Option<String>,
    #[serde(default)]
    rect: Option<ElementRect>,
}

fn sanitize_snapshot(raw: RawBrowserSnapshot) -> BrowserSnapshot {
    let mut redacted_count = 0usize;
    let mut elements = Vec::with_capacity(raw.interactive_elements.len());
    let mut text_excerpt = raw.text_excerpt;

    for node in raw.interactive_elements {
        let sensitive = is_sensitive_dom_node(&node);
        if sensitive {
            let trimmed_text = node.text.trim();
            if trimmed_text.len() >= 4 && trimmed_text != "[REDACTED]" {
                text_excerpt = text_excerpt.replace(trimmed_text, "[REDACTED]");
            }
            if let Some(raw_val) = node.value.as_deref().map(str::trim) {
                if raw_val.len() >= 4 && raw_val != "[REDACTED]" {
                    text_excerpt = text_excerpt.replace(raw_val, "[REDACTED]");
                }
            }
        }
        let value = if sensitive {
            redacted_count += 1;
            Some("[REDACTED]".to_string())
        } else {
            node.value
        };
        let text = if sensitive {
            "[REDACTED]".to_string()
        } else {
            node.text
        };

        elements.push(DomNodeSnapshot {
            ref_id: node.ref_id,
            tag: node.tag,
            input_type: node.input_type,
            name: node.name,
            id: node.id,
            aria_label: node.aria_label,
            placeholder: node.placeholder,
            test_id: node.test_id,
            role: node.role,
            text,
            value,
            is_password: sensitive,
            selector: node.selector,
            frame_origin: node.frame_origin,
            rect: node.rect,
        });
    }

    let origin = NormalizedOrigin::parse(&raw.origin)
        .or_else(|_| NormalizedOrigin::parse(&raw.url))
        .map(|o| o.as_str().to_string())
        .unwrap_or(raw.origin);

    BrowserSnapshot {
        url: raw.url,
        origin,
        title: raw.title,
        text_excerpt,
        interactive_elements: elements,
        redacted_field_count: redacted_count.max(raw.redacted_field_count),
    }
}

fn is_sensitive_dom_node(node: &RawDomNodeSnapshot) -> bool {
    if node.is_password {
        return true;
    }
    if node
        .input_type
        .as_deref()
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("password"))
    {
        return true;
    }
    if node.autocomplete.as_deref().is_some_and(|ac| {
        let lower = ac.to_ascii_lowercase();
        lower.contains("password")
            || lower.contains("one-time-code")
            || contains_word(&lower, "otp")
            || contains_word(&lower, "token")
            || contains_word(&lower, "pin")
    }) {
        return true;
    }

    for candidate in [
        node.autocomplete.as_deref(),
        node.name.as_deref(),
        node.id.as_deref(),
        node.aria_label.as_deref(),
        node.placeholder.as_deref(),
        node.test_id.as_deref(),
        Some(node.selector.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        let lower = candidate.to_ascii_lowercase();
        if SENSITIVE_FIELD_MARKERS.iter().any(|m| lower.contains(m))
            || contains_word(&lower, "otp")
            || contains_word(&lower, "token")
            || contains_word(&lower, "pin")
        {
            return true;
        }
    }
    false
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    haystack
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token == needle)
}

pub fn check_eval_exception(res: &Value, op_name: &str) -> Result<(), BotBrowserError> {
    if let Some(details) = res.get("exceptionDetails").filter(|v| !v.is_null()) {
        let msg = details
            .get("exception")
            .and_then(|e| e.get("description").or_else(|| e.get("value")))
            .and_then(|v| v.as_str())
            .or_else(|| details.get("text").and_then(|v| v.as_str()))
            .unwrap_or("unknown JS exception");
        return Err(BotBrowserError::Cdp(format!("{op_name}: JS exception: {msg}")));
    }
    Ok(())
}

fn extract_eval_value(res: &Value) -> Option<&Value> {
    res.get("result")
        .and_then(|r| r.get("value"))
        .or_else(|| res.get("value"))
}

fn parse_eval_json<T: for<'de> Deserialize<'de>>(
    res: &Value,
    op_name: &str,
) -> Result<T, BotBrowserError> {
    check_eval_exception(res, op_name)?;
    let val = extract_eval_value(res)
        .ok_or_else(|| BotBrowserError::Cdp(format!("{op_name}: missing Runtime.evaluate result")))?;
    if let Some(json_str) = val.as_str() {
        serde_json::from_str::<T>(json_str)
            .map_err(|e| BotBrowserError::Cdp(format!("{op_name}: invalid JSON payload: {e}")))
    } else {
        serde_json::from_value::<T>(val.clone())
            .map_err(|e| BotBrowserError::Cdp(format!("{op_name}: unexpected result shape: {e}")))
    }
}

fn decode_screenshot_png(res: &Value) -> Result<Vec<u8>, BotBrowserError> {
    let b64 = res
        .get("data")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            BotBrowserError::Cdp("Page.captureScreenshot missing base64 'data'".to_string())
        })?;
    BASE64_STANDARD
        .decode(b64)
        .map_err(|e| BotBrowserError::Cdp(format!("invalid screenshot base64: {e}")))
}

fn crop_element_png_if_needed(
    raw_png: Vec<u8>,
    rect: &ElementRect,
    viewport_width: f64,
    _viewport_height: f64,
) -> Result<Vec<u8>, BotBrowserError> {
    let img = image::load_from_memory_with_format(&raw_png, ImageFormat::Png)
        .map_err(|e| BotBrowserError::Cdp(format!("failed to decode screenshot PNG: {e}")))?;
    let (img_w, img_h) = img.dimensions();
    let dpr = if rect.device_pixel_ratio.is_finite() && rect.device_pixel_ratio > 0.0 {
        rect.device_pixel_ratio
    } else {
        1.0
    };

    let near_1x =
        (img_w as f64 - rect.width).abs() <= 2.0 && (img_h as f64 - rect.height).abs() <= 2.0;
    let near_dpr = (img_w as f64 - rect.width * dpr).abs() <= 2.0 * dpr
        && (img_h as f64 - rect.height * dpr).abs() <= 2.0 * dpr;
    let looks_full_viewport_1x =
        viewport_width > 0.0 && (img_w as f64 - viewport_width).abs() <= 2.0 && rect.width < viewport_width - 4.0;

    if (near_1x || (dpr > 1.0 && near_dpr)) && !looks_full_viewport_1x {
        return Ok(raw_png);
    }

    let scale = if viewport_width > 0.0 {
        let ratio = img_w as f64 / viewport_width;
        if dpr > 1.0 && (ratio - dpr).abs() < 0.25 {
            dpr
        } else if (ratio - 1.0).abs() < 0.25 {
            1.0
        } else if ratio > 0.5 {
            ratio
        } else {
            1.0
        }
    } else if dpr > 1.0
        && ((rect.x + rect.width) * dpr) <= img_w as f64 + 2.0
        && ((rect.y + rect.height) * dpr) <= img_h as f64 + 2.0
    {
        dpr
    } else {
        1.0
    };

    let crop_x = (rect.x * scale).round().max(0.0) as u32;
    let crop_y = (rect.y * scale).round().max(0.0) as u32;
    let mut crop_w = (rect.width * scale).round().max(1.0) as u32;
    let mut crop_h = (rect.height * scale).round().max(1.0) as u32;

    if crop_x >= img_w || crop_y >= img_h {
        return Err(BotBrowserError::Cdp(format!(
            "element crop origin ({crop_x}, {crop_y}) is outside screenshot bounds ({img_w}x{img_h})"
        )));
    }
    crop_w = crop_w.min(img_w - crop_x);
    crop_h = crop_h.min(img_h - crop_y);

    let cropped = img.crop_imm(crop_x, crop_y, crop_w, crop_h);
    let mut out = Cursor::new(Vec::new());
    cropped
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| BotBrowserError::Cdp(format!("failed to encode cropped PNG: {e}")))?;
    Ok(out.into_inner())
}
