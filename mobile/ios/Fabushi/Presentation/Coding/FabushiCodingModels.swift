import Foundation

// MARK: - Coding Provider Kinds, Permission Modes & Approval Decisions (`coding_agent_contracts.rs`)

public enum FabushiCodingProviderKind: String, CaseIterable, Codable, Sendable, Equatable, Identifiable {
    case claudeCode = "claude_code"
    case codex = "codex"
    case openCode = "open_code"
    case cursorCli = "cursor_cli"
    case antigravity = "antigravity"
    case acpCustom = "acp_custom"

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .claudeCode: return "Claude Code"
        case .codex: return "Codex"
        case .openCode: return "OpenCode"
        case .cursorCli: return "Cursor CLI"
        case .antigravity: return "Antigravity"
        case .acpCustom: return "Custom ACP"
        }
    }

    public var recommendedUsername: String {
        switch self {
        case .claudeCode: return "claude_code_bot"
        case .codex: return "codex_bot"
        case .openCode: return "opencode_bot"
        case .cursorCli: return "cursor_cli_bot"
        case .antigravity: return "antigravity_bot"
        case .acpCustom: return "acp_coding_bot"
        }
    }

    public var defaultBinary: String? {
        switch self {
        case .claudeCode: return "claude"
        case .codex: return "codex"
        case .openCode: return "opencode"
        case .cursorCli: return "cursor-agent"
        case .antigravity: return "agy"
        case .acpCustom: return nil
        }
    }

    public var supportsAcp: Bool {
        switch self {
        case .openCode, .cursorCli, .acpCustom: return true
        case .claudeCode, .codex, .antigravity: return false
        }
    }

    public static func parse(_ raw: String) -> FabushiCodingProviderKind? {
        switch raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "claude_code", "claudecode", "claude-code", "claude":
            return .claudeCode
        case "codex", "openai_codex", "openai-codex":
            return .codex
        case "open_code", "opencode", "open-code":
            return .openCode
        case "cursor_cli", "cursorcli", "cursor-cli", "cursor", "cursor-agent", "cursor_agent":
            return .cursorCli
        case "antigravity", "agy":
            return .antigravity
        case "acp_custom", "acpcustom", "acp-custom", "acp", "custom_acp":
            return .acpCustom
        default:
            return nil
        }
    }
}

public enum FabushiCodingPermissionMode: String, CaseIterable, Codable, Sendable, Equatable, Identifiable {
    case ask = "ask"
    case autoEdit = "auto_edit"
    case fullAccess = "full_access"

    public var id: String { rawValue }

    public var localizationKey: String {
        switch self {
        case .ask: return "m4.coding.permissionAsk"
        case .autoEdit: return "m4.coding.permissionAutoEdit"
        case .fullAccess: return "m4.coding.permissionFullAccess"
        }
    }
}

public enum FabushiCodingApprovalDecision: String, CaseIterable, Codable, Sendable, Equatable {
    case allowOnce = "allow_once"
    case allowSession = "allow_session"
    case deny = "deny"
}

// MARK: - Provider Catalog (`coding.listProvidersCatalog`, `coding.parseProvider`)

public struct FabushiCodingProviderCatalogItemDTO: Codable, Sendable, Equatable, Identifiable {
    public let kind: FabushiCodingProviderKind
    public let supportsAcp: Bool

    public var id: String { kind.rawValue }

    public enum CodingKeys: String, CodingKey {
        case kind
        case supportsAcp = "supports_acp"
    }
}

public struct FabushiCodingProvidersCatalogResponse: Codable, Sendable, Equatable {
    public let providers: [FabushiCodingProviderCatalogItemDTO]
}

public struct FabushiCodingParseProviderRequest: Encodable, Sendable, Equatable {
    public let provider: String
}

// MARK: - Coding Agent Bot Config & CRUD (`coding.listBots`, `createBot`, `patchBot`, `fabushi.botCreateNl`)

public struct FabushiCodingAgentBotConfigDTO: Codable, Sendable, Equatable {
    public let providerKind: FabushiCodingProviderKind
    public let workspacePath: String
    public let useWorktree: Bool
    public let permissionMode: FabushiCodingPermissionMode
    public let executableOverride: String?
    public let extraArgs: [String]
    public let boundDeviceId: String?
    public let boundNodeId: String?
    public let modelOverride: String?
    public let allowGroupMembers: Bool

    public enum CodingKeys: String, CodingKey {
        case providerKind = "provider_kind"
        case workspacePath = "workspace_path"
        case useWorktree = "use_worktree"
        case permissionMode = "permission_mode"
        case executableOverride = "executable_override"
        case extraArgs = "extra_args"
        case boundDeviceId = "bound_device_id"
        case boundNodeId = "bound_node_id"
        case modelOverride = "model_override"
        case allowGroupMembers = "allow_group_members"
    }

    public init(
        providerKind: FabushiCodingProviderKind = .claudeCode,
        workspacePath: String = "",
        useWorktree: Bool = true,
        permissionMode: FabushiCodingPermissionMode = .ask,
        executableOverride: String? = nil,
        extraArgs: [String] = [],
        boundDeviceId: String? = nil,
        boundNodeId: String? = nil,
        modelOverride: String? = nil,
        allowGroupMembers: Bool = false
    ) {
        self.providerKind = providerKind
        self.workspacePath = workspacePath
        self.useWorktree = useWorktree
        self.permissionMode = permissionMode
        self.executableOverride = executableOverride
        self.extraArgs = extraArgs
        self.boundDeviceId = boundDeviceId
        self.boundNodeId = boundNodeId
        self.modelOverride = modelOverride
        self.allowGroupMembers = allowGroupMembers
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.providerKind = (try? c.decode(FabushiCodingProviderKind.self, forKey: .providerKind)) ?? .claudeCode
        self.workspacePath = (try? c.decodeIfPresent(String.self, forKey: .workspacePath)) ?? ""
        self.useWorktree = (try? c.decodeIfPresent(Bool.self, forKey: .useWorktree)) ?? false
        self.permissionMode = (try? c.decode(FabushiCodingPermissionMode.self, forKey: .permissionMode)) ?? .ask
        self.executableOverride = try? c.decodeIfPresent(String.self, forKey: .executableOverride)
        self.extraArgs = (try? c.decodeIfPresent([String].self, forKey: .extraArgs)) ?? []
        self.boundDeviceId = try? c.decodeIfPresent(String.self, forKey: .boundDeviceId)
        self.boundNodeId = try? c.decodeIfPresent(String.self, forKey: .boundNodeId)
        self.modelOverride = try? c.decodeIfPresent(String.self, forKey: .modelOverride)
        self.allowGroupMembers = (try? c.decodeIfPresent(Bool.self, forKey: .allowGroupMembers)) ?? false
    }
}

public struct FabushiCodingBotSummaryDTO: Codable, Sendable, Equatable, Identifiable {
    public let botUserId: Int64
    public let ownerUserId: Int64
    public let agentId: String
    public let username: String
    public let displayName: String
    public let description: String
    public let title: String
    public let avatarShape: String
    public let avatarColor: String
    public let preferredModel: String
    public let thinkingLevel: String
    public let approvalMode: String
    public let toolsetPreset: String
    public let executionState: String
    public let companionChatId: Int64?
    public let codingAgent: FabushiCodingAgentBotConfigDTO?

    public var id: Int64 { botUserId }

    /// Normalized `D10-09` preferred model (`""` == Follow Settings).
    public var normalizedPreferredModel: String {
        FabushiMagpieBotModelPolicy.normalizePreferredModel(preferredModel)
    }

    public enum CodingKeys: String, CodingKey {
        case botUserId = "bot_user_id"
        case ownerUserId = "owner_user_id"
        case agentId = "agent_id"
        case username
        case displayName = "display_name"
        case description
        case title
        case avatarShape = "avatar_shape"
        case avatarColor = "avatar_color"
        case preferredModel = "preferred_model"
        case thinkingLevel = "thinking_level"
        case approvalMode = "approval_mode"
        case toolsetPreset = "toolset_preset"
        case executionState = "execution_state"
        case companionChatId = "companion_chat_id"
        case codingAgent = "coding_agent"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.botUserId = try c.decode(Int64.self, forKey: .botUserId)
        self.ownerUserId = (try? c.decodeIfPresent(Int64.self, forKey: .ownerUserId)) ?? 0
        self.agentId = (try? c.decodeIfPresent(String.self, forKey: .agentId)) ?? ""
        self.username = (try? c.decodeIfPresent(String.self, forKey: .username)) ?? ""
        self.displayName = (try? c.decodeIfPresent(String.self, forKey: .displayName)) ?? ""
        self.description = (try? c.decodeIfPresent(String.self, forKey: .description)) ?? ""
        self.title = (try? c.decodeIfPresent(String.self, forKey: .title)) ?? ""
        self.avatarShape = (try? c.decodeIfPresent(String.self, forKey: .avatarShape)) ?? ""
        self.avatarColor = (try? c.decodeIfPresent(String.self, forKey: .avatarColor)) ?? ""
        self.preferredModel = (try? c.decodeIfPresent(String.self, forKey: .preferredModel)) ?? ""
        self.thinkingLevel = (try? c.decodeIfPresent(String.self, forKey: .thinkingLevel)) ?? ""
        self.approvalMode = (try? c.decodeIfPresent(String.self, forKey: .approvalMode)) ?? ""
        self.toolsetPreset = (try? c.decodeIfPresent(String.self, forKey: .toolsetPreset)) ?? ""
        self.executionState = (try? c.decodeIfPresent(String.self, forKey: .executionState)) ?? "idle"
        self.companionChatId = try? c.decodeIfPresent(Int64.self, forKey: .companionChatId)
        self.codingAgent = try? c.decodeIfPresent(FabushiCodingAgentBotConfigDTO.self, forKey: .codingAgent)
    }
}

public struct FabushiCodingBotRegistryEntryDTO: Codable, Sendable, Equatable {
    public let botUserId: Int64
    public let ownerUserId: Int64
    public let username: String
    public let displayName: String
    public let description: String
    public let codingAgent: FabushiCodingAgentBotConfigDTO?

    public enum CodingKeys: String, CodingKey {
        case botUserId = "bot_user_id"
        case ownerUserId = "owner_user_id"
        case username
        case displayName = "display_name"
        case description
        case codingAgent = "coding_agent"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.botUserId = try c.decode(Int64.self, forKey: .botUserId)
        self.ownerUserId = (try? c.decodeIfPresent(Int64.self, forKey: .ownerUserId)) ?? 0
        self.username = (try? c.decodeIfPresent(String.self, forKey: .username)) ?? ""
        self.displayName = (try? c.decodeIfPresent(String.self, forKey: .displayName)) ?? ""
        self.description = (try? c.decodeIfPresent(String.self, forKey: .description)) ?? ""
        self.codingAgent = try? c.decodeIfPresent(FabushiCodingAgentBotConfigDTO.self, forKey: .codingAgent)
    }
}

public struct FabushiCodingCreateBotRequest: Encodable, Sendable, Equatable {
    public let displayName: String
    public let username: String?
    public let description: String?
    public let preferredModel: String?
    public let codingAgent: FabushiCodingAgentBotConfigDTO

    public enum CodingKeys: String, CodingKey {
        case displayName = "display_name"
        case username
        case description
        case preferredModel = "preferred_model"
        case codingAgent = "coding_agent"
    }

    public init(
        displayName: String,
        username: String? = nil,
        description: String? = nil,
        preferredModel: String? = nil,
        codingAgent: FabushiCodingAgentBotConfigDTO
    ) {
        self.displayName = displayName
        self.username = username
        self.description = description
        self.preferredModel = preferredModel
        self.codingAgent = codingAgent
    }
}

public struct FabushiCodingCreateBotResponse: Codable, Sendable, Equatable {
    public let bot: FabushiCodingBotSummaryDTO
    public let registry: FabushiCodingBotRegistryEntryDTO
    public let botToken: String
    public let companionChatId: Int64

    public enum CodingKeys: String, CodingKey {
        case bot
        case registry
        case botToken = "bot_token"
        case companionChatId = "companion_chat_id"
    }
}

public struct FabushiCodingPatchBotRequest: Encodable, Sendable, Equatable {
    public let botId: UInt64
    public let preferredModel: String?
    public let codingAgent: FabushiCodingAgentBotConfigDTO?

    public enum CodingKeys: String, CodingKey {
        case botId = "bot_id"
        case preferredModel = "preferred_model"
        case codingAgent = "coding_agent"
    }

    public init(
        botId: UInt64,
        preferredModel: String? = nil,
        codingAgent: FabushiCodingAgentBotConfigDTO? = nil
    ) {
        self.botId = botId
        self.preferredModel = preferredModel
        self.codingAgent = codingAgent
    }
}

public struct FabushiCodingBotFatherNlRequest: Encodable, Sendable, Equatable {
    public let prompt: String
    public let avatarShape: String?
    public let avatarColor: String?

    public enum CodingKeys: String, CodingKey {
        case prompt
        case avatarShape = "avatar_shape"
        case avatarColor = "avatar_color"
    }

    public init(prompt: String, avatarShape: String? = nil, avatarColor: String? = nil) {
        self.prompt = prompt
        self.avatarShape = avatarShape
        self.avatarColor = avatarColor
    }
}

public struct FabushiCodingBotFatherNlResponse: Codable, Sendable, Equatable {
    public let bot: FabushiCodingBotRegistryEntryDTO
    public let botToken: String
    public let companionChatId: Int64
    public let summaryMarkdown: String

    public enum CodingKeys: String, CodingKey {
        case bot
        case botToken = "bot_token"
        case companionChatId = "companion_chat_id"
        case summaryMarkdown = "summary_markdown"
    }
}

// MARK: - Threads, Checkpoints & Diffs (`D10-02` `owner_user_id`-Isolated `CodingStore`)

public struct FabushiCodingDiffStatDTO: Codable, Sendable, Equatable {
    public let filesChanged: Int
    public let insertions: Int
    public let deletions: Int
    public let changedFiles: [String]

    public enum CodingKeys: String, CodingKey {
        case filesChanged = "files_changed"
        case insertions
        case deletions
        case changedFiles = "changed_files"
    }

    public init(
        filesChanged: Int = 0,
        insertions: Int = 0,
        deletions: Int = 0,
        changedFiles: [String] = []
    ) {
        self.filesChanged = filesChanged
        self.insertions = insertions
        self.deletions = deletions
        self.changedFiles = changedFiles
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.filesChanged = (try? c.decodeIfPresent(Int.self, forKey: .filesChanged)) ?? 0
        self.insertions = (try? c.decodeIfPresent(Int.self, forKey: .insertions)) ?? 0
        self.deletions = (try? c.decodeIfPresent(Int.self, forKey: .deletions)) ?? 0
        self.changedFiles = (try? c.decodeIfPresent([String].self, forKey: .changedFiles)) ?? []
    }
}

public struct FabushiCodingTokenUsageDTO: Codable, Sendable, Equatable {
    public let inputTokens: UInt64
    public let outputTokens: UInt64
    public let cachedTokens: UInt64
    public let costUsd: Double?

    public enum CodingKeys: String, CodingKey {
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cachedTokens = "cached_tokens"
        case costUsd = "cost_usd"
    }
}

public struct FabushiCodingThreadUpsertRequest: Encodable, Sendable, Equatable {
    public let botUserId: Int64
    public let chatId: Int64
    public let threadId: String
    public let topicId: Int64
    public let providerSessionId: String?
    public let workspacePath: String
    public let worktreePath: String?
    public let branchName: String?
    public let useWorktree: Bool
    public let activeModel: String?
    public let permissionMode: FabushiCodingPermissionMode
    public let turnCount: UInt64
    public let lastSummary: String?
    public let botProviderKind: FabushiCodingProviderKind?

    public enum CodingKeys: String, CodingKey {
        case botUserId = "bot_user_id"
        case chatId = "chat_id"
        case threadId = "thread_id"
        case topicId = "topic_id"
        case providerSessionId = "provider_session_id"
        case workspacePath = "workspace_path"
        case worktreePath = "worktree_path"
        case branchName = "branch_name"
        case useWorktree = "use_worktree"
        case activeModel = "active_model"
        case permissionMode = "permission_mode"
        case turnCount = "turn_count"
        case lastSummary = "last_summary"
        case botProviderKind = "bot_provider_kind"
    }

    public init(
        botUserId: Int64,
        chatId: Int64,
        threadId: String,
        topicId: Int64 = 0,
        providerSessionId: String? = nil,
        workspacePath: String,
        worktreePath: String? = nil,
        branchName: String? = nil,
        useWorktree: Bool = true,
        activeModel: String? = nil,
        permissionMode: FabushiCodingPermissionMode = .ask,
        turnCount: UInt64 = 0,
        lastSummary: String? = nil,
        botProviderKind: FabushiCodingProviderKind? = nil
    ) {
        self.botUserId = botUserId
        self.chatId = chatId
        self.threadId = threadId
        self.topicId = topicId
        self.providerSessionId = providerSessionId
        self.workspacePath = workspacePath
        self.worktreePath = worktreePath
        self.branchName = branchName
        self.useWorktree = useWorktree
        self.activeModel = activeModel
        self.permissionMode = permissionMode
        self.turnCount = turnCount
        self.lastSummary = lastSummary
        self.botProviderKind = botProviderKind
    }
}

public struct FabushiCodingThreadUpsertResponse: Codable, Sendable, Equatable {
    public let ownerUserId: UInt64
    public let threadId: String
    public let botUserId: Int64
    public let chatId: Int64
    public let topicId: Int64
    public let turnCount: UInt64
    public let updatedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case ownerUserId = "owner_user_id"
        case threadId = "thread_id"
        case botUserId = "bot_user_id"
        case chatId = "chat_id"
        case topicId = "topic_id"
        case turnCount = "turn_count"
        case updatedAtMs = "updated_at_ms"
    }
}

public struct FabushiCodingGetThreadRequest: Encodable, Sendable, Equatable {
    public let botUserId: Int64
    public let chatId: Int64
    public let topicId: Int64?

    public enum CodingKeys: String, CodingKey {
        case botUserId = "bot_user_id"
        case chatId = "chat_id"
        case topicId = "topic_id"
    }

    public init(botUserId: Int64, chatId: Int64, topicId: Int64? = nil) {
        self.botUserId = botUserId
        self.chatId = chatId
        self.topicId = topicId
    }
}

public struct FabushiCodingThreadDTO: Codable, Sendable, Equatable, Identifiable {
    public let ownerUserId: UInt64
    public let botUserId: Int64
    public let chatId: Int64
    public let topicId: Int64
    public let threadId: String
    public let providerSessionId: String?
    public let workspacePath: String
    public let worktreePath: String?
    public let branchName: String?
    public let useWorktree: Bool
    public let activeModel: String?
    public let permissionMode: FabushiCodingPermissionMode
    public let turnCount: UInt64
    public let lastSummary: String?
    public let updatedAtMs: Int64

    public var id: String { "\(ownerUserId):\(threadId)" }

    public enum CodingKeys: String, CodingKey {
        case ownerUserId = "owner_user_id"
        case botUserId = "bot_user_id"
        case chatId = "chat_id"
        case topicId = "topic_id"
        case threadId = "thread_id"
        case providerSessionId = "provider_session_id"
        case workspacePath = "workspace_path"
        case worktreePath = "worktree_path"
        case branchName = "branch_name"
        case useWorktree = "use_worktree"
        case activeModel = "active_model"
        case permissionMode = "permission_mode"
        case turnCount = "turn_count"
        case lastSummary = "last_summary"
        case updatedAtMs = "updated_at_ms"
    }
}

public struct FabushiCodingSaveCheckpointRequest: Encodable, Sendable, Equatable {
    public let threadId: String
    public let turnIdx: UInt64
    public let beforeRef: String
    public let afterRef: String
    public let diffStat: FabushiCodingDiffStatDTO
    public let promptPreview: String
    public let replyPreview: String

    public enum CodingKeys: String, CodingKey {
        case threadId = "thread_id"
        case turnIdx = "turn_idx"
        case beforeRef = "before_ref"
        case afterRef = "after_ref"
        case diffStat = "diff_stat"
        case promptPreview = "prompt_preview"
        case replyPreview = "reply_preview"
    }

    public init(
        threadId: String,
        turnIdx: UInt64,
        beforeRef: String,
        afterRef: String,
        diffStat: FabushiCodingDiffStatDTO = FabushiCodingDiffStatDTO(),
        promptPreview: String = "",
        replyPreview: String = ""
    ) {
        self.threadId = threadId
        self.turnIdx = turnIdx
        self.beforeRef = beforeRef
        self.afterRef = afterRef
        self.diffStat = diffStat
        self.promptPreview = promptPreview
        self.replyPreview = replyPreview
    }
}

public struct FabushiCodingSaveCheckpointResponse: Codable, Sendable, Equatable {
    public let ownerUserId: UInt64
    public let threadId: String
    public let turnIdx: UInt64
    public let beforeRef: String
    public let afterRef: String
    public let createdAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case ownerUserId = "owner_user_id"
        case threadId = "thread_id"
        case turnIdx = "turn_idx"
        case beforeRef = "before_ref"
        case afterRef = "after_ref"
        case createdAtMs = "created_at_ms"
    }
}

public struct FabushiCodingGetCheckpointsRequest: Encodable, Sendable, Equatable {
    public let threadId: String
    public let limit: UInt64

    public enum CodingKeys: String, CodingKey {
        case threadId = "thread_id"
        case limit
    }

    public init(threadId: String, limit: UInt64 = 20) {
        self.threadId = threadId
        self.limit = limit
    }
}

public struct FabushiCodingCheckpointDTO: Codable, Sendable, Equatable, Identifiable {
    public let ownerUserId: UInt64
    public let threadId: String
    public let turnIdx: UInt64
    public let beforeRef: String
    public let afterRef: String
    public let diffStat: FabushiCodingDiffStatDTO
    public let promptPreview: String
    public let replyPreview: String
    public let createdAtMs: Int64

    public var id: String { "\(ownerUserId):\(threadId):\(turnIdx)" }

    public enum CodingKeys: String, CodingKey {
        case ownerUserId = "owner_user_id"
        case threadId = "thread_id"
        case turnIdx = "turn_idx"
        case beforeRef = "before_ref"
        case afterRef = "after_ref"
        case diffStat = "diff_stat"
        case promptPreview = "prompt_preview"
        case replyPreview = "reply_preview"
        case createdAtMs = "created_at_ms"
    }
}

// MARK: - 12 Coding Control Actions & Tagged Host Command / Event Envelopes

public enum FabushiCodingControlAction: String, CaseIterable, Sendable, Equatable {
    case setWorkspace = "set_workspace"
    case worktreeOp = "worktree_op"
    case getDiff = "get_diff"
    case rollbackCheckpoint = "rollback_checkpoint"
    case listModels = "list_models"
    case setModel = "set_model"
    case setPermissionMode = "set_permission_mode"
    case commitAndPr = "commit_and_pr"
    case getStatus = "get_status"
    case checkLogin = "check_login"
    case resetSession = "reset_session"
    case buildHandoffSummary = "build_handoff_summary"
}

public struct FabushiCodingHostCommandEnvelope: Sendable, Equatable {
    public let cmd: String
    public let ownerUserId: UInt64?
    public let targetDeviceId: String?
    public let turnId: String?
    public let rawPayload: FabushiJSONValue

    public init?(payload: FabushiJSONValue) {
        guard let cmd = payload["cmd"]?.stringValue, !cmd.isEmpty else {
            return nil
        }
        self.cmd = cmd
        self.ownerUserId = payload["owner_user_id"]?.uint64Value
        self.targetDeviceId = payload["target_device_id"]?.stringValue
        self.turnId = payload["turn_id"]?.stringValue
        self.rawPayload = payload
    }
}

public struct FabushiCodingHostEventSendResponse: Codable, Sendable, Equatable {
    public let sent: Bool
}

public struct FabushiCodingLiveApprovalPrompt: Identifiable, Sendable, Equatable {
    public let requestId: String
    public let turnId: String
    public let kind: String
    public let title: String
    public let detail: String
    public let ruleKey: String

    public var id: String { requestId }

    public init(
        requestId: String,
        turnId: String,
        kind: String,
        title: String,
        detail: String,
        ruleKey: String
    ) {
        self.requestId = requestId
        self.turnId = turnId
        self.kind = kind
        self.title = title
        self.detail = detail
        self.ruleKey = ruleKey
    }
}

// MARK: - Remote Coding Hosts (`computers.list`)

public struct FabushiCodingRemoteComputerDTO: Codable, Sendable, Equatable, Identifiable {
    public let nodeId: String
    public let ownerUserId: UInt64
    public let displayName: String
    public let ipAddress: String
    public let host: String?
    public let osName: String?
    public let online: Bool

    public var id: String { nodeId }

    public enum CodingKeys: String, CodingKey {
        case nodeId = "node_id"
        case ownerUserId = "owner_user_id"
        case displayName = "display_name"
        case ipAddress = "ip_address"
        case host
        case osName = "os_name"
        case online
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.nodeId = try c.decode(String.self, forKey: .nodeId)
        self.ownerUserId = (try? c.decodeIfPresent(UInt64.self, forKey: .ownerUserId)) ?? 0
        self.displayName = (try? c.decodeIfPresent(String.self, forKey: .displayName)) ?? nodeId
        self.ipAddress = (try? c.decodeIfPresent(String.self, forKey: .ipAddress)) ?? ""
        self.host = try? c.decodeIfPresent(String.self, forKey: .host)
        self.osName = try? c.decodeIfPresent(String.self, forKey: .osName)
        self.online = (try? c.decodeIfPresent(Bool.self, forKey: .online)) ?? true
    }
}

public struct FabushiCodingComputerListResponse: Codable, Sendable, Equatable {
    public let computers: [FabushiCodingRemoteComputerDTO]

    public enum CodingKeys: String, CodingKey {
        case computers
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.computers = (try? c.decodeIfPresent([FabushiCodingRemoteComputerDTO].self, forKey: .computers)) ?? []
    }
}
