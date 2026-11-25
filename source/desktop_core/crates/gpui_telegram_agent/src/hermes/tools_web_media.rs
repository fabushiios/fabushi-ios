use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use std::time::Instant;

use bot_browser::backend::{
    BrowserSessionBackend, CdpBrowserBackend, KeyEventKind, KeyInputEvent, PointerButton,
    PointerEventKind, PointerInputEvent,
};
use bot_browser::cdp::{CdpBridge, LoginFormSelectors, WsCdpTransport};
use bot_browser::orchestrator::{
    BotBrowserOrchestrator, ClickAssistResult, VaultFillResult, VaultListResult,
    VaultRequestCardResult,
};
use bot_browser::script_app::ScriptAppMetadata;
use bot_browser::types::{
    BotBrowserError, BrowserBackendKind, BrowserSnapshot, NormalizedOrigin, ReusePolicy,
    SemanticLocator,
};
use gpui_telegram_media::{
    compute_pcm_rms_level, decode_wav_pcm, encode_wav_i16, waveform_from_pcm_i16,
    ThumbnailGenerator,
};

use super::browser_session::{
    looks_like_secret_value, redact_snapshot_for_observation, redact_url_for_trace, AiTurnGuard,
    BrowserAutomationSession, SessionBackendSource, StepGateInfo,
};
use super::security::scan_content_for_injection;
use super::toolsets::{
    HermesInlineButton, HermesToolArtifact, HermesToolContext, HermesToolResult, HermesUiAction,
};
use super::trajectory::SecretRedactor;

pub async fn execute_web_media_tool(
    name: &str,
    args: Value,
    ctx: &HermesToolContext,
) -> HermesToolResult {
    match name {
        "web_search" => tool_web_search(args, ctx).await,
        "web_extract" => tool_web_extract(args, ctx).await,
        "browser_navigate" => tool_browser_navigate(args, ctx).await,
        "browser_snapshot" => tool_browser_snapshot(args, ctx).await,
        "browser_click" => tool_browser_click(args, ctx).await,
        "browser_type" => tool_browser_type(args, ctx).await,
        "browser_screenshot" => tool_browser_screenshot(args, ctx).await,
        "browser_press_key" => tool_browser_press_key(args, ctx).await,
        "browser_scroll" => tool_browser_scroll(args, ctx).await,
        "browser_wait_for" => tool_browser_wait_for(args, ctx).await,
        "browser_extract_text" => tool_browser_extract_text(args, ctx).await,
        "browser_request_takeover" => tool_browser_request_takeover(args, ctx).await,
        "distill_browser_script_app" => tool_distill_browser_script_app(args, ctx).await,
        "vault_list" => tool_vault_list(args, ctx).await,
        "vault_request_card" => tool_vault_request_card(args, ctx).await,
        "vault_fill" => tool_vault_fill(args, ctx).await,
        "human_assist_qr" => tool_human_assist_qr(args, ctx).await,
        "human_assist_click" => tool_human_assist_click(args, ctx).await,
        "text_to_speech" => tool_text_to_speech(args, ctx).await,
        "speech_to_text" => tool_speech_to_text(args, ctx).await,
        "vision_analyze" => tool_vision_analyze(args, ctx).await,
        "image_generate" => tool_image_generate(args, ctx).await,
        "video_generate" => tool_video_generate(args, ctx).await,
        _ => HermesToolResult::err(name, format!("Unknown web/media tool '{name}'")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
}

async fn tool_web_search(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(query) = args.get("query").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
        return HermesToolResult::err("web_search", "Missing required 'query' parameter");
    };
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("duckduckgo")
        .trim()
        .to_ascii_lowercase();
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 20) as usize;
    let endpoint_override = args.get("endpoint_url").and_then(|v| v.as_str());

    let res = match provider.as_str() {
        "tavily" => search_tavily(query, limit, endpoint_override, ctx).await,
        "brave" => search_brave(query, limit, endpoint_override, ctx).await,
        "perplexity" => search_perplexity(query, endpoint_override, ctx).await,
        "searxng" => search_searxng(query, limit, endpoint_override, ctx).await,
        "duckduckgo" | "ddg" => search_duckduckgo(query, limit, endpoint_override, ctx).await,
        other => Err(format!("Unsupported web_search provider '{other}'")),
    };

    match res {
        Ok(items) => {
            let formatted = if items.is_empty() {
                format!("No search results found for '{query}' via {provider}.")
            } else {
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        format!(
                            "{}. [{}]({})\n   {}",
                            i + 1,
                            item.title,
                            item.url,
                            item.snippet
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            HermesToolResult::ok(
                "web_search",
                formatted,
                Some(json!({ "provider": provider, "query": query, "results": items })),
            )
        }
        Err(e) => HermesToolResult::err("web_search", e),
    }
}

async fn search_tavily(
    query: &str,
    limit: usize,
    endpoint_override: Option<&str>,
    ctx: &HermesToolContext,
) -> Result<Vec<WebSearchResult>, String> {
    let api_key = std::env::var("TAVILY_API_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "TAVILY_API_KEY is not configured".to_string())?;
    let url = endpoint_override.unwrap_or("https://api.tavily.com/search");
    let resp = ctx
        .http_client
        .post(url)
        .json(&json!({
            "api_key": api_key,
            "query": query,
            "max_results": limit,
            "search_depth": "basic"
        }))
        .send()
        .await
        .map_err(|e| format!("Tavily request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Tavily returned HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Tavily JSON parse failed: {e}"))?;
    let mut results = Vec::new();
    if let Some(arr) = body.get("results").and_then(|v| v.as_array()) {
        for item in arr.iter().take(limit) {
            results.push(WebSearchResult {
                title: item.get("title").and_then(|v| v.as_str()).unwrap_or("").into(),
                url: item.get("url").and_then(|v| v.as_str()).unwrap_or("").into(),
                snippet: item.get("content").and_then(|v| v.as_str()).unwrap_or("").into(),
                source: "tavily".into(),
            });
        }
    }
    Ok(results)
}

async fn search_brave(
    query: &str,
    limit: usize,
    endpoint_override: Option<&str>,
    ctx: &HermesToolContext,
) -> Result<Vec<WebSearchResult>, String> {
    let api_key = std::env::var("BRAVE_API_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "BRAVE_API_KEY is not configured".to_string())?;
    let base = endpoint_override.unwrap_or("https://api.search.brave.com/res/v1/web/search");
    let url = format!("{base}?q={}&count={limit}", percent_encode(query));
    let resp = ctx
        .http_client
        .get(&url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Brave Search request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Brave Search returned HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Brave JSON parse failed: {e}"))?;
    let mut results = Vec::new();
    if let Some(arr) = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|v| v.as_array())
    {
        for item in arr.iter().take(limit) {
            results.push(WebSearchResult {
                title: item.get("title").and_then(|v| v.as_str()).unwrap_or("").into(),
                url: item.get("url").and_then(|v| v.as_str()).unwrap_or("").into(),
                snippet: item.get("description").and_then(|v| v.as_str()).unwrap_or("").into(),
                source: "brave".into(),
            });
        }
    }
    Ok(results)
}

async fn search_perplexity(
    query: &str,
    endpoint_override: Option<&str>,
    ctx: &HermesToolContext,
) -> Result<Vec<WebSearchResult>, String> {
    let api_key = std::env::var("PERPLEXITY_API_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "PERPLEXITY_API_KEY is not configured".to_string())?;
    let url = endpoint_override.unwrap_or("https://api.perplexity.ai/chat/completions");
    let resp = ctx
        .http_client
        .post(url)
        .bearer_auth(api_key)
        .json(&json!({
            "model": "sonar",
            "messages": [{ "role": "user", "content": query }]
        }))
        .send()
        .await
        .map_err(|e| format!("Perplexity request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Perplexity returned HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Perplexity JSON parse failed: {e}"))?;
    let answer = body
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut results = vec![WebSearchResult {
        title: format!("Perplexity Answer: {query}"),
        url: "https://www.perplexity.ai".into(),
        snippet: answer.into(),
        source: "perplexity".into(),
    }];
    if let Some(citations) = body.get("citations").and_then(|c| c.as_array()) {
        for c in citations {
            if let Some(u) = c.as_str() {
                results.push(WebSearchResult {
                    title: u.to_string(),
                    url: u.to_string(),
                    snippet: String::new(),
                    source: "perplexity_citation".into(),
                });
            }
        }
    }
    Ok(results)
}

async fn search_searxng(
    query: &str,
    limit: usize,
    endpoint_override: Option<&str>,
    ctx: &HermesToolContext,
) -> Result<Vec<WebSearchResult>, String> {
    let base = endpoint_override
        .map(ToString::to_string)
        .or_else(|| std::env::var("SEARXNG_URL").ok())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "SearXNG endpoint_url or SEARXNG_URL is not configured".to_string())?;
    let url = format!(
        "{}/search?format=json&q={}",
        base.trim_end_matches('/'),
        percent_encode(query)
    );
    let resp = ctx
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("SearXNG request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("SearXNG returned HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("SearXNG JSON parse failed: {e}"))?;
    let mut results = Vec::new();
    if let Some(arr) = body.get("results").and_then(|v| v.as_array()) {
        for item in arr.iter().take(limit) {
            results.push(WebSearchResult {
                title: item.get("title").and_then(|v| v.as_str()).unwrap_or("").into(),
                url: item.get("url").and_then(|v| v.as_str()).unwrap_or("").into(),
                snippet: item.get("content").and_then(|v| v.as_str()).unwrap_or("").into(),
                source: "searxng".into(),
            });
        }
    }
    Ok(results)
}

async fn search_duckduckgo(
    query: &str,
    limit: usize,
    endpoint_override: Option<&str>,
    ctx: &HermesToolContext,
) -> Result<Vec<WebSearchResult>, String> {
    let base = endpoint_override.unwrap_or("https://api.duckduckgo.com/");
    let sep = if base.contains('?') { '&' } else { '?' };
    let url = format!("{base}{sep}q={}&format=json&no_html=1&skip_disambig=1", percent_encode(query));
    let resp = ctx
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("DuckDuckGo returned HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("DuckDuckGo JSON parse failed: {e}"))?;
    let mut results = Vec::new();
    let abstract_text = body.get("AbstractText").and_then(|v| v.as_str()).unwrap_or("");
    let abstract_url = body.get("AbstractURL").and_then(|v| v.as_str()).unwrap_or("");
    let heading = body.get("Heading").and_then(|v| v.as_str()).unwrap_or(query);
    if !abstract_text.is_empty() {
        results.push(WebSearchResult {
            title: heading.into(),
            url: abstract_url.into(),
            snippet: abstract_text.into(),
            source: "duckduckgo".into(),
        });
    }
    if let Some(topics) = body.get("RelatedTopics").and_then(|v| v.as_array()) {
        for topic in topics {
            if results.len() >= limit {
                break;
            }
            if let (Some(text), Some(first_url)) = (
                topic.get("Text").and_then(|v| v.as_str()),
                topic.get("FirstURL").and_then(|v| v.as_str()),
            ) {
                results.push(WebSearchResult {
                    title: text.split(" - ").next().unwrap_or(text).into(),
                    url: first_url.into(),
                    snippet: text.into(),
                    source: "duckduckgo".into(),
                });
            }
        }
    }
    Ok(results)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedWebPage {
    pub title: String,
    pub markdown: String,
    pub links: Vec<(String, String)>,
}

async fn tool_web_extract(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(url) = args.get("url").and_then(|v| v.as_str()).map(str::trim) else {
        return HermesToolResult::err("web_extract", "Missing required 'url' parameter");
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return HermesToolResult::err("web_extract", "web_extract only supports http:// and https:// URLs");
    }
    if url.contains("169.254.169.254") || url.contains("metadata.google.internal") {
        return HermesToolResult::err("web_extract", "Blocked cloud metadata endpoint URL");
    }

    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(20_000) as usize;
    let include_links = args
        .get("include_links")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let resp = match ctx.http_client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return HermesToolResult::err("web_extract", format!("HTTP fetch failed: {e}")),
    };
    let status = resp.status();
    if !status.is_success() {
        return HermesToolResult::err("web_extract", format!("Target URL returned HTTP {status}"));
    }
    let body = match resp.text().await {
        Ok(t) => t,
        Err(e) => return HermesToolResult::err("web_extract", format!("Failed reading body: {e}")),
    };

    let extracted = html_to_markdown(&body, include_links);
    let mut md = extracted.markdown;
    if md.chars().count() > max_chars {
        md = md.chars().take(max_chars).collect::<String>();
        md.push_str("\n...[truncated]...");
    }

    let scan = scan_content_for_injection(&md);
    let output = if scan.flagged {
        format!(
            "# {}\n\n[SECURITY WARNING: Potential indirect prompt-injection/secrets flagged]\n\n{}",
            extracted.title, scan.sanitized_text
        )
    } else if extracted.title.is_empty() {
        md.clone()
    } else {
        format!("# {}\n\n{}", extracted.title, md)
    };

    HermesToolResult::ok(
        "web_extract",
        output,
        Some(json!({
            "url": url,
            "title": extracted.title,
            "link_count": extracted.links.len(),
            "injection_flagged": scan.flagged,
        })),
    )
}

/// Pure-Rust HTML-to-Markdown converter stripping script/style/head and preserving headings, lists, code, links.
pub fn html_to_markdown(html: &str, include_links: bool) -> ExtractedWebPage {
    let title = extract_tag_inner(html, "title")
        .map(|s| decode_html_entities(s.trim()))
        .unwrap_or_default();

    let mut cleaned = strip_tag_blocks(html, "script");
    cleaned = strip_tag_blocks(&cleaned, "style");
    cleaned = strip_tag_blocks(&cleaned, "noscript");
    cleaned = strip_tag_blocks(&cleaned, "svg");
    cleaned = strip_tag_blocks(&cleaned, "head");

    let mut out = String::with_capacity(cleaned.len() / 2);
    let mut links = Vec::new();
    let bytes = cleaned.as_bytes();
    let mut i = 0;
    let mut link_href: Option<String> = None;
    let mut link_text = String::new();
    let mut in_pre = false;

    while i < cleaned.len() {
        if bytes[i] == b'<' {
            if let Some(rel_end) = cleaned[i..].find('>') {
                let tag_raw = &cleaned[i + 1..i + rel_end];
                let tag_trim = tag_raw.trim();
                let is_close = tag_trim.starts_with('/');
                let tag_name = tag_trim
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();

                match (tag_name.as_str(), is_close) {
                    ("h1", false) => out.push_str("\n# "),
                    ("h2", false) => out.push_str("\n## "),
                    ("h3", false) => out.push_str("\n### "),
                    ("h4", false) => out.push_str("\n#### "),
                    ("h5", false) => out.push_str("\n##### "),
                    ("h6", false) => out.push_str("\n###### "),
                    ("h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article" | "blockquote", true) => {
                        out.push('\n');
                    }
                    ("p" | "div" | "br" | "tr", false) => out.push('\n'),
                    ("li", false) => out.push_str("\n- "),
                    ("pre", false) => {
                        in_pre = true;
                        out.push_str("\n```\n");
                    }
                    ("pre", true) => {
                        in_pre = false;
                        out.push_str("\n```\n");
                    }
                    ("code", false) if !in_pre => out.push('`'),
                    ("code", true) if !in_pre => out.push('`'),
                    ("strong" | "b", _) => out.push_str("**"),
                    ("em" | "i", _) => out.push('*'),
                    ("a", false) if include_links => {
                        link_href = extract_attr(tag_raw, "href");
                        link_text.clear();
                    }
                    ("a", true) if include_links => {
                        if let Some(href) = link_href.take() {
                            let label = link_text.trim().to_string();
                            if !label.is_empty() && !href.is_empty() {
                                links.push((label.clone(), href.clone()));
                                out.push_str(&format!("[{label}]({href})"));
                            } else {
                                out.push_str(&label);
                            }
                            link_text.clear();
                        }
                    }
                    _ => {}
                }
                i += rel_end + 1;
                continue;
            }
        }

        let next_tag = cleaned[i..].find('<').map(|p| i + p).unwrap_or(cleaned.len());
        let chunk = decode_html_entities(&cleaned[i..next_tag]);
        if link_href.is_some() {
            link_text.push_str(&chunk);
        } else if in_pre {
            out.push_str(&chunk);
        } else {
            let collapsed = collapse_inline_whitespace(&chunk);
            if !collapsed.is_empty() {
                if !out.ends_with(['\n', ' ', '`', '*']) && !collapsed.starts_with([' ', '.', ',', ':', ';', '!', '?']) {
                    out.push(' ');
                }
                out.push_str(&collapsed);
            }
        }
        i = next_tag;
    }

    let normalized = normalize_markdown_newlines(&out);
    ExtractedWebPage {
        title,
        markdown: normalized,
        links,
    }
}

fn extract_tag_inner(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = lower.find(&open)?;
    let after_open = html[start..].find('>')? + start + 1;
    let end = lower[after_open..].find(&close)? + after_open;
    Some(html[after_open..end].to_string())
}

fn strip_tag_blocks(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(html.len());
    let mut idx = 0;
    while idx < html.len() {
        if let Some(rel) = lower[idx..].find(&open) {
            let start = idx + rel;
            out.push_str(&html[idx..start]);
            if let Some(close_rel) = lower[start..].find(&close) {
                idx = start + close_rel + close.len();
            } else {
                break;
            }
        } else {
            out.push_str(&html[idx..]);
            break;
        }
    }
    out
}

fn extract_attr(tag_raw: &str, attr: &str) -> Option<String> {
    let lower = tag_raw.to_ascii_lowercase();
    let needle = format!("{attr}=");
    let pos = lower.find(&needle)?;
    let rest = tag_raw[pos + needle.len()..].trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        stripped.split('"').next().map(ToString::to_string)
    } else if let Some(stripped) = rest.strip_prefix('\'') {
        stripped.split('\'').next().map(ToString::to_string)
    } else {
        rest.split_whitespace().next().map(ToString::to_string)
    }
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn collapse_inline_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out
}

fn normalize_markdown_newlines(s: &str) -> String {
    let mut lines = Vec::new();
    let mut blank_run = 0usize;
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !lines.is_empty() {
                lines.push(String::new());
            }
        } else {
            blank_run = 0;
            lines.push(trimmed.to_string());
        }
    }
    lines.join("\n").trim().to_string()
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'_' | b'.' | b'~') {
            out.push(*b as char);
        } else if *b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

// ----------------------------------------------------------------------------
// Pure Obscura Browser & Zero-Knowledge Vault / Assist Tools
// ----------------------------------------------------------------------------

fn explicit_session_id(args: &Value) -> Option<&str> {
    args.get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn resolve_session_id(args: &Value, ctx: &HermesToolContext) -> String {
    explicit_session_id(args)
        .map(ToString::to_string)
        .unwrap_or_else(|| ctx.session_id.clone())
}

enum AcquiredBrowserSession {
    TurnScoped(Arc<BrowserAutomationSession>),
    Standalone(AiTurnGuard),
}

impl AcquiredBrowserSession {
    fn session(&self) -> &Arc<BrowserAutomationSession> {
        match self {
            Self::TurnScoped(s) => s,
            Self::Standalone(g) => g.session(),
        }
    }
}

async fn resolve_and_lock_session(
    tool_name: &str,
    args: &Value,
    ctx: &HermesToolContext,
    origin_hint: Option<&NormalizedOrigin>,
) -> Result<AcquiredBrowserSession, HermesToolResult> {
    let Some(ref registry) = ctx.browser_sessions else {
        return Err(HermesToolResult::err(
            tool_name,
            "No BrowserSessionRegistry attached to HermesToolContext",
        ));
    };
    let explicit_sid = explicit_session_id(args);
    let cdp_url = resolve_obscura_endpoint(args, ctx);
    let session = registry
        .resolve_session(
            explicit_sid,
            &ctx.session_id,
            ctx.bot_browser.as_ref(),
            cdp_url.as_deref(),
            origin_hint,
        )
        .await
        .map_err(|e| HermesToolResult::err(tool_name, e.to_string()))?;

    let canonical_sid = session.session_id().to_string();
    if let Some(ref guard_map) = ctx.browser_ai_guards {
        let mut guards = guard_map.lock().await;
        if !guards.contains_key(&canonical_sid) {
            let guard = session
                .try_begin_ai_turn()
                .map_err(|e| HermesToolResult::err(tool_name, e.to_string()))?;
            guards.insert(canonical_sid, guard);
        }
        Ok(AcquiredBrowserSession::TurnScoped(session))
    } else {
        let guard = session
            .try_begin_standalone_step()
            .map_err(|e| HermesToolResult::err(tool_name, e.to_string()))?;
        Ok(AcquiredBrowserSession::Standalone(guard))
    }
}

fn parse_semantic_locator(args: &Value, allow_text_field: bool) -> SemanticLocator {
    let get_opt = |k: &str| -> Option<String> {
        args.get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    };
    let css_selector = get_opt("css_selector").or_else(|| get_opt("selector"));
    let text_anchor = get_opt("text_anchor").or_else(|| {
        if allow_text_field {
            get_opt("text")
        } else {
            None
        }
    });
    SemanticLocator {
        test_id: get_opt("test_id"),
        aria_label: get_opt("aria_label"),
        role: get_opt("role"),
        text_anchor,
        css_selector,
        name_attr: get_opt("name_attr"),
    }
}

fn has_non_css_semantic_fields(locator: &SemanticLocator) -> bool {
    locator.test_id.is_some()
        || locator.aria_label.is_some()
        || locator.role.is_some()
        || locator.text_anchor.is_some()
        || locator.name_attr.is_some()
}

fn attach_resumed_context(mut res: HermesToolResult, gate_info: &StepGateInfo) -> HermesToolResult {
    if !gate_info.resumed_from_takeover {
        return res;
    }
    let url_str = gate_info.refreshed_url.as_deref().unwrap_or("");
    let title_str = gate_info.refreshed_title.as_deref().unwrap_or("");
    let excerpt = gate_info
        .refreshed_snapshot
        .as_ref()
        .map(|s| s.text_excerpt.as_str())
        .unwrap_or("");
    res.output = format!(
        "[Resumed from user takeover — live URL: {url_str}, title: {title_str}, excerpt: {excerpt}]\n{}",
        res.output
    );
    let resumed_meta = json!({
        "resumed_from_takeover": true,
        "refreshed_url": gate_info.refreshed_url,
        "refreshed_title": gate_info.refreshed_title,
        "refreshed_snapshot": gate_info.refreshed_snapshot,
    });
    match &mut res.structured {
        Some(Value::Object(map)) => {
            map.insert("resumed_from_takeover".into(), json!(true));
            if let Some(ref u) = gate_info.refreshed_url {
                map.insert("refreshed_url".into(), json!(u));
            }
            if let Some(ref t) = gate_info.refreshed_title {
                map.insert("refreshed_title".into(), json!(t));
            }
            if let Some(ref s) = gate_info.refreshed_snapshot {
                if let Ok(sv) = serde_json::to_value(s) {
                    map.insert("refreshed_snapshot".into(), sv);
                }
            }
        }
        _ => {
            res.structured = Some(resumed_meta);
        }
    }
    res
}

fn resolve_task_id(args: &Value, ctx: &HermesToolContext) -> String {
    args.get("task_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("task_{}_{}", ctx.session_id, ctx.turn_index))
}

fn resolve_obscura_endpoint(args: &Value, ctx: &HermesToolContext) -> Option<String> {
    args.get("cdp_url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            ctx.browser_cdp_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            std::env::var("HERMES_BROWSER_CDP_URL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

fn require_orchestrator<'a>(
    tool_name: &str,
    ctx: &'a HermesToolContext,
) -> Result<&'a Arc<BotBrowserOrchestrator>, HermesToolResult> {
    ctx.bot_browser.as_ref().ok_or_else(|| {
        HermesToolResult::err(
            tool_name,
            "Obscura BotBrowserOrchestrator is not attached to HermesToolContext",
        )
    })
}

fn direct_obscura_bridge(
    tool_name: &str,
    args: &Value,
    ctx: &HermesToolContext,
) -> Result<CdpBridge, HermesToolResult> {
    let Some(endpoint) = resolve_obscura_endpoint(args, ctx) else {
        return Err(HermesToolResult::err(
            tool_name,
            "Obscura browser engine is not attached (HermesToolContext.bot_browser is None) and no Obscura CDP endpoint (cdp_url / HERMES_BROWSER_CDP_URL) was provided.",
        ));
    };
    let transport = WsCdpTransport::with_client(
        endpoint,
        ctx.http_client.clone(),
        std::time::Duration::from_secs(15),
    );
    Ok(CdpBridge::new(Arc::new(transport)))
}

fn parse_login_selectors(args: &Value) -> Result<LoginFormSelectors, String> {
    let nested = args.get("selectors");
    let extract = |key: &str| -> Option<String> {
        args.get(key)
            .and_then(|v| v.as_str())
            .or_else(|| nested.and_then(|obj| obj.get(key)).and_then(|v| v.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    };

    let username_selector = extract("username_selector")
        .ok_or_else(|| "Missing required 'username_selector'".to_string())?;
    let password_selector = extract("password_selector")
        .ok_or_else(|| "Missing required 'password_selector'".to_string())?;
    let totp_selector = extract("totp_selector");
    let submit_selector = extract("submit_selector");

    Ok(LoginFormSelectors {
        username_selector,
        password_selector,
        totp_selector,
        submit_selector,
    })
}

fn serialize_tool_json<T: Serialize>(tool_name: &str, value: &T) -> HermesToolResult {
    match serde_json::to_value(value) {
        Ok(structured) => {
            let output = serde_json::to_string_pretty(&structured).unwrap_or_default();
            HermesToolResult::ok(tool_name, output, Some(structured))
        }
        Err(e) => HermesToolResult::err(tool_name, format!("Failed serializing result: {e}")),
    }
}

fn snapshot_to_tool_result(snapshot: &BrowserSnapshot) -> HermesToolResult {
    serialize_tool_json("browser_snapshot", snapshot)
}

fn normalize_selector_for_sensitivity(selector: &str) -> String {
    let chars: Vec<char> = selector.chars().collect();
    let mut out = String::with_capacity(selector.len() + 8);
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '-' {
            out.push('_');
            continue;
        }
        if ch.is_ascii_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_ascii_uppercase() && next_is_lower)
            {
                out.push('_');
            }
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn is_sensitive_selector_str(selector: &str) -> bool {
    let lower = selector.to_ascii_lowercase();
    let normalized = normalize_selector_for_sensitivity(selector);
    const MARKERS: &[&str] = &[
        "password",
        "passwd",
        "passcode",
        "pwd",
        "one-time-code",
        "one_time_code",
        "totp",
        "secret",
        "api_key",
        "apikey",
        "auth_token",
        "access_token",
        "refresh_token",
        "id_token",
        "oauth_token",
        "bearer_token",
        "session_id",
        "ct0",
    ];
    MARKERS
        .iter()
        .any(|m| lower.contains(m) || normalized.contains(m))
        || [lower.as_str(), normalized.as_str()].into_iter().any(|s| {
            s.split(|c: char| !c.is_ascii_alphanumeric())
                .any(|tok| matches!(tok, "otp" | "token" | "pin" | "bearer" | "cookie"))
        })
}

fn is_sensitive_locator(locator: &SemanticLocator, raw_selector: Option<&str>) -> bool {
    locator.is_sensitive_hint()
        || raw_selector.is_some_and(is_sensitive_selector_str)
        || [
            locator.css_selector.as_deref(),
            locator.name_attr.as_deref(),
            locator.test_id.as_deref(),
            locator.aria_label.as_deref(),
            locator.text_anchor.as_deref(),
            locator.role.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(is_sensitive_selector_str)
}

async fn tool_browser_navigate(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(url) = args.get("url").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
        return HermesToolResult::err("browser_navigate", "Missing required 'url'");
    };
    let origin = match NormalizedOrigin::parse(url) {
        Ok(o) => {
            let host = o.host();
            if host == "169.254.169.254" || host == "metadata.google.internal" {
                return HermesToolResult::err("browser_navigate", "Blocked cloud metadata endpoint URL");
            }
            o
        }
        Err(e) => return HermesToolResult::err("browser_navigate", e.to_string()),
    };

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_navigate", &args, ctx, Some(&origin)).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess
            .prepare_step("browser_navigate", None, Some(url.to_string()))
            .await
        {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_navigate", e.to_string()),
        };

        if sess.auto_pause_on_sensitive_origin() && origin.is_high_sensitivity() {
            if let Err(e) = sess
                .request_takeover_and_wait(
                    "High-sensitivity origin navigation requires user confirmation or takeover",
                    true,
                    true,
                )
                .await
            {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_navigate",
                    0,
                    e.to_string(),
                );
                return HermesToolResult::err("browser_navigate", e.to_string());
            }
        }

        let start = Instant::now();
        let nav_res: Result<(String, Option<Value>), BotBrowserError> = match sess.source() {
            SessionBackendSource::Orchestrator { orchestrator, .. } => orchestrator
                .browser_navigate(sess.session_id(), url)
                .await
                .map(|r| (r.url.clone(), serde_json::to_value(&r).ok())),
            SessionBackendSource::Registered(backend) => backend
                .navigate(url)
                .await
                .map(|r| (r.url.clone(), serde_json::to_value(&r).ok())),
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match nav_res {
            Ok((final_url, mut val)) => {
                let redacted_url = redact_url_for_trace(&final_url);
                if let Some(Value::Object(ref mut map)) = val {
                    if map.contains_key("url") {
                        map.insert("url".to_string(), json!(redacted_url));
                    }
                }
                sess.record_navigate(&final_url, elapsed_ms);
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_navigate",
                    elapsed_ms,
                    None,
                    Some(final_url.clone()),
                );
                let tool_res = HermesToolResult::ok(
                    "browser_navigate",
                    format!(
                        "Navigated session '{}' to {}",
                        sess.session_id(),
                        redacted_url
                    ),
                    val,
                );
                attach_resumed_context(tool_res, &gate_info)
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_navigate",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("browser_navigate", e.to_string())
            }
        };
    }

    let session_id = resolve_session_id(&args, ctx);
    let redacted_url = redact_url_for_trace(url);
    if let Some(orch) = ctx.bot_browser.as_ref() {
        return match orch.browser_navigate(&session_id, url).await {
            Ok(mut res) => {
                res.url = redact_url_for_trace(&res.url);
                HermesToolResult::ok(
                    "browser_navigate",
                    format!("Navigated Obscura session '{session_id}' to {redacted_url}"),
                    serde_json::to_value(&res).ok(),
                )
            }
            Err(e) => HermesToolResult::err("browser_navigate", e.to_string()),
        };
    }

    let bridge = match direct_obscura_bridge("browser_navigate", &args, ctx) {
        Ok(b) => b,
        Err(err_res) => return err_res,
    };
    match bridge.navigate(url).await {
        Ok(raw) => HermesToolResult::ok(
            "browser_navigate",
            format!("Navigated Obscura page to {redacted_url}"),
            Some(json!({ "session_id": session_id, "url": redacted_url, "origin": origin.as_str(), "cdp": raw })),
        ),
        Err(e) => HermesToolResult::err("browser_navigate", e.to_string()),
    }
}

async fn tool_browser_snapshot(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let selector = args
        .get("selector")
        .or_else(|| args.get("css_selector"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_snapshot", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let loc = selector.map(SemanticLocator::by_css);
        let gate_info = match sess.prepare_step("browser_snapshot", loc.clone(), None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_snapshot", e.to_string()),
        };
        let start = Instant::now();
        let snap_res = match sess.source() {
            SessionBackendSource::Orchestrator { orchestrator, .. } => {
                orchestrator
                    .tool_browser_snapshot(sess.session_id(), selector)
                    .await
            }
            SessionBackendSource::Registered(backend) => backend.snapshot_dom().await,
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match snap_res {
            Ok(mut snapshot) => {
                redact_snapshot_for_observation(&mut snapshot);
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_snapshot",
                    elapsed_ms,
                    loc,
                    Some(snapshot.url.clone()),
                );
                attach_resumed_context(snapshot_to_tool_result(&snapshot), &gate_info)
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_snapshot",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("browser_snapshot", e.to_string())
            }
        };
    }

    let session_id = resolve_session_id(&args, ctx);
    if let Some(orch) = ctx.bot_browser.as_ref() {
        return match orch.tool_browser_snapshot(&session_id, selector).await {
            Ok(snapshot) => snapshot_to_tool_result(&snapshot),
            Err(e) => HermesToolResult::err("browser_snapshot", e.to_string()),
        };
    }

    let bridge = match direct_obscura_bridge("browser_snapshot", &args, ctx) {
        Ok(b) => b,
        Err(err_res) => return err_res,
    };
    match bridge.capture_redacted_snapshot(selector).await {
        Ok(snapshot) => snapshot_to_tool_result(&snapshot),
        Err(e) => HermesToolResult::err("browser_snapshot", e.to_string()),
    }
}

async fn tool_browser_click(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let locator = parse_semantic_locator(&args, false);
    let effective_selector = selector.or(locator.css_selector.as_deref());
    let x = args.get("x").and_then(|v| v.as_f64());
    let y = args.get("y").and_then(|v| v.as_f64());

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_click", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let opt_loc = (!locator.is_empty()).then(|| locator.clone());
        let gate_info = match sess.prepare_step("browser_click", opt_loc.clone(), None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_click", e.to_string()),
        };
        let start = Instant::now();
        let use_orch = matches!(sess.source(), SessionBackendSource::Orchestrator { .. })
            && (x.is_some()
                || (effective_selector.is_some() && !has_non_css_semantic_fields(&locator)));
        let click_res: Result<Value, BotBrowserError> = if use_orch {
            match sess.source() {
                SessionBackendSource::Orchestrator { orchestrator, .. } => orchestrator
                    .browser_click(sess.session_id(), effective_selector, x, y)
                    .await
                    .and_then(|r| {
                        serde_json::to_value(&r)
                            .map_err(|e| BotBrowserError::Cdp(e.to_string()))
                    }),
                _ => unreachable!(),
            }
        } else {
            let backend = sess.backend();
            if !locator.is_empty() {
                backend.click_locator(&locator).await.and_then(|r| {
                    serde_json::to_value(&r).map_err(|e| BotBrowserError::Cdp(e.to_string()))
                })
            } else if let (Some(px), Some(py)) = (x, y) {
                match backend
                    .dispatch_pointer_event(PointerInputEvent {
                        kind: PointerEventKind::Down,
                        x: px,
                        y: py,
                        button: PointerButton::Left,
                        click_count: 1,
                        delta_x: 0.0,
                        delta_y: 0.0,
                        modifiers: 0,
                    })
                    .await
                {
                    Ok(()) => backend
                        .dispatch_pointer_event(PointerInputEvent {
                            kind: PointerEventKind::Up,
                            x: px,
                            y: py,
                            button: PointerButton::Left,
                            click_count: 1,
                            delta_x: 0.0,
                            delta_y: 0.0,
                            modifiers: 0,
                        })
                        .await
                        .map(|()| {
                            json!({ "ok": true, "operation": "click_coords", "x": px, "y": py })
                        }),
                    Err(e) => Err(e),
                }
            } else {
                Err(BotBrowserError::Cdp(
                    "Provide a semantic locator / 'selector' or ('x', 'y')".to_string(),
                ))
            }
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match click_res {
            Ok(val) => {
                let page_url = sess.backend().current_url().await.ok();
                if let Some(ref rec_loc) = opt_loc {
                    sess.record_click(page_url.clone(), rec_loc.clone(), elapsed_ms);
                }
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_click",
                    elapsed_ms,
                    opt_loc,
                    page_url,
                );
                let output = serde_json::to_string_pretty(&val).unwrap_or_default();
                attach_resumed_context(
                    HermesToolResult::ok("browser_click", output, Some(val)),
                    &gate_info,
                )
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_click",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("browser_click", e.to_string())
            }
        };
    }

    let session_id = resolve_session_id(&args, ctx);
    if let Some(orch) = ctx.bot_browser.as_ref() {
        return match orch.browser_click(&session_id, effective_selector, x, y).await {
            Ok(res) => serialize_tool_json("browser_click", &res),
            Err(e) => HermesToolResult::err("browser_click", e.to_string()),
        };
    }

    let bridge = match direct_obscura_bridge("browser_click", &args, ctx) {
        Ok(b) => b,
        Err(err_res) => return err_res,
    };
    if let Some(sel) = effective_selector {
        return match bridge.click_selector(sel).await {
            Ok(res) => HermesToolResult::ok(
                "browser_click",
                format!("Clicked selector '{sel}'"),
                Some(res),
            ),
            Err(e) => HermesToolResult::err("browser_click", e.to_string()),
        };
    }
    if let (Some(px), Some(py)) = (x, y) {
        return match bridge.click_coords(px, py).await {
            Ok(res) => HermesToolResult::ok(
                "browser_click",
                format!("Clicked coordinates ({px}, {py})"),
                Some(res),
            ),
            Err(e) => HermesToolResult::err("browser_click", e.to_string()),
        };
    }
    HermesToolResult::err("browser_click", "Provide either 'selector' or ('x', 'y')")
}

async fn tool_browser_type(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(text) = args.get("text").and_then(|v| v.as_str()) else {
        return HermesToolResult::err("browser_type", "Missing required 'text'");
    };
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let locator = parse_semantic_locator(&args, false);
    let effective_selector = selector.or(locator.css_selector.as_deref());
    let submit = args.get("submit").and_then(|v| v.as_bool()).unwrap_or(false);
    let param_hint = args
        .get("parameter_name_hint")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_type", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let opt_loc = (!locator.is_empty()).then(|| locator.clone());
        let gate_info = match sess.prepare_step("browser_type", opt_loc.clone(), None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_type", e.to_string()),
        };

        let start = Instant::now();
        if is_sensitive_locator(&locator, effective_selector) {
            let err_msg = "Refusing to type into password/OTP/token field via browser_type; use vault_request_card / vault_fill.";
            sess.emit_step_failed(
                gate_info.step_index,
                "browser_type",
                start.elapsed().as_millis() as u64,
                err_msg,
            );
            return HermesToolResult::err("browser_type", err_msg);
        }

        let use_orch = matches!(sess.source(), SessionBackendSource::Orchestrator { .. })
            && !has_non_css_semantic_fields(&locator);

        let type_res: Result<Value, BotBrowserError> = if use_orch {
            match sess.source() {
                SessionBackendSource::Orchestrator { orchestrator, .. } => orchestrator
                    .browser_type(sess.session_id(), effective_selector, text, submit)
                    .await
                    .and_then(|r| {
                        serde_json::to_value(&r)
                            .map_err(|e| BotBrowserError::Cdp(e.to_string()))
                    }),
                _ => unreachable!(),
            }
        } else {
            let backend = sess.backend();
            if !locator.is_empty() {
                match backend.fill_locator(&locator, text, false).await {
                    Ok(outcome) => {
                        if submit {
                            let _ = backend.press_key("Enter").await;
                        }
                        serde_json::to_value(&outcome)
                            .map_err(|e| BotBrowserError::Cdp(e.to_string()))
                    }
                    Err(e) => Err(e),
                }
            } else {
                let mut err_opt = None;
                for ch in text.chars() {
                    if let Err(e) = backend
                        .dispatch_key_event(KeyInputEvent {
                            kind: KeyEventKind::Char,
                            key: ch.to_string(),
                            code: None,
                            text: Some(ch.to_string()),
                            windows_virtual_key_code: None,
                            modifiers: 0,
                        })
                        .await
                    {
                        err_opt = Some(e);
                        break;
                    }
                }
                if let Some(e) = err_opt {
                    Err(e)
                } else {
                    if submit {
                        let _ = backend.press_key("Enter").await;
                    }
                    Ok(json!({
                        "ok": true,
                        "chars_typed": text.chars().count(),
                        "submitted": submit
                    }))
                }
            }
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match type_res {
            Ok(val) => {
                let page_url = sess.backend().current_url().await.ok();
                if let Some(ref rec_loc) = opt_loc {
                    sess.record_fill(
                        page_url.clone(),
                        rec_loc.clone(),
                        text,
                        false,
                        None,
                        param_hint,
                        elapsed_ms,
                    );
                }
                if submit {
                    sess.record_press_key(page_url.clone(), "Enter".to_string(), 0);
                }
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_type",
                    elapsed_ms,
                    opt_loc,
                    page_url,
                );
                let output = serde_json::to_string_pretty(&val).unwrap_or_default();
                attach_resumed_context(
                    HermesToolResult::ok("browser_type", output, Some(val)),
                    &gate_info,
                )
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_type",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("browser_type", e.to_string())
            }
        };
    }

    if is_sensitive_locator(&locator, effective_selector) {
        return HermesToolResult::err(
            "browser_type",
            "Refusing to type into password/OTP/token field via browser_type; use vault_request_card / vault_fill.",
        );
    }

    let session_id = resolve_session_id(&args, ctx);
    if let Some(orch) = ctx.bot_browser.as_ref() {
        return match orch
            .browser_type(&session_id, effective_selector, text, submit)
            .await
        {
            Ok(res) => serialize_tool_json("browser_type", &res),
            Err(e) => HermesToolResult::err("browser_type", e.to_string()),
        };
    }
    let bridge = match direct_obscura_bridge("browser_type", &args, ctx) {
        Ok(b) => b,
        Err(err_res) => return err_res,
    };
    if let Some(sel) = effective_selector {
        let sel_json = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".to_string());
        let focus_expr = format!("document.querySelector({sel_json})?.focus()");
        if let Err(e) = bridge.evaluate_script(&focus_expr).await {
            return HermesToolResult::err("browser_type", e.to_string());
        }
    }
    match bridge.insert_text(text).await {
        Ok(_) => {
            if submit {
                let _ = bridge
                    .transport()
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
                    .await;
                let _ = bridge
                    .transport()
                    .send_command(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": "keyUp",
                            "key": "Enter",
                            "code": "Enter",
                            "windowsVirtualKeyCode": 13
                        }),
                    )
                    .await;
            }
            HermesToolResult::ok(
                "browser_type",
                format!("Typed {} character(s)", text.chars().count()),
                Some(json!({ "chars_typed": text.chars().count(), "submitted": submit })),
            )
        }
        Err(e) => HermesToolResult::err("browser_type", e.to_string()),
    }
}

async fn tool_browser_screenshot(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let full_page = args
        .get("full_page")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (png_bytes, gate_info_opt, session_opt) = if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_screenshot", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess.prepare_step("browser_screenshot", None, None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_screenshot", e.to_string()),
        };
        let start = Instant::now();
        let shot_res = match sess.source() {
            SessionBackendSource::Orchestrator { orchestrator, .. } => {
                orchestrator
                    .browser_screenshot(sess.session_id(), full_page)
                    .await
            }
            SessionBackendSource::Registered(backend) => backend.capture_screenshot().await,
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        match shot_res {
            Ok(bytes) => {
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_screenshot",
                    elapsed_ms,
                    None,
                    None,
                );
                (bytes, Some(gate_info), Some(Arc::clone(sess)))
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_screenshot",
                    elapsed_ms,
                    e.to_string(),
                );
                return HermesToolResult::err("browser_screenshot", e.to_string());
            }
        }
    } else {
        let session_id = resolve_session_id(&args, ctx);
        let bytes = if let Some(orch) = ctx.bot_browser.as_ref() {
            match orch.browser_screenshot(&session_id, full_page).await {
                Ok(bytes) => bytes,
                Err(e) => return HermesToolResult::err("browser_screenshot", e.to_string()),
            }
        } else {
            let bridge = match direct_obscura_bridge("browser_screenshot", &args, ctx) {
                Ok(b) => b,
                Err(err_res) => return err_res,
            };
            match bridge.capture_screenshot_png(full_page).await {
                Ok(bytes) => bytes,
                Err(e) => return HermesToolResult::err("browser_screenshot", e.to_string()),
            }
        };
        (bytes, None, None)
    };
    let _ = session_opt;

    let mut out_path = None;
    if let Some(raw_p) = args.get("output_path").and_then(|v| v.as_str()) {
        if let Ok(sec) = ctx.security.read() {
            if let Ok(resolved) = sec.validate_path(Path::new(raw_p)) {
                if let Some(parent) = resolved.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if fs::write(&resolved, &png_bytes).is_ok() {
                    out_path = Some(resolved.display().to_string());
                }
            }
        }
    }

    let b64 = base64_encode(&png_bytes);
    let res = HermesToolResult::ok(
        "browser_screenshot",
        format!("Captured Obscura screenshot ({} bytes)", png_bytes.len()),
        Some(json!({ "format": "png", "byte_len": png_bytes.len(), "output_path": out_path })),
    )
    .with_artifact(HermesToolArtifact {
        kind: "image".into(),
        name: "browser_screenshot.png".into(),
        mime_type: "image/png".into(),
        path: out_path,
        data_base64: Some(b64),
        metadata: json!({ "source": "obscura" }),
    });
    if let Some(ref gate_info) = gate_info_opt {
        attach_resumed_context(res, gate_info)
    } else {
        res
    }
}

async fn tool_browser_press_key(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(key) = args
        .get("key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return HermesToolResult::err("browser_press_key", "Missing required 'key'");
    };

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_press_key", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess.prepare_step("browser_press_key", None, None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_press_key", e.to_string()),
        };
        let start = Instant::now();
        let res = sess.backend().press_key(key).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match res {
            Ok(outcome) => {
                let page_url = sess.backend().current_url().await.ok();
                sess.record_press_key(page_url.clone(), key.to_string(), elapsed_ms);
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_press_key",
                    elapsed_ms,
                    None,
                    page_url,
                );
                attach_resumed_context(serialize_tool_json("browser_press_key", &outcome), &gate_info)
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_press_key",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("browser_press_key", e.to_string())
            }
        };
    }

    let backend = match resolve_fallback_backend("browser_press_key", &args, ctx).await {
        Ok(b) => b,
        Err(err_res) => return err_res,
    };
    match backend.press_key(key).await {
        Ok(outcome) => serialize_tool_json("browser_press_key", &outcome),
        Err(e) => HermesToolResult::err("browser_press_key", e.to_string()),
    }
}

async fn tool_browser_scroll(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let delta_x = args.get("delta_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let delta_y = args.get("delta_y").and_then(|v| v.as_f64()).unwrap_or(400.0);

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_scroll", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess.prepare_step("browser_scroll", None, None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_scroll", e.to_string()),
        };
        let start = Instant::now();
        let res = sess.backend().scroll(delta_x, delta_y).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match res {
            Ok(outcome) => {
                let page_url = sess.backend().current_url().await.ok();
                sess.record_scroll(page_url.clone(), delta_x, delta_y, elapsed_ms);
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_scroll",
                    elapsed_ms,
                    None,
                    page_url,
                );
                attach_resumed_context(serialize_tool_json("browser_scroll", &outcome), &gate_info)
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_scroll",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("browser_scroll", e.to_string())
            }
        };
    }

    let backend = match resolve_fallback_backend("browser_scroll", &args, ctx).await {
        Ok(b) => b,
        Err(err_res) => return err_res,
    };
    match backend.scroll(delta_x, delta_y).await {
        Ok(outcome) => serialize_tool_json("browser_scroll", &outcome),
        Err(e) => HermesToolResult::err("browser_scroll", e.to_string()),
    }
}

async fn tool_browser_wait_for(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let locator = parse_semantic_locator(&args, false);
    if locator.is_empty() {
        return HermesToolResult::err(
            "browser_wait_for",
            "Provide a semantic locator (test_id, aria_label, role, text_anchor, name_attr) or selector",
        );
    }
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5_000)
        .clamp(50, 30_000);

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_wait_for", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess
            .prepare_step("browser_wait_for", Some(locator.clone()), None)
            .await
        {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_wait_for", e.to_string()),
        };
        let start = Instant::now();
        let res = sess.backend().wait_for_locator(&locator, timeout_ms).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match res {
            Ok(outcome) => {
                let page_url = sess.backend().current_url().await.ok();
                sess.record_wait_for(page_url.clone(), locator.clone(), timeout_ms, elapsed_ms);
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_wait_for",
                    elapsed_ms,
                    Some(locator),
                    page_url,
                );
                attach_resumed_context(serialize_tool_json("browser_wait_for", &outcome), &gate_info)
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_wait_for",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("browser_wait_for", e.to_string())
            }
        };
    }

    let backend = match resolve_fallback_backend("browser_wait_for", &args, ctx).await {
        Ok(b) => b,
        Err(err_res) => return err_res,
    };
    match backend.wait_for_locator(&locator, timeout_ms).await {
        Ok(outcome) => serialize_tool_json("browser_wait_for", &outcome),
        Err(e) => HermesToolResult::err("browser_wait_for", e.to_string()),
    }
}

async fn tool_browser_extract_text(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let locator = parse_semantic_locator(&args, false);
    if locator.is_empty() {
        return HermesToolResult::err(
            "browser_extract_text",
            "Provide a semantic locator (test_id, aria_label, role, text_anchor, name_attr) or selector",
        );
    }
    if is_sensitive_locator(&locator, selector) {
        return HermesToolResult::err(
            "browser_extract_text",
            "Refusing to extract text from a sensitive/password element.",
        );
    }
    let output_key = args
        .get("output_key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("extracted_text")
        .to_string();

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("browser_extract_text", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess
            .prepare_step("browser_extract_text", Some(locator.clone()), None)
            .await
        {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("browser_extract_text", e.to_string()),
        };
        let start = Instant::now();
        let res = sess.backend().extract_text_locator(&locator).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match res {
            Ok(raw_text) => {
                let (mut safe_text, _) = SecretRedactor::new().redact_text(&raw_text);
                if looks_like_secret_value(&safe_text) {
                    safe_text = "[REDACTED]".to_string();
                }
                let page_url = sess.backend().current_url().await.ok();
                sess.record_extract_text(
                    page_url.clone(),
                    locator.clone(),
                    output_key.clone(),
                    elapsed_ms,
                );
                sess.emit_step_completed(
                    gate_info.step_index,
                    "browser_extract_text",
                    elapsed_ms,
                    Some(locator),
                    page_url,
                );
                let tool_res = HermesToolResult::ok(
                    "browser_extract_text",
                    safe_text.clone(),
                    Some(json!({ "output_key": output_key, "text": safe_text })),
                );
                attach_resumed_context(tool_res, &gate_info)
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "browser_extract_text",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("browser_extract_text", e.to_string())
            }
        };
    }

    let backend = match resolve_fallback_backend("browser_extract_text", &args, ctx).await {
        Ok(b) => b,
        Err(err_res) => return err_res,
    };
    match backend.extract_text_locator(&locator).await {
        Ok(raw_text) => {
            let (mut safe_text, _) = SecretRedactor::new().redact_text(&raw_text);
            if looks_like_secret_value(&safe_text) {
                safe_text = "[REDACTED]".to_string();
            }
            HermesToolResult::ok(
                "browser_extract_text",
                safe_text.clone(),
                Some(json!({ "output_key": output_key, "text": safe_text })),
            )
        }
        Err(e) => HermesToolResult::err("browser_extract_text", e.to_string()),
    }
}

async fn tool_browser_request_takeover(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(reason) = args
        .get("reason")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return HermesToolResult::err("browser_request_takeover", "Missing required 'reason'");
    };
    let wait_for_finish = args
        .get("wait_for_finish")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let record_checkpoint = args
        .get("record_checkpoint")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let acquired = match resolve_and_lock_session("browser_request_takeover", &args, ctx, None).await {
        Ok(a) => a,
        Err(err_res) => return err_res,
    };
    let sess = acquired.session();
    let gate_info = match sess
        .prepare_step("browser_request_takeover", None, None)
        .await
    {
        Ok(g) => g,
        Err(e) => return HermesToolResult::err("browser_request_takeover", e.to_string()),
    };
    let sid = sess.session_id().to_string();

    let ui_action = HermesUiAction {
        action_type: "browser_takeover_requested".to_string(),
        payload: json!({
            "session_id": sid,
            "reason": reason,
            "wait_for_finish": wait_for_finish,
        }),
    };
    ctx.emit_ui(&ui_action);

    let start = Instant::now();
    match sess
        .request_takeover_and_wait(reason, wait_for_finish, record_checkpoint)
        .await
    {
        Ok(outcome) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            sess.emit_step_completed(
                gate_info.step_index,
                "browser_request_takeover",
                elapsed_ms,
                None,
                outcome.current_url.clone(),
            );
            let msg = if outcome.resumed {
                format!(
                    "User completed browser takeover for session '{}' (URL: {}, title: {}). Live DOM refreshed.",
                    outcome.session_id,
                    outcome.current_url.as_deref().unwrap_or("unknown"),
                    outcome.page_title.as_deref().unwrap_or(""),
                )
            } else {
                format!(
                    "Browser takeover requested for session '{}': {}",
                    outcome.session_id, outcome.reason
                )
            };
            let res = HermesToolResult::ok(
                "browser_request_takeover",
                msg,
                serde_json::to_value(&outcome).ok(),
            )
            .with_ui_action(ui_action)
            .with_inline_buttons(vec![
                HermesInlineButton {
                    label: "Take Over".into(),
                    callback_data: format!("browser_takeover:{sid}"),
                },
                HermesInlineButton {
                    label: "Resume AI".into(),
                    callback_data: format!("browser_resume:{sid}"),
                },
                HermesInlineButton {
                    label: "Cancel AI".into(),
                    callback_data: format!("browser_cancel:{sid}"),
                },
            ]);
            attach_resumed_context(res, &gate_info)
        }
        Err(e) => {
            sess.emit_step_failed(
                gate_info.step_index,
                "browser_request_takeover",
                start.elapsed().as_millis() as u64,
                e.to_string(),
            );
            HermesToolResult::err("browser_request_takeover", e.to_string())
        }
    }
}

async fn tool_distill_browser_script_app(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(name) = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return HermesToolResult::err("distill_browser_script_app", "Missing required 'name'");
    };
    let Some(ref registry) = ctx.browser_sessions else {
        return HermesToolResult::err(
            "distill_browser_script_app",
            "No BrowserSessionRegistry attached to HermesToolContext",
        );
    };

    let target_sid = explicit_session_id(&args).unwrap_or(ctx.session_id.as_str());
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("automation")
        .to_string();
    let slug = args
        .get("slug")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let target_domains: Vec<String> = args
        .get("target_domains")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    let meta = ScriptAppMetadata {
        name: name.to_string(),
        slug,
        description,
        category,
        target_domains,
        ..Default::default()
    };

    match registry.distill_script(target_sid, meta) {
        Ok(app) => {
            if args.get("save_as_skill").and_then(|v| v.as_bool()) == Some(true) {
                let Some(ref bridge) = ctx.state_bridge else {
                    return HermesToolResult::err(
                        "distill_browser_script_app",
                        format!(
                            "Distilled '{}' ({}) but cannot save skill: no HermesStateBridge attached to HermesToolContext.",
                            app.name, app.slug
                        ),
                    );
                };
                let app_val = serde_json::to_value(&app).unwrap_or(Value::Null);
                let save_res = bridge
                    .skill_op(&json!({
                        "action": "save_distilled_browser_script",
                        "app": app_val,
                    }))
                    .await;
                let final_save_res = match save_res {
                    Ok(v) => Ok(v),
                    Err(err) if err.contains("Unknown skill action") => {
                        let req = super::skills::SkillsManager::build_distilled_browser_script_skill_request(&app);
                        bridge
                            .skill_op(&json!({
                                "action": "create",
                                "name": req.name,
                                "description": req.description,
                                "body": req.body,
                                "toolsets": req.toolsets,
                                "tags": req.tags,
                            }))
                            .await
                    }
                    Err(err) => Err(err),
                };
                if let Err(err) = final_save_res {
                    return HermesToolResult::err(
                        "distill_browser_script_app",
                        format!(
                            "Distilled '{}' ({}) but failed to save skill: {err}",
                            app.name, app.slug
                        ),
                    );
                }
            }
            let payload = serde_json::to_value(&app).unwrap_or(Value::Null);
            let ui_action = HermesUiAction {
                action_type: "browser_script_distilled".to_string(),
                payload: json!({
                    "app_id": app.app_id,
                    "slug": app.slug,
                    "name": app.name,
                    "step_count": app.steps.len(),
                    "app": payload,
                }),
            };
            ctx.emit_ui(&ui_action);
            HermesToolResult::ok(
                "distill_browser_script_app",
                format!(
                    "Distilled '{}' ({}) with {} deterministic step(s) and {} parameter(s).",
                    app.name,
                    app.slug,
                    app.steps.len(),
                    app.parameters.len(),
                ),
                Some(payload),
            )
            .with_ui_action(ui_action)
        }
        Err(e) => HermesToolResult::err("distill_browser_script_app", e.to_string()),
    }
}

async fn resolve_fallback_backend(
    tool_name: &str,
    args: &Value,
    ctx: &HermesToolContext,
) -> Result<Arc<dyn BrowserSessionBackend>, HermesToolResult> {
    let session_id = resolve_session_id(args, ctx);
    if let Some(orch) = ctx.bot_browser.as_ref() {
        return orch
            .ensure_session_backend(&session_id, None)
            .await
            .map(|b| b as Arc<dyn BrowserSessionBackend>)
            .map_err(|e| HermesToolResult::err(tool_name, e.to_string()));
    }
    let Some(endpoint) = resolve_obscura_endpoint(args, ctx) else {
        return Err(HermesToolResult::err(
            tool_name,
            "Obscura browser engine is not attached and no CDP endpoint was provided.",
        ));
    };
    Ok(Arc::new(CdpBrowserBackend::connect_cdp(
        BrowserBackendKind::ObscuraCdp,
        endpoint,
        std::time::Duration::from_secs(15),
    )))
}

fn resolve_orchestrator_for_session(
    ctx: &HermesToolContext,
    sess: &BrowserAutomationSession,
) -> Option<Arc<BotBrowserOrchestrator>> {
    ctx.bot_browser
        .clone()
        .or_else(|| sess.source().orchestrator().cloned())
        .or_else(|| {
            ctx.browser_sessions
                .as_ref()
                .and_then(|reg| reg.orchestrator())
        })
}

async fn tool_vault_list(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("vault_list", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess.prepare_step("vault_list", None, None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("vault_list", e.to_string()),
        };
        let start = Instant::now();
        let res = match sess.source() {
            SessionBackendSource::Orchestrator { orchestrator, .. } => {
                orchestrator.tool_vault_list(sess.session_id()).await
            }
            SessionBackendSource::Registered(backend) => {
                if let Some(orch) = resolve_orchestrator_for_session(ctx, sess) {
                    match backend.current_url().await {
                        Ok(url) => match NormalizedOrigin::parse(&url) {
                            Ok(origin) => orch.vault().list_for_origin(origin.as_str()).map(
                                |credentials| VaultListResult {
                                    origin: origin.as_str().to_string(),
                                    credentials,
                                },
                            ),
                            Err(e) => Err(e),
                        },
                        Err(e) => Err(e),
                    }
                } else {
                    Err(BotBrowserError::Cdp(
                        "Obscura BotBrowserOrchestrator is not attached to HermesToolContext"
                            .to_string(),
                    ))
                }
            }
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match res {
            Ok(list_res) => {
                sess.emit_step_completed(
                    gate_info.step_index,
                    "vault_list",
                    elapsed_ms,
                    None,
                    None,
                );
                attach_resumed_context(serialize_tool_json("vault_list", &list_res), &gate_info)
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "vault_list",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("vault_list", e.to_string())
            }
        };
    }

    let orch = match require_orchestrator("vault_list", ctx) {
        Ok(o) => o,
        Err(err_res) => return err_res,
    };
    let session_id = resolve_session_id(&args, ctx);
    match orch.tool_vault_list(&session_id).await {
        Ok(res) => serialize_tool_json("vault_list", &res),
        Err(e) => HermesToolResult::err("vault_list", e.to_string()),
    }
}

async fn tool_vault_request_card(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let selectors = match parse_login_selectors(&args) {
        Ok(s) => s,
        Err(msg) => return HermesToolResult::err("vault_request_card", msg),
    };
    let task_id = resolve_task_id(&args, ctx);
    let site_title = args
        .get("site_title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Site Login");
    let wait_for_finish = args
        .get("wait_for_finish")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("vault_request_card", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess.prepare_step("vault_request_card", None, None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("vault_request_card", e.to_string()),
        };
        let start = Instant::now();
        match sess.source() {
            SessionBackendSource::Orchestrator { orchestrator, .. } => {
                let res = orchestrator
                    .tool_vault_request_card(
                        sess.session_id(),
                        &task_id,
                        ctx.chat_id,
                        site_title,
                        selectors,
                    )
                    .await;
                let elapsed_ms = start.elapsed().as_millis() as u64;
                return match res {
                    Ok(card_res) => {
                        sess.emit_step_completed(
                            gate_info.step_index,
                            "vault_request_card",
                            elapsed_ms,
                            None,
                            None,
                        );
                        attach_resumed_context(
                            serialize_tool_json("vault_request_card", &card_res),
                            &gate_info,
                        )
                    }
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "vault_request_card",
                            elapsed_ms,
                            e.to_string(),
                        );
                        HermesToolResult::err("vault_request_card", e.to_string())
                    }
                };
            }
            SessionBackendSource::Registered(backend) => {
                let current_url = match backend.current_url().await {
                    Ok(u) => u,
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "vault_request_card",
                            start.elapsed().as_millis() as u64,
                            e.to_string(),
                        );
                        return HermesToolResult::err("vault_request_card", e.to_string());
                    }
                };
                let origin = match NormalizedOrigin::parse(&current_url) {
                    Ok(o) => o,
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "vault_request_card",
                            start.elapsed().as_millis() as u64,
                            e.to_string(),
                        );
                        return HermesToolResult::err("vault_request_card", e.to_string());
                    }
                };
                let sid = sess.session_id().to_string();
                let card_id = format!("card_{}", uuid::Uuid::new_v4().simple());
                let ui_action = HermesUiAction {
                    action_type: "vault_login_card".to_string(),
                    payload: json!({
                        "card_id": card_id,
                        "task_id": task_id,
                        "session_id": sid,
                        "chat_id": ctx.chat_id,
                        "origin": origin.as_str(),
                        "target_url": redact_url_for_trace(&current_url),
                        "site_title": site_title,
                        "is_high_sensitivity": origin.is_high_sensitivity(),
                        "suggested_reuse_policy": ReusePolicy::default_for_origin(&origin),
                        "selectors": {
                            "username_selector": selectors.username_selector,
                            "password_selector": selectors.password_selector,
                            "totp_selector": selectors.totp_selector,
                            "submit_selector": selectors.submit_selector,
                        },
                    }),
                };
                ctx.emit_ui(&ui_action);

                let takeover_res = sess
                    .request_takeover_and_wait(
                        &format!("Enter login credentials for '{site_title}' ({origin})"),
                        wait_for_finish,
                        true,
                    )
                    .await;
                let elapsed_ms = start.elapsed().as_millis() as u64;
                return match takeover_res {
                    Ok(outcome) => {
                        let latest_cred = resolve_orchestrator_for_session(ctx, sess)
                            .and_then(|orch| orch.vault().list_for_origin(origin.as_str()).ok())
                            .and_then(|mut list| list.drain(..).next());
                        let (status, credential_id, account_hint) = if let Some(meta) = latest_cred {
                            (
                                if outcome.resumed {
                                    "saved".to_string()
                                } else {
                                    "awaiting_user_takeover".to_string()
                                },
                                Some(meta.credential_id),
                                Some(meta.account_hint),
                            )
                        } else {
                            (
                                if outcome.resumed {
                                    "resumed_from_takeover".to_string()
                                } else {
                                    "awaiting_user_takeover".to_string()
                                },
                                None,
                                None,
                            )
                        };
                        let card_res = VaultRequestCardResult {
                            status,
                            card_id,
                            credential_id,
                            account_hint,
                            origin: origin.as_str().to_string(),
                        };
                        sess.emit_step_completed(
                            gate_info.step_index,
                            "vault_request_card",
                            elapsed_ms,
                            None,
                            outcome.current_url.or_else(|| Some(current_url)),
                        );
                        attach_resumed_context(
                            serialize_tool_json("vault_request_card", &card_res)
                                .with_ui_action(ui_action)
                                .with_inline_buttons(vec![
                                    HermesInlineButton {
                                        label: "Take Over".into(),
                                        callback_data: format!("browser_takeover:{sid}"),
                                    },
                                    HermesInlineButton {
                                        label: "Resume AI".into(),
                                        callback_data: format!("browser_resume:{sid}"),
                                    },
                                    HermesInlineButton {
                                        label: "Cancel AI".into(),
                                        callback_data: format!("browser_cancel:{sid}"),
                                    },
                                ]),
                            &gate_info,
                        )
                    }
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "vault_request_card",
                            elapsed_ms,
                            e.to_string(),
                        );
                        HermesToolResult::err("vault_request_card", e.to_string())
                    }
                };
            }
        }
    }

    let orch = match require_orchestrator("vault_request_card", ctx) {
        Ok(o) => o,
        Err(err_res) => return err_res,
    };
    let session_id = resolve_session_id(&args, ctx);

    match orch
        .tool_vault_request_card(&session_id, &task_id, ctx.chat_id, site_title, selectors)
        .await
    {
        Ok(res) => serialize_tool_json("vault_request_card", &res),
        Err(e) => HermesToolResult::err("vault_request_card", e.to_string()),
    }
}

async fn tool_vault_fill(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(credential_id) = args
        .get("credential_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return HermesToolResult::err("vault_fill", "Missing required 'credential_id'");
    };
    let selectors = match parse_login_selectors(&args) {
        Ok(s) => s,
        Err(msg) => return HermesToolResult::err("vault_fill", msg),
    };
    let task_id = resolve_task_id(&args, ctx);

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("vault_fill", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess.prepare_step("vault_fill", None, None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("vault_fill", e.to_string()),
        };
        let start = Instant::now();
        let fill_outcome: Result<(VaultFillResult, Option<String>), BotBrowserError> =
            match sess.source() {
                SessionBackendSource::Orchestrator { orchestrator, .. } => {
                    match orchestrator
                        .tool_vault_fill(
                            sess.session_id(),
                            &task_id,
                            ctx.chat_id,
                            credential_id,
                            selectors.clone(),
                        )
                        .await
                    {
                        Ok(res) => {
                            let page_url = sess.backend().current_url().await.ok();
                            Ok((res, page_url))
                        }
                        Err(e) => Err(e),
                    }
                }
                SessionBackendSource::Registered(backend) => {
                    let Some(orch) = resolve_orchestrator_for_session(ctx, sess) else {
                        let elapsed_ms = start.elapsed().as_millis() as u64;
                        let err_msg = "Obscura BotBrowserOrchestrator is not attached to HermesToolContext";
                        sess.emit_step_failed(gate_info.step_index, "vault_fill", elapsed_ms, err_msg);
                        return HermesToolResult::err("vault_fill", err_msg);
                    };

                    async {
                        let meta = orch.vault().get_meta(credential_id)?;
                        let current_url = backend.current_url().await?;
                        let page_origin = NormalizedOrigin::parse(&current_url)?;
                        if page_origin.as_str() != meta.origin {
                            return Err(BotBrowserError::OriginMismatch {
                                expected: meta.origin,
                                actual: page_origin.as_str().to_string(),
                            });
                        }

                        if meta.reuse_policy == ReusePolicy::ConfirmEveryTime {
                            let confirm_ui = HermesUiAction {
                                action_type: "vault_reuse_confirm_card".to_string(),
                                payload: json!({
                                    "session_id": sess.session_id(),
                                    "credential_id": meta.credential_id,
                                    "origin": meta.origin,
                                    "account_hint": meta.account_hint,
                                }),
                            };
                            ctx.emit_ui(&confirm_ui);
                            sess.request_takeover_and_wait(
                                &format!(
                                    "Confirm credential reuse for '{}' on {}",
                                    meta.account_hint, meta.origin
                                ),
                                true,
                                false,
                            )
                            .await?;
                        }

                        let secret = orch.vault().decrypt_for_origin(
                            credential_id,
                            page_origin.as_str(),
                            None,
                        )?;
                        let totp_code = match secret.totp_secret.as_deref() {
                            Some(raw) => Some(bot_browser::generate_totp_now(raw)?),
                            None => None,
                        };

                        backend
                            .fill_locator(
                                &SemanticLocator::by_css(&selectors.username_selector),
                                secret.username.as_str(),
                                true,
                            )
                            .await?;
                        backend
                            .fill_locator(
                                &SemanticLocator::by_css(&selectors.password_selector),
                                secret.password.as_str(),
                                true,
                            )
                            .await?;

                        let mut used_totp = false;
                        if let (Some(totp_sel), Some(code)) =
                            (selectors.totp_selector.as_deref(), totp_code.as_ref())
                        {
                            backend
                                .fill_locator(&SemanticLocator::by_css(totp_sel), code.as_str(), true)
                                .await?;
                            used_totp = true;
                        }
                        drop(totp_code);
                        drop(secret);

                        if let Some(submit_sel) = selectors.submit_selector.as_deref() {
                            backend
                                .click_locator(&SemanticLocator::by_css(submit_sel))
                                .await?;
                        }

                        Ok((
                            VaultFillResult {
                                status: "filled_and_submitted".to_string(),
                                origin: page_origin.as_str().to_string(),
                                credential_id: credential_id.to_string(),
                                used_totp,
                                card_id: None,
                            },
                            Some(current_url),
                        ))
                    }
                    .await
                }
            };

        let elapsed_ms = start.elapsed().as_millis() as u64;
        return match fill_outcome {
            Ok((res, page_url)) => {
                let totp_sel = res
                    .used_totp
                    .then(|| selectors.totp_selector.as_deref())
                    .flatten();
                sess.record_vault_fill(
                    page_url.clone(),
                    credential_id,
                    &selectors.username_selector,
                    &selectors.password_selector,
                    totp_sel,
                    selectors.submit_selector.as_deref(),
                    elapsed_ms,
                );
                sess.emit_step_completed(
                    gate_info.step_index,
                    "vault_fill",
                    elapsed_ms,
                    None,
                    page_url,
                );
                attach_resumed_context(serialize_tool_json("vault_fill", &res), &gate_info)
            }
            Err(e) => {
                sess.emit_step_failed(
                    gate_info.step_index,
                    "vault_fill",
                    elapsed_ms,
                    e.to_string(),
                );
                HermesToolResult::err("vault_fill", e.to_string())
            }
        };
    }

    let orch = match require_orchestrator("vault_fill", ctx) {
        Ok(o) => o,
        Err(err_res) => return err_res,
    };
    let session_id = resolve_session_id(&args, ctx);

    let start = Instant::now();
    match orch
        .tool_vault_fill(
            &session_id,
            &task_id,
            ctx.chat_id,
            credential_id,
            selectors.clone(),
        )
        .await
    {
        Ok(res) => {
            if let Some(ref reg) = ctx.browser_sessions {
                if let Some(sess) = reg.get_session(&session_id) {
                    let page_url = sess.backend().current_url().await.ok();
                    let totp_sel = res
                        .used_totp
                        .then(|| selectors.totp_selector.as_deref())
                        .flatten();
                    sess.record_vault_fill(
                        page_url,
                        credential_id,
                        &selectors.username_selector,
                        &selectors.password_selector,
                        totp_sel,
                        selectors.submit_selector.as_deref(),
                        start.elapsed().as_millis() as u64,
                    );
                }
            }
            serialize_tool_json("vault_fill", &res)
        }
        Err(e) => HermesToolResult::err("vault_fill", e.to_string()),
    }
}

async fn tool_human_assist_qr(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(qr_selector) = args
        .get("qr_selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return HermesToolResult::err("human_assist_qr", "Missing required 'qr_selector'");
    };
    let success_url_contains = args
        .get("success_url_contains")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let auth_cookie_names: Vec<String> = args
        .get("auth_cookie_names")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(str::trim).filter(|s| !s.is_empty()))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let task_id = resolve_task_id(&args, ctx);

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("human_assist_qr", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess.prepare_step("human_assist_qr", None, None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("human_assist_qr", e.to_string()),
        };
        let start = Instant::now();
        match sess.source() {
            SessionBackendSource::Orchestrator { orchestrator, .. } => {
                let res = orchestrator
                    .tool_human_assist_qr(
                        sess.session_id(),
                        &task_id,
                        ctx.chat_id,
                        qr_selector,
                        success_url_contains,
                        auth_cookie_names,
                    )
                    .await;
                let elapsed_ms = start.elapsed().as_millis() as u64;
                return match res {
                    Ok(qr_res) => {
                        let page_url = sess.backend().current_url().await.ok();
                        sess.record_human_checkpoint(
                            page_url.clone(),
                            format!("Human QR login checkpoint ({qr_selector})"),
                        );
                        sess.emit_step_completed(
                            gate_info.step_index,
                            "human_assist_qr",
                            elapsed_ms,
                            None,
                            page_url,
                        );
                        attach_resumed_context(
                            serialize_tool_json("human_assist_qr", &qr_res),
                            &gate_info,
                        )
                    }
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "human_assist_qr",
                            elapsed_ms,
                            e.to_string(),
                        );
                        HermesToolResult::err("human_assist_qr", e.to_string())
                    }
                };
            }
            SessionBackendSource::Registered(backend) => {
                let initial_url = match backend.current_url().await {
                    Ok(u) => u,
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "human_assist_qr",
                            start.elapsed().as_millis() as u64,
                            e.to_string(),
                        );
                        return HermesToolResult::err("human_assist_qr", e.to_string());
                    }
                };
                let origin = match NormalizedOrigin::parse(&initial_url) {
                    Ok(o) => o,
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "human_assist_qr",
                            start.elapsed().as_millis() as u64,
                            e.to_string(),
                        );
                        return HermesToolResult::err("human_assist_qr", e.to_string());
                    }
                };
                let png_b64 = backend
                    .capture_screenshot()
                    .await
                    .ok()
                    .map(|bytes| base64_encode(&bytes));
                let card_id = format!("card_{}", uuid::Uuid::new_v4().simple());
                let sid = sess.session_id().to_string();
                let ui_action = HermesUiAction {
                    action_type: "human_assist_qr_card".to_string(),
                    payload: json!({
                        "card_id": card_id,
                        "task_id": task_id,
                        "session_id": sid,
                        "chat_id": ctx.chat_id,
                        "origin": origin.as_str(),
                        "target_url": redact_url_for_trace(&initial_url),
                        "qr_selector": qr_selector,
                        "success_url_contains": success_url_contains,
                        "auth_cookie_names": auth_cookie_names,
                        "png_base64": png_b64,
                    }),
                };
                ctx.emit_ui(&ui_action);

                if let Err(e) = sess
                    .request_takeover_and_wait(
                        &format!("Human QR login checkpoint ({qr_selector})"),
                        false,
                        true,
                    )
                    .await
                {
                    sess.emit_step_failed(
                        gate_info.step_index,
                        "human_assist_qr",
                        start.elapsed().as_millis() as u64,
                        e.to_string(),
                    );
                    return HermesToolResult::err("human_assist_qr", e.to_string());
                }

                let (refresh_interval, max_wait) = resolve_orchestrator_for_session(ctx, sess)
                    .map(|o| (o.config().qr_refresh_interval, o.config().qr_max_wait))
                    .unwrap_or((
                        std::time::Duration::from_millis(200),
                        std::time::Duration::from_secs(180),
                    ));
                let wait_res = sess
                    .wait_for_qr_assist_or_resume(
                        gate_info.step_index,
                        card_id,
                        &initial_url,
                        success_url_contains,
                        &auth_cookie_names,
                        refresh_interval,
                        max_wait,
                    )
                    .await;
                let elapsed_ms = start.elapsed().as_millis() as u64;
                return match wait_res {
                    Ok(qr_res) => {
                        sess.emit_step_completed(
                            gate_info.step_index,
                            "human_assist_qr",
                            elapsed_ms,
                            None,
                            qr_res.redirected_url.clone().or(Some(initial_url)),
                        );
                        attach_resumed_context(
                            serialize_tool_json("human_assist_qr", &qr_res)
                                .with_ui_action(ui_action),
                            &gate_info,
                        )
                    }
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "human_assist_qr",
                            elapsed_ms,
                            e.to_string(),
                        );
                        HermesToolResult::err("human_assist_qr", e.to_string())
                    }
                };
            }
        }
    }

    let orch = match require_orchestrator("human_assist_qr", ctx) {
        Ok(o) => o,
        Err(err_res) => return err_res,
    };
    let session_id = resolve_session_id(&args, ctx);

    match orch
        .tool_human_assist_qr(
            &session_id,
            &task_id,
            ctx.chat_id,
            qr_selector,
            success_url_contains,
            auth_cookie_names,
        )
        .await
    {
        Ok(res) => {
            if let Some(ref reg) = ctx.browser_sessions {
                if let Some(sess) = reg.get_session(&session_id) {
                    let page_url = sess.backend().current_url().await.ok();
                    sess.record_human_checkpoint(
                        page_url,
                        format!("Human QR login checkpoint ({qr_selector})"),
                    );
                }
            }
            serialize_tool_json("human_assist_qr", &res)
        }
        Err(e) => HermesToolResult::err("human_assist_qr", e.to_string()),
    }
}

async fn tool_human_assist_click(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(target_selector) = args
        .get("target_selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return HermesToolResult::err("human_assist_click", "Missing required 'target_selector'");
    };
    let instruction = args
        .get("instruction")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Please click the requested verification points");
    let max_clicks = args
        .get("max_clicks")
        .and_then(|v| v.as_u64())
        .unwrap_or(3)
        .clamp(1, 16) as u8;
    let task_id = resolve_task_id(&args, ctx);

    if ctx.browser_sessions.is_some() {
        let acquired = match resolve_and_lock_session("human_assist_click", &args, ctx, None).await {
            Ok(a) => a,
            Err(err_res) => return err_res,
        };
        let sess = acquired.session();
        let gate_info = match sess.prepare_step("human_assist_click", None, None).await {
            Ok(g) => g,
            Err(e) => return HermesToolResult::err("human_assist_click", e.to_string()),
        };
        let start = Instant::now();
        match sess.source() {
            SessionBackendSource::Orchestrator { orchestrator, .. } => {
                let res = orchestrator
                    .tool_human_assist_click(
                        sess.session_id(),
                        &task_id,
                        ctx.chat_id,
                        target_selector,
                        instruction,
                        max_clicks,
                    )
                    .await;
                let elapsed_ms = start.elapsed().as_millis() as u64;
                return match res {
                    Ok(click_res) => {
                        let page_url = sess.backend().current_url().await.ok();
                        sess.record_human_checkpoint(
                            page_url.clone(),
                            format!("Human click assist checkpoint: {instruction}"),
                        );
                        sess.emit_step_completed(
                            gate_info.step_index,
                            "human_assist_click",
                            elapsed_ms,
                            None,
                            page_url,
                        );
                        attach_resumed_context(
                            serialize_tool_json("human_assist_click", &click_res),
                            &gate_info,
                        )
                    }
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "human_assist_click",
                            elapsed_ms,
                            e.to_string(),
                        );
                        HermesToolResult::err("human_assist_click", e.to_string())
                    }
                };
            }
            SessionBackendSource::Registered(backend) => {
                let current_url = match backend.current_url().await {
                    Ok(u) => u,
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "human_assist_click",
                            start.elapsed().as_millis() as u64,
                            e.to_string(),
                        );
                        return HermesToolResult::err("human_assist_click", e.to_string());
                    }
                };
                let origin = match NormalizedOrigin::parse(&current_url) {
                    Ok(o) => o,
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "human_assist_click",
                            start.elapsed().as_millis() as u64,
                            e.to_string(),
                        );
                        return HermesToolResult::err("human_assist_click", e.to_string());
                    }
                };
                let png_b64 = backend
                    .capture_screenshot()
                    .await
                    .ok()
                    .map(|bytes| base64_encode(&bytes));
                let card_id = format!("card_{}", uuid::Uuid::new_v4().simple());
                let sid = sess.session_id().to_string();
                let ui_action = HermesUiAction {
                    action_type: "human_assist_click_card".to_string(),
                    payload: json!({
                        "card_id": card_id,
                        "task_id": task_id,
                        "session_id": sid,
                        "chat_id": ctx.chat_id,
                        "origin": origin.as_str(),
                        "target_url": redact_url_for_trace(&current_url),
                        "target_selector": target_selector,
                        "instruction": instruction,
                        "max_clicks": max_clicks,
                        "png_base64": png_b64,
                    }),
                };
                ctx.emit_ui(&ui_action);

                let takeover_res = sess
                    .request_takeover_and_wait(
                        &format!("Human click assist: {instruction} ({target_selector})"),
                        true,
                        true,
                    )
                    .await;
                let elapsed_ms = start.elapsed().as_millis() as u64;
                return match takeover_res {
                    Ok(outcome) => {
                        let click_res = ClickAssistResult {
                            status: "completed_via_takeover".to_string(),
                            card_id,
                            count: 0,
                            dispatched_points: Vec::new(),
                        };
                        sess.emit_step_completed(
                            gate_info.step_index,
                            "human_assist_click",
                            elapsed_ms,
                            None,
                            outcome.current_url.or(Some(current_url)),
                        );
                        attach_resumed_context(
                            serialize_tool_json("human_assist_click", &click_res)
                                .with_ui_action(ui_action),
                            &gate_info,
                        )
                    }
                    Err(e) => {
                        sess.emit_step_failed(
                            gate_info.step_index,
                            "human_assist_click",
                            elapsed_ms,
                            e.to_string(),
                        );
                        HermesToolResult::err("human_assist_click", e.to_string())
                    }
                };
            }
        }
    }

    let orch = match require_orchestrator("human_assist_click", ctx) {
        Ok(o) => o,
        Err(err_res) => return err_res,
    };
    let session_id = resolve_session_id(&args, ctx);

    match orch
        .tool_human_assist_click(
            &session_id,
            &task_id,
            ctx.chat_id,
            target_selector,
            instruction,
            max_clicks,
        )
        .await
    {
        Ok(res) => {
            if let Some(ref reg) = ctx.browser_sessions {
                if let Some(sess) = reg.get_session(&session_id) {
                    let page_url = sess.backend().current_url().await.ok();
                    sess.record_human_checkpoint(
                        page_url,
                        format!("Human click assist checkpoint: {instruction}"),
                    );
                }
            }
            serialize_tool_json("human_assist_click", &res)
        }
        Err(e) => HermesToolResult::err("human_assist_click", e.to_string()),
    }
}

// ----------------------------------------------------------------------------
// Audio TTS / STT, Vision, ImageGen, VideoGen
// ----------------------------------------------------------------------------

/// Synthesizes deterministic PCM/RIFF WAVE speech cadence audio in pure Rust.
pub fn synthesize_speech_wav(text: &str, sample_rate: u32) -> Result<(Vec<u8>, u64, Vec<u8>), String> {
    let sr = sample_rate.clamp(8000, 48000);
    let chars: Vec<char> = text.chars().collect();
    let syllable_ms = 75usize;
    let samples_per_syl = (sr as usize * syllable_ms) / 1000;
    let min_syls = 4usize;
    let syl_count = chars.len().clamp(min_syls, 240);
    let mut pcm = Vec::with_capacity(syl_count * samples_per_syl);

    for idx in 0..syl_count {
        let ch = chars.get(idx).copied().unwrap_or(' ');
        let is_pause = ch.is_whitespace() || matches!(ch, ',' | '.' | '!' | '?' | ';' | '，' | '。');
        let code = ch as u32;
        let base_freq = 160.0 + ((code % 19) as f64) * 14.0;
        let formant2 = base_freq * 2.25;
        for s in 0..samples_per_syl {
            if is_pause {
                pcm.push(0);
                continue;
            }
            let t = s as f64 / sr as f64;
            let phase = s as f64 / samples_per_syl as f64;
            // Smooth Hann-like syllable amplitude envelope
            let env = (std::f64::consts::PI * phase).sin().powf(0.8);
            let wave = 0.65 * (2.0 * std::f64::consts::PI * base_freq * t).sin()
                + 0.35 * (2.0 * std::f64::consts::PI * formant2 * t).sin();
            let amp = (wave * env * 12000.0).clamp(-32767.0, 32767.0) as i16;
            pcm.push(amp);
        }
    }

    let wav_bytes = encode_wav_i16(&pcm, sr, 1).map_err(|e| e.to_string())?;
    let duration_ms = ((pcm.len() as u64) * 1000) / (sr as u64);
    let (_, packed_waveform) = waveform_from_pcm_i16(&pcm);
    Ok((wav_bytes, duration_ms, packed_waveform))
}

async fn tool_text_to_speech(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(text) = args.get("text").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
        return HermesToolResult::err("text_to_speech", "Missing required 'text' parameter");
    };
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("local")
        .trim()
        .to_ascii_lowercase();
    let voice = args
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or("alloy");
    let sample_rate = args
        .get("sample_rate")
        .and_then(|v| v.as_u64())
        .unwrap_or(16_000) as u32;

    let mut used_provider = "local_pcm_riff".to_string();
    let mut mime = "audio/wav".to_string();
    let mut ext = "wav";
    let mut audio_bytes: Option<Vec<u8>> = None;
    let mut duration_ms = 0u64;
    let mut waveform_b64 = None;

    if matches!(provider.as_str(), "openai" | "auto") {
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            if !api_key.trim().is_empty() {
                if let Ok(resp) = ctx
                    .http_client
                    .post("https://api.openai.com/v1/audio/speech")
                    .bearer_auth(api_key)
                    .json(&json!({
                        "model": "gpt-4o-mini-tts",
                        "voice": voice,
                        "input": text,
                        "response_format": "wav"
                    }))
                    .send()
                    .await
                {
                    if resp.status().is_success() {
                        if let Ok(bytes) = resp.bytes().await {
                            audio_bytes = Some(bytes.to_vec());
                            used_provider = "openai".into();
                        }
                    }
                }
            }
        }
    } else if provider == "elevenlabs" {
        if let Ok(api_key) = std::env::var("ELEVENLABS_API_KEY") {
            if !api_key.trim().is_empty() {
                let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{voice}");
                if let Ok(resp) = ctx
                    .http_client
                    .post(&url)
                    .header("xi-api-key", api_key)
                    .json(&json!({ "text": text }))
                    .send()
                    .await
                {
                    if resp.status().is_success() {
                        if let Ok(bytes) = resp.bytes().await {
                            audio_bytes = Some(bytes.to_vec());
                            used_provider = "elevenlabs".into();
                            mime = "audio/mpeg".into();
                            ext = "mp3";
                        }
                    }
                }
            }
        }
    }

    if audio_bytes.is_none() {
        match synthesize_speech_wav(text, sample_rate) {
            Ok((wav, dur, wf)) => {
                audio_bytes = Some(wav);
                duration_ms = dur;
                waveform_b64 = Some(base64_encode(&wf));
                if provider != "local" && provider != "auto" {
                    used_provider = format!("{provider}_fallback_local_wav");
                }
            }
            Err(e) => return HermesToolResult::err("text_to_speech", e),
        }
    }

    let bytes = audio_bytes.unwrap_or_default();
    let mut saved_path = None;
    if let Some(raw_path) = args.get("output_path").and_then(|v| v.as_str()) {
        let resolved = match ctx.security.read() {
            Ok(sec) => match sec.validate_path(Path::new(raw_path)) {
                Ok(p) => p,
                Err(e) => return HermesToolResult::err("text_to_speech", e),
            },
            Err(_) => return HermesToolResult::err("text_to_speech", "SecurityEngine lock poisoned"),
        };
        if let Some(parent) = resolved.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(&resolved, &bytes) {
            return HermesToolResult::err("text_to_speech", format!("Failed writing TTS audio: {e}"));
        }
        saved_path = Some(resolved.display().to_string());
    }

    let is_riff_wave = bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE";
    let b64 = base64_encode(&bytes);

    HermesToolResult::ok(
        "text_to_speech",
        format!(
            "Synthesized {} bytes ({}) via {} [duration_ms={}]",
            bytes.len(),
            if is_riff_wave { "RIFF/WAVE" } else { mime.as_str() },
            used_provider,
            duration_ms
        ),
        Some(json!({
            "provider": used_provider,
            "voice": voice,
            "mime_type": mime,
            "byte_len": bytes.len(),
            "sample_rate": sample_rate,
            "duration_ms": duration_ms,
            "is_riff_wave": is_riff_wave,
            "output_path": saved_path,
            "waveform_base64": waveform_b64,
        })),
    )
    .with_artifact(HermesToolArtifact {
        kind: "audio".into(),
        name: format!("tts_output.{ext}"),
        mime_type: mime,
        path: saved_path,
        data_base64: Some(b64),
        metadata: json!({ "sample_rate": sample_rate, "duration_ms": duration_ms }),
    })
}

async fn tool_speech_to_text(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("local")
        .to_ascii_lowercase();
    let language = args
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("en");

    let audio_bytes = if let Some(b64) = args.get("audio_base64").and_then(|v| v.as_str()) {
        match base64_decode(b64) {
            Ok(b) => b,
            Err(e) => return HermesToolResult::err("speech_to_text", format!("Invalid audio_base64: {e}")),
        }
    } else if let Some(raw_path) = args.get("audio_path").and_then(|v| v.as_str()) {
        let resolved = match ctx.security.read() {
            Ok(sec) => match sec.validate_path(Path::new(raw_path)) {
                Ok(p) => p,
                Err(e) => return HermesToolResult::err("speech_to_text", e),
            },
            Err(_) => return HermesToolResult::err("speech_to_text", "SecurityEngine lock poisoned"),
        };
        match fs::read(&resolved) {
            Ok(b) => b,
            Err(e) => return HermesToolResult::err("speech_to_text", format!("Failed reading audio: {e}")),
        }
    } else {
        return HermesToolResult::err("speech_to_text", "Provide 'audio_path' or 'audio_base64'");
    };

    if matches!(provider.as_str(), "openai" | "whisper") {
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            if !api_key.trim().is_empty() {
                let part = reqwest::multipart::Part::bytes(audio_bytes.clone())
                    .file_name("audio.wav")
                    .mime_str("audio/wav");
                if let Ok(file_part) = part {
                    let form = reqwest::multipart::Form::new()
                        .text("model", "whisper-1")
                        .text("language", language.to_string())
                        .part("file", file_part);
                    if let Ok(resp) = ctx
                        .http_client
                        .post("https://api.openai.com/v1/audio/transcriptions")
                        .bearer_auth(api_key)
                        .multipart(form)
                        .send()
                        .await
                    {
                        if resp.status().is_success() {
                            if let Ok(body) = resp.json::<Value>().await {
                                let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                return HermesToolResult::ok(
                                    "speech_to_text",
                                    text.clone(),
                                    Some(json!({
                                        "provider": "openai_whisper",
                                        "transcript": text,
                                        "confidence": 0.95,
                                        "language": language,
                                    })),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Local WAV PCM inspection & deterministic STT adapter
    if let Ok(decoded) = decode_wav_pcm(&audio_bytes) {
        let rms = compute_pcm_rms_level(&decoded.samples);
        let confidence = if rms > 100 { 0.88 } else { 0.42 };
        let transcript = if rms <= 20 {
            "[silence detected in audio buffer]".to_string()
        } else {
            format!(
                "[local audio segment: {}ms @ {}Hz, {}ch, rms={}]",
                decoded.duration_ms, decoded.sample_rate, decoded.channels, rms
            )
        };
        return HermesToolResult::ok(
            "speech_to_text",
            transcript.clone(),
            Some(json!({
                "provider": "local_wav_stt",
                "transcript": transcript,
                "confidence": confidence,
                "duration_ms": decoded.duration_ms,
                "sample_rate": decoded.sample_rate,
                "channels": decoded.channels,
                "rms_level": rms,
                "language": language,
            })),
        );
    }

    HermesToolResult::ok(
        "speech_to_text",
        format!("[local audio payload ({} bytes); configure OPENAI_API_KEY or pass WAV PCM for full transcription]", audio_bytes.len()),
        Some(json!({
            "provider": "local_stt_fallback",
            "transcript": "",
            "confidence": 0.5,
            "duration_ms": 0,
            "byte_len": audio_bytes.len(),
            "language": language,
        })),
    )
}

async fn tool_vision_analyze(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(prompt) = args.get("prompt").and_then(|v| v.as_str()) else {
        return HermesToolResult::err("vision_analyze", "Missing required 'prompt'");
    };
    let detail = args.get("detail").and_then(|v| v.as_str()).unwrap_or("auto");

    if let Some(image_url) = args.get("image_url").and_then(|v| v.as_str()) {
        let payload = json!({
            "type": "image_url",
            "image_url": { "url": image_url, "detail": detail }
        });
        return HermesToolResult::ok(
            "vision_analyze",
            format!("Prepared multimodal vision inspection payload for URL '{image_url}' with prompt: {prompt}"),
            Some(json!({
                "prompt": prompt,
                "source": "url",
                "openai_block": payload,
            })),
        );
    }

    let bytes = if let Some(b64) = args.get("image_base64").and_then(|v| v.as_str()) {
        match base64_decode(b64) {
            Ok(b) => b,
            Err(e) => return HermesToolResult::err("vision_analyze", format!("Invalid image_base64: {e}")),
        }
    } else if let Some(raw_path) = args.get("image_path").and_then(|v| v.as_str()) {
        let resolved = match ctx.security.read() {
            Ok(sec) => match sec.validate_path(Path::new(raw_path)) {
                Ok(p) => p,
                Err(e) => return HermesToolResult::err("vision_analyze", e),
            },
            Err(_) => return HermesToolResult::err("vision_analyze", "SecurityEngine lock poisoned"),
        };
        match fs::read(&resolved) {
            Ok(b) => b,
            Err(e) => return HermesToolResult::err("vision_analyze", format!("Failed reading image: {e}")),
        }
    } else {
        return HermesToolResult::err(
            "vision_analyze",
            "Provide 'image_path', 'image_base64', or 'image_url'",
        );
    };

    let (width, height, format, has_alpha) = match ThumbnailGenerator::probe_metadata(&bytes) {
        Ok(meta) => (meta.width, meta.height, meta.format, meta.has_alpha),
        Err(_) => probe_basic_image_header(&bytes),
    };

    let mime = match format.as_str() {
        "png" => "image/png",
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };
    let b64 = base64_encode(&bytes);

    let openai_block = json!({
        "type": "image_url",
        "image_url": { "url": format!("data:{mime};base64,{b64}"), "detail": detail }
    });
    let anthropic_block = json!({
        "type": "image",
        "source": { "type": "base64", "media_type": mime, "data": b64 }
    });

    HermesToolResult::ok(
        "vision_analyze",
        format!(
            "Inspected {format} image ({width}x{height}, has_alpha={has_alpha}, {} bytes) for prompt: {prompt}",
            bytes.len()
        ),
        Some(json!({
            "width": width,
            "height": height,
            "format": format,
            "has_alpha": has_alpha,
            "byte_len": bytes.len(),
            "mime_type": mime,
            "openai_block": openai_block,
            "anthropic_block": anthropic_block,
        })),
    )
}

fn probe_basic_image_header(bytes: &[u8]) -> (u32, u32, String, bool) {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return (w, h, "png".into(), true);
    }
    if bytes.starts_with(b"GIF8") && bytes.len() >= 10 {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return (w, h, "gif".into(), true);
    }
    let text_prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    if text_prefix.contains("<svg") {
        return (512, 512, "svg".into(), true);
    }
    (0, 0, "unknown".into(), false)
}

async fn tool_image_generate(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(prompt) = args.get("prompt").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
        return HermesToolResult::err("image_generate", "Missing required 'prompt'");
    };
    let size = args.get("size").and_then(|v| v.as_str()).unwrap_or("512x512");
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("local")
        .to_ascii_lowercase();
    let format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("svg")
        .to_ascii_lowercase();

    let (w, h) = parse_size_dims(size);

    if matches!(provider.as_str(), "openai" | "auto") {
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            if !api_key.trim().is_empty() {
                if let Ok(resp) = ctx
                    .http_client
                    .post("https://api.openai.com/v1/images/generations")
                    .bearer_auth(api_key)
                    .json(&json!({
                        "model": "gpt-image-1",
                        "prompt": prompt,
                        "size": size,
                        "response_format": "b64_json"
                    }))
                    .send()
                    .await
                {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<Value>().await {
                            if let Some(b64) = body
                                .get("data")
                                .and_then(|d| d.get(0))
                                .and_then(|i| i.get("b64_json"))
                                .and_then(|v| v.as_str())
                            {
                                let bytes = base64_decode(b64).unwrap_or_default();
                                let saved = maybe_write_asset(args.get("output_path").and_then(|v| v.as_str()), &bytes, ctx);
                                return HermesToolResult::ok(
                                    "image_generate",
                                    format!("Generated OpenAI image ({} bytes)", bytes.len()),
                                    Some(json!({ "provider": "openai", "format": "png", "output_path": saved })),
                                )
                                .with_artifact(HermesToolArtifact {
                                    kind: "image".into(),
                                    name: "generated.png".into(),
                                    mime_type: "image/png".into(),
                                    path: saved,
                                    data_base64: Some(b64.to_string()),
                                    metadata: json!({ "prompt": prompt, "size": size }),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    let (bytes, mime, filename) = if format == "png" {
        (generate_deterministic_png(prompt, w.min(256), h.min(256)), "image/png", "generated.png")
    } else {
        let svg = generate_deterministic_svg(prompt, w, h);
        (svg.into_bytes(), "image/svg+xml", "generated.svg")
    };

    let saved = maybe_write_asset(args.get("output_path").and_then(|v| v.as_str()), &bytes, ctx);
    let b64 = base64_encode(&bytes);

    HermesToolResult::ok(
        "image_generate",
        format!("Generated local {format} placeholder image ({w}x{h}, {} bytes) for '{prompt}'", bytes.len()),
        Some(json!({
            "provider": "local_deterministic",
            "format": format,
            "width": w,
            "height": h,
            "byte_len": bytes.len(),
            "output_path": saved,
        })),
    )
    .with_artifact(HermesToolArtifact {
        kind: "image".into(),
        name: filename.into(),
        mime_type: mime.into(),
        path: saved,
        data_base64: Some(b64),
        metadata: json!({ "prompt": prompt, "width": w, "height": h }),
    })
}

async fn tool_video_generate(args: Value, ctx: &HermesToolContext) -> HermesToolResult {
    let Some(prompt) = args.get("prompt").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
        return HermesToolResult::err("video_generate", "Missing required 'prompt'");
    };
    let duration_secs = args
        .get("duration_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(4);
    let resolution = args
        .get("resolution")
        .and_then(|v| v.as_str())
        .unwrap_or("1280x720");

    let storyboard_svg = generate_deterministic_svg(
        &format!("VIDEO ({duration_secs}s @ {resolution}): {prompt}"),
        640,
        360,
    );
    let manifest = json!({
        "provider": "local_storyboard",
        "prompt": prompt,
        "duration_secs": duration_secs,
        "resolution": resolution,
        "frames": [
            { "t": 0.0, "description": format!("Opening scene: {prompt}") },
            { "t": (duration_secs as f64) * 0.5, "description": format!("Midpoint progression: {prompt}") },
            { "t": duration_secs as f64, "description": format!("Closing frame: {prompt}") },
        ],
        "preview_svg_base64": base64_encode(storyboard_svg.as_bytes()),
    });

    let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap_or_default();
    let saved = maybe_write_asset(args.get("output_path").and_then(|v| v.as_str()), &manifest_bytes, ctx);

    HermesToolResult::ok(
        "video_generate",
        format!(
            "Generated offline video storyboard manifest ({duration_secs}s @ {resolution}) for '{prompt}'"
        ),
        Some(manifest),
    )
    .with_artifact(HermesToolArtifact {
        kind: "video_storyboard".into(),
        name: "storyboard.svg".into(),
        mime_type: "image/svg+xml".into(),
        path: saved,
        data_base64: Some(base64_encode(storyboard_svg.as_bytes())),
        metadata: json!({ "duration_secs": duration_secs, "resolution": resolution }),
    })
}

fn maybe_write_asset(
    raw_path: Option<&str>,
    bytes: &[u8],
    ctx: &HermesToolContext,
) -> Option<String> {
    let raw = raw_path?;
    let sec = ctx.security.read().ok()?;
    let resolved = sec.validate_path(Path::new(raw)).ok()?;
    if let Some(parent) = resolved.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&resolved, bytes).ok()?;
    Some(resolved.display().to_string())
}

fn parse_size_dims(size: &str) -> (u32, u32) {
    if let Some((a, b)) = size.split_once('x') {
        if let (Ok(w), Ok(h)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
            return (w.clamp(16, 2048), h.clamp(16, 2048));
        }
    }
    (512, 512)
}

fn generate_deterministic_svg(prompt: &str, w: u32, h: u32) -> String {
    let hash = prompt.bytes().fold(5381u32, |acc, b| acc.wrapping_mul(33).wrapping_add(b as u32));
    let hue = hash % 360;
    let escaped = prompt
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\
<defs><linearGradient id=\"g\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\">\
<stop offset=\"0%\" stop-color=\"hsl({hue},65%,28%)\"/>\
<stop offset=\"100%\" stop-color=\"hsl({},70%,48%)\"/>\
</linearGradient></defs>\
<rect width=\"{w}\" height=\"{h}\" fill=\"url(#g)\"/>\
<text x=\"50%\" y=\"50%\" dominant-baseline=\"middle\" text-anchor=\"middle\" fill=\"#ffffff\" font-family=\"sans-serif\" font-size=\"18\">{escaped}</text>\
</svg>",
        (hue + 55) % 360
    )
}

/// Encodes a valid uncompressed RGBA PNG in pure Rust (`\x89PNG...`).
fn generate_deterministic_png(prompt: &str, w: u32, h: u32) -> Vec<u8> {
    let hash = prompt.bytes().fold(5381u32, |acc, b| acc.wrapping_mul(33).wrapping_add(b as u32));
    let r_base = (hash & 0xFF) as u8;
    let g_base = ((hash >> 8) & 0xFF) as u8;
    let b_base = ((hash >> 16) & 0xFF) as u8;

    let mut raw_scanlines = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for y in 0..h {
        raw_scanlines.push(0); // Filter type 0 (None)
        for x in 0..w {
            raw_scanlines.push(r_base.wrapping_add(((x * 255) / w.max(1)) as u8 / 2));
            raw_scanlines.push(g_base.wrapping_add(((y * 255) / h.max(1)) as u8 / 2));
            raw_scanlines.push(b_base);
            raw_scanlines.push(255);
        }
    }

    // Wrap raw scanlines in uncompressed DEFLATE stored blocks inside a zlib stream
    let mut zlib = Vec::new();
    zlib.push(0x78);
    zlib.push(0x01);
    let chunks: Vec<&[u8]> = raw_scanlines.chunks(65535).collect();
    for (idx, chunk) in chunks.iter().enumerate() {
        let is_last = idx + 1 == chunks.len();
        zlib.push(if is_last { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    let adler = adler32(&raw_scanlines);
    zlib.extend_from_slice(&adler.to_be_bytes());

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    push_png_chunk(&mut png, b"IHDR", &ihdr);
    push_png_chunk(&mut png, b"IDAT", &zlib);
    push_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn push_png_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(tag);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in data {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

// ----------------------------------------------------------------------------
// Pure-Rust Base64 Codec (zero external dependencies)
// ----------------------------------------------------------------------------

pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let clean: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if clean.len() % 4 != 0 {
        return Err("Invalid base64 length".into());
    }
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut vals = [0u8; 4];
        let mut pad = 0usize;
        for (idx, &ch) in chunk.iter().enumerate() {
            if ch == b'=' {
                pad += 1;
                vals[idx] = 0;
            } else {
                vals[idx] = match ch {
                    b'A'..=b'Z' => ch - b'A',
                    b'a'..=b'z' => ch - b'a' + 26,
                    b'0'..=b'9' => ch - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => return Err(format!("Invalid base64 byte '{}'", ch as char)),
                };
            }
        }
        let n = ((vals[0] as u32) << 18)
            | ((vals[1] as u32) << 12)
            | ((vals[2] as u32) << 6)
            | (vals[3] as u32);
        out.push(((n >> 16) & 0xFF) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xFF) as u8);
        }
        if pad < 1 {
            out.push((n & 0xFF) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::toolsets::HermesToolExecutor;

    #[test]
    fn test_html_to_markdown_and_base64() {
        let html = "<html><head><title>Doc Title</title><script>evil()</script></head><body><h1>Hello</h1><p>Visit <a href=\"https://example.com\">Docs</a> &amp; test.</p><ul><li>Item 1</li></ul></body></html>";
        let ex = html_to_markdown(html, true);
        assert_eq!(ex.title, "Doc Title");
        assert!(ex.markdown.contains("# Hello"));
        assert!(ex.markdown.contains("[Docs](https://example.com)"));
        assert!(ex.markdown.contains("- Item 1"));
        assert!(!ex.markdown.contains("evil()"));

        let b = b"RIFF1234WAVE";
        assert_eq!(base64_decode(&base64_encode(b)).unwrap(), b);
    }

    #[tokio::test]
    async fn test_tts_riff_wave_and_stt_and_vision_and_image_gen() {
        let tmp = std::env::temp_dir().join(format!("hermes_media_{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&tmp).unwrap();
        let ctx = HermesToolContext::new(&tmp);

        let tts = HermesToolExecutor::execute(
            "text_to_speech",
            json!({ "text": "Hello Hermes", "provider": "local", "output_path": "speech.wav" }),
            &ctx,
        )
        .await;
        assert!(tts.success, "{}", tts.output);
        let wav_bytes = fs::read(tmp.join("speech.wav")).unwrap();
        assert_eq!(&wav_bytes[0..4], b"RIFF");
        assert_eq!(&wav_bytes[8..12], b"WAVE");

        let stt = HermesToolExecutor::execute(
            "speech_to_text",
            json!({ "audio_path": "speech.wav", "provider": "local" }),
            &ctx,
        )
        .await;
        assert!(stt.success, "{}", stt.output);
        assert!(stt.structured.unwrap()["duration_ms"].as_i64().unwrap() > 0);

        let img = HermesToolExecutor::execute(
            "image_generate",
            json!({ "prompt": "Sunset mountain", "format": "png", "size": "64x64", "output_path": "out.png" }),
            &ctx,
        )
        .await;
        assert!(img.success, "{}", img.output);

        let vis = HermesToolExecutor::execute(
            "vision_analyze",
            json!({ "prompt": "Describe colors", "image_path": "out.png" }),
            &ctx,
        )
        .await;
        assert!(vis.success, "{}", vis.output);
        assert_eq!(vis.structured.unwrap()["width"], 64);

        let _ = fs::remove_dir_all(&tmp);
    }
}
