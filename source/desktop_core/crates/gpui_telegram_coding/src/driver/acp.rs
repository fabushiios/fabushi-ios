//! Agent Client Protocol (ACP v1 & v2) JSON-RPC 2.0 over stdio driver.
//!
//! Routes `OpenCode`, `CursorCli`, `Antigravity`, and `AcpCustom`.

use super::{
    CodingProviderDriver, DriverContext, DriverTurnOutput, DriverTurnRequest,
    GRACEFUL_CANCEL_TIMEOUT, ManagedChild, preview_str, read_bounded_line,
    resolve_provider_executable, value_to_preview, write_ndjson_line,
};
use async_trait::async_trait;
use gpui_telegram_proto::{
    CodingApprovalDecision, CodingHostEvent, CodingProviderKind, CodingTokenUsage,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::io::BufReader;
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

pub struct AcpDriver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpCommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Builds the executable and argument vector for an ACP provider.
pub fn build_acp_command_spec(req: &DriverTurnRequest) -> Result<AcpCommandSpec, String> {
    let program = resolve_provider_executable(
        req.provider_kind,
        req.executable_override.as_deref(),
    )?;

    let mut args = Vec::new();
    let has_override = req
        .executable_override
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());

    if !has_override {
        match req.provider_kind {
            CodingProviderKind::OpenCode
            | CodingProviderKind::Antigravity
            | CodingProviderKind::CursorCli => {
                args.push("acp".to_string());
            }
            CodingProviderKind::AcpCustom => {}
            CodingProviderKind::ClaudeCode | CodingProviderKind::Codex => {}
        }
    }
    args.extend(req.extra_args.iter().cloned());

    Ok(AcpCommandSpec { program, args })
}

#[async_trait]
impl CodingProviderDriver for AcpDriver {
    async fn run_turn(
        &self,
        req: &DriverTurnRequest,
        ctx: &mut DriverContext,
    ) -> DriverTurnOutput {
        let spec = match build_acp_command_spec(req) {
            Ok(s) => s,
            Err(err) => {
                return DriverTurnOutput {
                    ok: false,
                    error: Some(err),
                    ..Default::default()
                };
            }
        };

        let mut cmd = Command::new(&spec.program);
        cmd.current_dir(&req.cwd).args(&spec.args);

        let (mut child, mut stdin, mut stdout) = match ManagedChild::spawn(cmd) {
            Ok(parts) => parts,
            Err(err) => {
                return DriverTurnOutput {
                    ok: false,
                    error: Some(err),
                    ..Default::default()
                };
            }
        };

        let mut next_id: u64 = 1;
        let mut alloc_id = || {
            let id = next_id;
            next_id += 1;
            id
        };

        let mut accumulated_text = String::new();
        let mut saw_error: Option<String> = None;
        let mut token_usage: Option<CodingTokenUsage> = None;

        // 1. initialize
        let init_id = alloc_id();
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false
                },
                "clientInfo": {
                    "name": "gpui-telegram-coding",
                    "title": "GPUI Telegram Coding Host",
                    "version": "1.4.0"
                }
            }
        });

        if let Err(err) = write_ndjson_line(&mut stdin, &init_req).await {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            return DriverTurnOutput {
                ok: false,
                error: Some(err),
                ..Default::default()
            };
        }

        if let Err(err) = wait_rpc_response(
            init_id,
            &mut stdout,
            &mut stdin,
            ctx,
            &mut accumulated_text,
        )
        .await
        {
            let cancelled = ctx.is_cancelled();
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            return DriverTurnOutput {
                ok: false,
                cancelled,
                final_text: accumulated_text,
                error: Some(err),
                ..Default::default()
            };
        }

        // 2. session/load (if resuming) or session/new
        let cwd_str = req.cwd.to_string_lossy().into_owned();
        let mut session_result: Option<Value> = None;
        let mut active_session_id: Option<String> = None;

        if let Some(ref existing_sid) = req.provider_session_id {
            let load_id = alloc_id();
            let load_req = json!({
                "jsonrpc": "2.0",
                "id": load_id,
                "method": "session/load",
                "params": {
                    "sessionId": existing_sid,
                    "cwd": cwd_str,
                    "mcpServers": []
                }
            });
            if write_ndjson_line(&mut stdin, &load_req).await.is_ok() {
                if let Ok(res) = wait_rpc_response(
                    load_id,
                    &mut stdout,
                    &mut stdin,
                    ctx,
                    &mut accumulated_text,
                )
                .await
                {
                    active_session_id = extract_session_id(&res).or_else(|| Some(existing_sid.clone()));
                    session_result = Some(res);
                }
            }
        }

        if active_session_id.is_none() {
            let new_id = alloc_id();
            let new_req = json!({
                "jsonrpc": "2.0",
                "id": new_id,
                "method": "session/new",
                "params": {
                    "cwd": cwd_str,
                    "mcpServers": []
                }
            });
            if let Err(err) = write_ndjson_line(&mut stdin, &new_req).await {
                child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
                return DriverTurnOutput {
                    ok: false,
                    error: Some(err),
                    ..Default::default()
                };
            }
            match wait_rpc_response(
                new_id,
                &mut stdout,
                &mut stdin,
                ctx,
                &mut accumulated_text,
            )
            .await
            {
                Ok(res) => {
                    active_session_id = extract_session_id(&res);
                    session_result = Some(res);
                }
                Err(err) => {
                    let cancelled = ctx.is_cancelled();
                    child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
                    return DriverTurnOutput {
                        ok: false,
                        cancelled,
                        final_text: accumulated_text,
                        error: Some(err),
                        ..Default::default()
                    };
                }
            }
        }

        let Some(session_id) = active_session_id else {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            return DriverTurnOutput {
                ok: false,
                error: Some("ACP agent did not return a sessionId".to_string()),
                ..Default::default()
            };
        };

        // Best-effort model selection if server advertised models in session/new result.
        if let (Some(desired_model), Some(res)) = (req.model.as_deref(), session_result.as_ref()) {
            let supports_models = res.get("models").is_some_and(|v| !v.is_null());
            let current_model = res
                .get("models")
                .and_then(|m| m.get("currentModelId"))
                .and_then(|v| v.as_str());
            if supports_models && current_model != Some(desired_model) {
                let set_id = alloc_id();
                let set_req = json!({
                    "jsonrpc": "2.0",
                    "id": set_id,
                    "method": "session/set_model",
                    "params": {
                        "sessionId": session_id,
                        "modelId": desired_model
                    }
                });
                if write_ndjson_line(&mut stdin, &set_req).await.is_ok() {
                    let _ = wait_rpc_response(
                        set_id,
                        &mut stdout,
                        &mut stdin,
                        ctx,
                        &mut accumulated_text,
                    )
                    .await;
                }
            }
        }

        ctx.emit_turn_started(req, Some(session_id.clone()));

        // 3. session/prompt
        let prompt_id = alloc_id();
        let prompt_req = json!({
            "jsonrpc": "2.0",
            "id": prompt_id,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": [
                    { "type": "text", "text": req.prompt }
                ]
            }
        });

        if let Err(err) = write_ndjson_line(&mut stdin, &prompt_req).await {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            return DriverTurnOutput {
                ok: false,
                provider_session_id: Some(session_id),
                error: Some(err),
                ..Default::default()
            };
        }

        let (apr_tx, mut apr_rx) = mpsc::unbounded_channel::<(Value, Value, CodingApprovalDecision)>();
        let mut cancel_initiated = ctx.is_cancelled();
        let mut prompt_finished = false;
        let mut cancel_timer: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;

        if cancel_initiated {
            send_acp_cancel(&mut stdin, &session_id, &child).await;
            cancel_timer = Some(Box::pin(tokio::time::sleep(GRACEFUL_CANCEL_TIMEOUT)));
        }

        loop {
            tokio::select! {
                line_res = read_bounded_line(&mut stdout) => {
                    match line_res {
                        Ok(Some(line)) => {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                                tracing::debug!("ignoring non-JSON ACP line: {}", preview_str(trimmed, 160));
                                continue;
                            };

                            if is_matching_rpc_response(&msg, prompt_id) {
                                if let Some(err_obj) = msg.get("error") {
                                    saw_error = Some(format_rpc_error(err_obj));
                                } else if let Some(res) = msg.get("result") {
                                    if let Some(usage) = parse_acp_usage(res) {
                                        token_usage = Some(usage);
                                    }
                                    if accumulated_text.is_empty() {
                                        if let Some(txt) = extract_prompt_result_text(res) {
                                            ctx.emit_event(CodingHostEvent::TurnTextDelta {
                                                turn_id: ctx.turn_id.clone(),
                                                delta: txt.clone(),
                                            });
                                            accumulated_text.push_str(&txt);
                                        }
                                    }
                                    if res.get("stopReason").and_then(|v| v.as_str()) == Some("cancelled") {
                                        cancel_initiated = true;
                                    }
                                }
                                prompt_finished = true;
                                break;
                            }

                            if let (Some(req_id), Some(method)) = (msg.get("id").cloned(), msg.get("method").and_then(|m| m.as_str())) {
                                handle_acp_server_request(
                                    req_id,
                                    method,
                                    msg.get("params"),
                                    ctx,
                                    &apr_tx,
                                    &mut stdin,
                                ).await;
                                continue;
                            }

                            if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                                handle_acp_notification(
                                    method,
                                    msg.get("params"),
                                    ctx,
                                    &mut accumulated_text,
                                );
                            }
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(err) => {
                            saw_error = Some(format!("ACP stdout read error: {err}"));
                            break;
                        }
                    }
                }
                Some((rpc_id, options_val, decision)) = apr_rx.recv() => {
                    let reply = build_permission_response(rpc_id, &options_val, decision);
                    if let Err(err) = write_ndjson_line(&mut stdin, &reply).await {
                        saw_error = Some(err);
                        break;
                    }
                }
                _ = ctx.wait_cancelled(), if !cancel_initiated => {
                    cancel_initiated = true;
                    send_acp_cancel(&mut stdin, &session_id, &child).await;
                    cancel_timer = Some(Box::pin(tokio::time::sleep(GRACEFUL_CANCEL_TIMEOUT)));
                }
                _ = async {
                    if let Some(ref mut timer) = cancel_timer {
                        timer.as_mut().await;
                    }
                }, if cancel_timer.is_some() => {
                    child.graceful_terminate(std::time::Duration::from_millis(50)).await;
                    break;
                }
            }
        }

        drop(stdin);
        let exit_status = if cancel_initiated && !prompt_finished {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            None
        } else {
            child.wait_or_kill(GRACEFUL_CANCEL_TIMEOUT).await
        };

        if cancel_initiated {
            return DriverTurnOutput {
                ok: false,
                cancelled: true,
                final_text: accumulated_text,
                error: Some("Turn cancelled".to_string()),
                provider_session_id: Some(session_id),
                token_usage,
            };
        }

        if let Some(err) = saw_error {
            return DriverTurnOutput {
                ok: false,
                cancelled: false,
                final_text: accumulated_text,
                error: Some(err),
                provider_session_id: Some(session_id),
                token_usage,
            };
        }

        if !prompt_finished {
            let stderr = child.stderr_summary();
            let detail = match exit_status {
                Some(st) if !st.success() => format!("ACP process exited with {st}"),
                _ => "ACP process closed stdout before completing `session/prompt`".to_string(),
            };
            let full_err = if stderr.is_empty() {
                detail
            } else {
                format!("{detail}: {stderr}")
            };
            return DriverTurnOutput {
                ok: false,
                cancelled: false,
                final_text: accumulated_text,
                error: Some(full_err),
                provider_session_id: Some(session_id),
                token_usage,
            };
        }

        DriverTurnOutput {
            ok: true,
            cancelled: false,
            final_text: accumulated_text,
            error: None,
            provider_session_id: Some(session_id),
            token_usage,
        }
    }
}

async fn send_acp_cancel(stdin: &mut ChildStdin, session_id: &str, child: &ManagedChild) {
    let cancel_notif = json!({
        "jsonrpc": "2.0",
        "method": "session/cancel",
        "params": {
            "sessionId": session_id
        }
    });
    let _ = write_ndjson_line(stdin, &cancel_notif).await;
    child.signal_interrupt().await;
}

async fn wait_rpc_response(
    target_id: u64,
    stdout: &mut BufReader<ChildStdout>,
    stdin: &mut ChildStdin,
    ctx: &mut DriverContext,
    accumulated_text: &mut String,
) -> Result<Value, String> {
    loop {
        tokio::select! {
            line_res = read_bounded_line(stdout) => {
                let line = line_res
                    .map_err(|e| format!("ACP stdout read failed: {e}"))?
                    .ok_or_else(|| "ACP process closed stdout during handshake".to_string())?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };
                if is_matching_rpc_response(&msg, target_id) {
                    if let Some(err) = msg.get("error") {
                        return Err(format_rpc_error(err));
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                }
                if let (Some(req_id), Some(method)) = (msg.get("id").cloned(), msg.get("method").and_then(|m| m.as_str())) {
                    let err_reply = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": { "code": -32601, "message": format!("Method `{method}` not handled during init") }
                    });
                    let _ = write_ndjson_line(stdin, &err_reply).await;
                } else if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                    handle_acp_notification(method, msg.get("params"), ctx, accumulated_text);
                }
            }
            _ = ctx.wait_cancelled() => {
                return Err("Turn cancelled".to_string());
            }
        }
    }
}

async fn handle_acp_server_request(
    req_id: Value,
    method: &str,
    params: Option<&Value>,
    ctx: &DriverContext,
    apr_tx: &mpsc::UnboundedSender<(Value, Value, CodingApprovalDecision)>,
    stdin: &mut ChildStdin,
) {
    if method == "session/request_permission" {
        let p = params.unwrap_or(&Value::Null);
        let tool_call = p.get("toolCall").unwrap_or(&Value::Null);
        let title = tool_call
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Approve ACP tool action")
            .to_string();
        let kind = tool_call
            .get("kind")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("tool")
            .to_string();
        let detail = tool_call
            .get("rawInput")
            .and_then(|v| value_to_preview(v, 320))
            .unwrap_or_else(|| title.clone());
        let rule_key = format!("acp:{kind}:{}", preview_str(&title, 80));
        let options_val = p.get("options").cloned().unwrap_or(Value::Array(Vec::new()));

        let ctx_clone = ctx.clone();
        let tx_clone = apr_tx.clone();
        tokio::spawn(async move {
            let decision = ctx_clone
                .request_approval(&kind, title, detail, rule_key)
                .await;
            let _ = tx_clone.send((req_id, options_val, decision));
        });
        return;
    }

    let err_reply = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "error": {
            "code": -32601,
            "message": format!("Unsupported client method: {method}")
        }
    });
    let _ = write_ndjson_line(stdin, &err_reply).await;
}

fn build_permission_response(
    rpc_id: Value,
    options_val: &Value,
    decision: CodingApprovalDecision,
) -> Value {
    let options = options_val.as_array().map(Vec::as_slice).unwrap_or(&[]);

    let find_opt = |predicates: &[&str]| -> Option<String> {
        for pred in predicates {
            if let Some(opt) = options.iter().find(|o| {
                o.get("kind")
                    .and_then(|k| k.as_str())
                    .map(|k| k.eq_ignore_ascii_case(pred))
                    .unwrap_or(false)
            }) {
                if let Some(id) = opt.get("optionId").and_then(|v| v.as_str()) {
                    return Some(id.to_string());
                }
            }
        }
        None
    };

    let selected = match decision {
        CodingApprovalDecision::AllowOnce => {
            find_opt(&["allow_once", "allow", "allow_always"])
        }
        CodingApprovalDecision::AllowSession => {
            find_opt(&["allow_always", "allow_once", "allow"])
        }
        CodingApprovalDecision::Deny => {
            find_opt(&["reject_once", "reject_always", "deny", "reject"])
        }
    };

    let outcome = if let Some(option_id) = selected {
        json!({
            "outcome": "selected",
            "optionId": option_id
        })
    } else {
        json!({
            "outcome": "cancelled"
        })
    };

    json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "result": {
            "outcome": outcome
        }
    })
}

fn handle_acp_notification(
    method: &str,
    params: Option<&Value>,
    ctx: &DriverContext,
    accumulated_text: &mut String,
) {
    if method != "session/update" {
        return;
    }
    let Some(p) = params else {
        return;
    };
    let update = p.get("update").unwrap_or(p);
    let kind = update
        .get("sessionUpdate")
        .or_else(|| update.get("type"))
        .or_else(|| update.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match kind {
        "agent_message_chunk" | "agentMessageChunk" => {
            if let Some(delta) = extract_chunk_text(update) {
                accumulated_text.push_str(&delta);
                ctx.emit_event(CodingHostEvent::TurnTextDelta {
                    turn_id: ctx.turn_id.clone(),
                    delta,
                });
            }
        }
        "agent_thought_chunk" | "agentThoughtChunk" => {
            if let Some(delta) = extract_chunk_text(update) {
                ctx.emit_event(CodingHostEvent::TurnThinkingDelta {
                    turn_id: ctx.turn_id.clone(),
                    delta,
                });
            }
        }
        "tool_call" | "toolCall" | "tool_call_update" | "toolCallUpdate" => {
            let call_id = update
                .get("toolCallId")
                .or_else(|| update.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let tool_name = update
                .get("kind")
                .or_else(|| update.get("toolName"))
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let title = update
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&tool_name)
                .to_string();
            let status = update
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or(if kind.contains("update") || kind.contains("Update") {
                    "updated"
                } else {
                    "in_progress"
                })
                .to_string();
            let output_preview = update
                .get("rawOutput")
                .and_then(|v| value_to_preview(v, 240))
                .or_else(|| update.get("content").and_then(|v| value_to_preview(v, 240)));

            ctx.emit_event(CodingHostEvent::TurnToolStep {
                turn_id: ctx.turn_id.clone(),
                call_id,
                tool_name,
                title,
                status,
                output_preview,
            });
        }
        "plan" => {
            let detail = update
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e.get("content").and_then(|c| c.as_str()))
                        .take(4)
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .filter(|s| !s.is_empty());
            ctx.emit_event(CodingHostEvent::TurnActivity {
                turn_id: ctx.turn_id.clone(),
                state: "planning".to_string(),
                detail,
            });
        }
        "current_mode_update" | "currentModeUpdate" => {
            let mode = update
                .get("currentModeId")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            ctx.emit_event(CodingHostEvent::TurnActivity {
                turn_id: ctx.turn_id.clone(),
                state: "mode_updated".to_string(),
                detail: mode,
            });
        }
        _ => {}
    }
}

fn extract_chunk_text(update: &Value) -> Option<String> {
    if let Some(content) = update.get("content") {
        if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
            return Some(text.to_string());
        }
        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        }
        if let Some(arr) = content.as_array() {
            let joined: String = arr
                .iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .collect();
            if !joined.is_empty() {
                return Some(joined);
            }
        }
    }
    update
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn extract_prompt_result_text(res: &Value) -> Option<String> {
    if let Some(s) = res.get("text").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    res.get("content").and_then(|v| value_to_preview(v, 16_000))
}

fn extract_session_id(res: &Value) -> Option<String> {
    res.get("sessionId")
        .or_else(|| res.get("session_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_acp_usage(res: &Value) -> Option<CodingTokenUsage> {
    let usage = res.get("usage")?;
    let input_tokens = usage
        .get("inputTokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("outputTokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cached_tokens = usage
        .get("cachedReadTokens")
        .or_else(|| usage.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cost_usd = usage
        .get("costUsd")
        .or_else(|| usage.get("cost_usd"))
        .and_then(|v| v.as_f64());
    Some(CodingTokenUsage {
        input_tokens,
        output_tokens,
        cached_tokens,
        cost_usd,
    })
}

fn is_matching_rpc_response(msg: &Value, target_id: u64) -> bool {
    if msg.get("method").is_some() {
        return false;
    }
    let Some(id_val) = msg.get("id") else {
        return false;
    };
    id_val.as_u64() == Some(target_id)
        || id_val.as_str().and_then(|s| s.parse::<u64>().ok()) == Some(target_id)
}

fn format_rpc_error(err: &Value) -> String {
    let code = err.get("code").and_then(|c| c.as_i64());
    let message = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("ACP JSON-RPC error");
    match code {
        Some(c) => format!("ACP error ({c}): {message}"),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_telegram_proto::CodingPermissionMode;
    use parking_lot::{Mutex, RwLock};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::{oneshot, watch};

    #[cfg(unix)]
    fn write_mock_script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[test]
    fn acp_command_spec_uses_subcommands_when_not_overridden() {
        let base_req = DriverTurnRequest {
            turn_id: "t1".into(),
            turn_idx: 1,
            prompt: "hi".into(),
            cwd: PathBuf::from("/tmp"),
            provider_kind: CodingProviderKind::AcpCustom,
            permission_mode: CodingPermissionMode::Ask,
            provider_session_id: None,
            model: None,
            executable_override: Some("/bin/sh".into()),
            extra_args: vec!["--flag".into()],
            workspace_path_for_event: "/tmp".into(),
            worktree_path_for_event: None,
            git_branch_for_event: None,
        };
        let spec = build_acp_command_spec(&base_req).unwrap();
        assert_eq!(spec.program, PathBuf::from("/bin/sh"));
        assert_eq!(spec.args, vec!["--flag".to_string()]);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn acp_driver_streams_chunks_handles_approval_and_cancellation() {
        let dir = TempDir::new().unwrap();
        let script = write_mock_script(
            dir.path(),
            "mock_acp.py",
            r#"#!/usr/bin/env python3
import json, sys, time

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"protocolVersion": 1}}), flush=True)
    elif method == "session/new":
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"sessionId": "acp_sess_1"}}), flush=True)
    elif method == "session/load":
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"sessionId": msg["params"]["sessionId"]}}), flush=True)
    elif method == "session/prompt":
        prompt_text = msg["params"]["prompt"][0]["text"]
        if prompt_text == "hang":
            print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp_sess_1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "partial "}}}}), flush=True)
            time.sleep(10)
            sys.exit(0)
        print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp_sess_1", "update": {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "thinking..."}}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp_sess_1", "update": {"sessionUpdate": "tool_call", "toolCallId": "tc_1", "title": "Run tests", "kind": "execute", "status": "in_progress"}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "id": 99, "method": "session/request_permission", "params": {
            "sessionId": "acp_sess_1",
            "toolCall": {"toolCallId": "tc_1", "title": "Run cargo test", "kind": "execute", "rawInput": {"cmd": "cargo test"}},
            "options": [
                {"optionId": "opt_once", "name": "Allow once", "kind": "allow_once"},
                {"optionId": "opt_always", "name": "Always allow", "kind": "allow_always"},
                {"optionId": "opt_deny", "name": "Reject", "kind": "reject_once"}
            ]
        }}), flush=True)
        resp_line = sys.stdin.readline()
        resp = json.loads(resp_line)
        opt = resp["result"]["outcome"].get("optionId", "none")
        print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp_sess_1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": f"approved:{opt}"}}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5}}}), flush=True)
    elif method == "session/cancel":
        sys.exit(0)
"#,
        );

        let (emit_tx, mut emit_rx) = mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let pending = Arc::new(Mutex::new(HashMap::<String, oneshot::Sender<CodingApprovalDecision>>::new()));
        let rules = Arc::new(RwLock::new(HashSet::new()));

        let mut ctx = DriverContext {
            turn_id: "t_acp_1".into(),
            turn_idx: 1,
            permission_mode: CodingPermissionMode::Ask,
            emit: emit_tx,
            cancel_rx,
            pending_approvals: Arc::clone(&pending),
            session_allow_rules: Arc::clone(&rules),
        };

        let req = DriverTurnRequest {
            turn_id: "t_acp_1".into(),
            turn_idx: 1,
            prompt: "hello".into(),
            cwd: dir.path().to_path_buf(),
            provider_kind: CodingProviderKind::AcpCustom,
            permission_mode: CodingPermissionMode::Ask,
            provider_session_id: None,
            model: None,
            executable_override: Some(script.to_string_lossy().into_owned()),
            extra_args: Vec::new(),
            workspace_path_for_event: dir.path().to_string_lossy().into_owned(),
            worktree_path_for_event: None,
            git_branch_for_event: None,
        };

        let driver_fut = AcpDriver.run_turn(&req, &mut ctx);
        let responder = tokio::spawn(async move {
            let mut saw_thought = false;
            let mut saw_tool = false;
            while let Some(ev) = emit_rx.recv().await {
                match ev {
                    CodingHostEvent::TurnThinkingDelta { .. } => saw_thought = true,
                    CodingHostEvent::TurnToolStep { .. } => saw_tool = true,
                    CodingHostEvent::TurnApprovalRequested { request_id, .. } => {
                        let tx = pending.lock().remove(&request_id).unwrap();
                        tx.send(CodingApprovalDecision::AllowSession).unwrap();
                        break;
                    }
                    _ => {}
                }
            }
            assert!(saw_thought);
            assert!(saw_tool);
        });

        let (out, _) = tokio::join!(driver_fut, responder);
        assert!(out.ok, "expected ok, got err: {:?}", out.error);
        assert_eq!(out.final_text, "approved:opt_always");
        assert_eq!(out.provider_session_id.as_deref(), Some("acp_sess_1"));
        assert_eq!(out.token_usage.as_ref().map(|u| u.input_tokens), Some(10));
        assert!(!rules.read().is_empty());

        // Test session/cancel graceful cancellation.
        let (emit_tx2, mut emit_rx2) = mpsc::unbounded_channel();
        let (cancel_tx2, cancel_rx2) = watch::channel(false);
        let mut ctx2 = DriverContext {
            turn_id: "t_acp_2".into(),
            turn_idx: 2,
            permission_mode: CodingPermissionMode::Ask,
            emit: emit_tx2,
            cancel_rx: cancel_rx2,
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            session_allow_rules: Arc::new(RwLock::new(HashSet::new())),
        };
        let req2 = DriverTurnRequest {
            turn_id: "t_acp_2".into(),
            prompt: "hang".into(),
            ..req
        };
        let fut2 = AcpDriver.run_turn(&req2, &mut ctx2);
        let canceller = tokio::spawn(async move {
            while let Some(ev) = emit_rx2.recv().await {
                if matches!(ev, CodingHostEvent::TurnTextDelta { .. }) {
                    let _ = cancel_tx2.send(true);
                    break;
                }
            }
        });
        let (out2, _) = tokio::join!(fut2, canceller);
        assert!(!out2.ok);
        assert!(out2.cancelled);
        assert_eq!(out2.final_text, "partial ");
    }
}
