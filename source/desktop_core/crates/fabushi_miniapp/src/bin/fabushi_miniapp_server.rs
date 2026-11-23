//! Standalone external HTTP server for the 全球法布施 (`@fabushi_bot/fabushi`) Mini App.

use std::path::PathBuf;
use std::sync::Arc;

use fabushi_miniapp::web_server::{build_router, FabushiServerConfig, FabushiServerState};
use tokio::net::TcpListener;

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_config() -> Result<FabushiServerConfig, String> {
    let mut listen = env_non_empty("FABUSHI_MINIAPP_LISTEN")
        .unwrap_or_else(|| "127.0.0.1:18090".to_string());
    let mut db_path = env_non_empty("FABUSHI_MINIAPP_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fabushi_miniapp.db"));
    let mut im_server_url = env_non_empty("IM_SERVER_URL")
        .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
    let mut bot_token = env_non_empty("FABUSHI_BOT_TOKEN").unwrap_or_default();
    let mut bot_token_file = env_non_empty("FABUSHI_BOT_TOKEN_FILE").map(PathBuf::from);
    let mut bot_username = env_non_empty("FABUSHI_BOT_USERNAME")
        .unwrap_or_else(|| "fabushi_bot".to_string());
    let mut short_name = env_non_empty("FABUSHI_MINIAPP_SHORT_NAME")
        .unwrap_or_else(|| "fabushi".to_string());
    let ai_base_url = env_non_empty("FABUSHI_AI_BASE_URL");
    let ai_api_key = env_non_empty("FABUSHI_AI_API_KEY");
    let ai_model = env_non_empty("FABUSHI_AI_MODEL");

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut next_val = |flag: &str| -> Result<String, String> {
            args.next()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        if let Some(v) = arg.strip_prefix("--listen=") {
            listen = v.to_string();
        } else if arg == "--listen" {
            listen = next_val("--listen")?;
        } else if let Some(v) = arg.strip_prefix("--db-path=") {
            db_path = PathBuf::from(v);
        } else if arg == "--db-path" {
            db_path = PathBuf::from(next_val("--db-path")?);
        } else if let Some(v) = arg.strip_prefix("--im-server-url=") {
            im_server_url = v.to_string();
        } else if arg == "--im-server-url" {
            im_server_url = next_val("--im-server-url")?;
        } else if let Some(v) = arg.strip_prefix("--bot-token=") {
            bot_token = v.to_string();
        } else if arg == "--bot-token" {
            bot_token = next_val("--bot-token")?;
        } else if let Some(v) = arg.strip_prefix("--bot-token-file=") {
            bot_token_file = Some(PathBuf::from(v));
        } else if arg == "--bot-token-file" {
            bot_token_file = Some(PathBuf::from(next_val("--bot-token-file")?));
        } else if let Some(v) = arg.strip_prefix("--bot-username=") {
            bot_username = v.to_string();
        } else if arg == "--bot-username" {
            bot_username = next_val("--bot-username")?;
        } else if let Some(v) = arg.strip_prefix("--short-name=") {
            short_name = v.to_string();
        } else if arg == "--short-name" {
            short_name = next_val("--short-name")?;
        } else if arg == "-h" || arg == "--help" {
            println!(
                "Usage: fabushi_miniapp_server [--listen 127.0.0.1:18090] [--db-path fabushi_miniapp.db] \
                 [--im-server-url http://127.0.0.1:8080] [--bot-token <TOKEN> | --bot-token-file <PATH>]"
            );
            std::process::exit(0);
        } else {
            return Err(format!("Unknown argument: {arg}"));
        }
    }

    if bot_token.is_empty() {
        if let Some(ref p) = bot_token_file {
            if p.exists() {
                bot_token = std::fs::read_to_string(p)
                    .map_err(|e| format!("read bot token file {}: {e}", p.display()))?
                    .trim()
                    .to_string();
            }
        }
    }

    if bot_token.is_empty() {
        return Err(
            "Bot token is required: pass --bot-token, --bot-token-file, or set FABUSHI_BOT_TOKEN"
                .to_string(),
        );
    }

    let _ = listen;
    Ok(FabushiServerConfig {
        im_server_url,
        bot_token,
        bot_username,
        short_name,
        db_path,
        ai_base_url,
        ai_api_key,
        ai_model,
    })
}

fn parse_listen_addr() -> String {
    let mut listen = env_non_empty("FABUSHI_MINIAPP_LISTEN")
        .unwrap_or_else(|| "127.0.0.1:18090".to_string());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(v) = arg.strip_prefix("--listen=") {
            listen = v.to_string();
        } else if arg == "--listen" {
            if let Some(v) = args.next() {
                listen = v;
            }
        }
    }
    listen
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = match parse_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {e}");
            std::process::exit(2);
        }
    };
    let listen_addr = parse_listen_addr();
    let state = match FabushiServerState::open(cfg) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("Failed to initialize FabushiServerState: {e}");
            std::process::exit(1);
        }
    };
    let listener = match TcpListener::bind(&listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind {listen_addr}: {e}");
            std::process::exit(1);
        }
    };
    let bound = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or(listen_addr);
    tracing::info!(
        addr = %bound,
        im_server_url = %state.config.im_server_url,
        bot_username = %state.config.bot_username,
        short_name = %state.config.short_name,
        "fabushi_miniapp_server listening"
    );

    if let Err(e) = axum::serve(listener, build_router(state)).await {
        eprintln!("fabushi_miniapp_server terminated with error: {e}");
        std::process::exit(1);
    }
}
