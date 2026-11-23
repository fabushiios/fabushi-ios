//! Git workspace isolation, non-invasive turn checkpoints, and PR helpers.

pub mod checkpoint;
pub mod pr;
pub mod worktree;

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Executes `git -C <cwd> <args...>` non-interactively and returns raw output.
pub(crate) fn run_git_raw(
    cwd: &Path,
    args: &[&str],
    extra_env: &[(&str, &std::ffi::OsStr)],
) -> Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output()
        .map_err(|e| format!("failed to execute git {}: {e}", args.join(" ")))
}

/// Executes `git -C <cwd> <args...>` and requires exit code 0, returning UTF-8 stdout.
pub(crate) fn run_git_ok(
    cwd: &Path,
    args: &[&str],
    extra_env: &[(&str, &std::ffi::OsStr)],
) -> Result<String, String> {
    let output = run_git_raw(cwd, args, extra_env)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(format!(
            "git {} failed (status {}): {}",
            args.join(" "),
            output.status,
            detail
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Formats raw bytes as lowercase hexadecimal.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}
