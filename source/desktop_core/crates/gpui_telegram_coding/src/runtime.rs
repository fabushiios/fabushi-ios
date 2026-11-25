//! `LocalCodingHostRuntime`: command router, per-thread turn serialization,
//! approval routing, git checkpoints, SQLite WAL state, and shutdown lifecycle.

use crate::driver::{
    DriverContext, DriverTurnRequest, execute_driver_turn,
};
use crate::git::checkpoint::{
    DEFAULT_MAX_PATCH_BYTES, capture_checkpoint, current_branch, diff_checkpoints, is_git_repo,
    restore_checkpoint,
};
use crate::git::pr::{commit_and_pr, count_dirty_entries};
use crate::git::worktree::{
    ensure_worktree, git_repo_root, list_worktrees, remove_worktree,
    thread_worktree_slug_for_owner, validate_workspace_dir,
};
use crate::probe::{check_provider_login_status, probe_local_coding_providers};
use crate::store::{
    CheckpointRecord, CodingStore, ThreadKey, ThreadRecord, TurnReceiptClaim, now_ms, preview_text,
};
use gpui_telegram_proto::{
    CodingAgentBotConfig, CodingApprovalDecision, CodingControlAction, CodingDiffStat,
    CodingHostCommand, CodingHostEvent, CodingProviderKind,
};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

/// Configuration for [`LocalCodingHostRuntime`].
#[derive(Debug, Clone)]
pub struct CodingHostRuntimeConfig {
    pub owner_user_id: u64,
    pub device_id: String,
    pub db_path: PathBuf,
    pub worktrees_root: PathBuf,
}

struct ActiveTurnHandle {
    thread_key: ThreadKey,
    cancel_tx: watch::Sender<bool>,
    pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<CodingApprovalDecision>>>>,
}

struct QueuedTurn {
    turn_id: String,
    key: ThreadKey,
    is_forum_topic: bool,
    prompt: String,
    config: CodingAgentBotConfig,
    emit: mpsc::UnboundedSender<CodingHostEvent>,
    cancel_rx: watch::Receiver<bool>,
    pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<CodingApprovalDecision>>>>,
}

struct RuntimeInner {
    config: CodingHostRuntimeConfig,
    store: Arc<CodingStore>,
    shutting_down: AtomicBool,
    thread_mutexes: Mutex<HashMap<ThreadKey, Arc<tokio::sync::Mutex<()>>>>,
    thread_workers: Mutex<HashMap<ThreadKey, mpsc::UnboundedSender<QueuedTurn>>>,
    thread_allow_rules: Mutex<HashMap<ThreadKey, Arc<RwLock<HashSet<String>>>>>,
    active_turns: Mutex<HashMap<String, Arc<ActiveTurnHandle>>>,
    worker_tasks: Mutex<Vec<JoinHandle<()>>>,
}

/// Pure-Rust local host execution runtime for Coding Agent Bots.
pub struct LocalCodingHostRuntime {
    inner: Arc<RuntimeInner>,
}

impl LocalCodingHostRuntime {
    /// Initializes persistent SQLite WAL storage and worktree container directory.
    pub fn new(config: CodingHostRuntimeConfig) -> Result<Self, String> {
        let store = Arc::new(CodingStore::open(&config.db_path)?);
        if !config.worktrees_root.as_os_str().is_empty() {
            std::fs::create_dir_all(&config.worktrees_root).map_err(|e| {
                format!(
                    "failed to create worktrees_root `{}`: {e}",
                    config.worktrees_root.display()
                )
            })?;
        }
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                config,
                store,
                shutting_down: AtomicBool::new(false),
                thread_mutexes: Mutex::new(HashMap::new()),
                thread_workers: Mutex::new(HashMap::new()),
                thread_allow_rules: Mutex::new(HashMap::new()),
                active_turns: Mutex::new(HashMap::new()),
                worker_tasks: Mutex::new(Vec::new()),
            }),
        })
    }

    /// Creates an isolated ephemeral runtime rooted under `std::env::temp_dir()`.
    pub fn new_ephemeral(
        owner_user_id: u64,
        device_id: impl Into<String>,
    ) -> Result<Self, String> {
        let base = std::env::temp_dir().join(format!(
            "gpui_telegram_coding_{}",
            uuid::Uuid::new_v4().simple()
        ));
        Self::new(CodingHostRuntimeConfig {
            owner_user_id,
            device_id: device_id.into(),
            db_path: base.join("coding_host.db"),
            worktrees_root: base.join("worktrees"),
        })
    }

    pub fn device_id(&self) -> &str {
        &self.inner.config.device_id
    }

    pub fn owner_user_id(&self) -> u64 {
        self.inner.config.owner_user_id
    }

    pub fn db_path(&self) -> &Path {
        &self.inner.config.db_path
    }

    pub fn worktrees_root(&self) -> &Path {
        &self.inner.config.worktrees_root
    }

    pub fn is_shutting_down(&self) -> bool {
        self.inner.shutting_down.load(Ordering::SeqCst)
    }

    /// Dispatches a `CodingHostCommand`, enforcing strict `owner_user_id` verification.
    pub async fn handle_command(
        &self,
        cmd: CodingHostCommand,
        emit: mpsc::UnboundedSender<CodingHostEvent>,
    ) {
        let (cmd_owner, target_device) = command_owner_and_target(&cmd);

        // SECURITY INVARIANT: Reject any owner_user_id mismatch immediately.
        if cmd_owner != self.inner.config.owner_user_id {
            tracing::warn!(
                expected_owner = self.inner.config.owner_user_id,
                actual_owner = cmd_owner,
                "rejecting CodingHostCommand due to owner_user_id mismatch"
            );
            emit_owner_mismatch_rejection(&cmd, self.inner.config.owner_user_id, &self.inner.config.device_id, &emit);
            return;
        }

        if let Some(target) = target_device.map(str::trim).filter(|s| !s.is_empty()) {
            if target != self.inner.config.device_id {
                tracing::debug!(
                    local_device = %self.inner.config.device_id,
                    target_device = %target,
                    "ignoring CodingHostCommand addressed to another device"
                );
                return;
            }
        }

        if self.inner.shutting_down.load(Ordering::SeqCst) {
            emit_shutdown_rejection(&cmd, &self.inner.config.device_id, &emit);
            return;
        }

        match cmd {
            CodingHostCommand::ProbeProviders { req_id, .. } => {
                let providers = probe_local_coding_providers().await;
                let _ = emit.send(CodingHostEvent::ProbeResult {
                    req_id,
                    device_id: self.inner.config.device_id.clone(),
                    providers,
                });
            }
            CodingHostCommand::StartTurn {
                turn_id,
                bot_user_id,
                chat_id,
                topic_id,
                is_forum_topic,
                prompt,
                config,
                ..
            } => {
                self.enqueue_start_turn(
                    turn_id,
                    ThreadKey::for_owner(
                        self.inner.config.owner_user_id,
                        bot_user_id,
                        chat_id,
                        topic_id,
                    ),
                    is_forum_topic,
                    prompt,
                    config,
                    emit,
                );
            }
            CodingHostCommand::CancelTurn { turn_id, .. } => {
                self.cancel_active_turn(&turn_id);
            }
            CodingHostCommand::ResolveApproval {
                turn_id,
                request_id,
                decision,
                ..
            } => {
                self.resolve_turn_approval(&turn_id, &request_id, decision);
            }
            CodingHostCommand::Control {
                req_id,
                bot_user_id,
                chat_id,
                topic_id,
                is_forum_topic,
                config,
                action,
                ..
            } => {
                let result = self
                    .handle_control_action(
                        req_id,
                        ThreadKey::for_owner(
                            self.inner.config.owner_user_id,
                            bot_user_id,
                            chat_id,
                            topic_id,
                        ),
                        is_forum_topic,
                        config,
                        action,
                    )
                    .await;
                let _ = emit.send(result);
            }
        }
    }

    /// Cancels all active turns, kills child processes, and drains background workers.
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);

        let active: Vec<Arc<ActiveTurnHandle>> = {
            let guard = self.inner.active_turns.lock();
            guard.values().cloned().collect()
        };
        for handle in active {
            let _ = handle.cancel_tx.send(true);
            let pending: Vec<_> = {
                let mut map = handle.pending_approvals.lock();
                map.drain().map(|(_, tx)| tx).collect()
            };
            for tx in pending {
                let _ = tx.send(CodingApprovalDecision::Deny);
            }
        }

        // Drop all worker channel senders so idle worker tasks exit naturally.
        self.inner.thread_workers.lock().clear();

        let tasks: Vec<JoinHandle<()>> = {
            let mut guard = self.inner.worker_tasks.lock();
            std::mem::take(&mut *guard)
        };
        for mut task in tasks {
            if tokio::time::timeout(std::time::Duration::from_secs(2), &mut task)
                .await
                .is_err()
            {
                task.abort();
            }
        }
    }

    fn enqueue_start_turn(
        &self,
        turn_id: String,
        key: ThreadKey,
        is_forum_topic: bool,
        prompt: String,
        config: CodingAgentBotConfig,
        emit: mpsc::UnboundedSender<CodingHostEvent>,
    ) {
        let deterministic_tid = default_thread_id(key);
        match self.inner.store.claim_turn_receipt(
            self.inner.config.owner_user_id,
            &turn_id,
            &deterministic_tid,
        ) {
            Ok(TurnReceiptClaim::Claimed) => {}
            Ok(TurnReceiptClaim::AlreadyCompleted(ev)) => {
                let _ = emit.send(ev);
                return;
            }
            Ok(TurnReceiptClaim::InProgress) => {
                tracing::debug!(turn_id = %turn_id, "ignoring duplicate in-flight StartTurn");
                return;
            }
            Ok(TurnReceiptClaim::FinishedWithoutEvent { turn_idx, status }) => {
                let _ = emit.send(CodingHostEvent::TurnCompleted {
                    turn_id,
                    ok: status == "completed",
                    final_text: String::new(),
                    error: (status != "completed").then(|| format!("Previous turn status: {status}")),
                    turn_idx,
                    checkpoint_ref: None,
                    diff_stat: None,
                    token_usage: None,
                });
                return;
            }
            Err(err) => {
                let _ = emit.send(CodingHostEvent::TurnCompleted {
                    turn_id,
                    ok: false,
                    final_text: String::new(),
                    error: Some(err),
                    turn_idx: 0,
                    checkpoint_ref: None,
                    diff_stat: None,
                    token_usage: None,
                });
                return;
            }
        }

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        let handle = Arc::new(ActiveTurnHandle {
            thread_key: key,
            cancel_tx,
            pending_approvals: Arc::clone(&pending_approvals),
        });
        self.inner
            .active_turns
            .lock()
            .insert(turn_id.clone(), handle);

        let worker_tx = self.get_or_spawn_thread_worker(key);
        let queued = QueuedTurn {
            turn_id: turn_id.clone(),
            key,
            is_forum_topic,
            prompt,
            config,
            emit: emit.clone(),
            cancel_rx,
            pending_approvals,
        };

        if let Err(err) = worker_tx.send(queued) {
            self.inner.active_turns.lock().remove(&turn_id);
            let _ = self.inner.store.finish_turn_receipt(
                self.inner.config.owner_user_id,
                &turn_id,
                "failed",
                0,
                None,
            );
            let _ = emit.send(CodingHostEvent::TurnCompleted {
                turn_id,
                ok: false,
                final_text: String::new(),
                error: Some(format!("failed to enqueue turn worker: {err}")),
                turn_idx: 0,
                checkpoint_ref: None,
                diff_stat: None,
                token_usage: None,
            });
        }
    }

    fn get_or_spawn_thread_worker(
        &self,
        key: ThreadKey,
    ) -> mpsc::UnboundedSender<QueuedTurn> {
        let mut workers = self.inner.thread_workers.lock();
        if let Some(existing) = workers.get(&key) {
            if !existing.is_closed() {
                return existing.clone();
            }
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<QueuedTurn>();
        let thread_mutex = self.inner.thread_mutex(key);
        let inner = Arc::clone(&self.inner);

        let jh = tokio::spawn(async move {
            while let Some(turn) = rx.recv().await {
                let _guard = thread_mutex.lock().await;
                execute_queued_turn(&inner, turn).await;
            }
        });

        self.inner.worker_tasks.lock().push(jh);
        workers.insert(key, tx.clone());
        tx
    }

    fn cancel_active_turn(&self, turn_id: &str) {
        let handle = self.inner.active_turns.lock().get(turn_id).cloned();
        let Some(handle) = handle else {
            return;
        };
        let _ = handle.cancel_tx.send(true);
        let waiters: Vec<_> = {
            let mut map = handle.pending_approvals.lock();
            map.drain().map(|(_, tx)| tx).collect()
        };
        for tx in waiters {
            let _ = tx.send(CodingApprovalDecision::Deny);
        }
    }

    fn resolve_turn_approval(
        &self,
        turn_id: &str,
        request_id: &str,
        decision: CodingApprovalDecision,
    ) {
        let handle = self.inner.active_turns.lock().get(turn_id).cloned();
        let Some(handle) = handle else {
            return;
        };
        if let Some(tx) = handle.pending_approvals.lock().remove(request_id) {
            let _ = tx.send(decision);
        }
    }

    async fn handle_control_action(
        &self,
        req_id: String,
        key: ThreadKey,
        is_forum_topic: bool,
        config: CodingAgentBotConfig,
        action: CodingControlAction,
    ) -> CodingHostEvent {
        let is_mutating_workspace = matches!(
            action,
            CodingControlAction::SetWorkspace { .. }
                | CodingControlAction::RollbackCheckpoint { .. }
                | CodingControlAction::CommitAndPr { .. }
        ) || matches!(
            &action,
            CodingControlAction::WorktreeOp { op, .. }
                if matches!(
                    op.trim().to_ascii_lowercase().as_str(),
                    "remove" | "clean" | "delete" | "force_remove"
                )
        );

        if is_mutating_workspace && self.inner.has_active_turn_on_thread(key) {
            return control_err(
                req_id,
                "A coding turn is currently active on this thread. Run `/stop` first before modifying the workspace or rolling back.",
            );
        }

        match action {
            CodingControlAction::SetWorkspace { path } => {
                let canonical = match tokio::task::spawn_blocking(move || validate_workspace_dir(&path)).await {
                    Ok(Ok(p)) => p,
                    Ok(Err(e)) => return control_err(req_id, e),
                    Err(e) => return control_err(req_id, format!("workspace validation panicked: {e}")),
                };
                let canonical_str = canonical.to_string_lossy().into_owned();
                let mut rec = self.inner.load_or_init_thread(key, is_forum_topic, &config, &canonical_str);
                rec.workspace_path = canonical_str.clone();
                rec.worktree_path = None;
                rec.branch_name = is_git_repo(&canonical).then(|| current_branch(&canonical)).flatten();
                rec.provider_session_id = None;
                rec.updated_at_ms = now_ms();
                if let Err(e) = self.inner.store.upsert_thread(&rec) {
                    return control_err(req_id, e);
                }
                self.inner.thread_allow_rules(key).write().clear();

                CodingHostEvent::ControlResult {
                    req_id,
                    ok: true,
                    markdown_reply: format!("Workspace updated to `{canonical_str}` (session reset)."),
                    updated_workspace: Some(canonical_str),
                    updated_use_worktree: None,
                    updated_model: None,
                    updated_permission_mode: None,
                    handoff_summary: None,
                }
            }
            CodingControlAction::WorktreeOp { op, branch } => {
                let op_norm = op.trim().to_ascii_lowercase();
                let mut rec = self.inner.load_or_init_thread(
                    key,
                    is_forum_topic,
                    &config,
                    &config.workspace_path,
                );
                let base_ws = match validate_workspace_dir(&rec.workspace_path) {
                    Ok(p) => p,
                    Err(e) => return control_err(req_id, e),
                };
                let Some(repo_root) = git_repo_root(&base_ws) else {
                    return control_err(
                        req_id,
                        format!("Workspace `{}` is not a git repository.", base_ws.display()),
                    );
                };

                match op_norm.as_str() {
                    "list" | "status" => {
                        let entries = match tokio::task::spawn_blocking(move || list_worktrees(&repo_root)).await {
                            Ok(Ok(v)) => v,
                            Ok(Err(e)) => return control_err(req_id, e),
                            Err(e) => return control_err(req_id, format!("worktree list failed: {e}")),
                        };
                        let mut lines = vec![format!("**Worktrees ({})**:", entries.len())];
                        for e in entries {
                            let br = e.branch.as_deref().unwrap_or("detached");
                            lines.push(format!("- `{}` (`{br}`)", e.path.display()));
                        }
                        control_ok(req_id, lines.join("\n"))
                    }
                    "on" | "enable" | "create" | "new" | "ensure" => {
                        let wt_root = self.inner.config.worktrees_root.clone();
                        let slug = thread_worktree_slug_for_owner(
                            self.inner.config.owner_user_id,
                            key.bot_user_id,
                            key.chat_id,
                            (key.topic_id != 0).then_some(key.topic_id),
                        );
                        let wt_res = tokio::task::spawn_blocking(move || {
                            ensure_worktree(&repo_root, &wt_root, &slug, branch.as_deref())
                        })
                        .await;
                        let wt = match wt_res {
                            Ok(Ok(w)) => w,
                            Ok(Err(e)) => return control_err(req_id, e),
                            Err(e) => return control_err(req_id, format!("worktree creation failed: {e}")),
                        };
                        let wt_str = wt.path.to_string_lossy().into_owned();
                        rec.use_worktree = true;
                        rec.worktree_path = Some(wt_str.clone());
                        rec.branch_name = Some(wt.branch.clone());
                        rec.updated_at_ms = now_ms();
                        if let Err(e) = self.inner.store.upsert_thread(&rec) {
                            return control_err(req_id, e);
                        }
                        let verb = if wt.created { "Created" } else { "Reusing" };
                        CodingHostEvent::ControlResult {
                            req_id,
                            ok: true,
                            markdown_reply: format!("{verb} worktree `{wt_str}` on branch `{}`.", wt.branch),
                            updated_workspace: None,
                            updated_use_worktree: Some(true),
                            updated_model: None,
                            updated_permission_mode: None,
                            handoff_summary: None,
                        }
                    }
                    "off" | "disable" => {
                        let prev_wt = rec.worktree_path.take();
                        let mut note = String::new();
                        if let Some(ref wt_str) = prev_wt {
                            let wt_p = PathBuf::from(wt_str);
                            let rr = repo_root.clone();
                            if let Ok(Err(err)) = tokio::task::spawn_blocking(move || remove_worktree(&rr, &wt_p, false)).await {
                                note = format!(" (kept existing dirty worktree `{wt_str}`: {err})");
                            }
                        }
                        rec.use_worktree = false;
                        rec.branch_name = current_branch(&base_ws);
                        rec.updated_at_ms = now_ms();
                        if let Err(e) = self.inner.store.upsert_thread(&rec) {
                            return control_err(req_id, e);
                        }
                        CodingHostEvent::ControlResult {
                            req_id,
                            ok: true,
                            markdown_reply: format!("Worktree isolation disabled for this thread{note}."),
                            updated_workspace: None,
                            updated_use_worktree: Some(false),
                            updated_model: None,
                            updated_permission_mode: None,
                            handoff_summary: None,
                        }
                    }
                    "clean" | "remove" | "delete" | "force_remove" => {
                        let force = op_norm.contains("force");
                        let Some(wt_str) = rec.worktree_path.clone() else {
                            return control_err(req_id, "No active worktree is attached to this thread.");
                        };
                        let wt_p = PathBuf::from(&wt_str);
                        let rr = repo_root.clone();
                        match tokio::task::spawn_blocking(move || remove_worktree(&rr, &wt_p, force)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => return control_err(req_id, e),
                            Err(e) => return control_err(req_id, format!("worktree removal failed: {e}")),
                        }
                        rec.use_worktree = false;
                        rec.worktree_path = None;
                        rec.branch_name = current_branch(&base_ws);
                        rec.updated_at_ms = now_ms();
                        if let Err(e) = self.inner.store.upsert_thread(&rec) {
                            return control_err(req_id, e);
                        }
                        CodingHostEvent::ControlResult {
                            req_id,
                            ok: true,
                            markdown_reply: format!("Removed worktree `{wt_str}`."),
                            updated_workspace: None,
                            updated_use_worktree: Some(false),
                            updated_model: None,
                            updated_permission_mode: None,
                            handoff_summary: None,
                        }
                    }
                    _ => control_err(
                        req_id,
                        format!("Unsupported worktree op `{op}`. Use `on`, `off`, `new [branch]`, `list`, or `clean`."),
                    ),
                }
            }
            CodingControlAction::GetDiff { turn_idx } => {
                let Some(rec) = self.inner.load_reconciled_thread(key, is_forum_topic, &config) else {
                    return control_ok(req_id, "No coding turns recorded for this thread yet.");
                };
                let cwd = resolve_effective_cwd_readonly(&rec, &config.workspace_path);
                if !is_git_repo(&cwd) {
                    return control_err(
                        req_id,
                        format!("Workspace `{}` is not a git repository.", cwd.display()),
                    );
                }
                let cp = match self.inner.store.get_checkpoint(
                    self.inner.config.owner_user_id,
                    &rec.thread_id,
                    turn_idx,
                ) {
                    Ok(Some(c)) => c,
                    Ok(None) => {
                        return control_ok(req_id, "No checkpoint recorded for the requested turn.");
                    }
                    Err(e) => return control_err(req_id, e),
                };
                let cwd_clone = cwd.clone();
                let before = cp.before_ref.clone();
                let after = cp.after_ref.clone();
                let res = tokio::task::spawn_blocking(move || {
                    diff_checkpoints(&cwd_clone, &before, &after, DEFAULT_MAX_PATCH_BYTES)
                })
                .await;
                match res {
                    Ok(Ok((stat, patch))) => {
                        if stat.files_changed == 0 || patch.trim().is_empty() {
                            control_ok(req_id, format!("Turn #{} produced no git diff changes.", cp.turn_idx))
                        } else {
                            control_ok(
                                req_id,
                                format!(
                                    "**Turn #{} Diff** ({} file(s), +{} -{}):\n```diff\n{}\n```",
                                    cp.turn_idx,
                                    stat.files_changed,
                                    stat.insertions,
                                    stat.deletions,
                                    patch.trim_end()
                                ),
                            )
                        }
                    }
                    Ok(Err(e)) => control_err(req_id, e),
                    Err(e) => control_err(req_id, format!("diff failed: {e}")),
                }
            }
            CodingControlAction::RollbackCheckpoint { turn_idx } => {
                let Some(rec) = self.inner.load_reconciled_thread(key, is_forum_topic, &config) else {
                    return control_err(req_id, "No coding turns recorded for this thread yet.");
                };
                let cwd = resolve_effective_cwd_readonly(&rec, &config.workspace_path);
                if !is_git_repo(&cwd) {
                    return control_err(
                        req_id,
                        format!("Workspace `{}` is not a git repository.", cwd.display()),
                    );
                }
                let cp = match self.inner.store.get_checkpoint(
                    self.inner.config.owner_user_id,
                    &rec.thread_id,
                    turn_idx,
                ) {
                    Ok(Some(c)) => c,
                    Ok(None) => return control_err(req_id, "Requested checkpoint turn was not found."),
                    Err(e) => return control_err(req_id, e),
                };
                let target_ref = cp.before_ref.clone();
                let cwd_clone = cwd.clone();
                match tokio::task::spawn_blocking(move || restore_checkpoint(&cwd_clone, &target_ref)).await {
                    Ok(Ok(())) => control_ok(
                        req_id,
                        format!(
                            "Rolled back workspace `{}` to pre-turn #{} snapshot (`{}`). `HEAD` and commit history were left untouched.",
                            cwd.display(),
                            cp.turn_idx,
                            cp.before_ref
                        ),
                    ),
                    Ok(Err(e)) => control_err(req_id, e),
                    Err(e) => control_err(req_id, format!("rollback failed: {e}")),
                }
            }
            CodingControlAction::ListModels => {
                let rec = self.inner.load_reconciled_thread(key, is_forum_topic, &config);
                let active = rec
                    .as_ref()
                    .and_then(|r| r.active_model.clone())
                    .or_else(|| normalized_opt_str(config.model_override.as_deref()));
                let presets = model_suggestions_for(config.provider_kind);
                let active_label = active.as_deref().unwrap_or("(provider default)");
                let mut lines = vec![
                    format!("**{} Models** (current: `{active_label}`):", config.provider_kind.display_name()),
                ];
                for m in presets {
                    let marker = if active.as_deref() == Some(*m) { " *(active)*" } else { "" };
                    lines.push(format!("- `{m}`{marker}"));
                }
                lines.push("Use `/model <model_id>` to switch.".to_string());
                control_ok(req_id, lines.join("\n"))
            }
            CodingControlAction::SetModel { model } => {
                let trimmed = model.trim().to_string();
                if trimmed.is_empty() {
                    return control_err(req_id, "Model name cannot be empty.");
                }
                let mut rec = self.inner.load_or_init_thread(
                    key,
                    is_forum_topic,
                    &config,
                    &config.workspace_path,
                );
                rec.active_model = Some(trimmed.clone());
                rec.updated_at_ms = now_ms();
                if let Err(e) = self.inner.store.upsert_thread(&rec) {
                    return control_err(req_id, e);
                }
                CodingHostEvent::ControlResult {
                    req_id,
                    ok: true,
                    markdown_reply: format!("Active model set to `{trimmed}`."),
                    updated_workspace: None,
                    updated_use_worktree: None,
                    updated_model: Some(trimmed),
                    updated_permission_mode: None,
                    handoff_summary: None,
                }
            }
            CodingControlAction::SetPermissionMode { mode } => {
                let mut rec = self.inner.load_or_init_thread(
                    key,
                    is_forum_topic,
                    &config,
                    &config.workspace_path,
                );
                rec.permission_mode = mode;
                rec.updated_at_ms = now_ms();
                if let Err(e) = self.inner.store.upsert_thread(&rec) {
                    return control_err(req_id, e);
                }
                CodingHostEvent::ControlResult {
                    req_id,
                    ok: true,
                    markdown_reply: format!(
                        "Permission mode set to **{}** (`{}`).",
                        mode.label_en(),
                        mode.as_str()
                    ),
                    updated_workspace: None,
                    updated_use_worktree: None,
                    updated_model: None,
                    updated_permission_mode: Some(mode),
                    handoff_summary: None,
                }
            }
            CodingControlAction::CommitAndPr { message, title } => {
                let rec = self.inner.load_reconciled_thread(key, is_forum_topic, &config);
                let cwd = match rec.as_ref() {
                    Some(r) => resolve_effective_cwd_readonly(r, &config.workspace_path),
                    None => match validate_workspace_dir(&config.workspace_path) {
                        Ok(p) => p,
                        Err(e) => return control_err(req_id, e),
                    },
                };
                match commit_and_pr(&cwd, message.as_deref(), title.as_deref()).await {
                    Ok(reply) => control_ok(req_id, reply),
                    Err(e) => control_err(req_id, e),
                }
            }
            CodingControlAction::GetStatus => {
                let rec = self.inner.load_reconciled_thread(key, is_forum_topic, &config);
                let effective_ws = rec
                    .as_ref()
                    .map(|r| resolve_effective_cwd_readonly(r, &config.workspace_path))
                    .unwrap_or_else(|| PathBuf::from(&config.workspace_path));
                let is_git = is_git_repo(&effective_ws);
                let branch = if is_git {
                    current_branch(&effective_ws).unwrap_or_else(|| "detached".to_string())
                } else {
                    "n/a".to_string()
                };
                let dirty = if is_git {
                    count_dirty_entries(&effective_ws)
                } else {
                    0
                };
                let model = rec
                    .as_ref()
                    .and_then(|r| r.active_model.as_deref())
                    .or_else(|| {
                        config
                            .model_override
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or("(default)");
                let mode = rec
                    .as_ref()
                    .map(|r| r.permission_mode)
                    .unwrap_or(config.permission_mode);
                let use_wt = rec
                    .as_ref()
                    .map(|r| r.use_worktree)
                    .unwrap_or(config.use_worktree || is_forum_topic);
                let turns = rec.as_ref().map(|r| r.turn_count).unwrap_or(0);
                let sess = rec
                    .as_ref()
                    .and_then(|r| r.provider_session_id.as_deref())
                    .unwrap_or("(none)");
                let wt_str = rec
                    .as_ref()
                    .and_then(|r| r.worktree_path.as_deref())
                    .unwrap_or("(none)");

                let reply = format!(
                    "**Coding Agent Status**\n\
                     - Device: `{}`\n\
                     - Provider: **{}** (`{}`)\n\
                     - Workspace: `{}`\n\
                     - Worktree: `{wt_str}` (enabled: `{use_wt}`)\n\
                     - Git branch: `{branch}` (dirty entries: `{dirty}`)\n\
                     - Model: `{model}` | Mode: `{}`\n\
                     - Turns: `{turns}` | Session: `{sess}`",
                    self.inner.config.device_id,
                    config.provider_kind.display_name(),
                    config.provider_kind.as_str(),
                    effective_ws.display(),
                    mode.as_str(),
                );
                control_ok(req_id, reply)
            }
            CodingControlAction::CheckLogin => {
                let kind = config.provider_kind;
                let ov = config.executable_override.clone();
                let res = tokio::task::spawn_blocking(move || {
                    check_provider_login_status(kind, ov.as_deref())
                })
                .await;
                match res {
                    Ok((ok, msg)) => CodingHostEvent::ControlResult {
                        req_id,
                        ok,
                        markdown_reply: msg,
                        updated_workspace: None,
                        updated_use_worktree: None,
                        updated_model: None,
                        updated_permission_mode: None,
                        handoff_summary: None,
                    },
                    Err(e) => control_err(req_id, format!("login check failed: {e}")),
                }
            }
            CodingControlAction::ResetSession => {
                if let Some(mut rec) = self.inner.load_reconciled_thread(key, is_forum_topic, &config) {
                    rec.provider_session_id = None;
                    rec.updated_at_ms = now_ms();
                    if let Err(e) = self.inner.store.upsert_thread(&rec) {
                        return control_err(req_id, e);
                    }
                }
                self.inner.thread_allow_rules(key).write().clear();
                control_ok(
                    req_id,
                    "Session cleared. The next message will start a fresh provider session.",
                )
            }
            CodingControlAction::BuildHandoffSummary { target_bot_username } => {
                let rec = self.inner.load_reconciled_thread(key, is_forum_topic, &config);
                let effective_ws = rec
                    .as_ref()
                    .map(|r| resolve_effective_cwd_readonly(r, &config.workspace_path))
                    .unwrap_or_else(|| PathBuf::from(&config.workspace_path));
                let branch = if is_git_repo(&effective_ws) {
                    current_branch(&effective_ws).unwrap_or_else(|| "detached".to_string())
                } else {
                    "n/a".to_string()
                };
                let dirty = if is_git_repo(&effective_ws) {
                    count_dirty_entries(&effective_ws)
                } else {
                    0
                };
                let checkpoints = rec
                    .as_ref()
                    .and_then(|r| {
                        self.inner
                            .store
                            .list_recent_checkpoints(
                                self.inner.config.owner_user_id,
                                &r.thread_id,
                                5,
                            )
                            .ok()
                    })
                    .unwrap_or_default();

                let target_clean = target_bot_username.trim().trim_start_matches('@');
                let mut lines = vec![
                    format!("**Handoff Context for @{target_clean}** (from {})", config.provider_kind.display_name()),
                    format!("- Workspace: `{}`", effective_ws.display()),
                    format!("- Branch: `{branch}` (dirty entries: `{dirty}`)"),
                ];
                if let Some(ref r) = rec {
                    if let Some(ref summary) = r.last_summary {
                        if !summary.is_empty() {
                            lines.push(format!("- Latest summary: {summary}"));
                        }
                    }
                }
                if !checkpoints.is_empty() {
                    lines.push("- Recent turns:".to_string());
                    for cp in checkpoints {
                        lines.push(format!(
                            "  - Turn #{}: \"{}\" -> {} file(s) changed (+{} -{})",
                            cp.turn_idx,
                            cp.prompt_preview,
                            cp.diff_stat.files_changed,
                            cp.diff_stat.insertions,
                            cp.diff_stat.deletions
                        ));
                    }
                }
                let summary = lines.join("\n");
                CodingHostEvent::ControlResult {
                    req_id,
                    ok: true,
                    markdown_reply: summary.clone(),
                    updated_workspace: None,
                    updated_use_worktree: None,
                    updated_model: None,
                    updated_permission_mode: None,
                    handoff_summary: Some(summary),
                }
            }
        }
    }
}

impl Drop for LocalCodingHostRuntime {
    fn drop(&mut self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let active: Vec<Arc<ActiveTurnHandle>> = {
            let guard = self.inner.active_turns.lock();
            guard.values().cloned().collect()
        };
        for handle in active {
            let _ = handle.cancel_tx.send(true);
            let waiters: Vec<_> = {
                let mut map = handle.pending_approvals.lock();
                map.drain().map(|(_, tx)| tx).collect()
            };
            for tx in waiters {
                let _ = tx.send(CodingApprovalDecision::Deny);
            }
        }
        self.inner.thread_workers.lock().clear();
        let tasks = std::mem::take(&mut *self.inner.worker_tasks.lock());
        for task in tasks {
            task.abort();
        }
    }
}

impl RuntimeInner {
    fn thread_mutex(&self, key: ThreadKey) -> Arc<tokio::sync::Mutex<()>> {
        let mut guard = self.thread_mutexes.lock();
        Arc::clone(
            guard
                .entry(key)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    }

    fn thread_allow_rules(&self, key: ThreadKey) -> Arc<RwLock<HashSet<String>>> {
        let mut guard = self.thread_allow_rules.lock();
        Arc::clone(
            guard
                .entry(key)
                .or_insert_with(|| Arc::new(RwLock::new(HashSet::new()))),
        )
    }

    fn has_active_turn_on_thread(&self, key: ThreadKey) -> bool {
        self.active_turns
            .lock()
            .values()
            .any(|h| h.thread_key == key)
    }

    fn reconcile_thread_with_bot_config(
        &self,
        rec: &mut ThreadRecord,
        is_forum_topic: bool,
        config: &CodingAgentBotConfig,
    ) -> bool {
        let mut dirty = false;
        let mut clear_allow_rules = false;
        let is_legacy_uninitialized = rec.bot_permission_mode.is_none();

        if rec.bot_provider_kind != Some(config.provider_kind) {
            if rec.bot_provider_kind.is_some() {
                rec.provider_session_id = None;
                clear_allow_rules = true;
            }
            rec.bot_provider_kind = Some(config.provider_kind);
            dirty = true;
        }

        if rec.bot_workspace_path != config.workspace_path {
            let same_canonical_ws = !rec.workspace_path.trim().is_empty()
                && !config.workspace_path.trim().is_empty()
                && (rec.workspace_path == config.workspace_path
                    || std::fs::canonicalize(&rec.workspace_path).ok()
                        == std::fs::canonicalize(&config.workspace_path).ok());

            if !same_canonical_ws {
                rec.workspace_path = config.workspace_path.clone();
                rec.worktree_path = None;
                rec.branch_name = None;
                rec.provider_session_id = None;
                clear_allow_rules = true;
            }
            rec.bot_workspace_path = config.workspace_path.clone();
            dirty = true;
        }

        if rec.bot_use_worktree != config.use_worktree || is_legacy_uninitialized {
            rec.bot_use_worktree = config.use_worktree;
            rec.use_worktree = config.use_worktree || is_forum_topic;
            if !rec.use_worktree {
                rec.worktree_path = None;
                rec.branch_name = None;
            }
            dirty = true;
        }

        let incoming_model = normalized_opt_str(config.model_override.as_deref());
        if rec.bot_active_model != incoming_model || is_legacy_uninitialized {
            if !is_legacy_uninitialized || incoming_model.is_some() || rec.active_model.is_none() {
                rec.active_model = incoming_model.clone();
            }
            rec.bot_active_model = incoming_model;
            dirty = true;
        }

        if rec.bot_permission_mode != Some(config.permission_mode) {
            rec.bot_permission_mode = Some(config.permission_mode);
            rec.permission_mode = config.permission_mode;
            dirty = true;
        }

        if clear_allow_rules {
            let key = ThreadKey {
                owner_user_id: rec.owner_user_id,
                bot_user_id: rec.bot_user_id,
                chat_id: rec.chat_id,
                topic_id: rec.topic_id,
            };
            self.thread_allow_rules(key).write().clear();
        }

        dirty
    }

    fn load_reconciled_thread(
        &self,
        key: ThreadKey,
        is_forum_topic: bool,
        config: &CodingAgentBotConfig,
    ) -> Option<ThreadRecord> {
        let mut rec = self.store.get_thread(key).ok().flatten()?;
        if rec.owner_user_id != self.config.owner_user_id {
            return None;
        }
        if self.reconcile_thread_with_bot_config(&mut rec, is_forum_topic, config) {
            rec.updated_at_ms = now_ms();
            let _ = self.store.upsert_thread(&rec);
        }
        Some(rec)
    }

    fn load_or_init_thread(
        &self,
        key: ThreadKey,
        is_forum_topic: bool,
        config: &CodingAgentBotConfig,
        workspace_fallback: &str,
    ) -> ThreadRecord {
        if let Some(rec) = self.load_reconciled_thread(key, is_forum_topic, config) {
            return rec;
        }
        let bot_model = normalized_opt_str(config.model_override.as_deref());
        ThreadRecord {
            owner_user_id: key.owner_user_id,
            bot_user_id: key.bot_user_id,
            chat_id: key.chat_id,
            topic_id: key.topic_id,
            thread_id: default_thread_id(key),
            provider_session_id: None,
            workspace_path: workspace_fallback.to_string(),
            worktree_path: None,
            branch_name: None,
            use_worktree: config.use_worktree || is_forum_topic,
            active_model: bot_model.clone(),
            permission_mode: config.permission_mode,
            turn_count: 0,
            last_summary: None,
            updated_at_ms: now_ms(),
            bot_provider_kind: Some(config.provider_kind),
            bot_workspace_path: config.workspace_path.clone(),
            bot_use_worktree: config.use_worktree,
            bot_active_model: bot_model,
            bot_permission_mode: Some(config.permission_mode),
        }
    }
}

async fn execute_queued_turn(inner: &Arc<RuntimeInner>, turn: QueuedTurn) {
    let turn_id = turn.turn_id.clone();
    let emit = turn.emit.clone();

    if *turn.cancel_rx.borrow() {
        let ev = CodingHostEvent::TurnCompleted {
            turn_id: turn_id.clone(),
            ok: false,
            final_text: String::new(),
            error: Some("Turn cancelled before starting".to_string()),
            turn_idx: 0,
            checkpoint_ref: None,
            diff_stat: None,
            token_usage: None,
        };
        let _ = inner
            .store
            .finish_turn_receipt(inner.config.owner_user_id, &turn_id, "cancelled", 0, Some(&ev));
        inner.active_turns.lock().remove(&turn_id);
        let _ = emit.send(ev);
        return;
    }

    let mut rec = inner.load_or_init_thread(
        turn.key,
        turn.is_forum_topic,
        &turn.config,
        &turn.config.workspace_path,
    );
    let raw_ws = if rec.workspace_path.trim().is_empty() {
        turn.config.workspace_path.clone()
    } else {
        rec.workspace_path.clone()
    };

    let base_ws = match tokio::task::spawn_blocking(move || validate_workspace_dir(&raw_ws)).await {
        Ok(Ok(p)) => p,
        Ok(Err(err)) => {
            finish_turn_failure(inner, &turn_id, rec.turn_count, err, &emit);
            return;
        }
        Err(err) => {
            finish_turn_failure(
                inner,
                &turn_id,
                rec.turn_count,
                format!("workspace validation failed: {err}"),
                &emit,
            );
            return;
        }
    };

    let base_ws_str = base_ws.to_string_lossy().into_owned();
    rec.workspace_path = base_ws_str.clone();

    let effective_use_worktree = rec.use_worktree;

    let mut effective_cwd = base_ws.clone();
    let mut worktree_path_opt: Option<String> = None;
    let mut branch_name_opt: Option<String> = None;

    if effective_use_worktree {
        if let Some(repo_root) = git_repo_root(&base_ws) {
            let wt_root = inner.config.worktrees_root.clone();
            let slug = thread_worktree_slug_for_owner(
                inner.config.owner_user_id,
                turn.key.bot_user_id,
                turn.key.chat_id,
                (turn.key.topic_id != 0).then_some(turn.key.topic_id),
            );
            let preferred_branch = rec.branch_name.clone();
            let wt_res = tokio::task::spawn_blocking(move || {
                ensure_worktree(&repo_root, &wt_root, &slug, preferred_branch.as_deref())
            })
            .await;

            match wt_res {
                Ok(Ok(wt)) => {
                    let wt_str = wt.path.to_string_lossy().into_owned();
                    effective_cwd = wt.path;
                    worktree_path_opt = Some(wt_str);
                    branch_name_opt = Some(wt.branch);
                }
                Ok(Err(err)) => {
                    finish_turn_failure(
                        inner,
                        &turn_id,
                        rec.turn_count,
                        format!("worktree setup failed: {err}"),
                        &emit,
                    );
                    return;
                }
                Err(err) => {
                    finish_turn_failure(
                        inner,
                        &turn_id,
                        rec.turn_count,
                        format!("worktree task failed: {err}"),
                        &emit,
                    );
                    return;
                }
            }
        }
    } else {
        branch_name_opt = is_git_repo(&effective_cwd)
            .then(|| current_branch(&effective_cwd))
            .flatten();
    }

    rec.worktree_path = worktree_path_opt.clone();
    rec.branch_name = branch_name_opt.clone();
    let turn_idx = rec.turn_count + 1;
    let effective_model = rec.active_model.clone();
    let effective_perm = rec.permission_mode;

    if let Err(err) = inner.store.upsert_thread(&rec) {
        finish_turn_failure(inner, &turn_id, rec.turn_count, err, &emit);
        return;
    }

    let before_ref = format!(
        "refs/fabushi-coding/checkpoints/{}/{}-before",
        rec.thread_id, turn_idx
    );
    let after_ref = format!(
        "refs/fabushi-coding/checkpoints/{}/{}",
        rec.thread_id, turn_idx
    );

    let cwd_for_before = effective_cwd.clone();
    let before_ref_clone = before_ref.clone();
    let has_git_checkpoint = tokio::task::spawn_blocking(move || {
        if !is_git_repo(&cwd_for_before) {
            return Ok(false);
        }
        capture_checkpoint(&cwd_for_before, &before_ref_clone).map(|_| true)
    })
    .await
    .unwrap_or_else(|e| Err(format!("checkpoint task error: {e}")));

    let git_checkpoint_active = match has_git_checkpoint {
        Ok(b) => b,
        Err(err) => {
            tracing::warn!(turn_id = %turn_id, error = %err, "pre-turn checkpoint skipped");
            false
        }
    };

    let driver_req = DriverTurnRequest {
        turn_id: turn_id.clone(),
        turn_idx,
        prompt: turn.prompt.clone(),
        cwd: effective_cwd.clone(),
        provider_kind: turn.config.provider_kind,
        permission_mode: effective_perm,
        provider_session_id: rec.provider_session_id.clone(),
        model: effective_model,
        executable_override: turn.config.executable_override.clone(),
        extra_args: turn.config.extra_args.clone(),
        workspace_path_for_event: base_ws_str,
        worktree_path_for_event: worktree_path_opt,
        git_branch_for_event: branch_name_opt,
    };

    let mut driver_ctx = DriverContext {
        turn_id: turn_id.clone(),
        turn_idx,
        permission_mode: effective_perm,
        emit: emit.clone(),
        cancel_rx: turn.cancel_rx,
        pending_approvals: turn.pending_approvals,
        session_allow_rules: inner.thread_allow_rules(turn.key),
    };

    let output = execute_driver_turn(&driver_req, &mut driver_ctx).await;

    let (checkpoint_ref, diff_stat) = if git_checkpoint_active {
        let cwd_after = effective_cwd.clone();
        let b_ref = before_ref.clone();
        let a_ref = after_ref.clone();
        let cp_res = tokio::task::spawn_blocking(move || -> Result<(String, CodingDiffStat), String> {
            capture_checkpoint(&cwd_after, &a_ref)?;
            let (stat, _patch) = diff_checkpoints(&cwd_after, &b_ref, &a_ref, DEFAULT_MAX_PATCH_BYTES)?;
            Ok((a_ref, stat))
        })
        .await;

        match cp_res {
            Ok(Ok((a_ref, stat))) => {
                let cp_record = CheckpointRecord {
                    owner_user_id: inner.config.owner_user_id,
                    thread_id: rec.thread_id.clone(),
                    turn_idx,
                    before_ref: before_ref.clone(),
                    after_ref: a_ref.clone(),
                    diff_stat: stat.clone(),
                    prompt_preview: preview_text(&turn.prompt, 120),
                    reply_preview: preview_text(&output.final_text, 200),
                    created_at_ms: now_ms(),
                };
                let _ = inner.store.save_checkpoint(&cp_record);
                (Some(a_ref), Some(stat))
            }
            Ok(Err(err)) => {
                tracing::warn!(turn_id = %turn_id, error = %err, "post-turn checkpoint failed");
                (None, None)
            }
            Err(err) => {
                tracing::warn!(turn_id = %turn_id, error = %err, "post-turn checkpoint panicked");
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    rec.turn_count = turn_idx;
    if output.provider_session_id.is_some() {
        rec.provider_session_id = output.provider_session_id.clone();
    }
    if !output.final_text.trim().is_empty() {
        rec.last_summary = Some(preview_text(&output.final_text, 240));
    }
    rec.updated_at_ms = now_ms();
    let _ = inner.store.upsert_thread(&rec);

    let receipt_status = if output.cancelled {
        "cancelled"
    } else if output.ok {
        "completed"
    } else {
        "failed"
    };

    let completed_ev = CodingHostEvent::TurnCompleted {
        turn_id: turn_id.clone(),
        ok: output.ok,
        final_text: output.final_text,
        error: output.error,
        turn_idx,
        checkpoint_ref,
        diff_stat,
        token_usage: output.token_usage,
    };

    let _ = inner.store.finish_turn_receipt(
        inner.config.owner_user_id,
        &turn_id,
        receipt_status,
        turn_idx,
        Some(&completed_ev),
    );
    inner.active_turns.lock().remove(&turn_id);
    let _ = emit.send(completed_ev);
}

fn finish_turn_failure(
    inner: &Arc<RuntimeInner>,
    turn_id: &str,
    turn_idx: u64,
    err: String,
    emit: &mpsc::UnboundedSender<CodingHostEvent>,
) {
    let ev = CodingHostEvent::TurnCompleted {
        turn_id: turn_id.to_string(),
        ok: false,
        final_text: String::new(),
        error: Some(err),
        turn_idx,
        checkpoint_ref: None,
        diff_stat: None,
        token_usage: None,
    };
    let _ = inner.store.finish_turn_receipt(
        inner.config.owner_user_id,
        turn_id,
        "failed",
        turn_idx,
        Some(&ev),
    );
    inner.active_turns.lock().remove(turn_id);
    let _ = emit.send(ev);
}

fn resolve_effective_cwd_readonly(rec: &ThreadRecord, fallback_ws: &str) -> PathBuf {
    if let Some(ref wt) = rec.worktree_path {
        let p = Path::new(wt);
        if p.is_dir() {
            return p.to_path_buf();
        }
    }
    if !rec.workspace_path.trim().is_empty() {
        return PathBuf::from(&rec.workspace_path);
    }
    PathBuf::from(fallback_ws)
}

fn default_thread_id(key: ThreadKey) -> String {
    format!(
        "thr_u{}_{}_{}_{}",
        key.owner_user_id, key.bot_user_id, key.chat_id, key.topic_id
    )
}

fn command_owner_and_target(cmd: &CodingHostCommand) -> (u64, Option<&str>) {
    match cmd {
        CodingHostCommand::ProbeProviders {
            owner_user_id,
            target_device_id,
            ..
        }
        | CodingHostCommand::StartTurn {
            owner_user_id,
            target_device_id,
            ..
        }
        | CodingHostCommand::CancelTurn {
            owner_user_id,
            target_device_id,
            ..
        }
        | CodingHostCommand::ResolveApproval {
            owner_user_id,
            target_device_id,
            ..
        }
        | CodingHostCommand::Control {
            owner_user_id,
            target_device_id,
            ..
        } => (*owner_user_id, target_device_id.as_deref()),
    }
}

fn emit_owner_mismatch_rejection(
    cmd: &CodingHostCommand,
    expected_owner: u64,
    device_id: &str,
    emit: &mpsc::UnboundedSender<CodingHostEvent>,
) {
    match cmd {
        CodingHostCommand::ProbeProviders {
            req_id,
            owner_user_id,
            ..
        } => {
            tracing::warn!(
                req_id = %req_id,
                expected_owner,
                actual_owner = *owner_user_id,
                "rejected ProbeProviders owner mismatch"
            );
            let _ = emit.send(CodingHostEvent::ProbeResult {
                req_id: req_id.clone(),
                device_id: device_id.to_string(),
                providers: Vec::new(),
            });
        }
        CodingHostCommand::StartTurn {
            turn_id,
            owner_user_id,
            ..
        } => {
            let _ = emit.send(CodingHostEvent::TurnCompleted {
                turn_id: turn_id.clone(),
                ok: false,
                final_text: String::new(),
                error: Some(format!(
                    "Unauthorized: owner_user_id mismatch (got {owner_user_id}, expected {expected_owner})"
                )),
                turn_idx: 0,
                checkpoint_ref: None,
                diff_stat: None,
                token_usage: None,
            });
        }
        CodingHostCommand::Control {
            req_id,
            owner_user_id,
            ..
        } => {
            let _ = emit.send(CodingHostEvent::ControlResult {
                req_id: req_id.clone(),
                ok: false,
                markdown_reply: format!(
                    "🔒 Unauthorized: owner_user_id mismatch (got {owner_user_id}, expected {expected_owner})."
                ),
                updated_workspace: None,
                updated_use_worktree: None,
                updated_model: None,
                updated_permission_mode: None,
                handoff_summary: None,
            });
        }
        CodingHostCommand::CancelTurn { turn_id, .. }
        | CodingHostCommand::ResolveApproval { turn_id, .. } => {
            let _ = emit.send(CodingHostEvent::TurnActivity {
                turn_id: turn_id.clone(),
                state: "rejected".to_string(),
                detail: Some("Unauthorized: owner_user_id mismatch".to_string()),
            });
        }
    }
}

fn emit_shutdown_rejection(
    cmd: &CodingHostCommand,
    device_id: &str,
    emit: &mpsc::UnboundedSender<CodingHostEvent>,
) {
    match cmd {
        CodingHostCommand::ProbeProviders { req_id, .. } => {
            let _ = emit.send(CodingHostEvent::ProbeResult {
                req_id: req_id.clone(),
                device_id: device_id.to_string(),
                providers: Vec::new(),
            });
        }
        CodingHostCommand::StartTurn { turn_id, .. } => {
            let _ = emit.send(CodingHostEvent::TurnCompleted {
                turn_id: turn_id.clone(),
                ok: false,
                final_text: String::new(),
                error: Some("Coding host runtime is shutting down".to_string()),
                turn_idx: 0,
                checkpoint_ref: None,
                diff_stat: None,
                token_usage: None,
            });
        }
        CodingHostCommand::Control { req_id, .. } => {
            let _ = emit.send(control_err(
                req_id.clone(),
                "Coding host runtime is shutting down.",
            ));
        }
        CodingHostCommand::CancelTurn { .. } | CodingHostCommand::ResolveApproval { .. } => {}
    }
}

fn control_ok(req_id: String, markdown_reply: impl Into<String>) -> CodingHostEvent {
    CodingHostEvent::ControlResult {
        req_id,
        ok: true,
        markdown_reply: markdown_reply.into(),
        updated_workspace: None,
        updated_use_worktree: None,
        updated_model: None,
        updated_permission_mode: None,
        handoff_summary: None,
    }
}

fn control_err(req_id: String, markdown_reply: impl Into<String>) -> CodingHostEvent {
    CodingHostEvent::ControlResult {
        req_id,
        ok: false,
        markdown_reply: markdown_reply.into(),
        updated_workspace: None,
        updated_use_worktree: None,
        updated_model: None,
        updated_permission_mode: None,
        handoff_summary: None,
    }
}

fn normalized_opt_str(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn model_suggestions_for(kind: CodingProviderKind) -> &'static [&'static str] {
    match kind {
        CodingProviderKind::ClaudeCode => &[
            "claude-sonnet-4-6",
            "claude-opus-4-6",
            "claude-haiku-4-5",
        ],
        CodingProviderKind::Codex => &["gpt-5.4", "gpt-5.3-codex", "o3"],
        CodingProviderKind::OpenCode => &["anthropic/claude-sonnet-4-6", "openai/gpt-5.4"],
        CodingProviderKind::CursorCli => &["auto", "claude-4.5-sonnet", "gpt-5"],
        CodingProviderKind::Antigravity => &["gemini-2.5-pro", "claude-sonnet-4-6"],
        CodingProviderKind::AcpCustom => &["default"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_telegram_proto::CodingPermissionMode;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_git_repo(dir: &Path) {
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
        std::fs::write(dir.join("app.txt"), "v1\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "initial commit"]);
    }

    #[cfg(unix)]
    fn write_mock_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[tokio::test]
    async fn runtime_rejects_owner_mismatch_on_all_commands() {
        let tmp = TempDir::new().unwrap();
        let rt = LocalCodingHostRuntime::new(CodingHostRuntimeConfig {
            owner_user_id: 1001,
            device_id: "mac-1".into(),
            db_path: tmp.path().join("db/host.db"),
            worktrees_root: tmp.path().join("wt"),
        })
        .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();

        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "t_bad".into(),
                owner_user_id: 9999,
                target_device_id: None,
                bot_user_id: 800_001,
                chat_id: 1,
                topic_id: None,
                is_forum_topic: false,
                prompt: "rm -rf /".into(),
                config: CodingAgentBotConfig::default(),
            },
            tx.clone(),
        )
        .await;

        match rx.recv().await.unwrap() {
            CodingHostEvent::TurnCompleted { turn_id, ok, error, .. } => {
                assert_eq!(turn_id, "t_bad");
                assert!(!ok);
                assert!(error.unwrap().contains("owner_user_id mismatch"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        rt.handle_command(
            CodingHostCommand::Control {
                req_id: "c_bad".into(),
                owner_user_id: 9999,
                target_device_id: None,
                bot_user_id: 800_001,
                chat_id: 1,
                topic_id: None,
                is_forum_topic: false,
                config: CodingAgentBotConfig::default(),
                action: CodingControlAction::GetStatus,
            },
            tx,
        )
        .await;

        match rx.recv().await.unwrap() {
            CodingHostEvent::ControlResult { req_id, ok, markdown_reply, .. } => {
                assert_eq!(req_id, "c_bad");
                assert!(!ok);
                assert!(markdown_reply.contains("Unauthorized"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        rt.shutdown().await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn runtime_auto_creates_worktree_for_forum_topic_handles_allow_session_diff_rollback_and_controls() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        init_git_repo(&repo_dir);

        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let mock_acp = write_mock_script(
            &bin_dir,
            "mock_acp.py",
            r#"#!/usr/bin/env python3
import json, os, sys

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
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"sessionId": "s_rt_1"}}), flush=True)
    elif method == "session/load":
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"sessionId": msg["params"]["sessionId"]}}), flush=True)
    elif method == "session/prompt":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": 500,
            "method": "session/request_permission",
            "params": {
                "sessionId": "s_rt_1",
                "toolCall": {"toolCallId": "tc1", "title": "Write app.txt", "kind": "execute", "rawInput": {"file": "app.txt"}},
                "options": [
                    {"optionId": "allow_once", "name": "Once", "kind": "allow_once"},
                    {"optionId": "allow_always", "name": "Session", "kind": "allow_always"},
                    {"optionId": "deny", "name": "Deny", "kind": "reject_once"}
                ]
            }
        }), flush=True)
        resp = json.loads(sys.stdin.readline())
        opt = resp["result"]["outcome"]["optionId"]
        with open("app.txt", "w") as f:
            f.write(f"v2:{opt}\n")
        print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s_rt_1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": f"updated with {opt}"}}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"stopReason": "end_turn"}}), flush=True)
"#,
        );

        let rt = LocalCodingHostRuntime::new(CodingHostRuntimeConfig {
            owner_user_id: 42,
            device_id: "mac-studio".into(),
            db_path: tmp.path().join("db/coding.db"),
            worktrees_root: tmp.path().join("worktrees"),
        })
        .unwrap();

        let cfg = CodingAgentBotConfig {
            provider_kind: CodingProviderKind::AcpCustom,
            workspace_path: repo_dir.to_string_lossy().into_owned(),
            use_worktree: false, // should auto-enable because is_forum_topic == true!
            permission_mode: CodingPermissionMode::Ask,
            executable_override: Some(mock_acp.to_string_lossy().into_owned()),
            ..Default::default()
        };

        let (tx, mut rx) = mpsc::unbounded_channel();

        // Turn 1: triggers approval -> respond with AllowSession.
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "turn_1".into(),
                owner_user_id: 42,
                target_device_id: Some("mac-studio".into()),
                bot_user_id: 800_005,
                chat_id: -100_999,
                topic_id: Some(77),
                is_forum_topic: true,
                prompt: "update app.txt".into(),
                config: cfg.clone(),
            },
            tx.clone(),
        )
        .await;

        let mut worktree_dir: Option<PathBuf> = None;
        let mut turn_1_completed = None;

        while let Some(ev) = rx.recv().await {
            match ev {
                CodingHostEvent::TurnStarted {
                    worktree_path,
                    git_branch,
                    turn_idx,
                    ..
                } => {
                    assert_eq!(turn_idx, 1);
                    let wt = worktree_path.expect("forum topic must auto-enable worktree");
                    assert!(git_branch.unwrap().contains("fabushi/"));
                    worktree_dir = Some(PathBuf::from(wt));
                }
                CodingHostEvent::TurnApprovalRequested {
                    turn_id,
                    request_id,
                    ..
                } => {
                    rt.handle_command(
                        CodingHostCommand::ResolveApproval {
                            turn_id,
                            request_id,
                            owner_user_id: 42,
                            target_device_id: None,
                            decision: CodingApprovalDecision::AllowSession,
                        },
                        tx.clone(),
                    )
                    .await;
                }
                CodingHostEvent::TurnCompleted {
                    ok,
                    final_text,
                    diff_stat,
                    checkpoint_ref,
                    ..
                } => {
                    assert!(ok);
                    assert_eq!(final_text, "updated with allow_always");
                    assert!(checkpoint_ref.unwrap().ends_with("/1"));
                    let ds = diff_stat.unwrap();
                    assert_eq!(ds.files_changed, 1);
                    assert_eq!(ds.changed_files, vec!["app.txt".to_string()]);
                    turn_1_completed = Some(final_text);
                    break;
                }
                _ => {}
            }
        }
        assert!(turn_1_completed.is_some());
        let wt_path = worktree_dir.unwrap();
        assert_eq!(
            std::fs::read_to_string(wt_path.join("app.txt")).unwrap(),
            "v2:allow_always\n"
        );
        // Primary repo directory remained untouched!
        assert_eq!(
            std::fs::read_to_string(repo_dir.join("app.txt")).unwrap(),
            "v1\n"
        );

        // Idempotent retry of turn_1 must replay cached TurnCompleted without re-running!
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "turn_1".into(),
                owner_user_id: 42,
                target_device_id: Some("mac-studio".into()),
                bot_user_id: 800_005,
                chat_id: -100_999,
                topic_id: Some(77),
                is_forum_topic: true,
                prompt: "update app.txt".into(),
                config: cfg.clone(),
            },
            tx.clone(),
        )
        .await;
        match rx.recv().await.unwrap() {
            CodingHostEvent::TurnCompleted {
                turn_id,
                ok,
                turn_idx,
                ..
            } => {
                assert_eq!(turn_id, "turn_1");
                assert!(ok);
                assert_eq!(turn_idx, 1);
            }
            other => panic!("expected replayed TurnCompleted, got {other:?}"),
        }

        // Turn 2: same rule_key is auto-approved via AllowSession without prompting user!
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "turn_2".into(),
                owner_user_id: 42,
                target_device_id: None,
                bot_user_id: 800_005,
                chat_id: -100_999,
                topic_id: Some(77),
                is_forum_topic: true,
                prompt: "second pass".into(),
                config: cfg.clone(),
            },
            tx.clone(),
        )
        .await;
        loop {
            match rx.recv().await.unwrap() {
                CodingHostEvent::TurnApprovalRequested { .. } => {
                    panic!("Turn 2 should have been auto-approved by AllowSession rule!");
                }
                CodingHostEvent::TurnCompleted { ok, turn_idx, .. } => {
                    assert!(ok);
                    assert_eq!(turn_idx, 2);
                    break;
                }
                _ => {}
            }
        }

        // Control: GetDiff(Some(1)), RollbackCheckpoint(Some(1)), SetModel, SetPermissionMode, BuildHandoffSummary
        rt.handle_command(
            CodingHostCommand::Control {
                req_id: "ctrl_diff".into(),
                owner_user_id: 42,
                target_device_id: None,
                bot_user_id: 800_005,
                chat_id: -100_999,
                topic_id: Some(77),
                is_forum_topic: true,
                config: cfg.clone(),
                action: CodingControlAction::GetDiff { turn_idx: Some(1) },
            },
            tx.clone(),
        )
        .await;
        match rx.recv().await.unwrap() {
            CodingHostEvent::ControlResult { ok, markdown_reply, .. } => {
                assert!(ok);
                assert!(markdown_reply.contains("app.txt"));
                assert!(markdown_reply.contains("v2:allow_always"));
            }
            other => panic!("unexpected: {other:?}"),
        }

        rt.handle_command(
            CodingHostCommand::Control {
                req_id: "ctrl_rb".into(),
                owner_user_id: 42,
                target_device_id: None,
                bot_user_id: 800_005,
                chat_id: -100_999,
                topic_id: Some(77),
                is_forum_topic: true,
                config: cfg.clone(),
                action: CodingControlAction::RollbackCheckpoint { turn_idx: Some(1) },
            },
            tx.clone(),
        )
        .await;
        match rx.recv().await.unwrap() {
            CodingHostEvent::ControlResult { ok, .. } => assert!(ok),
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(wt_path.join("app.txt")).unwrap(), "v1\n");

        rt.handle_command(
            CodingHostCommand::Control {
                req_id: "ctrl_handoff".into(),
                owner_user_id: 42,
                target_device_id: None,
                bot_user_id: 800_005,
                chat_id: -100_999,
                topic_id: Some(77),
                is_forum_topic: true,
                config: cfg,
                action: CodingControlAction::BuildHandoffSummary {
                    target_bot_username: "codex_bot".into(),
                },
            },
            tx,
        )
        .await;
        match rx.recv().await.unwrap() {
            CodingHostEvent::ControlResult {
                ok,
                handoff_summary,
                ..
            } => {
                assert!(ok);
                let summary = handoff_summary.unwrap();
                assert!(summary.contains("@codex_bot"));
                assert!(summary.contains("Turn #1"));
            }
            other => panic!("unexpected: {other:?}"),
        }

        rt.shutdown().await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn runtime_serializes_same_thread_parallelizes_distinct_topics_cancels_and_runs_all_controls() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("repo");
        let alt_dir = tmp.path().join("alt_ws");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(&alt_dir).unwrap();
        init_git_repo(&repo_dir);

        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let mock_acp = write_mock_script(
            &bin_dir,
            "mock_parallel.py",
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
    elif method in ("session/new", "session/load"):
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"sessionId": "s_par"}}), flush=True)
    elif method == "session/prompt":
        txt = msg["params"]["prompt"][0]["text"]
        if txt == "sleep_long":
            print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s_par", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "started"}}}}), flush=True)
            time.sleep(10)
            sys.exit(0)
        time.sleep(0.15)
        print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s_par", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": f"echo:{txt}"}}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"stopReason": "end_turn"}}), flush=True)
    elif method == "session/cancel":
        sys.exit(0)
"#,
        );

        let rt = LocalCodingHostRuntime::new(CodingHostRuntimeConfig {
            owner_user_id: 77,
            device_id: "dev-a".into(),
            db_path: tmp.path().join("db/rt.db"),
            worktrees_root: tmp.path().join("wt"),
        })
        .unwrap();

        let cfg = CodingAgentBotConfig {
            provider_kind: CodingProviderKind::AcpCustom,
            workspace_path: repo_dir.to_string_lossy().into_owned(),
            use_worktree: false,
            permission_mode: gpui_telegram_proto::CodingPermissionMode::FullAccess,
            executable_override: Some(mock_acp.to_string_lossy().into_owned()),
            ..Default::default()
        };

        let (tx, mut rx) = mpsc::unbounded_channel();

        // Launch 2 turns on topic 1 (must serialize) and 1 turn on topic 2 (runs in parallel).
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "t_1a".into(),
                owner_user_id: 77,
                target_device_id: None,
                bot_user_id: 10,
                chat_id: 20,
                topic_id: Some(1),
                is_forum_topic: true,
                prompt: "one".into(),
                config: cfg.clone(),
            },
            tx.clone(),
        )
        .await;
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "t_1b".into(),
                owner_user_id: 77,
                target_device_id: None,
                bot_user_id: 10,
                chat_id: 20,
                topic_id: Some(1),
                is_forum_topic: true,
                prompt: "two".into(),
                config: cfg.clone(),
            },
            tx.clone(),
        )
        .await;
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "t_2a".into(),
                owner_user_id: 77,
                target_device_id: None,
                bot_user_id: 10,
                chat_id: 20,
                topic_id: Some(2),
                is_forum_topic: true,
                prompt: "three".into(),
                config: cfg.clone(),
            },
            tx.clone(),
        )
        .await;

        let mut completed = HashMap::new();
        while completed.len() < 3 {
            if let CodingHostEvent::TurnCompleted {
                turn_id,
                ok,
                turn_idx,
                final_text,
                ..
            } = rx.recv().await.unwrap()
            {
                assert!(ok);
                completed.insert(turn_id, (turn_idx, final_text));
            }
        }
        assert_eq!(completed["t_1a"], (1, "echo:one".to_string()));
        assert_eq!(completed["t_1b"], (2, "echo:two".to_string()));
        assert_eq!(completed["t_2a"], (1, "echo:three".to_string()));

        // Runtime CancelTurn test.
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "t_cancel".into(),
                owner_user_id: 77,
                target_device_id: None,
                bot_user_id: 10,
                chat_id: 20,
                topic_id: Some(3),
                is_forum_topic: true,
                prompt: "sleep_long".into(),
                config: cfg.clone(),
            },
            tx.clone(),
        )
        .await;
        loop {
            match rx.recv().await.unwrap() {
                CodingHostEvent::TurnTextDelta { turn_id, .. } if turn_id == "t_cancel" => {
                    rt.handle_command(
                        CodingHostCommand::CancelTurn {
                            turn_id: "t_cancel".into(),
                            owner_user_id: 77,
                            target_device_id: None,
                        },
                        tx.clone(),
                    )
                    .await;
                }
                CodingHostEvent::TurnCompleted {
                    turn_id,
                    ok,
                    error,
                    ..
                } if turn_id == "t_cancel" => {
                    assert!(!ok);
                    assert!(error.unwrap().to_lowercase().contains("cancel"));
                    break;
                }
                _ => {}
            }
        }

        // Control actions coverage: ListModels, SetModel, SetPermissionMode, GetStatus, WorktreeOp(list/off), ResetSession, SetWorkspace, CheckLogin.
        let controls = vec![
            ("m1", CodingControlAction::ListModels),
            (
                "m2",
                CodingControlAction::SetModel {
                    model: "custom-model-x".into(),
                },
            ),
            (
                "m3",
                CodingControlAction::SetPermissionMode {
                    mode: gpui_telegram_proto::CodingPermissionMode::AutoEdit,
                },
            ),
            ("m4", CodingControlAction::GetStatus),
            (
                "m5",
                CodingControlAction::WorktreeOp {
                    op: "list".into(),
                    branch: None,
                },
            ),
            (
                "m6",
                CodingControlAction::WorktreeOp {
                    op: "off".into(),
                    branch: None,
                },
            ),
            ("m7", CodingControlAction::ResetSession),
            (
                "m8",
                CodingControlAction::SetWorkspace {
                    path: alt_dir.to_string_lossy().into_owned(),
                },
            ),
            ("m9", CodingControlAction::CheckLogin),
        ];
        for (req_id, action) in controls {
            rt.handle_command(
                CodingHostCommand::Control {
                    req_id: req_id.into(),
                    owner_user_id: 77,
                    target_device_id: None,
                    bot_user_id: 10,
                    chat_id: 20,
                    topic_id: Some(1),
                    is_forum_topic: true,
                    config: cfg.clone(),
                    action,
                },
                tx.clone(),
            )
            .await;
            match rx.recv().await.unwrap() {
                CodingHostEvent::ControlResult {
                    req_id: r,
                    ok,
                    updated_model,
                    updated_permission_mode,
                    updated_use_worktree,
                    updated_workspace,
                    ..
                } => {
                    assert_eq!(r, req_id);
                    assert!(ok, "control {req_id} failed");
                    if req_id == "m2" {
                        assert_eq!(updated_model.as_deref(), Some("custom-model-x"));
                    }
                    if req_id == "m3" {
                        assert_eq!(
                            updated_permission_mode,
                            Some(gpui_telegram_proto::CodingPermissionMode::AutoEdit)
                        );
                    }
                    if req_id == "m6" {
                        assert_eq!(updated_use_worktree, Some(false));
                    }
                    if req_id == "m8" {
                        assert!(updated_workspace.is_some());
                    }
                }
                other => panic!("unexpected event for {req_id}: {other:?}"),
            }
        }

        // Also verify ProbeProviders command returns ProbeResult with matching req_id and device_id.
        rt.handle_command(
            CodingHostCommand::ProbeProviders {
                req_id: "probe_1".into(),
                owner_user_id: 77,
                target_device_id: Some("dev-a".into()),
            },
            tx,
        )
        .await;
        match rx.recv().await.unwrap() {
            CodingHostEvent::ProbeResult {
                req_id,
                device_id,
                providers,
            } => {
                assert_eq!(req_id, "probe_1");
                assert_eq!(device_id, "dev-a");
                assert_eq!(providers.len(), CodingProviderKind::ALL.len());
                assert!(
                    providers
                        .iter()
                        .any(|p| p.kind == CodingProviderKind::AcpCustom && p.installed)
                );
            }
            other => panic!("unexpected probe event: {other:?}"),
        }

        rt.shutdown().await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn runtime_reconciles_updated_bot_config_while_preserving_thread_overrides() {
        let tmp = TempDir::new().unwrap();
        let repo_a = tmp.path().join("repo_a");
        let repo_b = tmp.path().join("repo_b");
        std::fs::create_dir_all(&repo_a).unwrap();
        std::fs::create_dir_all(&repo_b).unwrap();
        init_git_repo(&repo_a);
        init_git_repo(&repo_b);

        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let mock_acp = write_mock_script(
            &bin_dir,
            "mock_reconcile.py",
            r#"#!/usr/bin/env python3
import json, os, sys

session_counter = 0
loaded_session = None
active_model = None

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"protocolVersion": 1}}), flush=True)
    elif method == "session/load":
        loaded_session = msg["params"]["sessionId"]
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"sessionId": loaded_session, "models": {"currentModelId": "init"}}}), flush=True)
    elif method == "session/new":
        session_counter += 1
        sid = f"sess_{os.path.basename(os.getcwd())}_{session_counter}"
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"sessionId": sid, "models": {"currentModelId": "init"}}}), flush=True)
    elif method == "session/set_model":
        active_model = msg["params"]["modelId"]
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {}}), flush=True)
    elif method == "session/prompt":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": 700,
            "method": "session/request_permission",
            "params": {
                "sessionId": loaded_session or "s",
                "toolCall": {"toolCallId": "tc1", "title": "Write", "kind": "execute", "rawInput": {}},
                "options": [
                    {"optionId": "allow_once", "name": "Once", "kind": "allow_once"},
                    {"optionId": "deny", "name": "Deny", "kind": "reject_once"}
                ]
            }
        }), flush=True)
        resp = json.loads(sys.stdin.readline())
        opt = resp["result"]["outcome"]["optionId"]
        res = f"cwd={os.getcwd()}|loaded={loaded_session}|model={active_model}|opt={opt}"
        print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": res}}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"stopReason": "end_turn"}}), flush=True)
"#,
        );

        let rt = LocalCodingHostRuntime::new(CodingHostRuntimeConfig {
            owner_user_id: 55,
            device_id: "mac-rec".into(),
            db_path: tmp.path().join("db/reconcile.db"),
            worktrees_root: tmp.path().join("wt"),
        })
        .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();

        // Turn 1: FullAccess, use_worktree = true, model_override = Some("m1"), workspace = repo_a.
        let cfg_v1 = CodingAgentBotConfig {
            provider_kind: CodingProviderKind::AcpCustom,
            workspace_path: repo_a.to_string_lossy().into_owned(),
            use_worktree: true,
            permission_mode: CodingPermissionMode::FullAccess,
            model_override: Some("m1".into()),
            executable_override: Some(mock_acp.to_string_lossy().into_owned()),
            ..Default::default()
        };

        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "rec_t1".into(),
                owner_user_id: 55,
                target_device_id: None,
                bot_user_id: 901,
                chat_id: 4001,
                topic_id: None,
                is_forum_topic: false,
                prompt: "p1".into(),
                config: cfg_v1.clone(),
            },
            tx.clone(),
        )
        .await;

        let mut t1_session = None;
        loop {
            match rx.recv().await.unwrap() {
                CodingHostEvent::TurnStarted {
                    provider_session_id,
                    worktree_path,
                    ..
                } => {
                    assert!(worktree_path.is_some());
                    t1_session = provider_session_id;
                }
                CodingHostEvent::TurnApprovalRequested { .. } => {
                    panic!("FullAccess must auto-approve without prompting");
                }
                CodingHostEvent::TurnCompleted {
                    ok, final_text, ..
                } => {
                    assert!(ok);
                    assert!(final_text.contains("model=m1"));
                    assert!(final_text.contains("opt=allow_once"));
                    break;
                }
                _ => {}
            }
        }
        assert!(t1_session.is_some());

        // Thread sets explicit model override while bot baseline stays at cfg_v1.
        rt.handle_command(
            CodingHostCommand::Control {
                req_id: "rec_set_model".into(),
                owner_user_id: 55,
                target_device_id: None,
                bot_user_id: 901,
                chat_id: 4001,
                topic_id: None,
                is_forum_topic: false,
                config: cfg_v1.clone(),
                action: CodingControlAction::SetModel {
                    model: "thread-custom-m".into(),
                },
            },
            tx.clone(),
        )
        .await;
        match rx.recv().await.unwrap() {
            CodingHostEvent::ControlResult { ok, updated_model, .. } => {
                assert!(ok);
                assert_eq!(updated_model.as_deref(), Some("thread-custom-m"));
            }
            other => panic!("unexpected: {other:?}"),
        }

        // Turn 2 with unchanged cfg_v1 MUST preserve thread override ("thread-custom-m") and resume session.
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "rec_t2".into(),
                owner_user_id: 55,
                target_device_id: None,
                bot_user_id: 901,
                chat_id: 4001,
                topic_id: None,
                is_forum_topic: false,
                prompt: "p2".into(),
                config: cfg_v1.clone(),
            },
            tx.clone(),
        )
        .await;
        loop {
            match rx.recv().await.unwrap() {
                CodingHostEvent::TurnCompleted {
                    ok, final_text, ..
                } => {
                    assert!(ok);
                    assert!(final_text.contains("model=thread-custom-m"));
                    assert!(final_text.contains(&format!("loaded={}", t1_session.as_deref().unwrap())));
                    break;
                }
                _ => {}
            }
        }

        // Now simulate Bot Inspector PATCH: downgrade FullAccess -> Ask, switch workspace repo_a -> repo_b,
        // disable use_worktree, update model_override Some("m1") -> Some("m2").
        let cfg_v2 = CodingAgentBotConfig {
            provider_kind: CodingProviderKind::AcpCustom,
            workspace_path: repo_b.to_string_lossy().into_owned(),
            use_worktree: false,
            permission_mode: CodingPermissionMode::Ask,
            model_override: Some("m2".into()),
            executable_override: Some(mock_acp.to_string_lossy().into_owned()),
            ..Default::default()
        };

        // GetStatus immediately reflects reconciled bot config!
        rt.handle_command(
            CodingHostCommand::Control {
                req_id: "rec_status".into(),
                owner_user_id: 55,
                target_device_id: None,
                bot_user_id: 901,
                chat_id: 4001,
                topic_id: None,
                is_forum_topic: false,
                config: cfg_v2.clone(),
                action: CodingControlAction::GetStatus,
            },
            tx.clone(),
        )
        .await;
        match rx.recv().await.unwrap() {
            CodingHostEvent::ControlResult {
                ok, markdown_reply, ..
            } => {
                assert!(ok);
                assert!(markdown_reply.contains("Mode: `ask`"));
                assert!(markdown_reply.contains("Model: `m2`"));
                assert!(markdown_reply.contains("enabled: `false`"));
                assert!(markdown_reply.contains("repo_b"));
            }
            other => panic!("unexpected: {other:?}"),
        }

        // Turn 3 with cfg_v2 must prompt for approval (downgraded to Ask), run in repo_b (no worktree),
        // use fresh session (loaded=None), and use model m2.
        rt.handle_command(
            CodingHostCommand::StartTurn {
                turn_id: "rec_t3".into(),
                owner_user_id: 55,
                target_device_id: None,
                bot_user_id: 901,
                chat_id: 4001,
                topic_id: None,
                is_forum_topic: false,
                prompt: "p3".into(),
                config: cfg_v2.clone(),
            },
            tx.clone(),
        )
        .await;

        let canonical_repo_b = std::fs::canonicalize(&repo_b).unwrap();
        let mut saw_approval_prompt = false;
        loop {
            match rx.recv().await.unwrap() {
                CodingHostEvent::TurnStarted {
                    workspace_path,
                    worktree_path,
                    ..
                } => {
                    assert_eq!(PathBuf::from(&workspace_path), canonical_repo_b);
                    assert!(worktree_path.is_none());
                }
                CodingHostEvent::TurnApprovalRequested {
                    turn_id,
                    request_id,
                    ..
                } => {
                    saw_approval_prompt = true;
                    rt.handle_command(
                        CodingHostCommand::ResolveApproval {
                            turn_id,
                            request_id,
                            owner_user_id: 55,
                            target_device_id: None,
                            decision: CodingApprovalDecision::AllowOnce,
                        },
                        tx.clone(),
                    )
                    .await;
                }
                CodingHostEvent::TurnCompleted {
                    ok, final_text, ..
                } => {
                    assert!(ok);
                    assert!(saw_approval_prompt, "downgraded Ask mode must prompt");
                    assert!(final_text.contains(&format!("cwd={}", canonical_repo_b.display())));
                    assert!(final_text.contains("loaded=None"));
                    assert!(final_text.contains("model=m2"));
                    break;
                }
                _ => {}
            }
        }

        // Clearing model_override to None must also clear reconciled active_model.
        let cfg_v3 = CodingAgentBotConfig {
            model_override: None,
            ..cfg_v2
        };
        rt.handle_command(
            CodingHostCommand::Control {
                req_id: "rec_status_cleared_model".into(),
                owner_user_id: 55,
                target_device_id: None,
                bot_user_id: 901,
                chat_id: 4001,
                topic_id: None,
                is_forum_topic: false,
                config: cfg_v3,
                action: CodingControlAction::GetStatus,
            },
            tx,
        )
        .await;
        match rx.recv().await.unwrap() {
            CodingHostEvent::ControlResult {
                ok, markdown_reply, ..
            } => {
                assert!(ok);
                assert!(markdown_reply.contains("Model: `(default)`"));
            }
            other => panic!("unexpected: {other:?}"),
        }

        rt.shutdown().await;
    }

    #[tokio::test]
    async fn runtime_isolates_owner_user_ids_sharing_single_db_path() {
        let tmp = TempDir::new().unwrap();
        let shared_db = tmp.path().join("shared_coding_host_state.db");
        let shared_wt_root = tmp.path().join("shared-worktrees");
        let alice_repo = tmp.path().join("alice_repo");
        let bob_repo = tmp.path().join("bob_repo");
        std::fs::create_dir_all(&alice_repo).unwrap();
        std::fs::create_dir_all(&bob_repo).unwrap();
        init_git_repo(&alice_repo);
        init_git_repo(&bob_repo);

        let rt_alice = LocalCodingHostRuntime::new(CodingHostRuntimeConfig {
            owner_user_id: 1001,
            device_id: "dev-shared".into(),
            db_path: shared_db.clone(),
            worktrees_root: shared_wt_root.clone(),
        })
        .unwrap();

        let alice_cfg = CodingAgentBotConfig {
            provider_kind: CodingProviderKind::ClaudeCode,
            workspace_path: alice_repo.to_string_lossy().into_owned(),
            use_worktree: false,
            permission_mode: CodingPermissionMode::AutoEdit,
            model_override: Some("alice-secret-model".into()),
            ..Default::default()
        };

        let (tx_a, mut rx_a) = mpsc::unbounded_channel();
        rt_alice
            .handle_command(
                CodingHostCommand::Control {
                    req_id: "alice_wt_on".into(),
                    owner_user_id: 1001,
                    target_device_id: None,
                    bot_user_id: 500,
                    chat_id: 102,
                    topic_id: None,
                    is_forum_topic: false,
                    config: alice_cfg.clone(),
                    action: CodingControlAction::WorktreeOp {
                        op: "on".into(),
                        branch: None,
                    },
                },
                tx_a,
            )
            .await;
        match rx_a.recv().await.unwrap() {
            CodingHostEvent::ControlResult {
                ok, markdown_reply, ..
            } => {
                assert!(ok, "alice worktree on failed: {markdown_reply}");
                assert!(markdown_reply.contains("u1001_b500_c102"), "{markdown_reply}");
            }
            other => panic!("unexpected: {other:?}"),
        }

        // Seed Alice's provider_session_id, turn_count=17, last_summary, and a checkpoint directly.
        {
            let store = CodingStore::open(&shared_db).unwrap();
            let mut rec = store
                .get_thread(ThreadKey::for_owner(1001, 500, 102, None))
                .unwrap()
                .expect("alice thread must exist");
            rec.turn_count = 17;
            rec.provider_session_id = Some("secret_claude_sess_alice".to_string());
            rec.last_summary = Some("alice confidential summary".to_string());
            store.upsert_thread(&rec).unwrap();

            store
                .save_checkpoint(&CheckpointRecord {
                    owner_user_id: 1001,
                    thread_id: rec.thread_id.clone(),
                    turn_idx: 1,
                    before_ref: "refs/a/1-before".into(),
                    after_ref: "refs/a/1".into(),
                    diff_stat: CodingDiffStat::default(),
                    prompt_preview: "alice secret prompt".into(),
                    reply_preview: "alice secret reply".into(),
                    created_at_ms: now_ms(),
                })
                .unwrap();
        }
        rt_alice.shutdown().await;

        let rt_bob = LocalCodingHostRuntime::new(CodingHostRuntimeConfig {
            owner_user_id: 2002,
            device_id: "dev-shared".into(),
            db_path: shared_db.clone(),
            worktrees_root: shared_wt_root.clone(),
        })
        .unwrap();

        let bob_cfg = CodingAgentBotConfig {
            provider_kind: CodingProviderKind::ClaudeCode,
            workspace_path: bob_repo.to_string_lossy().into_owned(),
            use_worktree: false,
            permission_mode: CodingPermissionMode::Ask,
            model_override: None,
            ..Default::default()
        };

        let (tx_b, mut rx_b) = mpsc::unbounded_channel();
        rt_bob
            .handle_command(
                CodingHostCommand::Control {
                    req_id: "bob_status".into(),
                    owner_user_id: 2002,
                    target_device_id: None,
                    bot_user_id: 500,
                    chat_id: 102,
                    topic_id: None,
                    is_forum_topic: false,
                    config: bob_cfg.clone(),
                    action: CodingControlAction::GetStatus,
                },
                tx_b.clone(),
            )
            .await;
        match rx_b.recv().await.unwrap() {
            CodingHostEvent::ControlResult {
                ok, markdown_reply, ..
            } => {
                assert!(ok);
                assert!(markdown_reply.contains(&bob_repo.display().to_string()), "{markdown_reply}");
                assert!(markdown_reply.contains("Turns: `0`"), "{markdown_reply}");
                assert!(markdown_reply.contains("Session: `(none)`"), "{markdown_reply}");
                assert!(markdown_reply.contains("Worktree: `(none)`"), "{markdown_reply}");
                assert!(!markdown_reply.contains("alice"), "{markdown_reply}");
                assert!(!markdown_reply.contains("17"), "{markdown_reply}");
                assert!(!markdown_reply.contains("secret_claude_sess_alice"), "{markdown_reply}");
            }
            other => panic!("unexpected: {other:?}"),
        }

        rt_bob
            .handle_command(
                CodingHostCommand::Control {
                    req_id: "bob_handoff".into(),
                    owner_user_id: 2002,
                    target_device_id: None,
                    bot_user_id: 500,
                    chat_id: 102,
                    topic_id: None,
                    is_forum_topic: false,
                    config: bob_cfg.clone(),
                    action: CodingControlAction::BuildHandoffSummary {
                        target_bot_username: "codex_bot".into(),
                    },
                },
                tx_b.clone(),
            )
            .await;
        match rx_b.recv().await.unwrap() {
            CodingHostEvent::ControlResult {
                ok, handoff_summary, ..
            } => {
                assert!(ok);
                let summary = handoff_summary.unwrap();
                assert!(!summary.contains("alice"), "{summary}");
                assert!(!summary.contains("Turn #1"), "{summary}");
            }
            other => panic!("unexpected: {other:?}"),
        }

        // Bob creating a worktree even on the SAME shared git repo uses u2002_b500_c102, never colliding with Alice's u1001_b500_c102.
        let bob_shared_repo_cfg = CodingAgentBotConfig {
            workspace_path: alice_repo.to_string_lossy().into_owned(),
            ..bob_cfg
        };
        rt_bob
            .handle_command(
                CodingHostCommand::Control {
                    req_id: "bob_wt_on".into(),
                    owner_user_id: 2002,
                    target_device_id: None,
                    bot_user_id: 500,
                    chat_id: 102,
                    topic_id: None,
                    is_forum_topic: false,
                    config: bob_shared_repo_cfg,
                    action: CodingControlAction::WorktreeOp {
                        op: "on".into(),
                        branch: None,
                    },
                },
                tx_b,
            )
            .await;
        match rx_b.recv().await.unwrap() {
            CodingHostEvent::ControlResult {
                ok, markdown_reply, ..
            } => {
                assert!(ok, "bob worktree on failed: {markdown_reply}");
                assert!(markdown_reply.contains("u2002_b500_c102"), "{markdown_reply}");
                assert!(markdown_reply.contains("fabushi/u2002_b500_c102"), "{markdown_reply}");
                assert!(!markdown_reply.contains("u1001"), "{markdown_reply}");
            }
            other => panic!("unexpected: {other:?}"),
        }

        rt_bob.shutdown().await;
    }
}
