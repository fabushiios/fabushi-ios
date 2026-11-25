//! Git worktree isolation for parallel per-thread/topic coding sessions.

use super::{hex_encode, run_git_ok, run_git_raw};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_prunable: bool,
}

/// Canonicalizes `workspace_path` and ensures it exists and is a directory.
pub fn validate_workspace_dir(workspace_path: &str) -> Result<PathBuf, String> {
    let trimmed = workspace_path.trim();
    if trimmed.is_empty() {
        return Err("workspace_path cannot be empty".to_string());
    }
    let path = Path::new(trimmed);
    if !path.exists() {
        return Err(format!("workspace_path does not exist: {trimmed}"));
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| format!("failed to canonicalize workspace_path `{trimmed}`: {e}"))?;
    if !canonical.is_dir() {
        return Err(format!(
            "workspace_path is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// Resolves the canonical git repository top-level root for `path`, if inside a git repo.
pub fn git_repo_root(path: &Path) -> Option<PathBuf> {
    let out = run_git_ok(path, &["rev-parse", "--show-toplevel"], &[]).ok()?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return None;
    }
    std::fs::canonicalize(trimmed).ok().or_else(|| Some(PathBuf::from(trimmed)))
}

/// Builds a deterministic, filesystem-safe worktree slug for a Telegram thread.
pub fn thread_worktree_slug(bot_user_id: i64, chat_id: i64, topic_id: Option<i64>) -> String {
    let raw = match topic_id {
        Some(tid) if tid != 0 => format!("b{bot_user_id}_c{chat_id}_t{tid}"),
        _ => format!("b{bot_user_id}_c{chat_id}"),
    };
    sanitize_slug(&raw)
}

/// Builds an owner-isolated, filesystem-safe worktree slug so distinct local accounts never
/// collide on worktree directory paths or `fabushi/*` Git branches.
pub fn thread_worktree_slug_for_owner(
    owner_user_id: u64,
    bot_user_id: i64,
    chat_id: i64,
    topic_id: Option<i64>,
) -> String {
    let raw = match topic_id {
        Some(tid) if tid != 0 => format!("u{owner_user_id}_b{bot_user_id}_c{chat_id}_t{tid}"),
        _ => format!("u{owner_user_id}_b{bot_user_id}_c{chat_id}"),
    };
    sanitize_slug(&raw)
}

fn sanitize_slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches(|c| c == '.' || c == '-');
    if trimmed.is_empty() || trimmed == ".." {
        "thread".to_string()
    } else {
        trimmed.replace("..", "-")
    }
}

fn repo_hash_12(repo_root: &Path) -> String {
    let canonical = std::fs::canonicalize(repo_root).unwrap_or_else(|_| repo_root.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    hex_encode(&digest)[..12].to_string()
}

fn sanitize_branch_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.contains("..")
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.ends_with(".lock")
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("invalid git branch name: `{trimmed}`"));
    }
    Ok(trimmed.to_string())
}

/// Creates or reuses an isolated git worktree under
/// `<worktrees_root>/<repo_hash_12>/<thread_slug>` on `fabushi/<thread_slug>` (or `preferred_branch`).
pub fn ensure_worktree(
    repo_root: &Path,
    worktrees_root: &Path,
    thread_slug: &str,
    preferred_branch: Option<&str>,
) -> Result<WorktreeInfo, String> {
    let canonical_repo = validate_workspace_dir(&repo_root.to_string_lossy())?;
    let actual_root = git_repo_root(&canonical_repo).ok_or_else(|| {
        format!(
            "`{}` is not inside a git repository",
            canonical_repo.display()
        )
    })?;

    // Verify repository has at least one commit (HEAD exists) so worktree creation works cleanly.
    if run_git_ok(&actual_root, &["rev-parse", "--verify", "HEAD"], &[]).is_err() {
        return Err(format!(
            "git repository `{}` has no commits yet (unborn HEAD); create an initial commit first",
            actual_root.display()
        ));
    }

    let safe_slug = sanitize_slug(thread_slug);
    let hash_dir = repo_hash_12(&actual_root);
    let container_dir = worktrees_root.join(&hash_dir);
    std::fs::create_dir_all(&container_dir).map_err(|e| {
        format!(
            "failed to create worktrees container `{}`: {e}",
            container_dir.display()
        )
    })?;
    let canonical_container = std::fs::canonicalize(&container_dir).unwrap_or(container_dir);
    let target_path = canonical_container.join(&safe_slug);

    let desired_branch = match preferred_branch.map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) => sanitize_branch_name(b)?,
        None => format!("fabushi/{safe_slug}"),
    };

    // Reuse if target_path is already a registered worktree of this repo.
    let existing = list_worktrees(&actual_root)?;
    if target_path.exists() {
        let canonical_target = std::fs::canonicalize(&target_path).unwrap_or_else(|_| target_path.clone());
        if let Some(entry) = existing.iter().find(|e| e.path == canonical_target) {
            let current_branch = entry
                .branch
                .clone()
                .or_else(|| current_branch_name(&canonical_target))
                .unwrap_or_else(|| desired_branch.clone());
            return Ok(WorktreeInfo {
                path: canonical_target,
                branch: current_branch,
                created: false,
            });
        }
        return Err(format!(
            "worktree path `{}` already exists but is not registered as a git worktree of `{}`",
            target_path.display(),
            actual_root.display()
        ));
    }

    // Prune stale metadata in case a directory was deleted externally earlier.
    let _ = run_git_raw(&actual_root, &["worktree", "prune"], &[]);

    let target_str = target_path.to_string_lossy().into_owned();
    let local_ref = format!("refs/heads/{desired_branch}");
    let branch_exists = run_git_raw(
        &actual_root,
        &["show-ref", "--verify", "--quiet", &local_ref],
        &[],
    )?
    .status
    .success();

    if branch_exists {
        // Check if branch is already checked out in another worktree.
        if let Some(checked_out) = existing
            .iter()
            .find(|e| e.branch.as_deref() == Some(desired_branch.as_str()))
        {
            return Err(format!(
                "branch `{desired_branch}` is already checked out at `{}`",
                checked_out.path.display()
            ));
        }
        run_git_ok(
            &actual_root,
            &["worktree", "add", &target_str, &desired_branch],
            &[],
        )?;
    } else {
        run_git_ok(
            &actual_root,
            &["worktree", "add", "-b", &desired_branch, &target_str],
            &[],
        )?;
    }

    let canonical_target = std::fs::canonicalize(&target_path).unwrap_or(target_path);
    Ok(WorktreeInfo {
        path: canonical_target,
        branch: desired_branch,
        created: true,
    })
}

fn current_branch_name(cwd: &Path) -> Option<String> {
    let out = run_git_ok(cwd, &["rev-parse", "--abbrev-ref", "HEAD"], &[]).ok()?;
    let trimmed = out.trim();
    if trimmed.is_empty() || trimmed == "HEAD" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Lists all git worktrees registered on `repo_root` via `git worktree list --porcelain`.
pub fn list_worktrees(repo_root: &Path) -> Result<Vec<WorktreeEntry>, String> {
    let actual_root = git_repo_root(repo_root).unwrap_or_else(|| repo_root.to_path_buf());
    let out = run_git_ok(&actual_root, &["worktree", "list", "--porcelain"], &[])?;
    let mut entries = Vec::new();
    let mut current: Option<WorktreeEntry> = None;

    for line in out.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            continue;
        }
        if let Some(path_str) = line.strip_prefix("worktree ") {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            let p = PathBuf::from(path_str);
            let canonical = std::fs::canonicalize(&p).unwrap_or(p);
            current = Some(WorktreeEntry {
                path: canonical,
                head: None,
                branch: None,
                is_bare: false,
                is_prunable: false,
            });
        } else if let Some(entry) = current.as_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                entry.head = Some(head.trim().to_string());
            } else if let Some(branch_ref) = line.strip_prefix("branch ") {
                let b = branch_ref.trim();
                entry.branch = Some(
                    b.strip_prefix("refs/heads/")
                        .unwrap_or(b)
                        .to_string(),
                );
            } else if line == "bare" {
                entry.is_bare = true;
            } else if line.starts_with("prunable") {
                entry.is_prunable = true;
            }
        }
    }
    if let Some(entry) = current.take() {
        entries.push(entry);
    }

    Ok(entries)
}

/// Removes a worktree from `repo_root`. Refuses dirty removal unless `force == true`.
pub fn remove_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    force: bool,
) -> Result<(), String> {
    let actual_root = git_repo_root(repo_root).unwrap_or_else(|| repo_root.to_path_buf());
    let canonical_wt = std::fs::canonicalize(worktree_path)
        .unwrap_or_else(|_| worktree_path.to_path_buf());

    if canonical_wt == actual_root {
        return Err("refusing to remove the primary repository worktree".to_string());
    }

    if !force && canonical_wt.exists() {
        let status = run_git_ok(&canonical_wt, &["status", "--porcelain"], &[])?;
        if !status.trim().is_empty() {
            return Err(format!(
                "refusing to remove dirty worktree `{}` with uncommitted changes (use force=true to override)",
                canonical_wt.display()
            ));
        }
    }

    let wt_str = canonical_wt.to_string_lossy().into_owned();
    if force {
        run_git_ok(&actual_root, &["worktree", "remove", "--force", &wt_str], &[])?;
    } else {
        run_git_ok(&actual_root, &["worktree", "remove", &wt_str], &[])?;
    }
    let _ = run_git_raw(&actual_root, &["worktree", "prune"], &[]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_repo_with_commit(dir: &Path) {
        let run = |args: &[&str]| {
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
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init"]);
        std::fs::write(dir.join("README.md"), "# demo\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "initial"]);
    }

    #[test]
    fn validate_workspace_and_worktree_lifecycle() {
        let repo_dir = TempDir::new().unwrap();
        let wt_root = TempDir::new().unwrap();
        init_repo_with_commit(repo_dir.path());

        let valid = validate_workspace_dir(&repo_dir.path().to_string_lossy()).unwrap();
        assert!(valid.is_dir());
        assert!(validate_workspace_dir("/definitely/nonexistent/path/xyz").is_err());

        let wt1 = ensure_worktree(&valid, wt_root.path(), "topic_7", None).unwrap();
        assert!(wt1.created);
        assert_eq!(wt1.branch, "fabushi/topic_7");
        assert!(wt1.path.exists());
        assert!(wt1.path.to_string_lossy().contains("topic_7"));

        // Reusing returns existing worktree with created == false.
        let wt1_again = ensure_worktree(&valid, wt_root.path(), "topic_7", None).unwrap();
        assert!(!wt1_again.created);
        assert_eq!(wt1_again.path, wt1.path);
        assert_eq!(wt1_again.branch, "fabushi/topic_7");

        let list = list_worktrees(&valid).unwrap();
        assert!(list.iter().any(|e| e.path == wt1.path && e.branch.as_deref() == Some("fabushi/topic_7")));

        // Dirty worktree refuses removal when force == false.
        std::fs::write(wt1.path.join("dirty.txt"), "uncommitted").unwrap();
        let err = remove_worktree(&valid, &wt1.path, false).unwrap_err();
        assert!(err.contains("dirty"));
        assert!(wt1.path.exists());

        // force == true succeeds.
        remove_worktree(&valid, &wt1.path, true).unwrap();
        assert!(!wt1.path.exists());

        // Also test preferred_branch.
        let wt2 = ensure_worktree(&valid, wt_root.path(), "topic_8", Some("feat/custom")).unwrap();
        assert!(wt2.created);
        assert_eq!(wt2.branch, "feat/custom");
        remove_worktree(&valid, &wt2.path, false).unwrap();
    }
}
