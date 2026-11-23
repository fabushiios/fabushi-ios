//! Unified coding provider driver abstraction, bounded NDJSON reader, and process lifecycle.

pub mod acp;
pub mod claude;
pub mod codex;

use crate::probe::{default_search_dirs, find_provider_binary_in_dirs, resolve_binary_in_dirs};
use async_trait::async_trait;
use gpui_telegram_proto::{
    CodingApprovalDecision, CodingHostEvent, CodingPermissionMode, CodingProviderKind,
    CodingTokenUsage,
};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

/// Maximum allowed single NDJSON line length (16 MiB) to prevent OOM on malformed CLI output.
pub const MAX_NDJSON_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Grace period after protocol cancel / `SIGINT` before force-killing child process.
pub const GRACEFUL_CANCEL_TIMEOUT: Duration = Duration::from_millis(500);

/// Input specification for a single driver turn execution.
#[derive(Debug, Clone)]
pub struct DriverTurnRequest {
    pub turn_id: String,
    pub turn_idx: u64,
    pub prompt: String,
    pub cwd: PathBuf,
    pub provider_kind: CodingProviderKind,
    pub permission_mode: CodingPermissionMode,
    pub provider_session_id: Option<String>,
    pub model: Option<String>,
    pub executable_override: Option<String>,
    pub extra_args: Vec<String>,
    pub workspace_path_for_event: String,
    pub worktree_path_for_event: Option<String>,
    pub git_branch_for_event: Option<String>,
}

/// Output summary produced by a driver turn execution.
#[derive(Debug, Clone, Default)]
pub struct DriverTurnOutput {
    pub ok: bool,
    pub cancelled: bool,
    pub final_text: String,
    pub error: Option<String>,
    pub provider_session_id: Option<String>,
    pub token_usage: Option<CodingTokenUsage>,
}

/// Shared runtime hooks given to each driver during a turn.
#[derive(Clone)]
pub struct DriverContext {
    pub turn_id: String,
    pub turn_idx: u64,
    pub permission_mode: CodingPermissionMode,
    pub emit: mpsc::UnboundedSender<CodingHostEvent>,
    pub cancel_rx: watch::Receiver<bool>,
    pub pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<CodingApprovalDecision>>>>,
    pub session_allow_rules: Arc<RwLock<HashSet<String>>>,
}

impl DriverContext {
    pub fn is_cancelled(&self) -> bool {
        *self.cancel_rx.borrow()
    }

    pub async fn wait_cancelled(&mut self) {
        if *self.cancel_rx.borrow() {
            return;
        }
        while self.cancel_rx.changed().await.is_ok() {
            if *self.cancel_rx.borrow() {
                return;
            }
        }
    }

    pub fn emit_event(&self, event: CodingHostEvent) {
        let _ = self.emit.send(event);
    }

    pub fn emit_turn_started(&self, req: &DriverTurnRequest, provider_session_id: Option<String>) {
        self.emit_event(CodingHostEvent::TurnStarted {
            turn_id: req.turn_id.clone(),
            provider_session_id,
            workspace_path: req.workspace_path_for_event.clone(),
            worktree_path: req.worktree_path_for_event.clone(),
            git_branch: req.git_branch_for_event.clone(),
            turn_idx: req.turn_idx,
        });
    }

    /// Requests approval for a tool/command/file action, respecting `permission_mode`
    /// and `AllowSession` rules before emitting `TurnApprovalRequested`.
    pub async fn request_approval(
        &self,
        kind: &str,
        title: String,
        detail: String,
        rule_key: String,
    ) -> CodingApprovalDecision {
        if self.is_cancelled() {
            return CodingApprovalDecision::Deny;
        }

        if self.permission_mode == CodingPermissionMode::FullAccess {
            return CodingApprovalDecision::AllowOnce;
        }
        if self.permission_mode == CodingPermissionMode::AutoEdit && is_edit_kind(kind) {
            return CodingApprovalDecision::AllowOnce;
        }

        if self.session_allow_rules.read().contains(&rule_key) {
            return CodingApprovalDecision::AllowSession;
        }

        let request_id = format!("apr_{}", uuid::Uuid::new_v4().simple());
        let (tx, rx) = oneshot::channel();
        self.pending_approvals
            .lock()
            .insert(request_id.clone(), tx);

        self.emit_event(CodingHostEvent::TurnApprovalRequested {
            turn_id: self.turn_id.clone(),
            request_id: request_id.clone(),
            kind: kind.to_string(),
            title,
            detail,
            rule_key: rule_key.clone(),
        });

        let mut cancel_rx = self.cancel_rx.clone();
        let decision = tokio::select! {
            res = rx => match res {
                Ok(d) => d,
                Err(_) => CodingApprovalDecision::Deny,
            },
            _ = async {
                if !*cancel_rx.borrow() {
                    while cancel_rx.changed().await.is_ok() {
                        if *cancel_rx.borrow() {
                            break;
                        }
                    }
                }
            } => CodingApprovalDecision::Deny,
        };

        self.pending_approvals.lock().remove(&request_id);
        if decision == CodingApprovalDecision::AllowSession {
            self.session_allow_rules.write().insert(rule_key);
        }
        decision
    }
}

fn is_edit_kind(kind: &str) -> bool {
    matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "edit" | "write" | "file_change" | "filechange" | "patch" | "apply_patch"
    )
}

#[async_trait]
pub trait CodingProviderDriver: Send + Sync {
    async fn run_turn(
        &self,
        req: &DriverTurnRequest,
        ctx: &mut DriverContext,
    ) -> DriverTurnOutput;
}

/// Dispatches a turn to the concrete driver for `req.provider_kind`.
pub async fn execute_driver_turn(
    req: &DriverTurnRequest,
    ctx: &mut DriverContext,
) -> DriverTurnOutput {
    match req.provider_kind {
        CodingProviderKind::ClaudeCode => claude::ClaudeDriver.run_turn(req, ctx).await,
        CodingProviderKind::Codex => codex::CodexDriver.run_turn(req, ctx).await,
        CodingProviderKind::OpenCode
        | CodingProviderKind::CursorCli
        | CodingProviderKind::Antigravity
        | CodingProviderKind::AcpCustom => acp::AcpDriver.run_turn(req, ctx).await,
    }
}

/// Resolves the executable path for `kind` using `executable_override` or standard search dirs.
pub fn resolve_provider_executable(
    kind: CodingProviderKind,
    executable_override: Option<&str>,
) -> Result<PathBuf, String> {
    let dirs = default_search_dirs();
    if let Some(ov) = executable_override.map(str::trim).filter(|s| !s.is_empty()) {
        return resolve_binary_in_dirs(ov, &dirs).ok_or_else(|| {
            format!(
                "configured executable_override `{ov}` for {} was not found or is not executable",
                kind.display_name()
            )
        });
    }

    if kind == CodingProviderKind::AcpCustom {
        return Err(
            "Custom ACP provider requires `executable_override` to be set".to_string(),
        );
    }

    find_provider_binary_in_dirs(kind, &dirs).ok_or_else(|| {
        format!(
            "{} CLI binary was not found on PATH or standard install directories",
            kind.display_name()
        )
    })
}

/// Reads one newline-delimited line from `reader`, failing cleanly if a single line
/// exceeds `MAX_NDJSON_LINE_BYTES` (16 MiB).
pub async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    read_bounded_line_with_limit(reader, MAX_NDJSON_LINE_BYTES).await
}

pub async fn read_bounded_line_with_limit<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }

        if let Some(nl_pos) = chunk.iter().position(|&b| b == b'\n') {
            if buf.len().saturating_add(nl_pos) > max_bytes {
                reader.consume(nl_pos + 1);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("NDJSON line exceeded maximum cap of {max_bytes} bytes"),
                ));
            }
            buf.extend_from_slice(&chunk[..nl_pos]);
            reader.consume(nl_pos + 1);
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }

        let chunk_len = chunk.len();
        if buf.len().saturating_add(chunk_len) > max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("NDJSON line exceeded maximum cap of {max_bytes} bytes"),
            ));
        }
        buf.extend_from_slice(chunk);
        reader.consume(chunk_len);
    }
}

/// Writes one JSON object as a single NDJSON line and flushes `stdin`.
pub async fn write_ndjson_line(
    stdin: &mut ChildStdin,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let mut line =
        serde_json::to_string(payload).map_err(|e| format!("failed to serialize JSON: {e}"))?;
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("failed to write to CLI stdin: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("failed to flush CLI stdin: {e}"))?;
    Ok(())
}

/// Lifecycle wrapper for a spawned coding CLI child process with automatic cleanup on drop
/// and bounded stderr tail capture.
pub struct ManagedChild {
    child: Option<Child>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    stderr_task: Option<JoinHandle<()>>,
}

impl ManagedChild {
    pub fn spawn(
        mut cmd: Command,
    ) -> Result<(Self, ChildStdin, BufReader<ChildStdout>), String> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn coding CLI process: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "child process missing piped stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "child process missing piped stdout".to_string())?;
        let stderr = child.stderr.take();

        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(64)));
        let tail_clone = Arc::clone(&stderr_tail);
        let stderr_task = stderr.map(|err_stream| {
            tokio::spawn(async move {
                let mut reader = BufReader::new(err_stream);
                while let Ok(Some(line)) = read_bounded_line_with_limit(&mut reader, 64 * 1024).await
                {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    tracing::debug!(target: "gpui_telegram_coding::cli_stderr", "{trimmed}");
                    let mut guard = tail_clone.lock();
                    if guard.len() >= 40 {
                        guard.pop_front();
                    }
                    guard.push_back(trimmed.to_string());
                }
            })
        });

        Ok((
            Self {
                child: Some(child),
                stderr_tail,
                stderr_task,
            },
            stdin,
            BufReader::new(stdout),
        ))
    }

    pub fn stderr_summary(&self) -> String {
        let guard = self.stderr_tail.lock();
        guard.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    pub async fn try_wait_exit(&mut self) -> Option<std::process::ExitStatus> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => {
                self.child = None;
                Some(status)
            }
            _ => None,
        }
    }

    /// Waits for natural exit up to `timeout` after EOF, killing if still alive.
    pub async fn wait_or_kill(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let Some(mut child) = self.child.take() else {
            return None;
        };
        let status = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(s)) => Some(s),
            Ok(Err(_)) => None,
            Err(_) => {
                let _ = child.kill().await;
                child.wait().await.ok()
            }
        };
        if let Some(task) = self.stderr_task.take() {
            let _ = tokio::time::timeout(Duration::from_millis(100), task).await;
        }
        status
    }

    /// Sends `SIGINT` (on Unix), waits `grace`, and force-kills if still running.
    pub async fn graceful_terminate(&mut self, grace: Duration) {
        let Some(mut child) = self.child.take() else {
            return;
        };

        #[cfg(unix)]
        if let Some(pid) = child.id() {
            let _ = Command::new("kill")
                .args(["-INT", &format!("-{pid}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
        #[cfg(not(unix))]
        {
            let _ = child.start_kill();
        }

        if tokio::time::timeout(grace, child.wait()).await.is_err() {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                let _ = Command::new("kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
    }

    /// Sends `SIGINT` non-destructively without consuming `self.child`.
    pub async fn signal_interrupt(&self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.as_ref().and_then(|c| c.id()) {
            let _ = Command::new("kill")
                .args(["-INT", &format!("-{pid}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                let _ = std::process::Command::new("kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
            let _ = child.start_kill();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
    }
}

/// Truncates tool output previews safely on UTF-8 boundaries.
pub(crate) fn preview_str(raw: &str, max_chars: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let mut s: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

pub(crate) fn value_to_preview(val: &serde_json::Value, max_chars: usize) -> Option<String> {
    if val.is_null() {
        return None;
    }
    if let Some(s) = val.as_str() {
        let t = s.trim();
        return (!t.is_empty()).then(|| preview_str(t, max_chars));
    }
    if let Some(arr) = val.as_array() {
        let joined: Vec<String> = arr
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("content").and_then(|c| c.get("text")).and_then(|v| v.as_str()))
                    .or_else(|| item.as_str())
                    .map(str::to_string)
            })
            .collect();
        if !joined.is_empty() {
            return Some(preview_str(&joined.join("\n"), max_chars));
        }
    }
    Some(preview_str(&val.to_string(), max_chars))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn bounded_ndjson_reader_enforces_line_limit() {
        let data = b"{\"ok\":true}\n{\"big\":\"12345678901234567890\"}\n";
        let mut reader = BufReader::new(&data[..]);

        let first = read_bounded_line_with_limit(&mut reader, 24).await.unwrap();
        assert_eq!(first.as_deref(), Some("{\"ok\":true}"));

        let err = read_bounded_line_with_limit(&mut reader, 24)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
