//! Claude Code CLI (`stream-json`) driver.

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
use std::path::PathBuf;
use tokio::process::Command;
use tokio::sync::mpsc;

pub struct ClaudeDriver;

/// Builds the CLI arguments for Claude Code `stream-json` execution.
pub fn build_claude_args(req: &DriverTurnRequest) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
    ];

    if let Some(ref sid) = req.provider_session_id {
        if !sid.trim().is_empty() {
            args.push("--resume".to_string());
            args.push(sid.clone());
        }
    }

    if let Some(ref model) = req.model {
        if !model.trim().is_empty() {
            args.push("--model".to_string());
            args.push(model.clone());
        }
    }

    match req.permission_mode {
        CodingPermissionMode::Ask => {
            args.push("--permission-prompt-tool".to_string());
            args.push("stdio".to_string());
        }
        CodingPermissionMode::AutoEdit => {
            args.push("--permission-mode".to_string());
            args.push("acceptEdits".to_string());
        }
        CodingPermissionMode::FullAccess => {
            args.push("--permission-mode".to_string());
            args.push("bypassPermissions".to_string());
        }
    }

    args.extend(req.extra_args.iter().cloned());
    args
}

#[async_trait]
impl CodingProviderDriver for ClaudeDriver {
    async fn run_turn(
        &self,
        req: &DriverTurnRequest,
        ctx: &mut DriverContext,
    ) -> DriverTurnOutput {
        let program: PathBuf = match resolve_provider_executable(
            CodingProviderKind::ClaudeCode,
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

        let args = build_claude_args(req);
        let mut cmd = Command::new(&program);
        cmd.current_dir(&req.cwd)
            .args(&args)
            .env_remove("CLAUDECODE");

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

        let user_msg = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": req.prompt
            }
        });
        if let Err(err) = write_ndjson_line(&mut stdin, &user_msg).await {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            return DriverTurnOutput {
                ok: false,
                error: Some(err),
                ..Default::default()
            };
        }

        let (apr_tx, mut apr_rx) =
            mpsc::unbounded_channel::<(String, Value, CodingApprovalDecision)>();

        let mut session_id = req.provider_session_id.clone();
        let mut turn_started_emitted = false;
        let mut streamed_message_text = false;
        let mut streamed_message_thinking = false;
        let mut accumulated_text = String::new();
        let mut token_usage: Option<CodingTokenUsage> = None;
        let mut saw_result = false;
        let mut result_is_error = false;
        let mut result_error_text: Option<String> = None;
        let mut cancel_initiated = ctx.is_cancelled();
        let mut cancel_timer: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;

        if cancel_initiated {
            send_claude_interrupt(&mut stdin, &child).await;
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
                                tracing::debug!("ignoring non-JSON Claude line: {}", preview_str(trimmed, 160));
                                continue;
                            };

                            if let Some(sid) = msg.get("session_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                session_id = Some(sid.to_string());
                            }

                            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match msg_type {
                                "system" => {
                                    if !turn_started_emitted {
                                        turn_started_emitted = true;
                                        ctx.emit_turn_started(req, session_id.clone());
                                    }
                                    if let Some(sub) = msg.get("subtype").and_then(|v| v.as_str()) {
                                        ctx.emit_event(CodingHostEvent::TurnActivity {
                                            turn_id: ctx.turn_id.clone(),
                                            state: sub.to_string(),
                                            detail: msg.get("model").and_then(|v| v.as_str()).map(str::to_string),
                                        });
                                    }
                                }
                                "stream_event" => {
                                    if !turn_started_emitted {
                                        turn_started_emitted = true;
                                        ctx.emit_turn_started(req, session_id.clone());
                                    }
                                    if let Some(ev) = msg.get("event") {
                                        let ev_type = ev.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                        match ev_type {
                                            "message_start" => {
                                                streamed_message_text = false;
                                                streamed_message_thinking = false;
                                            }
                                            "content_block_start" => {
                                                if let Some(cb) = ev.get("content_block") {
                                                    if cb.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                                                        let call_id = cb.get("id").and_then(|v| v.as_str()).unwrap_or("tool").to_string();
                                                        let name = cb.get("name").and_then(|v| v.as_str()).unwrap_or("tool").to_string();
                                                        ctx.emit_event(CodingHostEvent::TurnToolStep {
                                                            turn_id: ctx.turn_id.clone(),
                                                            call_id,
                                                            tool_name: name.clone(),
                                                            title: name,
                                                            status: "in_progress".to_string(),
                                                            output_preview: None,
                                                        });
                                                    }
                                                }
                                            }
                                            "content_block_delta" => {
                                                if let Some(delta) = ev.get("delta") {
                                                    let d_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                                    if d_type == "text_delta" {
                                                        if let Some(txt) = delta.get("text").and_then(|v| v.as_str()) {
                                                            streamed_message_text = true;
                                                            accumulated_text.push_str(txt);
                                                            ctx.emit_event(CodingHostEvent::TurnTextDelta {
                                                                turn_id: ctx.turn_id.clone(),
                                                                delta: txt.to_string(),
                                                            });
                                                        }
                                                    } else if d_type == "thinking_delta" {
                                                        if let Some(thinking) = delta.get("thinking").and_then(|v| v.as_str()) {
                                                            streamed_message_thinking = true;
                                                            ctx.emit_event(CodingHostEvent::TurnThinkingDelta {
                                                                turn_id: ctx.turn_id.clone(),
                                                                delta: thinking.to_string(),
                                                            });
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                "assistant" => {
                                    if !turn_started_emitted {
                                        turn_started_emitted = true;
                                        ctx.emit_turn_started(req, session_id.clone());
                                    }
                                    if let Some(blocks) = msg
                                        .get("message")
                                        .and_then(|m| m.get("content"))
                                        .and_then(|c| c.as_array())
                                    {
                                        for block in blocks {
                                            let b_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                            match b_type {
                                                "thinking" => {
                                                    if !streamed_message_thinking {
                                                        if let Some(thinking) = block.get("thinking").and_then(|v| v.as_str()) {
                                                            if !thinking.is_empty() {
                                                                ctx.emit_event(CodingHostEvent::TurnThinkingDelta {
                                                                    turn_id: ctx.turn_id.clone(),
                                                                    delta: thinking.to_string(),
                                                                });
                                                            }
                                                        }
                                                    }
                                                }
                                                "text" => {
                                                    if !streamed_message_text {
                                                        if let Some(txt) = block.get("text").and_then(|v| v.as_str()) {
                                                            if !txt.is_empty() {
                                                                accumulated_text.push_str(txt);
                                                                ctx.emit_event(CodingHostEvent::TurnTextDelta {
                                                                    turn_id: ctx.turn_id.clone(),
                                                                    delta: txt.to_string(),
                                                                });
                                                            }
                                                        }
                                                    }
                                                }
                                                "tool_use" => {
                                                    let call_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("tool").to_string();
                                                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("tool").to_string();
                                                    let detail = block
                                                        .get("input")
                                                        .and_then(|v| format_claude_tool_title(&name, v))
                                                        .unwrap_or_else(|| name.clone());
                                                    ctx.emit_event(CodingHostEvent::TurnToolStep {
                                                        turn_id: ctx.turn_id.clone(),
                                                        call_id,
                                                        tool_name: name,
                                                        title: detail,
                                                        status: "in_progress".to_string(),
                                                        output_preview: None,
                                                    });
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    streamed_message_text = false;
                                    streamed_message_thinking = false;
                                }
                                "user" => {
                                    if let Some(blocks) = msg
                                        .get("message")
                                        .and_then(|m| m.get("content"))
                                        .and_then(|c| c.as_array())
                                    {
                                        for block in blocks {
                                            if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                                                let call_id = block
                                                    .get("tool_use_id")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("tool")
                                                    .to_string();
                                                let is_err = block
                                                    .get("is_error")
                                                    .and_then(|v| v.as_bool())
                                                    .unwrap_or(false);
                                                let preview = block
                                                    .get("content")
                                                    .and_then(|v| value_to_preview(v, 240));
                                                ctx.emit_event(CodingHostEvent::TurnToolStep {
                                                    turn_id: ctx.turn_id.clone(),
                                                    call_id,
                                                    tool_name: "tool".to_string(),
                                                    title: if is_err { "Tool error".to_string() } else { "Tool completed".to_string() },
                                                    status: if is_err { "failed".to_string() } else { "completed".to_string() },
                                                    output_preview: preview,
                                                });
                                            }
                                        }
                                    }
                                }
                                "control_request" => {
                                    if !turn_started_emitted {
                                        turn_started_emitted = true;
                                        ctx.emit_turn_started(req, session_id.clone());
                                    }
                                    let proto_req_id = msg
                                        .get("request_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let inner = msg.get("request").unwrap_or(&Value::Null);
                                    let subtype = inner
                                        .get("subtype")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("can_use_tool");

                                    if subtype == "can_use_tool" || inner.get("tool_name").is_some() {
                                        let tool_name = inner
                                            .get("tool_name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Tool")
                                            .to_string();
                                        let input_val = inner
                                            .get("input")
                                            .cloned()
                                            .filter(|v| v.is_object())
                                            .unwrap_or_else(|| json!({}));
                                        let kind = classify_claude_tool_kind(&tool_name);
                                        let detail = value_to_preview(&input_val, 360)
                                            .unwrap_or_else(|| tool_name.clone());
                                        let target = summarize_claude_rule_target(&input_val);
                                        let rule_key = format!("claude:{tool_name}:{target}");
                                        let title = format!("Allow {tool_name}");

                                        let ctx_clone = ctx.clone();
                                        let tx_clone = apr_tx.clone();
                                        tokio::spawn(async move {
                                            let decision = ctx_clone
                                                .request_approval(kind, title, detail, rule_key)
                                                .await;
                                            let _ = tx_clone.send((proto_req_id, input_val, decision));
                                        });
                                    } else {
                                        let ack = json!({
                                            "type": "control_response",
                                            "response": {
                                                "subtype": "success",
                                                "request_id": proto_req_id,
                                                "response": {}
                                            }
                                        });
                                        let _ = write_ndjson_line(&mut stdin, &ack).await;
                                    }
                                }
                                "result" => {
                                    saw_result = true;
                                    result_is_error = msg
                                        .get("is_error")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false);
                                    token_usage = parse_claude_usage(&msg);
                                    if let Some(res_str) = msg.get("result").and_then(|v| v.as_str()) {
                                        if result_is_error {
                                            result_error_text = Some(res_str.to_string());
                                        } else if accumulated_text.is_empty() && !res_str.is_empty() {
                                            accumulated_text.push_str(res_str);
                                            ctx.emit_event(CodingHostEvent::TurnTextDelta {
                                                turn_id: ctx.turn_id.clone(),
                                                delta: res_str.to_string(),
                                            });
                                        }
                                    }
                                    if result_is_error && result_error_text.is_none() {
                                        if let Some(errs) = msg.get("errors").and_then(|v| v.as_array()) {
                                            let joined = errs
                                                .iter()
                                                .filter_map(|e| e.as_str())
                                                .collect::<Vec<_>>()
                                                .join("; ");
                                            if !joined.is_empty() {
                                                result_error_text = Some(joined);
                                            }
                                        }
                                    }
                                    break;
                                }
                                _ => {}
                            }
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(err) => {
                            result_is_error = true;
                            result_error_text = Some(format!("Claude stdout read error: {err}"));
                            break;
                        }
                    }
                }
                Some((proto_req_id, input_val, decision)) = apr_rx.recv() => {
                    let resp = build_claude_control_response(&proto_req_id, &input_val, decision);
                    if let Err(err) = write_ndjson_line(&mut stdin, &resp).await {
                        result_is_error = true;
                        result_error_text = Some(err);
                        break;
                    }
                }
                _ = ctx.wait_cancelled(), if !cancel_initiated => {
                    cancel_initiated = true;
                    send_claude_interrupt(&mut stdin, &child).await;
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
        let exit_status = if cancel_initiated && !saw_result {
            child.graceful_terminate(GRACEFUL_CANCEL_TIMEOUT).await;
            None
        } else {
            child.wait_or_kill(GRACEFUL_CANCEL_TIMEOUT).await
        };

        if !turn_started_emitted {
            ctx.emit_turn_started(req, session_id.clone());
        }

        if cancel_initiated {
            return DriverTurnOutput {
                ok: false,
                cancelled: true,
                final_text: accumulated_text,
                error: Some("Turn cancelled".to_string()),
                provider_session_id: session_id,
                token_usage,
            };
        }

        if result_is_error {
            return DriverTurnOutput {
                ok: false,
                cancelled: false,
                final_text: accumulated_text,
                error: result_error_text.or_else(|| Some("Claude turn reported an error".to_string())),
                provider_session_id: session_id,
                token_usage,
            };
        }

        if !saw_result {
            let stderr = child.stderr_summary();
            let detail = match exit_status {
                Some(st) if !st.success() => format!("Claude CLI exited with {st}"),
                _ => "Claude CLI closed stdout before emitting `result`".to_string(),
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
                provider_session_id: session_id,
                token_usage,
            };
        }

        DriverTurnOutput {
            ok: true,
            cancelled: false,
            final_text: accumulated_text,
            error: None,
            provider_session_id: session_id,
            token_usage,
        }
    }
}

async fn send_claude_interrupt(stdin: &mut tokio::process::ChildStdin, child: &ManagedChild) {
    let interrupt = json!({
        "type": "control_request",
        "request_id": format!("int_{}", uuid::Uuid::new_v4().simple()),
        "request": {
            "subtype": "interrupt"
        }
    });
    let _ = write_ndjson_line(stdin, &interrupt).await;
    child.signal_interrupt().await;
}

fn build_claude_control_response(
    request_id: &str,
    input_val: &Value,
    decision: CodingApprovalDecision,
) -> Value {
    let inner = match decision {
        CodingApprovalDecision::AllowOnce | CodingApprovalDecision::AllowSession => {
            json!({
                "behavior": "allow",
                "updatedInput": input_val
            })
        }
        CodingApprovalDecision::Deny => {
            json!({
                "behavior": "deny",
                "message": "Denied by user"
            })
        }
    };
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": inner
        }
    })
}

fn classify_claude_tool_kind(tool_name: &str) -> &'static str {
    match tool_name {
        "Bash" | "bash" => "shell",
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "edit",
        "Read" | "Glob" | "Grep" => "read",
        _ => "tool",
    }
}

fn summarize_claude_rule_target(input: &Value) -> String {
    if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
        return cmd
            .split_whitespace()
            .next()
            .unwrap_or("cmd")
            .chars()
            .take(40)
            .collect();
    }
    if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
        return preview_str(path, 60);
    }
    "any".to_string()
}

fn format_claude_tool_title(tool_name: &str, input: &Value) -> Option<String> {
    if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
        return Some(format!("{tool_name}: {}", preview_str(cmd, 80)));
    }
    if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
        return Some(format!("{tool_name}: {}", preview_str(path, 80)));
    }
    None
}

fn parse_claude_usage(msg: &Value) -> Option<CodingTokenUsage> {
    let usage = msg.get("usage");
    let cost_usd = msg
        .get("total_cost_usd")
        .or_else(|| msg.get("cost_usd"))
        .and_then(|v| v.as_f64());

    if usage.is_none() && cost_usd.is_none() {
        return None;
    }

    let u = usage.unwrap_or(&Value::Null);
    let input_tokens = u
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output_tokens = u
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_read = u
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_create = u
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cached_tokens = u
        .get("cached_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(cache_read + cache_create);

    Some(CodingTokenUsage {
        input_tokens,
        output_tokens,
        cached_tokens,
        cost_usd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn claude_cli_args_cover_all_permission_modes_and_resume() {
        let mut req = DriverTurnRequest {
            turn_id: "t1".into(),
            turn_idx: 1,
            prompt: "hi".into(),
            cwd: PathBuf::from("/tmp"),
            provider_kind: CodingProviderKind::ClaudeCode,
            permission_mode: CodingPermissionMode::Ask,
            provider_session_id: Some("sess_prev".into()),
            model: Some("claude-sonnet-4-6".into()),
            executable_override: None,
            extra_args: vec!["--max-turns".into(), "4".into()],
            workspace_path_for_event: "/tmp".into(),
            worktree_path_for_event: None,
            git_branch_for_event: None,
        };
        let ask_args = build_claude_args(&req);
        assert!(ask_args.windows(2).any(|w| w == ["--permission-prompt-tool", "stdio"]));
        assert!(ask_args.windows(2).any(|w| w == ["--resume", "sess_prev"]));
        assert!(ask_args.windows(2).any(|w| w == ["--model", "claude-sonnet-4-6"]));

        req.permission_mode = CodingPermissionMode::AutoEdit;
        assert!(build_claude_args(&req).windows(2).any(|w| w == ["--permission-mode", "acceptEdits"]));

        req.permission_mode = CodingPermissionMode::FullAccess;
        assert!(build_claude_args(&req).windows(2).any(|w| w == ["--permission-mode", "bypassPermissions"]));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn claude_driver_handles_stream_events_control_request_deny_and_result() {
        let dir = TempDir::new().unwrap();
        let script = write_mock_script(
            dir.path(),
            "mock_claude.py",
            r#"#!/usr/bin/env python3
import json, sys

line = sys.stdin.readline()
user_msg = json.loads(line)
assert user_msg["type"] == "user"

print(json.dumps({"type": "system", "subtype": "init", "session_id": "cl_sess_1", "model": "sonnet"}), flush=True)
print(json.dumps({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "thinking_delta", "thinking": "checking..."}}}), flush=True)
print(json.dumps({"type": "control_request", "request_id": "cr_1", "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "rm -rf /tmp/x"}}}), flush=True)

resp = json.loads(sys.stdin.readline())
assert resp["type"] == "control_response"
behavior = resp["response"]["response"]["behavior"]
print(json.dumps({"type": "assistant", "session_id": "cl_sess_1", "message": {"content": [{"type": "text", "text": f"tool={behavior}"}]}}), flush=True)
print(json.dumps({"type": "result", "subtype": "success", "is_error": False, "session_id": "cl_sess_1", "total_cost_usd": 0.02, "usage": {"input_tokens": 100, "output_tokens": 25, "cache_read_input_tokens": 40}}), flush=True)
"#,
        );

        let (emit_tx, mut emit_rx) = mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let pending = Arc::new(Mutex::new(HashMap::<String, oneshot::Sender<CodingApprovalDecision>>::new()));
        let rules = Arc::new(RwLock::new(HashSet::new()));

        let mut ctx = DriverContext {
            turn_id: "t_cl_1".into(),
            turn_idx: 1,
            permission_mode: CodingPermissionMode::Ask,
            emit: emit_tx,
            cancel_rx,
            pending_approvals: Arc::clone(&pending),
            session_allow_rules: rules,
        };
        let req = DriverTurnRequest {
            turn_id: "t_cl_1".into(),
            turn_idx: 1,
            prompt: "delete tmp".into(),
            cwd: dir.path().to_path_buf(),
            provider_kind: CodingProviderKind::ClaudeCode,
            permission_mode: CodingPermissionMode::Ask,
            provider_session_id: None,
            model: None,
            executable_override: Some(script.to_string_lossy().into_owned()),
            extra_args: Vec::new(),
            workspace_path_for_event: dir.path().to_string_lossy().into_owned(),
            worktree_path_for_event: None,
            git_branch_for_event: None,
        };

        let driver_fut = ClaudeDriver.run_turn(&req, &mut ctx);
        let responder = tokio::spawn(async move {
            while let Some(ev) = emit_rx.recv().await {
                if let CodingHostEvent::TurnApprovalRequested { request_id, .. } = ev {
                    let tx = pending.lock().remove(&request_id).unwrap();
                    tx.send(CodingApprovalDecision::Deny).unwrap();
                    break;
                }
            }
        });

        let (out, _) = tokio::join!(driver_fut, responder);
        assert!(out.ok, "err: {:?}", out.error);
        assert_eq!(out.final_text, "tool=deny");
        assert_eq!(out.provider_session_id.as_deref(), Some("cl_sess_1"));
        let usage = out.token_usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 25);
        assert_eq!(usage.cached_tokens, 40);
        assert_eq!(usage.cost_usd, Some(0.02));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn claude_driver_handles_allow_once_and_cancel() {
        let dir = TempDir::new().unwrap();
        let script = write_mock_script(
            dir.path(),
            "mock_claude_cancel.py",
            r#"#!/usr/bin/env python3
import json, sys, time

user_msg = json.loads(sys.stdin.readline())
prompt = user_msg["message"]["content"]
print(json.dumps({"type": "system", "subtype": "init", "session_id": "cl_sess_2"}), flush=True)
if prompt == "hang":
    print(json.dumps({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "half "}}}), flush=True)
    time.sleep(10)
    sys.exit(0)
print(json.dumps({"type": "control_request", "request_id": "cr_2", "request": {"subtype": "can_use_tool", "tool_name": "Edit", "input": {"file_path": "a.rs"}}}), flush=True)
resp = json.loads(sys.stdin.readline())
behavior = resp["response"]["response"]["behavior"]
print(json.dumps({"type": "result", "subtype": "success", "is_error": False, "result": f"edit={behavior}", "session_id": "cl_sess_2"}), flush=True)
"#,
        );

        let (emit_tx, mut emit_rx) = mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let pending = Arc::new(Mutex::new(HashMap::<String, oneshot::Sender<CodingApprovalDecision>>::new()));
        let mut ctx = DriverContext {
            turn_id: "t_cl_2".into(),
            turn_idx: 1,
            permission_mode: CodingPermissionMode::Ask,
            emit: emit_tx,
            cancel_rx,
            pending_approvals: Arc::clone(&pending),
            session_allow_rules: Arc::new(RwLock::new(HashSet::new())),
        };
        let req = DriverTurnRequest {
            turn_id: "t_cl_2".into(),
            turn_idx: 1,
            prompt: "edit".into(),
            cwd: dir.path().to_path_buf(),
            provider_kind: CodingProviderKind::ClaudeCode,
            permission_mode: CodingPermissionMode::Ask,
            provider_session_id: None,
            model: None,
            executable_override: Some(script.to_string_lossy().into_owned()),
            extra_args: Vec::new(),
            workspace_path_for_event: dir.path().to_string_lossy().into_owned(),
            worktree_path_for_event: None,
            git_branch_for_event: None,
        };
        let fut = ClaudeDriver.run_turn(&req, &mut ctx);
        let responder = tokio::spawn(async move {
            while let Some(ev) = emit_rx.recv().await {
                if let CodingHostEvent::TurnApprovalRequested { request_id, .. } = ev {
                    pending.lock().remove(&request_id).unwrap().send(CodingApprovalDecision::AllowOnce).unwrap();
                    break;
                }
            }
        });
        let (out, _) = tokio::join!(fut, responder);
        assert!(out.ok);
        assert_eq!(out.final_text, "edit=allow");

        // Now test cancel on hung Claude process.
        let (emit_tx2, mut emit_rx2) = mpsc::unbounded_channel();
        let (cancel_tx2, cancel_rx2) = watch::channel(false);
        let mut ctx2 = DriverContext {
            turn_id: "t_cl_3".into(),
            turn_idx: 2,
            permission_mode: CodingPermissionMode::Ask,
            emit: emit_tx2,
            cancel_rx: cancel_rx2,
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            session_allow_rules: Arc::new(RwLock::new(HashSet::new())),
        };
        let req2 = DriverTurnRequest {
            turn_id: "t_cl_3".into(),
            prompt: "hang".into(),
            ..req
        };
        let fut2 = ClaudeDriver.run_turn(&req2, &mut ctx2);
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
        assert_eq!(out2.final_text, "half ");
    }
}
