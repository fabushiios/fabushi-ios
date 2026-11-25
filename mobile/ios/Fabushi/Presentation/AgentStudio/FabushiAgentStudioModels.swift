import Foundation

// MARK: - 7 Canonical Agent Studio Tabs & Deep-Link Alias Mapping

public enum FabushiStudioTab: String, CaseIterable, Hashable, Sendable, Identifiable {
    case overview
    case kanban
    case skills
    case memory
    case cron
    case approvals
    case sessions

    public var id: String { rawValue }

    public var accessibilityIdentifier: String {
        switch self {
        case .overview: return FabushiAccessibilityIDs.Studio.tabOverview
        case .kanban: return FabushiAccessibilityIDs.Studio.tabKanban
        case .skills: return FabushiAccessibilityIDs.Studio.tabSkills
        case .memory: return FabushiAccessibilityIDs.Studio.tabMemory
        case .cron: return FabushiAccessibilityIDs.Studio.tabCron
        case .approvals: return FabushiAccessibilityIDs.Studio.tabApprovals
        case .sessions: return FabushiAccessibilityIDs.Studio.tabSessions
        }
    }

    public var systemImage: String {
        switch self {
        case .overview: return "sparkles.rectangle.stack"
        case .kanban: return "rectangle.split.3x1"
        case .skills: return "wand.and.stars"
        case .memory: return "brain.head.profile"
        case .cron: return "clock.arrow.2.circlepath"
        case .approvals: return "checkmark.shield"
        case .sessions: return "text.magnifyingglass"
        }
    }

    public var titleKey: String {
        switch self {
        case .overview: return "m4.studio.tab.overview"
        case .kanban: return "m4.studio.tab.kanban"
        case .skills: return "m4.studio.tab.skills"
        case .memory: return "m4.studio.tab.memory"
        case .cron: return "m4.studio.tab.cron"
        case .approvals: return "m4.studio.tab.approvals"
        case .sessions: return "m4.studio.tab.sessions"
        }
    }

    /// Maps all canonical `fabushi://studio/<tab>` deep-link aliases case-insensitively.
    public static func fromDeepLinkAlias(_ raw: String?) -> FabushiStudioTab? {
        guard let raw else { return nil }
        let normalized = raw
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: "-", with: "_")
        switch normalized {
        case "overview", "home", "toolsets", "tools", "mcp", "coding", "agents":
            return .overview
        case "kanban", "tasks", "task", "subagents", "board":
            return .kanban
        case "skills", "skill", "script_app", "scripts":
            return .skills
        case "memory", "soul", "memory_soul", "memorysoul", "user", "honcho", "quarantine":
            return .memory
        case "cron", "schedule", "schedules", "heartbeat", "routines":
            return .cron
        case "approvals", "approval", "takeover", "dock":
            return .approvals
        case "sessions", "session", "history", "trajectories":
            return .sessions
        default:
            return nil
        }
    }

    /// Indicates whether a deep-link alias targets a specific auxiliary console on Overview.
    public static func auxiliarySheetTarget(from raw: String?) -> String? {
        guard let raw else { return nil }
        let normalized = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        switch normalized {
        case "mcp": return "mcp"
        case "coding": return "coding"
        default: return nil
        }
    }
}

// MARK: - Status, Commands & 29 Toolsets (`fabushi.status`, `listCommands`, `listToolsets`, `updateToolsets`)

/// Decodes BOTH embedded local `fabushi.status` AND remote `HermesStatusDto` seamlessly.
public struct FabushiStudioStatusResponse: Codable, Sendable, Equatable {
    // Shared / local fields
    public let engine: String
    public let version: String?
    public let commandsCount: Int?
    public let toolsetsCount: Int
    public let fts5Available: Bool?
    public let journalMode: String?
    public let toolsets: FabushiStudioToolsetSummaryDTO?

    // Remote `HermesStatusDto` fields
    public let healthy: Bool?
    public let activeModel: String?
    public let activeProvider: String?
    public let apiMode: String?
    public let toolsCount: Int?
    public let skillsCount: Int?
    public let cronCount: Int?
    public let sessionsCount: Int?
    public let pendingApprovalsCount: Int?
    public let heartbeatEnabled: Bool?
    public let allowLocalFallback: Bool?
    public let hasActiveSubscription: Bool?
    public let hasConfiguredProvider: Bool?
    public let needsProviderSelection: Bool?

    public var isRemotePayload: Bool {
        healthy != nil || activeModel != nil || needsProviderSelection != nil
    }

    public enum CodingKeys: String, CodingKey {
        case engine
        case version
        case commandsCount = "commands_count"
        case toolsetsCount = "toolsets_count"
        case fts5Available = "fts5_available"
        case journalMode = "journal_mode"
        case toolsets
        case healthy
        case activeModel = "active_model"
        case activeProvider = "active_provider"
        case apiMode = "api_mode"
        case toolsCount = "tools_count"
        case skillsCount = "skills_count"
        case cronCount = "cron_count"
        case sessionsCount = "sessions_count"
        case pendingApprovalsCount = "pending_approvals_count"
        case heartbeatEnabled = "heartbeat_enabled"
        case allowLocalFallback = "allow_local_fallback"
        case hasActiveSubscription = "has_active_subscription"
        case hasConfiguredProvider = "has_configured_provider"
        case needsProviderSelection = "needs_provider_selection"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.engine = (try? c.decode(String.self, forKey: .engine)) ?? "hermes_embedded"
        self.version = try? c.decodeIfPresent(String.self, forKey: .version)
        self.commandsCount = try? c.decodeIfPresent(Int.self, forKey: .commandsCount)
        self.toolsetsCount = (try? c.decode(Int.self, forKey: .toolsetsCount)) ?? 29
        self.fts5Available = try? c.decodeIfPresent(Bool.self, forKey: .fts5Available)
        self.journalMode = try? c.decodeIfPresent(String.self, forKey: .journalMode)
        self.toolsets = try? c.decodeIfPresent(FabushiStudioToolsetSummaryDTO.self, forKey: .toolsets)
        self.healthy = try? c.decodeIfPresent(Bool.self, forKey: .healthy)
        self.activeModel = try? c.decodeIfPresent(String.self, forKey: .activeModel)
        self.activeProvider = try? c.decodeIfPresent(String.self, forKey: .activeProvider)
        self.apiMode = try? c.decodeIfPresent(String.self, forKey: .apiMode)
        self.toolsCount = try? c.decodeIfPresent(Int.self, forKey: .toolsCount)
        self.skillsCount = try? c.decodeIfPresent(Int.self, forKey: .skillsCount)
        self.cronCount = try? c.decodeIfPresent(Int.self, forKey: .cronCount)
        self.sessionsCount = try? c.decodeIfPresent(Int.self, forKey: .sessionsCount)
        self.pendingApprovalsCount = try? c.decodeIfPresent(Int.self, forKey: .pendingApprovalsCount)
        self.heartbeatEnabled = try? c.decodeIfPresent(Bool.self, forKey: .heartbeatEnabled)
        self.allowLocalFallback = try? c.decodeIfPresent(Bool.self, forKey: .allowLocalFallback)
        self.hasActiveSubscription = try? c.decodeIfPresent(Bool.self, forKey: .hasActiveSubscription)
        self.hasConfiguredProvider = try? c.decodeIfPresent(Bool.self, forKey: .hasConfiguredProvider)
        self.needsProviderSelection = try? c.decodeIfPresent(Bool.self, forKey: .needsProviderSelection)
    }
}

public struct FabushiStudioCommandDescriptorDTO: Codable, Sendable, Equatable, Identifiable {
    public let name: String
    public let slashName: String
    public let aliases: [String]
    public let syntax: String
    public let category: String
    public let description: String
    public let acceptsArgs: Bool

    public var id: String { slashName }

    public enum CodingKeys: String, CodingKey {
        case name
        case slashName = "slash_name"
        case aliases
        case syntax
        case category
        case description
        case acceptsArgs = "accepts_args"
    }
}

public struct FabushiStudioListCommandsResponse: Codable, Sendable, Equatable {
    public let count: Int
    public let commands: [FabushiStudioCommandDescriptorDTO]
}

public struct FabushiStudioToolsetItemDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let enabled: Bool
    public let description: String
    public let tools: [String]

    public var isIosSandboxGated: Bool {
        FabushiStudioToolsetCatalog.iosSandboxGatedIds.contains(id)
    }

    public var isRequiredCore: Bool {
        id == "core"
    }
}

public struct FabushiStudioToolsetSummaryDTO: Codable, Sendable, Equatable {
    public let activePreset: String?
    public let activeCustomToolset: String?
    public let enabledCount: Int
    public let totalToolsets: Int
    public let activeToolCount: Int
    public let toolsets: [FabushiStudioToolsetItemDTO]

    public enum CodingKeys: String, CodingKey {
        case activePreset = "active_preset"
        case activeCustomToolset = "active_custom_toolset"
        case enabledCount = "enabled_count"
        case totalToolsets = "total_toolsets"
        case activeToolCount = "active_tool_count"
        case toolsets
    }
}

public struct FabushiStudioListToolsetsResponse: Codable, Sendable, Equatable {
    public let count: Int
    public let summary: FabushiStudioToolsetSummaryDTO
}

/// Complete static catalog of all 29 Hermes toolsets, 6 presets, and 16 `ToolsetId::Browser` tools
/// (highlighting the 6 new `v1.6.11` browser automation tools).
public enum FabushiStudioToolsetCatalog: Sendable {
    public struct ToolsetMetadata: Sendable, Equatable, Identifiable {
        public let id: String
        public let aliases: [String]
        public let iosSandboxGated: Bool
        public let isRequiredCore: Bool

        public init(id: String, aliases: [String] = [], iosSandboxGated: Bool = false, isRequiredCore: Bool = false) {
            self.id = id
            self.aliases = aliases
            self.iosSandboxGated = iosSandboxGated
            self.isRequiredCore = isRequiredCore
        }
    }

    public struct PresetMetadata: Sendable, Equatable, Identifiable {
        public let id: String
        public let titleKey: String
        public let iosAllowed: Bool

        public init(id: String, titleKey: String, iosAllowed: Bool) {
            self.id = id
            self.titleKey = titleKey
            self.iosAllowed = iosAllowed
        }
    }

    public struct BrowserToolMetadata: Sendable, Equatable, Identifiable {
        public let name: String
        public let riskLevel: String
        public let readOnly: Bool
        public let isV1611New: Bool
        public let summaryEn: String
        public let summaryZh: String

        public var id: String { name }

        public func summary(locale: FabushiLocale) -> String {
            locale == .zhHans ? summaryZh : summaryEn
        }
    }

    /// Exact 4 desktop-only toolsets blocked by `source/host/src/handlers_agent.rs` in iOS sandbox.
    public static let iosSandboxGatedIds: Set<String> = [
        "terminal",
        "code_execution",
        "desktop_ui",
        "browser",
    ]

    /// All 29 exact `ToolsetId` variants in canonical upstream order (`hermes/toolsets.rs:L28-L362`).
    public static let allToolsets: [ToolsetMetadata] = [
        ToolsetMetadata(id: "core", isRequiredCore: true),
        ToolsetMetadata(id: "web"),
        ToolsetMetadata(id: "terminal", aliases: ["shell"], iosSandboxGated: true),
        ToolsetMetadata(id: "file", aliases: ["files"]),
        ToolsetMetadata(id: "memory"),
        ToolsetMetadata(id: "skills"),
        ToolsetMetadata(id: "vision"),
        ToolsetMetadata(id: "image_gen", aliases: ["imagegen"]),
        ToolsetMetadata(id: "video_gen", aliases: ["videogen"]),
        ToolsetMetadata(id: "browser", iosSandboxGated: true),
        ToolsetMetadata(id: "audio_tts", aliases: ["audiotts", "tts"]),
        ToolsetMetadata(id: "audio_stt", aliases: ["audiostt", "stt"]),
        ToolsetMetadata(id: "cronjob", aliases: ["cron"]),
        ToolsetMetadata(id: "subagents", aliases: ["delegate"]),
        ToolsetMetadata(id: "code_execution", aliases: ["codeexecution"], iosSandboxGated: true),
        ToolsetMetadata(id: "clarify"),
        ToolsetMetadata(id: "todo"),
        ToolsetMetadata(id: "planning", aliases: ["plan"]),
        ToolsetMetadata(id: "reasoning", aliases: ["think"]),
        ToolsetMetadata(id: "mcp"),
        ToolsetMetadata(id: "kanban"),
        ToolsetMetadata(id: "desktop_ui", aliases: ["desktopui"], iosSandboxGated: true),
        ToolsetMetadata(id: "computer_use", aliases: ["computeruse", "cua"]),
        ToolsetMetadata(id: "connections"),
        ToolsetMetadata(id: "honcho"),
        ToolsetMetadata(id: "session_search", aliases: ["sessionsearch"]),
        ToolsetMetadata(id: "workspace"),
        ToolsetMetadata(id: "voice", aliases: ["talk"]),
        ToolsetMetadata(id: "platform_im", aliases: ["im"]),
    ]

    /// All 6 `ToolsetPreset` identifiers (`hermes/toolsets.rs:L206-L239`).
    public static let presets: [PresetMetadata] = [
        PresetMetadata(id: "safe", titleKey: "m4.studio.toolsets.presetSafe", iosAllowed: true),
        PresetMetadata(id: "minimal", titleKey: "m4.studio.toolsets.presetMinimal", iosAllowed: true),
        PresetMetadata(id: "hermes-cli", titleKey: "m4.studio.toolsets.presetCli", iosAllowed: false),
        PresetMetadata(id: "hermes-telegram", titleKey: "m4.studio.toolsets.presetTelegram", iosAllowed: false),
        PresetMetadata(id: "hermes-acp", titleKey: "m4.studio.toolsets.presetAcp", iosAllowed: false),
        PresetMetadata(id: "full", titleKey: "m4.studio.toolsets.presetFull", iosAllowed: false),
    ]

    /// Complete 16 `ToolsetId::Browser` tools (`toolsets.rs:L1421-L1731`), marking the 6 new `v1.6.11` tools.
    public static let browserTools: [BrowserToolMetadata] = [
        BrowserToolMetadata(name: "browser_navigate", riskLevel: "medium", readOnly: false, isV1611New: false, summaryEn: "Navigate active browser session to an HTTPS URL", summaryZh: "导航至目标网页地址"),
        BrowserToolMetadata(name: "browser_snapshot", riskLevel: "read_only", readOnly: true, isV1611New: false, summaryEn: "Capture redacted DOM snapshot with passwords masked", summaryZh: "采集脱敏 DOM 快照（自动屏蔽密码）"),
        BrowserToolMetadata(name: "browser_click", riskLevel: "medium", readOnly: false, isV1611New: false, summaryEn: "Click element by SemanticLocator or coordinates", summaryZh: "通过语义定位器或坐标点击元素"),
        BrowserToolMetadata(name: "browser_type", riskLevel: "medium", readOnly: false, isV1611New: false, summaryEn: "Type non-secret text into target input field", summaryZh: "向非敏感输入框填充文本"),
        BrowserToolMetadata(name: "browser_screenshot", riskLevel: "read_only", readOnly: true, isV1611New: false, summaryEn: "Capture visual screenshot of page viewport", summaryZh: "截取页面视图截图"),
        BrowserToolMetadata(name: "vault_list", riskLevel: "read_only", readOnly: true, isV1611New: false, summaryEn: "List matching origin credentials in Zero-Knowledge Vault", summaryZh: "列出当前域名的零知识保险库凭据"),
        BrowserToolMetadata(name: "vault_request_card", riskLevel: "medium", readOnly: false, isV1611New: false, summaryEn: "Request human-approved vault credential card", summaryZh: "请求人工确认的保险库凭据卡片"),
        BrowserToolMetadata(name: "vault_fill", riskLevel: "medium", readOnly: false, isV1611New: false, summaryEn: "Fill approved vault secret directly without exposing plaintext to AI", summaryZh: "安全填充已批准凭据，明文不暴露给模型"),
        BrowserToolMetadata(name: "human_assist_qr", riskLevel: "medium", readOnly: false, isV1611New: false, summaryEn: "Relay login QR code to user for manual scan", summaryZh: "将登录二维码转发给用户扫码完成"),
        BrowserToolMetadata(name: "human_assist_click", riskLevel: "medium", readOnly: false, isV1611New: false, summaryEn: "Request guided human click for anti-bot verification", summaryZh: "请求人工点击完成交互验证"),
        BrowserToolMetadata(name: "browser_press_key", riskLevel: "medium", readOnly: false, isV1611New: true, summaryEn: "Send keyboard key press (Enter, Tab, Escape, ArrowDown)", summaryZh: "发送键盘按键（Enter、Tab、Escape 等）"),
        BrowserToolMetadata(name: "browser_scroll", riskLevel: "low", readOnly: false, isV1611New: true, summaryEn: "Scroll viewport by delta_x / delta_y offsets", summaryZh: "按水平与垂直偏移量滚动页面"),
        BrowserToolMetadata(name: "browser_wait_for", riskLevel: "read_only", readOnly: true, isV1611New: true, summaryEn: "Wait up to 30s for SemanticLocator visibility", summaryZh: "等待语义定位器元素出现（最长 30 秒）"),
        BrowserToolMetadata(name: "browser_extract_text", riskLevel: "read_only", readOnly: true, isV1611New: true, summaryEn: "Extract secret-redacted text from non-sensitive element", summaryZh: "提取非敏感元素文本并自动脱敏"),
        BrowserToolMetadata(name: "browser_request_takeover", riskLevel: "medium", readOnly: false, isV1611New: true, summaryEn: "Pause AI gate and hand interactive browser control to user", summaryZh: "暂停 AI 控制门控并交由人工接管浏览器"),
        BrowserToolMetadata(name: "distill_browser_script_app", riskLevel: "low", readOnly: false, isV1611New: true, summaryEn: "Distill recorded browser trace into a zero-LLM ScriptApp skill", summaryZh: "将浏览器操作轨迹蒸馏为零大模型脚本应用技能"),
    ]
}

// MARK: - Browser Takeover Telemetry (`D11-03` `browser_session.rs`)

public enum FabushiStudioBrowserControlMode: Sendable, Equatable {
    case idle
    case aiControlling(activeStep: Int, statusText: String)
    case waitingForTakeover(reason: String)
    case userTakeover(pausedAtStep: Int)
}

public struct FabushiStudioBrowserTakeoverRequest: Identifiable, Sendable, Equatable {
    public let sessionId: String
    public let reason: String
    public let mode: FabushiStudioBrowserControlMode
    public let updatedAtMs: Int64

    public var id: String { sessionId }

    public var takeOverCallbackData: String { "browser_takeover:\(sessionId)" }
    public var resumeAiCallbackData: String { "browser_resume:\(sessionId)" }
    public var cancelAiCallbackData: String { "browser_cancel:\(sessionId)" }

    public init(
        sessionId: String,
        reason: String,
        mode: FabushiStudioBrowserControlMode,
        updatedAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000)
    ) {
        self.sessionId = sessionId
        self.reason = reason
        self.mode = mode
        self.updatedAtMs = updatedAtMs
    }
}

public enum FabushiStudioBrowserTakeoverAction: String, Sendable, Equatable {
    case takeOver = "browser_takeover"
    case resumeAi = "browser_resume"
    case cancelAi = "browser_cancel"

    public func callbackPayload(sessionId: String) -> String {
        "\(rawValue):\(sessionId)"
    }
}

// MARK: - Tasks FSM (`fabushi.listTasks`, `createTask`, `transitionTask`, `approveTask`)

/// Decodes BOTH Rust's PascalCase `TaskState` (`"Pending"`, `"WaitingAuth"`, ...) AND snake_case;
/// exports strict lowercase snake_case via `rpcTransitionValue` for `fabushi.transitionTask`.
public enum FabushiStudioTaskState: String, CaseIterable, Codable, Sendable, Equatable {
    case pending
    case running
    case waitingAuth = "waiting_auth"
    case waitingConnection = "waiting_connection"
    case completed
    case failed
    case cancelled

    public var rpcTransitionValue: String { rawValue }

    public var isTerminal: Bool {
        switch self {
        case .completed, .failed, .cancelled:
            return true
        case .pending, .running, .waitingAuth, .waitingConnection:
            return false
        }
    }

    public var allowedNextStates: [FabushiStudioTaskState] {
        switch self {
        case .pending:
            return [.running, .cancelled]
        case .running:
            return [.waitingAuth, .waitingConnection, .completed, .failed, .cancelled]
        case .waitingAuth, .waitingConnection:
            return [.running, .cancelled]
        case .completed, .failed, .cancelled:
            return []
        }
    }

    public var localizationKey: String {
        switch self {
        case .pending: return "m4.studio.taskState.pending"
        case .running: return "m4.studio.taskState.running"
        case .waitingAuth: return "m4.studio.taskState.waitingAuth"
        case .waitingConnection: return "m4.studio.taskState.waitingConnection"
        case .completed: return "m4.studio.taskState.completed"
        case .failed: return "m4.studio.taskState.failed"
        case .cancelled: return "m4.studio.taskState.cancelled"
        }
    }

    public static func parse(_ raw: String) -> FabushiStudioTaskState? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        switch trimmed.lowercased() {
        case "pending": return .pending
        case "running": return .running
        case "waitingauth", "waiting_auth": return .waitingAuth
        case "waitingconnection", "waiting_connection": return .waitingConnection
        case "completed": return .completed
        case "failed": return .failed
        case "cancelled", "canceled": return .cancelled
        default: return nil
        }
    }

    public init(from decoder: Decoder) throws {
        let single = try decoder.singleValueContainer()
        let raw = try single.decode(String.self)
        guard let parsed = Self.parse(raw) else {
            throw DecodingError.dataCorruptedError(
                in: single,
                debugDescription: "Unknown FabushiStudioTaskState value '\(raw)'"
            )
        }
        self = parsed
    }

    public func encode(to encoder: Encoder) throws {
        var single = encoder.singleValueContainer()
        try single.encode(rpcTransitionValue)
    }
}

/// Heterogeneous 2-element tuple `[TaskState, UInt64]` emitted by Rust `TaskFsm.state_history`.
public struct FabushiStudioTaskHistoryEntry: Codable, Sendable, Equatable {
    public let state: FabushiStudioTaskState
    public let timestampMs: UInt64

    public init(state: FabushiStudioTaskState, timestampMs: UInt64) {
        self.state = state
        self.timestampMs = timestampMs
    }

    public init(from decoder: Decoder) throws {
        var container = try decoder.unkeyedContainer()
        self.state = try container.decode(FabushiStudioTaskState.self)
        self.timestampMs = try container.decode(UInt64.self)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.unkeyedContainer()
        try container.encode(state)
        try container.encode(timestampMs)
    }
}

public struct FabushiStudioTaskFsmDTO: Codable, Sendable, Equatable, Identifiable {
    public let taskId: String
    public let state: FabushiStudioTaskState
    public let stateHistory: [FabushiStudioTaskHistoryEntry]
    public let abortReason: String?

    public var id: String { taskId }

    public enum CodingKeys: String, CodingKey {
        case taskId = "task_id"
        case state
        case stateHistory = "state_history"
        case abortReason = "abort_reason"
    }

    public init(
        taskId: String,
        state: FabushiStudioTaskState,
        stateHistory: [FabushiStudioTaskHistoryEntry] = [],
        abortReason: String? = nil
    ) {
        self.taskId = taskId
        self.state = state
        self.stateHistory = stateHistory
        self.abortReason = abortReason
    }
}

// MARK: - Kanban (`fabushi.kanban*`)

public struct FabushiStudioKanbanBoardDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let description: String
    public let columns: [String]
    public let createdAt: Int64
    public let updatedAt: Int64

    public enum CodingKeys: String, CodingKey {
        case id
        case name
        case description
        case columns
        case createdAt = "created_at"
        case updatedAt = "updated_at"
    }
}

public struct FabushiStudioKanbanCommentDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let author: String
    public let body: String
    public let createdAt: Int64

    public enum CodingKeys: String, CodingKey {
        case id
        case author
        case body
        case createdAt = "created_at"
    }
}

public struct FabushiStudioKanbanChecklistItemDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let text: String
    public let checked: Bool
}

public struct FabushiStudioKanbanCardDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let boardId: String
    public let columnName: String
    public let title: String
    public let description: String
    public let priority: String
    public let assignee: String?
    public let labels: [String]
    public let comments: [FabushiStudioKanbanCommentDTO]
    public let checklist: [FabushiStudioKanbanChecklistItemDTO]
    public let position: Int64
    public let createdAt: Int64
    public let updatedAt: Int64

    public enum CodingKeys: String, CodingKey {
        case id
        case boardId = "board_id"
        case columnName = "column_name"
        case title
        case description
        case priority
        case assignee
        case labels
        case comments
        case checklist
        case position
        case createdAt = "created_at"
        case updatedAt = "updated_at"
    }
}

public struct FabushiStudioDeleteCardResultDTO: Codable, Sendable, Equatable {
    public let deleted: Bool
    public let cardId: String

    public enum CodingKeys: String, CodingKey {
        case deleted
        case cardId = "card_id"
    }
}

// MARK: - Skills & Distilled `AutomationScriptApp` (`D11-04`, `fabushi.skills*`)

/// Decodes BOTH Rust unit variants (`"bundled"`, `"workspace"`, `"user"`, `"learned"`, `"hub"`)
/// AND object tuple variant (`{"custom": "..."}`).
public enum FabushiStudioSkillOrigin: Codable, Sendable, Equatable {
    case bundled
    case workspace
    case user
    case learned
    case hub
    case custom(String)

    public var displayName: String {
        switch self {
        case .bundled: return "bundled"
        case .workspace: return "workspace"
        case .user: return "user"
        case .learned: return "learned"
        case .hub: return "hub"
        case .custom(let value): return value
        }
    }

    private enum ObjectKeys: String, CodingKey {
        case custom
    }

    public init(from decoder: Decoder) throws {
        if let single = try? decoder.singleValueContainer(),
           let raw = try? single.decode(String.self)
        {
            switch raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
            case "bundled": self = .bundled
            case "workspace": self = .workspace
            case "user", "": self = .user
            case "learned": self = .learned
            case "hub": self = .hub
            default: self = .custom(raw)
            }
            return
        }
        let keyed = try decoder.container(keyedBy: ObjectKeys.self)
        let customValue = try keyed.decode(String.self, forKey: .custom)
        self = .custom(customValue)
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .bundled:
            var single = encoder.singleValueContainer()
            try single.encode("bundled")
        case .workspace:
            var single = encoder.singleValueContainer()
            try single.encode("workspace")
        case .user:
            var single = encoder.singleValueContainer()
            try single.encode("user")
        case .learned:
            var single = encoder.singleValueContainer()
            try single.encode("learned")
        case .hub:
            var single = encoder.singleValueContainer()
            try single.encode("hub")
        case .custom(let value):
            var keyed = encoder.container(keyedBy: ObjectKeys.self)
            try keyed.encode(value, forKey: .custom)
        }
    }
}

public enum FabushiStudioSkillSecurityRisk: String, Codable, Sendable, Equatable {
    case safe
    case low
    case medium
    case high
    case critical
}

public struct FabushiStudioSkillSecurityFindingDTO: Codable, Sendable, Equatable, Identifiable {
    public let ruleId: String
    public let severity: FabushiStudioSkillSecurityRisk
    public let location: String
    public let matchedSnippet: String
    public let reason: String

    public var id: String { "\(ruleId):\(location):\(matchedSnippet)" }

    public enum CodingKeys: String, CodingKey {
        case ruleId = "rule_id"
        case severity
        case location
        case matchedSnippet = "matched_snippet"
        case reason
    }
}

public struct FabushiStudioSkillSecurityReportDTO: Codable, Sendable, Equatable {
    public let riskLevel: FabushiStudioSkillSecurityRisk
    public let safeToAutoInstall: Bool
    public let findings: [FabushiStudioSkillSecurityFindingDTO]

    public enum CodingKeys: String, CodingKey {
        case riskLevel = "risk_level"
        case safeToAutoInstall = "safe_to_auto_install"
        case findings
    }
}

public struct FabushiStudioSkillSupplementaryFileDTO: Codable, Sendable, Equatable, Identifiable {
    public let relativePath: String
    public let kind: String
    public let sizeBytes: Int
    public let contentPreview: String?

    public var id: String { relativePath }

    public enum CodingKeys: String, CodingKey {
        case relativePath = "relative_path"
        case kind
        case sizeBytes = "size_bytes"
        case contentPreview = "content_preview"
    }
}

public struct FabushiStudioSkillFrontmatterDTO: Codable, Sendable, Equatable {
    public let name: String
    public let description: String
    public let platforms: [String]
    public let toolsets: [String]
    public let tags: [String]
    public let version: String
    public let origin: FabushiStudioSkillOrigin
}

public struct FabushiStudioSkillDocumentDTO: Codable, Sendable, Equatable, Identifiable {
    public let frontmatter: FabushiStudioSkillFrontmatterDTO
    public let body: String
    public let rawMarkdown: String
    public let sourcePath: String?
    public let supplementaryFiles: [FabushiStudioSkillSupplementaryFileDTO]
    public let trusted: Bool
    public let securityReport: FabushiStudioSkillSecurityReportDTO

    public var id: String { frontmatter.name }

    /// Canonical `D11-04` detector matching `SkillsManager::build_distilled_browser_script_skill_request`.
    public var isDistilledScriptApp: Bool {
        let lowerTags = Set(frontmatter.tags.map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() })
        return lowerTags.contains("browser")
            && lowerTags.contains("script_app")
            && lowerTags.contains("distilled")
    }

    /// Parses an embedded JSON `AutomationScriptApp` fence or falls back to canonical Markdown
    /// `## AutomationScriptApp` / `## Parameters` metadata when inspecting a distilled skill.
    public func extractAutomationScriptApp() -> FabushiStudioAutomationScriptApp? {
        if let fenced = Self.extractJSONCodeBlock(from: body),
           let data = fenced.data(using: .utf8),
           let decoded = try? JSONDecoder().decode(FabushiStudioAutomationScriptApp.self, from: data)
        {
            return decoded
        }

        guard isDistilledScriptApp else { return nil }

        var appId = frontmatter.name
        var slug = frontmatter.name
        var version = frontmatter.version.isEmpty ? "1.0.0" : frontmatter.version
        var targetDomains: [String] = []
        var parameters: [FabushiStudioScriptParameter] = []

        for rawLine in body.unicodeScalars.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = String(rawLine).trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("- App ID:") {
                appId = line.replacingOccurrences(of: "- App ID:", with: "")
                    .trimmingCharacters(in: CharacterSet.whitespaces.union(CharacterSet(charactersIn: "`")))
            } else if line.hasPrefix("- Slug:") {
                slug = line.replacingOccurrences(of: "- Slug:", with: "")
                    .trimmingCharacters(in: CharacterSet.whitespaces.union(CharacterSet(charactersIn: "`")))
            } else if line.hasPrefix("- Version:") {
                version = line.replacingOccurrences(of: "- Version:", with: "")
                    .trimmingCharacters(in: CharacterSet.whitespaces.union(CharacterSet(charactersIn: "`")))
            } else if line.hasPrefix("- Target Domains:") {
                let tail = line.replacingOccurrences(of: "- Target Domains:", with: "")
                    .trimmingCharacters(in: .whitespaces)
                if tail.lowercased() != "any" && !tail.isEmpty {
                    targetDomains = tail
                        .split(separator: ",")
                        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                        .filter { !$0.isEmpty }
                }
            } else if line.hasPrefix("- `"),
                      let closingTick = line.dropFirst(3).firstIndex(of: "`")
            {
                let paramName = String(line[line.index(line.startIndex, offsetBy: 3)..<closingTick])
                let remainder = String(line[line.index(after: closingTick)...])
                let lowerRem = remainder.lowercased()
                let isRequired = lowerRem.contains("required=true")
                let isSecret = lowerRem.contains("secret=true")
                let desc: String
                if let colonIdx = remainder.firstIndex(of: ":") {
                    desc = String(remainder[remainder.index(after: colonIdx)...])
                        .trimmingCharacters(in: .whitespaces)
                } else {
                    desc = ""
                }
                parameters.append(
                    FabushiStudioScriptParameter(
                        name: paramName,
                        description: desc,
                        required: isRequired,
                        defaultValue: nil,
                        isSecret: isSecret
                    )
                )
            }
        }

        let defaultUrl: String
        if let domain = targetDomains.first, !domain.isEmpty {
            defaultUrl = domain.hasPrefix("http://") || domain.hasPrefix("https://")
                ? domain
                : "https://\(domain)"
        } else {
            defaultUrl = "https://example.com"
        }

        return FabushiStudioAutomationScriptApp(
            schemaVersion: 1,
            appId: appId,
            slug: slug,
            name: frontmatter.name,
            version: version,
            description: frontmatter.description,
            category: "automation",
            targetDomains: targetDomains,
            parameters: parameters,
            steps: [.navigate(urlTemplate: defaultUrl)],
            requiredCapabilities: frontmatter.toolsets,
            authorId: nil,
            sourceTraceSummary: nil
        )
    }

    private static func extractJSONCodeBlock(from markdown: String) -> String? {
        guard let fenceStart = markdown.range(of: "```json") else { return nil }
        let afterStart = markdown[fenceStart.upperBound...]
        guard let fenceEnd = afterStart.range(of: "```") else { return nil }
        return String(afterStart[..<fenceEnd.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    public enum CodingKeys: String, CodingKey {
        case frontmatter
        case body
        case rawMarkdown = "raw_markdown"
        case sourcePath = "source_path"
        case supplementaryFiles = "supplementary_files"
        case trusted
        case securityReport = "security_report"
    }
}

public struct FabushiStudioDeleteSkillResultDTO: Codable, Sendable, Equatable {
    public let deleted: Bool
    public let name: String
}

// MARK: - `AutomationScriptApp` Schema (`D11-04`, `bot-browser/src/script_app.rs`)

public struct FabushiStudioSemanticLocator: Codable, Sendable, Equatable {
    public let testId: String?
    public let ariaLabel: String?
    public let role: String?
    public let textAnchor: String?
    public let cssSelector: String?
    public let nameAttr: String?

    public init(
        testId: String? = nil,
        ariaLabel: String? = nil,
        role: String? = nil,
        textAnchor: String? = nil,
        cssSelector: String? = nil,
        nameAttr: String? = nil
    ) {
        self.testId = testId
        self.ariaLabel = ariaLabel
        self.role = role
        self.textAnchor = textAnchor
        self.cssSelector = cssSelector
        self.nameAttr = nameAttr
    }

    public var isEmpty: Bool {
        let fields = [testId, ariaLabel, role, textAnchor, cssSelector, nameAttr]
        return fields.allSatisfy { ($0 ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    public var isSensitiveHint: Bool {
        let joined = [testId, ariaLabel, role, textAnchor, cssSelector, nameAttr]
            .compactMap { $0 }
            .joined(separator: " ")
            .lowercased()
        let hints = [
            "password", "passwd", "passcode", "pwd",
            "otp", "totp", "2fa", "mfa", "pin",
            "secret", "token", "cookie", "api_key", "apikey",
            "credit", "card", "cvv", "cvc", "ssn",
        ]
        return hints.contains { joined.contains($0) }
    }

    public var displaySummary: String {
        if let testId, !testId.isEmpty { return "test_id=\(testId)" }
        if let ariaLabel, !ariaLabel.isEmpty { return "aria=\(ariaLabel)" }
        if let role, !role.isEmpty, let textAnchor, !textAnchor.isEmpty {
            return "\(role):\"\(textAnchor)\""
        }
        if let textAnchor, !textAnchor.isEmpty { return "text=\"\(textAnchor)\"" }
        if let nameAttr, !nameAttr.isEmpty { return "name=\(nameAttr)" }
        if let cssSelector, !cssSelector.isEmpty { return cssSelector }
        return "(empty locator)"
    }

    public enum CodingKeys: String, CodingKey {
        case testId = "test_id"
        case ariaLabel = "aria_label"
        case role
        case textAnchor = "text_anchor"
        case cssSelector = "css_selector"
        case nameAttr = "name_attr"
    }
}

public struct FabushiStudioScriptParameter: Codable, Sendable, Equatable, Identifiable {
    public let name: String
    public let description: String
    public let required: Bool
    public let defaultValue: String?
    public let isSecret: Bool

    public var id: String { name }

    public init(
        name: String,
        description: String = "",
        required: Bool = true,
        defaultValue: String? = nil,
        isSecret: Bool = false
    ) {
        self.name = name
        self.description = description
        self.required = required
        self.defaultValue = defaultValue
        self.isSecret = isSecret
    }

    public enum CodingKeys: String, CodingKey {
        case name
        case description
        case required
        case defaultValue = "default_value"
        case isSecret = "is_secret"
    }
}

/// All 10 `ScriptStep` variants matching `#[serde(tag = "kind", rename_all = "snake_case")]`.
public enum FabushiStudioScriptStep: Codable, Sendable, Equatable {
    case navigate(urlTemplate: String)
    case waitFor(locator: FabushiStudioSemanticLocator, timeoutMs: UInt64)
    case click(locator: FabushiStudioSemanticLocator)
    case fill(locator: FabushiStudioSemanticLocator, valueTemplate: String, secretVaultKey: String?)
    case pressKey(key: String)
    case scroll(deltaX: Double, deltaY: Double)
    case extractText(locator: FabushiStudioSemanticLocator, outputKey: String)
    case assertVisible(locator: FabushiStudioSemanticLocator)
    case assertTextContains(locator: FabushiStudioSemanticLocator, expectedTemplate: String)
    case humanCheckpoint(reason: String)

    private enum CodingKeys: String, CodingKey {
        case kind
        case urlTemplate = "url_template"
        case locator
        case timeoutMs = "timeout_ms"
        case valueTemplate = "value_template"
        case secretVaultKey = "secret_vault_key"
        case key
        case deltaX = "delta_x"
        case deltaY = "delta_y"
        case outputKey = "output_key"
        case expectedTemplate = "expected_template"
        case reason
    }

    public var kindName: String {
        switch self {
        case .navigate: return "navigate"
        case .waitFor: return "wait_for"
        case .click: return "click"
        case .fill: return "fill"
        case .pressKey: return "press_key"
        case .scroll: return "scroll"
        case .extractText: return "extract_text"
        case .assertVisible: return "assert_visible"
        case .assertTextContains: return "assert_text_contains"
        case .humanCheckpoint: return "human_checkpoint"
        }
    }

    public var displaySummary: String {
        switch self {
        case .navigate(let url): return "Navigate → \(url)"
        case .waitFor(let loc, let ms): return "Wait (\(ms)ms) → \(loc.displaySummary)"
        case .click(let loc): return "Click → \(loc.displaySummary)"
        case .fill(let loc, let val, let vaultKey):
            if let vaultKey, !vaultKey.isEmpty {
                return "Vault Fill [\(vaultKey)] → \(loc.displaySummary)"
            }
            return "Fill '\(val)' → \(loc.displaySummary)"
        case .pressKey(let key): return "Press Key → \(key)"
        case .scroll(let dx, let dy): return "Scroll (dx: \(Int(dx)), dy: \(Int(dy)))"
        case .extractText(let loc, let out): return "Extract → \(out) from \(loc.displaySummary)"
        case .assertVisible(let loc): return "Assert Visible → \(loc.displaySummary)"
        case .assertTextContains(let loc, let expected): return "Assert Contains '\(expected)' → \(loc.displaySummary)"
        case .humanCheckpoint(let reason): return "Human Checkpoint: \(reason)"
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try c.decode(String.self, forKey: .kind)
        switch kind {
        case "navigate":
            self = .navigate(urlTemplate: try c.decode(String.self, forKey: .urlTemplate))
        case "wait_for":
            self = .waitFor(
                locator: try c.decode(FabushiStudioSemanticLocator.self, forKey: .locator),
                timeoutMs: try c.decode(UInt64.self, forKey: .timeoutMs)
            )
        case "click":
            self = .click(locator: try c.decode(FabushiStudioSemanticLocator.self, forKey: .locator))
        case "fill":
            self = .fill(
                locator: try c.decode(FabushiStudioSemanticLocator.self, forKey: .locator),
                valueTemplate: try c.decode(String.self, forKey: .valueTemplate),
                secretVaultKey: try c.decodeIfPresent(String.self, forKey: .secretVaultKey)
            )
        case "press_key":
            self = .pressKey(key: try c.decode(String.self, forKey: .key))
        case "scroll":
            self = .scroll(
                deltaX: try c.decode(Double.self, forKey: .deltaX),
                deltaY: try c.decode(Double.self, forKey: .deltaY)
            )
        case "extract_text":
            self = .extractText(
                locator: try c.decode(FabushiStudioSemanticLocator.self, forKey: .locator),
                outputKey: try c.decode(String.self, forKey: .outputKey)
            )
        case "assert_visible":
            self = .assertVisible(locator: try c.decode(FabushiStudioSemanticLocator.self, forKey: .locator))
        case "assert_text_contains":
            self = .assertTextContains(
                locator: try c.decode(FabushiStudioSemanticLocator.self, forKey: .locator),
                expectedTemplate: try c.decode(String.self, forKey: .expectedTemplate)
            )
        case "human_checkpoint":
            self = .humanCheckpoint(reason: try c.decode(String.self, forKey: .reason))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .kind,
                in: c,
                debugDescription: "Unknown ScriptStep kind '\(kind)'"
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(kindName, forKey: .kind)
        switch self {
        case .navigate(let url):
            try c.encode(url, forKey: .urlTemplate)
        case .waitFor(let loc, let timeoutMs):
            try c.encode(loc, forKey: .locator)
            try c.encode(timeoutMs, forKey: .timeoutMs)
        case .click(let loc):
            try c.encode(loc, forKey: .locator)
        case .fill(let loc, let val, let vaultKey):
            try c.encode(loc, forKey: .locator)
            try c.encode(val, forKey: .valueTemplate)
            try c.encodeIfPresent(vaultKey, forKey: .secretVaultKey)
        case .pressKey(let key):
            try c.encode(key, forKey: .key)
        case .scroll(let dx, let dy):
            try c.encode(dx, forKey: .deltaX)
            try c.encode(dy, forKey: .deltaY)
        case .extractText(let loc, let out):
            try c.encode(loc, forKey: .locator)
            try c.encode(out, forKey: .outputKey)
        case .assertVisible(let loc):
            try c.encode(loc, forKey: .locator)
        case .assertTextContains(let loc, let expected):
            try c.encode(loc, forKey: .locator)
            try c.encode(expected, forKey: .expectedTemplate)
        case .humanCheckpoint(let reason):
            try c.encode(reason, forKey: .reason)
        }
    }
}

public struct FabushiStudioAutomationScriptApp: Codable, Sendable, Equatable, Identifiable {
    public let schemaVersion: UInt32
    public let appId: String
    public let slug: String
    public let name: String
    public let version: String
    public let description: String
    public let category: String
    public let targetDomains: [String]
    public let parameters: [FabushiStudioScriptParameter]
    public let steps: [FabushiStudioScriptStep]
    public let requiredCapabilities: [String]
    public let authorId: String?
    public let sourceTraceSummary: String?

    public var id: String { appId }

    public init(
        schemaVersion: UInt32 = 1,
        appId: String,
        slug: String,
        name: String,
        version: String = "1.0.0",
        description: String = "",
        category: String = "automation",
        targetDomains: [String] = [],
        parameters: [FabushiStudioScriptParameter] = [],
        steps: [FabushiStudioScriptStep],
        requiredCapabilities: [String] = ["browser"],
        authorId: String? = nil,
        sourceTraceSummary: String? = nil
    ) {
        self.schemaVersion = schemaVersion
        self.appId = appId
        self.slug = slug
        self.name = name
        self.version = version
        self.description = description
        self.category = category
        self.targetDomains = targetDomains
        self.parameters = parameters
        self.steps = steps
        self.requiredCapabilities = requiredCapabilities
        self.authorId = authorId
        self.sourceTraceSummary = sourceTraceSummary
    }

    public enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case appId = "app_id"
        case slug
        case name
        case version
        case description
        case category
        case targetDomains = "target_domains"
        case parameters
        case steps
        case requiredCapabilities = "required_capabilities"
        case authorId = "author_id"
        case sourceTraceSummary = "source_trace_summary"
    }
}

public struct FabushiStudioScriptStepTimingDTO: Codable, Sendable, Equatable {
    public let stepIndex: Int
    public let kind: String
    public let durationMs: UInt64

    public enum CodingKeys: String, CodingKey {
        case stepIndex = "step_index"
        case kind
        case durationMs = "duration_ms"
    }
}

public struct FabushiStudioScriptReplayReport: Codable, Sendable, Equatable {
    public let appId: String
    public let version: String
    public let success: Bool
    public let stepsExecuted: Int
    public let extractedOutputs: [String: String]
    public let stepTimings: [FabushiStudioScriptStepTimingDTO]
    public let totalDurationMs: UInt64
    public let usedLlm: Bool
    public let finalUrl: String?
    public let error: String?

    public enum CodingKeys: String, CodingKey {
        case appId = "app_id"
        case version
        case success
        case stepsExecuted = "steps_executed"
        case extractedOutputs = "extracted_outputs"
        case stepTimings = "step_timings"
        case totalDurationMs = "total_duration_ms"
        case usedLlm = "used_llm"
        case finalUrl = "final_url"
        case error
    }
}

// MARK: - Memory, Soul, Honcho & Quarantine (`fabushi.memory*`, `soul*`, `honcho*`)

public enum FabushiStudioMemoryTarget: String, CaseIterable, Codable, Sendable, Equatable {
    case memory
    case user

    public var charLimit: Int {
        switch self {
        case .memory: return 2_200
        case .user: return 1_375
        }
    }
}

public struct FabushiStudioMemoryReadDTO: Codable, Sendable, Equatable {
    public let target: String
    public let raw: String
    public let entries: [String]
    public let charCount: Int
    public let charLimit: Int
    public let remainingChars: Int
    public let usagePercent: Float

    public enum CodingKeys: String, CodingKey {
        case target
        case raw
        case entries
        case charCount = "char_count"
        case charLimit = "char_limit"
        case remainingChars = "remaining_chars"
        case usagePercent = "usage_percent"
    }
}

public struct FabushiStudioMemoryMutationDTO: Codable, Sendable, Equatable {
    public let target: String
    public let action: String
    public let entries: [String]
    public let raw: String
    public let charCount: Int
    public let charLimit: Int
    public let remainingChars: Int
    public let usagePercent: Float
    public let changed: Bool

    public enum CodingKeys: String, CodingKey {
        case target
        case action
        case entries
        case raw
        case charCount = "char_count"
        case charLimit = "char_limit"
        case remainingChars = "remaining_chars"
        case usagePercent = "usage_percent"
        case changed
    }
}

public struct FabushiStudioQuarantineProposalDTO: Codable, Sendable, Equatable, Identifiable {
    public let proposalId: String
    public let sourceChatId: Int64
    public let proposedBy: String
    public let target: String
    public let content: String
    public let status: String
    public let createdAtMs: Int64
    public let reviewedBy: String?
    public let reviewedAtMs: Int64?
    public let reviewNote: String?

    public var id: String { proposalId }

    public enum CodingKeys: String, CodingKey {
        case proposalId = "proposal_id"
        case sourceChatId = "source_chat_id"
        case proposedBy = "proposed_by"
        case target
        case content
        case status
        case createdAtMs = "created_at_ms"
        case reviewedBy = "reviewed_by"
        case reviewedAtMs = "reviewed_at_ms"
        case reviewNote = "review_note"
    }
}

public struct FabushiStudioSoulProfileDTO: Codable, Sendable, Equatable {
    public let name: String
    public let role: String
    public let tone: String
    public let persona: String
    public let directives: [String]
    public let traits: [String]
    public let rawMarkdown: String

    public enum CodingKeys: String, CodingKey {
        case name
        case role
        case tone
        case persona
        case directives
        case traits
        case rawMarkdown = "raw_markdown"
    }
}

public struct FabushiStudioHonchoObservationDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let category: String
    public let content: String
    public let confidence: Float
    public let sessionId: String?
    public let observedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case id
        case category
        case content
        case confidence
        case sessionId = "session_id"
        case observedAtMs = "observed_at_ms"
    }
}

public struct FabushiStudioHonchoProfileDTO: Codable, Sendable, Equatable {
    public let userId: String
    public let summary: String
    public let preferences: [String]
    public let recurringGoals: [String]
    public let communicationStyle: [String]
    public let facts: [String]
    public let skills: [String]
    public let observations: [FabushiStudioHonchoObservationDTO]
    public let updatedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case summary
        case preferences
        case recurringGoals = "recurring_goals"
        case communicationStyle = "communication_style"
        case facts
        case skills
        case observations
        case updatedAtMs = "updated_at_ms"
    }
}

public struct FabushiStudioHonchoQueryResultDTO: Codable, Sendable, Equatable {
    public let userId: String
    public let query: String
    public let answer: String
    public let matchedObservations: [FabushiStudioHonchoObservationDTO]

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case query
        case answer
        case matchedObservations = "matched_observations"
    }
}

// MARK: - Cron Schedules (`fabushi.cron*`)

public struct FabushiStudioCronJobDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let schedule: String
    public let timezone: String?
    public let targetChatId: Int64?
    public let prompt: String
    public let model: String?
    public let skillsJson: String
    public let enabled: Bool
    public let deliverTo: String?
    public let originChatId: Int64?
    public let nextRunAt: Int64?
    public let lastRunAt: Int64?
    public let createdAt: Int64

    public enum CodingKeys: String, CodingKey {
        case id
        case name
        case schedule
        case timezone
        case targetChatId = "target_chat_id"
        case prompt
        case model
        case skillsJson = "skills_json"
        case enabled
        case deliverTo = "deliver_to"
        case originChatId = "origin_chat_id"
        case nextRunAt = "next_run_at"
        case lastRunAt = "last_run_at"
        case createdAt = "created_at"
    }
}

public struct FabushiStudioCronPreviewSummaryDTO: Codable, Sendable, Equatable {
    public let scheduledFiresNext7Days: Int
    public let firesOnWeekend: Bool
    public let firesOvernight: Bool

    public enum CodingKeys: String, CodingKey {
        case scheduledFiresNext7Days = "scheduled_fires_next_7_days"
        case firesOnWeekend = "fires_on_weekend"
        case firesOvernight = "fires_overnight"
    }
}

public struct FabushiStudioCronPreviewDTO: Codable, Sendable, Equatable {
    public let valid: Bool
    public let nextRun: String?
    public let summary: FabushiStudioCronPreviewSummaryDTO

    public enum CodingKeys: String, CodingKey {
        case valid
        case nextRun = "next_run"
        case summary
    }
}

public struct FabushiStudioDeleteCronResultDTO: Codable, Sendable, Equatable {
    public let deleted: Bool
    public let jobId: String

    public enum CodingKeys: String, CodingKey {
        case deleted
        case jobId = "job_id"
    }
}

// MARK: - Sessions (`fabushi.sessions*`, `session*`)

public struct FabushiStudioSessionDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let chatId: Int64?
    public let botId: String?
    public let source: String
    public let model: String
    public let apiMode: String
    public let title: String?
    public let parentSessionId: String?
    public let branchFromSeq: Int64?
    public let startedAt: Int64
    public let endedAt: Int64?
    public let messageCount: Int64
    public let inputTokens: Int64
    public let outputTokens: Int64
    public let totalCostUsd: Double
    public let systemPromptHash: String?
    public let activeToolsetsJson: String
    public let metadataJson: String

    public enum CodingKeys: String, CodingKey {
        case id
        case chatId = "chat_id"
        case botId = "bot_id"
        case source
        case model
        case apiMode = "api_mode"
        case title
        case parentSessionId = "parent_session_id"
        case branchFromSeq = "branch_from_seq"
        case startedAt = "started_at"
        case endedAt = "ended_at"
        case messageCount = "message_count"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case totalCostUsd = "total_cost_usd"
        case systemPromptHash = "system_prompt_hash"
        case activeToolsetsJson = "active_toolsets_json"
        case metadataJson = "metadata_json"
    }
}

public struct FabushiStudioSessionSearchHitDTO: Codable, Sendable, Equatable, Identifiable {
    public let sessionId: String
    public let sessionTitle: String?
    public let messageId: String
    public let seq: Int64
    public let role: String
    public let snippet: String
    public let content: String
    public let createdAt: Int64
    public let backend: String

    public var id: String { "\(sessionId):\(messageId):\(seq)" }

    public enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case sessionTitle = "session_title"
        case messageId = "message_id"
        case seq
        case role
        case snippet
        case content
        case createdAt = "created_at"
        case backend
    }
}

// MARK: - Approvals & Value-Bound Approval Ledger (`fabushi.approval*`)

public enum FabushiStudioApprovalStatus: String, Codable, Sendable, Equatable {
    case pending
    case approvedOnce = "approved_once"
    case approvedSession = "approved_session"
    case approvedPermanent = "approved_permanent"
    case denied
}

/// Wire decision values strictly honoring Rust `ApprovalStatus::parse` (`state_db.rs:440-448`):
/// local mode NEVER sends bare `"approved"`, which Rust maps back to `Pending`!
public enum FabushiStudioApprovalDecisionWire: String, CaseIterable, Sendable, Equatable {
    case approvedOnce = "approved_once"
    case approvedSession = "approved_session"
    case approvedPermanent = "approved_permanent"
    case denied = "denied"

    public var localStatusValue: String { rawValue }
    public var remoteDecisionValue: String { rawValue }
}

public struct FabushiStudioApprovalRecordDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let sessionId: String
    public let command: String
    public let riskLevel: String
    public let patternName: String
    public let status: FabushiStudioApprovalStatus
    public let decidedBy: String?
    public let decidedAt: Int64?
    public let createdAt: Int64

    public enum CodingKeys: String, CodingKey {
        case id
        case sessionId = "session_id"
        case command
        case riskLevel = "risk_level"
        case patternName = "pattern_name"
        case status
        case decidedBy = "decided_by"
        case decidedAt = "decided_at"
        case createdAt = "created_at"
    }
}

public struct FabushiStudioValueBoundTokenDTO: Codable, Sendable, Equatable, Identifiable {
    public let approvalId: String
    public let action: String
    public let target: String
    public let workingDirectory: String?
    public let contentSha256: String?
    public let createdAtMs: Int64
    public let expiresAtMs: Int64
    public let spent: Bool
    public let retired: Bool

    public var id: String { approvalId }

    public enum CodingKeys: String, CodingKey {
        case approvalId = "approval_id"
        case action
        case target
        case workingDirectory = "working_directory"
        case contentSha256 = "content_sha256"
        case createdAtMs = "created_at_ms"
        case expiresAtMs = "expires_at_ms"
        case spent
        case retired
    }
}

public struct FabushiStudioToolDockResolveResponse: Codable, Sendable, Equatable {
    public let requestId: String
    public let resolved: Bool
    public let approved: Bool

    public enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
        case resolved
        case approved
    }
}

public struct FabushiStudioLiveToolApprovalRequest: Identifiable, Sendable, Equatable {
    public let approvalId: String
    public let toolName: String
    public let summary: String
    public let receivedAtMs: Int64

    public var id: String { approvalId }

    public init(
        approvalId: String,
        toolName: String,
        summary: String,
        receivedAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000)
    ) {
        self.approvalId = approvalId
        self.toolName = toolName
        self.summary = summary
        self.receivedAtMs = receivedAtMs
    }
}

// MARK: - Bots & Slash Command Execution (`fabushi.botsList`, `fabushi.runCommand`)

public struct FabushiStudioBotSummaryDTO: Codable, Sendable, Equatable, Identifiable {
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
    public let notifyOnAgentUpdates: Bool
    public let hiddenFromSidebar: Bool
    public let memoryEntriesCount: Int
    public let skillsCount: Int
    public let executionState: String
    public let companionChatId: Int64?
    public let codingAgent: FabushiJSONValue?

    public var id: Int64 { botUserId }

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
        case notifyOnAgentUpdates = "notify_on_agent_updates"
        case hiddenFromSidebar = "hidden_from_sidebar"
        case memoryEntriesCount = "memory_entries_count"
        case skillsCount = "skills_count"
        case executionState = "execution_state"
        case companionChatId = "companion_chat_id"
        case codingAgent = "coding_agent"
    }
}

public struct FabushiStudioRunCommandResponse: Codable, Sendable, Equatable {
    public let handled: Bool
    public let command: String
    public let sessionId: String?
    public let output: String
    public let replyText: String?

    public enum CodingKeys: String, CodingKey {
        case handled
        case command
        case sessionId = "session_id"
        case output
        case replyText = "reply_text"
    }
}
