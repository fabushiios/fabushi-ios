pub mod client_tty;
pub mod pty_os;

use self::pty_os::SpawnedPty;
use crate::error::{EngineResult, RemoteEngineError};
use gpui_telegram_proto::remote_control_contracts::{
    ClipboardPayloadDto, LocalIpcRequest, LocalIpcResponse, TerminalExecResultDto,
    TerminalSessionDto,
};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::broadcast;

pub const FABUSHI_IN_MUX_ENV: &str = "FABUSHI_IN_MUX";
pub const DEFAULT_SCROLLBACK_LINES: usize = 50_000;
const STREAM_RING_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Returns true when the current process is already running inside a Fabushi PTY mux session.
pub fn is_inside_mux() -> bool {
    std::env::var(FABUSHI_IN_MUX_ENV).as_deref() == Ok("1")
}

/// Detects OSC 52 clipboard escape sequences (`ESC ] 52 ; <sel> ; <b64> BEL|ST`) in PTY output.
pub fn detect_osc52_sequences(chunk: &[u8]) -> Vec<ClipboardPayloadDto> {
    let mut scanner = Osc52StreamScanner::default();
    scanner.feed(chunk)
}

/// Stateful streaming OSC 52 detector supporting escape sequences split across PTY read chunks.
#[derive(Debug, Default)]
pub struct Osc52StreamScanner {
    pending: Vec<u8>,
}

impl Osc52StreamScanner {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<ClipboardPayloadDto> {
        self.pending.extend_from_slice(chunk);
        if self.pending.len() > 256 * 1024 {
            let keep = self.pending.len() - 64 * 1024;
            self.pending.drain(..keep);
        }
        let mut results = Vec::new();
        let mut consumed_until = 0usize;
        let buf = &self.pending;
        let mut i = 0usize;
        while i < buf.len() {
            if buf[i] == 0x1b && buf.get(i + 1) == Some(&b']') {
                if buf.get(i + 2..i + 5) == Some(b"52;") {
                    let start = i + 5;
                    let mut end = None;
                    let mut next_idx = start;
                    let mut j = start;
                    while j < buf.len() {
                        if buf[j] == 0x07 {
                            end = Some(j);
                            next_idx = j + 1;
                            break;
                        }
                        if buf[j] == 0x1b && buf.get(j + 1) == Some(&b'\\') {
                            end = Some(j);
                            next_idx = j + 2;
                            break;
                        }
                        j += 1;
                    }
                    if let Some(e) = end {
                        if let Ok(payload_str) = std::str::from_utf8(&buf[start..e]) {
                            if let Some((_sel, b64)) = payload_str.split_once(';') {
                                use base64::Engine as _;
                                if let Ok(decoded) =
                                    base64::engine::general_purpose::STANDARD.decode(b64.trim())
                                {
                                    if let Ok(text) = String::from_utf8(decoded) {
                                        results.push(ClipboardPayloadDto {
                                            mime_type: "text/plain".to_string(),
                                            text: Some(text),
                                            image_b64: None,
                                            origin_id: "pty-osc52".to_string(),
                                            seq: results.len() as u64 + 1,
                                        });
                                    }
                                }
                            }
                        }
                        consumed_until = next_idx;
                        i = next_idx;
                        continue;
                    } else {
                        // Incomplete OSC 52 sequence; keep suffix from `i` for next chunk.
                        consumed_until = i;
                        break;
                    }
                }
            }
            consumed_until = i + 1;
            i += 1;
        }
        if consumed_until > 0 {
            self.pending.drain(..consumed_until);
        }
        results
    }
}

#[derive(Debug, Clone)]
pub struct PtyAttachment {
    pub session: TerminalSessionDto,
    pub client_id: String,
    pub scrollback_snapshot: String,
}

#[derive(Debug, Clone)]
struct AttachedClientState {
    is_remote: bool,
    ignore_size: bool,
    cols: Option<u16>,
    rows: Option<u16>,
}

pub struct PtySessionHandle {
    pub info: TerminalSessionDto,
    pub scrollback: VecDeque<String>,
    partial_line: String,
    stream_bytes: VecDeque<u8>,
    stream_base_offset: u64,
    total_bytes_emitted: u64,
    attached: HashMap<String, AttachedClientState>,
    pty: Option<SpawnedPty>,
    output_tx: broadcast::Sender<Vec<u8>>,
    osc52_scanner: Osc52StreamScanner,
    last_osc52: Option<ClipboardPayloadDto>,
    exit_code: Option<i32>,
}

impl PtySessionHandle {
    fn push_output(&mut self, bytes: &[u8]) -> Vec<ClipboardPayloadDto> {
        self.info.last_activity_ms = chrono::Utc::now().timestamp_millis();
        for &b in bytes {
            self.stream_bytes.push_back(b);
        }
        self.total_bytes_emitted = self.total_bytes_emitted.saturating_add(bytes.len() as u64);
        while self.stream_bytes.len() > STREAM_RING_MAX_BYTES {
            self.stream_bytes.pop_front();
            self.stream_base_offset = self.stream_base_offset.saturating_add(1);
        }

        let text = String::from_utf8_lossy(bytes);
        for ch in text.chars() {
            self.partial_line.push(ch);
            if ch == '\n' {
                let line = std::mem::take(&mut self.partial_line);
                if self.scrollback.len() >= DEFAULT_SCROLLBACK_LINES {
                    self.scrollback.pop_front();
                }
                self.scrollback.push_back(line);
            }
        }
        if self.partial_line.len() > 64 * 1024 {
            let line = std::mem::take(&mut self.partial_line);
            if self.scrollback.len() >= DEFAULT_SCROLLBACK_LINES {
                self.scrollback.pop_front();
            }
            self.scrollback.push_back(line);
        }

        let _ = self.output_tx.send(bytes.to_vec());
        let oscs = self.osc52_scanner.feed(bytes);
        if let Some(last) = oscs.last() {
            self.last_osc52 = Some(last.clone());
        }
        oscs
    }

    fn effective_size(&self) -> Option<(u16, u16)> {
        if let Some((_, c)) = self
            .attached
            .iter()
            .find(|(_, st)| st.is_remote && !st.ignore_size && st.cols.is_some() && st.rows.is_some())
        {
            return Some((c.cols.unwrap(), c.rows.unwrap()));
        }
        if let Some((_, c)) = self
            .attached
            .iter()
            .find(|(_, st)| !st.ignore_size && st.cols.is_some() && st.rows.is_some())
        {
            return Some((c.cols.unwrap(), c.rows.unwrap()));
        }
        None
    }

    fn read_since(&self, since_offset: u64) -> (u64, Vec<u8>, bool) {
        let start = since_offset.max(self.stream_base_offset);
        let skip = (start - self.stream_base_offset) as usize;
        let data: Vec<u8> = self.stream_bytes.iter().skip(skip).copied().collect();
        (self.total_bytes_emitted, data, self.info.alive)
    }
}

/// Persistent in-process PTY multiplexer (`destroy_unattached = false`, 50,000-line scrollback).
pub struct PtyMuxServer {
    sessions: RwLock<HashMap<String, Arc<RwLock<PtySessionHandle>>>>,
    osc52_tx: broadcast::Sender<(String, ClipboardPayloadDto)>,
    exec_lock: tokio::sync::Mutex<()>,
}

impl Default for PtyMuxServer {
    fn default() -> Self {
        Self::new()
    }
}

fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "/bin/sh".to_string())
    }
}

impl PtyMuxServer {
    pub fn new() -> Self {
        let (osc52_tx, _) = broadcast::channel(64);
        Self {
            sessions: RwLock::new(HashMap::new()),
            osc52_tx,
            exec_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn subscribe_osc52(&self) -> broadcast::Receiver<(String, ClipboardPayloadDto)> {
        self.osc52_tx.subscribe()
    }

    pub fn list_sessions(&self) -> Vec<TerminalSessionDto> {
        let guard = self.sessions.read();
        let mut out: Vec<_> = guard.values().map(|h| h.read().info.clone()).collect();
        out.sort_by_key(|s| s.created_at_ms);
        out
    }

    pub fn has_session(&self, name_or_id: &str) -> bool {
        self.get_session(name_or_id).is_some()
    }

    pub fn get_session(&self, name_or_id: &str) -> Option<TerminalSessionDto> {
        self.find_handle(name_or_id).map(|h| h.read().info.clone())
    }

    fn find_handle(&self, name_or_id: &str) -> Option<Arc<RwLock<PtySessionHandle>>> {
        let guard = self.sessions.read();
        guard
            .values()
            .find(|h| {
                let info = &h.read().info;
                info.name == name_or_id || info.session_id == name_or_id
            })
            .cloned()
    }

    pub fn new_session(
        &self,
        name: Option<&str>,
        shell: Option<&str>,
        cols: u16,
        rows: u16,
        cwd: Option<&str>,
    ) -> EngineResult<TerminalSessionDto> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let session_name = name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("term-{}", &session_id[..6]));
        if self.has_session(&session_name) {
            return Err(RemoteEngineError::InvalidArgument(format!(
                "terminal session '{session_name}' already exists"
            )));
        }

        let resolved_shell = shell
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(default_shell);
        let cols = cols.max(20);
        let rows = rows.max(5);
        let now = chrono::Utc::now().timestamp_millis();
        let dto = TerminalSessionDto {
            session_id: session_id.clone(),
            name: session_name.clone(),
            shell: resolved_shell.clone(),
            cols,
            rows,
            attached_clients: 0,
            created_at_ms: now,
            last_activity_ms: now,
            alive: true,
            cwd: cwd.map(ToString::to_string),
        };
        let (output_tx, _) = broadcast::channel(512);
        let handle = Arc::new(RwLock::new(PtySessionHandle {
            info: dto.clone(),
            scrollback: VecDeque::new(),
            partial_line: String::new(),
            stream_bytes: VecDeque::new(),
            stream_base_offset: 0,
            total_bytes_emitted: 0,
            attached: HashMap::new(),
            pty: None,
            output_tx,
            osc52_scanner: Osc52StreamScanner::default(),
            last_osc52: None,
            exit_code: None,
        }));

        let h_out = Arc::clone(&handle);
        let osc_tx = self.osc52_tx.clone();
        let sid_for_osc = session_id.clone();
        let h_exit = Arc::clone(&handle);
        let pty = SpawnedPty::spawn(
            &resolved_shell,
            cols,
            rows,
            cwd,
            Arc::new(move |bytes| {
                let oscs = h_out.write().push_output(bytes);
                for payload in oscs {
                    let _ = osc_tx.send((sid_for_osc.clone(), payload));
                }
            }),
            Arc::new(move |code| {
                let mut h = h_exit.write();
                h.info.alive = false;
                h.exit_code = code;
            }),
        )?;

        handle.write().pty = Some(pty);
        self.sessions.write().insert(session_id, handle);
        Ok(dto)
    }

    pub fn attach_session(
        &self,
        name_or_id: &str,
        client_id: &str,
        is_remote: bool,
        ignore_size: bool,
    ) -> EngineResult<PtyAttachment> {
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        let mut h = handle.write();
        h.attached.insert(
            client_id.to_string(),
            AttachedClientState {
                is_remote,
                ignore_size,
                cols: None,
                rows: None,
            },
        );
        h.info.attached_clients = h.attached.len() as u32;
        let mut snap: String = h.scrollback.iter().cloned().collect();
        snap.push_str(&h.partial_line);
        Ok(PtyAttachment {
            session: h.info.clone(),
            client_id: client_id.to_string(),
            scrollback_snapshot: snap,
        })
    }

    pub fn detach_client(&self, name_or_id: &str, client_id: &str) -> EngineResult<()> {
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        let mut h = handle.write();
        h.attached.remove(client_id);
        h.info.attached_clients = h.attached.len() as u32;
        if let Some((cols, rows)) = h.effective_size() {
            h.info.cols = cols;
            h.info.rows = rows;
            if let Some(ref pty) = h.pty {
                let _ = pty.resize(cols, rows);
            }
        }
        // Note: destroy_unattached = false -> PTY stays alive when attached_clients reaches 0!
        Ok(())
    }

    pub fn kill_session(&self, name_or_id: &str) -> EngineResult<bool> {
        let mut guard = self.sessions.write();
        let key = guard
            .iter()
            .find(|(_, h)| {
                let info = &h.read().info;
                info.name == name_or_id || info.session_id == name_or_id
            })
            .map(|(k, _)| k.clone());
        let Some(k) = key else {
            return Ok(false);
        };
        if let Some(handle) = guard.remove(&k) {
            let mut h = handle.write();
            h.info.alive = false;
            if let Some(pty) = h.pty.take() {
                let _ = pty.kill();
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn rename_session(&self, old_name: &str, new_name: &str) -> EngineResult<TerminalSessionDto> {
        let new_trimmed = new_name.trim();
        if new_trimmed.is_empty() {
            return Err(RemoteEngineError::InvalidArgument(
                "new session name cannot be empty".into(),
            ));
        }
        if self.has_session(new_trimmed) {
            return Err(RemoteEngineError::InvalidArgument(format!(
                "session '{new_trimmed}' already exists"
            )));
        }
        let handle = self
            .find_handle(old_name)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{old_name}' not found")))?;
        let mut h = handle.write();
        h.info.name = new_trimmed.to_string();
        Ok(h.info.clone())
    }

    pub fn resize_session(
        &self,
        name_or_id: &str,
        cols: u16,
        rows: u16,
        is_remote: bool,
        ignore_size: bool,
    ) -> EngineResult<()> {
        self.resize_client(name_or_id, "__default__", cols, rows, is_remote, ignore_size)
    }

    pub fn resize_client(
        &self,
        name_or_id: &str,
        client_id: &str,
        cols: u16,
        rows: u16,
        is_remote: bool,
        ignore_size: bool,
    ) -> EngineResult<()> {
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        let cols = cols.max(10);
        let rows = rows.max(4);
        let mut h = handle.write();
        let entry = h
            .attached
            .entry(client_id.to_string())
            .or_insert(AttachedClientState {
                is_remote,
                ignore_size,
                cols: None,
                rows: None,
            });
        entry.is_remote = is_remote;
        entry.ignore_size = ignore_size;
        entry.cols = Some(cols);
        entry.rows = Some(rows);
        h.info.attached_clients = h.attached.len() as u32;

        let has_active_remote = h
            .attached
            .values()
            .any(|st| st.is_remote && !st.ignore_size);
        let should_apply = !ignore_size && (is_remote || !has_active_remote);
        if should_apply {
            h.info.cols = cols;
            h.info.rows = rows;
            if let Some(ref pty) = h.pty {
                pty.resize(cols, rows)?;
            }
        }
        Ok(())
    }

    pub fn write_input(&self, name_or_id: &str, data: &[u8]) -> EngineResult<()> {
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        let h = handle.read();
        if !h.info.alive {
            return Err(RemoteEngineError::Other(format!(
                "session '{name_or_id}' has exited"
            )));
        }
        let pty = h
            .pty
            .as_ref()
            .ok_or_else(|| RemoteEngineError::Other("PTY handle missing".into()))?;
        pty.write_all(data)
    }

    pub fn read_scrollback(&self, name_or_id: &str, max_lines: Option<usize>) -> EngineResult<String> {
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        let h = handle.read();
        let take = max_lines.unwrap_or(h.scrollback.len() + usize::from(!h.partial_line.is_empty()));
        let mut lines: Vec<String> = h.scrollback.iter().cloned().collect();
        if !h.partial_line.is_empty() {
            lines.push(h.partial_line.clone());
        }
        let skip = lines.len().saturating_sub(take);
        Ok(lines.into_iter().skip(skip).collect())
    }

    pub fn read_output_since(
        &self,
        name_or_id: &str,
        since_offset: u64,
    ) -> EngineResult<(u64, Vec<u8>, bool)> {
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        let res = handle.read().read_since(since_offset);
        Ok(res)
    }

    pub fn latest_osc52(&self, name_or_id: &str) -> Option<ClipboardPayloadDto> {
        self.find_handle(name_or_id)
            .and_then(|h| h.read().last_osc52.clone())
    }

    pub fn subscribe_output(&self, name_or_id: &str) -> EngineResult<broadcast::Receiver<Vec<u8>>> {
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        Ok(handle.read().output_tx.subscribe())
    }

    pub async fn exec_command(
        &self,
        name_or_id: &str,
        command: &str,
        timeout_ms: u64,
    ) -> EngineResult<TerminalExecResultDto> {
        let _exec_guard = self.exec_lock.lock().await;
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        let (session_id, shell, mut rx) = {
            let h = handle.read();
            if !h.info.alive {
                return Err(RemoteEngineError::Other(format!(
                    "terminal session '{name_or_id}' is not alive"
                )));
            }
            (
                h.info.session_id.clone(),
                h.info.shell.clone(),
                h.output_tx.subscribe(),
            )
        };

        let full_token = uuid::Uuid::new_v4().simple().to_string();
        let token = &full_token[..16];
        let sentinel_prefix = format!("__FABUSHI_EXEC_{token}__:");
        const SENTINEL_SUFFIX: &str = "__FB_END__";
        let script = build_exec_script(&shell, cfg!(windows), command, token);

        self.write_input(name_or_id, script.as_bytes())?;

        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_millis(timeout_ms.max(100));
        let mut collected = String::new();
        let mut timed_out = false;
        let mut exit_code = -1i32;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                timed_out = true;
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(chunk)) => {
                    collected.push_str(&String::from_utf8_lossy(&chunk));
                    let normalized = normalize_pty_output(&collected);
                    if let Some(pos) = normalized.find(&sentinel_prefix) {
                        let after = &normalized[pos + sentinel_prefix.len()..];
                        if let Some(end_pos) =
                            after.find(SENTINEL_SUFFIX).or_else(|| after.find('\n'))
                        {
                            let code_str = after[..end_pos].trim();
                            exit_code = code_str.parse::<i32>().unwrap_or(0);
                            let before = &normalized[..pos];
                            let cleaned = strip_exec_wrapper_echo(before, command, token);
                            return Ok(TerminalExecResultDto {
                                session_id,
                                exit_code,
                                output: cleaned,
                                timed_out: false,
                            });
                        }
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Err(_) => {
                    timed_out = true;
                    break;
                }
            }
        }

        let normalized = normalize_pty_output(&collected);
        let cleaned = strip_exec_wrapper_echo(&normalized, command, token);
        Ok(TerminalExecResultDto {
            session_id,
            exit_code,
            output: cleaned,
            timed_out,
        })
    }

    pub fn attached_client_ids(&self, name_or_id: &str) -> EngineResult<HashSet<String>> {
        let handle = self
            .find_handle(name_or_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("session '{name_or_id}' not found")))?;
        Ok(handle.read().attached.keys().cloned().collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecShellKind {
    PosixSh,
    WindowsCmd,
    WindowsPowerShell,
}

fn detect_exec_shell_kind(shell: &str, is_windows: bool) -> ExecShellKind {
    let first = shell.split_whitespace().next().unwrap_or("");
    let cleaned = first.trim_matches(|c| c == '"' || c == '\'');
    let base = cleaned
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let stem = base.strip_suffix(".exe").unwrap_or(&base);
    match stem {
        "powershell" | "pwsh" => ExecShellKind::WindowsPowerShell,
        "cmd" => ExecShellKind::WindowsCmd,
        "sh" | "bash" | "zsh" | "dash" | "fish" | "ash" => ExecShellKind::PosixSh,
        _ if is_windows => ExecShellKind::WindowsCmd,
        _ => ExecShellKind::PosixSh,
    }
}

fn build_exec_script(shell: &str, is_windows: bool, command: &str, token: &str) -> String {
    match detect_exec_shell_kind(shell, is_windows) {
        ExecShellKind::WindowsCmd => {
            let normalized_cmd = command.replace("\r\n", "\r").replace('\n', "\r");
            // Reset stale ERRORLEVEL with `ver >nul` and split sentinel with `^` so PTY echo never matches sentinel_prefix.
            format!(
                "ver >nul\r{normalized_cmd}\recho __FABUSHI_^EXEC_{token}__:%ERRORLEVEL%__FB_END__\r"
            )
        }
        ExecShellKind::WindowsPowerShell => {
            let normalized_cmd = command.replace("\r\n", "\r").replace('\n', "\r");
            format!(
                "$global:LASTEXITCODE = 0\r{normalized_cmd}\r$__fb_ok = $?; $__fb_ec = if ($__fb_ok) {{ if ($LASTEXITCODE) {{ $LASTEXITCODE }} else {{ 0 }} }} else {{ if ($LASTEXITCODE) {{ $LASTEXITCODE }} else {{ 1 }} }}; Write-Output (\"__FABUSHI_\" + \"EXEC_{token}__:\" + $__fb_ec + \"__FB_END__\")\r"
            )
        }
        ExecShellKind::PosixSh => {
            // Split sentinel literal across single-quoted chunks so PTY ECHO never matches sentinel_prefix.
            format!(
                "{command}\n__fb_ec=$?; printf '\\n__FABUSHI_''EXEC_%s__:%d__FB_END__\\n' '{token}' \"$__fb_ec\"\n"
            )
        }
    }
}

/// Normalizes VT100/ANSI screen-diff output (such as Windows ConPTY CSI cursor positioning and
/// cursor-forward spaces) into plain UTF-8 text with standard `\n` line breaks.
pub fn normalize_pty_output(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0usize;
    let mut cur_row: u32 = 1;
    let mut cur_col: u32 = 1;
    let mut has_output = false;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            0x00 => {
                i += 1;
            }
            b'\r' => {
                cur_col = 1;
                i += 1;
            }
            0x08 => {
                cur_col = cur_col.saturating_sub(1).max(1);
                i += 1;
            }
            b'\n' => {
                out.push('\n');
                cur_row = cur_row.saturating_add(1);
                cur_col = 1;
                i += 1;
            }
            b'\t' => {
                out.push('\t');
                cur_col = ((cur_col.saturating_sub(1)) / 8 + 1) * 8 + 1;
                has_output = true;
                i += 1;
            }
            0x1B => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    // OSC sequence: ESC ] ... (BEL | ST)
                    b']' => {
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1B && bytes.get(i + 1) == Some(&b'\\') {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    // DCS / APC / PM / SOS strings terminated by ST (ESC \)
                    b'P' | b'_' | b'^' | b'X' => {
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x1B && bytes.get(i + 1) == Some(&b'\\') {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    // CSI sequence: ESC [ <params/intermediates> <final 0x40..=0x7E>
                    b'[' => {
                        i += 1;
                        let param_start = i;
                        while i < bytes.len() && (0x20..=0x3F).contains(&bytes[i]) {
                            i += 1;
                        }
                        let param_end = i;
                        if i >= bytes.len() {
                            break;
                        }
                        let final_byte = bytes[i];
                        i += 1;

                        let param_bytes = &bytes[param_start..param_end];
                        let is_standard = param_bytes
                            .iter()
                            .all(|&c| c.is_ascii_digit() || c == b';');
                        if is_standard {
                            let param_str = std::str::from_utf8(param_bytes).unwrap_or("");
                            let mut parts = param_str.split(';').map(|s| {
                                if s.is_empty() {
                                    None
                                } else {
                                    s.parse::<u32>().ok()
                                }
                            });
                            let p0 = parts.next().and_then(|o| o);
                            let p1 = parts.next().and_then(|o| o);

                            match final_byte {
                                b'H' | b'f' => {
                                    let row = p0.unwrap_or(1).max(1);
                                    let col = p1.unwrap_or(1).max(1);
                                    if row != cur_row {
                                        if has_output && !out.ends_with('\n') {
                                            out.push('\n');
                                        }
                                        cur_row = row;
                                        cur_col = col;
                                    } else {
                                        if col > cur_col {
                                            let spaces = (col - cur_col).min(16);
                                            for _ in 0..spaces {
                                                out.push(' ');
                                            }
                                            has_output = true;
                                        }
                                        cur_col = col;
                                    }
                                }
                                b'G' => {
                                    let col = p0.unwrap_or(1).max(1);
                                    if col > cur_col {
                                        let spaces = (col - cur_col).min(16);
                                        for _ in 0..spaces {
                                            out.push(' ');
                                        }
                                        has_output = true;
                                    }
                                    cur_col = col;
                                }
                                b'C' => {
                                    let n = p0.unwrap_or(1).max(1).min(16);
                                    for _ in 0..n {
                                        out.push(' ');
                                    }
                                    cur_col = cur_col.saturating_add(n);
                                    has_output = true;
                                }
                                b'B' | b'E' => {
                                    let n = p0.unwrap_or(1).max(1);
                                    if has_output && !out.ends_with('\n') {
                                        out.push('\n');
                                    }
                                    cur_row = cur_row.saturating_add(n);
                                    if final_byte == b'E' {
                                        cur_col = 1;
                                    }
                                }
                                b'd' => {
                                    let row = p0.unwrap_or(1).max(1);
                                    if row != cur_row && has_output && !out.ends_with('\n') {
                                        out.push('\n');
                                    }
                                    cur_row = row;
                                }
                                b'J' => {
                                    if has_output && !out.ends_with('\n') {
                                        out.push('\n');
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    // Charset selection sequences: ESC ( X / ESC ) X / ESC * X / ESC + X
                    b'(' | b')' | b'*' | b'+' => {
                        i = (i + 2).min(bytes.len());
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            0x01..=0x1F | 0x7F => {
                i += 1;
            }
            _ => {
                if let Some(ch) = raw[i..].chars().next() {
                    out.push(ch);
                    i += ch.len_utf8();
                    cur_col = cur_col.saturating_add(1);
                    has_output = true;
                } else {
                    i += 1;
                }
            }
        }
    }

    out
}

fn strip_exec_wrapper_echo(raw: &str, command: &str, token: &str) -> String {
    let cmd_first_line = command.lines().next().unwrap_or("").trim();
    let lines: Vec<&str> = raw
        .lines()
        .filter(|line| {
            let t = line.trim();
            !t.contains("__fb_ec=")
                && !t.contains("$__fb_ok")
                && !t.contains("$__fb_ec")
                && !t.contains("$global:LASTEXITCODE = 0")
                && !t.contains("if ($LASTEXITCODE) { $LASTEXITCODE }")
                && !t.contains("__FABUSHI_")
                && !t.contains("__FB_END__")
                && (token.is_empty() || !t.contains(token))
                && !t.contains("ver >nul")
                && (cmd_first_line.is_empty()
                    || (t != cmd_first_line
                        && !t.ends_with(&format!(">{cmd_first_line}"))
                        && !t.ends_with(&format!("> {cmd_first_line}"))))
        })
        .collect();
    lines.join("\n").trim().to_string()
}

/// Dispatches `lterm.*` / `term.*` JSON-RPC IPC calls against a shared `PtyMuxServer`.
pub async fn handle_terminal_ipc_method(
    mux: &PtyMuxServer,
    req: &LocalIpcRequest,
) -> Option<LocalIpcResponse> {
    let p = &req.params;
    match req.method.as_str() {
        "lterm.ls" | "term.ls" => {
            let sessions = mux.list_sessions();
            Some(LocalIpcResponse::ok(
                req.id.clone(),
                serde_json::json!({ "sessions": sessions }),
            ))
        }
        "lterm.has" | "term.has" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let exists = mux.has_session(name);
            Some(LocalIpcResponse::ok(
                req.id.clone(),
                serde_json::json!({ "exists": exists, "name": name }),
            ))
        }
        "lterm.new" | "term.new" => {
            let name = p.get("name").and_then(|v| v.as_str());
            let shell = p.get("shell").and_then(|v| v.as_str());
            let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(120) as u16;
            let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(36) as u16;
            let cwd = p.get("cwd").and_then(|v| v.as_str());
            match mux.new_session(name, shell, cols, rows, cwd) {
                Ok(dto) => Some(LocalIpcResponse::ok(
                    req.id.clone(),
                    serde_json::to_value(dto).unwrap_or_default(),
                )),
                Err(err) => Some(LocalIpcResponse::err(
                    req.id.clone(),
                    "terminal_new_failed",
                    err.to_string(),
                )),
            }
        }
        "lterm.attach" | "term.attach" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let client_id = p
                .get("client_id")
                .and_then(|v| v.as_str())
                .unwrap_or("cli-tty");
            let is_remote = p.get("is_remote").and_then(|v| v.as_bool()).unwrap_or(false);
            let ignore_size = p.get("ignore_size").and_then(|v| v.as_bool()).unwrap_or(false);
            match mux.attach_session(name, client_id, is_remote, ignore_size) {
                Ok(att) => {
                    let (offset, _, _) = mux
                        .read_output_since(&att.session.session_id, 0)
                        .unwrap_or((0, Vec::new(), true));
                    Some(LocalIpcResponse::ok(
                        req.id.clone(),
                        serde_json::json!({
                            "session": att.session,
                            "client_id": att.client_id,
                            "scrollback": att.scrollback_snapshot,
                            "offset": offset,
                        }),
                    ))
                }
                Err(err) => Some(LocalIpcResponse::err(
                    req.id.clone(),
                    "terminal_attach_failed",
                    err.to_string(),
                )),
            }
        }
        "lterm.detach" | "term.detach" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let client_id = p
                .get("client_id")
                .and_then(|v| v.as_str())
                .unwrap_or("cli-tty");
            match mux.detach_client(name, client_id) {
                Ok(()) => Some(LocalIpcResponse::ok(
                    req.id.clone(),
                    serde_json::json!({ "detached": true }),
                )),
                Err(err) => Some(LocalIpcResponse::err(
                    req.id.clone(),
                    "terminal_detach_failed",
                    err.to_string(),
                )),
            }
        }
        "lterm.kill" | "term.kill" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            match mux.kill_session(name) {
                Ok(killed) => Some(LocalIpcResponse::ok(
                    req.id.clone(),
                    serde_json::json!({ "killed": killed, "name": name }),
                )),
                Err(err) => Some(LocalIpcResponse::err(
                    req.id.clone(),
                    "terminal_kill_failed",
                    err.to_string(),
                )),
            }
        }
        "lterm.rename" | "term.rename" => {
            let old = p.get("old_name").and_then(|v| v.as_str()).unwrap_or("");
            let new = p.get("new_name").and_then(|v| v.as_str()).unwrap_or("");
            match mux.rename_session(old, new) {
                Ok(dto) => Some(LocalIpcResponse::ok(
                    req.id.clone(),
                    serde_json::to_value(dto).unwrap_or_default(),
                )),
                Err(err) => Some(LocalIpcResponse::err(
                    req.id.clone(),
                    "terminal_rename_failed",
                    err.to_string(),
                )),
            }
        }
        "lterm.resize" | "term.resize" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let client_id = p
                .get("client_id")
                .and_then(|v| v.as_str())
                .unwrap_or("cli-tty");
            let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(120) as u16;
            let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(36) as u16;
            let is_remote = p.get("is_remote").and_then(|v| v.as_bool()).unwrap_or(false);
            let ignore_size = p.get("ignore_size").and_then(|v| v.as_bool()).unwrap_or(false);
            match mux.resize_client(name, client_id, cols, rows, is_remote, ignore_size) {
                Ok(()) => Some(LocalIpcResponse::ok(
                    req.id.clone(),
                    serde_json::json!({ "resized": true }),
                )),
                Err(err) => Some(LocalIpcResponse::err(
                    req.id.clone(),
                    "terminal_resize_failed",
                    err.to_string(),
                )),
            }
        }
        "lterm.write" | "term.write" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let bytes = if let Some(b64) = p.get("data_b64").and_then(|v| v.as_str()) {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap_or_default()
            } else {
                p.get("input")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec()
            };
            match mux.write_input(name, &bytes) {
                Ok(()) => Some(LocalIpcResponse::ok(
                    req.id.clone(),
                    serde_json::json!({ "written": bytes.len() }),
                )),
                Err(err) => Some(LocalIpcResponse::err(
                    req.id.clone(),
                    "terminal_write_failed",
                    err.to_string(),
                )),
            }
        }
        "lterm.read" | "term.read" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(since) = p.get("since_offset").and_then(|v| v.as_u64()) {
                match mux.read_output_since(name, since) {
                    Ok((next_offset, bytes, alive)) => {
                        use base64::Engine as _;
                        let data_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        Some(LocalIpcResponse::ok(
                            req.id.clone(),
                            serde_json::json!({
                                "next_offset": next_offset,
                                "data_b64": data_b64,
                                "alive": alive,
                            }),
                        ))
                    }
                    Err(err) => Some(LocalIpcResponse::err(
                        req.id.clone(),
                        "terminal_read_failed",
                        err.to_string(),
                    )),
                }
            } else {
                let max_lines = p
                    .get("max_lines")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                match mux.read_scrollback(name, max_lines) {
                    Ok(output) => Some(LocalIpcResponse::ok(
                        req.id.clone(),
                        serde_json::json!({ "output": output }),
                    )),
                    Err(err) => Some(LocalIpcResponse::err(
                        req.id.clone(),
                        "terminal_read_failed",
                        err.to_string(),
                    )),
                }
            }
        }
        "lterm.exec" | "term.exec" => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let cmd = p.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let timeout_ms = p.get("timeout_ms").and_then(|v| v.as_u64()).unwrap_or(10_000);
            match mux.exec_command(name, cmd, timeout_ms).await {
                Ok(res) => Some(LocalIpcResponse::ok(
                    req.id.clone(),
                    serde_json::to_value(res).unwrap_or_default(),
                )),
                Err(err) => Some(LocalIpcResponse::err(
                    req.id.clone(),
                    "terminal_exec_failed",
                    err.to_string(),
                )),
            }
        }
        _ => None,
    }
}
