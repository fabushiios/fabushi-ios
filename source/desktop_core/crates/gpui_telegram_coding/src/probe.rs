//! Non-invasive local CLI provider probing.
//!
//! Searches `PATH` and standard macOS/Linux/Windows tool install locations for
//! supported coding CLIs and runs `<binary> --version` with a strict 1500ms
//! timeout so probing never hangs or launches interactive browsers/MCP servers.

use gpui_telegram_proto::{CodingProviderKind, DetectedCodingProvider};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hard timeout enforced per `<binary> --version` probe.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// Candidate executable names for each provider kind.
pub fn candidate_binaries(kind: CodingProviderKind) -> &'static [&'static str] {
    match kind {
        CodingProviderKind::ClaudeCode => &["claude"],
        CodingProviderKind::Codex => &["codex"],
        CodingProviderKind::OpenCode => &["opencode"],
        CodingProviderKind::CursorCli => &["cursor-agent", "agent"],
        CodingProviderKind::Antigravity => &["agy"],
        CodingProviderKind::AcpCustom => &[],
    }
}

/// Human-oriented login/configuration hint for each provider kind.
pub fn login_hint_for(kind: CodingProviderKind) -> &'static str {
    match kind {
        CodingProviderKind::ClaudeCode => {
            "Run `claude auth login` (or start `claude` once) to authenticate."
        }
        CodingProviderKind::Codex => {
            "Run `codex login` or configure `OPENAI_API_KEY` in your shell environment."
        }
        CodingProviderKind::OpenCode => {
            "Run `opencode auth login` or configure your provider API key."
        }
        CodingProviderKind::CursorCli => {
            "Run `cursor-agent login` or set `CURSOR_API_KEY`."
        }
        CodingProviderKind::Antigravity => {
            "Sign in with the Antigravity desktop app or `agy` CLI."
        }
        CodingProviderKind::AcpCustom => {
            "Configure `executable_override` (and optional `extra_args`) pointing to an ACP-compatible CLI."
        }
    }
}

/// Collects deduplicated directories to search for coding CLI executables.
pub fn default_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    let mut push_dir = |p: PathBuf| {
        if !p.as_os_str().is_empty() && seen.insert(p.clone()) {
            dirs.push(p);
        }
    };

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            push_dir(dir);
        }
    }

    for fixed in ["/opt/homebrew/bin", "/usr/local/bin"] {
        push_dir(PathBuf::from(fixed));
    }

    if let Some(home) = home_dir() {
        for rel in [
            ".local/bin",
            ".npm-global/bin",
            ".bun/bin",
            ".cargo/bin",
        ] {
            push_dir(home.join(rel));
        }
    }

    if let Some(appdata) = std::env::var_os("APPDATA") {
        if !appdata.is_empty() {
            push_dir(PathBuf::from(appdata).join("npm"));
        }
    }

    dirs
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
}

/// Resolves an executable path if `bin` is an explicit path or a name in `dirs`.
pub fn resolve_binary_in_dirs(bin: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let trimmed = bin.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = Path::new(trimmed);
    if candidate.is_absolute() || trimmed.contains('/') || trimmed.contains('\\') {
        return is_executable_file(candidate).then(|| candidate.to_path_buf());
    }

    for dir in dirs {
        for name in platform_candidate_names(trimmed) {
            let p = dir.join(&name);
            if is_executable_file(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// Finds the first matching binary for `kind` across `dirs`.
pub fn find_provider_binary_in_dirs(kind: CodingProviderKind, dirs: &[PathBuf]) -> Option<PathBuf> {
    for name in candidate_binaries(kind) {
        if let Some(found) = resolve_binary_in_dirs(name, dirs) {
            return Some(found);
        }
    }
    None
}

/// Finds the first matching binary for `kind` across [`default_search_dirs`].
pub fn find_provider_binary(kind: CodingProviderKind) -> Option<PathBuf> {
    find_provider_binary_in_dirs(kind, &default_search_dirs())
}

fn platform_candidate_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".cmd") || lower.ends_with(".bat") {
            vec![name.to_string()]
        } else {
            vec![
                format!("{name}.exe"),
                format!("{name}.cmd"),
                format!("{name}.bat"),
                name.to_string(),
            ]
        }
    }
    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Result of running `<binary> --version` under a hard timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryVersionProbe {
    pub exited_ok: bool,
    pub timed_out: bool,
    pub version: Option<String>,
    pub detail: Option<String>,
}

/// Runs `<binary> --version` synchronously with a strict `timeout` hard kill.
pub fn probe_binary_version_sync(binary: &Path, timeout: Duration) -> BinaryVersionProbe {
    let mut cmd = Command::new(binary);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("CI", "1")
        .env("TERM", "dumb");

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            return BinaryVersionProbe {
                exited_ok: false,
                timed_out: false,
                version: None,
                detail: Some(err.to_string()),
            };
        }
    };

    let pid = child.id();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_handle = std::thread::spawn(move || read_capped_pipe(stdout_pipe, 16 * 1024));
    let stderr_handle = std::thread::spawn(move || read_capped_pipe(stderr_pipe, 16 * 1024));

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let mut exited_ok = false;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exited_ok = status.success();
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    kill_process_group_or_child(&mut child, pid);
                    let _ = child.wait();
                    break;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            Err(err) => {
                kill_process_group_or_child(&mut child, pid);
                let _ = child.wait();
                return BinaryVersionProbe {
                    exited_ok: false,
                    timed_out: false,
                    version: None,
                    detail: Some(err.to_string()),
                };
            }
        }
    }

    if timed_out {
        return BinaryVersionProbe {
            exited_ok: false,
            timed_out: true,
            version: None,
            detail: Some(format!("`--version` timed out after {}ms", timeout.as_millis())),
        };
    }

    let stdout_text = stdout_handle.join().unwrap_or_default();
    let stderr_text = stderr_handle.join().unwrap_or_default();
    let version = parse_version_line(&stdout_text).or_else(|| parse_version_line(&stderr_text));
    let detail = if !exited_ok {
        parse_version_line(&stderr_text).or_else(|| parse_version_line(&stdout_text))
    } else {
        None
    };

    BinaryVersionProbe {
        exited_ok,
        timed_out,
        version,
        detail,
    }
}

fn kill_process_group_or_child(child: &mut std::process::Child, pid: u32) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(unix))]
    let _ = pid;
    let _ = child.kill();
}

fn read_capped_pipe<R: std::io::Read>(pipe: Option<R>, max_bytes: usize) -> String {
    let Some(mut reader) = pipe else {
        return String::new();
    };
    let mut out = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let remaining = max_bytes.saturating_sub(out.len());
                if remaining == 0 {
                    break;
                }
                let take = n.min(remaining);
                out.extend_from_slice(&buf[..take]);
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_version_line(raw: &str) -> Option<String> {
    raw.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| {
            let max = 128;
            if line.chars().count() > max {
                line.chars().take(max).collect()
            } else {
                line.to_string()
            }
        })
}

fn probe_provider_in_dirs(kind: CodingProviderKind, dirs: &[PathBuf]) -> DetectedCodingProvider {
    if kind == CodingProviderKind::AcpCustom {
        return DetectedCodingProvider {
            kind,
            display_name: kind.display_name().to_string(),
            binary_path: None,
            version: None,
            installed: true,
            recommended_username: kind.recommended_username().to_string(),
            supports_acp: kind.supports_acp(),
            login_hint: login_hint_for(kind).to_string(),
        };
    }

    let Some(bin_path) = find_provider_binary_in_dirs(kind, dirs) else {
        return DetectedCodingProvider {
            kind,
            display_name: kind.display_name().to_string(),
            binary_path: None,
            version: None,
            installed: false,
            recommended_username: kind.recommended_username().to_string(),
            supports_acp: kind.supports_acp(),
            login_hint: login_hint_for(kind).to_string(),
        };
    };

    let probe = probe_binary_version_sync(&bin_path, PROBE_TIMEOUT);
    DetectedCodingProvider {
        kind,
        display_name: kind.display_name().to_string(),
        binary_path: Some(bin_path.display().to_string()),
        version: probe.version,
        installed: probe.exited_ok && !probe.timed_out,
        recommended_username: kind.recommended_username().to_string(),
        supports_acp: kind.supports_acp(),
        login_hint: login_hint_for(kind).to_string(),
    }
}

/// Probes all supported coding providers deterministically using ONLY `dirs`.
///
/// Runs provider checks concurrently across scoped threads and enforces the
/// 1500ms hard timeout per binary.
pub fn probe_in_dirs(dirs: &[PathBuf]) -> Vec<DetectedCodingProvider> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = CodingProviderKind::ALL
            .iter()
            .copied()
            .map(|kind| scope.spawn(move || probe_provider_in_dirs(kind, dirs)))
            .collect();

        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect()
    })
}

/// Synchronously probes local coding CLI providers across `PATH` and standard tool directories.
pub fn probe_local_coding_providers_sync() -> Vec<DetectedCodingProvider> {
    let dirs = default_search_dirs();
    probe_in_dirs(&dirs)
}

/// Asynchronously probes local coding CLI providers without blocking the Tokio executor.
pub async fn probe_local_coding_providers() -> Vec<DetectedCodingProvider> {
    tokio::task::spawn_blocking(probe_local_coding_providers_sync)
        .await
        .unwrap_or_default()
}

/// Passive non-invasive login/readiness check for `CheckLogin` control action.
pub fn check_provider_login_status(
    kind: CodingProviderKind,
    executable_override: Option<&str>,
) -> (bool, String) {
    let dirs = default_search_dirs();
    let binary = if let Some(ov) = executable_override.map(str::trim).filter(|s| !s.is_empty()) {
        resolve_binary_in_dirs(ov, &dirs)
    } else if kind == CodingProviderKind::AcpCustom {
        None
    } else {
        find_provider_binary_in_dirs(kind, &dirs)
    };

    if kind == CodingProviderKind::AcpCustom && executable_override.is_none() {
        return (
            false,
            format!(
                "**Custom ACP** is not configured yet.\n\n- {}",
                login_hint_for(kind)
            ),
        );
    }

    let Some(bin_path) = binary else {
        return (
            false,
            format!(
                "**{}** CLI binary was not found.\n\n- {}",
                kind.display_name(),
                login_hint_for(kind)
            ),
        );
    };

    let version_probe = probe_binary_version_sync(&bin_path, PROBE_TIMEOUT);
    if version_probe.timed_out || !version_probe.exited_ok {
        let reason = version_probe
            .detail
            .unwrap_or_else(|| "`--version` exited with non-zero status".to_string());
        return (
            false,
            format!(
                "**{}** binary at `{}` failed health check: {reason}\n\n- {}",
                kind.display_name(),
                bin_path.display(),
                login_hint_for(kind)
            ),
        );
    }

    let passive_auth = passive_auth_hint(kind);
    let version_str = version_probe
        .version
        .as_deref()
        .unwrap_or("unknown version");
    (
        true,
        format!(
            "**{}** is available (`{}` — `{version_str}`).\n- Auth status: {passive_auth}\n- Hint: {}",
            kind.display_name(),
            bin_path.display(),
            login_hint_for(kind)
        ),
    )
}

fn passive_auth_hint(kind: CodingProviderKind) -> String {
    let home = home_dir();
    match kind {
        CodingProviderKind::ClaudeCode => {
            let has_env = std::env::var_os("ANTHROPIC_API_KEY")
                .is_some_and(|v| !v.is_empty());
            let has_creds = home
                .as_ref()
                .is_some_and(|h| h.join(".claude").join(".credentials.json").exists() || h.join(".claude.json").exists());
            if has_env {
                "ANTHROPIC_API_KEY is set".to_string()
            } else if has_creds {
                "local Claude credentials detected".to_string()
            } else {
                "no local credential file detected (verify via `claude auth status`)".to_string()
            }
        }
        CodingProviderKind::Codex => {
            let has_env = std::env::var_os("OPENAI_API_KEY")
                .is_some_and(|v| !v.is_empty());
            let has_auth = home
                .as_ref()
                .is_some_and(|h| h.join(".codex").join("auth.json").exists());
            if has_env {
                "OPENAI_API_KEY is set".to_string()
            } else if has_auth {
                "local `~/.codex/auth.json` detected".to_string()
            } else {
                "no local `~/.codex/auth.json` detected".to_string()
            }
        }
        CodingProviderKind::OpenCode => "CLI installed; verify provider keys with `opencode auth list`".to_string(),
        CodingProviderKind::CursorCli => {
            if std::env::var_os("CURSOR_API_KEY").is_some_and(|v| !v.is_empty()) {
                "CURSOR_API_KEY is set".to_string()
            } else {
                "CLI installed; run `cursor-agent status` if unauthenticated".to_string()
            }
        }
        CodingProviderKind::Antigravity => "Antigravity CLI detected".to_string(),
        CodingProviderKind::AcpCustom => "Custom ACP executable ready".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[cfg(unix)]
    fn write_exec_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[test]
    #[cfg(unix)]
    fn probe_in_dirs_detects_installed_providers_and_always_includes_acp_custom() {
        let dir = TempDir::new().unwrap();
        write_exec_script(dir.path(), "claude", "#!/bin/sh\necho '1.2.3 (Claude Code)'\n");
        write_exec_script(dir.path(), "agent", "#!/bin/sh\necho 'cursor-agent 0.9.1'\n");
        write_exec_script(dir.path(), "agy", "#!/bin/sh\necho '1.5.9'\n");

        // Non-executable file named codex must NOT count as executable.
        fs::write(dir.path().join("codex"), "#!/bin/sh\necho '0.0.1'\n").unwrap();

        let results = probe_in_dirs(&[dir.path().to_path_buf()]);
        assert_eq!(results.len(), CodingProviderKind::ALL.len());

        let claude = results
            .iter()
            .find(|p| p.kind == CodingProviderKind::ClaudeCode)
            .unwrap();
        assert!(claude.installed);
        assert_eq!(claude.version.as_deref(), Some("1.2.3 (Claude Code)"));
        assert!(claude.binary_path.is_some());

        let codex = results
            .iter()
            .find(|p| p.kind == CodingProviderKind::Codex)
            .unwrap();
        assert!(!codex.installed);
        assert!(codex.binary_path.is_none());

        let cursor = results
            .iter()
            .find(|p| p.kind == CodingProviderKind::CursorCli)
            .unwrap();
        assert!(cursor.installed);
        assert_eq!(cursor.version.as_deref(), Some("cursor-agent 0.9.1"));

        let agy = results
            .iter()
            .find(|p| p.kind == CodingProviderKind::Antigravity)
            .unwrap();
        assert!(agy.installed);
        assert_eq!(agy.version.as_deref(), Some("1.5.9"));

        let acp = results
            .iter()
            .find(|p| p.kind == CodingProviderKind::AcpCustom)
            .unwrap();
        assert!(acp.installed);
        assert!(acp.supports_acp);
        assert!(acp.binary_path.is_none());
    }

    #[test]
    #[cfg(unix)]
    fn probe_enforces_1500ms_hard_timeout_and_kills_hung_process() {
        let dir = TempDir::new().unwrap();
        write_exec_script(
            dir.path(),
            "opencode",
            "#!/bin/sh\nsleep 10\necho 'should not appear'\n",
        );

        let started = Instant::now();
        let results = probe_in_dirs(&[dir.path().to_path_buf()]);
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(3),
            "probe took too long: {elapsed:?}"
        );
        let opencode = results
            .iter()
            .find(|p| p.kind == CodingProviderKind::OpenCode)
            .unwrap();
        assert!(!opencode.installed);
        assert!(opencode.binary_path.is_some());
        assert!(opencode.version.is_none());
    }
}
