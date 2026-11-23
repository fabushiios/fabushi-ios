use chrono::Utc;
use ed25519_dalek::{SigningKey, VerifyingKey};
use gpui_telegram_agent::hermes::memory::MemoryTarget;
use gpui_telegram_agent::hermes::skills::{CreateSkillRequest, SkillOrigin};
use gpui_telegram_agent::hermes::state_db::{
    ApprovalStatus, CreateApprovalRequest, CreateCronJobRequest, CreateKanbanCardRequest,
    CreateSessionRequest, SessionListFilter,
};
use gpui_telegram_agent::hermes::toolsets::{ToolsetId, ToolsetPreset};
use gpui_telegram_agent::multi_agent::{
    evaluate_group_wake_gate, evaluate_outbound_turn_review, is_explicit_pass_sentinel,
    FloorControlQueue, FloorPriority, GroupWakeGateInput, LoopBudget, OutboundTurnReviewInput,
    ScopedCapabilityToken, SharedGroupWorkspace, DEFAULT_MAX_LOOP_HOPS,
};
use gpui_telegram_agent::tasks::{CronRoutine, TaskFsm, TaskState};
use gpui_telegram_client::net::{
    HermesApprovalDecisionRequest, HermesCommandRequest, HermesTurnRequest,
};
use gpui_telegram_coding::store::{CheckpointRecord, ThreadKey, ThreadRecord};
use gpui_telegram_proto::coding_agent_contracts::{
    CodingHostEvent, CodingProviderKind,
};
use gpui_telegram_proto::miniapp_contracts::{
    CreateFabushiBotRequest, PatchFabushiBotRequest,
};
use magpie_gateway::{
    all_presets, find_preset, parse_dotenv_providers, parse_magpie_import_link,
    resolve_group_targets, resolve_preset_endpoints, MaskingConfig, MaskingEngine, RoutingGroup,
    UsageRecord,
};
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::error::{HostError, HostResult};
use crate::host::FabushiAppHost;

const IOS_FORBIDDEN_TOOLSETS: [ToolsetId; 4] = [
    ToolsetId::Terminal,
    ToolsetId::CodeExecution,
    ToolsetId::DesktopUi,
    ToolsetId::Browser,
];

fn is_forbidden_ios_toolset(id: ToolsetId) -> bool {
    IOS_FORBIDDEN_TOOLSETS.contains(&id)
}

fn parse_req<T: serde::de::DeserializeOwned>(params: &Value) -> HostResult<T> {
    serde_json::from_value(params.clone())
        .map_err(|e| HostError::invalid_arguments(format!("Invalid request parameters: {e}")))
}

fn req_str<'a>(params: &'a Value, key: &str) -> HostResult<&'a str> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required string '{key}'")))
}

fn req_raw_str<'a>(params: &'a Value, key: &str) -> HostResult<&'a str> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required string '{key}'")))
}

fn req_i64(params: &Value, key: &str) -> HostResult<i64> {
    params
        .get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required i64 '{key}'")))
}

fn req_u64(params: &Value, key: &str) -> HostResult<u64> {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| HostError::invalid_arguments(format!("Missing required u64 '{key}'")))
}

fn to_val<T: serde::Serialize>(v: T) -> HostResult<Value> {
    serde_json::to_value(v).map_err(|e| HostError::internal(format!("Serialization error: {e}")))
}

pub fn handle_fabushi(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "status" => {
            if params
                .get("remote")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && host.fenced_client_token(&client)?.is_some()
            {
                let res = host.run_cancellable(req_id, async move {
                    client.hermes_status().await.map_err(HostError::from)
                })?;
                return to_val(res);
            }
            let commands_count = host.hermes.list_commands().len();
            let toolsets_summary = host
                .hermes
                .toolsets()
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .summary_json();
            let journal_mode = host
                .hermes
                .store()
                .journal_mode()
                .unwrap_or_else(|_| "wal".to_string());
            Ok(json!({
                "engine": "hermes_embedded",
                "version": env!("CARGO_PKG_VERSION"),
                "commands_count": commands_count,
                "toolsets_count": ToolsetId::ALL.len(),
                "fts5_available": host.hermes.store().fts5_available(),
                "journal_mode": journal_mode,
                "toolsets": toolsets_summary,
            }))
        }
        "listCommands" => {
            let cmds = host.hermes.list_commands();
            Ok(json!({ "count": cmds.len(), "commands": cmds }))
        }
        "autocompleteCommands" => {
            let prefix = params.get("prefix").and_then(|v| v.as_str()).unwrap_or("/");
            let matches = host.hermes.autocomplete_commands(prefix);
            Ok(json!({ "prefix": prefix, "count": matches.len(), "commands": matches }))
        }
        "listToolsets" => {
            let summary = host
                .hermes
                .toolsets()
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .summary_json();
            Ok(json!({ "count": ToolsetId::ALL.len(), "summary": summary }))
        }
        "updateToolsets" => {
            let mut reg = host
                .hermes
                .toolsets()
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(enables) = params.get("enable").and_then(|v| v.as_array()) {
                let items: Vec<&str> = enables.iter().filter_map(|v| v.as_str()).collect();
                if host.is_ios_sandbox() {
                    for raw in &items {
                        let norm = raw.trim().to_ascii_lowercase().replace('-', "_");
                        if let Some(preset) = ToolsetPreset::from_str(raw.trim()) {
                            if preset.toolsets().iter().any(|t| is_forbidden_ios_toolset(*t)) {
                                return Err(HostError::capability_unavailable(format!(
                                    "Toolset preset '{}' includes desktop-only toolsets unavailable in the iOS sandbox",
                                    preset.as_str()
                                )));
                            }
                        } else if let Some(custom) = reg.custom_toolsets.get(raw.trim()) {
                            if custom.toolsets.iter().any(|t| is_forbidden_ios_toolset(*t)) {
                                return Err(HostError::capability_unavailable(format!(
                                    "Custom toolset '{}' includes desktop-only toolsets unavailable in the iOS sandbox",
                                    custom.name
                                )));
                            }
                        } else if let Some(id) = ToolsetId::from_str(&norm) {
                            if is_forbidden_ios_toolset(id) {
                                return Err(HostError::capability_unavailable(format!(
                                    "Toolset '{}' is unavailable in the iOS sandbox",
                                    id.as_str()
                                )));
                            }
                        }
                    }
                }
                for item in items {
                    reg.enable_by_name(item.trim())
                        .map_err(HostError::invalid_arguments)?;
                }
            }
            if let Some(disables) = params.get("disable").and_then(|v| v.as_array()) {
                for item in disables.iter().filter_map(|v| v.as_str()) {
                    reg.disable_by_name(item.trim())
                        .map_err(HostError::invalid_arguments)?;
                }
            }
            Ok(reg.summary_json())
        }
        "runTurn" => {
            let req: HermesTurnRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client.hermes_run_turn(&req).await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "runCommand" => {
            let req: HermesCommandRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .hermes_run_command(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "executeLocalTerminalTool" => {
            host.require_non_ios_host_capability("fabushi.executeLocalTerminalTool")?;
            Err(HostError::capability_unavailable(
                "Local terminal tool execution must route through a bound computer host",
            ))
        }
        // --- Tasks FSM ---
        "listTasks" => {
            let tasks: Vec<_> = host.task_registry.lock().values().cloned().collect();
            to_val(tasks)
        }
        "createTask" => {
            let task_id = params
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("task_{}", uuid::Uuid::new_v4().simple()));
            let task = TaskFsm::new(task_id.clone());
            host.task_registry
                .lock()
                .insert(task_id.clone(), task.clone());
            host.emit_event(
                "taskStateUpdated",
                json!({ "taskId": task_id, "state": "pending" }),
            );
            to_val(task)
        }
        "transitionTask" => {
            let task_id = req_str(params, "task_id")?;
            let state_str = req_str(params, "state")?;
            let target = TaskState::from_str(state_str).ok_or_else(|| {
                HostError::invalid_arguments(format!("Invalid TaskState '{state_str}'"))
            })?;
            let out = {
                let mut reg = host.task_registry.lock();
                let task = reg
                    .get_mut(task_id)
                    .ok_or_else(|| HostError::not_found(format!("Task '{task_id}' not found")))?;
                task.transition_to(target)?;
                to_val(task.clone())?
            };
            // Lock on task_registry is explicitly dropped BEFORE invoking emit_event / foreign C callbacks (B-03).
            host.emit_event(
                "taskStateUpdated",
                json!({ "taskId": task_id, "state": target.as_str() }),
            );
            Ok(out)
        }
        "approveTask" => {
            let task_id = req_str(params, "task_id")?.to_string();
            let approved = params
                .get("approved")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if params
                .get("remote")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && host.fenced_client_token(&client)?.is_some()
            {
                return host.run_cancellable(req_id, async move {
                    client
                        .approve_task(&task_id, approved)
                        .await
                        .map_err(HostError::from)
                });
            }
            let mut reg = host.task_registry.lock();
            let task = reg
                .get_mut(&task_id)
                .ok_or_else(|| HostError::not_found(format!("Task '{task_id}' not found")))?;
            if approved {
                let _ = task.transition_to(TaskState::Running);
            } else {
                let _ = task.abort("Denied by user");
            }
            to_val(task.clone())
        }
        // --- Kanban (SQLite WAL via HermesStateStore) ---
        "kanbanBoards" => {
            let boards = host.hermes.store().list_kanban_boards()?;
            to_val(boards)
        }
        "kanbanCreateBoard" => {
            let name = req_str(params, "name")?;
            let desc = params
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let cols: Option<Vec<String>> = params
                .get("columns")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok());
            let board = host.with_fenced_session_read(&client, || {
                Ok(host.hermes.store().create_kanban_board(name, desc, cols)?)
            })?;
            to_val(board)
        }
        "kanbanCreateCard" => {
            let req: CreateKanbanCardRequest = parse_req(params)?;
            let card = host.with_fenced_session_read(&client, || {
                Ok(host.hermes.store().create_kanban_card(req)?)
            })?;
            to_val(card)
        }
        "kanbanListCards" => {
            let board_id = req_str(params, "board_id")?;
            let col = params.get("column_name").and_then(|v| v.as_str());
            let cards = host.hermes.store().list_kanban_cards(board_id, col)?;
            to_val(cards)
        }
        "kanbanMoveCard" => {
            let card_id = req_str(params, "card_id")?;
            let target_col = req_str(params, "target_column")?;
            let position = params.get("position").and_then(|v| v.as_i64());
            let card = host.with_fenced_session_read(&client, || {
                Ok(host
                    .hermes
                    .store()
                    .move_kanban_card(card_id, target_col, position)?)
            })?;
            to_val(card)
        }
        "kanbanDeleteCard" => {
            let card_id = req_str(params, "card_id")?;
            let deleted = host.with_fenced_session_read(&client, || {
                Ok(host.hermes.store().delete_kanban_card(card_id)?)
            })?;
            Ok(json!({ "deleted": deleted, "card_id": card_id }))
        }
        // --- Skills (SKILL.md) ---
        "skillsList" => {
            let filter = params.get("filter").and_then(|v| v.as_str());
            let list = host.hermes.skills().skills_list(filter);
            to_val(list)
        }
        "skillSave" => {
            if let Some(raw_md) = params.get("markdown").and_then(|v| v.as_str()) {
                let doc = host.hermes.skills().skill_upsert_markdown(
                    raw_md,
                    Some(SkillOrigin::User),
                    true,
                )?;
                return to_val(doc);
            }
            let req: CreateSkillRequest = parse_req(params)?;
            let doc = host.hermes.skills().skill_create(req)?;
            to_val(doc)
        }
        "skillPatch" => {
            let name = req_str(params, "name")?;
            let old_text = req_str(params, "old_text")?;
            let new_text = req_str(params, "new_text")?;
            let doc = host.hermes.skills().skill_patch(name, old_text, new_text)?;
            to_val(doc)
        }
        "skillDelete" => {
            let name = req_str(params, "name")?;
            let deleted = host.hermes.skills().skill_delete(name)?;
            Ok(json!({ "deleted": deleted, "name": name }))
        }
        // --- Memory / Soul / Honcho ---
        "memoryGet" => {
            let target_str = params
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("memory");
            let target = MemoryTarget::parse(target_str).ok_or_else(|| {
                HostError::invalid_arguments(format!("Invalid MemoryTarget '{target_str}'"))
            })?;
            let res = host.hermes.memory().read(target)?;
            to_val(res)
        }
        "memorySave" => {
            let target_str = params
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("memory");
            let target = MemoryTarget::parse(target_str).ok_or_else(|| {
                HostError::invalid_arguments(format!("Invalid MemoryTarget '{target_str}'"))
            })?;
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let res = host.hermes.memory().set_raw(target, content)?;
            to_val(res)
        }
        "memoryMutate" => {
            let target_str = params
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("memory");
            let target = MemoryTarget::parse(target_str).ok_or_else(|| {
                HostError::invalid_arguments(format!("Invalid MemoryTarget '{target_str}'"))
            })?;
            let op = req_str(params, "op")?;
            let res = match op {
                "add" => host.hermes.memory().add(target, req_str(params, "entry")?)?,
                "remove" => host
                    .hermes
                    .memory()
                    .remove(target, req_str(params, "match_substr")?)?,
                "clear" => host.hermes.memory().clear(target)?,
                other => {
                    return Err(HostError::invalid_arguments(format!(
                        "Unsupported memoryMutate op '{other}'"
                    )));
                }
            };
            to_val(res)
        }
        "memoryQuarantineList" => {
            let pending = host.hermes.memory().list_pending_quarantine()?;
            to_val(pending)
        }
        "soulGet" => {
            let soul = host.hermes.memory().read_soul()?;
            to_val(soul)
        }
        "soulSave" => {
            let markdown = req_str(params, "markdown")?;
            let soul = host.hermes.memory().write_soul_raw(markdown)?;
            to_val(soul)
        }
        "honchoGet" => {
            let user_id = params
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let prof = host.hermes.honcho().profile(user_id)?;
            to_val(prof)
        }
        "honchoPost" => {
            let user_id = params
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let question = req_str(params, "question")?;
            let res = host.hermes.honcho().query(user_id, question)?;
            to_val(res)
        }
        // --- Cron & Heartbeat ---
        "cronList" => {
            let enabled_only = params
                .get("enabled_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let jobs = host.hermes.store().list_cron_jobs(enabled_only)?;
            to_val(jobs)
        }
        "cronSave" => {
            let req: CreateCronJobRequest = parse_req(params)?;
            let routine = CronRoutine::new("validate".to_string(), req.schedule.clone());
            if !routine.validate_expression() {
                return Err(HostError::invalid_arguments(format!(
                    "Invalid cron schedule expression '{}'",
                    req.schedule
                )));
            }
            let job = host.with_fenced_session_read(&client, || {
                Ok(host.hermes.store().create_cron_job(req)?)
            })?;
            to_val(job)
        }
        "cronPreview" => {
            let expr = req_str(params, "schedule")?;
            let mut routine = CronRoutine::new("preview".to_string(), expr.to_string());
            if !routine.validate_expression() {
                return Err(HostError::invalid_arguments(format!(
                    "Invalid cron schedule expression '{expr}'"
                )));
            }
            let now = Utc::now();
            let next_run = routine.compute_next_run(now);
            let summary = routine.preview_next_7_days(now);
            Ok(json!({
                "valid": true,
                "next_run": next_run.map(|d| d.to_rfc3339()),
                "summary": {
                    "scheduled_fires_next_7_days": summary.scheduled_fires_next_7_days,
                    "fires_on_weekend": summary.fires_on_weekend,
                    "fires_overnight": summary.fires_overnight,
                },
            }))
        }
        "cronDelete" => {
            let job_id = req_str(params, "job_id")?;
            let deleted = host.with_fenced_session_read(&client, || {
                Ok(host.hermes.store().delete_cron_job(job_id)?)
            })?;
            Ok(json!({ "deleted": deleted, "job_id": job_id }))
        }
        // --- Sessions ---
        "sessionsList" => {
            let filter: SessionListFilter = params
                .get("filter")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let list = host.hermes.store().list_sessions(filter)?;
            to_val(list)
        }
        "sessionCreate" => {
            let req: CreateSessionRequest = parse_req(params)?;
            let rec = host.with_fenced_session_read(&client, || {
                Ok(host.hermes.store().create_session(req)?)
            })?;
            to_val(rec)
        }
        "sessionGet" => {
            let session_id = req_str(params, "session_id")?;
            let rec = host.hermes.store().get_session(session_id)?;
            to_val(rec)
        }
        "sessionSearch" => {
            let query = req_str(params, "query")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let hits = host.hermes.store().session_search(query, None, limit)?;
            to_val(hits)
        }
        // --- Approvals & Value-Bound Approval Ledger ---
        "approvalsList" => {
            let session_id = params.get("session_id").and_then(|v| v.as_str());
            let pending_only = params
                .get("pending_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let list = host.hermes.store().list_approvals(session_id, pending_only)?;
            to_val(list)
        }
        "approvalEnqueue" => {
            let req: CreateApprovalRequest = parse_req(params)?;
            let rec = host.with_fenced_session_read(&client, || {
                Ok(host.hermes.store().enqueue_approval(req)?)
            })?;
            to_val(rec)
        }
        "approvalDecide" => {
            if params
                .get("remote")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && host.fenced_client_token(&client)?.is_some()
            {
                let approval_id = req_str(params, "approval_id")?.to_string();
                let req: HermesApprovalDecisionRequest = parse_req(params)?;
                let res = host.run_cancellable(req_id, async move {
                    client
                        .hermes_decide_approval(&approval_id, &req)
                        .await
                        .map_err(HostError::from)
                })?;
                return to_val(res);
            }
            if let Some(req_id_str) = params.get("request_id").and_then(|v| v.as_str()) {
                let approved = params
                    .get("approved")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let resolved = host.tool_dock.lock().resolve(req_id_str, approved).is_some();
                return Ok(json!({
                    "request_id": req_id_str,
                    "resolved": resolved,
                    "approved": approved
                }));
            }
            let approval_id = req_str(params, "approval_id")?;
            let status_str = params
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("approved");
            let status = ApprovalStatus::parse(status_str);
            let decided_by = params
                .get("decided_by")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let rec = host.with_fenced_session_read(&client, || {
                Ok(host
                    .hermes
                    .store()
                    .decide_approval(approval_id, status, decided_by)?)
            })?;
            to_val(rec)
        }
        "approvalIssueValueBound" => {
            let act = req_str(params, "action")?;
            let target = req_str(params, "target")?;
            let cwd = params
                .get("working_directory")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let sha = params
                .get("content_sha256")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let ttl_ms = params.get("ttl_ms").and_then(|v| v.as_i64());
            let token = host
                .approval_ledger
                .lock()
                .issue_approval_now(act, target, cwd, sha, ttl_ms);
            to_val(token)
        }
        "approvalConsumeValueBound" => {
            let approval_id = req_str(params, "approval_id")?;
            let act = req_str(params, "action")?;
            let target = req_str(params, "target")?;
            let cwd = params.get("working_directory").and_then(|v| v.as_str());
            let sha = params.get("content_sha256").and_then(|v| v.as_str());
            let now_ms = params
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| Utc::now().timestamp_millis());
            let consumed = host
                .approval_ledger
                .lock()
                .verify_and_consume(approval_id, act, target, cwd, sha, now_ms)
                .map_err(|e| HostError::new("approval_rejected", e.to_string(), false))?;
            to_val(consumed)
        }
        // --- Bots ---
        "botsList" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_fabushi_bots().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "botGet" => {
            let bot_id = req_u64(params, "bot_id")?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .get_fabushi_bot(bot_id)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "botCreate" => {
            let req: CreateFabushiBotRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .create_fabushi_bot(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "botPatch" => {
            let bot_id = req_u64(params, "bot_id")?;
            let req: PatchFabushiBotRequest = parse_req(params.get("patch").unwrap_or(params))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .patch_fabushi_bot(bot_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "botCreateNl" => {
            let prompt = req_str(params, "prompt")?.to_string();
            let avatar_shape = params
                .get("avatar_shape")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let avatar_color = params
                .get("avatar_color")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let res = host.run_cancellable(req_id, async move {
                client
                    .create_from_natural_language_with_avatar(&prompt, avatar_shape, avatar_color)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        _ => Err(HostError::unknown_command(&format!("fabushi.{action}"))),
    }
}

fn parse_floor_priority(raw: &str) -> FloorPriority {
    match raw.trim().to_ascii_lowercase().as_str() {
        "user_interruption" | "userinterruption" | "user" => FloorPriority::UserInterruption,
        "direct_mention" | "directmention" | "mention" => FloorPriority::DirectMention,
        "task_assignee" | "taskassignee" | "assignee" => FloorPriority::TaskAssignee,
        _ => FloorPriority::GeneralContribution,
    }
}

pub fn handle_multi_agent(
    host: &FabushiAppHost,
    _req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    match action {
        "floorRequest" => {
            let chat_id = req_i64(params, "chat_id")?;
            let agent_id = req_str(params, "agent_id")?;
            let priority_str = params
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("general_contribution");
            let lease_ms = params
                .get("lease_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(30_000);
            let priority = parse_floor_priority(priority_str);
            let mut queues = host.floor_queues.lock();
            let q = queues
                .entry(chat_id)
                .or_insert_with(|| FloorControlQueue::new(lease_ms));
            let granted = q.request_floor(agent_id, priority);
            let current = q.current_speaker_id();
            let queued = q.queued_count();
            let active_grant = q.active_grant();
            Ok(json!({
                "chat_id": chat_id,
                "granted": granted,
                "current_speaker": current,
                "queued_count": queued,
                "active_grant": active_grant,
            }))
        }
        "floorPreemptUser" => {
            let chat_id = req_i64(params, "chat_id")?;
            let mut queues = host.floor_queues.lock();
            let q = queues
                .entry(chat_id)
                .or_insert_with(|| FloorControlQueue::new(30_000));
            let prev_speaker = q.current_speaker_id();
            let preempted_nonce = q.preempt_for_user_interruption();
            let current = q.current_speaker_id();
            Ok(json!({
                "chat_id": chat_id,
                "preempted_agent": prev_speaker,
                "preempted_nonce": preempted_nonce,
                "current_speaker": current,
                "queued_count": q.queued_count(),
            }))
        }
        "floorRelease" => {
            let chat_id = req_i64(params, "chat_id")?;
            let agent_id = req_str(params, "agent_id")?;
            let mut queues = host.floor_queues.lock();
            let q = queues
                .entry(chat_id)
                .or_insert_with(|| FloorControlQueue::new(30_000));
            q.release_floor(agent_id);
            Ok(json!({
                "chat_id": chat_id,
                "current_speaker": q.current_speaker_id(),
                "queued_count": q.queued_count(),
            }))
        }
        "floorStatus" => {
            let chat_id = req_i64(params, "chat_id")?;
            let queues = host.floor_queues.lock();
            if let Some(q) = queues.get(&chat_id) {
                Ok(json!({
                    "chat_id": chat_id,
                    "current_speaker": q.current_speaker_id(),
                    "queued_count": q.queued_count(),
                    "active_grant": q.active_grant(),
                }))
            } else {
                Ok(json!({
                    "chat_id": chat_id,
                    "current_speaker": Value::Null,
                    "queued_count": 0,
                    "active_grant": Value::Null,
                }))
            }
        }
        "loopBudgetRecordHop" => {
            let chat_id = req_i64(params, "chat_id")?;
            let agent_id = req_str(params, "agent_id")?;
            let max_hops = params
                .get("max_hops")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .unwrap_or(DEFAULT_MAX_LOOP_HOPS);
            let mut budgets = host.loop_budgets.lock();
            let b = budgets
                .entry(chat_id)
                .or_insert_with(|| LoopBudget::new(max_hops));
            match b.record_hop(agent_id) {
                Ok(hop) => Ok(json!({
                    "chat_id": chat_id,
                    "current_hop": hop,
                    "max_hops": b.max_hops(),
                    "exhausted": b.is_exhausted(),
                    "trace": b.trace(),
                })),
                Err(err) => Err(HostError::new("loop_budget_exceeded", err.to_string(), false)),
            }
        }
        "loopBudgetStatus" => {
            let chat_id = req_i64(params, "chat_id")?;
            let budgets = host.loop_budgets.lock();
            if let Some(b) = budgets.get(&chat_id) {
                Ok(json!({
                    "chat_id": chat_id,
                    "current_hop": b.current_hop(),
                    "max_hops": b.max_hops(),
                    "exhausted": b.is_exhausted(),
                    "trace": b.trace(),
                }))
            } else {
                Ok(json!({
                    "chat_id": chat_id,
                    "current_hop": 0,
                    "max_hops": DEFAULT_MAX_LOOP_HOPS,
                    "exhausted": false,
                    "trace": [],
                }))
            }
        }
        "loopBudgetReset" => {
            let chat_id = req_i64(params, "chat_id")?;
            if let Some(b) = host.loop_budgets.lock().get_mut(&chat_id) {
                b.reset();
            }
            Ok(json!({ "chat_id": chat_id, "reset": true }))
        }
        "workspaceSet" => {
            let chat_id = req_i64(params, "chat_id")?;
            let key = req_str(params, "key")?;
            let value = params
                .get("value")
                .cloned()
                .ok_or_else(|| HostError::invalid_arguments("Missing 'value'"))?;
            let updated_by = params
                .get("updated_by")
                .and_then(|v| v.as_str())
                .unwrap_or("user");
            let expected_revision = params.get("expected_revision").and_then(|v| v.as_u64());
            let client = host.client();
            let (revision, snap) = host.with_fenced_session_read(&client, || {
                let mut wss = host.group_workspaces.lock();
                let ws = wss
                    .entry(chat_id)
                    .or_insert_with(|| SharedGroupWorkspace::new(chat_id));
                let rev = if let Some(exp) = expected_revision {
                    ws.set_with_expected_revision(exp, key, value, updated_by)
                        .map_err(|e| HostError::conflict(e.to_string()))?
                } else {
                    ws.set(key, value, updated_by)
                };
                let snapshot = ws.snapshot();
                let _ = host.host_db.kv_set(
                    "multi_agent_workspace",
                    &chat_id.to_string(),
                    &snapshot.to_string(),
                );
                Ok((rev, snapshot))
            })?;
            Ok(json!({ "revision": revision, "snapshot": snap }))
        }
        "workspaceGet" => {
            let chat_id = req_i64(params, "chat_id")?;
            let key = req_str(params, "key")?;
            let wss = host.group_workspaces.lock();
            let entry = wss.get(&chat_id).and_then(|ws| ws.get(key)).cloned();
            to_val(entry)
        }
        "workspaceSnapshot" => {
            let chat_id = req_i64(params, "chat_id")?;
            let wss = host.group_workspaces.lock();
            if let Some(ws) = wss.get(&chat_id) {
                Ok(ws.snapshot())
            } else {
                Ok(SharedGroupWorkspace::new(chat_id).snapshot())
            }
        }
        "workspaceRemove" => {
            let chat_id = req_i64(params, "chat_id")?;
            let key = req_str(params, "key")?;
            let client = host.client();
            let removed = host.with_fenced_session_read(&client, || {
                let mut wss = host.group_workspaces.lock();
                let res = if let Some(ws) = wss.get_mut(&chat_id) {
                    let r = ws.remove(key);
                    if r.is_some() {
                        let _ = host.host_db.kv_set(
                            "multi_agent_workspace",
                            &chat_id.to_string(),
                            &ws.snapshot().to_string(),
                        );
                    }
                    r
                } else {
                    None
                };
                Ok(res)
            })?;
            Ok(json!({ "removed": removed }))
        }
        "evaluateWakeGate" => {
            let input: GroupWakeGateInput = parse_req(params)?;
            let decision = evaluate_group_wake_gate(&input);
            to_val(decision)
        }
        "evaluateOutboundReview" => {
            let input: OutboundTurnReviewInput = parse_req(params)?;
            let verdict = evaluate_outbound_turn_review(&input);
            to_val(verdict)
        }
        "isPassSentinel" => {
            let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
            Ok(json!({ "is_pass": is_explicit_pass_sentinel(text) }))
        }
        "issueCapabilityToken" => {
            let agent_id = params
                .get("agent_id")
                .or_else(|| params.get("subject"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| HostError::invalid_arguments("Missing 'agent_id'"))?;
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let actions: Vec<String> = params
                .get("allowed_actions")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let ttl_secs = params
                .get("ttl_secs")
                .and_then(|v| v.as_i64())
                .unwrap_or(300);
            let mut seed = Zeroizing::new([0u8; 32]);
            getrandom::getrandom(&mut *seed)
                .map_err(|e| HostError::crypto(format!("CSPRNG error: {e}")))?;
            let signing_key = SigningKey::from_bytes(&*seed);
            let verifying_key: VerifyingKey = signing_key.verifying_key();
            let token = ScopedCapabilityToken::issue(
                &signing_key,
                agent_id,
                chat_id,
                actions,
                ttl_secs,
            );
            Ok(json!({
                "token": token,
                "verifying_key_hex": hex::encode(verifying_key.to_bytes()),
            }))
        }
        "verifyCapabilityToken" => {
            let token: ScopedCapabilityToken = parse_req(
                params
                    .get("token")
                    .ok_or_else(|| HostError::invalid_arguments("Missing 'token'"))?,
            )?;
            let vk_hex = req_str(params, "verifying_key_hex")?;
            let vk_bytes = hex::decode(vk_hex)
                .map_err(|e| HostError::invalid_arguments(format!("Invalid hex key: {e}")))?;
            let arr: [u8; 32] = vk_bytes
                .try_into()
                .map_err(|_| HostError::invalid_arguments("verifying_key_hex must be 32 bytes"))?;
            let vk = VerifyingKey::from_bytes(&arr)
                .map_err(|e| HostError::crypto(format!("Invalid Ed25519 key: {e}")))?;
            let now_ts = params
                .get("now_ts")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| Utc::now().timestamp());
            let valid = if let (Some(agent_id), Some(chat_id), Some(action_name)) = (
                params.get("agent_id").and_then(|v| v.as_str()),
                params.get("chat_id").and_then(|v| v.as_i64()),
                params.get("action").and_then(|v| v.as_str()),
            ) {
                token.verify_for_scope(&vk, now_ts, agent_id, chat_id, action_name)
            } else {
                token.verify(&vk, now_ts)
            };
            let permits = params
                .get("action")
                .and_then(|v| v.as_str())
                .map(|a| token.permits_action(a));
            Ok(json!({ "valid": valid, "permits_action": permits }))
        }
        _ => Err(HostError::unknown_command(&format!("multi_agent.{action}"))),
    }
}

pub fn handle_magpie(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "listPresets" => {
            let presets = all_presets();
            Ok(json!({ "count": presets.len(), "presets": presets }))
        }
        "findPreset" => {
            let id = req_str(params, "id")?;
            let preset = find_preset(id)
                .ok_or_else(|| HostError::not_found(format!("Magpie preset '{id}' not found")))?;
            to_val(preset)
        }
        "resolvePresetEndpoints" => {
            let preset_id = req_str(params, "preset_id")?;
            let region = params.get("region").and_then(|v| v.as_str());
            let protocol: Option<magpie_gateway::ProtocolKind> = params
                .get("protocol")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok());
            let resolved = resolve_preset_endpoints(preset_id, region, protocol)
                .ok_or_else(|| HostError::not_found(format!("Unable to resolve preset '{preset_id}'")))?;
            to_val(resolved)
        }
        "state" => {
            if params
                .get("remote")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                if let Some(token) = host.fenced_client_token(&client)? {
                    let res = host.run_cancellable(req_id, async move {
                        client
                            .hermes
                            .magpie_state(&token)
                            .await
                            .map_err(HostError::from)
                    })?;
                    return to_val(res);
                }
            }
            let groups: Vec<_> = host.magpie_groups.lock().values().cloned().collect();
            let usage = host.magpie_usage.lock().summary(None);
            Ok(json!({
                "presets_count": all_presets().len(),
                "groups": groups,
                "usage": usage,
            }))
        }
        "listGroups" => {
            let groups: Vec<_> = host.magpie_groups.lock().values().cloned().collect();
            to_val(groups)
        }
        "upsertGroup" => {
            let group: RoutingGroup = parse_req(params)?;
            let id = group.id.clone();
            let encoded = serde_json::to_string(&group).unwrap_or_default();
            host.with_fenced_session_read(&client, || {
                host.magpie_groups.lock().insert(id.clone(), group.clone());
                let _ = host.host_db.kv_set("magpie_groups", &id, &encoded);
                Ok(())
            })?;
            to_val(group)
        }
        "deleteGroup" => {
            let id = req_str(params, "id")?;
            let removed = host.with_fenced_session_read(&client, || {
                let r = host.magpie_groups.lock().remove(id).is_some();
                let _ = host.host_db.kv_delete("magpie_groups", id);
                Ok(r)
            })?;
            Ok(json!({ "deleted": removed, "id": id }))
        }
        "resolveRoute" => {
            let target = req_str(params, "target")?;
            let default_provider = params
                .get("default_provider")
                .and_then(|v| v.as_str())
                .unwrap_or("deepseek");
            let session_key = params.get("session_key").and_then(|v| v.as_str());
            let prompt = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
            let msg_count = params
                .get("message_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as usize;
            let tool_count = params
                .get("tool_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let now_ms = params
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| Utc::now().timestamp_millis());
            let groups = host.magpie_groups.lock();
            let mut metrics = host.magpie_metrics.lock();
            let targets = resolve_group_targets(
                target,
                &groups,
                &mut metrics,
                default_provider,
                session_key,
                prompt,
                msg_count,
                tool_count,
                now_ms,
            )
            .map_err(|e| HostError::new("routing_error", e.to_string(), false))?;
            let serialized: Vec<Value> = targets
                .into_iter()
                .map(|t| {
                    json!({
                        "provider_id": t.provider_id,
                        "model": t.model,
                        "strategy": t.strategy,
                        "reason": t.reason,
                    })
                })
                .collect();
            Ok(Value::Array(serialized))
        }
        "recordUsage" => {
            let target = req_str(params, "target")?;
            let tokens = params.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let latency_ms = params.get("latency_ms").and_then(|v| v.as_u64());
            let now_ms = params
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| Utc::now().timestamp_millis());
            host.magpie_metrics
                .lock()
                .record_usage(target, tokens, latency_ms, now_ms);
            if let Ok(rec) = serde_json::from_value::<UsageRecord>(params.clone()) {
                let _ = host.magpie_usage.lock().record(rec);
            }
            Ok(json!({ "recorded": true }))
        }
        "recordFailure" => {
            let target = req_str(params, "target")?;
            let status_code = params
                .get("status_code")
                .and_then(|v| v.as_u64())
                .map(|n| n as u16);
            let retry_after_secs = params.get("retry_after_secs").and_then(|v| v.as_u64());
            let now_ms = params
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| Utc::now().timestamp_millis());
            host.magpie_metrics
                .lock()
                .record_failure(target, status_code, retry_after_secs, now_ms);
            Ok(json!({ "recorded": true }))
        }
        "maskingUpdate" => {
            let cfg: MaskingConfig = parse_req(params)?;
            *host.magpie_masking.lock() = MaskingEngine::new(cfg.clone());
            to_val(cfg)
        }
        "maskText" => {
            let text = req_raw_str(params, "text")?;
            if let Some(cfg_val) = params.get("config") {
                let cfg: MaskingConfig = parse_req(cfg_val)?;
                let temp_engine = MaskingEngine::new(cfg);
                let res = temp_engine.mask_text(text);
                return to_val(res);
            }
            let engine = host.magpie_masking.lock();
            let res = engine.mask_text(text);
            to_val(res)
        }
        "maskingPreview" => {
            let text = req_raw_str(params, "text")?;
            if let Some(cfg_val) = params.get("config") {
                let cfg: MaskingConfig = parse_req(cfg_val)?;
                let temp_engine = MaskingEngine::new(cfg);
                let res = temp_engine.preview_text(text);
                return to_val(res);
            }
            let engine = host.magpie_masking.lock();
            let res = engine.preview_text(text);
            to_val(res)
        }
        "unmaskText" => {
            let masked_text = req_raw_str(params, "text")?;
            let engine = host.magpie_masking.lock();
            let restored = engine.unmask_text(masked_text);
            Ok(json!({ "text": restored }))
        }
        "importLink" => {
            let uri = req_str(params, "uri")?;
            let spec = parse_magpie_import_link(uri)
                .map_err(|e| HostError::invalid_arguments(e.to_string()))?;
            to_val(spec)
        }
        "importDotenv" => {
            let text = req_str(params, "text")?;
            let specs = parse_dotenv_providers(text);
            to_val(specs)
        }
        "usageSummary" => {
            let since_ms = params.get("since_ms").and_then(|v| v.as_i64());
            let summary = host.magpie_usage.lock().summary(since_ms);
            to_val(summary)
        }
        "scanLocalSubscriptions" => {
            host.require_non_ios_host_capability("magpie.scanLocalSubscriptions")?;
            Err(HostError::capability_unavailable(
                "Local CLI dotfile subscription scan is disabled on iOS; use vault.* zero-knowledge AI OAuth sync",
            ))
        }
        _ => Err(HostError::unknown_command(&format!("magpie.{action}"))),
    }
}

pub fn handle_coding(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "listProvidersCatalog" => {
            let providers: Vec<Value> = CodingProviderKind::ALL
                .iter()
                .map(|k| {
                    json!({
                        "kind": k.as_str(),
                        "supports_acp": k.supports_acp(),
                    })
                })
                .collect();
            Ok(json!({ "providers": providers }))
        }
        "parseProvider" => {
            let raw = req_str(params, "provider")?;
            let kind = CodingProviderKind::parse(raw).ok_or_else(|| {
                HostError::invalid_arguments(format!("Unknown coding provider '{raw}'"))
            })?;
            Ok(json!({ "kind": kind.as_str(), "supports_acp": kind.supports_acp() }))
        }
        "listBots" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_fabushi_bots().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "createBot" => {
            let req: CreateFabushiBotRequest = parse_req(params)?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .create_fabushi_bot(&req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "patchBot" => {
            let bot_id = req_u64(params, "bot_id")?;
            let req: PatchFabushiBotRequest = parse_req(params.get("patch").unwrap_or(params))?;
            let res = host.run_cancellable(req_id, async move {
                client
                    .patch_fabushi_bot(bot_id, &req)
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "sendHostEvent" => {
            let ev: CodingHostEvent = parse_req(params)?;
            client.send_coding_host_event(ev)?;
            Ok(json!({ "sent": true }))
        }
        "upsertThread" => {
            let rec = ThreadRecord {
                bot_user_id: req_i64(params, "bot_user_id")?,
                chat_id: req_i64(params, "chat_id")?,
                topic_id: params.get("topic_id").and_then(|v| v.as_i64()).unwrap_or(0),
                thread_id: req_str(params, "thread_id")?.to_string(),
                provider_session_id: params.get("provider_session_id").and_then(|v| v.as_str()).map(ToOwned::to_owned),
                workspace_path: params.get("workspace_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                worktree_path: params.get("worktree_path").and_then(|v| v.as_str()).map(ToOwned::to_owned),
                branch_name: params.get("branch_name").and_then(|v| v.as_str()).map(ToOwned::to_owned),
                use_worktree: params.get("use_worktree").and_then(|v| v.as_bool()).unwrap_or(false),
                active_model: params.get("active_model").and_then(|v| v.as_str()).map(ToOwned::to_owned),
                permission_mode: params
                    .get("permission_mode")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default(),
                turn_count: params.get("turn_count").and_then(|v| v.as_u64()).unwrap_or(0),
                last_summary: params.get("last_summary").and_then(|v| v.as_str()).map(ToOwned::to_owned),
                updated_at_ms: params
                    .get("updated_at_ms")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| Utc::now().timestamp_millis()),
                bot_provider_kind: params
                    .get("bot_provider_kind")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok()),
                bot_workspace_path: params.get("bot_workspace_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                bot_use_worktree: params.get("bot_use_worktree").and_then(|v| v.as_bool()).unwrap_or(false),
                bot_active_model: params.get("bot_active_model").and_then(|v| v.as_str()).map(ToOwned::to_owned),
                bot_permission_mode: params
                    .get("bot_permission_mode")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok()),
            };
            host.with_fenced_session_read(&client, || {
                host.coding_store
                    .lock()
                    .upsert_thread(&rec)
                    .map_err(HostError::storage)
            })?;
            Ok(json!({
                "thread_id": rec.thread_id,
                "bot_user_id": rec.bot_user_id,
                "chat_id": rec.chat_id,
                "topic_id": rec.topic_id,
                "turn_count": rec.turn_count,
                "updated_at_ms": rec.updated_at_ms,
            }))
        }
        "getThread" => {
            let bot_user_id = req_i64(params, "bot_user_id")?;
            let chat_id = req_i64(params, "chat_id")?;
            let topic_id = params.get("topic_id").and_then(|v| v.as_i64());
            let key = ThreadKey::new(bot_user_id, chat_id, topic_id);
            let rec = host
                .coding_store
                .lock()
                .get_thread(key)
                .map_err(HostError::storage)?;
            Ok(match rec {
                Some(r) => json!({
                    "bot_user_id": r.bot_user_id,
                    "chat_id": r.chat_id,
                    "topic_id": r.topic_id,
                    "thread_id": r.thread_id,
                    "provider_session_id": r.provider_session_id,
                    "workspace_path": r.workspace_path,
                    "worktree_path": r.worktree_path,
                    "branch_name": r.branch_name,
                    "use_worktree": r.use_worktree,
                    "active_model": r.active_model,
                    "permission_mode": r.permission_mode,
                    "turn_count": r.turn_count,
                    "last_summary": r.last_summary,
                    "updated_at_ms": r.updated_at_ms,
                }),
                None => Value::Null,
            })
        }
        "saveCheckpoint" => {
            let cp = CheckpointRecord {
                thread_id: req_str(params, "thread_id")?.to_string(),
                turn_idx: params.get("turn_idx").and_then(|v| v.as_u64()).unwrap_or(0),
                before_ref: params.get("before_ref").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                after_ref: params.get("after_ref").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                diff_stat: params
                    .get("diff_stat")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default(),
                prompt_preview: params.get("prompt_preview").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                reply_preview: params.get("reply_preview").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                created_at_ms: params
                    .get("created_at_ms")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| Utc::now().timestamp_millis()),
            };
            host.with_fenced_session_read(&client, || {
                host.coding_store
                    .lock()
                    .save_checkpoint(&cp)
                    .map_err(HostError::storage)
            })?;
            Ok(json!({
                "thread_id": cp.thread_id,
                "turn_idx": cp.turn_idx,
                "before_ref": cp.before_ref,
                "after_ref": cp.after_ref,
                "created_at_ms": cp.created_at_ms,
            }))
        }
        "getCheckpoints" => {
            let thread_id = req_str(params, "thread_id")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let cps = host
                .coding_store
                .lock()
                .list_recent_checkpoints(thread_id, limit)
                .map_err(HostError::storage)?;
            let out: Vec<Value> = cps
                .into_iter()
                .map(|c| {
                    json!({
                        "thread_id": c.thread_id,
                        "turn_idx": c.turn_idx,
                        "before_ref": c.before_ref,
                        "after_ref": c.after_ref,
                        "diff_stat": c.diff_stat,
                        "prompt_preview": c.prompt_preview,
                        "reply_preview": c.reply_preview,
                        "created_at_ms": c.created_at_ms,
                    })
                })
                .collect();
            Ok(Value::Array(out))
        }
        // Fail-closed local CLI execution on iOS
        "probeLocal" | "startLocalTurn" | "localControl" => {
            host.require_non_ios_host_capability(&format!("coding.{action}"))?;
            Err(HostError::capability_unavailable(
                "Local coding CLI subprocess execution is disabled in mobile host; dispatch to a remote desktop/cloud coding host",
            ))
        }
        _ => Err(HostError::unknown_command(&format!("coding.{action}"))),
    }
}

pub fn handle_mcp(
    host: &FabushiAppHost,
    req_id: &str,
    action: &str,
    params: &Value,
) -> HostResult<Value> {
    let client = host.client();
    match action {
        "listServers" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_mcp_servers().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "registerServer" => {
            if params
                .get("transport")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("stdio"))
            {
                host.require_non_ios_host_capability("mcp.registerLocalStdioServer")?;
            }
            let payload = params.clone();
            host.run_cancellable(req_id, async move {
                client
                    .register_mcp_server(&payload)
                    .await
                    .map_err(HostError::from)
            })
        }
        "listTools" => {
            let res = host.run_cancellable(req_id, async move {
                client.list_mcp_tools().await.map_err(HostError::from)
            })?;
            to_val(res)
        }
        "callTool" => {
            let payload = params.clone();
            host.run_cancellable(req_id, async move {
                client
                    .call_mcp_tool(&payload)
                    .await
                    .map_err(HostError::from)
            })
        }
        "approvePermission" => {
            let payload = params.clone();
            host.run_cancellable(req_id, async move {
                client
                    .approve_mcp_permission(&payload)
                    .await
                    .map_err(HostError::from)
            })
        }
        "listGrants" => {
            let res = host.run_cancellable(req_id, async move {
                client
                    .list_mcp_grants_async()
                    .await
                    .map_err(HostError::from)
            })?;
            to_val(res)
        }
        "revokeGrant" => {
            let family_id = req_str(params, "family_id")?.to_string();
            let fid_move = family_id.clone();
            host.run_cancellable(req_id, async move {
                client
                    .revoke_mcp_grant_async(&fid_move)
                    .await
                    .map_err(HostError::from)
            })?;
            Ok(json!({ "revoked": true, "family_id": family_id }))
        }
        "localRegistryList" => {
            let reg = host
                .hermes
                .mcp_registry()
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let defs = reg.list_tools();
            to_val(defs)
        }
        "permissionDockResolve" => {
            let request_id = req_str(params, "request_id")?;
            let approved = params
                .get("approved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let resolved = host.tool_dock.lock().resolve(request_id, approved).is_some();
            Ok(json!({ "request_id": request_id, "resolved": resolved, "approved": approved }))
        }
        _ => Err(HostError::unknown_command(&format!("mcp.{action}"))),
    }
}
