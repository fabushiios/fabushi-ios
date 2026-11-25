pub mod acp;
pub mod computer;
pub mod fabushi_bot;
pub mod hermes;
pub mod mcp;
pub mod multi_agent;
pub mod talk;
pub mod tasks;
pub mod wallet;

/// Canonical unified Fabushi Agent facade combining the multi-bot host (`fabushi_bot`)
/// and the embedded pure-Rust agent execution runtime (`hermes`).
pub mod fabushi {
    pub use crate::fabushi_bot::*;
    pub use crate::hermes::state_db::HermesStateStore as FabushiStateStore;
    pub use crate::hermes::toolsets::{
        HermesToolContext as FabushiToolContext, HermesToolExecutor as FabushiToolExecutor,
    };
    pub use crate::hermes::browser_session::{
        forward_events_to_mpsc, BrowserAutomationEvent, BrowserAutomationSession,
        BrowserSessionConfig, BrowserSessionRegistry, StepGateInfo, TakeoverOutcome,
    };
    pub use crate::hermes::commands::{CommandDistilledScriptResult, CommandUiEffect};
    pub use crate::hermes::{
        CronRunOutcome, HeartbeatConfig, HeartbeatManager, HeartbeatTickOutcome,
        HermesEngine as FabushiAgentEngine, HermesEngineConfig as FabushiEngineConfig,
        HermesEvent as FabushiEvent, HermesTurnRequest as FabushiTurnRequest,
        HermesTurnResult as FabushiTurnResult, MessageQueueManager, QueuedMessage,
        RecordedToolStep, SessionRuntimeSettings,
    };
    pub use bot_browser::{BrowserControlGate, BrowserControlMode};
    pub type FabushiAgentHost = crate::fabushi_bot::FabushiBotHost;
}

pub use fabushi::{
    FabushiAgentEngine, FabushiAgentHost, FabushiEngineConfig, FabushiEvent, FabushiStateStore,
    FabushiToolContext, FabushiToolExecutor, FabushiTurnRequest, FabushiTurnResult,
};

pub use multi_agent::{
    evaluate_group_wake_gate, evaluate_outbound_turn_review, is_explicit_pass_sentinel,
    FloorControlQueue, FloorPriority, FloorTurnGrant, GroupWakeBotCandidate, GroupWakeBotScore,
    GroupWakeGateDecision, GroupWakeGateInput, GroupWakeReason, GroupWorkspaceBridge,
    InMemoryGroupWorkspaceBridge, LoopBudget, LoopBudgetExceededError, OutboundReviewReason,
    OutboundTurnReviewInput, OutboundTurnVerdict, RoomMemberTurnOutcome, ScopedCapabilityToken,
    SharedGroupWorkspace, SharedGroupWorkspaceAdapter, SharedWorkspaceEntry, TurnDeliveryIntake,
    DEFAULT_MAX_LOOP_HOPS,
};

pub use fabushi_bot::{
    build_group_turn_guidance, is_room_session_key, parse_room_session_chat_id, room_session_key,
    GroupTurnPromptContext, ROOM_SESSION_KEY_PREFIX,
};

pub use acp::{
    AcpRunner, AgentTurnInput, AgentTurnOutput, HostSubagentLineage, SubagentArtifactReceipt,
    SubagentLifecycleStatus, SubagentLineage, SubagentNode, SubagentTreeNode, TextDeltaCallback,
};
pub use computer::{
    parse_cloud_credentials_from_prompt, redact_cloud_secrets, AgentCloudServerConfigInput,
    AgentComputerExecInput, AgentComputerExecOutput, AgentComputerNodeSummary,
    ComputerCloudProvider, ComputerControlLease, ComputerLeaseBroker, CuaAction, CuaExecutor,
    LeaseState, CUA_BACKEND_UNAVAILABLE_ERR,
};
pub use hermes::browser_session::{
    forward_events_to_mpsc, BrowserAutomationEvent, BrowserAutomationSession, BrowserSessionConfig,
    BrowserSessionRegistry, StepGateInfo, TakeoverOutcome,
};
pub use hermes::commands::{
    CommandDescriptor, CommandDistilledScriptResult, CommandExecutionOutcome, CommandUiEffect,
    InlineActionButton, SlashCommandParser, SlashCommandRegistry,
};
pub use hermes::context::{
    AssembledPrompt, CompressionConfig, ContextCompressor, ContextRefResolver, PromptBuilder,
    WorkspaceContextDiscovery,
};
pub use hermes::memory::{
    HonchoDialectics, MemoryManager, MemoryQuarantineQueue, MemoryRollupEntry, MemoryRollupHorizon,
    MemoryTarget, QuarantineStatus, QuarantinedMemoryProposal, SoulProfile,
};
pub use hermes::providers::{ApiMode, MagpiePresetApplyInput, MoaCoordinator, ProviderConfig, ProviderRouter};
pub use hermes::security::{
    compute_content_sha256, ApprovalConsumeError, ApprovalDecisionAction, ApprovalMode,
    AutoReviewPolicy, AutoReviewVerdict, SecurityEngine, ToolApprovalTier,
    ValueBoundApprovalLedger, ValueBoundApprovalToken, DEFAULT_VALUE_BOUND_APPROVAL_TTL_MS,
};
pub use hermes::skills::SkillsManager;
pub use hermes::state_db::HermesStateStore;
pub use hermes::tools_agentic::{
    DesktopPanelState, IntegrationConnection, KanbanBoard, KanbanCard, LocalAgenticStore,
    LocalCronJob, LocalSkillEntry,
};
pub use hermes::toolsets::{
    AgenticStateBackend, HermesToolContext, HermesToolExecutor, PlatformImBridge,
    PlatformImCaller, ToolsetId, ToolsetPreset,
};
pub use hermes::trajectory::{SecretRedactor, TrajectoryExporter, TrajectoryFormat};
pub use hermes::{
    CronRunOutcome, HeartbeatConfig, HeartbeatManager, HeartbeatTickOutcome, HermesEngine,
    HermesEngineConfig, HermesEvent, HermesTurnRequest, HermesTurnResult, MessageQueueManager,
    QueuedMessage, RecordedToolStep, SessionRuntimeSettings,
};
pub use mcp::{
    McpRegistry, McpToolDefinition, McpToolHandler, ToolPermissionDock, ToolPermissionRequest,
    ToolRiskLevel,
};
pub use talk::{TalkCoordinator, TalkSessionState, VadBargeInDetector, MAX_AUDIO_BUFFER_CHUNKS};
pub use tasks::{
    compile_cron_matcher, compute_next_run_at, parse_every_interval_ms,
    summarize_schedule_next_7_days, ApprovalDecision, ApprovalGate, AutomationScheduleSummary,
    CronExecutionRecord, CronMatcher, CronRoutine, TaskFsm, TaskState,
    SAND_AUTO_REVIEW_APPROVAL_TTL_MS,
};
