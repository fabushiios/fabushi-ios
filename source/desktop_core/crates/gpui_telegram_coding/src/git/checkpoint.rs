//! Non-invasive per-turn Git checkpoints mirroring `t3code` `GitVcsDriverCore`.
//!
//! Never mutates the user's `.git/index`, `HEAD`, or branch commit history:
//! dirty states are snapshotted via a temporary `GIT_INDEX_FILE` in `std::env::temp_dir()`
//! and stored under `refs/fabushi-coding/checkpoints/<thread_id>/<turn_idx>`.

use super::{run_git_ok, run_git_raw};
use gpui_telegram_proto::CodingDiffStat;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Default patch byte cap for `diff_checkpoints` (16 KiB).
pub const DEFAULT_MAX_PATCH_BYTES: usize = 16_384;

/// RAII guard for a temporary `GIT_INDEX_FILE` path in `std::env::temp_dir()`.
///
/// Note: Git requires that a new temporary index file NOT pre-exist as an empty
/// 0-byte file; this guard only allocates the unique path and removes the file
/// on drop.
struct TempIndexFile {
    path: PathBuf,
}

impl TempIndexFile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "gpui_coding_git_idx_{}.idx",
            uuid::Uuid::new_v4().simple()
        ));
        Self { path }
    }

    fn as_os_str(&self) -> &OsStr {
        self.path.as_os_str()
    }
}

impl Drop for TempIndexFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Returns `true` when `cwd` is inside a Git working tree.
pub fn is_git_repo(cwd: &Path) -> bool {
    let Ok(out) = run_git_raw(cwd, &["rev-parse", "--is-inside-work-tree"], &[]) else {
        return false;
    };
    out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
}

/// Resolves `HEAD` OID if `cwd` has a valid `HEAD` AND a completely clean working tree.
pub fn resolve_clean_head(cwd: &Path) -> Option<String> {
    let head = run_git_ok(cwd, &["rev-parse", "--verify", "HEAD"], &[]).ok()?;
    let head_oid = head.trim().to_string();
    if head_oid.is_empty() {
        return None;
    }
    let status = run_git_ok(cwd, &["status", "--porcelain"], &[]).ok()?;
    if status.trim().is_empty() {
        Some(head_oid)
    } else {
        None
    }
}

/// Returns the current checked-out branch name (`None` if detached or not a git repo).
pub fn current_branch(cwd: &Path) -> Option<String> {
    let out = run_git_ok(cwd, &["rev-parse", "--abbrev-ref", "HEAD"], &[]).ok()?;
    let trimmed = out.trim();
    if trimmed.is_empty() || trimmed == "HEAD" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Captures a non-invasive snapshot of `cwd` (tracked + untracked files) under `ref_name`.
///
/// Fast-paths to `git update-ref <ref_name> HEAD` when clean at `HEAD`; otherwise
/// uses a temporary `GIT_INDEX_FILE` (`read-tree` + `add -A` + `write-tree` + `commit-tree`)
/// without touching `.git/index`, `HEAD`, or branch history.
pub fn capture_checkpoint(cwd: &Path, ref_name: &str) -> Result<String, String> {
    validate_ref_name(ref_name)?;
    if !is_git_repo(cwd) {
        return Err(format!("`{}` is not a git repository", cwd.display()));
    }

    if let Some(head_oid) = resolve_clean_head(cwd) {
        run_git_ok(cwd, &["update-ref", ref_name, &head_oid], &[])?;
        return Ok(head_oid);
    }

    let tmp_idx = TempIndexFile::new();
    let idx_env = [("GIT_INDEX_FILE", tmp_idx.as_os_str())];

    let head_oid = run_git_ok(cwd, &["rev-parse", "--verify", "HEAD"], &[])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(ref oid) = head_oid {
        run_git_ok(cwd, &["read-tree", oid], &idx_env)?;
    }
    run_git_ok(cwd, &["add", "-A"], &idx_env)?;
    let tree_oid = run_git_ok(cwd, &["write-tree"], &idx_env)?
        .trim()
        .to_string();

    let author_name = std::env::var("GIT_AUTHOR_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Fabushi Coding Agent".to_string());
    let author_email = std::env::var("GIT_AUTHOR_EMAIL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "coding@fabushi.local".to_string());

    let commit_env = [
        ("GIT_AUTHOR_NAME", OsStr::new(author_name.as_str())),
        ("GIT_AUTHOR_EMAIL", OsStr::new(author_email.as_str())),
        ("GIT_COMMITTER_NAME", OsStr::new(author_name.as_str())),
        ("GIT_COMMITTER_EMAIL", OsStr::new(author_email.as_str())),
    ];

    let mut commit_args = vec!["commit-tree", &tree_oid];
    if let Some(ref parent) = head_oid {
        commit_args.push("-p");
        commit_args.push(parent);
    }
    commit_args.push("-m");
    commit_args.push("fabushi coding turn checkpoint");

    let commit_oid = run_git_ok(cwd, &commit_args, &commit_env)?
        .trim()
        .to_string();
    run_git_ok(cwd, &["update-ref", ref_name, &commit_oid], &[])?;
    Ok(commit_oid)
}

/// Computes `(CodingDiffStat, truncated_patch)` between `from_ref` and `to_ref`.
pub fn diff_checkpoints(
    cwd: &Path,
    from_ref: &str,
    to_ref: &str,
    max_patch_bytes: usize,
) -> Result<(CodingDiffStat, String), String> {
    let limit = if max_patch_bytes == 0 {
        DEFAULT_MAX_PATCH_BYTES
    } else {
        max_patch_bytes
    };

    let numstat_out = run_git_ok(
        cwd,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            "--numstat",
            from_ref,
            to_ref,
        ],
        &[],
    )?;
    let stat = parse_numstat(&numstat_out);

    let full_patch = run_git_ok(
        cwd,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            "--no-color",
            from_ref,
            to_ref,
        ],
        &[],
    )?;

    let patch = truncate_patch_utf8(&full_patch, limit);
    Ok((stat, patch))
}

fn parse_numstat(raw: &str) -> CodingDiffStat {
    let mut stat = CodingDiffStat::default();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let Some(add_str) = parts.next() else {
            continue;
        };
        let Some(del_str) = parts.next() else {
            continue;
        };
        let Some(file_path) = parts.next() else {
            continue;
        };

        let ins = add_str.trim().parse::<usize>().unwrap_or(0);
        let del = del_str.trim().parse::<usize>().unwrap_or(0);
        stat.files_changed += 1;
        stat.insertions += ins;
        stat.deletions += del;
        stat.changed_files.push(file_path.to_string());
    }
    stat
}

fn truncate_patch_utf8(patch: &str, max_bytes: usize) -> String {
    if patch.len() <= max_bytes {
        return patch.to_string();
    }

    let mut cut = max_bytes;
    while cut > 0 && !patch.is_char_boundary(cut) {
        cut -= 1;
    }
    if let Some(last_nl) = patch[..cut].rfind('\n') {
        if last_nl > 0 {
            cut = last_nl + 1;
        }
    }

    let total = patch.len();
    format!(
        "{}\n... [diff truncated at {cut} of {total} bytes (limit {max_bytes} bytes)]",
        &patch[..cut]
    )
}

/// Restores tracked files and removes newly added untracked files to match `target_ref`,
/// without moving `HEAD` or altering commit history.
pub fn restore_checkpoint(cwd: &Path, target_ref: &str) -> Result<(), String> {
    if !is_git_repo(cwd) {
        return Err(format!("`{}` is not a git repository", cwd.display()));
    }
    let tree_spec = format!("{target_ref}^{{tree}}");
    run_git_ok(cwd, &["rev-parse", "--verify", &tree_spec], &[])?;

    let target_files = list_nul_paths(
        &run_git_ok(
            cwd,
            &["ls-tree", "-r", "--name-only", "-z", target_ref],
            &[],
        )?,
    );
    let target_set: HashSet<String> = target_files.iter().cloned().collect();

    let mut current_files = list_nul_paths(&run_git_ok(cwd, &["ls-files", "-z"], &[])?);
    current_files.extend(list_nul_paths(&run_git_ok(
        cwd,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        &[],
    )?));

    // Delete files present in working tree/index that do not exist in target_ref.
    for rel in current_files {
        if !target_set.contains(&rel) {
            let full = cwd.join(&rel);
            if full.exists() || full.is_symlink() {
                if full.is_dir() {
                    let _ = std::fs::remove_dir_all(&full);
                } else {
                    std::fs::remove_file(&full).map_err(|e| {
                        format!("failed to remove file `{}` during rollback: {e}", full.display())
                    })?;
                }
                prune_empty_parent_dirs(full.parent(), cwd);
            }
        }
    }

    // Checkout exact target_ref tree via temporary index file so user's .git/index is not abused.
    if !target_files.is_empty() {
        let tmp_idx = TempIndexFile::new();
        let idx_env = [("GIT_INDEX_FILE", tmp_idx.as_os_str())];
        run_git_ok(cwd, &["read-tree", target_ref], &idx_env)?;
        run_git_ok(cwd, &["checkout-index", "-a", "-f"], &idx_env)?;
    }

    // Keep `.git/index` synced to `HEAD` (never moves `HEAD`) so deleted newly-staged files
    // do not appear as phantom staged deletions.
    if run_git_raw(cwd, &["rev-parse", "--verify", "HEAD"], &[])?
        .status
        .success()
    {
        let _ = run_git_raw(cwd, &["read-tree", "HEAD"], &[]);
    }

    Ok(())
}

fn list_nul_paths(raw: &str) -> Vec<String> {
    raw.split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn prune_empty_parent_dirs(mut dir: Option<&Path>, stop_at: &Path) {
    while let Some(d) = dir {
        if d == stop_at || !d.starts_with(stop_at) {
            break;
        }
        match std::fs::read_dir(d) {
            Ok(mut entries) => {
                if entries.next().is_none() {
                    let _ = std::fs::remove_dir(d);
                    dir = d.parent();
                } else {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn validate_ref_name(ref_name: &str) -> Result<(), String> {
    let trimmed = ref_name.trim();
    if trimmed.is_empty()
        || !trimmed.starts_with("refs/")
        || trimmed.contains("..")
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("invalid checkpoint ref name: `{ref_name}`"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn checkpoints_never_mutate_head_and_rollback_cleanly() {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init"]);
        std::fs::write(dir.path().join("main.rs"), "fn main() { println!(\"v1\"); }\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init commit"]);

        let head_before = git(dir.path(), &["rev-parse", "HEAD"]).trim().to_string();
        let log_before = git(dir.path(), &["log", "--oneline"]).trim().to_string();

        // Pre-existing dirty modification + untracked file before turn starts.
        std::fs::write(dir.path().join("notes.txt"), "user pre-existing note\n").unwrap();

        let ref_before = "refs/fabushi-coding/checkpoints/thread1/1-before";
        let ref_after = "refs/fabushi-coding/checkpoints/thread1/1";

        let before_oid = capture_checkpoint(dir.path(), ref_before).unwrap();
        assert_ne!(before_oid, head_before);

        // Simulate coding turn changes: modify tracked file, edit pre-existing untracked, add nested file.
        std::fs::write(
            dir.path().join("main.rs"),
            "fn main() { println!(\"v2\"); }\n// added line\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
        std::fs::write(dir.path().join("src/nested/helper.rs"), "pub fn help() {}\n").unwrap();

        let after_oid = capture_checkpoint(dir.path(), ref_after).unwrap();
        assert_ne!(after_oid, before_oid);

        // Verify HEAD & git log & staging index remain completely untouched!
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]).trim(), head_before);
        assert_eq!(git(dir.path(), &["log", "--oneline"]).trim(), log_before);
        assert_eq!(git(dir.path(), &["diff", "--cached"]).trim(), "");

        // Verify diff_checkpoints accuracy & clean truncation.
        let (stat, patch) = diff_checkpoints(dir.path(), ref_before, ref_after, 16384).unwrap();
        assert_eq!(stat.files_changed, 2);
        assert!(stat.insertions >= 3);
        assert!(stat.deletions >= 1);
        assert!(stat.changed_files.contains(&"main.rs".to_string()));
        assert!(stat.changed_files.contains(&"src/nested/helper.rs".to_string()));
        assert!(patch.contains("helper.rs"));

        let (_stat_trunc, small_patch) = diff_checkpoints(dir.path(), ref_before, ref_after, 40).unwrap();
        assert!(small_patch.contains("truncated"));

        // Rollback to before_ref restores main.rs, keeps notes.txt, and deletes src/nested/helper.rs.
        restore_checkpoint(dir.path(), ref_before).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).unwrap(),
            "fn main() { println!(\"v1\"); }\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes.txt")).unwrap(),
            "user pre-existing note\n"
        );
        assert!(!dir.path().join("src/nested/helper.rs").exists());
        assert!(!dir.path().join("src").exists());

        // HEAD & git log still completely untouched.
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]).trim(), head_before);
        assert_eq!(git(dir.path(), &["log", "--oneline"]).trim(), log_before);
    }

    #[test]
    fn clean_head_fast_path_reuses_head_oid() {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init"]);
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "c1"]);

        let head = git(dir.path(), &["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(resolve_clean_head(dir.path()).as_deref(), Some(head.as_str()));

        let cp = capture_checkpoint(dir.path(), "refs/fabushi-coding/checkpoints/t2/1-before").unwrap();
        assert_eq!(cp, head);
    }
}
