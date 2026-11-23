//! Standalone HTTP service for the 全球法布施 (`@fabushi_bot/fabushi`) Mini App.
//!
//! Operates as an independent third-party Mini App service per Telegram Mini Apps architecture:
//! - Owns its own SQLite database (`fabushi_profiles`, `fabushi_transfer_logs`, etc.)
//! - Verifies `X-Telegram-Init-Data` using standard Telegram `WebAppData` HMAC-SHA256 with `bot_token`
//! - Sends bot DM receipts strictly via standard `POST {im_server_url}/bot{bot_token}/sendMessage`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{FromRequestParts, Path as AxumPath, Query, State};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use parking_lot::{Mutex, RwLock};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tower_http::cors::CorsLayer;

use crate::state::{
    dharma_ai_fallback_reply, AiChatMessage, HttpResponsePayload, LockedPractice, UserProfileDto,
};
use crate::storage;
use crate::sutras::{format_leaderboard_bytes, sutra_by_title};

type HmacSha256 = Hmac<Sha256>;

const DEFAULT_AI_SYSTEM_PROMPT: &str = "\
你现在是「全球法布施 (@fabushi_bot)」小程序内的慈悲修行助手。\
请以温和、恭敬、简明、正念的语言回答用户的佛法修行、诵经、禅修与全球法布施问题，\
适当引用经典原文或日常修行建议。切勿编造虚假的实时数据。";

#[derive(Debug, Clone)]
pub struct FabushiServerConfig {
    pub im_server_url: String,
    pub bot_token: String,
    pub bot_username: String,
    pub short_name: String,
    pub db_path: PathBuf,
    pub ai_base_url: Option<String>,
    pub ai_api_key: Option<String>,
    pub ai_model: Option<String>,
}

impl Default for FabushiServerConfig {
    fn default() -> Self {
        Self {
            im_server_url: "http://127.0.0.1:8080".to_string(),
            bot_token: String::new(),
            bot_username: "fabushi_bot".to_string(),
            short_name: "fabushi".to_string(),
            db_path: PathBuf::from("fabushi_miniapp.db"),
            ai_base_url: None,
            ai_api_key: None,
            ai_model: None,
        }
    }
}

pub struct FabushiServerState {
    pub db: Mutex<Connection>,
    pub http: reqwest::Client,
    pub sdk_js_cache: RwLock<Option<String>>,
    pub config: FabushiServerConfig,
}

impl FabushiServerState {
    pub fn open(mut config: FabushiServerConfig) -> Result<Self, String> {
        config.im_server_url = config.im_server_url.trim().trim_end_matches('/').to_string();
        config.bot_token = config.bot_token.trim().to_string();
        config.bot_username = config.bot_username.trim().trim_start_matches('@').to_string();
        config.short_name = config.short_name.trim().to_string();

        let conn = if config.db_path == Path::new(":memory:") {
            Connection::open_in_memory().map_err(|e| format!("open in-memory db: {e}"))?
        } else {
            if let Some(parent) = config.db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create db parent dir {}: {e}", parent.display()))?;
            }
            let c = Connection::open(&config.db_path)
                .map_err(|e| format!("open db {}: {e}", config.db_path.display()))?;
            let _ = c.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;");
            c
        };

        storage::init_fabushi_tables(&conn)
            .map_err(|e| format!("initialize fabushi tables: {e}"))?;

        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("build reqwest client: {e}"))?;

        Ok(Self {
            db: Mutex::new(conn),
            http,
            sdk_js_cache: RwLock::new(None),
            config,
        })
    }
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode_component(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Verifies standard Telegram WebApp `initData` HMAC-SHA256 using `bot_token`.
pub fn verify_telegram_init_data(
    bot_token: &str,
    init_data_raw: &str,
) -> Result<BTreeMap<String, String>, String> {
    if bot_token.trim().is_empty() {
        return Err("Server bot_token is not configured".to_string());
    }
    let mut params = BTreeMap::new();
    let mut provided_hash: Option<String> = None;

    for pair in init_data_raw.trim().trim_start_matches('?').split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_k, raw_v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let key = percent_decode_component(raw_k);
        let val = percent_decode_component(raw_v);
        if key == "hash" {
            provided_hash = Some(val);
        } else {
            params.insert(key, val);
        }
    }

    let provided_hash = provided_hash.ok_or_else(|| "initData missing hash".to_string())?;
    let provided_lower = provided_hash.to_ascii_lowercase();
    if provided_lower.len() != 64 {
        return Err("initData hash length is invalid".to_string());
    }

    let data_check_string = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let secret_key = hmac_sha256(b"WebAppData", bot_token.as_bytes());
    let expected = bytes_to_lower_hex(&hmac_sha256(&secret_key, data_check_string.as_bytes()));

    if provided_lower.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() != 1 {
        return Err("initData HMAC-SHA256 verification failed".to_string());
    }
    params.insert("hash".to_string(), provided_lower);
    Ok(params)
}

#[derive(Clone, Debug)]
pub struct FabushiInitDataAuth {
    pub user_id: i64,
    pub username: String,
    pub display_name: String,
}

fn unauthorized(code: &str, msg: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "type": "error",
            "error": code,
            "message": msg,
        })),
    )
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "type": "error",
            "error": "bad_request",
            "message": msg,
        })),
    )
        .into_response()
}

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "type": "error",
            "error": "not_found",
            "message": msg,
        })),
    )
        .into_response()
}

fn internal_err(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "type": "error",
            "error": "internal_error",
            "message": msg,
        })),
    )
        .into_response()
}

fn extract_init_data_header(parts: &Parts) -> Option<String> {
    if let Some(raw) = parts
        .headers
        .get("x-telegram-init-data")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(raw.to_string());
    }
    if let Some(auth) = parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
    {
        if let Some(rest) = auth.strip_prefix("tma ").or_else(|| auth.strip_prefix("TMA ")) {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn parse_init_data_user(params: &BTreeMap<String, String>) -> Option<(i64, String, String)> {
    let user_raw = params.get("user")?;
    let user_val: Value = serde_json::from_str(user_raw).ok()?;
    let id = user_val.get("id").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
            .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
    })?;
    if id <= 0 {
        return None;
    }
    let username = user_val
        .get("username")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("user_{id}"));
    let first = user_val
        .get("first_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let last = user_val
        .get("last_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let display = match (first.is_empty(), last.is_empty()) {
        (false, false) => format!("{first} {last}"),
        (false, true) => first.to_string(),
        (true, false) => last.to_string(),
        (true, true) => username.clone(),
    };
    Some((id, username, display))
}

impl FromRequestParts<Arc<FabushiServerState>> for FabushiInitDataAuth {
    type Rejection = (StatusCode, Json<Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<FabushiServerState>,
    ) -> Result<Self, Self::Rejection> {
        let Some(raw_init_data) = extract_init_data_header(parts) else {
            return Err(unauthorized(
                "missing_init_data",
                "Missing X-Telegram-Init-Data header",
            ));
        };
        let Ok(parsed) = verify_telegram_init_data(&state.config.bot_token, &raw_init_data) else {
            return Err(unauthorized(
                "invalid_init_data",
                "Invalid or expired X-Telegram-Init-Data HMAC signature",
            ));
        };
        let Some((user_id, username, display_name)) = parse_init_data_user(&parsed) else {
            return Err(unauthorized(
                "missing_init_data_user",
                "Signed X-Telegram-Init-Data does not contain a valid user payload",
            ));
        };
        Ok(FabushiInitDataAuth {
            user_id,
            username,
            display_name,
        })
    }
}

fn ensure_fabushi_user(
    state: &FabushiServerState,
    auth: &FabushiInitDataAuth,
) -> Result<UserProfileDto, String> {
    let is_admin = auth.username == "fabushi_official_publisher"
        || auth.username == "admin"
        || auth.user_id == 1;
    let conn = state.db.lock();
    storage::ensure_profile(
        &conn,
        auth.user_id,
        &auth.username,
        &auth.display_name,
        "",
        is_admin,
    )
    .map_err(|e| e.to_string())
}

/// Sends a receipt DM back to the user exclusively via standard Telegram Bot HTTP API `/bot{token}/sendMessage`.
async fn send_bot_api_dm_receipt(state: &FabushiServerState, user_id: i64, text: &str) {
    let token = state.config.bot_token.trim();
    let base = state.config.im_server_url.trim().trim_end_matches('/');
    let trimmed = text.trim();
    if token.is_empty() || base.is_empty() || trimmed.is_empty() {
        return;
    }
    let url = format!("{base}/bot{token}/sendMessage");
    let body = json!({
        "chat_id": user_id,
        "text": trimmed,
        "reply_markup": {
            "inline_keyboard": [[{
                "text": "🪷 打开 全球法布施",
                "web_app": {
                    "url": format!("gpui-miniapp://{}/{}", state.config.bot_username, state.config.short_name)
                }
            }]]
        }
    });
    match state.http.post(&url).json(&body).send().await {
        Ok(resp) if !resp.status().is_success() => {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            tracing::debug!(
                user_id,
                %status,
                err_body,
                "Bot API sendMessage receipt not delivered (e.g. write permission not granted or blocked)"
            );
        }
        Err(err) => {
            tracing::warn!(user_id, error = %err, "Failed to send Bot API DM receipt");
        }
        Ok(_) => {}
    }
}

fn format_targets_label(targets: &[String]) -> String {
    if targets.is_empty() || targets.iter().any(|c| c.eq_ignore_ascii_case("ALL")) {
        "全球所有节点".to_string()
    } else if targets.len() <= 4 {
        targets.join(", ")
    } else {
        format!("{} 等 {} 个国家/地区", targets[..3].join(", "), targets.len())
    }
}

async fn health_handler() -> Response {
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "service": "fabushi_miniapp_server" })),
    )
        .into_response()
}

async fn serve_index() -> Response {
    Html(crate::hosted_bundle_html()).into_response()
}

async fn serve_telegram_sdk_proxy(State(state): State<Arc<FabushiServerState>>) -> Response {
    if let Some(cached) = state.sdk_js_cache.read().clone() {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
            cached,
        )
            .into_response();
    }
    let base = state.config.im_server_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return err_response(StatusCode::BAD_GATEWAY, "im_server_url_missing", "IM server URL is not configured");
    }
    let url = format!("{base}/js/telegram-web-app.js");
    match state.http.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(js) if !js.trim().is_empty() => {
                *state.sdk_js_cache.write() = Some(js.clone());
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
                    js,
                )
                    .into_response()
            }
            _ => err_response(StatusCode::BAD_GATEWAY, "empty_sdk", "Empty telegram-web-app.js from IM server"),
        },
        Ok(resp) => err_response(
            StatusCode::BAD_GATEWAY,
            "sdk_fetch_failed",
            &format!("IM server returned {} for /js/telegram-web-app.js", resp.status()),
        ),
        Err(err) => err_response(
            StatusCode::BAD_GATEWAY,
            "sdk_fetch_failed",
            &format!("Failed to fetch /js/telegram-web-app.js from IM server: {err}"),
        ),
    }
}

fn err_response(status: StatusCode, code: &str, msg: &str) -> Response {
    (
        status,
        Json(json!({ "type": "error", "error": code, "message": msg })),
    )
        .into_response()
}

async fn serve_app_wasm() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/wasm"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        crate::app_wasm_bytes(),
    )
        .into_response()
}

async fn serve_asset(AxumPath(name): AxumPath<String>) -> Response {
    let clean = name.trim().trim_start_matches('/');
    match crate::asset_bytes(clean) {
        Some((bytes, content_type)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "public, max-age=300"),
            ],
            bytes,
        )
            .into_response(),
        None => err_response(StatusCode::NOT_FOUND, "asset_not_found", "Mini App asset not found"),
    }
}

async fn bootstrap_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
) -> Response {
    let profile = match ensure_fabushi_user(&state, &auth) {
        Ok(p) => p,
        Err(e) => return internal_err(&e),
    };
    let bundle = {
        let conn = state.db.lock();
        storage::get_bootstrap_bundle(
            &conn,
            auth.user_id,
            &profile.username,
            &profile.display_name,
            &profile.avatar_url,
            profile.is_admin,
        )
    };
    match bundle {
        Ok(b) => (
            StatusCode::OK,
            Json(HttpResponsePayload::Bootstrap {
                profile: b.profile,
                locked_practice: b.locked_practice,
                leaderboard: b.leaderboard,
                practice_leaderboard: b.practice_leaderboard,
                practice_records: b.practice_records,
                practice_stats: b.practice_stats,
                unlocked_achievement_ids: b.unlocked_achievement_ids,
                online_global_sending_count: b.online_global_sending_count,
                online_zen_room_count: b.online_zen_room_count,
            }),
        )
            .into_response(),
        Err(e) => internal_err(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    pub content_title: String,
    #[serde(default = "default_content_kind")]
    pub content_kind: String,
    #[serde(default = "default_target_countries")]
    pub target_countries: Vec<String>,
    #[serde(default)]
    pub bytes_sent: u64,
    #[serde(default)]
    pub beam_count: u32,
    #[serde(default = "default_notify_receipt")]
    pub notify_receipt: bool,
    #[serde(default)]
    pub receipt_only: bool,
    #[serde(default)]
    pub session_bytes: Option<u64>,
    #[serde(default)]
    pub session_beams: Option<u32>,
}

fn default_content_kind() -> String {
    "sutra".to_string()
}

fn default_target_countries() -> Vec<String> {
    vec!["ALL".to_string()]
}

fn default_notify_receipt() -> bool {
    true
}

async fn record_transfer_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<TransferRequest>,
) -> Response {
    let title = req.content_title.trim();
    if title.is_empty() {
        return bad_request("content_title must not be empty");
    }
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let targets = if req.target_countries.is_empty() {
        default_target_countries()
    } else {
        req.target_countries
    };
    let beams = if req.beam_count > 0 {
        req.beam_count
    } else {
        targets.len().max(1) as u32
    };
    let bytes = if req.bytes_sent > 0 {
        req.bytes_sent
    } else {
        sutra_by_title(title)
            .map(|s| s.content.len() as u64)
            .unwrap_or(1024)
            .saturating_mul(beams as u64)
            .max(1024)
    };
    let kind = if req.content_kind.trim().is_empty() {
        "sutra"
    } else {
        req.content_kind.trim()
    };

    let (res, display_bytes, display_beams) = if req.receipt_only {
        let query_res = {
            let conn = state.db.lock();
            (|| -> rusqlite::Result<storage::TransferRecordResult> {
                let profile = storage::get_profile(&conn, auth.user_id)?;
                let leaderboard = storage::get_global_leaderboard(&conn, auth.user_id, 50)?;
                Ok(storage::TransferRecordResult {
                    total_transfer_bytes: profile.total_transfer_bytes,
                    total_transfer_count: profile.total_transfer_count,
                    leaderboard,
                })
            })()
        };
        (
            query_res,
            req.session_bytes.unwrap_or(req.bytes_sent).max(1024),
            req.session_beams.unwrap_or(req.beam_count).max(1),
        )
    } else {
        let rec_res = {
            let conn = state.db.lock();
            storage::record_transfer(&conn, auth.user_id, title, kind, &targets, bytes, beams)
        };
        (
            rec_res,
            req.session_bytes.filter(|&v| v > 0).unwrap_or(bytes),
            req.session_beams.filter(|&v| v > 0).unwrap_or(beams),
        )
    };

    match res {
        Ok(tr) => {
            if req.notify_receipt {
                let receipt = format!(
                    "🌍 **全球法布施功德回执**\n• 布施内容：《{}》\n• 送达节点：{}（{} 束法光，本次 {}）\n• 累计法布施：{}（共 {} 次）\n愿法音遍传十方，一切有情同沾法喜 🙏",
                    title,
                    format_targets_label(&targets),
                    display_beams,
                    format_leaderboard_bytes(display_bytes),
                    format_leaderboard_bytes(tr.total_transfer_bytes),
                    tr.total_transfer_count
                );
                send_bot_api_dm_receipt(&state, auth.user_id, &receipt).await;
            }
            (
                StatusCode::OK,
                Json(HttpResponsePayload::TransferRecorded {
                    total_transfer_bytes: tr.total_transfer_bytes,
                    total_transfer_count: tr.total_transfer_count,
                    leaderboard: Some(tr.leaderboard),
                }),
            )
                .into_response()
        }
        Err(e) => internal_err(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct PracticeSessionRequest {
    #[serde(alias = "sutra_name")]
    pub sutra: String,
    #[serde(default)]
    pub duration_minutes: u32,
    #[serde(default)]
    pub chant_count: u32,
    #[serde(default)]
    pub start_time_ms: Option<i64>,
    #[serde(default)]
    pub end_time_ms: Option<i64>,
    #[serde(default)]
    pub notes: Option<String>,
}

async fn practice_session_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<PracticeSessionRequest>,
) -> Response {
    let sutra = req.sutra.trim();
    if sutra.is_empty() {
        return bad_request("sutra must not be empty");
    }
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let res = {
        let conn = state.db.lock();
        storage::record_practice_session(
            &conn,
            auth.user_id,
            sutra,
            req.duration_minutes,
            req.chant_count,
            req.start_time_ms,
            req.end_time_ms,
            req.notes.as_deref(),
        )
    };
    match res {
        Ok(ps) => {
            let notes_suffix = ps
                .record
                .notes
                .as_deref()
                .map(|n| format!("\n• 修行感悟：{n}"))
                .unwrap_or_default();
            let receipt = format!(
                "🪷 **禅室共修功德回执**\n• 修行功课：《{}》\n• 本次记录：诵持 {} 遍 · {} 分钟\n• 累计共修：{} 次 · 连续 {} 天{}\n随喜赞叹，愿以此功德庄严佛净土 🙏",
                ps.record.sutra_name,
                ps.record.chant_count,
                ps.record.duration_minutes,
                ps.profile.total_practice_sessions,
                ps.profile.consecutive_days,
                notes_suffix
            );
            send_bot_api_dm_receipt(&state, auth.user_id, &receipt).await;
            (
                StatusCode::OK,
                Json(HttpResponsePayload::PracticeSessionSaved {
                    record: ps.record,
                    practice_stats: ps.practice_stats,
                    profile: ps.profile,
                }),
            )
                .into_response()
        }
        Err(e) => internal_err(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct PracticeRecordsQuery {
    #[serde(default = "default_records_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_records_limit() -> usize {
    100
}

#[derive(Debug, Deserialize)]
pub struct ManualPracticeRecordRequest {
    #[serde(default)]
    pub id: Option<i64>,
    pub sutra_name: String,
    #[serde(default)]
    pub chant_count: u32,
    #[serde(default)]
    pub duration_minutes: u32,
    #[serde(default)]
    pub record_date: String,
    #[serde(default)]
    pub local_time: String,
    #[serde(default)]
    pub notes: Option<String>,
}

async fn list_practice_records_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Query(q): Query<PracticeRecordsQuery>,
) -> Response {
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let res = {
        let conn = state.db.lock();
        (|| -> rusqlite::Result<_> {
            let records = storage::list_practice_records(
                &conn,
                auth.user_id,
                q.limit.clamp(1, 500),
                q.offset,
            )?;
            let stats = storage::compute_practice_stats(&conn, auth.user_id)?;
            Ok((records, stats))
        })()
    };
    match res {
        Ok((practice_records, practice_stats)) => (
            StatusCode::OK,
            Json(HttpResponsePayload::PracticeRecordsRefreshed {
                practice_records,
                practice_stats,
            }),
        )
            .into_response(),
        Err(e) => internal_err(&e.to_string()),
    }
}

async fn create_practice_record_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<ManualPracticeRecordRequest>,
) -> Response {
    let sutra = req.sutra_name.trim();
    if sutra.is_empty() {
        return bad_request("sutra_name must not be empty");
    }
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let res = {
        let conn = state.db.lock();
        (|| -> rusqlite::Result<_> {
            let (_, stats) = storage::create_manual_practice_record(
                &conn,
                auth.user_id,
                sutra,
                req.chant_count,
                req.duration_minutes,
                &req.record_date,
                &req.local_time,
                req.notes.as_deref(),
            )?;
            let records = storage::list_practice_records(&conn, auth.user_id, 100, 0)?;
            Ok((records, stats))
        })()
    };
    match res {
        Ok((practice_records, practice_stats)) => (
            StatusCode::OK,
            Json(HttpResponsePayload::PracticeRecordsRefreshed {
                practice_records,
                practice_stats,
            }),
        )
            .into_response(),
        Err(e) => internal_err(&e.to_string()),
    }
}

async fn update_practice_record_by_id(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    AxumPath(id): AxumPath<i64>,
    Json(req): Json<ManualPracticeRecordRequest>,
) -> Response {
    update_practice_record_impl(&auth, &state, id, req).await
}

async fn update_practice_record_body(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<ManualPracticeRecordRequest>,
) -> Response {
    let Some(id) = req.id else {
        return bad_request("record id is required");
    };
    update_practice_record_impl(&auth, &state, id, req).await
}

async fn update_practice_record_impl(
    auth: &FabushiInitDataAuth,
    state: &Arc<FabushiServerState>,
    id: i64,
    req: ManualPracticeRecordRequest,
) -> Response {
    let sutra = req.sutra_name.trim();
    if sutra.is_empty() {
        return bad_request("sutra_name must not be empty");
    }
    if let Err(e) = ensure_fabushi_user(state, auth) {
        return internal_err(&e);
    }
    let res = {
        let conn = state.db.lock();
        (|| -> rusqlite::Result<_> {
            let updated = storage::update_practice_record(
                &conn,
                auth.user_id,
                id,
                sutra,
                req.chant_count,
                req.duration_minutes,
                &req.record_date,
                &req.local_time,
                req.notes.as_deref(),
            )?;
            if updated.is_none() {
                return Ok(None);
            }
            let records = storage::list_practice_records(&conn, auth.user_id, 100, 0)?;
            let stats = storage::compute_practice_stats(&conn, auth.user_id)?;
            Ok(Some((records, stats)))
        })()
    };
    match res {
        Ok(Some((practice_records, practice_stats))) => (
            StatusCode::OK,
            Json(HttpResponsePayload::PracticeRecordsRefreshed {
                practice_records,
                practice_stats,
            }),
        )
            .into_response(),
        Ok(None) => not_found("Practice record not found"),
        Err(e) => internal_err(&e.to_string()),
    }
}

async fn delete_practice_record_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    AxumPath(id): AxumPath<i64>,
) -> Response {
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let res = {
        let conn = state.db.lock();
        (|| -> rusqlite::Result<_> {
            let (deleted, stats) = storage::delete_practice_record(&conn, auth.user_id, id)?;
            if !deleted {
                return Ok(None);
            }
            let records = storage::list_practice_records(&conn, auth.user_id, 100, 0)?;
            Ok(Some((records, stats)))
        })()
    };
    match res {
        Ok(Some((practice_records, practice_stats))) => (
            StatusCode::OK,
            Json(HttpResponsePayload::PracticeRecordsRefreshed {
                practice_records,
                practice_stats,
            }),
        )
            .into_response(),
        Ok(None) => not_found("Practice record not found"),
        Err(e) => internal_err(&e.to_string()),
    }
}

async fn get_profile_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
) -> Response {
    let profile = match ensure_fabushi_user(&state, &auth) {
        Ok(p) => p,
        Err(e) => return internal_err(&e),
    };
    let locked_practice = {
        let conn = state.db.lock();
        storage::get_locked_practice(&conn, auth.user_id)
            .ok()
            .flatten()
    };
    (
        StatusCode::OK,
        Json(HttpResponsePayload::ProfileUpdated {
            profile,
            locked_practice,
        }),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub locked_practice: Option<LockedPractice>,
    #[serde(default)]
    pub locale: Option<String>,
}

async fn update_profile_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<UpdateProfileRequest>,
) -> Response {
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let res = {
        let conn = state.db.lock();
        storage::update_profile(
            &conn,
            auth.user_id,
            req.display_name.as_deref(),
            req.bio.as_deref(),
            req.locked_practice.as_ref(),
            req.locale.as_deref(),
        )
    };
    match res {
        Ok((profile, locked_practice)) => (
            StatusCode::OK,
            Json(HttpResponsePayload::ProfileUpdated {
                profile,
                locked_practice,
            }),
        )
            .into_response(),
        Err(e) => internal_err(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct ActivateMembershipRequest {
    #[serde(default = "default_vip_plan")]
    pub plan: String,
}

fn default_vip_plan() -> String {
    "yearly".to_string()
}

async fn activate_membership_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<ActivateMembershipRequest>,
) -> Response {
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let plan_norm = match req.plan.trim().to_ascii_lowercase().as_str() {
        "monthly" => "monthly",
        "quarterly" => "quarterly",
        _ => "yearly",
    };
    let res = {
        let conn = state.db.lock();
        storage::activate_membership(&conn, auth.user_id, plan_norm)
    };
    match res {
        Ok(profile) => (
            StatusCode::OK,
            Json(HttpResponsePayload::MembershipActivated { profile }),
        )
            .into_response(),
        Err(e) => internal_err(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct FollowRequest {
    pub target_username: String,
}

async fn follow_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<FollowRequest>,
) -> Response {
    let target = req.target_username.trim();
    if target.is_empty() {
        return bad_request("target_username must not be empty");
    }
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let res = {
        let conn = state.db.lock();
        storage::toggle_follow(&conn, auth.user_id, target)
    };
    match res {
        Ok(f) => (
            StatusCode::OK,
            Json(HttpResponsePayload::FollowToggled {
                target_username: f.target_username,
                is_following: f.is_following,
                follower_count: f.follower_count,
            }),
        )
            .into_response(),
        Err(e) => internal_err(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct LeaderboardQuery {
    #[serde(default = "default_leaderboard_kind")]
    pub kind: String,
    #[serde(default = "default_leaderboard_limit")]
    pub limit: usize,
}

fn default_leaderboard_kind() -> String {
    "global".to_string()
}

fn default_leaderboard_limit() -> usize {
    50
}

async fn leaderboard_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Query(q): Query<LeaderboardQuery>,
) -> Response {
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let limit = q.limit.clamp(1, 100);
    let kind_norm = if q.kind.trim().eq_ignore_ascii_case("practice") {
        "practice".to_string()
    } else {
        "global".to_string()
    };
    let res = {
        let conn = state.db.lock();
        if kind_norm == "practice" {
            storage::get_practice_leaderboard(&conn, auth.user_id, limit)
        } else {
            storage::get_global_leaderboard(&conn, auth.user_id, limit)
        }
    };
    match res {
        Ok(entries) => (
            StatusCode::OK,
            Json(HttpResponsePayload::LeaderboardFetched {
                kind: kind_norm,
                entries,
            }),
        )
            .into_response(),
        Err(e) => internal_err(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    #[serde(default = "default_activity_type")]
    pub activity_type: String,
}

fn default_activity_type() -> String {
    "global_sending".to_string()
}

async fn heartbeat_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<HeartbeatRequest>,
) -> Response {
    if let Err(e) = ensure_fabushi_user(&state, &auth) {
        return internal_err(&e);
    }
    let activity = if req.activity_type.trim() == "zen_room" {
        "zen_room".to_string()
    } else {
        "global_sending".to_string()
    };
    let res = {
        let conn = state.db.lock();
        storage::upsert_online_heartbeat(&conn, auth.user_id, &activity)
    };
    match res {
        Ok(online_count) => (
            StatusCode::OK,
            Json(HttpResponsePayload::HeartbeatAck {
                activity_type: activity,
                online_count,
            }),
        )
            .into_response(),
        Err(e) => internal_err(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct AiChatRequest {
    #[serde(alias = "prompt")]
    pub message: String,
    #[serde(default)]
    pub history: Vec<AiChatMessage>,
}

async fn call_external_ai_chat(
    state: &FabushiServerState,
    base_url: &str,
    locked_title: Option<&str>,
    history: &[AiChatMessage],
    question: &str,
) -> Option<String> {
    let endpoint = {
        let trimmed = base_url.trim().trim_end_matches('/');
        if trimmed.ends_with("/chat/completions") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/chat/completions")
        }
    };
    let model = state
        .config
        .ai_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("gpt-4o-mini");

    let mut sys = DEFAULT_AI_SYSTEM_PROMPT.to_string();
    if let Some(lp) = locked_title {
        sys.push_str(&format!("\n用户当前一门深入的主修功课：《{lp}》。"));
    }

    let mut msgs = vec![json!({ "role": "system", "content": sys })];
    for m in history.iter().rev().take(6).rev() {
        let role = if m.is_user { "user" } else { "assistant" };
        msgs.push(json!({ "role": role, "content": m.text.trim() }));
    }
    msgs.push(json!({ "role": "user", "content": question }));

    let mut req = state.http.post(&endpoint).json(&json!({
        "model": model,
        "messages": msgs,
    }));
    if let Some(key) = state
        .config
        .ai_api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: Value = resp.json().await.ok()?;
    let reply = val
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    Some(reply)
}

async fn ai_chat_handler(
    auth: FabushiInitDataAuth,
    State(state): State<Arc<FabushiServerState>>,
    Json(req): Json<AiChatRequest>,
) -> Response {
    let question = req.message.trim().to_string();
    if question.is_empty() {
        return bad_request("message must not be empty");
    }
    let profile = match ensure_fabushi_user(&state, &auth) {
        Ok(p) => p,
        Err(e) => return internal_err(&e),
    };
    let locked_title = {
        let conn = state.db.lock();
        storage::get_locked_practice(&conn, auth.user_id)
            .ok()
            .flatten()
            .map(|lp| lp.title)
    };

    if let Some(base_url) = state
        .config
        .ai_base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(reply) = call_external_ai_chat(
            &state,
            base_url,
            locked_title.as_deref(),
            &req.history,
            &question,
        )
        .await
        {
            return (
                StatusCode::OK,
                Json(HttpResponsePayload::AiChatReply { reply }),
            )
                .into_response();
        }
    }

    let fallback = dharma_ai_fallback_reply(&question, locked_title.as_deref(), &profile.locale);
    (
        StatusCode::OK,
        Json(HttpResponsePayload::AiChatReply { reply: fallback }),
    )
        .into_response()
}

pub fn build_router(state: Arc<FabushiServerState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/", get(serve_index))
        .route("/index.html", get(serve_index))
        .route("/js/telegram-web-app.js", get(serve_telegram_sdk_proxy))
        .route("/api/v1/fabushi-miniapp/app.wasm", get(serve_app_wasm))
        .route("/api/v1/fabushi-miniapp/assets/{*name}", get(serve_asset))
        .route("/api/v1/fabushi-miniapp/bootstrap", get(bootstrap_handler))
        .route("/api/v1/fabushi-miniapp/transfer", post(record_transfer_handler))
        .route(
            "/api/v1/fabushi-miniapp/practice/session",
            post(practice_session_handler),
        )
        .route(
            "/api/v1/fabushi-miniapp/practice/record",
            get(list_practice_records_handler)
                .post(create_practice_record_handler)
                .put(update_practice_record_body),
        )
        .route(
            "/api/v1/fabushi-miniapp/practice/record/{id}",
            put(update_practice_record_by_id).delete(delete_practice_record_handler),
        )
        .route(
            "/api/v1/fabushi-miniapp/profile",
            get(get_profile_handler)
                .put(update_profile_handler)
                .post(update_profile_handler),
        )
        .route(
            "/api/v1/fabushi-miniapp/membership",
            post(activate_membership_handler),
        )
        .route("/api/v1/fabushi-miniapp/follow", post(follow_handler))
        .route(
            "/api/v1/fabushi-miniapp/leaderboard",
            get(leaderboard_handler),
        )
        .route("/api/v1/fabushi-miniapp/heartbeat", post(heartbeat_handler))
        .route("/api/v1/fabushi-miniapp/ai-chat", post(ai_chat_handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign_test_init_data(bot_token: &str, user_id: i64, username: &str) -> String {
        let user_json = json!({
            "id": user_id,
            "username": username,
            "first_name": "Test",
            "last_name": "User",
        })
        .to_string();
        let pairs = [
            ("auth_date", "1700000000"),
            ("query_id", "qid_ext_1"),
            ("user", user_json.as_str()),
        ];
        let dcs = pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        let secret = hmac_sha256(b"WebAppData", bot_token.as_bytes());
        let hash = bytes_to_lower_hex(&hmac_sha256(&secret, dcs.as_bytes()));
        format!(
            "auth_date=1700000000&query_id=qid_ext_1&user={}&hash={hash}",
            url_encode(&user_json)
        )
    }

    fn url_encode(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(b as char);
            } else {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
        out
    }

    #[test]
    fn verifies_standard_telegram_webapp_init_data_hmac() {
        let token = "123456:AAFakeExternalBotTokenForUnitTests";
        let signed = sign_test_init_data(token, 100_042, "dharma_user");
        let verified = verify_telegram_init_data(token, &signed).expect("valid initData");
        let (uid, uname, display) = parse_init_data_user(&verified).expect("parsed user");
        assert_eq!(uid, 100_042);
        assert_eq!(uname, "dharma_user");
        assert_eq!(display, "Test User");

        let forged = signed.replace("100042", "999999");
        assert!(verify_telegram_init_data(token, &forged).is_err());
        assert!(verify_telegram_init_data("wrong_token", &signed).is_err());
    }
}
