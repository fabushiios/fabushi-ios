use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD as B64_STD, URL_SAFE_NO_PAD as B64_URL},
};
use chrono::Utc;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::backend::BrowserSessionBackend;
use crate::totp::generate_totp_now;
use crate::types::{BotBrowserError, NormalizedOrigin, ReusePolicy};
use crate::vault::CredentialVault;

pub use crate::types::{SemanticLocator, StepTimingMetric};

pub const SCRIPT_APP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScriptStep {
    Navigate {
        url_template: String,
    },
    WaitFor {
        locator: SemanticLocator,
        timeout_ms: u64,
    },
    Click {
        locator: SemanticLocator,
    },
    Fill {
        locator: SemanticLocator,
        value_template: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret_vault_key: Option<String>,
    },
    PressKey {
        key: String,
    },
    Scroll {
        delta_x: f64,
        delta_y: f64,
    },
    ExtractText {
        locator: SemanticLocator,
        output_key: String,
    },
    AssertVisible {
        locator: SemanticLocator,
    },
    AssertTextContains {
        locator: SemanticLocator,
        expected_template: String,
    },
    HumanCheckpoint {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptParameter {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(default)]
    pub is_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationScriptApp {
    pub schema_version: u32,
    pub app_id: String,
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub target_domains: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<ScriptParameter>,
    pub steps: Vec<ScriptStep>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_trace_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordedActionKind {
    Navigate {
        url: String,
    },
    WaitFor {
        locator: SemanticLocator,
        timeout_ms: u64,
    },
    Click {
        locator: SemanticLocator,
    },
    Fill {
        locator: SemanticLocator,
        value: String,
        #[serde(default)]
        is_secret: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vault_key_hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parameter_name_hint: Option<String>,
    },
    PressKey {
        key: String,
    },
    Scroll {
        delta_x: f64,
        delta_y: f64,
    },
    ExtractText {
        locator: SemanticLocator,
        output_key: String,
    },
    AssertVisible {
        locator: SemanticLocator,
    },
    AssertTextContains {
        locator: SemanticLocator,
        expected: String,
    },
    HumanCheckpoint {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedBrowserAction {
    pub seq: usize,
    pub timestamp_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    pub action: RecordedActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceParameterBinding {
    pub param_name: String,
    pub literal_value: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub is_secret: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrowserActionTrace {
    pub trace_id: String,
    pub session_id: String,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_url: Option<String>,
    pub actions: Vec<RecordedBrowserAction>,
    #[serde(default)]
    pub parameter_bindings: Vec<TraceParameterBinding>,
}

impl BrowserActionTrace {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            trace_id: format!("trace_{}", Uuid::new_v4().simple()),
            session_id: session_id.into(),
            started_at_ms: Utc::now().timestamp_millis(),
            completed_at_ms: None,
            initial_url: None,
            actions: Vec::new(),
            parameter_bindings: Vec::new(),
        }
    }

    pub fn push_action(
        &mut self,
        page_url: Option<String>,
        action: RecordedActionKind,
        duration_ms: Option<u64>,
    ) {
        let next_seq = self.actions.len() + 1;
        self.actions.push(RecordedBrowserAction {
            seq: next_seq,
            timestamp_ms: Utc::now().timestamp_millis(),
            page_url,
            action,
            duration_ms,
        });
    }

    pub fn mark_completed(&mut self) {
        self.completed_at_ms = Some(Utc::now().timestamp_millis());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptAppMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub target_domains: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(default)]
    pub parameter_overrides: Vec<ScriptParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptReplayReport {
    pub app_id: String,
    pub version: String,
    pub success: bool,
    pub steps_executed: usize,
    pub extracted_outputs: BTreeMap<String, String>,
    pub step_timings: Vec<StepTimingMetric>,
    pub total_duration_ms: u64,
    pub used_llm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[async_trait]
pub trait VaultSecretResolver: Send + Sync {
    async fn resolve_secret(
        &self,
        secret_vault_key: &str,
        current_origin: Option<&str>,
    ) -> Result<Zeroizing<String>, BotBrowserError>;
}

#[derive(Default)]
pub struct InMemoryVaultSecretResolver {
    secrets: RwLock<BTreeMap<String, Zeroizing<String>>>,
}

impl InMemoryVaultSecretResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: impl Into<String>, value: impl Into<String>) {
        self.secrets
            .write()
            .insert(key.into(), Zeroizing::new(value.into()));
    }
}

#[async_trait]
impl VaultSecretResolver for InMemoryVaultSecretResolver {
    async fn resolve_secret(
        &self,
        secret_vault_key: &str,
        _current_origin: Option<&str>,
    ) -> Result<Zeroizing<String>, BotBrowserError> {
        self.secrets
            .read()
            .get(secret_vault_key)
            .cloned()
            .ok_or_else(|| BotBrowserError::CredentialNotFound(secret_vault_key.to_string()))
    }
}

pub struct CredentialVaultSecretResolver {
    vault: Arc<CredentialVault>,
    confirm_every_time_approved: bool,
}

impl CredentialVaultSecretResolver {
    pub fn new(vault: Arc<CredentialVault>) -> Self {
        Self {
            vault,
            confirm_every_time_approved: false,
        }
    }

    pub fn with_confirm_every_time_approved(mut self, approved: bool) -> Self {
        self.confirm_every_time_approved = approved;
        self
    }
}

fn split_vault_key_field(secret_vault_key: &str) -> (&str, &str) {
    if let Some((id, f)) = secret_vault_key.rsplit_once(':') {
        if matches!(f, "password" | "username" | "totp") {
            return (id, f);
        }
    }
    if let Some((id, f)) = secret_vault_key.rsplit_once('.') {
        if matches!(f, "password" | "username" | "totp") {
            return (id, f);
        }
    }
    (secret_vault_key, "password")
}

#[async_trait]
impl VaultSecretResolver for CredentialVaultSecretResolver {
    async fn resolve_secret(
        &self,
        secret_vault_key: &str,
        current_origin: Option<&str>,
    ) -> Result<Zeroizing<String>, BotBrowserError> {
        let Some(origin_raw) = current_origin.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err(BotBrowserError::CredentialNotFound(format!(
                "{secret_vault_key} (missing active origin)"
            )));
        };
        let origin = NormalizedOrigin::parse(origin_raw)?;
        let (cred_part, field) = split_vault_key_field(secret_vault_key);

        let resolved_meta = match self.vault.get_meta(cred_part) {
            Ok(meta) => meta,
            Err(BotBrowserError::CredentialNotFound(_)) => {
                let metas = self.vault.list_for_origin(origin.as_str())?;
                if let Some(found) = metas
                    .iter()
                    .find(|m| m.account_hint == cred_part || m.credential_id == cred_part)
                    .cloned()
                {
                    found
                } else if metas.len() == 1 {
                    metas[0].clone()
                } else {
                    return Err(BotBrowserError::CredentialNotFound(
                        secret_vault_key.to_string(),
                    ));
                }
            }
            Err(e) => return Err(e),
        };

        if resolved_meta.origin != origin.as_str() {
            return Err(BotBrowserError::OriginMismatch {
                expected: resolved_meta.origin,
                actual: origin.as_str().to_string(),
            });
        }

        if resolved_meta.reuse_policy == ReusePolicy::ConfirmEveryTime
            && !self.confirm_every_time_approved
        {
            return Err(BotBrowserError::ApprovalRequired {
                card_id: "script_replay".to_string(),
                credential_id: resolved_meta.credential_id,
                origin: origin.as_str().to_string(),
            });
        }

        let decrypted = self.vault.decrypt_for_origin(
            &resolved_meta.credential_id,
            origin.as_str(),
            None,
        )?;
        match field {
            "username" => Ok(decrypted.username.clone()),
            "totp" => {
                let Some(totp_secret) = decrypted.totp_secret.as_ref() else {
                    return Err(BotBrowserError::Totp(format!(
                        "credential {} has no TOTP secret",
                        resolved_meta.credential_id
                    )));
                };
                generate_totp_now(totp_secret.as_str())
            }
            _ => Ok(decrypted.password.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BrowserControlMode {
    Idle,
    AiControlling {
        active_step: usize,
        status_text: String,
    },
    WaitingForTakeover {
        reason: String,
    },
    UserTakeover {
        paused_at_step: usize,
    },
}

pub struct BrowserControlGate {
    mode_tx: watch::Sender<BrowserControlMode>,
    _keep_rx: watch::Receiver<BrowserControlMode>,
    state: Mutex<BrowserControlMode>,
    active_step: AtomicUsize,
    cancelled: AtomicBool,
    notify: Notify,
}

impl Default for BrowserControlGate {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserControlGate {
    pub fn new() -> Self {
        let (mode_tx, _keep_rx) = watch::channel(BrowserControlMode::Idle);
        Self {
            mode_tx,
            _keep_rx,
            state: Mutex::new(BrowserControlMode::Idle),
            active_step: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    pub fn mode(&self) -> BrowserControlMode {
        self.state.lock().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<BrowserControlMode> {
        self.mode_tx.subscribe()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::Release);
        self.active_step.store(0, Ordering::Release);
        {
            let mut guard = self.state.lock();
            *guard = BrowserControlMode::Idle;
            self.mode_tx.send_replace(BrowserControlMode::Idle);
        }
        self.notify.notify_waiters();
    }

    pub fn start_ai(&self, status_text: impl Into<String>) {
        {
            let mut guard = self.state.lock();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            self.active_step.store(0, Ordering::Release);
            let next = BrowserControlMode::AiControlling {
                active_step: 0,
                status_text: status_text.into(),
            };
            *guard = next.clone();
            self.mode_tx.send_replace(next);
        }
        self.notify.notify_waiters();
    }

    pub fn set_active_step(&self, step: usize, status_text: impl Into<String>) {
        self.active_step.store(step, Ordering::Release);
        let mut guard = self.state.lock();
        if matches!(*guard, BrowserControlMode::AiControlling { .. }) {
            let next = BrowserControlMode::AiControlling {
                active_step: step,
                status_text: status_text.into(),
            };
            *guard = next.clone();
            self.mode_tx.send_replace(next);
        }
    }

    pub fn request_takeover(&self, reason: impl Into<String>) {
        let next = BrowserControlMode::WaitingForTakeover {
            reason: reason.into(),
        };
        let mut guard = self.state.lock();
        *guard = next.clone();
        self.mode_tx.send_replace(next);
    }

    pub fn user_take_over(&self) {
        let step = self.active_step.load(Ordering::Acquire);
        let mut guard = self.state.lock();
        if matches!(
            *guard,
            BrowserControlMode::AiControlling { .. } | BrowserControlMode::WaitingForTakeover { .. }
        ) {
            let next = BrowserControlMode::UserTakeover {
                paused_at_step: step,
            };
            *guard = next.clone();
            self.mode_tx.send_replace(next);
        }
    }

    pub fn user_resume_ai(&self) {
        let step = self.active_step.load(Ordering::Acquire);
        {
            let mut guard = self.state.lock();
            if matches!(
                *guard,
                BrowserControlMode::WaitingForTakeover { .. }
                    | BrowserControlMode::UserTakeover { .. }
            ) {
                let next = BrowserControlMode::AiControlling {
                    active_step: step,
                    status_text: "Resumed by user".to_string(),
                };
                *guard = next.clone();
                self.mode_tx.send_replace(next);
            }
        }
        self.notify.notify_waiters();
    }

    pub fn user_cancel_ai(&self) {
        {
            let mut guard = self.state.lock();
            self.cancelled.store(true, Ordering::Release);
            *guard = BrowserControlMode::Idle;
            self.mode_tx.send_replace(BrowserControlMode::Idle);
        }
        self.notify.notify_waiters();
    }

    pub fn complete_ai(&self) {
        {
            let mut guard = self.state.lock();
            *guard = BrowserControlMode::Idle;
            self.mode_tx.send_replace(BrowserControlMode::Idle);
        }
        self.notify.notify_waiters();
    }

    pub async fn wait_if_paused_or_cancelled(&self, step_idx: usize) -> Result<(), BotBrowserError> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if self.cancelled.load(Ordering::Acquire) {
                return Err(BotBrowserError::ControlCancelled(format!(
                    "cancelled at step {step_idx}"
                )));
            }

            let is_paused = matches!(
                *self.state.lock(),
                BrowserControlMode::WaitingForTakeover { .. }
                    | BrowserControlMode::UserTakeover { .. }
            );
            if !is_paused {
                return Ok(());
            }

            notified.await;
        }
    }
}

pub struct ScriptCompiler;

impl ScriptCompiler {
    pub fn compile_from_trace(
        trace: &BrowserActionTrace,
        meta: ScriptAppMetadata,
    ) -> Result<AutomationScriptApp, BotBrowserError> {
        let name = meta.name.trim();
        if name.is_empty() {
            return Err(BotBrowserError::ScriptCompilation(
                "script app name cannot be empty".to_string(),
            ));
        }
        if trace.actions.is_empty() {
            return Err(BotBrowserError::ScriptCompilation(
                "cannot compile empty BrowserActionTrace".to_string(),
            ));
        }

        let mut sorted_actions = trace.actions.clone();
        sorted_actions.sort_by_key(|a| a.seq);

        let mut parameters: BTreeMap<String, ScriptParameter> = BTreeMap::new();
        for p in &meta.parameter_overrides {
            let key = sanitize_param_name(&p.name);
            if !key.is_empty() {
                let mut cloned = p.clone();
                cloned.name = key.clone();
                if is_sensitive_param_name(&key) {
                    cloned.is_secret = true;
                }
                if cloned.is_secret {
                    cloned.default_value = None;
                }
                parameters.insert(key, cloned);
            }
        }

        let mut domains: BTreeSet<String> = BTreeSet::new();
        for d in &meta.target_domains {
            let cleaned = d.trim().to_ascii_lowercase();
            if !cleaned.is_empty() {
                domains.insert(cleaned);
            }
        }
        if let Some(init_url) = trace.initial_url.as_deref() {
            if let Ok(origin) = NormalizedOrigin::parse(init_url) {
                domains.insert(origin.host().to_string());
            }
        }

        let mut steps: Vec<ScriptStep> = Vec::with_capacity(sorted_actions.len());
        for recorded in &sorted_actions {
            if let Some(page_url) = recorded.page_url.as_deref() {
                if let Ok(origin) = NormalizedOrigin::parse(page_url) {
                    domains.insert(origin.host().to_string());
                }
            }

            match &recorded.action {
                RecordedActionKind::Navigate { url } => {
                    let origin = NormalizedOrigin::parse(url).map_err(|e| {
                        BotBrowserError::ScriptCompilation(format!(
                            "unsafe navigate URL at seq {}: {e}",
                            recorded.seq
                        ))
                    })?;
                    domains.insert(origin.host().to_string());
                    let sanitized_url = sanitize_navigate_url_template(
                        url,
                        &trace.parameter_bindings,
                        &mut parameters,
                    )?;
                    steps.push(ScriptStep::Navigate {
                        url_template: sanitized_url,
                    });
                }
                RecordedActionKind::WaitFor {
                    locator,
                    timeout_ms,
                } => {
                    if locator.is_empty() {
                        return Err(BotBrowserError::ScriptCompilation(format!(
                            "empty WaitFor locator at seq {}",
                            recorded.seq
                        )));
                    }
                    steps.push(ScriptStep::WaitFor {
                        locator: locator.clone(),
                        timeout_ms: (*timeout_ms).max(100),
                    });
                }
                RecordedActionKind::Click { locator } => {
                    if locator.is_empty() {
                        return Err(BotBrowserError::ScriptCompilation(format!(
                            "empty Click locator at seq {}",
                            recorded.seq
                        )));
                    }
                    steps.push(ScriptStep::Click {
                        locator: locator.clone(),
                    });
                }
                RecordedActionKind::Fill {
                    locator,
                    value,
                    is_secret,
                    vault_key_hint,
                    parameter_name_hint,
                } => {
                    if locator.is_empty() {
                        return Err(BotBrowserError::ScriptCompilation(format!(
                            "empty Fill locator at seq {}",
                            recorded.seq
                        )));
                    }
                    let binding_match = trace
                        .parameter_bindings
                        .iter()
                        .find(|b| !b.literal_value.is_empty() && b.literal_value == *value);

                    let hint_is_sensitive = parameter_name_hint
                        .as_deref()
                        .is_some_and(is_sensitive_param_name);
                    let binding_is_sensitive = binding_match
                        .is_some_and(|b| b.is_secret || is_sensitive_param_name(&b.param_name));

                    let sensitive = *is_secret
                        || locator.is_sensitive_hint()
                        || contains_secret_material(value)
                        || hint_is_sensitive
                        || binding_is_sensitive;

                    if sensitive {
                        if let Some(vault_key) =
                            vault_key_hint.as_deref().map(str::trim).filter(|s| !s.is_empty())
                        {
                            steps.push(ScriptStep::Fill {
                                locator: locator.clone(),
                                value_template: String::new(),
                                secret_vault_key: Some(vault_key.to_string()),
                            });
                        } else {
                            let base_pname = parameter_name_hint
                                .as_deref()
                                .or_else(|| binding_match.map(|b| b.param_name.as_str()))
                                .map(sanitize_param_name)
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| format!("secret_field_{}", recorded.seq));
                            let pname = unique_param_name(&base_pname, true, None, &parameters);
                            let desc = binding_match
                                .map(|b| b.description.clone())
                                .filter(|d| !d.trim().is_empty())
                                .unwrap_or_else(|| {
                                    format!("Secret value for {}", locator.display_label())
                                });
                            parameters.entry(pname.clone()).or_insert_with(|| ScriptParameter {
                                name: pname.clone(),
                                description: desc,
                                required: true,
                                default_value: None,
                                is_secret: true,
                            });
                            steps.push(ScriptStep::Fill {
                                locator: locator.clone(),
                                value_template: format!("{{{{{pname}}}}}"),
                                secret_vault_key: Some(format!("secret.{pname}")),
                            });
                        }
                    } else if let Some(binding) = binding_match {
                        let base_pname = Some(sanitize_param_name(&binding.param_name))
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| format!("param_{}", recorded.seq));
                        let pname = unique_param_name(
                            &base_pname,
                            false,
                            Some(value.as_str()),
                            &parameters,
                        );
                        parameters.entry(pname.clone()).or_insert_with(|| ScriptParameter {
                            name: pname.clone(),
                            description: if binding.description.trim().is_empty() {
                                format!("Input for {}", locator.display_label())
                            } else {
                                binding.description.clone()
                            },
                            required: binding.required,
                            default_value: Some(value.clone()),
                            is_secret: false,
                        });
                        steps.push(ScriptStep::Fill {
                            locator: locator.clone(),
                            value_template: format!("{{{{{pname}}}}}"),
                            secret_vault_key: None,
                        });
                    } else if let Some(hint) =
                        parameter_name_hint.as_deref().map(str::trim).filter(|s| !s.is_empty())
                    {
                        let base_pname = Some(sanitize_param_name(hint))
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| format!("param_{}", recorded.seq));
                        let pname = unique_param_name(
                            &base_pname,
                            false,
                            Some(value.as_str()),
                            &parameters,
                        );
                        parameters.entry(pname.clone()).or_insert_with(|| ScriptParameter {
                            name: pname.clone(),
                            description: format!("Input for {}", locator.display_label()),
                            required: true,
                            default_value: Some(value.clone()),
                            is_secret: false,
                        });
                        steps.push(ScriptStep::Fill {
                            locator: locator.clone(),
                            value_template: format!("{{{{{pname}}}}}"),
                            secret_vault_key: None,
                        });
                    } else {
                        steps.push(ScriptStep::Fill {
                            locator: locator.clone(),
                            value_template: value.clone(),
                            secret_vault_key: None,
                        });
                    }
                }
                RecordedActionKind::PressKey { key } => {
                    if key.trim().is_empty() {
                        return Err(BotBrowserError::ScriptCompilation(format!(
                            "empty PressKey at seq {}",
                            recorded.seq
                        )));
                    }
                    steps.push(ScriptStep::PressKey { key: key.clone() });
                }
                RecordedActionKind::Scroll { delta_x, delta_y } => {
                    steps.push(ScriptStep::Scroll {
                        delta_x: *delta_x,
                        delta_y: *delta_y,
                    });
                }
                RecordedActionKind::ExtractText {
                    locator,
                    output_key,
                } => {
                    if locator.is_empty() || output_key.trim().is_empty() {
                        return Err(BotBrowserError::ScriptCompilation(format!(
                            "invalid ExtractText at seq {}",
                            recorded.seq
                        )));
                    }
                    steps.push(ScriptStep::ExtractText {
                        locator: locator.clone(),
                        output_key: output_key.trim().to_string(),
                    });
                }
                RecordedActionKind::AssertVisible { locator } => {
                    if locator.is_empty() {
                        return Err(BotBrowserError::ScriptCompilation(format!(
                            "empty AssertVisible locator at seq {}",
                            recorded.seq
                        )));
                    }
                    steps.push(ScriptStep::AssertVisible {
                        locator: locator.clone(),
                    });
                }
                RecordedActionKind::AssertTextContains { locator, expected } => {
                    if locator.is_empty() {
                        return Err(BotBrowserError::ScriptCompilation(format!(
                            "empty AssertTextContains locator at seq {}",
                            recorded.seq
                        )));
                    }
                    let expected_template = apply_non_secret_bindings(
                        expected,
                        &trace.parameter_bindings,
                        &mut parameters,
                    );
                    steps.push(ScriptStep::AssertTextContains {
                        locator: locator.clone(),
                        expected_template,
                    });
                }
                RecordedActionKind::HumanCheckpoint { reason } => {
                    if reason.trim().is_empty() {
                        return Err(BotBrowserError::ScriptCompilation(format!(
                            "empty HumanCheckpoint reason at seq {}",
                            recorded.seq
                        )));
                    }
                    steps.push(ScriptStep::HumanCheckpoint {
                        reason: reason.trim().to_string(),
                    });
                }
            }
        }

        if steps.is_empty() {
            return Err(BotBrowserError::ScriptCompilation(
                "compiled script has zero steps".to_string(),
            ));
        }

        let app_id = meta
            .app_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("app_{}", Uuid::new_v4().simple()));

        let slug = normalize_slug(
            meta.slug
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(name),
        );

        let version = if meta.version.trim().is_empty() {
            "1.0.0".to_string()
        } else {
            meta.version.trim().to_string()
        };

        let category = if meta.category.trim().is_empty() {
            "general".to_string()
        } else {
            meta.category.trim().to_string()
        };

        let target_domains: Vec<String> = domains.into_iter().collect();
        let source_trace_summary = Some(format!(
            "trace={} steps={} domains={}",
            trace.trace_id,
            steps.len(),
            if target_domains.is_empty() {
                "none".to_string()
            } else {
                target_domains.join(",")
            }
        ));

        let app = AutomationScriptApp {
            schema_version: SCRIPT_APP_SCHEMA_VERSION,
            app_id,
            slug,
            name: name.to_string(),
            version,
            description: meta.description.trim().to_string(),
            category,
            target_domains,
            parameters: parameters.into_values().collect(),
            steps,
            required_capabilities: meta.required_capabilities,
            author_id: meta.author_id,
            source_trace_summary,
        };

        validate_script_app_safe_for_publish(&app)?;
        Ok(app)
    }
}

pub fn validate_script_app_safe_for_publish(
    app: &AutomationScriptApp,
) -> Result<(), BotBrowserError> {
    if app.schema_version == 0 {
        return Err(BotBrowserError::ScriptValidation(
            "schema_version must be >= 1".to_string(),
        ));
    }
    if app.app_id.trim().is_empty() {
        return Err(BotBrowserError::ScriptValidation(
            "app_id cannot be empty".to_string(),
        ));
    }
    if app.name.trim().is_empty() {
        return Err(BotBrowserError::ScriptValidation(
            "name cannot be empty".to_string(),
        ));
    }
    if app.version.trim().is_empty() {
        return Err(BotBrowserError::ScriptValidation(
            "version cannot be empty".to_string(),
        ));
    }
    if app.steps.is_empty() {
        return Err(BotBrowserError::ScriptValidation(
            "steps cannot be empty".to_string(),
        ));
    }
    if app.slug.trim().is_empty()
        || !app
            .slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(BotBrowserError::ScriptValidation(format!(
            "invalid slug '{}': must be non-empty [a-z0-9_-]",
            app.slug
        )));
    }

    let mut param_map: BTreeMap<&str, &ScriptParameter> = BTreeMap::new();
    for p in &app.parameters {
        let pname = p.name.trim();
        if pname.is_empty() {
            return Err(BotBrowserError::ScriptValidation(
                "parameter name cannot be empty".to_string(),
            ));
        }
        if param_map.insert(pname, p).is_some() {
            return Err(BotBrowserError::ScriptValidation(format!(
                "duplicate parameter name '{pname}'"
            )));
        }
        if p.is_secret && p.default_value.as_deref().is_some_and(|v| !v.is_empty()) {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "secret parameter '{pname}' must not carry a default_value"
            )));
        }
        if is_sensitive_param_name(pname)
            && (!p.is_secret || p.default_value.as_deref().is_some_and(|v| !v.is_empty()))
        {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "sensitive parameter '{pname}' must be marked is_secret=true with no default_value"
            )));
        }
    }

    let app_val = serde_json::to_value(app).map_err(|e| {
        BotBrowserError::ScriptValidation(format!("failed to serialize app for validation: {e}"))
    })?;
    scan_json_value_for_secret_leaks(&app_val, "app", &app.parameters)?;

    for (idx, step) in app.steps.iter().enumerate() {
        match step {
            ScriptStep::Navigate { url_template } => {
                let dummy_url = replace_placeholders_with(url_template, "example");
                let origin = NormalizedOrigin::parse(&dummy_url).map_err(|e| {
                    BotBrowserError::ScriptValidation(format!(
                        "step[{idx}] invalid navigate url_template: {e}"
                    ))
                })?;
                if origin.host() == "169.254.169.254"
                    || origin.host() == "metadata.google.internal"
                {
                    return Err(BotBrowserError::ScriptValidation(format!(
                        "step[{idx}] blocked metadata URL"
                    )));
                }
            }
            ScriptStep::WaitFor {
                locator,
                timeout_ms,
            } => {
                validate_step_locator_non_empty(locator, idx)?;
                if *timeout_ms == 0 {
                    return Err(BotBrowserError::ScriptValidation(format!(
                        "step[{idx}] WaitFor timeout_ms must be > 0"
                    )));
                }
            }
            ScriptStep::Click { locator } | ScriptStep::AssertVisible { locator } => {
                validate_step_locator_non_empty(locator, idx)?;
            }
            ScriptStep::Fill {
                locator,
                value_template,
                secret_vault_key,
            } => {
                validate_step_locator_non_empty(locator, idx)?;

                let refs_secret_param = template_uses_secret_param(value_template, &app.parameters);
                if locator.is_sensitive_hint() || refs_secret_param {
                    let has_vault_key = secret_vault_key
                        .as_deref()
                        .is_some_and(|k| !k.trim().is_empty());
                    let pure_secret_placeholder = extract_pure_placeholder(value_template)
                        .and_then(|name| param_map.get(name))
                        .is_some_and(|p| p.is_secret && p.default_value.is_none());

                    let safe_with_vault =
                        has_vault_key && (value_template.trim().is_empty() || pure_secret_placeholder);

                    if !safe_with_vault && !pure_secret_placeholder {
                        return Err(BotBrowserError::SecretLeakDetected(format!(
                            "step[{idx}] sensitive Fill must use secret_vault_key or an is_secret=true placeholder with no default"
                        )));
                    }
                }
            }
            ScriptStep::PressKey { key } => {
                if key.trim().is_empty() {
                    return Err(BotBrowserError::ScriptValidation(format!(
                        "step[{idx}] PressKey cannot be empty"
                    )));
                }
            }
            ScriptStep::Scroll { .. } => {}
            ScriptStep::ExtractText {
                locator,
                output_key,
            } => {
                validate_step_locator_non_empty(locator, idx)?;
                if output_key.trim().is_empty() {
                    return Err(BotBrowserError::ScriptValidation(format!(
                        "step[{idx}] ExtractText output_key cannot be empty"
                    )));
                }
            }
            ScriptStep::AssertTextContains { locator, .. } => {
                validate_step_locator_non_empty(locator, idx)?;
            }
            ScriptStep::HumanCheckpoint { reason } => {
                if reason.trim().is_empty() {
                    return Err(BotBrowserError::ScriptValidation(format!(
                        "step[{idx}] HumanCheckpoint reason cannot be empty"
                    )));
                }
            }
        }
    }

    Ok(())
}

struct ControlGateGuard<'a> {
    gate: Option<&'a BrowserControlGate>,
}

impl<'a> ControlGateGuard<'a> {
    fn disarm(&mut self) {
        self.gate = None;
    }
}

impl Drop for ControlGateGuard<'_> {
    fn drop(&mut self) {
        if let Some(gate) = self.gate {
            gate.complete_ai();
        }
    }
}

pub struct DeterministicScriptRunner;

impl DeterministicScriptRunner {
    pub async fn run(
        backend: &dyn BrowserSessionBackend,
        app: &AutomationScriptApp,
        params: &BTreeMap<String, String>,
        vault_resolver: Option<&dyn VaultSecretResolver>,
        control: Option<&BrowserControlGate>,
    ) -> Result<ScriptReplayReport, BotBrowserError> {
        if app.steps.is_empty() {
            return Err(BotBrowserError::ScriptReplay(
                "cannot execute AutomationScriptApp with zero steps".to_string(),
            ));
        }

        // Verify all required non-vault parameters are supplied or have non-secret defaults.
        for p in &app.parameters {
            if p.required && !p.is_secret {
                let supplied = params.get(&p.name).is_some();
                let has_default = p.default_value.is_some();
                if !supplied && !has_default {
                    return Err(BotBrowserError::ScriptReplay(format!(
                        "missing required script parameter '{}'",
                        p.name
                    )));
                }
            }
        }

        let run_start = Instant::now();
        if let Some(gate) = control {
            if gate.is_cancelled() {
                return Err(BotBrowserError::ControlCancelled(
                    "cancelled before start".to_string(),
                ));
            }
            gate.start_ai(format!("Running {}", app.name));
        }
        let mut gate_guard = ControlGateGuard { gate: control };

        let mut extracted_outputs: BTreeMap<String, String> = BTreeMap::new();
        let mut step_timings: Vec<StepTimingMetric> = Vec::with_capacity(app.steps.len());

        for (idx, step) in app.steps.iter().enumerate() {
            let label = step_label(step);
            let kind_name = step_kind_name(step);

            if let Some(gate) = control {
                gate.set_active_step(idx, &label);
                if let Err(err) = gate.wait_if_paused_or_cancelled(idx).await {
                    gate.complete_ai();
                    gate_guard.disarm();
                    return Err(err);
                }
            }

            let step_start = Instant::now();
            let step_res: Result<(), BotBrowserError> = async {
                match step {
                    ScriptStep::Navigate { url_template } => {
                        let rendered = render_template(url_template, params, &app.parameters)?;
                        let _ = NormalizedOrigin::parse(&rendered)?;
                        backend.navigate(&rendered).await.map(|_| ())
                    }
                    ScriptStep::WaitFor {
                        locator,
                        timeout_ms,
                    } => backend
                        .wait_for_locator(locator, *timeout_ms)
                        .await
                        .map(|_| ()),
                    ScriptStep::Click { locator } => backend.click_locator(locator).await.map(|_| ()),
                    ScriptStep::Fill {
                        locator,
                        value_template,
                        secret_vault_key,
                    } => {
                        let is_secret = secret_vault_key.is_some()
                            || locator.is_sensitive_hint()
                            || template_uses_secret_param(value_template, &app.parameters);

                        let resolved_value: Zeroizing<String> = if let Some(vault_key) =
                            secret_vault_key
                        {
                            let current_origin = backend
                                .current_url()
                                .await
                                .ok()
                                .and_then(|u| NormalizedOrigin::parse(&u).ok())
                                .map(|o| o.as_str().to_string());

                            let fallback_secret_value = extract_pure_placeholder(value_template)
                                .and_then(|pname| {
                                    app.parameters
                                        .iter()
                                        .find(|p| {
                                            p.name == pname
                                                && p.is_secret
                                                && p.default_value.is_none()
                                        })
                                        .and_then(|p| params.get(&p.name))
                                        .cloned()
                                });

                            let mut from_vault = None;
                            if let Some(resolver) = vault_resolver {
                                match resolver
                                    .resolve_secret(vault_key, current_origin.as_deref())
                                    .await
                                {
                                    Ok(secret) => from_vault = Some(secret),
                                    Err(BotBrowserError::CredentialNotFound(_))
                                        if fallback_secret_value.is_some() => {}
                                    Err(e) => Err(e)?,
                                }
                            }

                            match from_vault {
                                Some(secret) => secret,
                                None => match fallback_secret_value {
                                    Some(val) => Zeroizing::new(val),
                                    None => Err(BotBrowserError::CredentialNotFound(
                                        vault_key.clone(),
                                    ))?,
                                },
                            }
                        } else {
                            Zeroizing::new(render_template(
                                value_template,
                                params,
                                &app.parameters,
                            )?)
                        };

                        backend
                            .fill_locator(locator, resolved_value.as_str(), is_secret)
                            .await
                            .map(|_| ())
                    }
                    ScriptStep::PressKey { key } => backend.press_key(key).await.map(|_| ()),
                    ScriptStep::Scroll { delta_x, delta_y } => {
                        backend.scroll(*delta_x, *delta_y).await.map(|_| ())
                    }
                    ScriptStep::ExtractText {
                        locator,
                        output_key,
                    } => {
                        let txt = backend.extract_text_locator(locator).await?;
                        extracted_outputs.insert(output_key.clone(), txt);
                        Ok(())
                    }
                    ScriptStep::AssertVisible { locator } => {
                        backend.wait_for_locator(locator, 2_000).await.map(|_| ())
                    }
                    ScriptStep::AssertTextContains {
                        locator,
                        expected_template,
                    } => {
                        let expected =
                            render_template(expected_template, params, &app.parameters)?;
                        let actual = backend.extract_text_locator(locator).await?;
                        if !actual.contains(&expected) {
                            let expected_display = if template_uses_secret_param(
                                expected_template,
                                &app.parameters,
                            ) || locator.is_sensitive_hint()
                            {
                                "[REDACTED]"
                            } else {
                                expected.as_str()
                            };
                            Err(BotBrowserError::ScriptReplay(format!(
                                "AssertTextContains failed for {}: expected substring '{}', got '{}'",
                                locator.display_label(),
                                expected_display,
                                actual
                            )))
                        } else {
                            Ok(())
                        }
                    }
                    ScriptStep::HumanCheckpoint { reason } => {
                        if let Some(gate) = control {
                            gate.request_takeover(reason.clone());
                            gate.wait_if_paused_or_cancelled(idx).await
                        } else {
                            Err(BotBrowserError::ScriptReplay(format!(
                                "human checkpoint '{reason}' requires BrowserControlGate"
                            )))
                        }
                    }
                }
            }
            .await;

            let duration_ms = step_start.elapsed().as_millis() as u64;
            let succeeded = step_res.is_ok();
            step_timings.push(StepTimingMetric {
                step_index: Some(idx),
                operation: kind_name.to_string(),
                label,
                backend: Some(backend.backend_kind()),
                duration_ms,
                success: succeeded,
                timestamp_ms: Utc::now().timestamp_millis(),
                detail: step_res.as_ref().err().map(ToString::to_string),
            });

            if let Err(err) = step_res {
                if let Some(gate) = control {
                    gate.complete_ai();
                }
                gate_guard.disarm();

                if matches!(
                    err,
                    BotBrowserError::ControlCancelled(_)
                        | BotBrowserError::ApprovalRequired { .. }
                        | BotBrowserError::CredentialNotFound(_)
                ) {
                    return Err(err);
                }

                let final_url = backend.current_url().await.ok();
                return Ok(ScriptReplayReport {
                    app_id: app.app_id.clone(),
                    version: app.version.clone(),
                    success: false,
                    steps_executed: step_timings.len(),
                    extracted_outputs,
                    step_timings,
                    total_duration_ms: run_start.elapsed().as_millis() as u64,
                    used_llm: false,
                    final_url,
                    error: Some(err.to_string()),
                });
            }
        }

        if let Some(gate) = control {
            gate.complete_ai();
        }
        gate_guard.disarm();

        let final_url = backend.current_url().await.ok();
        Ok(ScriptReplayReport {
            app_id: app.app_id.clone(),
            version: app.version.clone(),
            success: true,
            steps_executed: app.steps.len(),
            extracted_outputs,
            step_timings,
            total_duration_ms: run_start.elapsed().as_millis() as u64,
            used_llm: false,
            final_url,
            error: None,
        })
    }
}

fn validate_secret_vault_key_at(
    vk: &str,
    context: &str,
    params: &[ScriptParameter],
    depth: usize,
) -> Result<(), BotBrowserError> {
    if vk.contains('=') {
        return Err(BotBrowserError::SecretLeakDetected(format!(
            "plaintext '=' assignment detected in {context}"
        )));
    }
    let trimmed = vk.trim();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(after_basic_idx) = lower.find("basic ") {
        let valid_boundary = after_basic_idx == 0
            || !lower.as_bytes()[after_basic_idx - 1].is_ascii_alphanumeric();
        if valid_boundary {
            let after_basic = &trimmed[after_basic_idx + 6..];
            let tok = after_basic.split_whitespace().next().unwrap_or("");
            if tok.len() >= 8
                && tok.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-' | ':')
                })
                && tok.chars().any(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext Basic credential detected in {context}"
                )));
            }
        }
    }

    let scan_target = trimmed
        .strip_suffix(":password")
        .or_else(|| trimmed.strip_suffix(":username"))
        .or_else(|| trimmed.strip_suffix(":totp"))
        .unwrap_or(trimmed);
    scan_text_for_secret_leaks_with_depth(scan_target, context, params, depth)
}

fn validate_step_locator_non_empty(
    locator: &SemanticLocator,
    idx: usize,
) -> Result<(), BotBrowserError> {
    if locator.is_empty() {
        return Err(BotBrowserError::ScriptValidation(format!(
            "step[{idx}] locator cannot be empty"
        )));
    }
    Ok(())
}

fn unique_param_name(
    base: &str,
    want_secret: bool,
    expected_non_secret_default: Option<&str>,
    parameters: &BTreeMap<String, ScriptParameter>,
) -> String {
    let matches_existing = |cand: &str| -> bool {
        match parameters.get(cand) {
            None => true,
            Some(existing) => {
                if existing.is_secret != want_secret {
                    return false;
                }
                if want_secret {
                    existing.default_value.is_none()
                } else {
                    match expected_non_secret_default {
                        Some(val) => existing.default_value.as_deref() == Some(val),
                        None => true,
                    }
                }
            }
        }
    };
    if matches_existing(base) {
        return base.to_string();
    }
    let suffix = if want_secret { "secret" } else { "param" };
    let first_try = format!("{base}_{suffix}");
    if matches_existing(&first_try) {
        return first_try;
    }
    let mut counter = 2usize;
    loop {
        let cand = format!("{base}_{suffix}_{counter}");
        if matches_existing(&cand) {
            return cand;
        }
        counter += 1;
    }
}

fn sanitize_param_name(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

fn normalize_slug(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "automation-script".to_string()
    } else {
        trimmed.to_string()
    }
}

fn encode_query_component_preserving_placeholders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let prefix = &rest[..start];
        if !prefix.is_empty() {
            for chunk in url::form_urlencoded::byte_serialize(prefix.as_bytes()) {
                out.push_str(chunk);
            }
        }
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("}}") {
            out.push_str(&rest[start..start + 2 + end + 2]);
            rest = &after_start[end + 2..];
        } else {
            for chunk in url::form_urlencoded::byte_serialize(rest[start..].as_bytes()) {
                out.push_str(chunk);
            }
            rest = "";
            break;
        }
    }
    if !rest.is_empty() {
        for chunk in url::form_urlencoded::byte_serialize(rest.as_bytes()) {
            out.push_str(chunk);
        }
    }
    out
}

fn restore_encoded_placeholders(s: &str, parameters: &BTreeMap<String, ScriptParameter>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if i + 6 <= bytes.len() && s[i..i + 6].eq_ignore_ascii_case("%7b%7b") {
            if let Some(rel_end) = s[i + 6..]
                .to_ascii_lowercase()
                .find("%7d%7d")
            {
                let name = &s[i + 6..i + 6 + rel_end];
                if !name.is_empty()
                    && (parameters.contains_key(name)
                        || name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_'))
                {
                    out.push_str("{{");
                    out.push_str(name);
                    out.push_str("}}");
                    i += 6 + rel_end + 6;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn replace_outside_placeholders(input: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() || !input.contains(needle) {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        let before = &rest[..start];
        out.push_str(&before.replace(needle, replacement));
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("}}") {
            out.push_str(&rest[start..start + 2 + end + 2]);
            rest = &after_start[end + 2..];
        } else {
            out.push_str(&rest[start..].replace(needle, replacement));
            rest = "";
            break;
        }
    }
    if !rest.is_empty() {
        out.push_str(&rest.replace(needle, replacement));
    }
    out
}

fn sanitize_navigate_url_template(
    raw_url: &str,
    bindings: &[TraceParameterBinding],
    parameters: &mut BTreeMap<String, ScriptParameter>,
) -> Result<String, BotBrowserError> {
    let mut url_out = raw_url.to_string();
    if let Ok(parsed) = Url::parse(&url_out) {
        let mut new_query: Option<String> = None;
        let mut query_modified = false;

        if parsed.query().is_some() {
            let mut encoded_pairs: Vec<String> = Vec::new();
            for (k, v) in parsed.query_pairs() {
                let k_str = k.as_ref();
                let v_str = v.as_ref();
                if (is_sensitive_param_name(k_str) || contains_secret_material(v_str))
                    && extract_pure_placeholder(v_str).is_none()
                {
                    let base_pname = {
                        let s = sanitize_param_name(k_str);
                        if s.is_empty() {
                            "secret_query_param".to_string()
                        } else {
                            s
                        }
                    };
                    let pname = unique_param_name(&base_pname, true, None, parameters);
                    parameters.entry(pname.clone()).or_insert_with(|| ScriptParameter {
                        name: pname.clone(),
                        description: format!("Sensitive query parameter '{k_str}'"),
                        required: true,
                        default_value: None,
                        is_secret: true,
                    });
                    let enc_k = encode_query_component_preserving_placeholders(k_str);
                    encoded_pairs.push(format!("{enc_k}={{{{{pname}}}}}"));
                    query_modified = true;
                } else {
                    let enc_k = encode_query_component_preserving_placeholders(k_str);
                    let enc_v = encode_query_component_preserving_placeholders(v_str);
                    encoded_pairs.push(format!("{enc_k}={enc_v}"));
                }
            }
            if query_modified {
                new_query = Some(encoded_pairs.join("&"));
            }
        }

        let orig_fragment = parsed.fragment().map(ToString::to_string);
        let mut new_fragment: Option<String> = orig_fragment.clone();
        let mut fragment_modified = false;

        if let Some(frag) = orig_fragment.as_deref() {
            if !frag.is_empty() {
                let mut frag_parts: Vec<String> = Vec::new();
                for part in frag.split('&') {
                    if extract_pure_placeholder(part).is_some() {
                        frag_parts.push(part.to_string());
                        continue;
                    }
                    let kv_candidate = if let Some((fk, fv)) = part.split_once('=') {
                        Some((fk.to_string(), fv.to_string(), fk.to_string()))
                    } else {
                        let decoded_part = percent_decode_for_scan(part);
                        decoded_part
                            .split_once('=')
                            .map(|(fk, fv)| {
                                (
                                    fk.to_string(),
                                    fv.to_string(),
                                    encode_query_component_preserving_placeholders(fk),
                                )
                            })
                    };

                    if let Some((fk, fv, fk_out)) = kv_candidate {
                        if (is_sensitive_param_name(&fk) || contains_secret_material(&fv))
                            && extract_pure_placeholder(&fv).is_none()
                        {
                            let base_pname = {
                                let s = sanitize_param_name(&fk);
                                if s.is_empty() {
                                    "secret_fragment_param".to_string()
                                } else {
                                    s
                                }
                            };
                            let pname = unique_param_name(&base_pname, true, None, parameters);
                            parameters.entry(pname.clone()).or_insert_with(|| ScriptParameter {
                                name: pname.clone(),
                                description: format!("Sensitive fragment parameter '{fk}'"),
                                required: true,
                                default_value: None,
                                is_secret: true,
                            });
                            frag_parts.push(format!("{fk_out}={{{{{pname}}}}}"));
                            fragment_modified = true;
                            continue;
                        }
                    }

                    if contains_secret_material(part) {
                        let pname = unique_param_name("secret_fragment", true, None, parameters);
                        parameters.entry(pname.clone()).or_insert_with(|| ScriptParameter {
                            name: pname.clone(),
                            description: "Sensitive URL fragment".to_string(),
                            required: true,
                            default_value: None,
                            is_secret: true,
                        });
                        frag_parts.push(format!("{{{{{pname}}}}}"));
                        fragment_modified = true;
                        continue;
                    }

                    frag_parts.push(part.to_string());
                }
                if fragment_modified {
                    new_fragment = Some(frag_parts.join("&"));
                }
            }
        }

        if query_modified || fragment_modified {
            let mut rebuilt = parsed.clone();
            rebuilt.set_query(None);
            rebuilt.set_fragment(None);
            let mut out = restore_encoded_placeholders(
                rebuilt.as_str().trim_end_matches('?'),
                parameters,
            );
            if let Some(q) = new_query.as_deref().or_else(|| parsed.query()) {
                out.push('?');
                out.push_str(q);
            }
            if let Some(f) = new_fragment.as_deref() {
                out.push('#');
                out.push_str(&restore_encoded_placeholders(f, parameters));
            }
            url_out = out;
        }
    }
    Ok(apply_non_secret_bindings(&url_out, bindings, parameters))
}

fn apply_non_secret_bindings(
    raw: &str,
    bindings: &[TraceParameterBinding],
    parameters: &mut BTreeMap<String, ScriptParameter>,
) -> String {
    let mut out = raw.to_string();
    for b in bindings {
        if b.literal_value.is_empty() {
            continue;
        }
        let replaced = replace_outside_placeholders(&out, &b.literal_value, "");
        if replaced.len() != out.len() {
            let base_pname = sanitize_param_name(&b.param_name);
            if base_pname.is_empty() {
                continue;
            }
            let want_secret = b.is_secret || is_sensitive_param_name(&base_pname);
            let expected_default = if want_secret {
                None
            } else {
                Some(b.literal_value.as_str())
            };
            let pname = unique_param_name(&base_pname, want_secret, expected_default, parameters);
            let placeholder = format!("{{{{{pname}}}}}");
            out = replace_outside_placeholders(&out, &b.literal_value, &placeholder);
            parameters.entry(pname.clone()).or_insert_with(|| ScriptParameter {
                name: pname,
                description: b.description.clone(),
                required: b.required,
                default_value: if want_secret {
                    None
                } else {
                    Some(b.literal_value.clone())
                },
                is_secret: want_secret,
            });
        }
    }
    out
}

fn render_template(
    template: &str,
    supplied: &BTreeMap<String, String>,
    declared: &[ScriptParameter],
) -> Result<String, BotBrowserError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let key = after_start[..end].trim();
        let val = if let Some(v) = supplied.get(key) {
            v.as_str()
        } else if let Some(param) = declared.iter().find(|p| p.name == key) {
            if let Some(def) = param.default_value.as_deref() {
                def
            } else {
                return Err(BotBrowserError::ScriptReplay(format!(
                    "missing value for template parameter '{key}'"
                )));
            }
        } else {
            return Err(BotBrowserError::ScriptReplay(format!(
                "missing value for template parameter '{key}'"
            )));
        };
        out.push_str(val);
        rest = &after_start[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn replace_placeholders_with(template: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("}}") {
            out.push_str(replacement);
            rest = &after[end + 2..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn extract_pure_placeholder(template: &str) -> Option<&str> {
    let trimmed = template.trim();
    let inner = trimmed.strip_prefix("{{")?.strip_suffix("}}")?.trim();
    if inner.is_empty() || inner.contains("{{") || inner.contains("}}") {
        return None;
    }
    Some(inner)
}

fn template_uses_secret_param(template: &str, params: &[ScriptParameter]) -> bool {
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let key = after[..end].trim();
        if params.iter().any(|p| p.name == key && p.is_secret)
            || is_sensitive_param_name(key)
        {
            return true;
        }
        rest = &after[end + 2..];
    }
    false
}

fn is_sensitive_param_name(name: &str) -> bool {
    if is_sensitive_json_key(name) {
        return true;
    }
    let segments = split_key_segments(name);
    if segments
        .iter()
        .any(|seg| matches!(seg.as_str(), "bearer" | "basic" | "pin" | "otp" | "totp"))
    {
        return true;
    }
    let lower = name.trim().to_ascii_lowercase();
    lower.contains("password")
        || lower.contains("passwd")
        || lower.contains("passcode")
        || lower.contains("secret")
        || lower.contains("totp")
        || lower.contains("apikey")
}

const MAX_JSON_DEPTH: usize = 16;

const SENSITIVE_KV_KEYS: &[&str] = &[
    "auth_token",
    "auth-token",
    "ct0",
    "password",
    "passwd",
    "passcode",
    "pwd",
    "api_key",
    "api-key",
    "apikey",
    "x-api-key",
    "x_api_key",
    "access_token",
    "refresh_token",
    "id_token",
    "session_id",
    "set-cookie",
    "cookie",
    "authorization",
    "token",
    "secret",
];

fn contains_secret_material(text: &str) -> bool {
    scan_text_for_secret_leaks(text, "input", &[]).is_err()
}

fn percent_decode_for_scan(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn parse_hex4(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> (String, Option<u16>) {
    let mut hex = String::with_capacity(4);
    for _ in 0..4 {
        if let Some(h) = chars.peek().copied().filter(|c| c.is_ascii_hexdigit()) {
            hex.push(h);
            chars.next();
        } else {
            break;
        }
    }
    let parsed = if hex.len() == 4 {
        u16::from_str_radix(&hex, 16).ok()
    } else {
        None
    };
    (hex, parsed)
}

fn unescape_json_for_scan(input: &str) -> String {
    if !input.contains('\\') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek().copied() {
                Some('"') => {
                    out.push('"');
                    chars.next();
                }
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                Some('/') => {
                    out.push('/');
                    chars.next();
                }
                Some('b') => {
                    out.push('\u{0008}');
                    chars.next();
                }
                Some('f') => {
                    out.push('\u{000C}');
                    chars.next();
                }
                Some('n') => {
                    out.push('\n');
                    chars.next();
                }
                Some('r') => {
                    out.push('\r');
                    chars.next();
                }
                Some('t') => {
                    out.push('\t');
                    chars.next();
                }
                Some('u') => {
                    chars.next();
                    let (hex, code_opt) = parse_hex4(&mut chars);
                    if let Some(cp) = code_opt {
                        if (0xD800..=0xDBFF).contains(&cp) {
                            let mut probe = chars.clone();
                            if probe.next() == Some('\\') && probe.next() == Some('u') {
                                let (_, lo_opt) = parse_hex4(&mut probe);
                                if let Some(lo) = lo_opt.filter(|v| (0xDC00..=0xDFFF).contains(v))
                                {
                                    let combined = 0x10000
                                        + (((cp as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                                    if let Some(decoded) = char::from_u32(combined) {
                                        chars = probe;
                                        out.push(decoded);
                                        continue;
                                    }
                                }
                            }
                        } else if let Some(decoded) = char::from_u32(cp as u32) {
                            out.push(decoded);
                            continue;
                        }
                    }
                    out.push_str("\\u");
                    out.push_str(&hex);
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn is_protected_placeholder(val: &str, params: &[ScriptParameter]) -> bool {
    extract_pure_placeholder(val)
        .and_then(|name| params.iter().find(|p| p.name.trim() == name))
        .is_some_and(|p| p.is_secret && p.default_value.is_none())
}

fn unwrap_clean_quote_pair(s: &str) -> Option<&str> {
    let t = s.trim();
    let q = t.chars().next().filter(|&c| c == '"' || c == '\'')?;
    if t.len() >= 2 && t.ends_with(q) {
        let inside = &t[1..t.len() - 1];
        if !inside.contains(['"', '\'']) {
            return Some(inside.trim());
        }
    }
    None
}

fn is_protected_auth_header_value(val: &str, params: &[ScriptParameter]) -> bool {
    let trimmed = val.trim();
    if trimmed.is_empty() || is_protected_placeholder(trimmed, params) {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("bearer")
        .or_else(|| lower.strip_prefix("basic"))
    {
        let scheme_len = lower.len() - rest.len();
        let after_scheme = &trimmed[scheme_len..];
        if after_scheme
            .chars()
            .next()
            .is_some_and(|c| {
                c.is_whitespace() || matches!(c, ':' | '=' | '"' | '\'' | '(' | '[' | '{' | '<')
            })
        {
            let s1 = after_scheme.trim_start();
            let after_assign = if s1.starts_with(':') || s1.starts_with('=') {
                s1[1..].trim_start()
            } else {
                s1
            };
            if after_assign.is_empty() {
                return true;
            }
            if is_protected_placeholder(after_assign, params) {
                return true;
            }
            if let Some(inner) = unwrap_balanced_enclosure(after_assign) {
                return is_protected_placeholder(inner, params)
                    || unwrap_clean_quote_pair(inner)
                        .is_some_and(|q_inner| is_protected_placeholder(q_inner, params));
            }
            if let Some(inner) = unwrap_clean_quote_pair(after_assign) {
                return is_protected_placeholder(inner, params)
                    || unwrap_balanced_enclosure(inner)
                        .is_some_and(|b| is_protected_placeholder(b, params));
            }
        }
    }
    false
}

fn is_array_element_context(context: &str, prefix: &str) -> bool {
    context
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|idx| !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit()))
}

fn is_canonical_step_secret_vault_key(context: &str, raw_key: &str) -> bool {
    raw_key == "secret_vault_key"
        && (is_array_element_context(context, "app.steps[")
            || is_array_element_context(context, "request.manifest.steps["))
}

fn is_canonical_param_is_secret(context: &str, raw_key: &str, val: &serde_json::Value) -> bool {
    raw_key == "is_secret"
        && val.is_boolean()
        && (is_array_element_context(context, "app.parameters[")
            || is_array_element_context(context, "request.manifest.parameters["))
}

fn split_key_segments(raw_key: &str) -> Vec<String> {
    let chars: Vec<char> = raw_key.trim().chars().collect();
    let mut segments = Vec::new();
    let mut cur = String::new();
    for (idx, &ch) in chars.iter().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                segments.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if !cur.is_empty() {
            let prev = chars[idx - 1];
            if ch.is_ascii_uppercase() {
                let next_is_lower = chars.get(idx + 1).is_some_and(|n| n.is_ascii_lowercase());
                if prev.is_ascii_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_ascii_uppercase() && next_is_lower)
                {
                    segments.push(std::mem::take(&mut cur));
                }
            } else if (ch.is_ascii_digit() && prev.is_ascii_alphabetic())
                || (ch.is_ascii_lowercase() && prev.is_ascii_digit())
            {
                segments.push(std::mem::take(&mut cur));
            }
        }
        cur.push(ch.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        segments.push(cur);
    }
    segments
}

fn is_sensitive_json_key(raw_key: &str) -> bool {
    let segments = split_key_segments(raw_key);
    is_sensitive_json_key_with_segments(raw_key, &segments)
}

fn is_sensitive_json_key_with_segments(raw_key: &str, segments: &[String]) -> bool {
    const SINGLE_SEGMENTS: &[&str] = &[
        "ct0",
        "password",
        "passwd",
        "passcode",
        "pwd",
        "apikey",
        "authorization",
        "token",
        "secret",
        "cookie",
    ];
    if segments
        .iter()
        .any(|seg| SINGLE_SEGMENTS.contains(&seg.as_str()))
    {
        return true;
    }
    const PAIR_SEGMENTS: &[(&str, &str)] = &[
        ("ct", "0"),
        ("auth", "token"),
        ("api", "key"),
        ("access", "token"),
        ("refresh", "token"),
        ("id", "token"),
        ("session", "id"),
        ("set", "cookie"),
        ("private", "key"),
        ("pass", "word"),
        ("pass", "wd"),
        ("pass", "code"),
    ];
    if segments
        .windows(2)
        .any(|pair| PAIR_SEGMENTS.contains(&(pair[0].as_str(), pair[1].as_str())))
    {
        return true;
    }
    if raw_key.contains(['_', '-']) {
        let k_lower = raw_key.trim().to_ascii_lowercase();
        if SENSITIVE_KV_KEYS.iter().any(|key| {
            k_lower == *key
                || k_lower
                    .strip_suffix(key)
                    .is_some_and(|prefix| prefix.ends_with('_') || prefix.ends_with('-'))
                || k_lower
                    .strip_prefix(key)
                    .is_some_and(|suffix| suffix.starts_with('_') || suffix.starts_with('-'))
        }) {
            return true;
        }
    }
    false
}

fn is_safe_bearer_or_basic_token_with_braces(tok: &str, params: &[ScriptParameter]) -> bool {
    if is_protected_placeholder(tok, params) {
        return true;
    }
    let trimmed = tok.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | ',' | ';' | '.' | '!' | '?'
        )
    });
    if is_protected_placeholder(trimmed, params) {
        return true;
    }
    if let Some((lhs, rhs)) = trimmed.split_once([':', '=']) {
        if !lhs.is_empty()
            && !lhs.contains(['{', '}', '"', '\''])
            && !rhs.is_empty()
            && is_protected_placeholder(rhs, params)
        {
            return true;
        }
    }
    false
}

fn is_safe_bearer_or_basic_value(
    s: &str,
    is_bearer: bool,
    is_basic: bool,
    params: &[ScriptParameter],
) -> bool {
    let s_trim = s.trim();
    if s_trim.is_empty() || is_protected_placeholder(s_trim, params) {
        return true;
    }
    for raw_tok in s_trim.split_whitespace() {
        if raw_tok.contains("{{") || raw_tok.contains("}}") {
            if !is_safe_bearer_or_basic_token_with_braces(raw_tok, params) {
                return false;
            }
            continue;
        }
        if is_basic && contains_basic_credential_token(raw_tok) {
            return false;
        }
        if is_bearer {
            let core = raw_tok.trim_matches(|c: char| {
                matches!(
                    c,
                    '(' | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | '"'
                        | '\''
                        | ','
                        | ';'
                        | '.'
                        | '!'
                        | '?'
                        | ':'
                        | '='
                )
            });
            if core.is_empty() {
                continue;
            }
            let ascii_non_ws = core.bytes().filter(|b| b.is_ascii_graphic()).count();
            let has_digit_or_punct = core
                .bytes()
                .any(|b| b.is_ascii_digit() || b.is_ascii_punctuation());
            if !is_allowed_bearer_prose_noun(core)
                && ((ascii_non_ws >= 6 && has_digit_or_punct) || ascii_non_ws >= 12)
            {
                return false;
            }
        }
    }
    true
}

pub fn scan_json_value_for_secret_leaks(
    val: &serde_json::Value,
    context: &str,
    params: &[ScriptParameter],
) -> Result<(), BotBrowserError> {
    scan_json_value_with_depth(val, context, params, 0)
}

fn scan_json_value_with_depth(
    val: &serde_json::Value,
    context: &str,
    params: &[ScriptParameter],
    depth: usize,
) -> Result<(), BotBrowserError> {
    if depth > MAX_JSON_DEPTH {
        return Err(BotBrowserError::SecretLeakDetected(format!(
            "maximum JSON nesting depth ({MAX_JSON_DEPTH}) exceeded in {context}"
        )));
    }
    match val {
        serde_json::Value::String(s) => {
            scan_text_for_secret_leaks_with_depth(s, context, params, depth)
        }
        serde_json::Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                scan_json_value_with_depth(item, &format!("{context}[{idx}]"), params, depth + 1)?;
            }
            Ok(())
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                scan_text_for_secret_leaks_with_depth(
                    k,
                    &format!("{context}.<key>"),
                    params,
                    depth,
                )?;
                if is_canonical_param_is_secret(context, k, v) {
                    continue;
                }
                if is_canonical_step_secret_vault_key(context, k) {
                    let vk_ctx = format!("{context}.{k}");
                    match v {
                        serde_json::Value::Null => {}
                        serde_json::Value::String(vk) => {
                            validate_secret_vault_key_at(vk, &vk_ctx, params, depth + 1)?;
                        }
                        _ => {
                            return Err(BotBrowserError::SecretLeakDetected(format!(
                                "invalid non-string secret_vault_key detected in {context}"
                            )));
                        }
                    }
                    continue;
                }
                let k_trimmed = k.trim();
                let k_lower = k_trimmed.to_ascii_lowercase();
                let key_segments = split_key_segments(k);
                let has_bearer_seg =
                    k_lower == "bearer" || key_segments.iter().any(|seg| seg == "bearer");
                let has_basic_seg =
                    k_lower == "basic" || key_segments.iter().any(|seg| seg == "basic");
                if has_bearer_seg || has_basic_seg {
                    let safe = match v {
                        serde_json::Value::Null => true,
                        serde_json::Value::String(s) => {
                            if !is_safe_bearer_or_basic_value(
                                s,
                                has_bearer_seg,
                                has_basic_seg,
                                params,
                            ) {
                                false
                            } else {
                                let bearer_ok = !has_bearer_seg
                                    || scan_text_for_secret_leaks_with_depth(
                                        &format!("Bearer {s}"),
                                        context,
                                        params,
                                        depth + 1,
                                    )
                                    .is_ok();
                                let basic_ok = !has_basic_seg
                                    || scan_text_for_secret_leaks_with_depth(
                                        &format!("Basic {s}"),
                                        context,
                                        params,
                                        depth + 1,
                                    )
                                    .is_ok();
                                bearer_ok && basic_ok
                            }
                        }
                        _ => false,
                    };
                    if !safe {
                        return Err(BotBrowserError::SecretLeakDetected(format!(
                            "sensitive JSON key '{k}' with non-placeholder value detected in {context}"
                        )));
                    }
                }
                if is_sensitive_json_key(k) {
                    let safe = match v {
                        serde_json::Value::Null => true,
                        serde_json::Value::String(s) => {
                            s.trim().is_empty()
                                || is_protected_placeholder(s, params)
                                || ((k_lower == "authorization"
                                    || k_lower.ends_with("-authorization")
                                    || k_lower.ends_with("_authorization"))
                                    && is_protected_auth_header_value(s, params))
                        }
                        _ => false,
                    };
                    if !safe {
                        return Err(BotBrowserError::SecretLeakDetected(format!(
                            "sensitive JSON key '{k}' with non-placeholder value detected in {context}"
                        )));
                    }
                }
                scan_json_value_with_depth(v, &format!("{context}.{k}"), params, depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_json_kv_entry(
    k: &str,
    context: &str,
    params: &[ScriptParameter],
    depth: usize,
    is_null: bool,
    str_val: Option<&str>,
) -> Result<(), BotBrowserError> {
    scan_text_for_secret_leaks_with_depth(k, &format!("{context}.<key>"), params, depth)?;
    let k_trimmed = k.trim();
    let k_lower = k_trimmed.to_ascii_lowercase();
    let key_segments = split_key_segments(k);
    let has_bearer_seg = k_lower == "bearer" || key_segments.iter().any(|seg| seg == "bearer");
    let has_basic_seg = k_lower == "basic" || key_segments.iter().any(|seg| seg == "basic");
    if has_bearer_seg || has_basic_seg {
        let safe = if is_null {
            true
        } else if let Some(s) = str_val {
            if !is_safe_bearer_or_basic_value(s, has_bearer_seg, has_basic_seg, params) {
                false
            } else {
                let bearer_ok = !has_bearer_seg
                    || scan_text_for_secret_leaks_with_depth(
                        &format!("Bearer {s}"),
                        context,
                        params,
                        depth + 1,
                    )
                    .is_ok();
                let basic_ok = !has_basic_seg
                    || scan_text_for_secret_leaks_with_depth(
                        &format!("Basic {s}"),
                        context,
                        params,
                        depth + 1,
                    )
                    .is_ok();
                bearer_ok && basic_ok
            }
        } else {
            false
        };
        if !safe {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "sensitive JSON key '{k}' with non-placeholder value detected in {context}"
            )));
        }
    }
    if is_sensitive_json_key_with_segments(k, &key_segments) {
        let safe = if is_null {
            true
        } else if let Some(s) = str_val {
            s.trim().is_empty()
                || is_protected_placeholder(s, params)
                || ((k_lower == "authorization"
                    || k_lower.ends_with("-authorization")
                    || k_lower.ends_with("_authorization"))
                    && is_protected_auth_header_value(s, params))
        } else {
            false
        };
        if !safe {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "sensitive JSON key '{k}' with non-placeholder value detected in {context}"
            )));
        }
    }
    if let Some(s) = str_val {
        scan_text_for_secret_leaks_with_depth(s, &format!("{context}.{k}"), params, depth + 1)?;
    }
    Ok(())
}

fn scan_json_slice_with_duplicates(
    slice: &str,
    context: &str,
    params: &[ScriptParameter],
    base_depth: usize,
) -> Result<(), BotBrowserError> {
    struct ScanSeed<'a> {
        parent_key: Option<&'a str>,
        parent_ctx: &'a str,
        context: String,
        params: &'a [ScriptParameter],
        depth: usize,
        err_slot: &'a std::cell::RefCell<Option<BotBrowserError>>,
    }

    impl<'a> ScanSeed<'a> {
        fn record_err<E: serde::de::Error>(&self, err: BotBrowserError) -> E {
            let mut slot = self.err_slot.borrow_mut();
            if slot.is_none() {
                *slot = Some(err);
            }
            E::custom("secret_leak_detected")
        }

        fn check_primitive<E: serde::de::Error>(
            &self,
            is_null: bool,
            str_val: Option<&str>,
        ) -> Result<(), E> {
            if self.depth > MAX_JSON_DEPTH {
                return Err(self.record_err(BotBrowserError::SecretLeakDetected(format!(
                    "maximum JSON nesting depth ({MAX_JSON_DEPTH}) exceeded in {}",
                    self.context
                ))));
            }
            if let Some(k) = self.parent_key {
                if let Err(e) = check_json_kv_entry(
                    k,
                    self.parent_ctx,
                    self.params,
                    self.depth.saturating_sub(1),
                    is_null,
                    str_val,
                ) {
                    return Err(self.record_err(e));
                }
            } else if let Some(s) = str_val {
                if let Err(e) =
                    scan_text_for_secret_leaks_with_depth(s, &self.context, self.params, self.depth)
                {
                    return Err(self.record_err(e));
                }
            }
            Ok(())
        }
    }

    impl<'de, 'a> serde::de::DeserializeSeed<'de> for ScanSeed<'a> {
        type Value = ();

        fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
            if self.depth > MAX_JSON_DEPTH {
                return Err(self.record_err(BotBrowserError::SecretLeakDetected(format!(
                    "maximum JSON nesting depth ({MAX_JSON_DEPTH}) exceeded in {}",
                    self.context
                ))));
            }
            struct Vis<'a>(ScanSeed<'a>);
            impl<'de, 'a> serde::de::Visitor<'de> for Vis<'a> {
                type Value = ();

                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("any JSON value")
                }

                fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
                    self.0.check_primitive(false, None)
                }

                fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
                    self.0.check_primitive(false, None)
                }

                fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
                    self.0.check_primitive(false, None)
                }

                fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
                    self.0.check_primitive(false, None)
                }

                fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<(), E> {
                    self.0.check_primitive(false, Some(v))
                }

                fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
                    self.0.check_primitive(true, None)
                }

                fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
                    self.0.check_primitive(true, None)
                }

                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<(), A::Error> {
                    self.0.check_primitive(false, None)?;
                    let mut idx = 0usize;
                    while seq
                        .next_element_seed(ScanSeed {
                            parent_key: None,
                            parent_ctx: self.0.parent_ctx,
                            context: format!("{}[{idx}]", self.0.context),
                            params: self.0.params,
                            depth: self.0.depth + 1,
                            err_slot: self.0.err_slot,
                        })?
                        .is_some()
                    {
                        idx += 1;
                    }
                    Ok(())
                }

                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<(), A::Error> {
                    self.0.check_primitive(false, None)?;
                    while let Some(k) = map.next_key::<String>()? {
                        map.next_value_seed(ScanSeed {
                            parent_key: Some(&k),
                            parent_ctx: &self.0.context,
                            context: format!("{}.{k}", self.0.context),
                            params: self.0.params,
                            depth: self.0.depth + 1,
                            err_slot: self.0.err_slot,
                        })?;
                    }
                    Ok(())
                }
            }
            deserializer.deserialize_any(Vis(self))
        }
    }

    let err_slot = std::cell::RefCell::new(None);
    let mut de = serde_json::Deserializer::from_str(slice);
    let res = serde::de::DeserializeSeed::deserialize(
        ScanSeed {
            parent_key: None,
            parent_ctx: context,
            context: context.to_string(),
            params,
            depth: base_depth,
            err_slot: &err_slot,
        },
        &mut de,
    );
    if let Some(e) = err_slot.into_inner() {
        return Err(e);
    }
    let _ = res;
    Ok(())
}

enum IterJsonOutcome {
    Valid { consumed: usize },
    Invalid { fail_offset: usize },
}

fn parse_rfc8259_string(s: &str, start: usize) -> Result<(String, usize), usize> {
    let bytes = s.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return Err(start);
    }
    let mut out = String::new();
    let mut i = start + 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x20 {
            return Err(i);
        }
        if b == b'"' {
            return Ok((out, i + 1));
        }
        if b == b'\\' {
            i += 1;
            let Some(&esc) = bytes.get(i) else {
                return Err(bytes.len());
            };
            match esc {
                b'"' => {
                    out.push('"');
                    i += 1;
                }
                b'\\' => {
                    out.push('\\');
                    i += 1;
                }
                b'/' => {
                    out.push('/');
                    i += 1;
                }
                b'b' => {
                    out.push('\u{0008}');
                    i += 1;
                }
                b'f' => {
                    out.push('\u{000C}');
                    i += 1;
                }
                b'n' => {
                    out.push('\n');
                    i += 1;
                }
                b'r' => {
                    out.push('\r');
                    i += 1;
                }
                b't' => {
                    out.push('\t');
                    i += 1;
                }
                b'u' => {
                    i += 1;
                    let parse_4hex = |pos: usize| -> Result<u16, usize> {
                        if pos + 4 > bytes.len() {
                            return Err(bytes.len());
                        }
                        let hex_slice = std::str::from_utf8(&bytes[pos..pos + 4]).map_err(|_| pos)?;
                        u16::from_str_radix(hex_slice, 16).map_err(|_| pos)
                    };
                    let cp = parse_4hex(i)?;
                    i += 4;
                    if (0xD800..=0xDBFF).contains(&cp) {
                        if bytes.get(i) == Some(&b'\\') && bytes.get(i + 1) == Some(&b'u') {
                            let lo = parse_4hex(i + 2)?;
                            if (0xDC00..=0xDFFF).contains(&lo) {
                                i += 6;
                                let combined =
                                    0x10000 + (((cp as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                                out.push(char::from_u32(combined).unwrap_or(char::REPLACEMENT_CHARACTER));
                            } else {
                                return Err(i);
                            }
                        } else {
                            return Err(i);
                        }
                    } else if (0xDC00..=0xDFFF).contains(&cp) {
                        return Err(i - 4);
                    } else {
                        out.push(char::from_u32(cp as u32).unwrap_or(char::REPLACEMENT_CHARACTER));
                    }
                }
                _ => return Err(i),
            }
            continue;
        }
        if b < 0x80 {
            out.push(b as char);
            i += 1;
        } else {
            let ch = s[i..].chars().next().ok_or(i)?;
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Err(bytes.len())
}

fn parse_rfc8259_number(bytes: &[u8], start: usize) -> Result<usize, usize> {
    let mut i = start;
    if bytes.get(i) == Some(&b'-') {
        i += 1;
    }
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
                i += 1;
            }
        }
        _ => return Err(i.min(bytes.len())),
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        if !bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            return Err(i.min(bytes.len()));
        }
        while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            return Err(i.min(bytes.len()));
        }
        while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            i += 1;
        }
    }
    Ok(i)
}

fn scan_json_slice_iteratively(
    slice: &str,
    context: &str,
    params: &[ScriptParameter],
    base_depth: usize,
) -> Result<IterJsonOutcome, BotBrowserError> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ObjState {
        ExpectKeyOrEnd,
        ExpectKey,
        ExpectColon,
        ExpectValue,
        ExpectCommaOrEnd,
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ArrState {
        ExpectValueOrEnd,
        ExpectValue,
        ExpectCommaOrEnd,
    }
    enum StackFrame {
        Object { key: Option<String>, state: ObjState },
        Array { state: ArrState },
    }

    let bytes = slice.as_bytes();
    let skip_ws = |mut p: usize| -> usize {
        while bytes
            .get(p)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            p += 1;
        }
        p
    };

    let mut pos = skip_ws(0);
    let mut stack: Vec<StackFrame> = Vec::new();
    match bytes.get(pos) {
        Some(b'{') => {
            pos += 1;
            stack.push(StackFrame::Object {
                key: None,
                state: ObjState::ExpectKeyOrEnd,
            });
        }
        Some(b'[') => {
            pos += 1;
            stack.push(StackFrame::Array {
                state: ArrState::ExpectValueOrEnd,
            });
        }
        _ => {
            return Ok(IterJsonOutcome::Invalid {
                fail_offset: pos.min(bytes.len()),
            });
        }
    }

    let mut max_depth_seen = base_depth;
    let check_pending_kv = |frame_opt: Option<&mut StackFrame>,
                            is_null: bool,
                            str_val: Option<&str>,
                            cur_depth: usize|
     -> Result<(), BotBrowserError> {
        let safe_depth = if cur_depth > MAX_JSON_DEPTH {
            1
        } else {
            cur_depth
        };
        if let Some(StackFrame::Object { key, .. }) = frame_opt {
            if let Some(k) = key.take() {
                check_json_kv_entry(&k, context, params, safe_depth.saturating_sub(1), is_null, str_val)?;
                return Ok(());
            }
        }
        if let Some(s) = str_val {
            scan_text_for_secret_leaks_with_depth(s, context, params, safe_depth)?;
        }
        Ok(())
    };

    while !stack.is_empty() {
        pos = skip_ws(pos);
        let expect_val = match stack.last_mut().unwrap() {
            StackFrame::Object { state, .. } => match *state {
                ObjState::ExpectKeyOrEnd => {
                    if bytes.get(pos) == Some(&b'}') {
                        pos += 1;
                        stack.pop();
                        continue;
                    }
                    if bytes.get(pos) == Some(&b'"') {
                        match parse_rfc8259_string(slice, pos) {
                            Ok((k, next_pos)) => {
                                pos = next_pos;
                                if let Some(StackFrame::Object { key, state }) = stack.last_mut() {
                                    *key = Some(k);
                                    *state = ObjState::ExpectColon;
                                }
                                continue;
                            }
                            Err(err_pos) => {
                                return Ok(IterJsonOutcome::Invalid {
                                    fail_offset: err_pos,
                                });
                            }
                        }
                    }
                    return Ok(IterJsonOutcome::Invalid {
                        fail_offset: pos.min(bytes.len()),
                    });
                }
                ObjState::ExpectKey => {
                    if bytes.get(pos) == Some(&b'"') {
                        match parse_rfc8259_string(slice, pos) {
                            Ok((k, next_pos)) => {
                                pos = next_pos;
                                if let Some(StackFrame::Object { key, state }) = stack.last_mut() {
                                    *key = Some(k);
                                    *state = ObjState::ExpectColon;
                                }
                                continue;
                            }
                            Err(err_pos) => {
                                return Ok(IterJsonOutcome::Invalid {
                                    fail_offset: err_pos,
                                });
                            }
                        }
                    }
                    return Ok(IterJsonOutcome::Invalid {
                        fail_offset: pos.min(bytes.len()),
                    });
                }
                ObjState::ExpectColon => {
                    if bytes.get(pos) == Some(&b':') {
                        pos += 1;
                        *state = ObjState::ExpectValue;
                        continue;
                    }
                    return Ok(IterJsonOutcome::Invalid {
                        fail_offset: pos.min(bytes.len()),
                    });
                }
                ObjState::ExpectValue => true,
                ObjState::ExpectCommaOrEnd => match bytes.get(pos) {
                    Some(b',') => {
                        pos += 1;
                        *state = ObjState::ExpectKey;
                        continue;
                    }
                    Some(b'}') => {
                        pos += 1;
                        stack.pop();
                        continue;
                    }
                    _ => {
                        return Ok(IterJsonOutcome::Invalid {
                            fail_offset: pos.min(bytes.len()),
                        });
                    }
                },
            },
            StackFrame::Array { state } => match *state {
                ArrState::ExpectValueOrEnd => {
                    if bytes.get(pos) == Some(&b']') {
                        pos += 1;
                        stack.pop();
                        continue;
                    }
                    true
                }
                ArrState::ExpectValue => true,
                ArrState::ExpectCommaOrEnd => match bytes.get(pos) {
                    Some(b',') => {
                        pos += 1;
                        *state = ArrState::ExpectValue;
                        continue;
                    }
                    Some(b']') => {
                        pos += 1;
                        stack.pop();
                        continue;
                    }
                    _ => {
                        return Ok(IterJsonOutcome::Invalid {
                            fail_offset: pos.min(bytes.len()),
                        });
                    }
                },
            },
        };

        if !expect_val {
            continue;
        }

        let val_depth = base_depth + stack.len();
        max_depth_seen = max_depth_seen.max(val_depth);

        let mark_parent_after_val = |frame: &mut StackFrame| match frame {
            StackFrame::Object { state, .. } => *state = ObjState::ExpectCommaOrEnd,
            StackFrame::Array { state } => *state = ArrState::ExpectCommaOrEnd,
        };

        match bytes.get(pos) {
            Some(b'{') => {
                check_pending_kv(stack.last_mut(), false, None, val_depth)?;
                mark_parent_after_val(stack.last_mut().unwrap());
                pos += 1;
                stack.push(StackFrame::Object {
                    key: None,
                    state: ObjState::ExpectKeyOrEnd,
                });
            }
            Some(b'[') => {
                check_pending_kv(stack.last_mut(), false, None, val_depth)?;
                mark_parent_after_val(stack.last_mut().unwrap());
                pos += 1;
                stack.push(StackFrame::Array {
                    state: ArrState::ExpectValueOrEnd,
                });
            }
            Some(b'"') => match parse_rfc8259_string(slice, pos) {
                Ok((val_str, next_pos)) => {
                    check_pending_kv(stack.last_mut(), false, Some(&val_str), val_depth)?;
                    mark_parent_after_val(stack.last_mut().unwrap());
                    pos = next_pos;
                }
                Err(err_pos) => {
                    return Ok(IterJsonOutcome::Invalid {
                        fail_offset: err_pos,
                    });
                }
            },
            Some(b't') if bytes[pos..].starts_with(b"true") => {
                check_pending_kv(stack.last_mut(), false, None, val_depth)?;
                mark_parent_after_val(stack.last_mut().unwrap());
                pos += 4;
            }
            Some(b'f') if bytes[pos..].starts_with(b"false") => {
                check_pending_kv(stack.last_mut(), false, None, val_depth)?;
                mark_parent_after_val(stack.last_mut().unwrap());
                pos += 5;
            }
            Some(b'n') if bytes[pos..].starts_with(b"null") => {
                check_pending_kv(stack.last_mut(), true, None, val_depth)?;
                mark_parent_after_val(stack.last_mut().unwrap());
                pos += 4;
            }
            Some(b'-' | b'0'..=b'9') => match parse_rfc8259_number(bytes, pos) {
                Ok(next_pos) => {
                    check_pending_kv(stack.last_mut(), false, None, val_depth)?;
                    mark_parent_after_val(stack.last_mut().unwrap());
                    pos = next_pos;
                }
                Err(err_pos) => {
                    return Ok(IterJsonOutcome::Invalid {
                        fail_offset: err_pos,
                    });
                }
            },
            _ => {
                return Ok(IterJsonOutcome::Invalid {
                    fail_offset: pos.min(bytes.len()),
                });
            }
        }
    }

    if max_depth_seen > MAX_JSON_DEPTH {
        return Err(BotBrowserError::SecretLeakDetected(format!(
            "maximum JSON nesting depth ({MAX_JSON_DEPTH}) exceeded in {context}"
        )));
    }
    Ok(IterJsonOutcome::Valid { consumed: pos })
}

enum ParsedJsonContainer {
    Parsed(serde_json::Value),
    RecursionLimitExceeded,
    NotJson,
}

fn try_parse_json_container_classified(text: &str) -> ParsedJsonContainer {
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return ParsedJsonContainer::NotJson;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(val @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
            ParsedJsonContainer::Parsed(val)
        }
        Ok(_) => ParsedJsonContainer::NotJson,
        Err(err) => {
            if err.to_string().contains("recursion limit exceeded") {
                ParsedJsonContainer::RecursionLimitExceeded
            } else {
                ParsedJsonContainer::NotJson
            }
        }
    }
}

fn json_slice_has_duplicate_keys(slice: &str) -> bool {
    struct DupSeed(usize);
    impl<'de> serde::de::DeserializeSeed<'de> for DupSeed {
        type Value = bool;

        fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<bool, D::Error> {
            if self.0 > MAX_JSON_DEPTH {
                return Ok(true);
            }
            struct Vis(usize);
            impl<'de> serde::de::Visitor<'de> for Vis {
                type Value = bool;

                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("any JSON value")
                }

                fn visit_bool<E>(self, _: bool) -> Result<bool, E> {
                    Ok(false)
                }

                fn visit_i64<E>(self, _: i64) -> Result<bool, E> {
                    Ok(false)
                }

                fn visit_u64<E>(self, _: u64) -> Result<bool, E> {
                    Ok(false)
                }

                fn visit_f64<E>(self, _: f64) -> Result<bool, E> {
                    Ok(false)
                }

                fn visit_str<E>(self, _: &str) -> Result<bool, E> {
                    Ok(false)
                }

                fn visit_unit<E>(self) -> Result<bool, E> {
                    Ok(false)
                }

                fn visit_none<E>(self) -> Result<bool, E> {
                    Ok(false)
                }

                fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<bool, A::Error> {
                    let mut dup = false;
                    while let Some(d) = seq.next_element_seed(DupSeed(self.0 + 1))? {
                        dup |= d;
                    }
                    Ok(dup)
                }

                fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<bool, A::Error> {
                    let mut keys = BTreeSet::new();
                    let mut dup = false;
                    while let Some(k) = map.next_key::<String>()? {
                        if !keys.insert(k) {
                            dup = true;
                        }
                        dup |= map.next_value_seed(DupSeed(self.0 + 1))?;
                    }
                    Ok(dup)
                }
            }
            deserializer.deserialize_any(Vis(self.0))
        }
    }
    let mut de = serde_json::Deserializer::from_str(slice);
    serde::de::DeserializeSeed::deserialize(DupSeed(0), &mut de).unwrap_or(true)
}

fn scan_json_object_brace_map(
    variant: &str,
) -> (
    std::collections::HashMap<usize, (usize, usize)>,
    std::collections::HashSet<usize>,
) {
    let mut matched = std::collections::HashMap::new();
    let mut unclosed = std::collections::HashSet::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut in_str = false;
    let mut escaped = false;

    let bytes = variant.as_bytes();
    let mut iter = variant.char_indices().peekable();
    while let Some((idx, ch)) = iter.next() {
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            } else if ch == '\n' || ch == '\r' {
                in_str = false;
                for &(open_idx, _) in &stack {
                    unclosed.insert(open_idx);
                }
                stack.clear();
            }
            continue;
        }
        if stack.is_empty() {
            if ch == '{' {
                if bytes.get(idx + 1) == Some(&b'{') {
                    iter.next();
                    continue;
                }
                let after = variant[idx + 1..].trim_start();
                if after.starts_with(['"', '}']) {
                    stack.push((idx, 1));
                }
            }
            continue;
        }
        match ch {
            '"' => {
                in_str = true;
                escaped = false;
            }
            '{' | '[' => {
                let abs_depth = stack.len() + 1;
                if ch == '{' {
                    stack.push((idx, abs_depth));
                } else {
                    stack.push((usize::MAX, abs_depth));
                }
            }
            ']' => {
                if let Some(&(open_idx, max_sub)) = stack.last() {
                    if open_idx == usize::MAX {
                        stack.pop();
                        if let Some(parent) = stack.last_mut() {
                            parent.1 = parent.1.max(max_sub);
                        }
                    }
                }
            }
            '}' => {
                while let Some(&(open_idx, max_sub)) = stack.last() {
                    if open_idx == usize::MAX {
                        stack.pop();
                        if let Some(parent) = stack.last_mut() {
                            parent.1 = parent.1.max(max_sub);
                        }
                    } else {
                        break;
                    }
                }
                if let Some((open_idx, max_sub)) = stack.pop() {
                    let rel_depth = max_sub.saturating_sub(stack.len());
                    if let Some(parent) = stack.last_mut() {
                        parent.1 = parent.1.max(max_sub);
                    }
                    matched.insert(open_idx, (idx, rel_depth));
                }
            }
            _ => {}
        }
    }
    for (open_idx, _) in stack {
        if open_idx != usize::MAX {
            unclosed.insert(open_idx);
        }
    }
    (matched, unclosed)
}

fn line_col_to_byte_offset(s: &str, line: usize, col: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut cur_line = 1usize;
    let mut line_start = 0usize;
    if cur_line < line {
        for (i, b) in s.bytes().enumerate() {
            if b == b'\n' {
                cur_line += 1;
                if cur_line == line {
                    line_start = i + 1;
                    break;
                }
            }
        }
        if cur_line < line {
            return s.len();
        }
    }
    let target_chars = col.saturating_sub(1);
    let mut line_end = s.len();
    for (count, (rel_i, ch)) in s[line_start..].char_indices().enumerate() {
        if count == target_chars {
            return line_start + rel_i;
        }
        if ch == '\n' {
            line_end = line_start + rel_i;
            break;
        }
    }
    line_end
}

fn mark_unclosed_braces_before_fail(
    variant: &str,
    start: usize,
    fail_abs: usize,
    fail_skipped_opens: &mut std::collections::HashSet<usize>,
) {
    let end = fail_abs.min(variant.len());
    if start >= end {
        return;
    }
    let mut stack: Vec<usize> = Vec::new();
    let mut in_str = false;
    let mut escaped = false;
    for (rel_idx, ch) in variant[start..end].char_indices() {
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            } else if ch == '\n' || ch == '\r' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_str = true;
                escaped = false;
            }
            '{' => stack.push(start + rel_idx),
            '}' => {
                stack.pop();
            }
            _ => {}
        }
    }
    for open_pos in stack {
        fail_skipped_opens.insert(open_pos);
    }
}

fn scan_embedded_json_objects(
    variant: &str,
    context: &str,
    params: &[ScriptParameter],
    depth: usize,
) -> Result<Vec<(usize, usize)>, BotBrowserError> {
    let mut verified_spans = Vec::new();
    let Some(last_colon) = variant.rfind(':') else {
        return Ok(verified_spans);
    };
    let Some(last_close) = variant.rfind('}') else {
        return Ok(verified_spans);
    };
    if !variant.contains('{') {
        return Ok(verified_spans);
    }
    let (matched_braces, unclosed_braces) = scan_json_object_brace_map(variant);
    let mut skip_until = 0usize;
    let mut fail_points: BTreeSet<usize> = BTreeSet::new();
    let mut fail_skipped_opens: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut failed_parse_bytes = 0usize;
    let max_failed_parse_bytes = variant.len().saturating_mul(4).max(262_144);
    for (idx, ch) in variant.char_indices() {
        if idx >= last_colon || idx >= last_close {
            break;
        }
        if idx < skip_until || ch != '{' {
            continue;
        }
        if unclosed_braces.contains(&idx) {
            continue;
        }
        let slice = &variant[idx..];
        if slice.starts_with("{{") {
            continue;
        }
        let matched_close_opt = matched_braces.get(&idx).copied();
        let candidate = if let Some((close_idx, _max_depth)) = matched_close_opt {
            if fail_points.range((idx + 1)..=close_idx).next().is_some() {
                continue;
            }
            &variant[idx..=close_idx]
        } else {
            if !variant[idx + 1..].trim_start().starts_with(['"', '}']) {
                continue;
            }
            if fail_skipped_opens.contains(&idx) || failed_parse_bytes >= max_failed_parse_bytes {
                continue;
            }
            slice
        };
        let mut stream = serde_json::Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
        match stream.next() {
            Some(Ok(val)) => {
                if matches!(val, serde_json::Value::Object(_)) {
                    scan_json_value_with_depth(
                        &val,
                        &format!("{context}.<embedded_json>"),
                        params,
                        depth + 1,
                    )?;
                    let consumed = stream.byte_offset();
                    if consumed > 0 {
                        let json_slice = &candidate[..consumed];
                        if json_slice_has_duplicate_keys(json_slice) {
                            scan_json_slice_with_duplicates(
                                json_slice,
                                &format!("{context}.<embedded_json>"),
                                params,
                                depth + 1,
                            )?;
                        }
                        verified_spans.push((idx, idx + consumed));
                        skip_until = idx + consumed;
                    }
                }
            }
            Some(Err(err)) => {
                let mut handled_iter_valid = false;
                let err_rel = if err.to_string().contains("recursion limit exceeded") {
                    match scan_json_slice_iteratively(
                        candidate,
                        &format!("{context}.<embedded_json>"),
                        params,
                        depth + 1,
                    )? {
                        IterJsonOutcome::Valid { consumed } => {
                            if consumed > 0 {
                                verified_spans.push((idx, idx + consumed));
                                skip_until = idx + consumed;
                            }
                            handled_iter_valid = true;
                            0
                        }
                        IterJsonOutcome::Invalid { fail_offset } => fail_offset,
                    }
                } else {
                    line_col_to_byte_offset(candidate, err.line(), err.column())
                };
                if handled_iter_valid {
                    continue;
                }
                failed_parse_bytes = failed_parse_bytes.saturating_add(err_rel.max(1));
                if let Some((close_idx, _)) = matched_close_opt {
                    let fail_abs = idx.saturating_add(err_rel.max(1)).min(close_idx);
                    fail_points.insert(fail_abs);
                } else {
                    let fail_abs = idx.saturating_add(err_rel.max(1)).min(variant.len());
                    mark_unclosed_braces_before_fail(
                        variant,
                        idx,
                        fail_abs,
                        &mut fail_skipped_opens,
                    );
                }
            }
            None => {}
        }
    }
    Ok(verified_spans)
}

pub fn scan_text_for_secret_leaks(
    text: &str,
    context: &str,
    params: &[ScriptParameter],
) -> Result<(), BotBrowserError> {
    scan_text_for_secret_leaks_with_depth(text, context, params, 0)
}

fn scan_text_for_secret_leaks_with_depth(
    text: &str,
    context: &str,
    params: &[ScriptParameter],
    depth: usize,
) -> Result<(), BotBrowserError> {
    if depth > MAX_JSON_DEPTH {
        return Err(BotBrowserError::SecretLeakDetected(format!(
            "maximum JSON nesting depth ({MAX_JSON_DEPTH}) exceeded in {context}"
        )));
    }
    if text.trim().is_empty() {
        return Ok(());
    }

    // Step A: Direct JSON container leaf — recurse structurally (including all duplicate keys).
    let trimmed_text = text.trim();
    match try_parse_json_container_classified(trimmed_text) {
        ParsedJsonContainer::Parsed(val) => {
            scan_json_value_with_depth(
                &val,
                &format!("{context}.<json>"),
                params,
                depth + 1,
            )?;
            if matches!(
                val,
                serde_json::Value::Object(_) | serde_json::Value::Array(_)
            ) && json_slice_has_duplicate_keys(trimmed_text)
            {
                scan_json_slice_with_duplicates(
                    trimmed_text,
                    &format!("{context}.<json>"),
                    params,
                    depth + 1,
                )?;
            }
            return Ok(());
        }
        ParsedJsonContainer::RecursionLimitExceeded => {
            if let IterJsonOutcome::Valid { consumed } = scan_json_slice_iteratively(
                trimmed_text,
                &format!("{context}.<json>"),
                params,
                depth + 1,
            )? {
                if trimmed_text[consumed..].trim().is_empty() {
                    return Ok(());
                }
            }
        }
        ParsedJsonContainer::NotJson => {}
    }

    const MAX_DECODE_PASSES: usize = 4;
    const MAX_VARIANTS: usize = 64;

    // Step B: Purely JSON-escaped JSON container (e.g. `{\"auth_token\": \"secret\"}`).
    if text.contains('\\') {
        let mut unescaped = text.to_string();
        for _ in 0..MAX_DECODE_PASSES {
            let next = unescape_json_for_scan(&unescaped);
            if next == unescaped {
                break;
            }
            let next_trimmed = next.trim();
            match try_parse_json_container_classified(next_trimmed) {
                ParsedJsonContainer::Parsed(val) => {
                    scan_json_value_with_depth(
                        &val,
                        &format!("{context}.<json>"),
                        params,
                        depth + 1,
                    )?;
                    if matches!(
                        val,
                        serde_json::Value::Object(_) | serde_json::Value::Array(_)
                    ) && json_slice_has_duplicate_keys(next_trimmed)
                    {
                        scan_json_slice_with_duplicates(
                            next_trimmed,
                            &format!("{context}.<json>"),
                            params,
                            depth + 1,
                        )?;
                    }
                    return Ok(());
                }
                ParsedJsonContainer::RecursionLimitExceeded => {
                    if let IterJsonOutcome::Valid { consumed } = scan_json_slice_iteratively(
                        next_trimmed,
                        &format!("{context}.<json>"),
                        params,
                        depth + 1,
                    )? {
                        if next_trimmed[consumed..].trim().is_empty() {
                            return Ok(());
                        }
                    }
                }
                ParsedJsonContainer::NotJson => {}
            }
            unescaped = next;
        }
    }

    // Step C: Multi-pass decoding variants (Variant 0 always scanned first).
    if !text.contains(['%', '+', '\\']) {
        let verified_spans = scan_embedded_json_objects(text, context, params, depth)?;
        scan_variant_clean(text, context, params, &verified_spans)?;
        return Ok(());
    }

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut variants: Vec<String> = Vec::with_capacity(16);
    let push_unique = |s: String,
                       seen: &mut BTreeSet<String>,
                       variants: &mut Vec<String>,
                       frontier_out: Option<&mut Vec<String>>| {
        if variants.len() < MAX_VARIANTS && seen.insert(s.clone()) {
            if let Some(f) = frontier_out {
                f.push(s.clone());
            }
            variants.push(s);
        }
    };

    let mut frontier: Vec<String> = Vec::new();
    push_unique(
        text.to_string(),
        &mut seen,
        &mut variants,
        Some(&mut frontier),
    );

    for _ in 0..MAX_DECODE_PASSES {
        if frontier.is_empty() || variants.len() >= MAX_VARIANTS {
            break;
        }
        let mut next_frontier: Vec<String> = Vec::new();
        for cur in &frontier {
            if variants.len() >= MAX_VARIANTS {
                break;
            }
            if cur.contains('%') {
                let pct = percent_decode_for_scan(cur);
                if pct.contains('+') {
                    push_unique(pct.replace('+', " "), &mut seen, &mut variants, None);
                }
                push_unique(pct, &mut seen, &mut variants, Some(&mut next_frontier));
            } else if cur.contains('+') {
                push_unique(cur.replace('+', " "), &mut seen, &mut variants, None);
            }

            if cur.contains('\\') {
                let json_un = unescape_json_for_scan(cur);
                push_unique(json_un, &mut seen, &mut variants, Some(&mut next_frontier));
            }
        }
        frontier = next_frontier;
    }

    for (var_idx, variant) in variants.iter().enumerate() {
        if var_idx == 0 {
            // Variant 0 is identical to `text`, which Step A already verified is not a valid full JSON container.
            let verified_spans = scan_embedded_json_objects(variant, context, params, depth)?;
            scan_variant_clean(variant, context, params, &verified_spans)?;
            continue;
        }
        let var_trimmed = variant.trim();
        match try_parse_json_container_classified(var_trimmed) {
            ParsedJsonContainer::Parsed(val) => {
                scan_json_value_with_depth(&val, &format!("{context}.<json>"), params, depth + 1)?;
                if matches!(
                    val,
                    serde_json::Value::Object(_) | serde_json::Value::Array(_)
                ) && json_slice_has_duplicate_keys(var_trimmed)
                {
                    scan_json_slice_with_duplicates(
                        var_trimmed,
                        &format!("{context}.<json>"),
                        params,
                        depth + 1,
                    )?;
                }
            }
            ParsedJsonContainer::RecursionLimitExceeded => {
                let mut valid_full_json = false;
                if let IterJsonOutcome::Valid { consumed } = scan_json_slice_iteratively(
                    var_trimmed,
                    &format!("{context}.<json>"),
                    params,
                    depth + 1,
                )? {
                    if var_trimmed[consumed..].trim().is_empty() {
                        valid_full_json = true;
                    }
                }
                if !valid_full_json {
                    let verified_spans = scan_embedded_json_objects(variant, context, params, depth)?;
                    scan_variant_clean(variant, context, params, &verified_spans)?;
                }
            }
            ParsedJsonContainer::NotJson => {
                let verified_spans = scan_embedded_json_objects(variant, context, params, depth)?;
                scan_variant_clean(variant, context, params, &verified_spans)?;
            }
        }
    }

    Ok(())
}

fn matching_open_bracket(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

fn is_clean_outer_kv_boundary(
    after: &str,
    open_bracket: Option<char>,
    in_url_query: bool,
) -> bool {
    if after.is_empty() {
        return true;
    }
    if after.starts_with(char::is_whitespace)
        || (in_url_query && after.starts_with(['&', ';', '#']))
    {
        return true;
    }
    if after.starts_with(',') {
        let rest = &after[1..];
        return rest.is_empty() || rest.starts_with(char::is_whitespace);
    }
    if let Some(first) = after.chars().next() {
        if let Some(expected_open) = matching_open_bracket(first) {
            if open_bracket == Some(expected_open) {
                let rest = &after[first.len_utf8()..];
                return rest.is_empty()
                    || rest.starts_with(char::is_whitespace)
                    || (in_url_query && rest.starts_with(['&', ';', '#']))
                    || (rest.starts_with(',')
                        && (rest[1..].is_empty() || rest[1..].starts_with(char::is_whitespace)));
            }
        }
    }
    false
}

fn extract_kv_value_token_with_consumed(
    after_sep: &str,
    open_bracket: Option<char>,
    in_url_query: bool,
) -> (&str, usize) {
    let s = after_sep.trim_start();
    let leading_ws = after_sep.len() - s.len();
    if let Some(first @ ('"' | '\'')) = s.chars().next() {
        let q_inner = &s[1..];
        if let Some(end) = q_inner.find([first, '\n', '\r']) {
            if q_inner.as_bytes()[end] == first as u8 {
                let after_close = &q_inner[end + 1..];
                if is_clean_outer_kv_boundary(after_close, open_bracket, in_url_query) {
                    return (q_inner[..end].trim(), leading_ws + 1 + end + 1);
                }
            }
        }
    } else if s.starts_with("{{") {
        if let Some(rel_end) = s[2..].find("}}") {
            let ph_end = 4 + rel_end;
            let after_ph = &s[ph_end..];
            if is_clean_outer_kv_boundary(after_ph, open_bracket, in_url_query) {
                return (s[..ph_end].trim(), leading_ws + ph_end);
            }
        }
    }

    let mut end = s.len();
    for (idx, _) in s.char_indices() {
        if is_clean_outer_kv_boundary(&s[idx..], open_bracket, in_url_query) {
            end = idx;
            break;
        }
    }
    (s[..end].trim(), leading_ws + end)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthPayloadKind {
    Unquoted,
    Quoted,
    Bracketed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnclosureOverflow {
    open: char,
}

struct AuthPayload<'a> {
    unwrapped: &'a str,
    kind: AuthPayloadKind,
    has_broken_quote: bool,
    followed_by_kv_sep: Option<char>,
    rest_after_kv_sep: &'a str,
    consumed_len: usize,
    enclosure_overflow: Option<EnclosureOverflow>,
    overflow_already_verified: bool,
}

fn is_angle_comparison_op(s: &str, gt_idx: usize) -> bool {
    let bytes = s.as_bytes();
    if gt_idx == 0 || gt_idx + 1 >= bytes.len() {
        return false;
    }
    let b_prev = bytes[gt_idx - 1];
    if b_prev < 0x80 && !matches!(b_prev, b' ' | b'\t') {
        return false;
    }
    let b_next = bytes[gt_idx + 1];
    if b_next < 0x80 && !matches!(b_next, b' ' | b'\t') {
        return false;
    }
    let before = &s[..gt_idx];
    if !before
        .chars()
        .next_back()
        .is_some_and(|c| c.is_whitespace() && c != '\n' && c != '\r')
    {
        return false;
    }
    if before.trim_end().ends_with("}}") {
        return false;
    }
    let after = &s[gt_idx + 1..];
    if !after
        .chars()
        .next()
        .is_some_and(|c| c.is_whitespace() && c != '\n' && c != '\r')
    {
        return false;
    }
    let mut after_ws = after;
    for (i, c) in after.char_indices() {
        if c == '\n' || c == '\r' || !c.is_whitespace() {
            after_ws = &after[i..];
            break;
        }
        after_ws = "";
    }
    after_ws.chars().next().is_some_and(|c| {
        !matches!(c, '\n' | '\r' | '>' | '.' | ',' | ';' | '!' | '?')
    })
}

fn unwrap_balanced_enclosure(s: &str) -> Option<&str> {
    let open = if s.starts_with('(') && s.ends_with(')') && s.len() >= 2 {
        '('
    } else if s.starts_with('[') && s.ends_with(']') && s.len() >= 2 {
        '['
    } else if s.starts_with('{') && !s.starts_with("{{") && s.ends_with('}') && s.len() >= 2 {
        '{'
    } else if s.starts_with('<') && s.ends_with('>') && s.len() >= 2 {
        '<'
    } else {
        return None;
    };
    let mut stack: Vec<char> = Vec::new();
    for (idx, ch) in s.char_indices() {
        if ch == '(' || ch == '[' || ch == '{' || (open == '<' && ch == '<') {
            stack.push(ch);
        } else if let Some(expected) = match ch {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            '>' if open == '<' && !is_angle_comparison_op(s, idx) => Some('<'),
            _ => None,
        } {
            if stack.last().copied() != Some(expected) {
                return None;
            }
            stack.pop();
            if stack.is_empty() && idx + ch.len_utf8() < s.len() {
                return None;
            }
        }
    }
    if stack.is_empty() {
        Some(s[1..s.len() - 1].trim())
    } else {
        None
    }
}

fn is_clean_auth_enclosure_boundary(after_close: &str) -> bool {
    after_close.is_empty()
        || after_close.starts_with(char::is_whitespace)
        || (after_close.starts_with(|c: char| matches!(c, '.' | '!' | '?' | ',' | ';'))
            && (after_close[1..].is_empty() || after_close[1..].starts_with(char::is_whitespace)))
}

fn extract_auth_payload(after_sep: &str) -> AuthPayload<'_> {
    extract_auth_payload_with_memo(after_sep, None, None, false)
}

fn extract_auth_payload_with_memo<'a>(
    after_sep: &'a str,
    mut memo: Option<&mut EnclosureOverflowMemo>,
    full_text: Option<&str>,
    allow_small_verified: bool,
) -> AuthPayload<'a> {
    let s = after_sep.trim_start();
    let leading_ws = after_sep.len() - s.len();
    if let Some(q @ ('"' | '\'')) = s.chars().next() {
        let q_inner = &s[1..];
        if let Some(end) = q_inner.find(q) {
            let inner = q_inner[..end].trim();
            let after_close = &q_inner[end + 1..];
            if is_clean_auth_enclosure_boundary(after_close) {
                return AuthPayload {
                    unwrapped: inner,
                    kind: AuthPayloadKind::Quoted,
                    has_broken_quote: false,
                    followed_by_kv_sep: None,
                    rest_after_kv_sep: "",
                    consumed_len: leading_ws + 1 + end + 1,
                    enclosure_overflow: None,
                    overflow_already_verified: false,
                };
            }
            let trimmed_after = after_close.trim_start();
            if let Some(sep @ (':' | '=')) =
                trimmed_after.chars().next().filter(|c| matches!(c, ':' | '='))
            {
                if !inner.contains("{{") {
                    return AuthPayload {
                        unwrapped: inner,
                        kind: AuthPayloadKind::Quoted,
                        has_broken_quote: false,
                        followed_by_kv_sep: Some(sep),
                        rest_after_kv_sep: &trimmed_after[1..],
                        consumed_len: leading_ws + 1 + end + 1,
                        enclosure_overflow: None,
                        overflow_already_verified: false,
                    };
                }
            }
            let after_ws_end = after_close.find(char::is_whitespace).unwrap_or(after_close.len());
            let span_end = 1 + end + 1 + after_ws_end;
            let raw_tok = &s[..span_end];
            if !raw_tok.contains("{{")
                && !raw_tok.contains("}}")
                && (raw_tok.ends_with(':') || raw_tok.ends_with('='))
            {
                let sep = raw_tok.as_bytes()[raw_tok.len() - 1] as char;
                return AuthPayload {
                    unwrapped: &raw_tok[..raw_tok.len() - 1],
                    kind: AuthPayloadKind::Unquoted,
                    has_broken_quote: true,
                    followed_by_kv_sep: Some(sep),
                    rest_after_kv_sep: &s[span_end..],
                    consumed_len: leading_ws + span_end - 1,
                    enclosure_overflow: None,
                    overflow_already_verified: false,
                };
            }
            return AuthPayload {
                unwrapped: raw_tok,
                kind: AuthPayloadKind::Unquoted,
                has_broken_quote: true,
                followed_by_kv_sep: None,
                rest_after_kv_sep: "",
                consumed_len: leading_ws + span_end,
                enclosure_overflow: None,
                overflow_already_verified: false,
            };
        }
        return AuthPayload {
            unwrapped: s,
            kind: AuthPayloadKind::Unquoted,
            has_broken_quote: true,
            followed_by_kv_sep: None,
            rest_after_kv_sep: "",
            consumed_len: leading_ws + s.len(),
            enclosure_overflow: None,
            overflow_already_verified: false,
        };
    }

    if let Some(open @ ('(' | '[' | '{' | '<')) = s.chars().next() {
        if !(open == '{' && s.starts_with("{{")) {
            const MAX_ENCLOSURE_SCAN_BYTES: usize = 1024;

            if let (Some(m), Some(ft)) = (memo.as_deref_mut(), full_text) {
                let open_abs_pos = s.as_ptr() as usize - ft.as_ptr() as usize;
                let body_abs_start = open_abs_pos + 1;
                if m.fast_overflow_is_safe(
                    ft,
                    open,
                    open_abs_pos,
                    body_abs_start,
                    MAX_ENCLOSURE_SCAN_BYTES,
                    allow_small_verified,
                ) {
                    return AuthPayload {
                        unwrapped: "",
                        kind: AuthPayloadKind::Unquoted,
                        has_broken_quote: false,
                        followed_by_kv_sep: None,
                        rest_after_kv_sep: "",
                        consumed_len: leading_ws + s.len(),
                        enclosure_overflow: Some(EnclosureOverflow { open }),
                        overflow_already_verified: true,
                    };
                }
            }

            let content_trimmed = s[1..].trim_start();
            let content_start = s.len() - content_trimmed.len();

            let mut stack: Vec<char> = vec![open];
            let mut matched_end: Option<usize> = None;
            let mut mismatched = false;
            let mut in_quote: Option<char> = None;
            let mut escaped = false;
            let mut saw_quote = false;
            let mut non_ws_bytes = 0usize;
            let mut capped_end = content_start;
            let mut hit_cap = false;

            if s.is_ascii() {
                let bytes = s.as_bytes();
                let mut abs_idx = content_start;
                while abs_idx < bytes.len() {
                    let b = bytes[abs_idx];
                    if b.is_ascii_whitespace() {
                        capped_end = abs_idx + 1;
                        abs_idx += 1;
                        continue;
                    }
                    if non_ws_bytes >= MAX_ENCLOSURE_SCAN_BYTES {
                        hit_cap = true;
                        break;
                    }
                    let next_end = abs_idx + 1;
                    non_ws_bytes += 1;
                    capped_end = next_end;

                    if let Some(q) = in_quote {
                        if escaped {
                            escaped = false;
                        } else if b == b'\\' {
                            escaped = true;
                        } else if b == q as u8 {
                            in_quote = None;
                        }
                        abs_idx += 1;
                        continue;
                    }
                    if b == b'"' || b == b'\'' {
                        in_quote = Some(b as char);
                        saw_quote = true;
                        abs_idx += 1;
                        continue;
                    }

                    if !mismatched {
                        if b == b'(' || b == b'[' || b == b'{' || (open == '<' && b == b'<') {
                            stack.push(b as char);
                        } else if let Some(expected_open) = match b {
                            b')' => Some('('),
                            b']' => Some('['),
                            b'}' => Some('{'),
                            b'>' if open == '<' && !is_angle_comparison_op(s, abs_idx) => {
                                Some('<')
                            }
                            _ => None,
                        } {
                            if stack.last().copied() == Some(expected_open) {
                                stack.pop();
                                if stack.is_empty() {
                                    matched_end = Some(abs_idx);
                                    break;
                                }
                            } else {
                                mismatched = true;
                            }
                        }
                    }
                    abs_idx += 1;
                }
            } else {
                for (rel_idx, ch) in s[content_start..].char_indices() {
                    if ch.is_whitespace() {
                        capped_end = content_start + rel_idx + ch.len_utf8();
                        continue;
                    }
                    if non_ws_bytes >= MAX_ENCLOSURE_SCAN_BYTES {
                        hit_cap = true;
                        break;
                    }
                    let abs_idx = content_start + rel_idx;
                    let next_end = abs_idx + ch.len_utf8();
                    non_ws_bytes += ch.len_utf8();
                    capped_end = next_end;

                    if let Some(q) = in_quote {
                        if escaped {
                            escaped = false;
                        } else if ch == '\\' {
                            escaped = true;
                        } else if ch == q {
                            in_quote = None;
                        }
                        continue;
                    }
                    if ch == '"' || ch == '\'' {
                        in_quote = Some(ch);
                        saw_quote = true;
                        continue;
                    }

                    if !mismatched {
                        if ch == '(' || ch == '[' || ch == '{' || (open == '<' && ch == '<') {
                            stack.push(ch);
                        } else if let Some(expected_open) = match ch {
                            ')' => Some('('),
                            ']' => Some('['),
                            '}' => Some('{'),
                            '>' if open == '<' && !is_angle_comparison_op(s, abs_idx) => {
                                Some('<')
                            }
                            _ => None,
                        } {
                            if stack.last().copied() == Some(expected_open) {
                                stack.pop();
                                if stack.is_empty() {
                                    matched_end = Some(abs_idx);
                                    break;
                                }
                            } else {
                                mismatched = true;
                            }
                        }
                    }
                }
            }
            if let Some(end_idx) = matched_end {
                let inner = s[1..end_idx].trim();
                let after_close = &s[end_idx + 1..];
                let (effective_unwrapped, inner_broken_quote) =
                    if let Some(q_inner) = unwrap_clean_quote_pair(inner) {
                        (q_inner, false)
                    } else {
                        (inner, inner.contains(['"', '\'']))
                    };
                if is_clean_auth_enclosure_boundary(after_close) {
                    return AuthPayload {
                        unwrapped: effective_unwrapped,
                        kind: AuthPayloadKind::Bracketed,
                        has_broken_quote: inner_broken_quote,
                        followed_by_kv_sep: None,
                        rest_after_kv_sep: "",
                        consumed_len: leading_ws + end_idx + 1,
                        enclosure_overflow: None,
                        overflow_already_verified: false,
                    };
                }
                let trimmed_after = after_close.trim_start();
                if let Some(sep @ (':' | '=')) =
                    trimmed_after.chars().next().filter(|c| matches!(c, ':' | '='))
                {
                    if !effective_unwrapped.contains("{{") {
                        return AuthPayload {
                            unwrapped: effective_unwrapped,
                            kind: AuthPayloadKind::Bracketed,
                            has_broken_quote: inner_broken_quote,
                            followed_by_kv_sep: Some(sep),
                            rest_after_kv_sep: &trimmed_after[1..],
                            consumed_len: leading_ws + end_idx + 1,
                            enclosure_overflow: None,
                            overflow_already_verified: false,
                        };
                    }
                }
                let after_ws_end = after_close.find(char::is_whitespace).unwrap_or(after_close.len());
                let span_end = end_idx + 1 + after_ws_end;
                let raw_tok = &s[..span_end];
                if !raw_tok.contains("{{")
                    && !raw_tok.contains("}}")
                    && (raw_tok.ends_with(':') || raw_tok.ends_with('='))
                {
                    let sep = raw_tok.as_bytes()[raw_tok.len() - 1] as char;
                    return AuthPayload {
                        unwrapped: &raw_tok[..raw_tok.len() - 1],
                        kind: AuthPayloadKind::Unquoted,
                        has_broken_quote: raw_tok[..raw_tok.len() - 1].contains(['"', '\'']),
                        followed_by_kv_sep: Some(sep),
                        rest_after_kv_sep: &s[span_end..],
                        consumed_len: leading_ws + span_end - 1,
                        enclosure_overflow: None,
                        overflow_already_verified: false,
                    };
                }
                return AuthPayload {
                    unwrapped: raw_tok,
                    kind: AuthPayloadKind::Unquoted,
                    has_broken_quote: raw_tok.contains(['"', '\'']),
                    followed_by_kv_sep: None,
                    rest_after_kv_sep: "",
                    consumed_len: leading_ws + span_end,
                    enclosure_overflow: None,
                    overflow_already_verified: false,
                };
            }

            // Unclosed or mismatched enclosure (`(`, `[`, `{`, `<`): capture up to 1024
            // non-whitespace bytes across lines on UTF-8 char boundaries, and flag any
            // overflow past the cap so callers scan the remaining enclosure body safely.
            return AuthPayload {
                unwrapped: &s[..capped_end],
                kind: AuthPayloadKind::Unquoted,
                has_broken_quote: saw_quote || !hit_cap,
                followed_by_kv_sep: None,
                rest_after_kv_sep: "",
                consumed_len: leading_ws + capped_end,
                enclosure_overflow: hit_cap.then_some(EnclosureOverflow { open }),
                overflow_already_verified: false,
            };
        }
    }

    let ws_end = s.find(char::is_whitespace).unwrap_or(s.len());
    let ws_span = &s[..ws_end];
    let after_ws = s[ws_end..].trim_start();

    let (lhs_raw, followed_by_kv_sep, rest_after_kv_sep) = if !ws_span.contains("{{")
        && !ws_span.contains(['"', '\''])
        && (ws_span.ends_with(':') || ws_span.ends_with('='))
    {
        let sep = ws_span.as_bytes()[ws_span.len() - 1] as char;
        (&ws_span[..ws_span.len() - 1], Some(sep), &s[ws_end..])
    } else if let Some(sep @ (':' | '=')) =
        after_ws.chars().next().filter(|c| matches!(c, ':' | '='))
    {
        (ws_span, Some(sep), &after_ws[1..])
    } else {
        (ws_span, None, "")
    };

    let cleaned = if !lhs_raw.contains("{{") && !lhs_raw.contains(['"', '\'']) {
        lhs_raw.trim_end_matches(['.', '!', '?', ',', ';'])
    } else {
        lhs_raw
    };

    if let Some(inner) = unwrap_balanced_enclosure(cleaned) {
        let (effective_unwrapped, inner_broken_quote) =
            if let Some(q_inner) = unwrap_clean_quote_pair(inner) {
                (q_inner, false)
            } else {
                (inner, inner.contains(['"', '\'']))
            };
        return AuthPayload {
            unwrapped: effective_unwrapped,
            kind: AuthPayloadKind::Bracketed,
            has_broken_quote: inner_broken_quote,
            followed_by_kv_sep,
            rest_after_kv_sep,
            consumed_len: leading_ws + ws_end,
            enclosure_overflow: None,
            overflow_already_verified: false,
        };
    }

    AuthPayload {
        unwrapped: cleaned,
        kind: AuthPayloadKind::Unquoted,
        has_broken_quote: cleaned.contains(['"', '\'']),
        followed_by_kv_sep,
        rest_after_kv_sep,
        consumed_len: leading_ws + ws_end,
        enclosure_overflow: None,
        overflow_already_verified: false,
    }
}

fn looks_like_basic_credential(cred: &str) -> bool {
    if cred.len() < 8 {
        return false;
    }
    if !cred
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-'))
    {
        return false;
    }
    let check_decoded = |bytes: &[u8]| -> bool {
        if let Ok(text) = std::str::from_utf8(bytes) {
            if !text.chars().any(|c| c.is_control()) {
                if let Some((user, pass)) = text.split_once(':') {
                    if !user.is_empty() && !pass.is_empty() {
                        return true;
                    }
                }
            }
        }
        false
    };
    if cred.len() % 4 != 1 {
        if cred.len() <= 340 {
            let mut stack_buf = [0u8; 256];
            if let Ok(n) = B64_STD.decode_slice(cred, &mut stack_buf) {
                if check_decoded(&stack_buf[..n]) {
                    return true;
                }
            } else if let Ok(n) =
                B64_URL.decode_slice(cred.trim_end_matches('='), &mut stack_buf)
            {
                if check_decoded(&stack_buf[..n]) {
                    return true;
                }
            }
        } else if let Some(bytes) = B64_STD
            .decode(cred)
            .ok()
            .or_else(|| B64_URL.decode(cred.trim_end_matches('=')).ok())
        {
            if check_decoded(&bytes) {
                return true;
            }
        }
    }
    let has_digit = cred.bytes().any(|b| b.is_ascii_digit());
    let has_b64_sym = cred
        .bytes()
        .any(|b| matches!(b, b'+' | b'/' | b'=' | b'_'));
    let has_hyphen_with_upper =
        cred.contains('-') && cred.bytes().any(|b| b.is_ascii_uppercase());
    has_digit || has_b64_sym || has_hyphen_with_upper
}

fn contains_basic_credential_token(s: &str) -> bool {
    if looks_like_basic_credential(s) {
        return true;
    }
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'+' | b'/' | b'=' | b'_' | b'-')
        {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || matches!(bytes[i], b'+' | b'/' | b'=' | b'_' | b'-'))
            {
                i += 1;
            }
            if i - start >= 8 && looks_like_basic_credential(&s[start..i]) {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

fn is_unescaped_quote(bytes: &[u8], idx: usize) -> bool {
    let mut backslashes = 0usize;
    let mut i = idx;
    while i > 0 && bytes[i - 1] == b'\\' {
        backslashes += 1;
        i -= 1;
    }
    backslashes % 2 == 0
}

#[derive(Default)]
struct LineSegIndex {
    line_start: usize,
    line_end: usize,
    single_sensitive: Vec<(usize, usize)>,
    pair_sensitive: Vec<(usize, usize)>,
    bearer_segs: Vec<(usize, usize)>,
    basic_segs: Vec<(usize, usize)>,
}

impl LineSegIndex {
    fn build(text: &str, line_start: usize) -> Self {
        let line_end = line_start
            + text[line_start..]
                .find(['\n', '\r'])
                .unwrap_or(text.len() - line_start);
        let line = &text[line_start..line_end];
        let mut idx_obj = Self {
            line_start,
            line_end,
            ..Default::default()
        };

        const SINGLE_SEGMENTS: &[&str] = &[
            "ct0",
            "password",
            "passwd",
            "passcode",
            "pwd",
            "apikey",
            "authorization",
            "token",
            "secret",
            "cookie",
        ];
        const PAIR_SEGMENTS: &[(&str, &str)] = &[
            ("ct", "0"),
            ("auth", "token"),
            ("api", "key"),
            ("access", "token"),
            ("refresh", "token"),
            ("id", "token"),
            ("session", "id"),
            ("set", "cookie"),
            ("private", "key"),
            ("pass", "word"),
            ("pass", "wd"),
            ("pass", "code"),
        ];

        let mut cur = String::new();
        let mut cur_start = line_start;
        let mut prev_seg = String::new();
        let mut prev_start = line_start;
        let mut has_prev = false;

        let mut flush_seg = |seg: &mut String, seg_start: usize, seg_end: usize| {
            if seg.is_empty() {
                return;
            }
            if SINGLE_SEGMENTS.contains(&seg.as_str()) {
                idx_obj.single_sensitive.push((seg_start, seg_end));
            }
            if seg == "bearer" {
                idx_obj.bearer_segs.push((seg_start, seg_end));
            } else if seg == "basic" {
                idx_obj.basic_segs.push((seg_start, seg_end));
            }
            if has_prev && PAIR_SEGMENTS.contains(&(prev_seg.as_str(), seg.as_str())) {
                idx_obj.pair_sensitive.push((prev_start, seg_end));
            }
            std::mem::swap(&mut prev_seg, seg);
            prev_start = seg_start;
            has_prev = true;
            seg.clear();
        };

        let mut prev_ch: Option<char> = None;
        let mut it = line.char_indices().peekable();
        while let Some((rel_i, ch)) = it.next() {
            let abs_i = line_start + rel_i;
            if !ch.is_ascii_alphanumeric() {
                flush_seg(&mut cur, cur_start, abs_i);
                prev_ch = Some(ch);
                continue;
            }
            if !cur.is_empty() {
                let p = prev_ch.unwrap();
                if ch.is_ascii_uppercase() {
                    let next_is_lower = it.peek().is_some_and(|&(_, n)| n.is_ascii_lowercase());
                    if p.is_ascii_lowercase()
                        || p.is_ascii_digit()
                        || (p.is_ascii_uppercase() && next_is_lower)
                    {
                        flush_seg(&mut cur, cur_start, abs_i);
                        cur_start = abs_i;
                    }
                } else if (ch.is_ascii_digit() && p.is_ascii_alphabetic())
                    || (ch.is_ascii_lowercase() && p.is_ascii_digit())
                {
                    flush_seg(&mut cur, cur_start, abs_i);
                    cur_start = abs_i;
                }
            } else {
                cur_start = abs_i;
            }
            cur.push(ch.to_ascii_lowercase());
            prev_ch = Some(ch);
        }
        flush_seg(&mut cur, cur_start, line_end);
        idx_obj
    }

    fn contains_range(ranges: &[(usize, usize)], key_start: usize, key_end: usize) -> bool {
        let idx = ranges.partition_point(|&(s, _)| s < key_start);
        ranges
            .get(idx)
            .is_some_and(|&(s, e)| s >= key_start && e <= key_end)
    }

    fn query(&self, key_start: usize, key_end: usize) -> (bool, bool, bool) {
        let s = Self::contains_range(&self.single_sensitive, key_start, key_end)
            || Self::contains_range(&self.pair_sensitive, key_start, key_end);
        let br = Self::contains_range(&self.bearer_segs, key_start, key_end);
        let ba = Self::contains_range(&self.basic_segs, key_start, key_end);
        (s, br, ba)
    }
}

#[inline]
fn is_kv_ident_char(ch: char) -> bool {
    !ch.is_control()
        && !ch.is_whitespace()
        && (ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || !ch.is_ascii())
}

#[inline]
fn is_safe_h4_prose_rhs(after_sep_raw: &str) -> bool {
    let line_end = after_sep_raw.find(['\n', '\r']).unwrap_or(after_sep_raw.len());
    let rhs_line = after_sep_raw[..line_end].trim();
    if rhs_line.is_empty() {
        return false;
    }
    let word = rhs_line.strip_suffix('.').unwrap_or(rhs_line);
    !word.is_empty() && word.len() <= 3 && word.bytes().all(|b| b.is_ascii_alphabetic())
}

#[derive(Clone, Copy)]
struct QuoteCheckpoint {
    quote_char: u8,
    sub_or_dots_len: usize,
    tight_sub_or_dots_len: usize,
    chain_breaks_len: usize,
    prose_breaks_len: usize,
    lbracks_len: usize,
    quotes_len: usize,
}

#[derive(Default)]
struct KvLineScanMemo {
    line_start: usize,
    scanned_to: usize,
    quote_stack: Vec<QuoteCheckpoint>,
    dquotes: Vec<usize>,
    squotes: Vec<usize>,
    matched_rbrack_jump: std::collections::HashMap<usize, usize>,
    unmatched_rbrack: std::collections::HashSet<usize>,
    ended_in_quote_rbrack: std::collections::HashSet<usize>,
    matched_rbrack_jump_noquote: std::collections::HashMap<usize, usize>,
    unmatched_rbrack_noquote: std::collections::HashSet<usize>,
    cur_lbrack_run_start: Option<usize>,
    lbrack_runs: Vec<(usize, usize)>,
    sub_root_cache: std::collections::HashMap<u64, (usize, bool)>,
    line_sub_or_dots: Vec<usize>,
    line_tight_sub_or_dots: Vec<usize>,
    line_chain_breaks: Vec<usize>,
    line_prose_breaks: Vec<usize>,
    line_lbracks: Vec<usize>,
    line_quotes: Vec<usize>,
    line_has_url_scheme: bool,
    line_seg_index: Option<LineSegIndex>,
}

impl KvLineScanMemo {
    fn advance_to(&mut self, text: &str, target: usize) {
        let bytes = text.as_bytes();
        let limit = target.min(bytes.len());
        while self.scanned_to < limit {
            let b = bytes[self.scanned_to];
            if b == b'\n' || b == b'\r' {
                self.line_start = self.scanned_to + 1;
                self.quote_stack.clear();
                self.dquotes.clear();
                self.squotes.clear();
                self.matched_rbrack_jump.clear();
                self.unmatched_rbrack.clear();
                self.ended_in_quote_rbrack.clear();
                self.matched_rbrack_jump_noquote.clear();
                self.unmatched_rbrack_noquote.clear();
                self.cur_lbrack_run_start = None;
                self.lbrack_runs.clear();
                self.sub_root_cache.clear();
                self.line_sub_or_dots.clear();
                self.line_tight_sub_or_dots.clear();
                self.line_chain_breaks.clear();
                self.line_prose_breaks.clear();
                self.line_lbracks.clear();
                self.line_quotes.clear();
                self.line_has_url_scheme = false;
                self.line_seg_index = None;
            } else {
                if b == b'[' {
                    if self.cur_lbrack_run_start.is_none() {
                        self.cur_lbrack_run_start = Some(self.scanned_to);
                    }
                    self.line_lbracks.push(self.scanned_to);
                } else if !matches!(b, b' ' | b'\t') {
                    if let Some(start) = self.cur_lbrack_run_start.take() {
                        if self.scanned_to - start >= 8 {
                            self.lbrack_runs.push((start, self.scanned_to));
                        }
                    }
                }
                if (b == b'"' || b == b'\'') && is_unescaped_quote(bytes, self.scanned_to) {
                    if b == b'"' {
                        self.dquotes.push(self.scanned_to);
                    } else {
                        self.squotes.push(self.scanned_to);
                    }
                    if let Some(match_idx) = self.quote_stack.iter().rposition(|f| f.quote_char == b) {
                        let cp = self.quote_stack[match_idx];
                        self.quote_stack.truncate(match_idx);
                        self.line_sub_or_dots.truncate(cp.sub_or_dots_len);
                        self.line_tight_sub_or_dots.truncate(cp.tight_sub_or_dots_len);
                        self.line_chain_breaks.truncate(cp.chain_breaks_len);
                        self.line_prose_breaks.truncate(cp.prose_breaks_len);
                        self.line_lbracks.truncate(cp.lbracks_len);
                        self.line_quotes.truncate(cp.quotes_len);
                        self.line_quotes.push(self.scanned_to);
                        if text[self.scanned_to + 1..]
                            .chars()
                            .next()
                            .is_some_and(|nc| is_kv_ident_char(nc) || nc == '[')
                        {
                            self.line_sub_or_dots.push(self.scanned_to);
                            self.line_tight_sub_or_dots.push(self.scanned_to);
                        }
                    } else {
                        if self.scanned_to > self.line_start
                            && text[self.line_start..self.scanned_to]
                                .chars()
                                .next_back()
                                .is_some_and(|pc| {
                                    is_kv_ident_char(pc)
                                        || matches!(pc, ']' | '.' | '[' | '"' | '\'')
                                })
                        {
                            self.line_sub_or_dots.push(self.scanned_to);
                            self.line_tight_sub_or_dots.push(self.scanned_to);
                        }
                        self.quote_stack.push(QuoteCheckpoint {
                            quote_char: b,
                            sub_or_dots_len: self.line_sub_or_dots.len(),
                            tight_sub_or_dots_len: self.line_tight_sub_or_dots.len(),
                            chain_breaks_len: self.line_chain_breaks.len(),
                            prose_breaks_len: self.line_prose_breaks.len(),
                            lbracks_len: self.line_lbracks.len(),
                            quotes_len: self.line_quotes.len(),
                        });
                    }
                } else if b == b']' {
                    self.line_sub_or_dots.push(self.scanned_to);
                    self.line_tight_sub_or_dots.push(self.scanned_to);
                } else if b == b'.' {
                    let next_is_ws = bytes
                        .get(self.scanned_to + 1)
                        .is_some_and(|&nb| nb == b' ' || nb == b'\t');
                    self.line_sub_or_dots.push(self.scanned_to);
                    if next_is_ws {
                        self.line_prose_breaks.push(self.scanned_to);
                    } else {
                        self.line_tight_sub_or_dots.push(self.scanned_to);
                    }
                } else if matches!(b, b'=' | b':' | b',' | b';' | b'&' | b'?' | b'#' | b'/') {
                    self.line_chain_breaks.push(self.scanned_to);
                    if b == b'/'
                        && !self.line_has_url_scheme
                        && self.scanned_to > self.line_start
                        && bytes[self.scanned_to - 1] == b'/'
                    {
                        let prefix = &bytes[self.line_start..=self.scanned_to];
                        if (prefix.len() >= 7
                            && prefix[prefix.len() - 7..].eq_ignore_ascii_case(b"http://"))
                            || (prefix.len() >= 8
                                && prefix[prefix.len() - 8..].eq_ignore_ascii_case(b"https://"))
                        {
                            self.line_has_url_scheme = true;
                        }
                    }
                }
            }
            self.scanned_to += 1;
        }
    }

    fn last_before(sorted: &[usize], pos: usize) -> Option<usize> {
        let idx = sorted.partition_point(|&p| p < pos);
        idx.checked_sub(1).map(|i| sorted[i])
    }

    fn has_sub_or_dot_in_cur_chain(&self, pos: usize) -> bool {
        let last_marker = Self::last_before(&self.line_sub_or_dots, pos);
        let last_break = Self::last_before(&self.line_chain_breaks, pos);
        match (last_marker, last_break) {
            (Some(m), Some(b)) => m > b,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    fn has_tight_sub_or_dot_in_cur_chain(&self, pos: usize) -> bool {
        let last_marker = Self::last_before(&self.line_tight_sub_or_dots, pos);
        let last_break = Self::last_before(&self.line_chain_breaks, pos)
            .max(Self::last_before(&self.line_prose_breaks, pos));
        match (last_marker, last_break) {
            (Some(m), Some(b)) => m > b,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    fn has_quote_lbrack_or_prose_dot_in_cur_chain(&self, pos: usize) -> bool {
        let last_marker = Self::last_before(&self.line_quotes, pos)
            .max(Self::last_before(&self.line_lbracks, pos))
            .max(Self::last_before(&self.line_prose_breaks, pos));
        let last_break = Self::last_before(&self.line_chain_breaks, pos);
        match (last_marker, last_break) {
            (Some(m), Some(b)) => m > b,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    fn lbrack_run_first(&self, pos: usize) -> Option<usize> {
        if let Some(s) = self.cur_lbrack_run_start {
            if pos >= s {
                return Some(s);
            }
        }
        let idx = self.lbrack_runs.partition_point(|&(s, _)| s <= pos);
        if let Some(prev) = idx.checked_sub(1) {
            let (s, e) = self.lbrack_runs[prev];
            if pos < e {
                return Some(s);
            }
        }
        None
    }
}

fn extract_kv_key_before_sep_with_memo<'a>(
    text: &'a str,
    sep_idx: usize,
    sep_char: char,
    after_sep_raw: &str,
    memo: &mut KvLineScanMemo,
) -> Option<(usize, &'a str)> {
    let before = &text[..sep_idx];
    let before_trim = before.trim_end();
    if before_trim.is_empty() {
        return None;
    }
    memo.advance_to(text, before_trim.len());
    let bytes = before_trim.as_bytes();
    let is_sep_eq = sep_char == '=';
    let has_ws_after_sep = after_sep_raw.starts_with(char::is_whitespace);

    // Support chained bracketed subscripts (`auth_token[0]=...`, `auth_token[[0]=...`,
    // `auth_token[0] [1]=...`, `auth_token[0].prod=...`, `auth_token..prod=...`,
    // `auth_token[0] .prod=...`, `auth_token[0] "prod"=...`, `auth_token. "prod"=...`,
    // `auth_token."a" "b"=...`, `auth_token"prod"=...`, `auth_token[0]]=...`,
    // `auth_token[0] a=...`, `auth_token[<260>]=...`, `auth_token["<260>"]=...`,
    // `auth_token["]"]=...`, `"auth_token"[0]=...`, `[auth_token]=...`), dotted/hyphenated
    // suffixes, and unbalanced brackets while keeping backward scanning O(N) via `KvLineScanMemo`.
    let is_ident_ch = is_kv_ident_char;
    let is_valid_before_cont = |ch: char| -> bool {
        ch == ']' || ch == '"' || ch == '\'' || ch == '[' || ch == '.' || is_ident_ch(ch)
    };

    let mut cursor = before_trim.len();
    let mut saw_sub_or_dot = false;
    let mut saw_non_quote_prefix = false;
    let mut saw_structural_chain = false;
    let mut allow_root_bracket = false;
    let mut first_tail_span: Option<(usize, usize, usize)> = None;
    let mut cache_checkpoints: Vec<u64> = Vec::new();
    let mut loop_steps = 0usize;

    loop {
        if cursor <= memo.line_start {
            break;
        }
        loop_steps += 1;
        if saw_sub_or_dot && saw_non_quote_prefix {
            let ck = ((cursor as u64) << 3)
                | ((saw_structural_chain as u64) << 2)
                | ((is_sep_eq as u64) << 1)
                | (has_ws_after_sep as u64);
            if let Some(&(cached_root, cached_allow_root)) = memo.sub_root_cache.get(&ck) {
                cursor = cached_root;
                allow_root_bracket = cached_allow_root;
                break;
            }
            if loop_steps >= 4 {
                cache_checkpoints.push(ck);
            }
        }
        let last_b = bytes[cursor - 1];
        if last_b == b']' {
            let close_pos = cursor - 1;
            let find_open_bracket =
                |respect_quotes: bool, memo: &KvLineScanMemo| -> (Option<usize>, bool) {
                    let mut depth = 0usize;
                    let mut in_quote: Option<char> = None;
                    let mut idx = cursor;
                    while idx > memo.line_start {
                        let (rel_idx, ch) = before_trim[memo.line_start..idx]
                            .char_indices()
                            .next_back()
                            .unwrap();
                        let abs_idx = memo.line_start + rel_idx;
                        idx = abs_idx;
                        if matches!(ch, '\n' | '\r') {
                            break;
                        }
                        if respect_quotes {
                            if let Some(q) = in_quote {
                                if ch == q && is_unescaped_quote(bytes, abs_idx) {
                                    in_quote = None;
                                }
                                continue;
                            }
                            if (ch == '"' || ch == '\'') && is_unescaped_quote(bytes, abs_idx) {
                                in_quote = Some(ch);
                                continue;
                            }
                        }
                        if ch == ']' {
                            if in_quote.is_none() && abs_idx < close_pos {
                                if respect_quotes {
                                    if let Some(&jump_open) = memo.matched_rbrack_jump.get(&abs_idx)
                                    {
                                        idx = jump_open;
                                        continue;
                                    }
                                    if memo.unmatched_rbrack.contains(&abs_idx) {
                                        return (None, false);
                                    }
                                    if memo.ended_in_quote_rbrack.contains(&abs_idx) {
                                        return (None, true);
                                    }
                                } else {
                                    if let Some(&jump_open) =
                                        memo.matched_rbrack_jump_noquote.get(&abs_idx)
                                    {
                                        idx = jump_open;
                                        continue;
                                    }
                                    if memo.unmatched_rbrack_noquote.contains(&abs_idx) {
                                        return (None, false);
                                    }
                                }
                            }
                            depth += 1;
                        } else if ch == '[' {
                            if depth == 1 {
                                return (Some(abs_idx), in_quote.is_some());
                            }
                            depth = depth.saturating_sub(1);
                        }
                    }
                    (None, in_quote.is_some())
                };
            let (mut open_pos, ended_in_quote) = find_open_bracket(true, memo);
            if let Some(op) = open_pos {
                memo.matched_rbrack_jump.insert(close_pos, op);
            } else if ended_in_quote {
                memo.ended_in_quote_rbrack.insert(close_pos);
                open_pos = find_open_bracket(false, memo).0;
                if let Some(op) = open_pos {
                    memo.matched_rbrack_jump_noquote.insert(close_pos, op);
                } else {
                    memo.unmatched_rbrack_noquote.insert(close_pos);
                }
            }
            if open_pos.is_none() {
                if !ended_in_quote {
                    memo.unmatched_rbrack.insert(close_pos);
                }
                let raw_before = &before_trim[..close_pos];
                let trimmed_before = raw_before.trim_end_matches([' ', '\t']);
                let can_cross_unbalanced = !trimmed_before.is_empty()
                    && trimmed_before
                        .chars()
                        .next_back()
                        .is_some_and(is_valid_before_cont);
                if can_cross_unbalanced {
                    cursor = trimmed_before.len();
                    saw_sub_or_dot = true;
                    saw_non_quote_prefix = true;
                    saw_structural_chain = true;
                    continue;
                }
                break;
            }
            let op = open_pos.unwrap();
            let trimmed_before = before_trim[..op].trim_end_matches([' ', '\t']);
            if trimmed_before.is_empty()
                || !trimmed_before
                    .chars()
                    .next_back()
                    .is_some_and(is_valid_before_cont)
            {
                if op + 1 < cursor {
                    cursor = op;
                    allow_root_bracket = true;
                }
                break;
            }
            cursor = trimmed_before.len();
            saw_sub_or_dot = true;
            saw_non_quote_prefix = true;
            saw_structural_chain = true;
            continue;
        }

        if last_b == b'"' || last_b == b'\'' {
            let inner_end = cursor - 1;
            if !is_unescaped_quote(bytes, inner_end) {
                break;
            }
            let q_positions = if last_b == b'"' {
                &memo.dquotes
            } else {
                &memo.squotes
            };
            let pos_idx = q_positions.partition_point(|&p| p < inner_end);
            let Some(quote_start) = pos_idx.checked_sub(1).map(|i| q_positions[i]) else {
                if saw_sub_or_dot {
                    cursor = inner_end;
                }
                break;
            };
            if quote_start < memo.line_start {
                if saw_sub_or_dot {
                    cursor = inner_end;
                }
                break;
            }
            if first_tail_span.is_none() && cursor == before_trim.len() && quote_start + 1 < inner_end
            {
                first_tail_span = Some((quote_start, quote_start + 1, inner_end));
            }
            let raw_before = &before_trim[..quote_start];
            let trimmed_before = raw_before.trim_end_matches([' ', '\t']);
            if trimmed_before.ends_with(']') {
                cursor = trimmed_before.len();
                saw_sub_or_dot = true;
                saw_non_quote_prefix = true;
                saw_structural_chain = true;
                continue;
            }
            if trimmed_before.ends_with(['"', '\'']) {
                if trimmed_before.len() == raw_before.len() {
                    saw_structural_chain = true;
                }
                cursor = trimmed_before.len();
                saw_sub_or_dot = true;
                continue;
            }
            if trimmed_before.ends_with('.') {
                let sep_pos = trimmed_before.len() - 1;
                let before_dot_raw = &before_trim[..sep_pos];
                let before_dot = before_dot_raw.trim_end_matches([' ', '\t']);
                let no_space_after_dot = trimmed_before.len() == raw_before.len();
                let space_before_dot = before_dot.len() < before_dot_raw.len();
                let rbrack_before_dot = before_dot.ends_with(']');
                let quote_before_dot = before_dot.ends_with(['"', '\'']);
                if (no_space_after_dot
                    || saw_sub_or_dot
                    || is_sep_eq
                    || !has_ws_after_sep
                    || space_before_dot
                    || rbrack_before_dot
                    || quote_before_dot)
                    && before_dot
                        .chars()
                        .next_back()
                        .is_some_and(is_valid_before_cont)
                {
                    cursor = before_dot.len();
                    saw_sub_or_dot = true;
                    saw_non_quote_prefix = true;
                    saw_structural_chain = true;
                    continue;
                }
            }
            if trimmed_before.ends_with('[') {
                let sep_pos = trimmed_before.len() - 1;
                let before_lbrack_raw = &before_trim[..sep_pos];
                let before_lbrack_trim = before_lbrack_raw.trim_end_matches([' ', '\t']);
                let no_space_after_lbrack = trimmed_before.len() == raw_before.len();
                let can_cross = if before_lbrack_trim.len() < before_lbrack_raw.len() {
                    let prev_is_sub_or_q = before_lbrack_trim
                        .chars()
                        .next_back()
                        .is_some_and(|c| matches!(c, ']' | '"' | '\'' | '['));
                    (prev_is_sub_or_q
                        || memo.has_tight_sub_or_dot_in_cur_chain(sep_pos)
                        || ((no_space_after_lbrack
                            || saw_sub_or_dot
                            || is_sep_eq
                            || !has_ws_after_sep)
                            && (is_sep_eq || !has_ws_after_sep || saw_sub_or_dot)))
                        && before_lbrack_trim
                            .chars()
                            .next_back()
                            .is_some_and(is_valid_before_cont)
                } else {
                    (no_space_after_lbrack
                        || saw_sub_or_dot
                        || is_sep_eq
                        || !has_ws_after_sep)
                        && before_lbrack_raw
                            .chars()
                            .next_back()
                            .is_some_and(is_valid_before_cont)
                };
                if can_cross {
                    cursor = before_lbrack_trim.len();
                    saw_sub_or_dot = true;
                    saw_non_quote_prefix = true;
                    saw_structural_chain = true;
                    continue;
                }
            }
            if raw_before.chars().next_back().is_some_and(is_ident_ch) {
                cursor = quote_start;
                saw_sub_or_dot = true;
                saw_non_quote_prefix = true;
                saw_structural_chain = true;
                continue;
            }
            if saw_sub_or_dot
                && !saw_structural_chain
                && sep_char == ':'
                && has_ws_after_sep
                && !memo.line_has_url_scheme
                && is_safe_h4_prose_rhs(after_sep_raw)
            {
                let ident_end = trimmed_before.len();
                let mut ident_start = ident_end;
                for (rel_idx, ch) in trimmed_before[memo.line_start..].char_indices().rev() {
                    let abs_idx = memo.line_start + rel_idx;
                    if is_ident_ch(ch) {
                        ident_start = abs_idx;
                    } else {
                        break;
                    }
                }
                if ident_start < ident_end {
                    let ident = &trimmed_before[ident_start..ident_end];
                    let before_ident = trimmed_before[..ident_start].trim_end_matches([' ', '\t']);
                    let continues_chain = before_ident.chars().next_back().is_some_and(|c| {
                        matches!(
                            c,
                            '.' | '[' | ']' | '"' | '\'' | '?' | '&' | '=' | '/' | ':' | ';' | ','
                        )
                    });
                    let ident_segs = split_key_segments(ident);
                    let ident_sensitive = is_sensitive_json_key_with_segments(ident, &ident_segs)
                        || ident_segs.iter().any(|s| s == "bearer" || s == "basic");
                    let ident_capitalized = ident
                        .as_bytes()
                        .first()
                        .is_some_and(|b| b.is_ascii_uppercase());
                    if !continues_chain && !ident_sensitive && ident_capitalized {
                        return first_tail_span
                            .filter(|&(_, ks, ke)| ks < ke)
                            .map(|(span_s, ks, ke)| (span_s, &before_trim[ks..ke]));
                    }
                }
            }
            if trimmed_before.chars().next_back().is_some_and(is_ident_ch) {
                cursor = trimmed_before.len();
                saw_sub_or_dot = true;
                saw_non_quote_prefix = true;
                continue;
            }
            if saw_sub_or_dot {
                cursor = quote_start;
                break;
            }
            let start = quote_start + 1;
            if start == inner_end {
                return None;
            }
            return Some((quote_start, &before_trim[start..inner_end]));
        }

        let mut seg_start = cursor;
        for (rel_idx, ch) in before_trim[memo.line_start..cursor].char_indices().rev() {
            let abs_idx = memo.line_start + rel_idx;
            if is_ident_ch(ch) {
                seg_start = abs_idx;
            } else {
                break;
            }
        }
        if seg_start < cursor {
            if first_tail_span.is_none() && cursor == before_trim.len() {
                first_tail_span = Some((seg_start, seg_start, cursor));
            }
            cursor = seg_start;
            saw_non_quote_prefix = true;
            if cursor > memo.line_start {
                let raw_before = &before_trim[..cursor];
                let trimmed_before = raw_before.trim_end_matches([' ', '\t']);
                if trimmed_before.ends_with('.') {
                    let sep_pos = trimmed_before.len() - 1;
                    let before_dot_raw = &before_trim[..sep_pos];
                    let before_dot = before_dot_raw.trim_end_matches([' ', '\t']);
                    let no_space_after_dot = trimmed_before.len() == raw_before.len();
                    let space_before_dot = before_dot.len() < before_dot_raw.len();
                    let quote_before_dot = before_dot.ends_with(['"', '\'']);
                    if (no_space_after_dot
                        || saw_sub_or_dot
                        || is_sep_eq
                        || !has_ws_after_sep
                        || space_before_dot
                        || quote_before_dot)
                        && before_dot
                            .chars()
                            .next_back()
                            .is_some_and(is_valid_before_cont)
                    {
                        cursor = before_dot.len();
                        saw_sub_or_dot = true;
                        if !quote_before_dot {
                            saw_structural_chain = true;
                        }
                        continue;
                    }
                } else if trimmed_before.ends_with('[') {
                    let sep_pos = trimmed_before.len() - 1;
                    let before_lbrack_raw = &before_trim[..sep_pos];
                    let before_lbrack_trim = before_lbrack_raw.trim_end_matches([' ', '\t']);
                    let no_space_after_lbrack = trimmed_before.len() == raw_before.len();
                    let can_cross = if before_lbrack_trim.len() < before_lbrack_raw.len() {
                        let prev_is_sub_or_q = before_lbrack_trim
                            .chars()
                            .next_back()
                            .is_some_and(|c| matches!(c, ']' | '"' | '\'' | '['));
                        (prev_is_sub_or_q
                            || memo.has_tight_sub_or_dot_in_cur_chain(sep_pos)
                            || ((no_space_after_lbrack
                                || saw_sub_or_dot
                                || is_sep_eq
                                || !has_ws_after_sep)
                                && (is_sep_eq || !has_ws_after_sep || saw_sub_or_dot)))
                            && before_lbrack_trim
                                .chars()
                                .next_back()
                                .is_some_and(is_valid_before_cont)
                    } else {
                        (no_space_after_lbrack
                            || saw_sub_or_dot
                            || is_sep_eq
                            || !has_ws_after_sep)
                            && before_lbrack_raw
                                .chars()
                                .next_back()
                                .is_some_and(is_valid_before_cont)
                    };
                    if can_cross {
                        cursor = before_lbrack_trim.len();
                        saw_sub_or_dot = true;
                        saw_structural_chain = true;
                        continue;
                    }
                } else if trimmed_before.ends_with(']') {
                    cursor = trimmed_before.len();
                    saw_sub_or_dot = true;
                    saw_structural_chain = true;
                    continue;
                } else if trimmed_before
                    .chars()
                    .next_back()
                    .is_some_and(|c| c == '"' || c == '\'')
                {
                    if raw_before.len() == trimmed_before.len()
                        || memo.has_tight_sub_or_dot_in_cur_chain(cursor)
                    {
                        saw_structural_chain = true;
                    }
                    cursor = trimmed_before.len();
                    saw_sub_or_dot = true;
                    continue;
                } else if (((is_sep_eq || !has_ws_after_sep)
                    && memo.has_sub_or_dot_in_cur_chain(cursor))
                    || memo.has_tight_sub_or_dot_in_cur_chain(cursor)
                    || memo.has_quote_lbrack_or_prose_dot_in_cur_chain(trimmed_before.len()))
                    && trimmed_before.chars().next_back().is_some_and(is_ident_ch)
                {
                    cursor = trimmed_before.len();
                    saw_sub_or_dot = true;
                    saw_structural_chain = true;
                    continue;
                }
            }
            break;
        }

        if matches!(last_b, b'.' | b'[')
            && (saw_sub_or_dot
                || is_sep_eq
                || !has_ws_after_sep
                || (last_b == b'.'
                    && before_trim[..cursor - 1]
                        .trim_end_matches([' ', '\t'])
                        .ends_with(']')))
        {
            let sep_pos = if last_b == b'[' {
                memo.lbrack_run_first(cursor - 1).unwrap_or(cursor - 1)
            } else {
                cursor - 1
            };
            let raw_before = &before_trim[..sep_pos];
            let trimmed_before = raw_before.trim_end_matches([' ', '\t']);
            let can_cross = if last_b == b'.' {
                trimmed_before
                    .chars()
                    .next_back()
                    .is_some_and(is_valid_before_cont)
            } else if trimmed_before.len() < raw_before.len() {
                (is_sep_eq
                    || !has_ws_after_sep
                    || saw_sub_or_dot
                    || memo.has_tight_sub_or_dot_in_cur_chain(sep_pos)
                    || trimmed_before
                        .chars()
                        .next_back()
                        .is_some_and(|c| matches!(c, ']' | '"' | '\'' | '[')))
                    && trimmed_before
                        .chars()
                        .next_back()
                        .is_some_and(is_valid_before_cont)
            } else {
                raw_before
                    .chars()
                    .next_back()
                    .is_some_and(is_valid_before_cont)
            };
            if can_cross {
                cursor = trimmed_before.len();
                saw_sub_or_dot = true;
                saw_non_quote_prefix = true;
                saw_structural_chain = true;
                continue;
            }
        }

        break;
    }
    for ck in cache_checkpoints {
        memo.sub_root_cache.insert(ck, (cursor, allow_root_bracket));
    }
    if cursor == before_trim.len()
        || before_trim[cursor..].starts_with('.')
        || (before_trim[cursor..].starts_with('[') && !allow_root_bracket)
    {
        return None;
    }
    Some((cursor, &before_trim[cursor..]))
}

fn is_clean_auth_trailing(rest: &str, open_bracket: Option<char>) -> bool {
    let line_end = rest.find(['\n', '\r']).unwrap_or(rest.len());
    let trailing = rest[..line_end].trim();
    trailing.is_empty()
        || trailing == "."
        || (trailing.len() == 1
            && trailing
                .chars()
                .next()
                .and_then(matching_open_bracket)
                .is_some_and(|expected| open_bracket == Some(expected)))
}

fn find_bearer_kv_tail_end(s: &str, open_bracket: Option<char>, in_url_query: bool) -> usize {
    let mut depth = 0usize;
    let mut in_quote: Option<char> = None;
    for (idx, ch) in s.char_indices() {
        if ch == '\n' || ch == '\r' {
            return idx;
        }
        if let Some(q) = in_quote {
            if ch == q {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                in_quote = Some(ch);
            }
            '(' | '[' | '{' => {
                depth += 1;
            }
            ')' | ']' | '}' => {
                if depth == 0 {
                    if matching_open_bracket(ch) == open_bracket {
                        return idx;
                    }
                } else {
                    depth -= 1;
                }
            }
            '&' | ';' | '#' if depth == 0 && in_url_query => {
                return idx;
            }
            ',' if depth == 0 && matches!(open_bracket, Some('{' | '[')) => {
                let after = s[idx + 1..].trim_start();
                if after.starts_with(['"', '\'']) {
                    return idx;
                }
            }
            _ => {}
        }
    }
    s.len()
}

fn is_safe_bearer_kv_rhs_text(s: &str, params: &[ScriptParameter]) -> bool {
    let mut local_memo = EnclosureOverflowMemo::default();
    for (sep_idx, _) in s.match_indices([':', '=']) {
        let rhs_raw = &s[sep_idx + 1..];
        let rhs = extract_auth_payload(rhs_raw);
        if let Some(ov) = rhs.enclosure_overflow {
            let trimmed = rhs_raw.trim_start();
            let enclosure_body = &trimmed[1..];
            if !is_safe_enclosure_overflow_impl(
                enclosure_body,
                ov,
                true,
                false,
                params,
                Some(&mut local_memo),
                s,
                false,
            ) {
                return false;
            }
            continue;
        }
        if !rhs.unwrapped.is_empty()
            && !rhs.unwrapped.contains("{{")
            && !rhs.unwrapped.contains("}}")
        {
            let non_ws = rhs.unwrapped.bytes().filter(|b| b.is_ascii_graphic()).count();
            let has_dp = rhs
                .unwrapped
                .bytes()
                .any(|b| b.is_ascii_digit() || b.is_ascii_punctuation());
            if non_ws >= 6 || (non_ws >= 4 && has_dp) {
                return false;
            }
        }
    }
    true
}

#[derive(Default)]
struct EnclosureOverflowMemo {
    last_close: Option<[Option<usize>; 4]>,
    eof_safe_from: Option<(usize, bool)>,
    matched_brackets: std::collections::HashMap<usize, (usize, usize)>,
    unclosed_opens: std::collections::HashSet<usize>,
    angle_matched_brackets: std::collections::HashMap<usize, (usize, usize)>,
    angle_unclosed_opens: std::collections::HashSet<usize>,
    verified_matched: Vec<(usize, usize, usize, bool)>,
    non_ws_prefix: Option<Vec<u32>>,
}

impl EnclosureOverflowMemo {
    fn ensure_last_close(&mut self, text: &str) -> [Option<usize>; 4] {
        if let Some(lc) = self.last_close {
            return lc;
        }
        let mut lc = [None, None, None, None];
        for (idx, b) in text.as_bytes().iter().enumerate().rev() {
            match *b {
                b')' if lc[0].is_none() => lc[0] = Some(idx),
                b']' if lc[1].is_none() => lc[1] = Some(idx),
                b'}' if lc[2].is_none() => lc[2] = Some(idx),
                b'>' if lc[3].is_none() => lc[3] = Some(idx),
                _ => {}
            }
            if lc[0].is_some() && lc[1].is_some() && lc[2].is_some() && lc[3].is_some() {
                break;
            }
        }
        self.last_close = Some(lc);
        lc
    }

    fn last_close_for(&mut self, text: &str, open: char) -> Option<usize> {
        let lc = self.ensure_last_close(text);
        match open {
            '(' => lc[0],
            '[' => lc[1],
            '{' => lc[2],
            '<' => lc[3],
            _ => None,
        }
    }

    fn ensure_non_ws_prefix(&mut self, text: &str) -> &[u32] {
        if self.non_ws_prefix.is_none() {
            let mut prefix = Vec::with_capacity(text.len() + 1);
            prefix.push(0);
            let mut count = 0u32;
            if text.is_ascii() {
                for &b in text.as_bytes() {
                    if !b.is_ascii_whitespace() {
                        count = count.saturating_add(1);
                    }
                    prefix.push(count);
                }
            } else {
                for ch in text.chars() {
                    let add = if ch.is_whitespace() { 0 } else { ch.len_utf8() as u32 };
                    for _ in 0..ch.len_utf8() {
                        count = count.saturating_add(if add > 0 { 1 } else { 0 });
                        prefix.push(count);
                    }
                }
            }
            self.non_ws_prefix = Some(prefix);
        }
        self.non_ws_prefix.as_deref().unwrap()
    }

    fn non_ws_bytes_in(&mut self, text: &str, start: usize, end: usize) -> usize {
        if start >= end || end > text.len() {
            return 0;
        }
        let prefix = self.ensure_non_ws_prefix(text);
        prefix[end].saturating_sub(prefix[start]) as usize
    }

    fn fast_overflow_is_safe(
        &mut self,
        text: &str,
        open: char,
        open_abs_pos: usize,
        body_abs_start: usize,
        max_scan_bytes: usize,
        allow_small_verified: bool,
    ) -> bool {
        let known_unclosed = self
            .last_close_for(text, open)
            .is_none_or(|last| last < body_abs_start)
            || if open == '<' {
                self.angle_unclosed_opens.contains(&open_abs_pos)
            } else {
                self.unclosed_opens.contains(&open_abs_pos)
            };
        if known_unclosed {
            if let Some((eof_from, has_ph)) = self.eof_safe_from {
                if !has_ph
                    && body_abs_start >= eof_from
                    && (allow_small_verified
                        || self.non_ws_bytes_in(text, body_abs_start, text.len()) > max_scan_bytes)
                {
                    return true;
                }
            }
            return false;
        }
        let matched_close = if open == '<' {
            self.angle_matched_brackets.get(&open_abs_pos).copied()
        } else {
            self.matched_brackets.get(&open_abs_pos).copied()
        };
        if let Some((close_abs, _)) = matched_close {
            let covered = self
                .verified_matched
                .iter()
                .any(|&(s, e, _le, has_ph)| !has_ph && body_abs_start >= s && close_abs <= e);
            if covered
                && (allow_small_verified
                    || self.non_ws_bytes_in(text, body_abs_start, close_abs) > max_scan_bytes)
            {
                return true;
            }
        }
        false
    }
}

fn has_unclosed_quote(s: &str) -> bool {
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    for ch in s.chars() {
        if let Some(q) = in_quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                in_quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
        }
    }
    in_quote.is_some()
}

fn is_safe_enclosure_overflow(
    enclosure_body: &str,
    overflow: EnclosureOverflow,
    is_bearer: bool,
    is_basic: bool,
    params: &[ScriptParameter],
    memo: Option<&mut EnclosureOverflowMemo>,
    full_text: &str,
) -> bool {
    is_safe_enclosure_overflow_impl(
        enclosure_body,
        overflow,
        is_bearer,
        is_basic,
        params,
        memo,
        full_text,
        true,
    )
}

fn is_safe_enclosure_overflow_impl(
    enclosure_body: &str,
    overflow: EnclosureOverflow,
    is_bearer: bool,
    is_basic: bool,
    params: &[ScriptParameter],
    mut memo: Option<&mut EnclosureOverflowMemo>,
    full_text: &str,
    check_nested_rhs_kv: bool,
) -> bool {
    let body_abs_start = enclosure_body.as_ptr() as usize - full_text.as_ptr() as usize;
    let open_abs_pos = body_abs_start.saturating_sub(1);

    let mut matched_end: Option<(usize, usize)> = None;
    let mut known_unclosed = false;
    let mut unclosed_quote_at_eof = false;

    if let Some(m) = memo.as_deref_mut() {
        if m.last_close_for(full_text, overflow.open)
            .is_none_or(|last| last < body_abs_start)
        {
            known_unclosed = true;
        } else {
            let (matched_map, unclosed_set) = if overflow.open == '<' {
                (&m.angle_matched_brackets, &m.angle_unclosed_opens)
            } else {
                (&m.matched_brackets, &m.unclosed_opens)
            };
            if let Some(&(close_abs, close_len)) = matched_map.get(&open_abs_pos) {
                matched_end = Some((close_abs - body_abs_start, close_len));
            } else if unclosed_set.contains(&open_abs_pos) {
                known_unclosed = true;
            }
        }
    }

    if known_unclosed {
        if let Some(m) = memo.as_deref() {
            if m.eof_safe_from
                .is_some_and(|(s, has_ph)| !has_ph && body_abs_start >= s)
            {
                return true;
            }
        }
    }

    if matched_end.is_none() && !known_unclosed {
        let mut stack: Vec<(char, usize)> = vec![(overflow.open, open_abs_pos)];
        let mut mismatched = false;
        let mut in_quote: Option<char> = None;
        let mut escaped = false;
        let mut pending_matches: Vec<(usize, (usize, usize))> = Vec::new();
        let mut pending_unclosed: Vec<usize> = Vec::new();

        if enclosure_body.is_ascii() {
            let bytes = enclosure_body.as_bytes();
            let mut idx = 0usize;
            while idx < bytes.len() {
                let b = bytes[idx];
                if let Some(q) = in_quote {
                    if escaped {
                        escaped = false;
                    } else if b == b'\\' {
                        escaped = true;
                    } else if b == q as u8 {
                        in_quote = None;
                    }
                    idx += 1;
                    continue;
                }
                if b == b'"' || b == b'\'' {
                    in_quote = Some(b as char);
                    idx += 1;
                    continue;
                }
                if !mismatched {
                    if b == b'(' || b == b'[' || b == b'{' || (overflow.open == '<' && b == b'<') {
                        stack.push((b as char, body_abs_start + idx));
                    } else if let Some(expected_open) = match b {
                        b')' => Some('('),
                        b']' => Some('['),
                        b'}' => Some('{'),
                        b'>' if overflow.open == '<'
                            && !is_angle_comparison_op(enclosure_body, idx) =>
                        {
                            Some('<')
                        }
                        _ => None,
                    } {
                        if stack.last().map(|&(c, _)| c) == Some(expected_open) {
                            let (_, open_pos) = stack.pop().unwrap();
                            let close_abs = body_abs_start + idx;
                            pending_matches.push((open_pos, (close_abs, 1)));
                            if stack.is_empty() {
                                matched_end = Some((idx, 1));
                                break;
                            }
                        } else {
                            mismatched = true;
                            for &(_, op) in &stack {
                                pending_unclosed.push(op);
                            }
                        }
                    }
                }
                idx += 1;
            }
        } else {
            for (idx, ch) in enclosure_body.char_indices() {
                if let Some(q) = in_quote {
                    if escaped {
                        escaped = false;
                    } else if ch == '\\' {
                        escaped = true;
                    } else if ch == q {
                        in_quote = None;
                    }
                    continue;
                }
                if ch == '"' || ch == '\'' {
                    in_quote = Some(ch);
                    continue;
                }
                if !mismatched {
                    if ch == '(' || ch == '[' || ch == '{' || (overflow.open == '<' && ch == '<') {
                        stack.push((ch, body_abs_start + idx));
                    } else if let Some(expected_open) = match ch {
                        ')' => Some('('),
                        ']' => Some('['),
                        '}' => Some('{'),
                        '>' if overflow.open == '<'
                            && !is_angle_comparison_op(enclosure_body, idx) =>
                        {
                            Some('<')
                        }
                        _ => None,
                    } {
                        if stack.last().map(|&(c, _)| c) == Some(expected_open) {
                            let (_, open_pos) = stack.pop().unwrap();
                            let close_len = ch.len_utf8();
                            let close_abs = body_abs_start + idx;
                            pending_matches.push((open_pos, (close_abs, close_len)));
                            if stack.is_empty() {
                                matched_end = Some((idx, close_len));
                                break;
                            }
                        } else {
                            mismatched = true;
                            for &(_, op) in &stack {
                                pending_unclosed.push(op);
                            }
                        }
                    }
                }
            }
        }

        if matched_end.is_none() {
            unclosed_quote_at_eof = in_quote.is_some();
            if !mismatched {
                for &(_, op) in &stack {
                    pending_unclosed.push(op);
                }
            }
        }

        if let Some(m) = memo.as_deref_mut() {
            let (matched_map, unclosed_set) = if overflow.open == '<' {
                (&mut m.angle_matched_brackets, &mut m.angle_unclosed_opens)
            } else {
                (&mut m.matched_brackets, &mut m.unclosed_opens)
            };
            for (op, cl) in pending_matches {
                matched_map.entry(op).or_insert(cl);
            }
            for op in pending_unclosed {
                unclosed_set.insert(op);
            }
        }
    }

    let check_inner = |inner: &str| -> bool {
        if !is_safe_bearer_or_basic_value(inner, is_bearer, is_basic, params) {
            return false;
        }
        if is_bearer {
            if check_nested_rhs_kv && !is_safe_bearer_kv_rhs_text(inner, params) {
                return false;
            }
            for raw_tok in inner.split_whitespace() {
                if raw_tok.contains("{{") || raw_tok.contains("}}") {
                    continue;
                }
                let core = raw_tok.trim_matches(|c: char| {
                    matches!(
                        c,
                        '(' | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '<'
                            | '>'
                            | '"'
                            | '\''
                            | ','
                            | ';'
                            | '.'
                            | '!'
                            | '?'
                            | ':'
                            | '='
                    )
                });
                if core.is_empty() {
                    continue;
                }
                let ascii_non_ws = core.bytes().filter(|b| b.is_ascii_graphic()).count();
                if ascii_non_ws >= 6 && !is_allowed_bearer_prose_noun(core) {
                    return false;
                }
            }
        }
        true
    };

    if let Some((close_idx, close_len)) = matched_end {
        let close_abs = body_abs_start + close_idx;
        if let Some(m) = memo.as_deref() {
            if m.verified_matched
                .iter()
                .any(|&(s, e, _le, _has_ph)| body_abs_start >= s && close_abs <= e)
            {
                return true;
            }
        }

        let inner = &enclosure_body[..close_idx];
        if !check_inner(inner) {
            return false;
        }
        let after_close = &enclosure_body[close_idx + close_len..];
        let line_end = after_close.find(['\n', '\r']).unwrap_or(after_close.len());
        let same_line_after = &after_close[..line_end];
        if !same_line_after.trim().is_empty() {
            if !is_safe_bearer_or_basic_value(same_line_after, is_bearer, is_basic, params) {
                return false;
            }
            if is_bearer && check_nested_rhs_kv && !is_safe_bearer_kv_rhs_text(same_line_after, params)
            {
                return false;
            }
        }
        if let Some(m) = memo {
            let line_end_abs = body_abs_start + close_idx + close_len + line_end;
            let has_ph = inner.contains("{{") || inner.contains("}}");
            m.verified_matched
                .push((body_abs_start, close_abs, line_end_abs, has_ph));
        }
        return true;
    }

    if (enclosure_body.contains("{{") || enclosure_body.contains("}}"))
        && (unclosed_quote_at_eof || (known_unclosed && has_unclosed_quote(enclosure_body)))
    {
        return false;
    }

    if let Some(m) = memo.as_deref() {
        if m.eof_safe_from.is_some_and(|(s, _)| body_abs_start >= s) {
            return true;
        }
    }
    if !check_inner(enclosure_body) {
        return false;
    }
    if let Some(m) = memo {
        let has_ph = enclosure_body.contains("{{") || enclosure_body.contains("}}");
        m.eof_safe_from = Some(match m.eof_safe_from {
            Some((prev, prev_ph)) => (prev.min(body_abs_start), prev_ph || has_ph),
            None => (body_abs_start, has_ph),
        });
    }
    true
}

fn is_safe_bearer_or_basic_payload(
    payload: &AuthPayload<'_>,
    tail_slice: &str,
    after_assign: &str,
    is_bearer: bool,
    is_basic: bool,
    params: &[ScriptParameter],
    in_url_query: bool,
    mut memo: Option<&mut EnclosureOverflowMemo>,
    full_text: &str,
) -> bool {
    if (payload.unwrapped.contains("{{") || payload.unwrapped.contains("}}"))
        && payload.has_broken_quote
    {
        return false;
    }
    if let Some(ov) = payload.enclosure_overflow {
        let enclosure_body = &after_assign.trim_start()[1..];
        return is_safe_enclosure_overflow(
            enclosure_body,
            ov,
            is_bearer,
            is_basic,
            params,
            memo,
            full_text,
        );
    }
    if !is_safe_bearer_or_basic_value(payload.unwrapped, is_bearer, is_basic, params) {
        return false;
    }
    if is_bearer
        && matches!(
            payload.kind,
            AuthPayloadKind::Quoted | AuthPayloadKind::Bracketed
        )
        && !payload.unwrapped.contains("{{")
        && !payload.unwrapped.contains("}}")
    {
        let ascii_non_ws = payload
            .unwrapped
            .bytes()
            .filter(|b| b.is_ascii_graphic())
            .count();
        let has_digit_or_punct = payload
            .unwrapped
            .bytes()
            .any(|b| b.is_ascii_digit() || b.is_ascii_punctuation());
        if (ascii_non_ws >= 4 && has_digit_or_punct)
            || (ascii_non_ws >= 12 && !is_allowed_bearer_prose_noun(payload.unwrapped))
        {
            return false;
        }
    }
    if payload.followed_by_kv_sep.is_some() {
        let rhs_end = find_bearer_kv_tail_end(payload.rest_after_kv_sep, None, in_url_query);
        let rhs_bounded = &payload.rest_after_kv_sep[..rhs_end];
        let rhs = extract_auth_payload(rhs_bounded);
        if (rhs.unwrapped.contains("{{") || rhs.unwrapped.contains("}}")) && rhs.has_broken_quote {
            return false;
        }
        if let Some(ov) = rhs.enclosure_overflow {
            let rhs_body = &payload.rest_after_kv_sep.trim_start()[1..];
            if !is_safe_enclosure_overflow(
                rhs_body,
                ov,
                is_bearer,
                is_basic,
                params,
                memo.as_deref_mut(),
                full_text,
            ) {
                return false;
            }
        } else {
            if !is_safe_bearer_or_basic_value(rhs.unwrapped, is_bearer, is_basic, params) {
                return false;
            }
            if is_bearer
                && !rhs.unwrapped.is_empty()
                && !rhs.unwrapped.contains("{{")
                && !rhs.unwrapped.contains("}}")
            {
                let non_ws = rhs.unwrapped.bytes().filter(|b| b.is_ascii_graphic()).count();
                let has_dp = rhs
                    .unwrapped
                    .bytes()
                    .any(|b| b.is_ascii_digit() || b.is_ascii_punctuation());
                if non_ws >= 6 || (non_ws >= 4 && has_dp) {
                    return false;
                }
            }
        }
    }
    if !tail_slice.trim().is_empty() {
        if !is_safe_bearer_or_basic_value(tail_slice, is_bearer, is_basic, params) {
            return false;
        }
        if is_bearer && !is_safe_bearer_kv_rhs_text(tail_slice, params) {
            return false;
        }
    }
    true
}

fn advance_bracket_stack(text: &str, cursor: &mut usize, target: usize, stack: &mut Vec<char>) {
    if *cursor >= target {
        return;
    }
    for ch in text[*cursor..target].chars() {
        match ch {
            '(' | '[' | '{' => stack.push(ch),
            ')' | ']' | '}' => {
                if let Some(expected_open) = matching_open_bracket(ch) {
                    if stack.last().copied() == Some(expected_open) {
                        stack.pop();
                    }
                }
            }
            _ => {}
        }
    }
    *cursor = target;
}

fn strip_closing_outer_punct_and_bracket_with_stack<'a>(
    mut s: &'a str,
    open_stack: &[char],
) -> &'a str {
    s = s.trim_end_matches(['.', ',', ';', '!', '?']);
    if !s.ends_with([')', ']', '}']) || open_stack.is_empty() {
        return s;
    }
    let mut stack_idx = open_stack.len();
    while let Some(last_ch) = s.chars().next_back() {
        if let Some(exp) = matching_open_bracket(last_ch) {
            if stack_idx > 0 && open_stack[stack_idx - 1] == exp {
                stack_idx -= 1;
                s = &s[..s.len() - last_ch.len_utf8()];
                s = s.trim_end_matches(['.', ',', ';', '!', '?']);
                continue;
            }
        }
        break;
    }
    s
}

fn effective_auth_trailing_start(
    text_base_ptr: usize,
    after_assign: &str,
    payload: &AuthPayload<'_>,
    enclosing_quote: Option<char>,
) -> Option<usize> {
    let rest = &after_assign[payload.consumed_len..];
    let rest_abs_start = rest.as_ptr() as usize - text_base_ptr;

    let skip_horiz_ws = |s: &str| -> usize {
        for (idx, ch) in s.char_indices() {
            if ch == '\n' || ch == '\r' || !ch.is_whitespace() {
                return idx;
            }
        }
        s.len()
    };

    let skip_horiz_ws_and_punct = |s: &str| -> usize {
        for (idx, ch) in s.char_indices() {
            if ch == '\n' || ch == '\r' {
                return idx;
            }
            if !ch.is_whitespace()
                && !matches!(ch, '.' | ',' | ';' | '!' | '?' | ')' | ']' | '}')
            {
                return idx;
            }
        }
        s.len()
    };

    let is_line_end_or_eof = |s: &str| -> bool {
        s.is_empty() || s.starts_with('\n') || s.starts_with('\r')
    };

    if let Some(q) = enclosing_quote {
        let closed_inside_payload = payload
            .unwrapped
            .trim_end_matches(['.', ',', ';', '!', '?', ')', ']', '}'])
            .ends_with(q);
        if closed_inside_payload {
            let off = skip_horiz_ws_and_punct(rest);
            if is_line_end_or_eof(&rest[off..]) {
                return None;
            }
            return Some(rest_abs_start + off);
        }
        let ws_off = skip_horiz_ws(rest);
        let after_ws = &rest[ws_off..];
        if is_line_end_or_eof(after_ws) {
            return None;
        }
        if let Some(after_q) = after_ws.strip_prefix(q) {
            let off = skip_horiz_ws_and_punct(after_q);
            if is_line_end_or_eof(&after_q[off..]) {
                return None;
            }
            return Some(rest_abs_start + ws_off + q.len_utf8() + off);
        }
        return Some(rest_abs_start + ws_off);
    }

    let ws_off = skip_horiz_ws(rest);
    if is_line_end_or_eof(&rest[ws_off..]) {
        return None;
    }
    Some(rest_abs_start + ws_off)
}

fn check_and_memoize_same_line_trailing(
    text: &str,
    after_assign: &str,
    payload: &AuthPayload<'_>,
    enclosing_quote: Option<char>,
    is_bearer: bool,
    is_basic: bool,
    params: &[ScriptParameter],
    safe_line: &mut Option<(usize, usize)>,
) -> bool {
    let text_ptr = text.as_ptr() as usize;
    let tail_raw_abs_start =
        (after_assign.as_ptr() as usize - text_ptr) + payload.consumed_len;
    if safe_line.is_some_and(|(s, e)| tail_raw_abs_start >= s && tail_raw_abs_start <= e) {
        return true;
    }
    let Some(eff_start) =
        effective_auth_trailing_start(text_ptr, after_assign, payload, enclosing_quote)
    else {
        return true;
    };
    if safe_line.is_some_and(|(s, e)| eff_start >= s && eff_start <= e) {
        return true;
    }
    let line_end = eff_start
        + text[eff_start..]
            .find(['\n', '\r'])
            .unwrap_or(text.len() - eff_start);
    let tail_to_scan = &text[eff_start..line_end];
    if !is_safe_bearer_or_basic_value(tail_to_scan, is_bearer, is_basic, params) {
        return false;
    }
    if is_bearer && !is_safe_bearer_kv_rhs_text(tail_to_scan, params) {
        return false;
    }
    *safe_line = Some((eff_start, line_end));
    true
}

fn find_first_bearer_kv_payload_bound(
    s: &str,
    open_bracket: Option<char>,
    in_url_query: bool,
) -> usize {
    let mut depth = 0usize;
    let mut in_quote: Option<char> = None;
    let mut saw_kv_sep = false;
    let mut i = 0usize;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        let ch = s[i..].chars().next().unwrap();
        let clen = ch.len_utf8();
        if ch == '\n' || ch == '\r' {
            return i;
        }
        if let Some(q) = in_quote {
            if ch == q {
                in_quote = None;
            }
            i += clen;
            continue;
        }
        if ch.is_whitespace() && depth == 0 {
            if !saw_kv_sep {
                let after_ws = s[i..].trim_start();
                if after_ws.starts_with([':', '=']) {
                    saw_kv_sep = true;
                    let after_sep = after_ws[1..].trim_start();
                    if after_sep.starts_with(['"', '\'', '(', '[', '<'])
                        || (after_sep.starts_with('{') && !after_sep.starts_with("{{"))
                    {
                        return s.len();
                    }
                    let off = after_sep.as_ptr() as usize - s.as_ptr() as usize;
                    i = off;
                    continue;
                }
            }
            return i;
        }
        match ch {
            '"' | '\'' => in_quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    if matching_open_bracket(ch) == open_bracket {
                        return i;
                    }
                } else {
                    depth -= 1;
                }
            }
            '&' | ';' | '#' if depth == 0 && in_url_query => return i,
            ',' if depth == 0 && matches!(open_bracket, Some('{' | '[')) => {
                if s[i + 1..].trim_start().starts_with(['"', '\'']) {
                    return i;
                }
            }
            _ => {}
        }
        i += clen;
    }
    s.len()
}

fn is_clean_kv_key_identifier(key_name: &str) -> bool {
    if key_name.is_empty() || key_name.len() > 32 || !is_sensitive_json_key(key_name) {
        return false;
    }
    let mut digit_run = 0usize;
    for b in key_name.bytes() {
        if !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'[' | b']')) {
            return false;
        }
        if b.is_ascii_digit() {
            digit_run += 1;
            if digit_run >= 4 {
                return false;
            }
        } else {
            digit_run = 0;
        }
    }
    true
}

fn check_url_tail_segments<'a>(
    effective_tail: &'a str,
    params: &[ScriptParameter],
) -> Option<&'a str> {
    let mut url_end = effective_tail.len();
    let mut i = 0usize;
    while i < effective_tail.len() {
        if effective_tail[i..].starts_with("{{") {
            if let Some(rel) = effective_tail[i + 2..].find("}}") {
                if !effective_tail[i + 2..i + 2 + rel].contains(['\n', '\r']) {
                    i += 4 + rel;
                    continue;
                }
            }
        }
        let ch = effective_tail[i..].chars().next().unwrap();
        if ch.is_whitespace() {
            url_end = i;
            break;
        }
        i += ch.len_utf8();
    }
    let url_tail = &effective_tail[..url_end];
    let post_url_tail = effective_tail[url_end..].trim_start();

    let mut seg_start = 0usize;
    let mut pos = 0usize;
    let check_seg = |seg: &str| -> bool {
        let seg = seg.trim_start_matches(['&', ';', '#']);
        if seg.is_empty() {
            return true;
        }
        let mut eq_outside: Option<usize> = None;
        let mut s_pos = 0usize;
        while s_pos < seg.len() {
            if seg[s_pos..].starts_with("{{") {
                if let Some(rel) = seg[s_pos + 2..].find("}}") {
                    if !seg[s_pos + 2..s_pos + 2 + rel].contains(['\n', '\r']) {
                        s_pos += 4 + rel;
                        continue;
                    }
                }
            }
            let c = seg[s_pos..].chars().next().unwrap();
            if c == '=' {
                eq_outside = Some(s_pos);
                break;
            }
            s_pos += c.len_utf8();
        }
        let target = if let Some(eq_idx) = eq_outside {
            if eq_idx > 0 && !seg[..eq_idx].contains("{{") && !seg[..eq_idx].contains("}}") {
                return true;
            }
            if eq_idx == 0 {
                &seg[1..]
            } else {
                seg
            }
        } else {
            seg
        };
        if target.contains("{{") || target.contains("}}") {
            let mut rest = target;
            while let Some(start) = rest.find("{{") {
                let after = &rest[start + 2..];
                let Some(end) = after.find("}}") else {
                    return false;
                };
                let name = after[..end].trim();
                let valid_param = !name.is_empty()
                    && !name.contains("{{")
                    && params.iter().any(|p| {
                        p.name.trim() == name && (!p.is_secret || p.default_value.is_none())
                    });
                if !valid_param {
                    return false;
                }
                rest = &after[end + 2..];
            }
            if rest.contains("}}") {
                return false;
            }
            let literal_only = replace_placeholders_with(target, " ");
            return is_safe_bearer_or_basic_value(&literal_only, true, true, params);
        }
        is_safe_bearer_or_basic_value(target, true, true, params)
    };
    while pos < url_tail.len() {
        if url_tail[pos..].starts_with("{{") {
            if let Some(rel) = url_tail[pos + 2..].find("}}") {
                if !url_tail[pos + 2..pos + 2 + rel].contains(['\n', '\r']) {
                    pos += 4 + rel;
                    continue;
                }
            }
        }
        let ch = url_tail[pos..].chars().next().unwrap();
        if matches!(ch, '&' | ';' | '#') {
            if !check_seg(&url_tail[seg_start..pos]) {
                return None;
            }
            seg_start = pos + 1;
        }
        pos += ch.len_utf8();
    }
    if !check_seg(&url_tail[seg_start..]) {
        return None;
    }
    Some(post_url_tail)
}

fn is_safe_bearer_kv_rhs_text_prefix(
    s: &str,
    scan_limit: usize,
    params: &[ScriptParameter],
) -> bool {
    let mut local_memo = EnclosureOverflowMemo::default();
    let prefix = &s[..scan_limit.min(s.len())];
    for (sep_idx, _) in prefix.match_indices([':', '=']) {
        let rhs_raw = &s[sep_idx + 1..];
        let rhs = extract_auth_payload(rhs_raw);
        if let Some(ov) = rhs.enclosure_overflow {
            let trimmed = rhs_raw.trim_start();
            let enclosure_body = &trimmed[1..];
            if !is_safe_enclosure_overflow_impl(
                enclosure_body,
                ov,
                true,
                false,
                params,
                Some(&mut local_memo),
                s,
                false,
            ) {
                return false;
            }
            continue;
        }
        if !rhs.unwrapped.is_empty()
            && !rhs.unwrapped.contains("{{")
            && !rhs.unwrapped.contains("}}")
        {
            let non_ws = rhs.unwrapped.bytes().filter(|b| b.is_ascii_graphic()).count();
            let has_dp = rhs
                .unwrapped
                .bytes()
                .any(|b| b.is_ascii_digit() || b.is_ascii_punctuation());
            if non_ws >= 6 || (non_ws >= 4 && has_dp) {
                return false;
            }
        }
    }
    true
}

fn is_safe_kv_same_line_trailing(
    text: &str,
    after_sep: &str,
    consumed_len: usize,
    open_bracket: Option<char>,
    in_url_query: bool,
    params: &[ScriptParameter],
    kv_safe_line: &mut Option<(usize, usize, bool)>,
    kv_url_safe_line: &mut Option<(usize, usize, bool)>,
) -> bool {
    let text_ptr = text.as_ptr() as usize;
    let tail_abs_start = (after_sep.as_ptr() as usize - text_ptr) + consumed_len;
    if !in_url_query
        && kv_safe_line.is_some_and(|(s, e, _)| tail_abs_start >= s && tail_abs_start <= e)
    {
        return true;
    }
    if in_url_query
        && kv_url_safe_line.is_some_and(|(s, e, _)| tail_abs_start >= s && tail_abs_start <= e)
    {
        return true;
    }
    let rest = &after_sep[consumed_len..];
    let mut horiz_ws_end = rest.len();
    for (idx, ch) in rest.char_indices() {
        if ch == '\n' || ch == '\r' || !ch.is_whitespace() {
            horiz_ws_end = idx;
            break;
        }
    }
    let after_ws = &rest[horiz_ws_end..];
    if after_ws.is_empty() || after_ws.starts_with('\n') || after_ws.starts_with('\r') {
        return true;
    }
    let line_end = tail_abs_start + rest.find(['\n', '\r']).unwrap_or(rest.len());
    let same_line_tail = &text[tail_abs_start..line_end];

    let mut effective_tail = same_line_tail.trim_start();
    let url_tail_abs_start = effective_tail.as_ptr() as usize - text_ptr;
    if in_url_query && effective_tail.starts_with(['&', ';', '#']) {
        let Some(post_url) = check_url_tail_segments(effective_tail, params) else {
            return false;
        };
        effective_tail = post_url;
    }

    if let Some(ob) = open_bracket {
        if let Some(first) = effective_tail.chars().next() {
            if matching_open_bracket(first) == Some(ob) {
                effective_tail = effective_tail[first.len_utf8()..].trim_start();
            }
        }
    }

    if effective_tail.is_empty() {
        if in_url_query {
            let ws_b = url_tail_abs_start == 0
                || text.as_bytes()[url_tail_abs_start - 1].is_ascii_whitespace();
            let new_s = kv_url_safe_line
                .filter(|&(_, e, _)| e == line_end)
                .map_or(url_tail_abs_start, |(prev_s, _, _)| prev_s.min(url_tail_abs_start));
            *kv_url_safe_line = Some((new_s, line_end, ws_b));
        }
        return true;
    }

    let eff_abs_start = effective_tail.as_ptr() as usize - text_ptr;
    let eff_is_ws_boundary =
        eff_abs_start == 0 || text.as_bytes()[eff_abs_start - 1].is_ascii_whitespace();
    let active_watermark = if in_url_query {
        kv_url_safe_line.as_ref()
    } else {
        kv_safe_line.as_ref()
    };
    let scan_limit = match active_watermark {
        Some(&(s, e, true)) if e == line_end && eff_abs_start < s => s - eff_abs_start,
        _ => effective_tail.len(),
    };

    let toks_slice = effective_tail[..scan_limit].trim_end();
    let mut toks = toks_slice.split_whitespace().peekable();
    while let Some(raw_tok) = toks.next() {
        let next_is_sep = toks
            .peek()
            .is_some_and(|n| *n == ":" || *n == "=" || n.starts_with([':', '=']));
        let key_name = raw_tok.trim_end_matches([':', '=']);
        let has_following_val = if raw_tok.ends_with([':', '=']) {
            toks.peek().is_some()
        } else {
            next_is_sep
        };
        let is_kv_label = (raw_tok.ends_with([':', '=']) || next_is_sep)
            && has_following_val
            && !raw_tok.contains(['"', '\'', '{', '}'])
            && (is_safe_bearer_or_basic_value(key_name, true, true, params)
                || is_clean_kv_key_identifier(key_name));
        if is_kv_label {
            continue;
        }
        if !is_safe_bearer_or_basic_value(raw_tok, true, true, params) {
            return false;
        }
    }
    if !is_safe_bearer_kv_rhs_text_prefix(effective_tail, scan_limit, params) {
        return false;
    }
    if in_url_query {
        let ws_b = url_tail_abs_start == 0
            || text.as_bytes()[url_tail_abs_start - 1].is_ascii_whitespace();
        let new_s = kv_url_safe_line
            .filter(|&(_, e, _)| e == line_end)
            .map_or(url_tail_abs_start, |(prev_s, _, _)| prev_s.min(url_tail_abs_start));
        *kv_url_safe_line = Some((new_s, line_end, ws_b));
    } else {
        *kv_safe_line = Some((eff_abs_start, line_end, eff_is_ws_boundary));
    }
    true
}

struct BackwardCtxIndex {
    bracket_top_changes: Vec<(usize, Option<char>)>,
    url_in_query_changes: Vec<(usize, bool)>,
}

impl BackwardCtxIndex {
    fn build(text: &str, lower: &str) -> Self {
        let mut bracket_top_changes = vec![(0, None)];
        let mut stack: Vec<char> = Vec::new();
        let mut last_top: Option<char> = None;

        let mut url_in_query_changes = vec![(0, false)];
        let mut last_url_prefix_start = 0usize;
        let mut url_has_scheme = false;
        let mut url_has_qmark = false;
        let mut url_has_slash = false;
        let mut cur_in_uq = false;
        let mut next_dbrace: usize = 0;
        let mut next_nl: usize = 0;

        let bytes = text.as_bytes();
        let mut it = text.char_indices().peekable();
        while let Some((abs_i, c)) = it.next() {
            let next_pos = abs_i + c.len_utf8();
            match c {
                '(' | '[' | '{' => {
                    stack.push(c);
                    let top = Some(c);
                    if top != last_top {
                        bracket_top_changes.push((next_pos, top));
                        last_top = top;
                    }
                }
                ')' | ']' | '}' => {
                    if let Some(expected_open) = matching_open_bracket(c) {
                        if stack.last().copied() == Some(expected_open) {
                            stack.pop();
                            let top = stack.last().copied();
                            if top != last_top {
                                bracket_top_changes.push((next_pos, top));
                                last_top = top;
                            }
                        }
                    }
                }
                _ => {}
            }

            if c == '{' && text[abs_i..].starts_with("{{") {
                let search_from = abs_i + 2;
                if next_dbrace < search_from {
                    next_dbrace = text[search_from..]
                        .find("}}")
                        .map_or(usize::MAX, |r| search_from + r);
                }
                if next_nl < search_from {
                    next_nl = text[search_from..]
                        .find(['\n', '\r'])
                        .map_or(usize::MAX, |r| search_from + r);
                }
                if next_dbrace < usize::MAX && next_dbrace < next_nl {
                    let ph_end = next_dbrace + 2;
                    while it.peek().is_some_and(|&(ri, _)| ri < ph_end) {
                        let (ri, pc) = it.next().unwrap();
                        let p_next = ri + pc.len_utf8();
                        match pc {
                            '(' | '[' | '{' => {
                                stack.push(pc);
                                let top = Some(pc);
                                if top != last_top {
                                    bracket_top_changes.push((p_next, top));
                                    last_top = top;
                                }
                            }
                            ')' | ']' | '}' => {
                                if let Some(expected_open) = matching_open_bracket(pc) {
                                    if stack.last().copied() == Some(expected_open) {
                                        stack.pop();
                                        let top = stack.last().copied();
                                        if top != last_top {
                                            bracket_top_changes.push((p_next, top));
                                            last_top = top;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
                if let Some(&(ri, pc)) = it.peek() {
                    if ri == abs_i + 1 && pc == '{' {
                        it.next();
                        let p_next = ri + 1;
                        stack.push('{');
                        if last_top != Some('{') {
                            bracket_top_changes.push((p_next, Some('{')));
                            last_top = Some('{');
                        }
                    }
                }
                continue;
            }

            if c == '['
                && abs_i > 0
                && matches!(
                    bytes[abs_i - 1],
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b']' | b'%'
                )
            {
                continue;
            }

            if c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '(' | '[' | '{') {
                last_url_prefix_start = next_pos;
                url_has_scheme = false;
                url_has_qmark = false;
                url_has_slash = false;
                if cur_in_uq {
                    cur_in_uq = false;
                    url_in_query_changes.push((next_pos, false));
                }
            } else if c == '?' {
                url_has_qmark = true;
                let new_in_uq =
                    url_has_scheme || url_has_slash || abs_i == last_url_prefix_start;
                if new_in_uq != cur_in_uq {
                    cur_in_uq = new_in_uq;
                    url_in_query_changes.push((next_pos, new_in_uq));
                }
            } else if c == '/' {
                url_has_slash = true;
                if !url_has_scheme && abs_i > last_url_prefix_start && bytes[abs_i - 1] == b'/' {
                    let prefix_slice = &lower[last_url_prefix_start..=abs_i];
                    if prefix_slice.ends_with("http://") || prefix_slice.ends_with("https://") {
                        url_has_scheme = true;
                    }
                }
                let new_in_uq = url_has_scheme || url_has_qmark;
                if new_in_uq != cur_in_uq {
                    cur_in_uq = new_in_uq;
                    url_in_query_changes.push((next_pos, new_in_uq));
                }
            }
        }

        Self {
            bracket_top_changes,
            url_in_query_changes,
        }
    }

    fn open_bracket_at(&self, pos: usize) -> Option<char> {
        let idx = self.bracket_top_changes.partition_point(|&(p, _)| p <= pos);
        self.bracket_top_changes[idx - 1].1
    }

    fn url_in_query_at(&self, pos: usize) -> bool {
        let idx = self.url_in_query_changes.partition_point(|&(p, _)| p <= pos);
        self.url_in_query_changes[idx - 1].1
    }
}

fn scan_variant_clean(
    text: &str,
    context: &str,
    params: &[ScriptParameter],
    verified_json_spans: &[(usize, usize)],
) -> Result<(), BotBrowserError> {
    let lower = text.to_ascii_lowercase();
    let text_ptr = text.as_ptr() as usize;
    let in_verified_json = |pos: usize| verified_json_spans.iter().any(|&(s, e)| pos >= s && pos < e);
    let mut overflow_bearer_memo = EnclosureOverflowMemo::default();
    let mut overflow_basic_memo = EnclosureOverflowMemo::default();
    let mut kv_safe_line: Option<(usize, usize, bool)> = None;
    let mut kv_url_safe_line: Option<(usize, usize, bool)> = None;
    let mut bearer_url_safe_line: Option<(usize, usize)> = None;
    let mut basic_url_safe_line: Option<(usize, usize)> = None;
    let mut bearer_kv_safe_line: Option<(usize, usize)> = None;
    let mut basic_kv_safe_line: Option<(usize, usize)> = None;
    let mut both_kv_safe_line: Option<(usize, usize)> = None;
    let mut bearer_safe_line: Option<(usize, usize)> = None;
    let mut basic_safe_line: Option<(usize, usize)> = None;
    let mut kv_bracket_stack: Vec<char> = Vec::new();
    let mut kv_bracket_cursor = 0usize;
    let mut kv_line_memo = KvLineScanMemo::default();
    let mut url_delim_cursor = 0usize;
    let mut last_url_prefix_start = 0usize;
    let mut url_has_scheme = false;
    let mut url_has_qmark = false;
    let mut url_has_slash = false;
    let mut fwd_next_dbrace: usize = 0;
    let mut fwd_next_nl: usize = 0;
    let mut key_cache: [Option<(&str, bool, bool, bool, bool)>; 8] = [None; 8];
    let mut key_cache_next = 0usize;
    let mut backward_ctx: Option<BackwardCtxIndex> = None;

    // 1. Key-value assignment leaks (auth_token=..., authToken=..., auth_token_1=..., ct0=..., etc.)
    let text_bytes = text.as_bytes();
    let mut sep_pos = 0usize;
    while sep_pos < text_bytes.len() {
        let b = text_bytes[sep_pos];
        let sep_idx = sep_pos;
        if b < 0x80 {
            sep_pos += 1;
            if b != b'=' && b != b':' {
                continue;
            }
        } else {
            let ch = text[sep_pos..].chars().next().unwrap();
            sep_pos += ch.len_utf8();
            continue;
        }
        let ch = b as char;
        if in_verified_json(sep_idx) {
            continue;
        }
        let after_sep_raw = &text[sep_idx + 1..];
        let Some((key_start, key)) = extract_kv_key_before_sep_with_memo(
            text,
            sep_idx,
            ch,
            after_sep_raw,
            &mut kv_line_memo,
        ) else {
            continue;
        };
        let after_sep = after_sep_raw.trim_start();
        if ch == ':'
            && text[..key_start].ends_with("://")
            && key.contains('.')
            && after_sep
                .split(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#'))
                .next()
                .is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let key_end = key_start + key.len();
        let (is_sensitive_key, has_bearer_seg, has_basic_seg, is_bearer_or_basic_key) =
            if let Some(&(_, s, br, ba, bb)) = key_cache
                .iter()
                .flatten()
                .find(|&&(cached_k, _, _, _, _)| cached_k == key)
            {
                (s, br, ba, bb)
            } else {
                let (s, br, ba, bb) = if key.len() > 128 && !key.contains('\\') {
                    let seg_idx = kv_line_memo
                        .line_seg_index
                        .get_or_insert_with(|| LineSegIndex::build(text, kv_line_memo.line_start));
                    let clean_start = key_start == seg_idx.line_start
                        || !text.as_bytes()[key_start - 1].is_ascii_alphanumeric()
                        || !text.as_bytes()[key_start].is_ascii_alphanumeric();
                    let clean_end = key_end == seg_idx.line_end
                        || !text.as_bytes()[key_end - 1].is_ascii_alphanumeric()
                        || !text.as_bytes()[key_end].is_ascii_alphanumeric();
                    if key_start >= seg_idx.line_start
                        && key_end <= seg_idx.line_end
                        && clean_start
                        && clean_end
                    {
                        let (s, br, ba) = seg_idx.query(key_start, key_end);
                        let key_trimmed = key.trim();
                        let bb = !s
                            && (br || ba)
                            && !key_trimmed.eq_ignore_ascii_case("bearer")
                            && !key_trimmed.eq_ignore_ascii_case("basic");
                        (s, br, ba, bb)
                    } else {
                        let key_segments = split_key_segments(key);
                        let s = is_sensitive_json_key_with_segments(key, &key_segments);
                        let br = key_segments.iter().any(|seg| seg == "bearer");
                        let ba = key_segments.iter().any(|seg| seg == "basic");
                        let key_trimmed = key.trim();
                        let bb = !s
                            && (br || ba)
                            && !key_trimmed.eq_ignore_ascii_case("bearer")
                            && !key_trimmed.eq_ignore_ascii_case("basic");
                        (s, br, ba, bb)
                    }
                } else {
                    let mut key_segments = split_key_segments(key);
                    let mut s = is_sensitive_json_key_with_segments(key, &key_segments);
                    if !s && key.contains('\\') {
                        let unesc = unescape_json_for_scan(key);
                        if unesc != key {
                            let unesc_segs = split_key_segments(&unesc);
                            s = is_sensitive_json_key_with_segments(&unesc, &unesc_segs);
                            key_segments = unesc_segs;
                        }
                    }
                    let br = key_segments.iter().any(|seg| seg == "bearer");
                    let ba = key_segments.iter().any(|seg| seg == "basic");
                    let key_trimmed = key.trim();
                    let bb = !s
                        && (br || ba)
                        && !key_trimmed.eq_ignore_ascii_case("bearer")
                        && !key_trimmed.eq_ignore_ascii_case("basic");
                    (s, br, ba, bb)
                };
                key_cache[key_cache_next] = Some((key, s, br, ba, bb));
                key_cache_next = (key_cache_next + 1) & 7;
                (s, br, ba, bb)
            };
        if !is_sensitive_key && !is_bearer_or_basic_key {
            continue;
        }
        let (open_bracket, in_url_query) =
            if key_start < kv_bracket_cursor || key_start < url_delim_cursor {
                let ctx = backward_ctx
                    .get_or_insert_with(|| BackwardCtxIndex::build(text, &lower));
                (
                    ctx.open_bracket_at(key_start),
                    ctx.url_in_query_at(key_start),
                )
            } else {
                advance_bracket_stack(text, &mut kv_bracket_cursor, key_start, &mut kv_bracket_stack);
                let ob = kv_bracket_stack.last().copied();
                let mut it = text[url_delim_cursor..key_start].char_indices().peekable();
                while let Some((rel_i, c)) = it.next() {
                    let abs_i = url_delim_cursor + rel_i;
                    if c == '{' && text[abs_i..].starts_with("{{") {
                        let search_from = abs_i + 2;
                        if fwd_next_dbrace < search_from {
                            fwd_next_dbrace = text[search_from..]
                                .find("}}")
                                .map_or(usize::MAX, |r| search_from + r);
                        }
                        if fwd_next_nl < search_from {
                            fwd_next_nl = text[search_from..]
                                .find(['\n', '\r'])
                                .map_or(usize::MAX, |r| search_from + r);
                        }
                        if fwd_next_dbrace < key_start && fwd_next_dbrace < fwd_next_nl {
                            let ph_end = fwd_next_dbrace + 2;
                            while it
                                .peek()
                                .is_some_and(|&(ri, _)| url_delim_cursor + ri < ph_end)
                            {
                                it.next();
                            }
                            continue;
                        }
                        it.next();
                        continue;
                    }
                    if c == '['
                        && abs_i > 0
                        && matches!(
                            text.as_bytes()[abs_i - 1],
                            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b']' | b'%'
                        )
                    {
                        continue;
                    }
                    if c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '(' | '[' | '{') {
                        last_url_prefix_start = abs_i + c.len_utf8();
                        url_has_scheme = false;
                        url_has_qmark = false;
                        url_has_slash = false;
                    } else if c == '?' {
                        url_has_qmark = true;
                    } else if c == '/' {
                        url_has_slash = true;
                        if !url_has_scheme
                            && abs_i > last_url_prefix_start
                            && text.as_bytes()[abs_i - 1] == b'/'
                        {
                            let prefix_slice = &lower[last_url_prefix_start..=abs_i];
                            if prefix_slice.ends_with("http://")
                                || prefix_slice.ends_with("https://")
                            {
                                url_has_scheme = true;
                            }
                        }
                    }
                }
                url_delim_cursor = key_start;
                let in_uq = url_has_scheme
                    || (url_has_qmark
                        && (url_has_slash
                            || text[last_url_prefix_start..key_start].starts_with('?')));
                (ob, in_uq)
            };
        if is_bearer_or_basic_key {
            if after_sep.is_empty() {
                continue;
            }
            let starts_enclosed = after_sep.starts_with(['"', '\'', '(', '[', '<'])
                || (after_sep.starts_with('{') && !after_sep.starts_with("{{"));
            let (payload, bounded_rhs, rest_after_payload) = if starts_enclosed {
                let payload = extract_auth_payload(after_sep);
                let rest = &after_sep[payload.consumed_len..];
                (payload, after_sep, rest)
            } else {
                let rhs_end = if in_url_query {
                    find_bearer_kv_tail_end(after_sep, open_bracket, true)
                } else {
                    find_first_bearer_kv_payload_bound(after_sep, open_bracket, false)
                };
                let bounded_rhs = &after_sep[..rhs_end];
                let payload = extract_auth_payload(bounded_rhs);
                let rest = &after_sep[payload.consumed_len..];
                (payload, bounded_rhs, rest)
            };

            let mut memoized_url_span: Option<(usize, usize)> = None;
            let mut memoized_tail_span: Option<(usize, usize)> = None;
            let tail_slice = if payload.enclosure_overflow.is_some() {
                ""
            } else if in_url_query {
                let rest_abs_start = rest_after_payload.as_ptr() as usize - text_ptr;
                let rest_trimmed = rest_after_payload
                    .trim_start_matches(|c: char| c.is_whitespace() && c != '\n' && c != '\r');
                if rest_trimmed.is_empty() || rest_trimmed.starts_with(['\n', '\r']) {
                    ""
                } else if rest_trimmed.starts_with(['&', ';', '#']) {
                    let url_already_safe = kv_url_safe_line
                        .is_some_and(|(s, e, _)| rest_abs_start >= s && rest_abs_start <= e)
                        || (has_bearer_seg
                            && !has_basic_seg
                            && bearer_url_safe_line
                                .is_some_and(|(s, e)| rest_abs_start >= s && rest_abs_start <= e))
                        || (has_basic_seg
                            && !has_bearer_seg
                            && basic_url_safe_line
                                .is_some_and(|(s, e)| rest_abs_start >= s && rest_abs_start <= e));
                    if url_already_safe {
                        ""
                    } else {
                        let line_end = rest_abs_start
                            + rest_after_payload
                                .find(['\n', '\r'])
                                .unwrap_or(rest_after_payload.len());
                        let url_tail_abs_start = rest_trimmed.as_ptr() as usize - text_ptr;
                        let same_line_rest = &text[url_tail_abs_start..line_end];
                        let Some(post_url) = check_url_tail_segments(same_line_rest, params) else {
                            return Err(BotBrowserError::SecretLeakDetected(format!(
                                "plaintext '{key}' secret detected in {context}"
                            )));
                        };
                        memoized_url_span = Some((url_tail_abs_start, line_end));
                        post_url
                    }
                } else {
                    let safe_line_ref = if has_bearer_seg && !has_basic_seg {
                        &bearer_kv_safe_line
                    } else if has_basic_seg && !has_bearer_seg {
                        &basic_kv_safe_line
                    } else {
                        &both_kv_safe_line
                    };
                    if safe_line_ref.is_some_and(|(s, e)| rest_abs_start >= s && rest_abs_start <= e)
                    {
                        ""
                    } else {
                        let line_end = rest_abs_start
                            + rest_after_payload
                                .find(['\n', '\r'])
                                .unwrap_or(rest_after_payload.len());
                        memoized_tail_span = Some((rest_abs_start, line_end));
                        &text[rest_abs_start..line_end]
                    }
                }
            } else {
                let rest_abs_start = rest_after_payload.as_ptr() as usize - text_ptr;
                let safe_line_ref = if has_bearer_seg && !has_basic_seg {
                    &bearer_kv_safe_line
                } else if has_basic_seg && !has_bearer_seg {
                    &basic_kv_safe_line
                } else {
                    &both_kv_safe_line
                };
                if safe_line_ref.is_some_and(|(s, e)| rest_abs_start >= s && rest_abs_start <= e) {
                    ""
                } else {
                    let tail_end = find_bearer_kv_tail_end(rest_after_payload, open_bracket, false);
                    if tail_end > 0 {
                        memoized_tail_span = Some((rest_abs_start, rest_abs_start + tail_end));
                    }
                    &rest_after_payload[..tail_end]
                }
            };

            let memo_opt = if has_bearer_seg && !has_basic_seg {
                Some(&mut overflow_bearer_memo)
            } else if has_basic_seg && !has_bearer_seg {
                Some(&mut overflow_basic_memo)
            } else {
                None
            };
            if !is_safe_bearer_or_basic_payload(
                &payload,
                tail_slice,
                bounded_rhs,
                has_bearer_seg,
                has_basic_seg,
                params,
                in_url_query,
                memo_opt,
                text,
            ) {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext '{key}' secret detected in {context}"
                )));
            }
            if let Some((s, e)) = memoized_url_span {
                if has_bearer_seg && !has_basic_seg {
                    bearer_url_safe_line = Some((s, e));
                } else if has_basic_seg && !has_bearer_seg {
                    basic_url_safe_line = Some((s, e));
                } else {
                    let ws_b = s == 0 || text.as_bytes()[s - 1].is_ascii_whitespace();
                    kv_url_safe_line = Some((s, e, ws_b));
                }
            }
            if let Some((s, e)) = memoized_tail_span {
                if has_bearer_seg && !has_basic_seg {
                    bearer_kv_safe_line = Some((s, e));
                } else if has_basic_seg && !has_bearer_seg {
                    basic_kv_safe_line = Some((s, e));
                } else {
                    both_kv_safe_line = Some((s, e));
                }
            }
            continue;
        }
        let (token, consumed_len) =
            extract_kv_value_token_with_consumed(after_sep, open_bracket, in_url_query);
        if token.is_empty() {
            if !is_safe_kv_same_line_trailing(
                text,
                after_sep,
                consumed_len,
                open_bracket,
                in_url_query,
                params,
                &mut kv_safe_line,
                &mut kv_url_safe_line,
            ) {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext '{key}' secret detected in {context}"
                )));
            }
            continue;
        }
        let key_base_lower = key
            .split('[')
            .next()
            .unwrap_or(key)
            .trim()
            .trim_matches(['"', '\''])
            .to_ascii_lowercase();
        if key_base_lower == "authorization"
            || key_base_lower.ends_with("-authorization")
            || key_base_lower.ends_with("_authorization")
        {
            let mut after_closing_quote: Option<&str> = None;
            let scheme_after = {
                let lower_after = after_sep.to_ascii_lowercase();
                if let Some(rest) = lower_after
                    .strip_prefix("bearer")
                    .or_else(|| lower_after.strip_prefix("basic"))
                {
                    let scheme_len = lower_after.len() - rest.len();
                    let orig_rest = &after_sep[scheme_len..];
                    if orig_rest.chars().next().is_some_and(|c| {
                        c.is_whitespace()
                            || matches!(c, ':' | '=' | '"' | '\'' | '(' | '[' | '{' | '<')
                    }) {
                        Some(orig_rest)
                    } else {
                        None
                    }
                } else if let Some(q @ ('"' | '\'')) = after_sep.chars().next() {
                    let q_inner = &after_sep[1..];
                    if let Some(end) = q_inner.find([q, '\n', '\r']) {
                        if q_inner.as_bytes()[end] == q as u8 {
                            let after_close = &q_inner[end + 1..];
                            if is_clean_outer_kv_boundary(after_close, open_bracket, in_url_query) {
                                let inner_trimmed = q_inner[..end].trim();
                                let lower_inner = inner_trimmed.to_ascii_lowercase();
                                if let Some(rest) = lower_inner
                                    .strip_prefix("bearer")
                                    .or_else(|| lower_inner.strip_prefix("basic"))
                                {
                                    let scheme_len = lower_inner.len() - rest.len();
                                    let orig_rest = &inner_trimmed[scheme_len..];
                                    if orig_rest.chars().next().is_some_and(|c| {
                                        c.is_whitespace()
                                            || matches!(
                                                c,
                                                ':' | '=' | '"' | '\'' | '(' | '[' | '{' | '<'
                                            )
                                    }) {
                                        after_closing_quote = Some(after_close);
                                        Some(orig_rest)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(after_scheme_raw) = scheme_after {
                let s1 = after_scheme_raw.trim_start();
                let after_assign = if s1.starts_with(':') || s1.starts_with('=') {
                    s1[1..].trim_start()
                } else {
                    s1
                };
                let payload = extract_auth_payload(after_assign);
                let rest_same_line = &after_assign[payload.consumed_len..];
                let clean_inner = is_clean_auth_trailing(
                    rest_same_line,
                    if after_closing_quote.is_some() {
                        None
                    } else {
                        open_bracket
                    },
                );
                let clean_outer = after_closing_quote
                    .is_none_or(|after_close| is_clean_auth_trailing(after_close, open_bracket));
                if !clean_inner
                    || !clean_outer
                    || payload.followed_by_kv_sep.is_some()
                    || payload.has_broken_quote
                    || (!payload.unwrapped.is_empty()
                        && !is_protected_placeholder(payload.unwrapped, params))
                {
                    return Err(BotBrowserError::SecretLeakDetected(format!(
                        "plaintext '{key}' secret detected in {context}"
                    )));
                }
                continue;
            }
        }
        if is_protected_placeholder(token, params) {
            if !is_safe_kv_same_line_trailing(
                text,
                after_sep,
                consumed_len,
                open_bracket,
                in_url_query,
                params,
                &mut kv_safe_line,
                &mut kv_url_safe_line,
            ) {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext '{key}' secret detected in {context}"
                )));
            }
            continue;
        }
        return Err(BotBrowserError::SecretLeakDetected(format!(
            "plaintext '{key}' secret detected in {context}"
        )));
    }

    // 2. Bearer tokens ("Authorization: Bearer ..." or "Bearer <token>")
    let mut search_from = 0usize;
    let mut bearer_bracket_stack: Vec<char> = Vec::new();
    let mut bearer_bracket_cursor = 0usize;
    while let Some(rel_idx) = lower[search_from..].find("bearer") {
        let abs_idx = search_from + rel_idx;
        search_from = abs_idx + 6;
        if in_verified_json(abs_idx) {
            continue;
        }
        if abs_idx > 0 && lower.as_bytes()[abs_idx - 1].is_ascii_alphanumeric() {
            continue;
        }
        let enclosing_quote = (abs_idx > 0
            && matches!(text.as_bytes()[abs_idx - 1], b'"' | b'\''))
        .then(|| text.as_bytes()[abs_idx - 1] as char);
        let after = &text[abs_idx + 6..];
        if !after
            .chars()
            .next()
            .is_some_and(|c| {
                c.is_whitespace() || matches!(c, ':' | '=' | '"' | '\'' | '(' | '[' | '{' | '<')
            })
        {
            continue;
        }
        let s1 = after.trim_start();
        let had_assign_sep = s1.starts_with(':') || s1.starts_with('=');
        let after_assign = if had_assign_sep {
            s1[1..].trim_start()
        } else {
            s1
        };
        let payload = extract_auth_payload_with_memo(
            after_assign,
            Some(&mut overflow_bearer_memo),
            Some(text),
            false,
        );

        if let Some(ov) = payload.enclosure_overflow {
            if !payload.overflow_already_verified {
                let enclosure_body = &after_assign[1..];
                if !is_safe_enclosure_overflow(
                    enclosure_body,
                    ov,
                    true,
                    false,
                    params,
                    Some(&mut overflow_bearer_memo),
                    text,
                ) {
                    return Err(BotBrowserError::SecretLeakDetected(format!(
                        "plaintext Bearer token detected in {context}"
                    )));
                }
            }
            if payload.has_broken_quote
                && (payload.unwrapped.contains("{{") || payload.unwrapped.contains("}}"))
            {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "unprotected Bearer placeholder detected in {context}"
                )));
            }
            continue;
        }

        if !check_and_memoize_same_line_trailing(
            text,
            after_assign,
            &payload,
            enclosing_quote,
            true,
            false,
            params,
            &mut bearer_safe_line,
        ) {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "plaintext Bearer token detected in {context}"
            )));
        }

        if payload.unwrapped.contains("{{") || payload.unwrapped.contains("}}") {
            let unquoted_if_wrapped = enclosing_quote
                .and_then(|q| {
                    advance_bracket_stack(
                        text,
                        &mut bearer_bracket_cursor,
                        abs_idx - 1,
                        &mut bearer_bracket_stack,
                    );
                    strip_closing_outer_punct_and_bracket_with_stack(
                        payload.unwrapped,
                        &bearer_bracket_stack,
                    )
                    .strip_suffix(q)
                })
                .filter(|inner| {
                    is_protected_placeholder(inner, params)
                        || unwrap_balanced_enclosure(inner)
                            .is_some_and(|b| is_protected_placeholder(b, params))
                });
            let unbracketed_if_wrapped = (enclosing_quote.is_none() && !payload.has_broken_quote)
                .then(|| {
                    advance_bracket_stack(
                        text,
                        &mut bearer_bracket_cursor,
                        abs_idx,
                        &mut bearer_bracket_stack,
                    );
                    strip_closing_outer_punct_and_bracket_with_stack(
                        payload.unwrapped,
                        &bearer_bracket_stack,
                    )
                })
                .filter(|inner| {
                    is_protected_placeholder(inner, params)
                        || unwrap_balanced_enclosure(inner)
                            .is_some_and(|b| is_protected_placeholder(b, params))
                });
            let clean_inline_kv = !payload.has_broken_quote
                && payload.followed_by_kv_sep.is_none()
                && payload
                    .unwrapped
                    .split_once([':', '='])
                    .is_some_and(|(lhs, rhs)| {
                        !lhs.is_empty()
                            && !lhs.contains(['{', '}', '"', '\''])
                            && !rhs.is_empty()
                            && is_protected_placeholder(rhs, params)
                    });
            if unquoted_if_wrapped.is_none()
                && unbracketed_if_wrapped.is_none()
                && !clean_inline_kv
                && (payload.has_broken_quote
                    || payload.followed_by_kv_sep.is_some()
                    || !is_protected_placeholder(payload.unwrapped, params))
            {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "unprotected Bearer placeholder detected in {context}"
                )));
            }
            continue;
        }

        if payload.followed_by_kv_sep.is_some() {
            let rhs = extract_auth_payload(payload.rest_after_kv_sep);
            if !check_and_memoize_same_line_trailing(
                text,
                payload.rest_after_kv_sep,
                &rhs,
                None,
                true,
                false,
                params,
                &mut bearer_safe_line,
            ) {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext Bearer token detected in {context}"
                )));
            }
            if rhs.unwrapped.contains("{{") || rhs.unwrapped.contains("}}") {
                if rhs.has_broken_quote
                    || rhs.followed_by_kv_sep.is_some()
                    || !is_protected_placeholder(rhs.unwrapped, params)
                {
                    return Err(BotBrowserError::SecretLeakDetected(format!(
                        "plaintext Bearer token detected in {context}"
                    )));
                }
                continue;
            }
            if rhs.has_broken_quote {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext Bearer token detected in {context}"
                )));
            }
            if !rhs.unwrapped.is_empty() {
                if !is_protected_placeholder(rhs.unwrapped, params) {
                    return Err(BotBrowserError::SecretLeakDetected(format!(
                        "plaintext Bearer token detected in {context}"
                    )));
                }
                continue;
            }
            if is_protected_placeholder(payload.unwrapped, params)
                || is_allowed_bearer_prose_noun(payload.unwrapped)
            {
                continue;
            }
        }

        if let Some((_, rhs)) = payload.unwrapped.split_once([':', '=']) {
            if !rhs.is_empty() && !is_protected_placeholder(rhs, params) {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext Bearer token detected in {context}"
                )));
            }
        }

        let ascii_non_ws = payload
            .unwrapped
            .bytes()
            .filter(|b| b.is_ascii_graphic())
            .count();
        if had_assign_sep
            || payload.followed_by_kv_sep.is_some()
            || payload.has_broken_quote
            || matches!(
                payload.kind,
                AuthPayloadKind::Quoted | AuthPayloadKind::Bracketed
            )
        {
            if ascii_non_ws >= 6
                || (ascii_non_ws >= 4
                    && payload
                        .unwrapped
                        .bytes()
                        .any(|b| b.is_ascii_digit() || b.is_ascii_punctuation()))
            {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext Bearer token detected in {context}"
                )));
            }
        } else if ascii_non_ws >= 12 && !is_allowed_bearer_prose_noun(payload.unwrapped) {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "plaintext Bearer token detected in {context}"
            )));
        }
    }

    // 3. Standalone Basic <base64/token> (with chained KV traversal up to 8 levels)
    let mut search_from = 0usize;
    let mut basic_bracket_stack: Vec<char> = Vec::new();
    let mut basic_bracket_cursor = 0usize;
    while let Some(rel_idx) = lower[search_from..].find("basic") {
        let abs_idx = search_from + rel_idx;
        search_from = abs_idx + 5;
        if in_verified_json(abs_idx) {
            continue;
        }
        if abs_idx > 0 && lower.as_bytes()[abs_idx - 1].is_ascii_alphanumeric() {
            continue;
        }
        let enclosing_quote = (abs_idx > 0
            && matches!(text.as_bytes()[abs_idx - 1], b'"' | b'\''))
        .then(|| text.as_bytes()[abs_idx - 1] as char);
        let after = &text[abs_idx + 5..];
        if !after
            .chars()
            .next()
            .is_some_and(|c| {
                c.is_whitespace() || matches!(c, ':' | '=' | '"' | '\'' | '(' | '[' | '{' | '<')
            })
        {
            continue;
        }
        let s1 = after.trim_start();
        let had_assign_sep = s1.starts_with(':') || s1.starts_with('=');
        let after_assign = if had_assign_sep {
            s1[1..].trim_start()
        } else {
            s1
        };
        let mut cur_base = after_assign;
        let mut cur_payload = extract_auth_payload_with_memo(
            cur_base,
            Some(&mut overflow_basic_memo),
            Some(text),
            true,
        );
        for level in 0..8 {
            if let Some(ov) = cur_payload.enclosure_overflow {
                if !cur_payload.overflow_already_verified {
                    let enclosure_body = &cur_base.trim_start()[1..];
                    if !is_safe_enclosure_overflow(
                        enclosure_body,
                        ov,
                        false,
                        true,
                        params,
                        Some(&mut overflow_basic_memo),
                        text,
                    ) {
                        return Err(BotBrowserError::SecretLeakDetected(format!(
                            "plaintext Basic credential detected in {context}"
                        )));
                    }
                }
                if cur_payload.has_broken_quote
                    && (cur_payload.unwrapped.contains("{{")
                        || cur_payload.unwrapped.contains("}}"))
                {
                    return Err(BotBrowserError::SecretLeakDetected(format!(
                        "unprotected Basic auth placeholder detected in {context}"
                    )));
                }
                break;
            }
            let cur_enclosing = if level == 0 { enclosing_quote } else { None };
            if !check_and_memoize_same_line_trailing(
                text,
                cur_base,
                &cur_payload,
                cur_enclosing,
                false,
                true,
                params,
                &mut basic_safe_line,
            ) {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext Basic credential detected in {context}"
                )));
            }

            if cur_payload.unwrapped.contains("{{") || cur_payload.unwrapped.contains("}}") {
                let unquoted_if_wrapped = (level == 0)
                    .then_some(enclosing_quote)
                    .flatten()
                    .and_then(|q| {
                        advance_bracket_stack(
                            text,
                            &mut basic_bracket_cursor,
                            abs_idx - 1,
                            &mut basic_bracket_stack,
                        );
                        strip_closing_outer_punct_and_bracket_with_stack(
                            cur_payload.unwrapped,
                            &basic_bracket_stack,
                        )
                        .strip_suffix(q)
                    })
                    .filter(|inner| {
                        is_protected_placeholder(inner, params)
                            || unwrap_balanced_enclosure(inner)
                                .is_some_and(|b| is_protected_placeholder(b, params))
                    });
                let unbracketed_if_wrapped =
                    (level == 0 && enclosing_quote.is_none() && !cur_payload.has_broken_quote)
                        .then(|| {
                            advance_bracket_stack(
                                text,
                                &mut basic_bracket_cursor,
                                abs_idx,
                                &mut basic_bracket_stack,
                            );
                            strip_closing_outer_punct_and_bracket_with_stack(
                                cur_payload.unwrapped,
                                &basic_bracket_stack,
                            )
                        })
                        .filter(|inner| {
                            is_protected_placeholder(inner, params)
                                || unwrap_balanced_enclosure(inner)
                                    .is_some_and(|b| is_protected_placeholder(b, params))
                        });
                let clean_inline_kv = !cur_payload.has_broken_quote
                    && cur_payload.followed_by_kv_sep.is_none()
                    && cur_payload
                        .unwrapped
                        .split_once([':', '='])
                        .is_some_and(|(lhs, rhs)| {
                            !lhs.is_empty()
                                && !lhs.contains(['{', '}', '"', '\''])
                                && !rhs.is_empty()
                                && is_protected_placeholder(rhs, params)
                        });
                if unquoted_if_wrapped.is_none()
                    && unbracketed_if_wrapped.is_none()
                    && !clean_inline_kv
                    && (cur_payload.has_broken_quote
                        || cur_payload.followed_by_kv_sep.is_some()
                        || !is_protected_placeholder(cur_payload.unwrapped, params))
                {
                    return Err(BotBrowserError::SecretLeakDetected(format!(
                        "unprotected Basic auth placeholder detected in {context}"
                    )));
                }
                break;
            }

            if contains_basic_credential_token(cur_payload.unwrapped) {
                return Err(BotBrowserError::SecretLeakDetected(format!(
                    "plaintext Basic credential detected in {context}"
                )));
            }

            if cur_payload.followed_by_kv_sep.is_some() {
                cur_base = cur_payload.rest_after_kv_sep;
                cur_payload = extract_auth_payload(cur_base);
            } else if let Some((_, rhs_raw)) = cur_payload.unwrapped.split_once([':', '=']) {
                if rhs_raw.is_empty() {
                    break;
                }
                cur_base = rhs_raw;
                cur_payload = extract_auth_payload(cur_base);
            } else {
                break;
            }
        }
    }

    // 4. Well-known API key prefixes and JWTs
    for token in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '`'
                    | '<'
                    | '>'
                    | ','
                    | ';'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '='
                    | ':'
                    | '?'
                    | '&'
                    | '#'
                    | '/'
            )
    }) {
        if token.starts_with("sk-ant-") && token.len() >= 20 {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "Anthropic API key pattern detected in {context}"
            )));
        }
        if token.starts_with("sk-proj-") && token.len() >= 20 {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "OpenAI project key pattern detected in {context}"
            )));
        }
        if token.starts_with("sk-")
            && token.len() >= 19
            && token[3..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "API key 'sk-...' pattern detected in {context}"
            )));
        }
        if (token.starts_with("ghp_")
            || token.starts_with("gho_")
            || token.starts_with("ghu_")
            || token.starts_with("github_pat_"))
            && token.len() >= 24
        {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "GitHub token pattern detected in {context}"
            )));
        }
        if token.starts_with("AKIA")
            && token.len() == 20
            && token[4..]
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "AWS access key pattern detected in {context}"
            )));
        }
        if token.starts_with("eyJ") && is_jwt_like(token) {
            return Err(BotBrowserError::SecretLeakDetected(format!(
                "plaintext JWT detected in {context}"
            )));
        }
    }

    Ok(())
}

fn is_allowed_bearer_prose_noun(tok: &str) -> bool {
    let word = match tok.strip_suffix('.') {
        Some(w) if !w.contains('.') => w,
        _ => tok,
    };
    if !word.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let mut bytes = word.bytes();
    if bytes.next().is_none() || !bytes.all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    const ALLOWLIST: &[&str] = &[
        "authenticating",
        "authentication",
        "authorization",
        "credential",
        "credentials",
        "certification",
        "implementation",
        "configuration",
        "documentation",
        "verification",
        "specification",
        "integration",
        "management",
        "middleware",
        "protocol",
        "mechanism",
        "framework",
        "architecture",
    ];
    ALLOWLIST.contains(&word.to_ascii_lowercase().as_str())
}

fn is_jwt_like(token: &str) -> bool {
    let mut parts = token.split('.');
    let Some(h) = parts.next() else {
        return false;
    };
    let Some(p) = parts.next() else {
        return false;
    };
    let Some(s) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let is_b64url_10 = |seg: &str| {
        seg.len() >= 10
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    let is_b64url_6 = |seg: &str| {
        seg.len() >= 6
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    is_b64url_10(h) && is_b64url_10(p) && is_b64url_6(s)
}

fn step_kind_name(step: &ScriptStep) -> &'static str {
    match step {
        ScriptStep::Navigate { .. } => "navigate",
        ScriptStep::WaitFor { .. } => "wait_for",
        ScriptStep::Click { .. } => "click",
        ScriptStep::Fill { .. } => "fill",
        ScriptStep::PressKey { .. } => "press_key",
        ScriptStep::Scroll { .. } => "scroll",
        ScriptStep::ExtractText { .. } => "extract_text",
        ScriptStep::AssertVisible { .. } => "assert_visible",
        ScriptStep::AssertTextContains { .. } => "assert_text_contains",
        ScriptStep::HumanCheckpoint { .. } => "human_checkpoint",
    }
}

fn step_label(step: &ScriptStep) -> String {
    match step {
        ScriptStep::Navigate { url_template } => format!("navigate {url_template}"),
        ScriptStep::WaitFor { locator, .. } => format!("wait_for {}", locator.display_label()),
        ScriptStep::Click { locator } => format!("click {}", locator.display_label()),
        ScriptStep::Fill { locator, .. } => format!("fill {}", locator.display_label()),
        ScriptStep::PressKey { key } => format!("press_key {key}"),
        ScriptStep::Scroll { delta_x, delta_y } => format!("scroll ({delta_x}, {delta_y})"),
        ScriptStep::ExtractText {
            locator,
            output_key,
        } => format!("extract_text {} -> {output_key}", locator.display_label()),
        ScriptStep::AssertVisible { locator } => {
            format!("assert_visible {}", locator.display_label())
        }
        ScriptStep::AssertTextContains { locator, .. } => {
            format!("assert_text_contains {}", locator.display_label())
        }
        ScriptStep::HumanCheckpoint { reason } => format!("human_checkpoint: {reason}"),
    }
}
