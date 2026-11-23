//! Git commit, push, and GitHub CLI PR creation helpers.

use super::checkpoint::{current_branch, is_git_repo};
use super::{run_git_ok, run_git_raw};
use crate::probe::{default_search_dirs, resolve_binary_in_dirs};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

const GH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Stages & commits dirty changes (if any), pushes `HEAD` to `origin`, and creates a PR via `gh`.
pub async fn commit_and_pr(
    cwd: &Path,
    commit_message: Option<&str>,
    pr_title: Option<&str>,
) -> Result<String, String> {
    let cwd_owned = cwd.to_path_buf();
    let msg_owned = commit_message.map(str::to_string);
    let title_owned = pr_title.map(str::to_string);

    let msg_for_blocking = msg_owned.clone();
    let (committed_summary, branch_name, has_origin, push_result) =
        tokio::task::spawn_blocking(move || -> Result<(Option<String>, String, bool, Result<(), String>), String> {
            if !is_git_repo(&cwd_owned) {
                return Err(format!("`{}` is not a git repository.", cwd_owned.display()));
            }

            let status = run_git_ok(&cwd_owned, &["status", "--porcelain"], &[])?;
            let committed_summary = if !status.trim().is_empty() {
                let msg = msg_for_blocking
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("chore(coding): apply coding agent changes");
                run_git_ok(&cwd_owned, &["add", "-A"], &[])?;

                let author_name = std::env::var("GIT_AUTHOR_NAME")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Fabushi Coding Agent".to_string());
                let author_email = std::env::var("GIT_AUTHOR_EMAIL")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "coding@fabushi.local".to_string());

                run_git_ok(
                    &cwd_owned,
                    &["commit", "-m", msg],
                    &[
                        ("GIT_AUTHOR_NAME", std::ffi::OsStr::new(author_name.as_str())),
                        ("GIT_AUTHOR_EMAIL", std::ffi::OsStr::new(author_email.as_str())),
                        ("GIT_COMMITTER_NAME", std::ffi::OsStr::new(author_name.as_str())),
                        ("GIT_COMMITTER_EMAIL", std::ffi::OsStr::new(author_email.as_str())),
                    ],
                )?;
                let short_sha = run_git_ok(&cwd_owned, &["rev-parse", "--short", "HEAD"], &[])
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                Some(format!("Committed `{short_sha}`: {msg}"))
            } else {
                None
            };

            let branch = current_branch(&cwd_owned).ok_or_else(|| {
                "Repository is on a detached HEAD. Checkout or create a branch before pushing/creating a PR.".to_string()
            })?;

            let remotes = run_git_ok(&cwd_owned, &["remote"], &[])?;
            let has_origin = remotes.lines().any(|r| r.trim() == "origin");
            if !has_origin {
                return Ok((committed_summary, branch, false, Ok(())));
            }

            let push_res = run_git_ok(&cwd_owned, &["push", "-u", "origin", "HEAD"], &[])
                .map(|_| ());
            Ok((committed_summary, branch, true, push_res))
        })
        .await
        .map_err(|e| format!("commit_and_pr task failed: {e}"))??;

    let mut lines = Vec::new();
    if let Some(ref c) = committed_summary {
        lines.push(format!("- {c}"));
    } else {
        lines.push("- Working tree was already clean (no new commit needed).".to_string());
    }

    if !has_origin {
        lines.push(format!(
            "- Branch `{branch_name}` is ready locally, but no `origin` remote is configured."
        ));
        lines.push("  Next step: `git remote add origin <url> && git push -u origin HEAD`".to_string());
        return Ok(lines.join("\n"));
    }

    if let Err(push_err) = push_result {
        lines.push(format!("- Push to `origin/{branch_name}` failed: `{push_err}`"));
        lines.push("  Verify git remote credentials/permissions and retry.".to_string());
        return Ok(lines.join("\n"));
    }
    lines.push(format!("- Pushed branch `{branch_name}` to `origin`."));

    let dirs = default_search_dirs();
    let Some(gh_bin) = resolve_binary_in_dirs("gh", &dirs) else {
        lines.push("- `gh` (GitHub CLI) was not found in PATH.".to_string());
        lines.push("  Install `gh` (`brew install gh`) or open the PR from your repository host web UI.".to_string());
        return Ok(lines.join("\n"));
    };

    // Check if PR already exists for this branch.
    let view_out = run_gh_command(&gh_bin, cwd, &["pr", "view", "--json", "url", "-q", ".url"]).await;
    if let Ok(existing_url) = view_out {
        let url = existing_url.trim();
        if !url.is_empty() {
            lines.push(format!("- Pull Request already exists: {url}"));
            return Ok(lines.join("\n"));
        }
    }

    let create_res = if let Some(title) = title_owned.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let body = msg_owned
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Created by Fabushi Coding Agent.");
        run_gh_command(
            &gh_bin,
            cwd,
            &["pr", "create", "--title", title, "--body", body],
        )
        .await
    } else {
        run_gh_command(&gh_bin, cwd, &["pr", "create", "--fill"]).await
    };

    match create_res {
        Ok(out) => {
            let pr_url = out
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| l.starts_with("http://") || l.starts_with("https://"))
                .unwrap_or_else(|| out.trim());
            lines.push(format!("- Pull Request created: {pr_url}"));
        }
        Err(err) => {
            lines.push(format!("- Branch pushed, but `gh pr create` failed: `{err}`"));
            lines.push("  If unauthenticated, run `gh auth login` or create the PR manually.".to_string());
        }
    }

    Ok(lines.join("\n"))
}

async fn run_gh_command(gh_bin: &Path, cwd: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(gh_bin);
    cmd.current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);

    let child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn gh: {e}"))?;

    let output = tokio::time::timeout(GH_COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "gh command timed out".to_string())?
        .map_err(|e| format!("gh wait failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if !stderr.is_empty() { stderr } else { stdout });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Checks whether `cwd` has uncommitted changes (`git status --porcelain`).
pub fn count_dirty_entries(cwd: &Path) -> usize {
    let Ok(out) = run_git_raw(cwd, &["status", "--porcelain"], &[]) else {
        return 0;
    };
    if !out.status.success() {
        return 0;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Tester")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Tester")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    #[tokio::test]
    async fn commit_and_pr_commits_cleanly_without_origin_and_returns_guidance() {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-b", "feat/test-pr"]);
        std::fs::write(dir.path().join("a.txt"), "v1\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init"]);

        std::fs::write(dir.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(count_dirty_entries(dir.path()), 1);

        let reply = commit_and_pr(dir.path(), Some("feat: update a.txt"), Some("Update a.txt"))
            .await
            .unwrap();
        assert!(reply.contains("Committed"));
        assert!(reply.contains("no `origin` remote is configured"));
        assert_eq!(count_dirty_entries(dir.path()), 0);

        // Now attach a local bare origin remote and verify push succeeds cleanly.
        let bare = TempDir::new().unwrap();
        git(bare.path(), &["init", "--bare"]);
        let bare_str = bare.path().to_string_lossy().into_owned();
        git(dir.path(), &["remote", "add", "origin", &bare_str]);
        std::fs::write(dir.path().join("b.txt"), "pushed\n").unwrap();

        let reply2 = commit_and_pr(dir.path(), Some("feat: add b.txt"), Some("Add b.txt"))
            .await
            .unwrap();
        assert!(reply2.contains("Committed"));
        assert!(reply2.contains("Pushed branch `feat/test-pr` to `origin`"));
        assert_eq!(count_dirty_entries(dir.path()), 0);
    }
}
