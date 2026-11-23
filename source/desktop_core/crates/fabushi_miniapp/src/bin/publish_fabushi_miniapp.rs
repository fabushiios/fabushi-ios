//! Standard external Mini App publisher CLI for 全球法布施 (`@fabushi_bot/fabushi`).
//!
//! Registers `@fabushi_bot` and publishes its external `web_url` (`fabushi_miniapp_server`)
//! via standard `@BotFather` / Bot API (`publishMiniApp`, `setChatMenuButton`) endpoints.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:18080";
const DEFAULT_MINIAPP_URL: &str = "http://127.0.0.1:18090";
const PUBLISHER_USERNAME: &str = "fabushi_official_publisher";
const BOT_USERNAME: &str = "fabushi_bot";
const BOT_DISPLAY_NAME: &str = "全球法布施";
const MINIAPP_SHORT_NAME: &str = "fabushi";
const MINIAPP_TITLE: &str = "全球法布施";
const MINIAPP_DESC: &str =
    "全球法布施与净心禅室 — 3D/2D地球弘法传递、八部经典诵读、禅坐敲木鱼与全球修行榜单";
const BOTFATHER_USER_ID: i64 = 777_001;

struct Config {
    server_url: String,
    miniapp_url: String,
    password: String,
    write_bot_token: Option<PathBuf>,
    skip_miniapp_health_check: bool,
}

fn parse_args() -> Result<Config, String> {
    let mut server_url = std::env::var("GPUI_TELEGRAM_SERVER_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string());
    let mut miniapp_url = std::env::var("FABUSHI_MINIAPP_PUBLIC_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MINIAPP_URL.to_string());
    let password = std::env::var("FABUSHI_PUBLISHER_PASSWORD")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "FabushiOfficial#2026".to_string());
    let mut write_bot_token: Option<PathBuf> = std::env::var("FABUSHI_BOT_TOKEN_FILE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);
    let mut skip_miniapp_health_check = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(val) = arg.strip_prefix("--server-url=") {
            server_url = val.to_string();
        } else if arg == "--server-url" {
            let val = args
                .next()
                .ok_or_else(|| "--server-url requires a URL argument".to_string())?;
            server_url = val;
        } else if let Some(val) = arg.strip_prefix("--miniapp-url=") {
            miniapp_url = val.to_string();
        } else if arg == "--miniapp-url" {
            let val = args
                .next()
                .ok_or_else(|| "--miniapp-url requires a URL argument".to_string())?;
            miniapp_url = val;
        } else if let Some(val) = arg.strip_prefix("--write-bot-token=") {
            write_bot_token = Some(PathBuf::from(val));
        } else if arg == "--write-bot-token" {
            let val = args
                .next()
                .ok_or_else(|| "--write-bot-token requires a file path".to_string())?;
            write_bot_token = Some(PathBuf::from(val));
        } else if arg == "--skip-miniapp-health-check" {
            skip_miniapp_health_check = true;
        } else if arg == "-h" || arg == "--help" {
            println!(
                "Usage: publish_fabushi_miniapp [--server-url <IM_URL>] [--miniapp-url <MINIAPP_URL>] [--write-bot-token <PATH>] [--skip-miniapp-health-check]"
            );
            std::process::exit(0);
        } else {
            return Err(format!("Unknown argument: {arg}"));
        }
    }

    Ok(Config {
        server_url: server_url.trim_end_matches('/').to_string(),
        miniapp_url: miniapp_url.trim_end_matches('/').to_string(),
        password,
        write_bot_token,
        skip_miniapp_health_check,
    })
}

async fn register_or_login_publisher(
    client: &reqwest::Client,
    base: &str,
    password: &str,
) -> Result<String, String> {
    let reg_res = client
        .post(format!("{base}/api/v1/auth/register"))
        .json(&json!({
            "username": PUBLISHER_USERNAME,
            "password": password,
            "first_name": "全球法布施官方发布者",
            "device_name": "publish_fabushi_miniapp",
            "auto_welcome": true
        }))
        .send()
        .await
        .map_err(|e| format!("register request failed: {e}"))?;

    if reg_res.status().is_success() {
        let body: Value = reg_res
            .json()
            .await
            .map_err(|e| format!("parse register json: {e}"))?;
        if let Some(tok) = body.get("token").and_then(|v| v.as_str()) {
            println!("Registered publisher account @{PUBLISHER_USERNAME}");
            return Ok(tok.to_string());
        }
    }

    let login_res = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({
            "username": PUBLISHER_USERNAME,
            "password": password,
            "device_name": "publish_fabushi_miniapp"
        }))
        .send()
        .await
        .map_err(|e| format!("login request failed: {e}"))?;

    let status = login_res.status();
    let body: Value = login_res
        .json()
        .await
        .map_err(|e| format!("parse login json: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Failed to register or log in {PUBLISHER_USERNAME}: {status} {body}"
        ));
    }
    body.get("token")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| format!("login response missing token: {body}"))
}

async fn find_botfather_chat_id(
    client: &reqwest::Client,
    base: &str,
    bearer: &str,
) -> Result<i64, String> {
    let chats_res = client
        .get(format!("{base}/api/v1/chats"))
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("GET /api/v1/chats failed: {e}"))?;
    if chats_res.status().is_success() {
        let val: Value = chats_res.json().await.unwrap_or(Value::Null);
        let arr = val
            .get("chats")
            .and_then(|v| v.as_array())
            .or_else(|| val.as_array());
        if let Some(list) = arr {
            for c in list {
                let is_bf = c.get("peer_user_id").and_then(|v| v.as_i64())
                    == Some(BOTFATHER_USER_ID)
                    || c.get("peer_username").and_then(|v| v.as_str()) == Some("BotFather")
                    || c.get("title")
                        .and_then(|v| v.as_str())
                        .is_some_and(|t| t.contains("BotFather"));
                if is_bf {
                    if let Some(cid) = c
                        .get("chat_id")
                        .or_else(|| c.get("id"))
                        .and_then(|v| v.as_i64())
                    {
                        return Ok(cid);
                    }
                }
            }
        }
    }

    let search_res = client
        .get(format!("{base}/api/v1/search?q=BotFather"))
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("search BotFather failed: {e}"))?;
    if search_res.status().is_success() {
        let val: Value = search_res.json().await.unwrap_or(Value::Null);
        if let Some(results) = val.get("results").and_then(|v| v.as_array()) {
            for r in results {
                if r.get("username").and_then(|v| v.as_str()) == Some("BotFather") {
                    if let Some(cid) = r.get("chat_id").and_then(|v| v.as_i64()) {
                        return Ok(cid);
                    }
                }
            }
        }
    }

    let contact_res = client
        .post(format!("{base}/api/v1/contacts/add"))
        .bearer_auth(bearer)
        .json(&json!({ "username": "BotFather", "is_bot": true }))
        .send()
        .await
        .map_err(|e| format!("POST /api/v1/contacts/add (BotFather) failed: {e}"))?;
    let val: Value = contact_res.json().await.unwrap_or(Value::Null);
    val.get("chat_id")
        .or_else(|| val.get("id"))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| format!("Unable to resolve @BotFather DM chat_id: {val}"))
}

fn extract_backtick_token(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let candidate = after[..end].trim();
        if candidate.contains(':') && candidate.len() > 16 {
            return Some(candidate.to_string());
        }
        rest = &after[end + 1..];
    }
    None
}

async fn recover_token_via_botfather(
    client: &reqwest::Client,
    base: &str,
    bearer: &str,
) -> Result<String, String> {
    let bf_cid = find_botfather_chat_id(client, base, bearer).await?;
    for cmd in ["/token", "@fabushi_bot"] {
        let res = client
            .post(format!("{base}/api/v1/chats/{bf_cid}/messages"))
            .bearer_auth(bearer)
            .json(&json!({ "text": cmd }))
            .send()
            .await
            .map_err(|e| format!("send BotFather `{cmd}` failed: {e}"))?;
        if !res.status().is_success() {
            let st = res.status();
            let txt = res.text().await.unwrap_or_default();
            return Err(format!("BotFather `{cmd}` error: {st} {txt}"));
        }
    }

    let msgs_res = client
        .get(format!("{base}/api/v1/chats/{bf_cid}/messages?limit=20"))
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("GET BotFather messages failed: {e}"))?;
    let val: Value = msgs_res
        .json()
        .await
        .map_err(|e| format!("parse BotFather messages: {e}"))?;
    let list = val
        .get("messages")
        .and_then(|v| v.as_array())
        .or_else(|| val.as_array())
        .ok_or_else(|| format!("Unexpected BotFather messages shape: {val}"))?;
    for msg in list.iter().rev() {
        let text = msg
            .get("text")
            .or_else(|| msg.get("content").and_then(|c| c.get("text")))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Some(tok) = extract_backtick_token(text) {
            return Ok(tok);
        }
    }
    Err("Could not find bot_token in @BotFather /token response".to_string())
}

fn fabushi_ui_schema() -> Value {
    json!({
        "title": MINIAPP_TITLE,
        "subtitle": "@fabushi_bot · 全球法布施与净心禅室",
        "theme": {
            "bg_color": "#0B0E14",
            "secondary_bg_color": "#1B263B",
            "text_color": "#E8EAF6",
            "hint_color": "#9FA8DA",
            "button_color": "#D4AF37",
            "button_text_color": "#0B0E14",
            "header_bg_color": "#0B0E14",
            "section_bg_color": "#141B29"
        },
        "widgets": [
            {
                "id": "ai_summary",
                "kind": "Text",
                "label": "当前状态",
                "value": "已准备好向全球 249+ 国家与地区节点发送法布施，或进入禅室共修诵持。",
                "variant": "callout"
            },
            {
                "id": "mode",
                "kind": "Select",
                "label": "修行模式",
                "value": "全球法布施",
                "options": ["全球法布施", "禅室共修"]
            },
            {
                "id": "sutra",
                "kind": "Select",
                "label": "法宝经典",
                "value": "心经",
                "options": ["心经", "金刚经", "阿弥陀经", "大悲咒", "地藏经", "药师经", "法华经", "华严经"]
            },
            {
                "id": "target_country",
                "kind": "TextInput",
                "label": "目标国家/地区代码",
                "value": "ALL",
                "placeholder": "输入 ALL 或国家代码如 CN / US / JP"
            },
            {
                "id": "chant_count",
                "kind": "Counter",
                "label": "诵持遍数",
                "count": 108,
                "min": 1,
                "max": 10000
            }
        ],
        "main_button": {
            "text": "发起法布施 / 记录共修",
            "is_visible": true,
            "is_active": true,
            "action_id": "submit",
            "color": "#D4AF37",
            "text_color": "#0B0E14"
        },
        "secondary_button": {
            "text": "发送功德摘要到伴生聊天",
            "is_visible": true,
            "is_active": true,
            "action_id": "send_data",
            "sends_data": true
        }
    })
}

async fn create_or_update_fabushi_bot(
    client: &reqwest::Client,
    base: &str,
    miniapp_url: &str,
    bearer: &str,
) -> Result<String, String> {
    let create_res = client
        .post(format!("{base}/api/v1/bots"))
        .bearer_auth(bearer)
        .json(&json!({
            "display_name": BOT_DISPLAY_NAME,
            "username": BOT_USERNAME,
            "description": MINIAPP_DESC,
            "about": "全球法布施官方伴生机器人 · 打开主应用开启净心修行与全球普渡法布施",
            "ai_persona": "你是「全球法布施 (@fabushi_bot)」慈悲修行向导，协助行者进行全球法布施、八部经典持诵、禅室共修与修行功德查询。",
            "menu_button_text": BOT_DISPLAY_NAME,
            "menu_button_url": miniapp_url,
            "allow_attach_menu": true,
            "attach_peer_types": ["users", "bots", "groups", "channels"],
            "commands": [
                { "command": "start", "description": "打开全球法布施伴生指引" },
                { "command": "fabushi", "description": "进入全球法布施与禅室 Mini App" },
                { "command": "help", "description": "查看法布施与一门深入帮助" }
            ]
        }))
        .send()
        .await
        .map_err(|e| format!("POST /api/v1/bots failed: {e}"))?;

    if create_res.status().is_success() {
        let val: Value = create_res
            .json()
            .await
            .map_err(|e| format!("parse /api/v1/bots json: {e}"))?;
        if let Some(tok) = val.get("bot_token").and_then(|v| v.as_str()) {
            println!("Created @{BOT_USERNAME} with attach menu enabled");
            return Ok(tok.to_string());
        }
    }

    let _ = client
        .patch(format!("{base}/api/v1/fabushi-bots/{BOT_USERNAME}"))
        .bearer_auth(bearer)
        .json(&json!({
            "display_name": BOT_DISPLAY_NAME,
            "description": MINIAPP_DESC,
            "title": "全球法布施官方伴生机器人"
        }))
        .send()
        .await;

    let tok = recover_token_via_botfather(client, base, bearer).await?;
    println!("Updated existing @{BOT_USERNAME} and recovered Bot API token via @BotFather");
    Ok(tok)
}

async fn publish_and_configure_bot_api(
    client: &reqwest::Client,
    base: &str,
    miniapp_url: &str,
    bot_token: &str,
) -> Result<(), String> {
    let set_name_res = client
        .post(format!("{base}/bot{bot_token}/setMyName"))
        .json(&json!({ "name": BOT_DISPLAY_NAME }))
        .send()
        .await
        .map_err(|e| format!("setMyName failed: {e}"))?;
    if !set_name_res.status().is_success() {
        let txt = set_name_res.text().await.unwrap_or_default();
        return Err(format!("setMyName rejected: {txt}"));
    }

    let set_cmds_res = client
        .post(format!("{base}/bot{bot_token}/setMyCommands"))
        .json(&json!({
            "commands": [
                { "command": "start", "description": "打开全球法布施伴生指引" },
                { "command": "fabushi", "description": "进入全球法布施与禅室 Mini App" },
                { "command": "help", "description": "查看法布施与一门深入帮助" }
            ]
        }))
        .send()
        .await
        .map_err(|e| format!("setMyCommands failed: {e}"))?;
    if !set_cmds_res.status().is_success() {
        let txt = set_cmds_res.text().await.unwrap_or_default();
        return Err(format!("setMyCommands rejected: {txt}"));
    }

    let publish_res = client
        .post(format!("{base}/bot{bot_token}/publishMiniApp"))
        .json(&json!({
            "short_name": MINIAPP_SHORT_NAME,
            "title": MINIAPP_TITLE,
            "description": MINIAPP_DESC,
            "category": "lifestyle",
            "icon_emoji": "🪷",
            "web_url": miniapp_url,
            "is_main_app": true,
            "is_attach_enabled": true,
            "header_color": "#0B0E14",
            "background_color": "#0B0E14",
            "ui_schema": fabushi_ui_schema()
        }))
        .send()
        .await
        .map_err(|e| format!("publishMiniApp failed: {e}"))?;
    let pub_status = publish_res.status();
    let pub_body: Value = publish_res
        .json()
        .await
        .map_err(|e| format!("parse publishMiniApp response: {e}"))?;
    if !pub_status.is_success() || pub_body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!("publishMiniApp failed: {pub_status} {pub_body}"));
    }
    println!(
        "Published external Mini App @{BOT_USERNAME}/{MINIAPP_SHORT_NAME} ({MINIAPP_TITLE}) -> {miniapp_url}"
    );

    let menu_res = client
        .post(format!("{base}/bot{bot_token}/setChatMenuButton"))
        .json(&json!({
            "menu_button": {
                "type": "web_app",
                "text": MINIAPP_TITLE,
                "web_app": { "url": miniapp_url }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("setChatMenuButton failed: {e}"))?;
    let menu_status = menu_res.status();
    let menu_body: Value = menu_res
        .json()
        .await
        .map_err(|e| format!("parse setChatMenuButton response: {e}"))?;
    if !menu_status.is_success() || menu_body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!(
            "setChatMenuButton failed: {menu_status} {menu_body}"
        ));
    }
    println!("Configured WebApp menu button -> {miniapp_url}");
    Ok(())
}

async fn verify_published_endpoints(
    client: &reqwest::Client,
    base: &str,
    miniapp_url: &str,
    bearer: &str,
    skip_miniapp_health_check: bool,
) -> Result<(), String> {
    let miniapps_url =
        format!("{base}/api/v1/miniapps?q=%E5%85%A8%E7%90%83%E6%B3%95%E5%B8%83%E6%96%BD");
    let res1 = client
        .get(&miniapps_url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("GET {miniapps_url} failed: {e}"))?;
    if res1.status() != reqwest::StatusCode::OK {
        return Err(format!("{miniapps_url} returned {}", res1.status()));
    }
    let val1: Value = res1
        .json()
        .await
        .map_err(|e| format!("parse /api/v1/miniapps json: {e}"))?;
    let has_app1 = val1
        .get("apps")
        .and_then(|v| v.as_array())
        .is_some_and(|arr| {
            arr.iter()
                .any(|a| a.get("short_name").and_then(|v| v.as_str()) == Some(MINIAPP_SHORT_NAME))
        });
    if !has_app1 {
        return Err(format!(
            "/api/v1/miniapps?q=全球法布施 missing `{MINIAPP_SHORT_NAME}`: {val1}"
        ));
    }

    let search_url = format!("{base}/api/v1/search?q=fabushi&category=apps");
    let res2 = client
        .get(&search_url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("GET {search_url} failed: {e}"))?;
    if res2.status() != reqwest::StatusCode::OK {
        return Err(format!("{search_url} returned {}", res2.status()));
    }
    let val2: Value = res2
        .json()
        .await
        .map_err(|e| format!("parse /api/v1/search json: {e}"))?;
    let has_app2 = val2
        .get("apps")
        .and_then(|v| v.as_array())
        .is_some_and(|arr| {
            arr.iter()
                .any(|a| a.get("short_name").and_then(|v| v.as_str()) == Some(MINIAPP_SHORT_NAME))
        });
    if !has_app2 {
        return Err(format!(
            "/api/v1/search?q=fabushi&category=apps missing `{MINIAPP_SHORT_NAME}`: {val2}"
        ));
    }

    let im_entry_url = format!("{base}/miniapps/{BOT_USERNAME}/{MINIAPP_SHORT_NAME}");
    let res3 = client
        .get(&im_entry_url)
        .send()
        .await
        .map_err(|e| format!("GET {im_entry_url} failed: {e}"))?;
    if res3.status() != reqwest::StatusCode::OK {
        return Err(format!("{im_entry_url} returned {}", res3.status()));
    }
    let im_html = res3
        .text()
        .await
        .map_err(|e| format!("read {im_entry_url} body: {e}"))?;
    if !im_html.contains(miniapp_url) {
        return Err(format!(
            "{im_entry_url} did not reference external URL {miniapp_url}"
        ));
    }

    if !skip_miniapp_health_check {
        let health_url = format!("{miniapp_url}/health");
        let h_res = client
            .get(&health_url)
            .send()
            .await
            .map_err(|e| format!("GET {health_url} failed: {e}"))?;
        if h_res.status() != reqwest::StatusCode::OK {
            return Err(format!("{health_url} returned {}", h_res.status()));
        }

        let ext_html_res = client
            .get(format!("{miniapp_url}/"))
            .send()
            .await
            .map_err(|e| format!("GET {miniapp_url}/ failed: {e}"))?;
        if ext_html_res.status() != reqwest::StatusCode::OK {
            return Err(format!(
                "{miniapp_url}/ returned {}",
                ext_html_res.status()
            ));
        }

        let wasm_url = format!("{miniapp_url}/api/v1/fabushi-miniapp/app.wasm");
        let res4 = client
            .get(&wasm_url)
            .send()
            .await
            .map_err(|e| format!("GET {wasm_url} failed: {e}"))?;
        if res4.status() != reqwest::StatusCode::OK {
            return Err(format!("{wasm_url} returned {}", res4.status()));
        }
        let wasm_bytes = res4
            .bytes()
            .await
            .map_err(|e| format!("read {wasm_url} bytes: {e}"))?;
        if !wasm_bytes.starts_with(b"\0asm") {
            return Err(format!("{wasm_url} did not return valid WASM magic header"));
        }
    }

    println!(
        "Verified 200 OK:\n  - {miniapps_url}\n  - {search_url}\n  - {im_entry_url} -> {miniapp_url}"
    );
    Ok(())
}

fn write_token_file(path: &PathBuf, token: &str) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create token dir {}: {e}", parent.display()))?;
    }
    std::fs::write(path, format!("{token}\n"))
        .map_err(|e| format!("write bot token to {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    println!("Wrote @{BOT_USERNAME} Bot API token to {}", path.display());
    Ok(())
}

#[tokio::main]
async fn main() {
    let cfg = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Argument error: {e}");
            std::process::exit(2);
        }
    };

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build reqwest client");

    let result = async {
        let bearer = register_or_login_publisher(&client, &cfg.server_url, &cfg.password).await?;
        let bot_token =
            create_or_update_fabushi_bot(&client, &cfg.server_url, &cfg.miniapp_url, &bearer)
                .await?;
        if let Some(ref token_path) = cfg.write_bot_token {
            write_token_file(token_path, &bot_token)?;
        }
        publish_and_configure_bot_api(&client, &cfg.server_url, &cfg.miniapp_url, &bot_token)
            .await?;
        verify_published_endpoints(
            &client,
            &cfg.server_url,
            &cfg.miniapp_url,
            &bearer,
            cfg.skip_miniapp_health_check,
        )
        .await?;
        Ok::<(), String>(())
    }
    .await;

    if let Err(err) = result {
        eprintln!("publish_fabushi_miniapp failed: {err}");
        std::process::exit(1);
    }
}
