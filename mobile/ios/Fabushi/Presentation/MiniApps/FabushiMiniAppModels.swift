import Foundation

// MARK: - Mini App Launch & Control Mode Enums (Exact Rust Serde Parity)

public enum FabushiMiniAppLaunchMode: String, Codable, Sendable, Equatable, CaseIterable {
    case fullSize = "FullSize"
    case compactSheet = "CompactSheet"
    case fullScreen = "FullScreen"
    case attachMenu = "AttachMenu"
    case menuButton = "MenuButton"
    case inlineKeyboard = "InlineKeyboard"
    case directLink = "DirectLink"
    case mainMiniApp = "MainMiniApp"
}

public enum FabushiMiniAppControlMode: String, Codable, Sendable, Equatable, CaseIterable {
    case idle = "idle"
    case aiControlling = "ai_controlling"
    case waitingForTakeover = "waiting_for_takeover"
    case userTakeover = "user_takeover"

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .idle:
            return FabushiPlatformStrings.text(.miniAppWebMcpIdle, locale: locale)
        case .aiControlling:
            return FabushiPlatformStrings.text(.miniAppWebMcpAiControlling, locale: locale)
        case .waitingForTakeover:
            return FabushiPlatformStrings.text(.miniAppWebMcpWaitingTakeover, locale: locale)
        case .userTakeover:
            return FabushiPlatformStrings.text(.miniAppWebMcpUserTakeover, locale: locale)
        }
    }
}

public enum FabushiMiniAppMinimizeReason: String, Sendable, Equatable {
    case explicitMinimizeButton
    case openBotProfile
    case openMiniAppTos
    case openTelegramLink
    case openCompanionChat
    case sheetDismissOrEscape
}

public enum FabushiMiniAppCatalogFilter: String, CaseIterable, Identifiable, Sendable {
    case all
    case installed
    case recent

    public var id: String { rawValue }

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .all: return FabushiPlatformStrings.text(.miniAppFilterAll, locale: locale)
        case .installed: return FabushiPlatformStrings.text(.miniAppFilterInstalled, locale: locale)
        case .recent: return FabushiPlatformStrings.text(.miniAppFilterRecent, locale: locale)
        }
    }
}

// MARK: - Mini App & WebMCP DTOs

public struct FabushiMiniAppEntry: Codable, Sendable, Equatable, Identifiable {
    public var id: String { appId.isEmpty ? appKey : appId }
    public let appId: String
    public let botUserId: Int64
    public let botUsername: String
    public let botDisplayName: String
    public let shortName: String
    public let title: String
    public let description: String
    public let category: String
    public let iconEmoji: String
    public let webUrl: String
    public let peerTypes: [String]
    public let installCount: Int64
    public let isInstalled: Bool
    public let hasWriteAccess: Bool
    public let directChatId: Int64?
    public let createdByNl: Bool
    public let updatedAt: Int64
    public let mauCount: Int64
    public let isMainApp: Bool
    public let isRecentUsed: Bool
    public let lastLaunchedAt: Int64
    public let privacyPolicyUrl: String
    public let appTosAccepted: Bool
    public let headerColor: String?
    public let backgroundColor: String?

    public var appKey: String {
        "\(botUsername.trimmingCharacters(in: CharacterSet(charactersIn: "@")))/\(shortName)"
    }

    public init(
        appId: String,
        botUserId: Int64 = 0,
        botUsername: String,
        botDisplayName: String = "",
        shortName: String,
        title: String,
        description: String = "",
        category: String = "",
        iconEmoji: String = "🧩",
        webUrl: String = "",
        peerTypes: [String] = [],
        installCount: Int64 = 0,
        isInstalled: Bool = false,
        hasWriteAccess: Bool = false,
        directChatId: Int64? = nil,
        createdByNl: Bool = false,
        updatedAt: Int64 = 0,
        mauCount: Int64 = 0,
        isMainApp: Bool = false,
        isRecentUsed: Bool = false,
        lastLaunchedAt: Int64 = 0,
        privacyPolicyUrl: String = "",
        appTosAccepted: Bool = false,
        headerColor: String? = nil,
        backgroundColor: String? = nil
    ) {
        self.appId = appId
        self.botUserId = botUserId
        self.botUsername = botUsername
        self.botDisplayName = botDisplayName
        self.shortName = shortName
        self.title = title
        self.description = description
        self.category = category
        self.iconEmoji = iconEmoji
        self.webUrl = webUrl
        self.peerTypes = peerTypes
        self.installCount = installCount
        self.isInstalled = isInstalled
        self.hasWriteAccess = hasWriteAccess
        self.directChatId = directChatId
        self.createdByNl = createdByNl
        self.updatedAt = updatedAt
        self.mauCount = mauCount
        self.isMainApp = isMainApp
        self.isRecentUsed = isRecentUsed
        self.lastLaunchedAt = lastLaunchedAt
        self.privacyPolicyUrl = privacyPolicyUrl
        self.appTosAccepted = appTosAccepted
        self.headerColor = headerColor
        self.backgroundColor = backgroundColor
    }

    enum CodingKeys: String, CodingKey {
        case appId = "app_id"
        case botUserId = "bot_user_id"
        case botUsername = "bot_username"
        case botDisplayName = "bot_display_name"
        case shortName = "short_name"
        case title
        case description
        case category
        case iconEmoji = "icon_emoji"
        case webUrl = "web_url"
        case peerTypes = "peer_types"
        case installCount = "install_count"
        case isInstalled = "is_installed"
        case hasWriteAccess = "has_write_access"
        case directChatId = "direct_chat_id"
        case createdByNl = "created_by_nl"
        case updatedAt = "updated_at"
        case mauCount = "mau_count"
        case isMainApp = "is_main_app"
        case isRecentUsed = "is_recent_used"
        case lastLaunchedAt = "last_launched_at"
        case privacyPolicyUrl = "privacy_policy_url"
        case appTosAccepted = "app_tos_accepted"
        case headerColor = "header_color"
        case backgroundColor = "background_color"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        appId = try c.decodeIfPresent(String.self, forKey: .appId) ?? ""
        botUserId = try c.decodeIfPresent(Int64.self, forKey: .botUserId) ?? 0
        botUsername = try c.decodeIfPresent(String.self, forKey: .botUsername) ?? ""
        botDisplayName = try c.decodeIfPresent(String.self, forKey: .botDisplayName) ?? ""
        shortName = try c.decodeIfPresent(String.self, forKey: .shortName) ?? ""
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        category = try c.decodeIfPresent(String.self, forKey: .category) ?? ""
        iconEmoji = try c.decodeIfPresent(String.self, forKey: .iconEmoji) ?? "🧩"
        webUrl = try c.decodeIfPresent(String.self, forKey: .webUrl) ?? ""
        peerTypes = try c.decodeIfPresent([String].self, forKey: .peerTypes) ?? []
        installCount = try c.decodeIfPresent(Int64.self, forKey: .installCount) ?? 0
        isInstalled = try c.decodeIfPresent(Bool.self, forKey: .isInstalled) ?? false
        hasWriteAccess = try c.decodeIfPresent(Bool.self, forKey: .hasWriteAccess) ?? false
        directChatId = try c.decodeIfPresent(Int64.self, forKey: .directChatId)
        createdByNl = try c.decodeIfPresent(Bool.self, forKey: .createdByNl) ?? false
        updatedAt = try c.decodeIfPresent(Int64.self, forKey: .updatedAt) ?? 0
        mauCount = try c.decodeIfPresent(Int64.self, forKey: .mauCount) ?? 0
        isMainApp = try c.decodeIfPresent(Bool.self, forKey: .isMainApp) ?? false
        isRecentUsed = try c.decodeIfPresent(Bool.self, forKey: .isRecentUsed) ?? false
        lastLaunchedAt = try c.decodeIfPresent(Int64.self, forKey: .lastLaunchedAt) ?? 0
        privacyPolicyUrl = try c.decodeIfPresent(String.self, forKey: .privacyPolicyUrl) ?? ""
        appTosAccepted = try c.decodeIfPresent(Bool.self, forKey: .appTosAccepted) ?? false
        headerColor = try c.decodeIfPresent(String.self, forKey: .headerColor)
        backgroundColor = try c.decodeIfPresent(String.self, forKey: .backgroundColor)
    }

    public func withTosAccepted(_ accepted: Bool) -> FabushiMiniAppEntry {
        FabushiMiniAppEntry(
            appId: appId,
            botUserId: botUserId,
            botUsername: botUsername,
            botDisplayName: botDisplayName,
            shortName: shortName,
            title: title,
            description: description,
            category: category,
            iconEmoji: iconEmoji,
            webUrl: webUrl,
            peerTypes: peerTypes,
            installCount: installCount,
            isInstalled: isInstalled,
            hasWriteAccess: hasWriteAccess,
            directChatId: directChatId,
            createdByNl: createdByNl,
            updatedAt: updatedAt,
            mauCount: mauCount,
            isMainApp: isMainApp,
            isRecentUsed: isRecentUsed,
            lastLaunchedAt: lastLaunchedAt,
            privacyPolicyUrl: privacyPolicyUrl,
            appTosAccepted: accepted,
            headerColor: headerColor,
            backgroundColor: backgroundColor
        )
    }
}

public struct FabushiMiniAppInitData: Codable, Sendable, Equatable {
    public let queryId: String
    public let userJson: String
    public let authDate: Int64
    public let hash: String
    public let startParam: String?
    public let rawQueryString: String
    public let hmacVerified: Bool

    enum CodingKeys: String, CodingKey {
        case queryId = "query_id"
        case userJson = "user_json"
        case authDate = "auth_date"
        case hash
        case startParam = "start_param"
        case rawQueryString = "raw_query_string"
        case hmacVerified = "hmac_verified"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        queryId = try c.decodeIfPresent(String.self, forKey: .queryId) ?? ""
        userJson = try c.decodeIfPresent(String.self, forKey: .userJson) ?? ""
        authDate = try c.decodeIfPresent(Int64.self, forKey: .authDate) ?? 0
        hash = try c.decodeIfPresent(String.self, forKey: .hash) ?? ""
        startParam = try c.decodeIfPresent(String.self, forKey: .startParam)
        rawQueryString = try c.decodeIfPresent(String.self, forKey: .rawQueryString) ?? ""
        hmacVerified = try c.decodeIfPresent(Bool.self, forKey: .hmacVerified) ?? false
    }
}

public struct FabushiMiniAppLaunchContext: Codable, Sendable, Equatable {
    public let app: FabushiMiniAppEntry
    public let companionChatId: Int64?
    public let sourceChatId: Int64?
    public let launchMode: FabushiMiniAppLaunchMode
    public let startParam: String?
    public let initData: FabushiMiniAppInitData?

    enum CodingKeys: String, CodingKey {
        case app
        case companionChatId = "companion_chat_id"
        case sourceChatId = "source_chat_id"
        case launchMode = "launch_mode"
        case startParam = "start_param"
        case initData = "init_data"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        app = try c.decode(FabushiMiniAppEntry.self, forKey: .app)
        companionChatId = try c.decodeIfPresent(Int64.self, forKey: .companionChatId)
        sourceChatId = try c.decodeIfPresent(Int64.self, forKey: .sourceChatId)
        launchMode = try c.decodeIfPresent(FabushiMiniAppLaunchMode.self, forKey: .launchMode) ?? .fullSize
        startParam = try c.decodeIfPresent(String.self, forKey: .startParam)
        initData = try c.decodeIfPresent(FabushiMiniAppInitData.self, forKey: .initData)
    }
}

public struct FabushiMiniAppParsedUri: Codable, Sendable, Equatable {
    public let valid: Bool
    public let botUsername: String?
    public let shortName: String?
    public let startParam: String?

    enum CodingKeys: String, CodingKey {
        case valid
        case botUsername = "bot_username"
        case shortName = "short_name"
        case startParam = "start_param"
    }
}

public struct FabushiMiniAppSanitizeUrlResponse: Codable, Sendable, Equatable {
    public let sanitizedUrl: String?

    enum CodingKeys: String, CodingKey {
        case sanitizedUrl = "sanitized_url"
    }
}

public struct FabushiMiniAppVerifyInitDataResponse: Codable, Sendable, Equatable {
    public let valid: Bool
}

public struct FabushiMiniAppControlStateResponse: Codable, Sendable, Equatable {
    public let appKey: String
    public let mode: FabushiMiniAppControlMode

    enum CodingKeys: String, CodingKey {
        case appKey = "app_key"
        case mode
    }
}

public struct FabushiMiniAppDharmaAssetInfo: Codable, Sendable, Equatable {
    public let name: String
    public let mimeType: String
    public let sizeBytes: Int
    public let hasWasmMagic: Bool

    enum CodingKeys: String, CodingKey {
        case name
        case mimeType = "mime_type"
        case sizeBytes = "size_bytes"
        case hasWasmMagic = "has_wasm_magic"
    }
}

public struct FabushiMiniAppDharmaDispatchResponse: Codable, Sendable, Equatable {
    public let effectsCount: Int
    public let snapshot: FabushiJSONValue

    enum CodingKeys: String, CodingKey {
        case effectsCount = "effects_count"
        case snapshot
    }
}

public struct FabushiMiniAppActiveSession: Identifiable, Sendable, Equatable {
    public var id: String { app.appKey }
    public let app: FabushiMiniAppEntry
    public let resolvedURL: URL?
    public let companionChatId: Int64?
    public var controlMode: FabushiMiniAppControlMode
    public var statusText: String?
    public var isBackgroundWebMcp: Bool
    public let generation: UInt64

    public init(
        app: FabushiMiniAppEntry,
        resolvedURL: URL?,
        companionChatId: Int64? = nil,
        controlMode: FabushiMiniAppControlMode = .idle,
        statusText: String? = nil,
        isBackgroundWebMcp: Bool = false,
        generation: UInt64
    ) {
        self.app = app
        self.resolvedURL = resolvedURL
        self.companionChatId = companionChatId
        self.controlMode = controlMode
        self.statusText = statusText
        self.isBackgroundWebMcp = isBackgroundWebMcp
        self.generation = generation
    }
}

public struct FabushiMiniAppTosPromptState: Identifiable, Sendable, Equatable {
    public var id: String { app.appKey }
    public let app: FabushiMiniAppEntry
    public let pendingStartMinimized: Bool
    public let pendingCustomUrl: String?
    public let pendingStartParam: String?

    public init(
        app: FabushiMiniAppEntry,
        pendingStartMinimized: Bool = false,
        pendingCustomUrl: String? = nil,
        pendingStartParam: String? = nil
    ) {
        self.app = app
        self.pendingStartMinimized = pendingStartMinimized
        self.pendingCustomUrl = pendingCustomUrl
        self.pendingStartParam = pendingStartParam
    }
}
