use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::hermes::security::{
    ApprovalConsumeError, AutoReviewPolicy, AutoReviewVerdict, ToolApprovalTier,
    ValueBoundApprovalLedger, ValueBoundApprovalToken,
};

use super::registry::{McpToolDefinition, ToolRiskLevel};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPermissionRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub risk_level: ToolRiskLevel,
    pub caller_agent_id: String,
    pub requested_at: DateTime<Utc>,
}

impl ToolPermissionRequest {
    pub fn new(
        request_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: serde_json::Value,
        risk_level: ToolRiskLevel,
        caller_agent_id: impl Into<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            tool_name: tool_name.into(),
            arguments,
            risk_level,
            caller_agent_id: caller_agent_id.into(),
            requested_at: Utc::now(),
        }
    }
}

#[derive(Debug, Default)]
struct SharedDockPendingState {
    pending_requests: Vec<String>,
    requests_by_id: HashMap<String, ToolPermissionRequest>,
    resolved_history: Vec<(String, bool, DateTime<Utc>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissionDock {
    pub pending_requests: Vec<String>,
    pub requests_by_id: HashMap<String, ToolPermissionRequest>,
    pub resolved_history: Vec<(String, bool, DateTime<Utc>)>,
    #[serde(skip)]
    shared_policy: Arc<RwLock<AutoReviewPolicy>>,
    #[serde(skip)]
    shared_ledger: Arc<RwLock<ValueBoundApprovalLedger>>,
    #[serde(skip)]
    shared_pending: Arc<RwLock<SharedDockPendingState>>,
}

impl Default for ToolPermissionDock {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolPermissionDock {
    pub fn new() -> Self {
        Self {
            pending_requests: Vec::new(),
            requests_by_id: HashMap::new(),
            resolved_history: Vec::new(),
            shared_policy: Arc::new(RwLock::new(AutoReviewPolicy::default())),
            shared_ledger: Arc::new(RwLock::new(ValueBoundApprovalLedger::new())),
            shared_pending: Arc::new(RwLock::new(SharedDockPendingState::default())),
        }
    }

    pub fn with_policy(policy: AutoReviewPolicy) -> Self {
        let dock = Self::new();
        *dock.shared_policy.write() = policy;
        dock
    }

    pub fn push_request(&mut self, req_id: String) {
        if !self.pending_requests.contains(&req_id) {
            self.pending_requests.push(req_id.clone());
        }
        let mut shared = self.shared_pending.write();
        if !shared.pending_requests.contains(&req_id) {
            shared.pending_requests.push(req_id);
        }
    }

    pub fn submit_request(&mut self, req: ToolPermissionRequest) {
        let req_id = req.request_id.clone();
        if !self.pending_requests.contains(&req_id) {
            self.pending_requests.push(req_id.clone());
        }
        self.requests_by_id.insert(req_id.clone(), req.clone());

        let mut shared = self.shared_pending.write();
        if !shared.pending_requests.contains(&req_id) {
            shared.pending_requests.push(req_id.clone());
        }
        shared.requests_by_id.insert(req_id, req);
    }

    /// Shared (`&self`) submission path for `Arc<ToolPermissionDock>` attached to `HermesEngine`.
    pub fn submit_shared_request(&self, req: ToolPermissionRequest) {
        let req_id = req.request_id.clone();
        let mut shared = self.shared_pending.write();
        if !shared.pending_requests.contains(&req_id) {
            shared.pending_requests.push(req_id.clone());
        }
        shared.requests_by_id.insert(req_id, req);
    }

    pub fn list_shared_pending(&self) -> Vec<ToolPermissionRequest> {
        let shared = self.shared_pending.read();
        let mut out: Vec<ToolPermissionRequest> = shared
            .pending_requests
            .iter()
            .filter_map(|id| shared.requests_by_id.get(id).cloned())
            .collect();
        for id in &self.pending_requests {
            if !shared.requests_by_id.contains_key(id) {
                if let Some(req) = self.requests_by_id.get(id) {
                    out.push(req.clone());
                }
            }
        }
        out
    }

    pub fn pending_shared_count(&self) -> usize {
        self.list_shared_pending().len()
    }

    pub fn resolve_shared(&self, req_id: &str, allow: bool) -> Option<ToolPermissionRequest> {
        let mut shared = self.shared_pending.write();
        shared.pending_requests.retain(|id| id != req_id);
        let req = shared.requests_by_id.remove(req_id);
        if req.is_some() {
            shared
                .resolved_history
                .push((req_id.to_string(), allow, Utc::now()));
        }
        req
    }

    pub fn resolved_shared_history(&self) -> Vec<(String, bool, DateTime<Utc>)> {
        self.shared_pending.read().resolved_history.clone()
    }

    pub fn auto_review_policy(&self) -> AutoReviewPolicy {
        self.shared_policy.read().clone()
    }

    pub fn set_auto_review_policy(&self, policy: AutoReviewPolicy) {
        *self.shared_policy.write() = policy;
    }

    pub fn set_tool_tier(&self, tool_name: impl Into<String>, tier: ToolApprovalTier) {
        self.shared_policy.write().set_tool_tier(tool_name, tier);
    }

    pub fn add_allow_instruction(&self, instruction: impl Into<String>) {
        self.shared_policy
            .write()
            .add_allow_instruction(instruction);
    }

    pub fn add_block_instruction(&self, instruction: impl Into<String>) {
        self.shared_policy
            .write()
            .add_block_instruction(instruction);
    }

    /// Evaluates both static/dynamic [`ToolRiskLevel`] and 4-tier [`AutoReviewPolicy`] verdict.
    pub fn evaluate_auto_review(
        &self,
        tool: &McpToolDefinition,
        args: &serde_json::Value,
    ) -> (ToolRiskLevel, AutoReviewVerdict) {
        let risk = self.evaluate_risk(tool, args);
        let is_destructive = matches!(
            risk,
            ToolRiskLevel::High | ToolRiskLevel::CriticalDestructive
        ) || tool.requires_approval;
        let verdict = self
            .shared_policy
            .read()
            .evaluate(&tool.name, args, is_destructive);
        (risk, verdict)
    }

    /// Issues a single-use value-bound approval token in this dock's ledger.
    pub fn issue_value_bound_approval(
        &self,
        action: impl Into<String>,
        target: impl Into<String>,
        working_directory: Option<String>,
        content_sha256: Option<String>,
        created_at_ms: i64,
        ttl_ms: i64,
    ) -> ValueBoundApprovalToken {
        self.shared_ledger.write().issue_approval(
            action,
            target,
            working_directory,
            content_sha256,
            created_at_ms,
            ttl_ms,
        )
    }

    /// Verifies and consumes a single-use value-bound approval token against TOCTOU invariants.
    pub fn verify_and_consume_approval(
        &self,
        approval_id: &str,
        action: &str,
        target: &str,
        cwd: Option<&str>,
        content_sha256: Option<&str>,
        now_ms: i64,
    ) -> Result<(), ApprovalConsumeError> {
        self.shared_ledger.write().verify_and_consume(
            approval_id,
            action,
            target,
            cwd,
            content_sha256,
            now_ms,
        )
    }

    /// Retires an issued value-bound approval token so it can no longer be spent.
    pub fn retire_approval(&self, approval_id: &str) -> bool {
        self.shared_ledger.write().retire_approval(approval_id)
    }

    pub fn evaluate_risk(&self, tool: &McpToolDefinition, args: &serde_json::Value) -> ToolRiskLevel {
        if tool.risk_level == ToolRiskLevel::CriticalDestructive {
            return ToolRiskLevel::CriticalDestructive;
        }

        const DESTRUCTIVE_KEYWORDS: &[&str] = &["rm", "delete", "drop", "kill", "exec", "write"];

        fn check_value(val: &serde_json::Value) -> bool {
            match val {
                serde_json::Value::String(s) => {
                    let lower = s.to_lowercase();
                    let words: Vec<&str> = lower
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .filter(|w| !w.is_empty())
                        .collect();
                    for kw in DESTRUCTIVE_KEYWORDS {
                        if *kw == "rm" {
                            if words.contains(&"rm") {
                                return true;
                            }
                        } else if lower.contains(kw) {
                            return true;
                        }
                    }
                    false
                }
                serde_json::Value::Array(arr) => arr.iter().any(check_value),
                serde_json::Value::Object(map) => {
                    map.iter().any(|(k, v)| {
                        let k_lower = k.to_lowercase();
                        DESTRUCTIVE_KEYWORDS.iter().any(|kw| {
                            if *kw == "rm" {
                                k_lower.split('_').any(|part| part == "rm")
                            } else {
                                k_lower.contains(kw)
                            }
                        }) || check_value(v)
                    })
                }
                _ => false,
            }
        }

        if check_value(args) {
            ToolRiskLevel::CriticalDestructive
        } else {
            tool.risk_level
        }
    }

    pub fn get_request(&self, req_id: &str) -> Option<&ToolPermissionRequest> {
        self.requests_by_id.get(req_id)
    }

    pub fn list_pending(&self) -> Vec<&ToolPermissionRequest> {
        self.pending_requests
            .iter()
            .filter_map(|id| self.requests_by_id.get(id))
            .collect()
    }

    pub fn pending_count(&self) -> usize {
        self.pending_requests.len()
    }

    pub fn resolve(&mut self, req_id: &str, allow: bool) -> Option<ToolPermissionRequest> {
        self.pending_requests.retain(|id| id != req_id);
        let req = self.requests_by_id.remove(req_id);
        let now = Utc::now();
        if req.is_some() {
            self.resolved_history.push((req_id.to_string(), allow, now));
        }

        let mut shared = self.shared_pending.write();
        shared.pending_requests.retain(|id| id != req_id);
        let shared_req = shared.requests_by_id.remove(req_id);
        if req.is_some() || shared_req.is_some() {
            shared
                .resolved_history
                .push((req_id.to_string(), allow, now));
        }
        req.or(shared_req)
    }
}
