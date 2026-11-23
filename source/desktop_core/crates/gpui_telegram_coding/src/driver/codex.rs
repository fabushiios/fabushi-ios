//! Codex CLI (`codex app-server`) JSON-RPC 2.0 NDJSON driver.

use super::{
    CodingProviderDriver, DriverContext, DriverTurnOutput, DriverTurnRequest,
    GRACEFUL_CANCEL_TIMEOUT, ManagedChild, preview_str, read_bounded_line,
    resolve_provider_executable, value_to_preview, write_ndjson_line,
};
use async_trait::async_trait;
use gpui_telegram_proto::{
    CodingApprovalDecision, CodingHostEvent, CodingPermissionMode, CodingProviderKind,
    CodingTokenUsage,
};
use serde_json::{Value, json};
use tokio::io::BufReader;
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

pub struct CodexDriver;

#[async_trait]
impl CodingProviderDriver for CodexDriver {
    async fn run_turn(
        &self,
        req: &DriverTurnRequest,
        ctx: &mut DriverContext,
    ) -> DriverTurnOutput {
        let program = match resolve_provider_executable(
            CodingProviderKind::Codex,
            req.executable_override.as_deref(),
        ) {
            Ok(p) => p,
            Err(err) => {
                return DriverTurnOutput {
                    ok: false,
                    error: Some(err),
                    ..Default::default()
                };
            }
        };

        let mut args = vec!["app-server".to_string()];
        args.extend(req.extra_args.iter().cloned());

        let mut cmd = Command::new(&program);
        cmd.current_dir(&req.cwd).args(&args);

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
        let mut token_usage: Option<CodingTokenUsage> = None;
        let mut turn_error: Option<String> = None;

        // 1. initialize
        let init_id = alloc_id();
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "gpui_telegram_coding",
                    "title": "GPUI Telegram Coding Host",
                    "version": "1.4.0"
                },
                "capabilities": {
                    "experimentalApi": false
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
        if let Err(err) = wait_codex_rpc(
            init_id,
            &mut stdout,
            &mut stdin,
            ctx,
            &mut accumulated_text,
            &mut token_usage,
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

        let initialized_notif = json!({
            "jsonrpc": "2.0",
            "method": "initialized"
        });
        if let Err(err) = write_ndjson_line(&mut stdin, &initialized_notif).await {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            return DriverTurnOutput {
                ok: false,
                error: Some(err),
                ..Default::default()
            };
        }

        // 2. thread/resume (if session exists) or thread/start
        let (approval_policy, sandbox_mode) = map_codex_policies(req.permission_mode);
        let cwd_str = req.cwd.to_string_lossy().into_owned();
        let mut thread_id: Option<String> = None;

        if let Some(ref existing_tid) = req.provider_session_id {
            let resume_id = alloc_id();
            let mut params = json!({
                "threadId": existing_tid,
                "cwd": cwd_str,
                "approvalPolicy": approval_policy,
                "sandbox": sandbox_mode
            });
            if let Some(ref model) = req.model {
                params["model"] = json!(model);
            }
            let resume_req = json!({
                "jsonrpc": "2.0",
                "id": resume_id,
                "method": "thread/resume",
                "params": params
            });
            if write_ndjson_line(&mut stdin, &resume_req).await.is_ok() {
                if let Ok(res) = wait_codex_rpc(
                    resume_id,
                    &mut stdout,
                    &mut stdin,
                    ctx,
                    &mut accumulated_text,
                    &mut token_usage,
                )
                .await
                {
                    thread_id = extract_thread_id(&res).or_else(|| Some(existing_tid.clone()));
                }
            }
        }

        if thread_id.is_none() {
            let start_id = alloc_id();
            let mut params = json!({
                "cwd": cwd_str,
                "approvalPolicy": approval_policy,
                "sandbox": sandbox_mode
            });
            if let Some(ref model) = req.model {
                params["model"] = json!(model);
            }
            let start_req = json!({
                "jsonrpc": "2.0",
                "id": start_id,
                "method": "thread/start",
                "params": params
            });
            if let Err(err) = write_ndjson_line(&mut stdin, &start_req).await {
                child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
                return DriverTurnOutput {
                    ok: false,
                    error: Some(err),
                    ..Default::default()
                };
            }
            match wait_codex_rpc(
                start_id,
                &mut stdout,
                &mut stdin,
                ctx,
                &mut accumulated_text,
                &mut token_usage,
            )
            .await
            {
                Ok(res) => {
                    thread_id = extract_thread_id(&res);
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

        let Some(codex_thread_id) = thread_id else {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            return DriverTurnOutput {
                ok: false,
                error: Some("Codex app-server did not return a thread id".to_string()),
                ..Default::default()
            };
        };

        ctx.emit_turn_started(req, Some(codex_thread_id.clone()));

        // 3. turn/start
        let turn_start_id = alloc_id();
        let mut turn_params = json!({
            "threadId": codex_thread_id,
            "input": [
                {
                    "type": "text",
                    "text": req.prompt,
                    "text_elements": []
                }
            ],
            "cwd": cwd_str,
            "approvalPolicy": approval_policy
        });
        if let Some(ref model) = req.model {
            turn_params["model"] = json!(model);
        }
        let turn_start_req = json!({
            "jsonrpc": "2.0",
            "id": turn_start_id,
            "method": "turn/start",
            "params": turn_params
        });
        if let Err(err) = write_ndjson_line(&mut stdin, &turn_start_req).await {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            return DriverTurnOutput {
                ok: false,
                provider_session_id: Some(codex_thread_id),
                error: Some(err),
                ..Default::default()
            };
        }

        let (apr_tx, mut apr_rx) =
            mpsc::unbounded_channel::<(Value, &'static str, CodingApprovalDecision)>();
        let mut codex_turn_id: Option<String> = None;
        let mut turn_completed = false;
        let mut cancel_initiated = ctx.is_cancelled();
        let mut cancel_timer: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;

        if cancel_initiated {
            let int_id = alloc_id();
            send_codex_interrupt(
                &mut stdin,
                int_id,
                &codex_thread_id,
                codex_turn_id.as_deref(),
                &child,
            )
            .await;
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
                                tracing::debug!("ignoring non-JSON Codex line: {}", preview_str(trimmed, 160));
                                continue;
                            };

                            if is_matching_rpc_response(&msg, turn_start_id) {
                                if let Some(err) = msg.get("error") {
                                    turn_error = Some(format_rpc_error(err));
                                    break;
                                }
                                if let Some(tid) = msg
                                    .get("result")
                                    .and_then(|r| r.get("turn"))
                                    .and_then(|t| t.get("id"))
                                    .and_then(|v| v.as_str())
                                {
                                    codex_turn_id = Some(tid.to_string());
                                }
                                continue;
                            }

                            if let (Some(req_id), Some(method)) = (msg.get("id").cloned(), msg.get("method").and_then(|m| m.as_str())) {
                                handle_codex_server_request(
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
                                let params = msg.get("params");
                                if method == "turn/started" {
                                    if let Some(tid) = params
                                        .and_then(|p| p.get("turn"))
                                        .and_then(|t| t.get("id"))
                                        .and_then(|v| v.as_str())
                                    {
                                        codex_turn_id = Some(tid.to_string());
                                    }
                                } else if method == "turn/completed" {
                                    turn_completed = true;
                                    if let Some(turn_obj) = params.and_then(|p| p.get("turn")) {
                                        let status = turn_obj
                                            .get("status")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("completed");
                                        if status == "interrupted" {
                                            cancel_initiated = true;
                                        } else if status == "failed" {
                                            turn_error = turn_obj
                                                .get("error")
                                                .and_then(|e| e.get("message"))
                                                .and_then(|v| v.as_str())
                                                .map(str::to_string)
                                                .or_else(|| Some("Codex turn failed".to_string()));
                                        }
                                    }
                                    break;
                                } else {
                                    handle_codex_notification(
                                        method,
                                        params,
                                        ctx,
                                        &mut accumulated_text,
                                        &mut token_usage,
                                        &mut turn_error,
                                    );
                                }
                            }
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(err) => {
                            turn_error = Some(format!("Codex stdout read error: {err}"));
                            break;
                        }
                    }
                }
                Some((rpc_id, method_kind, decision)) = apr_rx.recv() => {
                    let reply = build_codex_approval_response(rpc_id, method_kind, decision);
                    if let Err(err) = write_ndjson_line(&mut stdin, &reply).await {
                        turn_error = Some(err);
                        break;
                    }
                }
                _ = ctx.wait_cancelled(), if !cancel_initiated => {
                    cancel_initiated = true;
                    let int_id = alloc_id();
                    send_codex_interrupt(
                        &mut stdin,
                        int_id,
                        &codex_thread_id,
                        codex_turn_id.as_deref(),
                        &child,
                    )
                    .await;
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
        let exit_status = if cancel_initiated && !turn_completed {
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
                provider_session_id: Some(codex_thread_id),
                token_usage,
            };
        }

        if let Some(err) = turn_error {
            return DriverTurnOutput {
                ok: false,
                cancelled: false,
                final_text: accumulated_text,
                error: Some(err),
                provider_session_id: Some(codex_thread_id),
                token_usage,
            };
        }

        if !turn_completed {
            let stderr = child.stderr_summary();
            let detail = match exit_status {
                Some(st) if !st.success() => format!("Codex app-server exited with {st}"),
                _ => "Codex app-server closed stdout before `turn/completed`".to_string(),
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
                provider_session_id: Some(codex_thread_id),
                token_usage,
            };
        }

        DriverTurnOutput {
            ok: true,
            cancelled: false,
            final_text: accumulated_text,
            error: None,
            provider_session_id: Some(codex_thread_id),
            token_usage,
        }
    }
}

fn map_codex_policies(mode: CodingPermissionMode) -> (&'static str, &'static str) {
    match mode {
        CodingPermissionMode::Ask => ("on-request", "workspace-write"),
        CodingPermissionMode::AutoEdit => ("on-request", "workspace-write"),
        CodingPermissionMode::FullAccess => ("never", "danger-full-access"),
    }
}

async fn send_codex_interrupt(
    stdin: &mut ChildStdin,
    rpc_id: u64,
    thread_id: &str,
    turn_id: Option<&str>,
    child: &ManagedChild,
) {
    if let Some(tid) = turn_id {
        let req = json!({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "method": "turn/interrupt",
            "params": {
                "threadId": thread_id,
                "turnId": tid
            }
        });
        let _ = write_ndjson_line(stdin, &req).await;
    }
    child.signal_interrupt().await;
}

async fn wait_codex_rpc(
    target_id: u64,
    stdout: &mut BufReader<ChildStdout>,
    stdin: &mut ChildStdin,
    ctx: &mut DriverContext,
    accumulated_text: &mut String,
    token_usage: &mut Option<CodingTokenUsage>,
) -> Result<Value, String> {
    let mut dummy_err = None;
    loop {
        tokio::select! {
            line_res = read_bounded_line(stdout) => {
                let line = line_res
                    .map_err(|e| format!("Codex stdout read failed: {e}"))?
                    .ok_or_else(|| "Codex app-server closed stdout during handshake".to_string())?;
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
                    handle_codex_notification(
                        method,
                        msg.get("params"),
                        ctx,
                        accumulated_text,
                        token_usage,
                        &mut dummy_err,
                    );
                }
            }
            _ = ctx.wait_cancelled() => {
                return Err("Turn cancelled".to_string());
            }
        }
    }
}

async fn handle_codex_server_request(
    req_id: Value,
    method: &str,
    params: Option<&Value>,
    ctx: &DriverContext,
    apr_tx: &mpsc::UnboundedSender<(Value, &'static str, CodingApprovalDecision)>,
    stdin: &mut ChildStdin,
) {
    let p = params.unwrap_or(&Value::Null);
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => {
            let cmd_str = p
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("shell command")
                .to_string();
            let reason = p
                .get("reason")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|r| format!("{cmd_str}\nReason: {r}"))
                .unwrap_or_else(|| cmd_str.clone());
            let first_word = cmd_str
                .split_whitespace()
                .next()
                .unwrap_or("cmd")
                .chars()
                .take(40)
                .collect::<String>();
            let rule_key = format!("codex:cmd:{first_word}");

            let ctx_clone = ctx.clone();
            let tx_clone = apr_tx.clone();
            tokio::spawn(async move {
                let decision = ctx_clone
                    .request_approval(
                        "shell",
                        "Allow command execution".to_string(),
                        reason,
                        rule_key,
                    )
                    .await;
                let _ = tx_clone.send((req_id, "command", decision));
            });
        }
        "item/fileChange/requestApproval" | "applyPatchApproval" => {
            let root = p
                .get("grantRoot")
                .and_then(|v| v.as_str())
                .unwrap_or("workspace");
            let reason = p
                .get("reason")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(root)
                .to_string();
            let rule_key = format!("codex:file:{}", preview_str(root, 60));

            let ctx_clone = ctx.clone();
            let tx_clone = apr_tx.clone();
            tokio::spawn(async move {
                let decision = ctx_clone
                    .request_approval(
                        "edit",
                        "Allow file changes".to_string(),
                        reason,
                        rule_key,
                    )
                    .await;
                let _ = tx_clone.send((req_id, "file", decision));
            });
        }
        _ => {
            let err_reply = json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {
                    "code": -32601,
                    "message": format!("Unsupported Codex server request: {method}")
                }
            });
            let _ = write_ndjson_line(stdin, &err_reply).await;
        }
    }
}

fn build_codex_approval_response(
    rpc_id: Value,
    _method_kind: &'static str,
    decision: CodingApprovalDecision,
) -> Value {
    let decision_str = match decision {
        CodingApprovalDecision::AllowOnce => "accept",
        CodingApprovalDecision::AllowSession => "acceptForSession",
        CodingApprovalDecision::Deny => "decline",
    };
    json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "result": {
            "decision": decision_str
        }
    })
}

fn handle_codex_notification(
    method: &str,
    params: Option<&Value>,
    ctx: &DriverContext,
    accumulated_text: &mut String,
    token_usage: &mut Option<CodingTokenUsage>,
    turn_error: &mut Option<String>,
) {
    let Some(p) = params else {
        return;
    };
    match method {
        "item/agentMessage/delta" => {
            if let Some(delta) = p.get("delta").and_then(|v| v.as_str()) {
                accumulated_text.push_str(delta);
                ctx.emit_event(CodingHostEvent::TurnTextDelta {
                    turn_id: ctx.turn_id.clone(),
                    delta: delta.to_string(),
                });
            }
        }
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
            if let Some(delta) = p.get("delta").and_then(|v| v.as_str()) {
                ctx.emit_event(CodingHostEvent::TurnThinkingDelta {
                    turn_id: ctx.turn_id.clone(),
                    delta: delta.to_string(),
                });
            }
        }
        "item/started" => {
            if let Some(item) = p.get("item") {
                emit_codex_item_step(item, "in_progress", ctx);
            }
        }
        "item/completed" => {
            if let Some(item) = p.get("item") {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if item_type == "agentMessage" {
                    if accumulated_text.is_empty() {
                        if let Some(txt) = item.get("text").and_then(|v| v.as_str()) {
                            if !txt.is_empty() {
                                accumulated_text.push_str(txt);
                                ctx.emit_event(CodingHostEvent::TurnTextDelta {
                                    turn_id: ctx.turn_id.clone(),
                                    delta: txt.to_string(),
                                });
                            }
                        }
                    }
                } else {
                    let status = item
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("completed");
                    emit_codex_item_step(item, status, ctx);
                }
            }
        }
        "thread/tokenUsage/updated" => {
            if let Some(breakdown) = p
                .get("tokenUsage")
                .and_then(|u| u.get("last").or_else(|| u.get("total")))
            {
                let input_tokens = breakdown
                    .get("inputTokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let output_tokens = breakdown
                    .get("outputTokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cached_tokens = breakdown
                    .get("cachedInputTokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                *token_usage = Some(CodingTokenUsage {
                    input_tokens,
                    output_tokens,
                    cached_tokens,
                    cost_usd: None,
                });
            }
        }
        "error" => {
            let will_retry = p.get("willRetry").and_then(|v| v.as_bool()).unwrap_or(false);
            if !will_retry {
                if let Some(msg) = p
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                {
                    *turn_error = Some(msg.to_string());
                }
            }
        }
        _ => {}
    }
}

fn emit_codex_item_step(item: &Value, default_status: &str, ctx: &DriverContext) {
    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let call_id = item
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("item")
        .to_string();

    match item_type {
        "commandExecution" => {
            let command = item
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("command");
            let status = item
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or(default_status)
                .to_string();
            let output_preview = item
                .get("aggregatedOutput")
                .and_then(|v| value_to_preview(v, 240));
            ctx.emit_event(CodingHostEvent::TurnToolStep {
                turn_id: ctx.turn_id.clone(),
                call_id,
                tool_name: "bash".to_string(),
                title: preview_str(command, 90),
                status,
                output_preview,
            });
        }
        "fileChange" => {
            let count = item
                .get("changes")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let status = item
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or(default_status)
                .to_string();
            ctx.emit_event(CodingHostEvent::TurnToolStep {
                turn_id: ctx.turn_id.clone(),
                call_id,
                tool_name: "file_change".to_string(),
                title: format!("Modify {count} file(s)"),
                status,
                output_preview: None,
            });
        }
        "mcpToolCall" | "dynamicToolCall" => {
            let tool = item
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let status = item
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or(default_status)
                .to_string();
            ctx.emit_event(CodingHostEvent::TurnToolStep {
                turn_id: ctx.turn_id.clone(),
                call_id,
                tool_name: tool.clone(),
                title: tool,
                status,
                output_preview: item.get("result").and_then(|v| value_to_preview(v, 240)),
            });
        }
        "plan" => {
            let detail = item.get("text").and_then(|v| value_to_preview(v, 200));
            ctx.emit_event(CodingHostEvent::TurnActivity {
                turn_id: ctx.turn_id.clone(),
                state: "planning".to_string(),
                detail,
            });
        }
        _ => {}
    }
}

fn extract_thread_id(res: &Value) -> Option<String> {
    res.get("thread")
        .and_then(|t| t.get("id"))
        .or_else(|| res.get("threadId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
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
        .unwrap_or("Codex app-server JSON-RPC error");
    match code {
        Some(c) => format!("Codex error ({c}): {message}"),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::{Mutex, RwLock};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
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

    #[tokio::test]
    #[cfg(unix)]
    async fn codex_driver_streams_and_handles_approval_and_interrupt() {
        let dir = TempDir::new().unwrap();
        let script = write_mock_script(
            dir.path(),
            "mock_codex.py",
            r#"#!/usr/bin/env python3
import json, sys, time

assert sys.argv[1] == "app-server"

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"userAgent": "codex-mock"}}), flush=True)
    elif method == "initialized":
        continue
    elif method in ("thread/start", "thread/resume"):
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"thread": {"id": "codex_thr_1"}}}), flush=True)
    elif method == "turn/start":
        prompt = msg["params"]["input"][0]["text"]
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"turn": {"id": "codex_turn_1", "status": "inProgress"}}}), flush=True)
        if prompt == "cancel_me":
            print(json.dumps({"jsonrpc": "2.0", "method": "item/agentMessage/delta", "params": {"threadId": "codex_thr_1", "turnId": "codex_turn_1", "itemId": "it1", "delta": "working..."}}), flush=True)
            for l2 in sys.stdin:
                m2 = json.loads(l2)
                if m2.get("method") == "turn/interrupt":
                    print(json.dumps({"jsonrpc": "2.0", "id": m2["id"], "result": {}}), flush=True)
                    print(json.dumps({"jsonrpc": "2.0", "method": "turn/completed", "params": {"threadId": "codex_thr_1", "turn": {"id": "codex_turn_1", "status": "interrupted"}}}), flush=True)
                    sys.exit(0)
            sys.exit(0)
        print(json.dumps({"jsonrpc": "2.0", "method": "item/reasoning/summaryTextDelta", "params": {"threadId": "codex_thr_1", "turnId": "codex_turn_1", "itemId": "r1", "delta": "planning", "summaryIndex": 0}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "item/started", "params": {"threadId": "codex_thr_1", "turnId": "codex_turn_1", "startedAtMs": 1, "item": {"type": "commandExecution", "id": "cmd_1", "command": "cargo check", "status": "inProgress"}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "id": 77, "method": "item/commandExecution/requestApproval", "params": {"threadId": "codex_thr_1", "turnId": "codex_turn_1", "itemId": "cmd_1", "startedAtMs": 1, "environmentId": None, "command": "cargo check"}}), flush=True)
        resp = json.loads(sys.stdin.readline())
        dec = resp["result"]["decision"]
        print(json.dumps({"jsonrpc": "2.0", "method": "item/agentMessage/delta", "params": {"threadId": "codex_thr_1", "turnId": "codex_turn_1", "itemId": "m1", "delta": f"codex:{dec}"}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "thread/tokenUsage/updated", "params": {"threadId": "codex_thr_1", "turnId": "codex_turn_1", "tokenUsage": {"total": {"totalTokens": 30, "inputTokens": 20, "cachedInputTokens": 5, "outputTokens": 10, "reasoningOutputTokens": 0}, "last": {"totalTokens": 30, "inputTokens": 20, "cachedInputTokens": 5, "outputTokens": 10, "reasoningOutputTokens": 0}}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "turn/completed", "params": {"threadId": "codex_thr_1", "turn": {"id": "codex_turn_1", "status": "completed"}}}), flush=True)
"#,
        );

        // 1. Test AllowOnce approval + streaming + token usage.
        let (emit_tx, mut emit_rx) = mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let pending = Arc::new(Mutex::new(HashMap::<String, oneshot::Sender<CodingApprovalDecision>>::new()));
        let rules = Arc::new(RwLock::new(HashSet::new()));
        let mut ctx = DriverContext {
            turn_id: "t_cx_1".into(),
            turn_idx: 1,
            permission_mode: CodingPermissionMode::Ask,
            emit: emit_tx,
            cancel_rx,
            pending_approvals: Arc::clone(&pending),
            session_allow_rules: rules,
        };
        let req = DriverTurnRequest {
            turn_id: "t_cx_1".into(),
            turn_idx: 1,
            prompt: "check".into(),
            cwd: dir.path().to_path_buf(),
            provider_kind: CodingProviderKind::Codex,
            permission_mode: CodingPermissionMode::Ask,
            provider_session_id: None,
            model: None,
            executable_override: Some(script.to_string_lossy().into_owned()),
            extra_args: Vec::new(),
            workspace_path_for_event: dir.path().to_string_lossy().into_owned(),
            worktree_path_for_event: None,
            git_branch_for_event: None,
        };
        let fut = CodexDriver.run_turn(&req, &mut ctx);
        let responder = tokio::spawn(async move {
            while let Some(ev) = emit_rx.recv().await {
                if let CodingHostEvent::TurnApprovalRequested { request_id, .. } = ev {
                    let tx = pending.lock().remove(&request_id).unwrap();
                    tx.send(CodingApprovalDecision::AllowOnce).unwrap();
                    break;
                }
            }
        });
        let (out, _) = tokio::join!(fut, responder);
        assert!(out.ok, "err: {:?}", out.error);
        assert_eq!(out.final_text, "codex:accept");
        assert_eq!(out.provider_session_id.as_deref(), Some("codex_thr_1"));
        assert_eq!(out.token_usage.as_ref().map(|u| u.output_tokens), Some(10));

        // 2. Test CancelTurn -> turn/interrupt graceful cancellation.
        let (emit_tx2, mut emit_rx2) = mpsc::unbounded_channel();
        let (cancel_tx2, cancel_rx2) = watch::channel(false);
        let mut ctx2 = DriverContext {
            turn_id: "t_cx_2".into(),
            turn_idx: 2,
            permission_mode: CodingPermissionMode::Ask,
            emit: emit_tx2,
            cancel_rx: cancel_rx2,
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            session_allow_rules: Arc::new(RwLock::new(HashSet::new())),
        };
        let req2 = DriverTurnRequest {
            turn_id: "t_cx_2".into(),
            prompt: "cancel_me".into(),
            ..req
        };
        let fut2 = CodexDriver.run_turn(&req2, &mut ctx2);
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
        assert_eq!(out2.final_text, "working...");
    }
}
