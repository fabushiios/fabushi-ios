//! Automation Script App Store wire contracts.
//!
//! Defines HTTP request/response/query DTOs for publishing, searching,
//! fetching, installing, uninstalling, locally caching, and recording runs of
//! `bot_browser::script_app::AutomationScriptApp` manifests.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const AUTOMATION_APP_SCHEMA_VERSION_V1: u32 = 1;

fn default_true() -> bool {
    true
}

/// Wire-compatible summary mirror of `bot_browser::script_app::ScriptParameter`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationAppParameterDto {
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

/// Request payload for `POST /api/v1/automation-apps`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PublishAutomationAppRequest {
    /// Serialized `bot_browser::script_app::AutomationScriptApp` JSON manifest.
    pub manifest: serde_json::Value,
    /// Whether the app is listed in the public store catalog (defaults to `true`).
    #[serde(default = "default_true")]
    pub is_published: bool,
    /// Optional release notes for this published version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
}

impl Default for PublishAutomationAppRequest {
    fn default() -> Self {
        Self {
            manifest: serde_json::Value::Null,
            is_published: true,
            changelog: None,
        }
    }
}

/// Catalog summary item returned in `GET /api/v1/automation-apps`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationAppSummary {
    pub app_id: String,
    pub slug: String,
    pub author_user_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_username: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub target_domains: Vec<String>,
    pub latest_version: String,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub step_count: usize,
    #[serde(default)]
    pub parameters: Vec<AutomationAppParameterDto>,
    #[serde(default)]
    pub install_count: u64,
    #[serde(default)]
    pub run_count: u64,
    #[serde(default = "default_true")]
    pub is_published: bool,
    #[serde(default)]
    pub is_installed: bool,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

/// Full automation script app record returned by `POST /api/v1/automation-apps`
/// and `GET /api/v1/automation-apps/{app_id}`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationAppDetail {
    pub app_id: String,
    pub slug: String,
    pub author_user_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_username: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub target_domains: Vec<String>,
    pub latest_version: String,
    /// Available published versions (newest first).
    #[serde(default)]
    pub versions: Vec<String>,
    /// Full `bot_browser::script_app::AutomationScriptApp` JSON object.
    pub manifest: serde_json::Value,
    #[serde(default)]
    pub content_sha256: String,
    #[serde(default)]
    pub install_count: u64,
    #[serde(default)]
    pub run_count: u64,
    #[serde(default = "default_true")]
    pub is_published: bool,
    #[serde(default)]
    pub is_installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

/// Query parameters for `GET /api/v1/automation-apps`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ListAutomationAppsQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

/// Response payload for `GET /api/v1/automation-apps`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ListAutomationAppsResponse {
    #[serde(default)]
    pub apps: Vec<AutomationAppSummary>,
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

/// User-installed automation script app entry (server `GET /api/v1/automation-apps/installed`
/// and client `LocalStore` offline cache).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InstalledAutomationApp {
    pub app_id: String,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub target_domains: Vec<String>,
    pub installed_version: String,
    #[serde(default)]
    pub latest_version: String,
    /// Full `bot_browser::script_app::AutomationScriptApp` JSON object for deterministic replay.
    pub manifest: serde_json::Value,
    #[serde(default)]
    pub content_sha256: String,
    #[serde(default)]
    pub installed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at_ms: Option<i64>,
    #[serde(default)]
    pub local_run_count: u64,
}

/// Response payload for `GET /api/v1/automation-apps/installed`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ListInstalledAutomationAppsResponse {
    #[serde(default)]
    pub apps: Vec<InstalledAutomationApp>,
    #[serde(default)]
    pub total: usize,
}

/// Response payload for `POST /api/v1/automation-apps/{app_id}/install`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallAutomationAppResponse {
    #[serde(default = "default_true")]
    pub ok: bool,
    pub app: InstalledAutomationApp,
    #[serde(default)]
    pub install_count: u64,
    #[serde(default)]
    pub newly_installed: bool,
}

/// Response payload for `DELETE /api/v1/automation-apps/{app_id}/install`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UninstallAutomationAppResponse {
    pub ok: bool,
    pub app_id: String,
    #[serde(default)]
    pub removed: bool,
}

/// Request payload for `POST /api/v1/automation-apps/{app_id}/runs`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordAutomationRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub success: bool,
    #[serde(default)]
    pub steps_executed: usize,
    #[serde(default)]
    pub total_duration_ms: u64,
    #[serde(default)]
    pub used_llm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Response payload for `POST /api/v1/automation-apps/{app_id}/runs`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordAutomationRunResponse {
    #[serde(default = "default_true")]
    pub ok: bool,
    pub app_id: String,
    #[serde(default)]
    pub run_count: u64,
    #[serde(default)]
    pub last_run_at_ms: i64,
}

/// Computes lowercase hex SHA-256 of canonical manifest JSON bytes.
pub fn compute_automation_manifest_sha256(manifest_json: &str) -> String {
    let digest = Sha256::digest(manifest_json.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}
