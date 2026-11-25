import Foundation

// MARK: - Automation App Store Wire DTOs (Exact `gpui_telegram_proto::automation_apps` Parity)

public struct FabushiAutomationAppParameterDto: Codable, Sendable, Equatable, Identifiable {
    public var id: String { name }
    public let name: String
    public let description: String
    public let required: Bool
    public let defaultValue: String?
    public let isSecret: Bool

    public init(
        name: String,
        description: String = "",
        required: Bool = false,
        defaultValue: String? = nil,
        isSecret: Bool = false
    ) {
        self.name = name
        self.description = description
        self.required = required
        self.defaultValue = defaultValue
        self.isSecret = isSecret
    }

    enum CodingKeys: String, CodingKey {
        case name
        case description
        case required
        case defaultValue = "default_value"
        case isSecret = "is_secret"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        required = try c.decodeIfPresent(Bool.self, forKey: .required) ?? false
        defaultValue = try c.decodeIfPresent(String.self, forKey: .defaultValue)
        isSecret = try c.decodeIfPresent(Bool.self, forKey: .isSecret) ?? false
    }
}

public struct FabushiPublishAutomationAppRequest: Codable, Sendable, Equatable {
    public let manifest: FabushiJSONValue
    public let isPublished: Bool
    public let changelog: String?

    public init(
        manifest: FabushiJSONValue,
        isPublished: Bool = true,
        changelog: String? = nil
    ) {
        self.manifest = manifest
        self.isPublished = isPublished
        self.changelog = changelog
    }

    enum CodingKeys: String, CodingKey {
        case manifest
        case isPublished = "is_published"
        case changelog
    }
}

public struct FabushiAutomationAppSummary: Codable, Sendable, Equatable, Identifiable {
    public var id: String { appId }
    public let appId: String
    public let slug: String
    public let authorUserId: UInt64
    public let authorUsername: String?
    public let name: String
    public let description: String
    public let category: String
    public let targetDomains: [String]
    public let latestVersion: String
    public let requiredCapabilities: [String]
    public let stepCount: Int
    public let parameters: [FabushiAutomationAppParameterDto]
    public let installCount: UInt64
    public let runCount: UInt64
    public let isPublished: Bool
    public let isInstalled: Bool
    public let createdAtMs: Int64
    public let updatedAtMs: Int64

    public init(
        appId: String,
        slug: String,
        authorUserId: UInt64 = 0,
        authorUsername: String? = nil,
        name: String,
        description: String = "",
        category: String = "",
        targetDomains: [String] = [],
        latestVersion: String = "1.0.0",
        requiredCapabilities: [String] = [],
        stepCount: Int = 0,
        parameters: [FabushiAutomationAppParameterDto] = [],
        installCount: UInt64 = 0,
        runCount: UInt64 = 0,
        isPublished: Bool = true,
        isInstalled: Bool = false,
        createdAtMs: Int64 = 0,
        updatedAtMs: Int64 = 0
    ) {
        self.appId = appId
        self.slug = slug
        self.authorUserId = authorUserId
        self.authorUsername = authorUsername
        self.name = name
        self.description = description
        self.category = category
        self.targetDomains = targetDomains
        self.latestVersion = latestVersion
        self.requiredCapabilities = requiredCapabilities
        self.stepCount = stepCount
        self.parameters = parameters
        self.installCount = installCount
        self.runCount = runCount
        self.isPublished = isPublished
        self.isInstalled = isInstalled
        self.createdAtMs = createdAtMs
        self.updatedAtMs = updatedAtMs
    }

    enum CodingKeys: String, CodingKey {
        case appId = "app_id"
        case slug
        case authorUserId = "author_user_id"
        case authorUsername = "author_username"
        case name
        case description
        case category
        case targetDomains = "target_domains"
        case latestVersion = "latest_version"
        case requiredCapabilities = "required_capabilities"
        case stepCount = "step_count"
        case parameters
        case installCount = "install_count"
        case runCount = "run_count"
        case isPublished = "is_published"
        case isInstalled = "is_installed"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        appId = try c.decodeIfPresent(String.self, forKey: .appId) ?? ""
        slug = try c.decodeIfPresent(String.self, forKey: .slug) ?? ""
        authorUserId = try c.decodeIfPresent(UInt64.self, forKey: .authorUserId) ?? 0
        authorUsername = try c.decodeIfPresent(String.self, forKey: .authorUsername)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        category = try c.decodeIfPresent(String.self, forKey: .category) ?? ""
        targetDomains = try c.decodeIfPresent([String].self, forKey: .targetDomains) ?? []
        latestVersion = try c.decodeIfPresent(String.self, forKey: .latestVersion) ?? "1.0.0"
        requiredCapabilities = try c.decodeIfPresent([String].self, forKey: .requiredCapabilities) ?? []
        stepCount = try c.decodeIfPresent(Int.self, forKey: .stepCount) ?? 0
        parameters = try c.decodeIfPresent([FabushiAutomationAppParameterDto].self, forKey: .parameters) ?? []
        installCount = try c.decodeIfPresent(UInt64.self, forKey: .installCount) ?? 0
        runCount = try c.decodeIfPresent(UInt64.self, forKey: .runCount) ?? 0
        isPublished = try c.decodeIfPresent(Bool.self, forKey: .isPublished) ?? true
        isInstalled = try c.decodeIfPresent(Bool.self, forKey: .isInstalled) ?? false
        createdAtMs = try c.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
        updatedAtMs = try c.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
    }
}

public struct FabushiAutomationAppDetail: Codable, Sendable, Equatable, Identifiable {
    public var id: String { appId }
    public let appId: String
    public let slug: String
    public let authorUserId: UInt64
    public let authorUsername: String?
    public let name: String
    public let description: String
    public let category: String
    public let targetDomains: [String]
    public let latestVersion: String
    public let versions: [String]
    public let manifest: FabushiJSONValue
    public let contentSha256: String
    public let installCount: UInt64
    public let runCount: UInt64
    public let isPublished: Bool
    public let isInstalled: Bool
    public let installedVersion: String?
    public let createdAtMs: Int64
    public let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case appId = "app_id"
        case slug
        case authorUserId = "author_user_id"
        case authorUsername = "author_username"
        case name
        case description
        case category
        case targetDomains = "target_domains"
        case latestVersion = "latest_version"
        case versions
        case manifest
        case contentSha256 = "content_sha256"
        case installCount = "install_count"
        case runCount = "run_count"
        case isPublished = "is_published"
        case isInstalled = "is_installed"
        case installedVersion = "installed_version"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        appId = try c.decodeIfPresent(String.self, forKey: .appId) ?? ""
        slug = try c.decodeIfPresent(String.self, forKey: .slug) ?? ""
        authorUserId = try c.decodeIfPresent(UInt64.self, forKey: .authorUserId) ?? 0
        authorUsername = try c.decodeIfPresent(String.self, forKey: .authorUsername)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        category = try c.decodeIfPresent(String.self, forKey: .category) ?? ""
        targetDomains = try c.decodeIfPresent([String].self, forKey: .targetDomains) ?? []
        latestVersion = try c.decodeIfPresent(String.self, forKey: .latestVersion) ?? "1.0.0"
        versions = try c.decodeIfPresent([String].self, forKey: .versions) ?? []
        manifest = try c.decodeIfPresent(FabushiJSONValue.self, forKey: .manifest) ?? .null
        contentSha256 = try c.decodeIfPresent(String.self, forKey: .contentSha256) ?? ""
        installCount = try c.decodeIfPresent(UInt64.self, forKey: .installCount) ?? 0
        runCount = try c.decodeIfPresent(UInt64.self, forKey: .runCount) ?? 0
        isPublished = try c.decodeIfPresent(Bool.self, forKey: .isPublished) ?? true
        isInstalled = try c.decodeIfPresent(Bool.self, forKey: .isInstalled) ?? false
        installedVersion = try c.decodeIfPresent(String.self, forKey: .installedVersion)
        createdAtMs = try c.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
        updatedAtMs = try c.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
    }
}

public struct FabushiListAutomationAppsQuery: Sendable, Equatable {
    public let q: String?
    public let category: String?
    public let domain: String?
    public let limit: UInt32?
    public let offset: UInt32?

    public init(
        q: String? = nil,
        category: String? = nil,
        domain: String? = nil,
        limit: UInt32? = nil,
        offset: UInt32? = nil
    ) {
        self.q = q
        self.category = category
        self.domain = domain
        self.limit = limit
        self.offset = offset
    }
}

public struct FabushiListAutomationAppsResponse: Codable, Sendable, Equatable {
    public let apps: [FabushiAutomationAppSummary]
    public let total: Int
    public let limit: UInt32
    public let offset: UInt32

    public init(
        apps: [FabushiAutomationAppSummary] = [],
        total: Int = 0,
        limit: UInt32 = 20,
        offset: UInt32 = 0
    ) {
        self.apps = apps
        self.total = total
        self.limit = limit
        self.offset = offset
    }

    enum CodingKeys: String, CodingKey {
        case apps
        case total
        case limit
        case offset
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        apps = try c.decodeIfPresent([FabushiAutomationAppSummary].self, forKey: .apps) ?? []
        total = try c.decodeIfPresent(Int.self, forKey: .total) ?? apps.count
        limit = try c.decodeIfPresent(UInt32.self, forKey: .limit) ?? 20
        offset = try c.decodeIfPresent(UInt32.self, forKey: .offset) ?? 0
    }
}

public struct FabushiInstalledAutomationApp: Codable, Sendable, Equatable, Identifiable {
    public var id: String { appId }
    public let appId: String
    public let slug: String
    public let name: String
    public let description: String
    public let category: String
    public let targetDomains: [String]
    public let installedVersion: String
    public let latestVersion: String
    public let manifest: FabushiJSONValue
    public let contentSha256: String
    public let installedAtMs: Int64
    public let lastRunAtMs: Int64?
    public let localRunCount: UInt64

    public init(
        appId: String,
        slug: String,
        name: String,
        description: String = "",
        category: String = "",
        targetDomains: [String] = [],
        installedVersion: String = "1.0.0",
        latestVersion: String = "1.0.0",
        manifest: FabushiJSONValue,
        contentSha256: String = "",
        installedAtMs: Int64 = 0,
        lastRunAtMs: Int64? = nil,
        localRunCount: UInt64 = 0
    ) {
        self.appId = appId
        self.slug = slug
        self.name = name
        self.description = description
        self.category = category
        self.targetDomains = targetDomains
        self.installedVersion = installedVersion
        self.latestVersion = latestVersion
        self.manifest = manifest
        self.contentSha256 = contentSha256
        self.installedAtMs = installedAtMs
        self.lastRunAtMs = lastRunAtMs
        self.localRunCount = localRunCount
    }

    enum CodingKeys: String, CodingKey {
        case appId = "app_id"
        case slug
        case name
        case description
        case category
        case targetDomains = "target_domains"
        case installedVersion = "installed_version"
        case latestVersion = "latest_version"
        case manifest
        case contentSha256 = "content_sha256"
        case installedAtMs = "installed_at_ms"
        case lastRunAtMs = "last_run_at_ms"
        case localRunCount = "local_run_count"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        appId = try c.decodeIfPresent(String.self, forKey: .appId) ?? ""
        slug = try c.decodeIfPresent(String.self, forKey: .slug) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        category = try c.decodeIfPresent(String.self, forKey: .category) ?? ""
        targetDomains = try c.decodeIfPresent([String].self, forKey: .targetDomains) ?? []
        installedVersion = try c.decodeIfPresent(String.self, forKey: .installedVersion) ?? "1.0.0"
        latestVersion = try c.decodeIfPresent(String.self, forKey: .latestVersion) ?? "1.0.0"
        manifest = try c.decodeIfPresent(FabushiJSONValue.self, forKey: .manifest) ?? .null
        contentSha256 = try c.decodeIfPresent(String.self, forKey: .contentSha256) ?? ""
        installedAtMs = try c.decodeIfPresent(Int64.self, forKey: .installedAtMs) ?? 0
        lastRunAtMs = try c.decodeIfPresent(Int64.self, forKey: .lastRunAtMs)
        localRunCount = try c.decodeIfPresent(UInt64.self, forKey: .localRunCount) ?? 0
    }

    public func decodedScriptApp() -> FabushiAutomationScriptApp? {
        guard let data = manifest.toJSONString().data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(FabushiAutomationScriptApp.self, from: data)
    }
}

public struct FabushiListInstalledAutomationAppsResponse: Codable, Sendable, Equatable {
    public let apps: [FabushiInstalledAutomationApp]
    public let total: Int

    public init(apps: [FabushiInstalledAutomationApp] = [], total: Int = 0) {
        self.apps = apps
        self.total = total
    }

    enum CodingKeys: String, CodingKey {
        case apps
        case total
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        apps = try c.decodeIfPresent([FabushiInstalledAutomationApp].self, forKey: .apps) ?? []
        total = try c.decodeIfPresent(Int.self, forKey: .total) ?? apps.count
    }
}

public struct FabushiInstallAutomationAppResponse: Codable, Sendable, Equatable {
    public let ok: Bool
    public let app: FabushiInstalledAutomationApp
    public let installCount: UInt64
    public let newlyInstalled: Bool

    enum CodingKeys: String, CodingKey {
        case ok
        case app
        case installCount = "install_count"
        case newlyInstalled = "newly_installed"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        ok = try c.decodeIfPresent(Bool.self, forKey: .ok) ?? true
        app = try c.decode(FabushiInstalledAutomationApp.self, forKey: .app)
        installCount = try c.decodeIfPresent(UInt64.self, forKey: .installCount) ?? 0
        newlyInstalled = try c.decodeIfPresent(Bool.self, forKey: .newlyInstalled) ?? false
    }
}

public struct FabushiUninstallAutomationAppResponse: Codable, Sendable, Equatable {
    public let ok: Bool
    public let appId: String
    public let removed: Bool

    enum CodingKeys: String, CodingKey {
        case ok
        case appId = "app_id"
        case removed
    }
}

public struct FabushiRecordAutomationRunRequest: Codable, Sendable, Equatable {
    public let version: String?
    public let success: Bool
    public let stepsExecuted: Int
    public let totalDurationMs: UInt64
    public let usedLlm: Bool
    public let error: String?

    public init(
        version: String? = nil,
        success: Bool,
        stepsExecuted: Int,
        totalDurationMs: UInt64,
        usedLlm: Bool = false,
        error: String? = nil
    ) {
        self.version = version
        self.success = success
        self.stepsExecuted = stepsExecuted
        self.totalDurationMs = totalDurationMs
        self.usedLlm = usedLlm
        self.error = error
    }

    enum CodingKeys: String, CodingKey {
        case version
        case success
        case stepsExecuted = "steps_executed"
        case totalDurationMs = "total_duration_ms"
        case usedLlm = "used_llm"
        case error
    }
}

public struct FabushiRecordAutomationRunResponse: Codable, Sendable, Equatable {
    public let ok: Bool
    public let appId: String
    public let runCount: UInt64
    public let lastRunAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case ok
        case appId = "app_id"
        case runCount = "run_count"
        case lastRunAtMs = "last_run_at_ms"
    }
}

// MARK: - Semantic Locator, Script Steps & `AutomationScriptApp` (`schema_version = 1`)

public struct FabushiBrowserSemanticLocator: Codable, Sendable, Equatable {
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

    enum CodingKeys: String, CodingKey {
        case testId = "test_id"
        case ariaLabel = "aria_label"
        case role
        case textAnchor = "text_anchor"
        case cssSelector = "css_selector"
        case nameAttr = "name_attr"
    }

    public var isEmpty: Bool {
        (testId ?? "").isEmpty
            && (ariaLabel ?? "").isEmpty
            && (role ?? "").isEmpty
            && (textAnchor ?? "").isEmpty
            && (nameAttr ?? "").isEmpty
            && (cssSelector ?? "").isEmpty
    }

    /// Deterministic priority label: `1. test_id -> 2. aria_label -> 3. role+text_anchor -> 4. name_attr -> 5. css`.
    public var highestPriorityStrategy: String {
        if let testId, !testId.isEmpty { return "test_id:\(testId)" }
        if let ariaLabel, !ariaLabel.isEmpty { return "aria_label:\(ariaLabel)" }
        if let role, !role.isEmpty, let textAnchor, !textAnchor.isEmpty {
            return "role+text:\(role):\(textAnchor)"
        }
        if let nameAttr, !nameAttr.isEmpty { return "name_attr:\(nameAttr)" }
        if let cssSelector, !cssSelector.isEmpty { return "css:\(cssSelector)" }
        return "empty"
    }
}

public enum FabushiScriptStep: Codable, Sendable, Equatable {
    case navigate(urlTemplate: String)
    case waitFor(locator: FabushiBrowserSemanticLocator, timeoutMs: UInt64)
    case click(locator: FabushiBrowserSemanticLocator)
    case fill(locator: FabushiBrowserSemanticLocator, valueTemplate: String, secretVaultKey: String?)
    case pressKey(key: String)
    case scroll(deltaX: Double, deltaY: Double)
    case extractText(locator: FabushiBrowserSemanticLocator, outputKey: String)
    case assertVisible(locator: FabushiBrowserSemanticLocator)
    case assertTextContains(locator: FabushiBrowserSemanticLocator, expectedTemplate: String)
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

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try c.decode(String.self, forKey: .kind)
        switch kind {
        case "navigate":
            self = .navigate(urlTemplate: try c.decode(String.self, forKey: .urlTemplate))
        case "wait_for":
            self = .waitFor(
                locator: try c.decode(FabushiBrowserSemanticLocator.self, forKey: .locator),
                timeoutMs: try c.decodeIfPresent(UInt64.self, forKey: .timeoutMs) ?? 5_000
            )
        case "click":
            self = .click(locator: try c.decode(FabushiBrowserSemanticLocator.self, forKey: .locator))
        case "fill":
            self = .fill(
                locator: try c.decode(FabushiBrowserSemanticLocator.self, forKey: .locator),
                valueTemplate: try c.decode(String.self, forKey: .valueTemplate),
                secretVaultKey: try c.decodeIfPresent(String.self, forKey: .secretVaultKey)
            )
        case "press_key":
            self = .pressKey(key: try c.decode(String.self, forKey: .key))
        case "scroll":
            self = .scroll(
                deltaX: try c.decodeIfPresent(Double.self, forKey: .deltaX) ?? 0,
                deltaY: try c.decodeIfPresent(Double.self, forKey: .deltaY) ?? 0
            )
        case "extract_text":
            self = .extractText(
                locator: try c.decode(FabushiBrowserSemanticLocator.self, forKey: .locator),
                outputKey: try c.decode(String.self, forKey: .outputKey)
            )
        case "assert_visible":
            self = .assertVisible(locator: try c.decode(FabushiBrowserSemanticLocator.self, forKey: .locator))
        case "assert_text_contains":
            self = .assertTextContains(
                locator: try c.decode(FabushiBrowserSemanticLocator.self, forKey: .locator),
                expectedTemplate: try c.decode(String.self, forKey: .expectedTemplate)
            )
        case "human_checkpoint":
            self = .humanCheckpoint(reason: try c.decodeIfPresent(String.self, forKey: .reason) ?? "Human review required")
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .kind,
                in: c,
                debugDescription: "Unknown ScriptStep kind '\(kind)'"
            )
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .navigate(let urlTemplate):
            try c.encode("navigate", forKey: .kind)
            try c.encode(urlTemplate, forKey: .urlTemplate)
        case .waitFor(let locator, let timeoutMs):
            try c.encode("wait_for", forKey: .kind)
            try c.encode(locator, forKey: .locator)
            try c.encode(timeoutMs, forKey: .timeoutMs)
        case .click(let locator):
            try c.encode("click", forKey: .kind)
            try c.encode(locator, forKey: .locator)
        case .fill(let locator, let valueTemplate, let secretVaultKey):
            try c.encode("fill", forKey: .kind)
            try c.encode(locator, forKey: .locator)
            try c.encode(valueTemplate, forKey: .valueTemplate)
            try c.encodeIfPresent(secretVaultKey, forKey: .secretVaultKey)
        case .pressKey(let key):
            try c.encode("press_key", forKey: .kind)
            try c.encode(key, forKey: .key)
        case .scroll(let deltaX, let deltaY):
            try c.encode("scroll", forKey: .kind)
            try c.encode(deltaX, forKey: .deltaX)
            try c.encode(deltaY, forKey: .deltaY)
        case .extractText(let locator, let outputKey):
            try c.encode("extract_text", forKey: .kind)
            try c.encode(locator, forKey: .locator)
            try c.encode(outputKey, forKey: .outputKey)
        case .assertVisible(let locator):
            try c.encode("assert_visible", forKey: .kind)
            try c.encode(locator, forKey: .locator)
        case .assertTextContains(let locator, let expectedTemplate):
            try c.encode("assert_text_contains", forKey: .kind)
            try c.encode(locator, forKey: .locator)
            try c.encode(expectedTemplate, forKey: .expectedTemplate)
        case .humanCheckpoint(let reason):
            try c.encode("human_checkpoint", forKey: .kind)
            try c.encode(reason, forKey: .reason)
        }
    }
}

public struct FabushiAutomationScriptApp: Codable, Sendable, Equatable, Identifiable {
    public static let schemaVersionV1: UInt32 = 1

    public var id: String { appId }
    public let schemaVersion: UInt32
    public let appId: String
    public let slug: String
    public let name: String
    public let version: String
    public let description: String
    public let category: String
    public let targetDomains: [String]
    public let parameters: [FabushiAutomationAppParameterDto]
    public let steps: [FabushiScriptStep]
    public let requiredCapabilities: [String]
    public let authorId: String?
    public let sourceTraceSummary: String?

    public init(
        schemaVersion: UInt32 = FabushiAutomationScriptApp.schemaVersionV1,
        appId: String,
        slug: String,
        name: String,
        version: String = "1.0.0",
        description: String = "",
        category: String = "general",
        targetDomains: [String] = [],
        parameters: [FabushiAutomationAppParameterDto] = [],
        steps: [FabushiScriptStep],
        requiredCapabilities: [String] = [],
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

    enum CodingKeys: String, CodingKey {
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

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        schemaVersion = try c.decodeIfPresent(UInt32.self, forKey: .schemaVersion) ?? Self.schemaVersionV1
        appId = try c.decodeIfPresent(String.self, forKey: .appId) ?? ""
        slug = try c.decodeIfPresent(String.self, forKey: .slug) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        version = try c.decodeIfPresent(String.self, forKey: .version) ?? "1.0.0"
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        category = try c.decodeIfPresent(String.self, forKey: .category) ?? "general"
        targetDomains = try c.decodeIfPresent([String].self, forKey: .targetDomains) ?? []
        parameters = try c.decodeIfPresent([FabushiAutomationAppParameterDto].self, forKey: .parameters) ?? []
        steps = try c.decodeIfPresent([FabushiScriptStep].self, forKey: .steps) ?? []
        requiredCapabilities = try c.decodeIfPresent([String].self, forKey: .requiredCapabilities) ?? []
        authorId = try c.decodeIfPresent(String.self, forKey: .authorId)
        sourceTraceSummary = try c.decodeIfPresent(String.self, forKey: .sourceTraceSummary)
    }

    public func toJSONValue() throws -> FabushiJSONValue {
        let data = try JSONEncoder().encode(self)
        return try JSONDecoder().decode(FabushiJSONValue.self, from: data)
    }
}

// MARK: - Recorded Trace, Control Gate, Profiles, Cookies & Replay Report

public enum FabushiRecordedActionKind: Sendable, Equatable, Codable {
    case navigate(url: String)
    case click(locator: FabushiBrowserSemanticLocator)
    case fill(locator: FabushiBrowserSemanticLocator, value: String, isSecret: Bool, parameterNameHint: String?)
    case pressKey(key: String)
    case scroll(deltaX: Double, deltaY: Double)
}

public struct FabushiRecordedBrowserAction: Identifiable, Sendable, Equatable, Codable {
    public var id: Int { seq }
    public let seq: Int
    public let timestampMs: Int64
    public let pageUrl: String?
    public let action: FabushiRecordedActionKind

    public init(
        seq: Int,
        timestampMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000),
        pageUrl: String? = nil,
        action: FabushiRecordedActionKind
    ) {
        self.seq = seq
        self.timestampMs = timestampMs
        self.pageUrl = pageUrl
        self.action = action
    }

    public var isSecretRedacted: Bool {
        if case .fill(_, _, let isSecret, _) = action {
            return isSecret
        }
        return false
    }

    public var summaryText: String {
        switch action {
        case .navigate(let url):
            return "navigate → \(url)"
        case .click(let locator):
            return "click [\(locator.highestPriorityStrategy)]"
        case .fill(let locator, let value, let isSecret, _):
            return "fill [\(locator.highestPriorityStrategy)] = \(isSecret ? "[REDACTED]" : value)"
        case .pressKey(let key):
            return "press_key [\(key)]"
        case .scroll(let dx, let dy):
            return "scroll (\(Int(dx)), \(Int(dy)))"
        }
    }
}

public struct FabushiBrowserActionTrace: Sendable, Equatable, Codable {
    public let traceId: String
    public let sessionId: String
    public let startedAtMs: Int64
    public var completedAtMs: Int64?
    public var initialUrl: String?
    public var actions: [FabushiRecordedBrowserAction]

    public init(
        traceId: String = "trace_\(UUID().uuidString.replacingOccurrences(of: "-", with: ""))",
        sessionId: String = UUID().uuidString,
        startedAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000),
        completedAtMs: Int64? = nil,
        initialUrl: String? = nil,
        actions: [FabushiRecordedBrowserAction] = []
    ) {
        self.traceId = traceId
        self.sessionId = sessionId
        self.startedAtMs = startedAtMs
        self.completedAtMs = completedAtMs
        self.initialUrl = initialUrl
        self.actions = actions
    }
}

public enum FabushiBrowserControlMode: Sendable, Equatable {
    case idle
    case aiControlling(activeStep: Int, statusText: String)
    case waitingForTakeover(reason: String)
    case userTakeover(pausedAtStep: Int)
    case cancelled

    public var isActiveRun: Bool {
        switch self {
        case .aiControlling, .waitingForTakeover, .userTakeover:
            return true
        case .idle, .cancelled:
            return false
        }
    }

    public func badgeTitle(locale: FabushiLocale) -> String {
        switch self {
        case .idle:
            return FabushiPlatformStrings.text(.browserControlIdle, locale: locale)
        case .aiControlling:
            return FabushiPlatformStrings.text(.browserControlAiRunning, locale: locale)
        case .waitingForTakeover:
            return FabushiPlatformStrings.text(.browserControlWaitingTakeover, locale: locale)
        case .userTakeover:
            return FabushiPlatformStrings.text(.browserControlUserTakeover, locale: locale)
        case .cancelled:
            return FabushiPlatformStrings.text(.browserControlCancelled, locale: locale)
        }
    }
}

public struct FabushiBrowserProfileRecord: Identifiable, Codable, Sendable, Equatable {
    public var id: String { profileKey }
    public let profileKey: String
    public let displayName: String
    public let storeIdentifier: UUID?
    public let isEphemeral: Bool
    public let createdAtMs: Int64

    public init(
        profileKey: String,
        displayName: String,
        storeIdentifier: UUID?,
        isEphemeral: Bool = false,
        createdAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000)
    ) {
        self.profileKey = profileKey
        self.displayName = displayName
        self.storeIdentifier = storeIdentifier
        self.isEphemeral = isEphemeral
        self.createdAtMs = createdAtMs
    }
}

public struct FabushiBrowserCookieRecord: Codable, Sendable, Equatable, Identifiable {
    public var id: String { "\(domain)|\(path)|\(name)" }
    public let name: String
    public let value: String
    public let domain: String
    public let path: String
    public let expiresUnixSeconds: Double?
    public let secure: Bool
    public let httpOnly: Bool

    public init(
        name: String,
        value: String,
        domain: String,
        path: String = "/",
        expiresUnixSeconds: Double? = nil,
        secure: Bool = false,
        httpOnly: Bool = false
    ) {
        self.name = name
        self.value = value
        self.domain = domain
        self.path = path
        self.expiresUnixSeconds = expiresUnixSeconds
        self.secure = secure
        self.httpOnly = httpOnly
    }

    enum CodingKeys: String, CodingKey {
        case name
        case value
        case domain
        case path
        case expiresUnixSeconds = "expires"
        case secure
        case httpOnly
    }
}

public struct FabushiBrowserSecretLeakFinding: Sendable, Equatable, Identifiable {
    public let id: String
    public let fieldPath: String
    public let rule: String
    public let summary: String

    public init(fieldPath: String, rule: String, summary: String) {
        self.id = "\(fieldPath):\(rule)"
        self.fieldPath = fieldPath
        self.rule = rule
        self.summary = summary
    }
}

public struct FabushiBrowserScriptReplayReport: Sendable, Equatable {
    public let appId: String
    public let version: String
    public let success: Bool
    public let stepsExecuted: Int
    public let totalDurationMs: UInt64
    public let usedLlm: Bool
    public let extractedOutputs: [String: String]
    public let error: String?

    public init(
        appId: String,
        version: String,
        success: Bool,
        stepsExecuted: Int,
        totalDurationMs: UInt64,
        usedLlm: Bool = false,
        extractedOutputs: [String: String] = [:],
        error: String? = nil
    ) {
        self.appId = appId
        self.version = version
        self.success = success
        self.stepsExecuted = stepsExecuted
        self.totalDurationMs = totalDurationMs
        self.usedLlm = usedLlm
        self.extractedOutputs = extractedOutputs
        self.error = error
    }
}

public enum FabushiBrowserInspectorTab: String, CaseIterable, Identifiable, Sendable {
    case copilot
    case recorder
    case session
    case store

    public var id: String { rawValue }

    public var systemImage: String {
        switch self {
        case .copilot: return "sparkles"
        case .recorder: return "record.circle"
        case .session: return "key.icloud"
        case .store: return "square.grid.2x2"
        }
    }

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .copilot: return FabushiPlatformStrings.text(.browserTabCopilot, locale: locale)
        case .recorder: return FabushiPlatformStrings.text(.browserTabRecorder, locale: locale)
        case .session: return FabushiPlatformStrings.text(.browserTabSession, locale: locale)
        case .store: return FabushiPlatformStrings.text(.browserTabStore, locale: locale)
        }
    }
}
