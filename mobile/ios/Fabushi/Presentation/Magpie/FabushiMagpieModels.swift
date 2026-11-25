import Foundation

// MARK: - Preset & Protocol Kinds (`magpie_gateway/src/presets.rs`)

public enum FabushiMagpiePresetKind: String, CaseIterable, Codable, Sendable, Equatable, Identifiable {
    case vendor
    case relay
    case local
    case remote
    case custom

    public var id: String { rawValue }
}

public enum FabushiMagpieProtocolKind: String, CaseIterable, Codable, Sendable, Equatable, Identifiable {
    case chatCompletions = "chat_completions"
    case codexResponses = "codex_responses"
    case anthropicMessages = "anthropic_messages"
    case geminiGenerateContent = "gemini_generate_content"

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .chatCompletions: return "OpenAI Chat Completions"
        case .codexResponses: return "OpenAI Responses"
        case .anthropicMessages: return "Anthropic Messages"
        case .geminiGenerateContent: return "Gemini GenerateContent"
        }
    }

    public static func parse(_ raw: String) -> FabushiMagpieProtocolKind? {
        let normalized = raw
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: "-", with: "_")
        switch normalized {
        case "chat_completions", "chat", "openai":
            return .chatCompletions
        case "codex_responses", "responses", "codex":
            return .codexResponses
        case "anthropic_messages", "anthropic", "messages", "claude":
            return .anthropicMessages
        case "gemini_generate_content", "gemini", "google_gemini", "generate_content":
            return .geminiGenerateContent
        default:
            return nil
        }
    }
}

public struct FabushiMagpiePresetRegionDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let chatUrl: String?
    public let responsesUrl: String?
    public let anthropicUrl: String?
    public let geminiUrl: String?
    public let keysUrl: String?
    public let website: String?
    public let models: [String]

    public enum CodingKeys: String, CodingKey {
        case id
        case name
        case chatUrl = "chat_url"
        case responsesUrl = "responses_url"
        case anthropicUrl = "anthropic_url"
        case geminiUrl = "gemini_url"
        case keysUrl = "keys_url"
        case website
        case models
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try c.decode(String.self, forKey: .id)
        self.name = (try? c.decode(String.self, forKey: .name)) ?? id
        self.chatUrl = try? c.decodeIfPresent(String.self, forKey: .chatUrl)
        self.responsesUrl = try? c.decodeIfPresent(String.self, forKey: .responsesUrl)
        self.anthropicUrl = try? c.decodeIfPresent(String.self, forKey: .anthropicUrl)
        self.geminiUrl = try? c.decodeIfPresent(String.self, forKey: .geminiUrl)
        self.keysUrl = try? c.decodeIfPresent(String.self, forKey: .keysUrl)
        self.website = try? c.decodeIfPresent(String.self, forKey: .website)
        self.models = (try? c.decodeIfPresent([String].self, forKey: .models)) ?? []
    }
}

public struct FabushiMagpiePresetDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let shortName: String?
    public let icon: String
    public let kind: FabushiMagpiePresetKind
    public let chatUrl: String?
    public let responsesUrl: String?
    public let anthropicUrl: String?
    public let geminiUrl: String?
    public let defaultProtocol: FabushiMagpieProtocolKind
    public let supportedProtocols: [FabushiMagpieProtocolKind]
    public let defaultModel: String
    public let suggestedModels: [String]
    public let noKey: Bool
    public let keyHint: String?
    public let regionLabel: String?
    public let keyPlans: Bool
    public let splitAccounts: Bool
    public let regions: [FabushiMagpiePresetRegionDTO]
    public let headerHints: [String]
    public let website: String?
    public let keysUrl: String?
    public let note: String?
    public let catalog: String?
    public let envVarHints: [String]

    public enum CodingKeys: String, CodingKey {
        case id
        case name
        case shortName = "short_name"
        case icon
        case kind
        case chatUrl = "chat_url"
        case responsesUrl = "responses_url"
        case anthropicUrl = "anthropic_url"
        case geminiUrl = "gemini_url"
        case defaultProtocol = "default_protocol"
        case supportedProtocols = "supported_protocols"
        case defaultModel = "default_model"
        case suggestedModels = "suggested_models"
        case noKey = "no_key"
        case keyHint = "key_hint"
        case regionLabel = "region_label"
        case keyPlans = "key_plans"
        case splitAccounts = "split_accounts"
        case regions
        case headerHints = "header_hints"
        case website
        case keysUrl = "keys_url"
        case note
        case catalog
        case envVarHints = "env_var_hints"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try c.decode(String.self, forKey: .id)
        self.name = (try? c.decode(String.self, forKey: .name)) ?? id
        self.shortName = try? c.decodeIfPresent(String.self, forKey: .shortName)
        self.icon = (try? c.decodeIfPresent(String.self, forKey: .icon)) ?? ""
        self.kind = (try? c.decode(FabushiMagpiePresetKind.self, forKey: .kind)) ?? .vendor
        self.chatUrl = try? c.decodeIfPresent(String.self, forKey: .chatUrl)
        self.responsesUrl = try? c.decodeIfPresent(String.self, forKey: .responsesUrl)
        self.anthropicUrl = try? c.decodeIfPresent(String.self, forKey: .anthropicUrl)
        self.geminiUrl = try? c.decodeIfPresent(String.self, forKey: .geminiUrl)
        self.defaultProtocol = (try? c.decode(FabushiMagpieProtocolKind.self, forKey: .defaultProtocol))
            ?? .chatCompletions
        self.supportedProtocols = (try? c.decodeIfPresent([FabushiMagpieProtocolKind].self, forKey: .supportedProtocols))
            ?? [self.defaultProtocol]
        self.defaultModel = (try? c.decodeIfPresent(String.self, forKey: .defaultModel)) ?? ""
        self.suggestedModels = (try? c.decodeIfPresent([String].self, forKey: .suggestedModels)) ?? []
        self.noKey = (try? c.decodeIfPresent(Bool.self, forKey: .noKey)) ?? false
        self.keyHint = try? c.decodeIfPresent(String.self, forKey: .keyHint)
        self.regionLabel = try? c.decodeIfPresent(String.self, forKey: .regionLabel)
        self.keyPlans = (try? c.decodeIfPresent(Bool.self, forKey: .keyPlans)) ?? false
        self.splitAccounts = (try? c.decodeIfPresent(Bool.self, forKey: .splitAccounts)) ?? false
        self.regions = (try? c.decodeIfPresent([FabushiMagpiePresetRegionDTO].self, forKey: .regions)) ?? []
        self.headerHints = (try? c.decodeIfPresent([String].self, forKey: .headerHints)) ?? []
        self.website = try? c.decodeIfPresent(String.self, forKey: .website)
        self.keysUrl = try? c.decodeIfPresent(String.self, forKey: .keysUrl)
        self.note = try? c.decodeIfPresent(String.self, forKey: .note)
        self.catalog = try? c.decodeIfPresent(String.self, forKey: .catalog)
        self.envVarHints = (try? c.decodeIfPresent([String].self, forKey: .envVarHints)) ?? []
    }
}

public struct FabushiMagpieListPresetsResponse: Codable, Sendable, Equatable {
    public let count: Int
    public let presets: [FabushiMagpiePresetDTO]
}

public struct FabushiMagpieFindPresetRequest: Encodable, Sendable, Equatable {
    public let id: String
}

public struct FabushiMagpieResolvePresetEndpointsRequest: Encodable, Sendable, Equatable {
    public let presetId: String
    public let region: String?
    public let protocolKind: FabushiMagpieProtocolKind?

    public enum CodingKeys: String, CodingKey {
        case presetId = "preset_id"
        case region
        case protocolKind = "protocol"
    }

    public init(presetId: String, region: String? = nil, protocolKind: FabushiMagpieProtocolKind? = nil) {
        self.presetId = presetId
        self.region = region
        self.protocolKind = protocolKind
    }
}

public struct FabushiMagpieResolvedEndpointDTO: Codable, Sendable, Equatable {
    public let presetId: String
    public let displayName: String
    public let regionId: String?
    public let protocolKind: FabushiMagpieProtocolKind
    public let baseUrl: String
    public let defaultModel: String
    public let suggestedModels: [String]
    public let noKey: Bool
    public let website: String?
    public let keysUrl: String?

    public enum CodingKeys: String, CodingKey {
        case presetId = "preset_id"
        case displayName = "display_name"
        case regionId = "region_id"
        case protocolKind = "protocol"
        case baseUrl = "base_url"
        case defaultModel = "default_model"
        case suggestedModels = "suggested_models"
        case noKey = "no_key"
        case website
        case keysUrl = "keys_url"
    }
}

// MARK: - Subscriptions & `D10-10` Active Canonicalization (`subscriptions.rs`, `settings_ai_providers.rs`)

public enum FabushiMagpieSubscriptionKind: String, CaseIterable, Codable, Sendable, Equatable, Identifiable {
    case googleVertex = "google_vertex"
    case codexChatgpt = "codex_chatgpt"
    case claudeCode = "claude_code"
    case githubCopilot = "github_copilot"
    case geminiCodeAssist = "gemini_code_assist"
    case antigravity = "antigravity"
    case grokSupergrok = "grok_supergrok"

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .googleVertex: return "Google Vertex AI"
        case .codexChatgpt: return "Codex / ChatGPT"
        case .claudeCode: return "Claude Code"
        case .githubCopilot: return "GitHub Copilot"
        case .geminiCodeAssist: return "Gemini Code Assist"
        case .antigravity: return "Antigravity"
        case .grokSupergrok: return "Grok / SuperGrok"
        }
    }
}

/// Exact `D10-10` canonical matching for `google_vertex` <-> `magpie_provider:google-vertex`.
public enum FabushiMagpieSubscriptionCanonicalizer: Sendable {
    public static let googleVertexKind = FabushiMagpieSubscriptionKind.googleVertex.rawValue
    public static let googleVertexMagpieProviderId = "magpie_provider:google-vertex"

    public static func isSubscriptionActive(
        subscriptionId: String,
        kind: String,
        activeProvider: String
    ) -> Bool {
        let trimmedSubId = subscriptionId.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedKind = kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let trimmedActive = activeProvider.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedActive.isEmpty else { return false }
        if !trimmedSubId.isEmpty && trimmedSubId == trimmedActive {
            return true
        }
        if trimmedKind == googleVertexKind && trimmedActive == googleVertexMagpieProviderId {
            return true
        }
        return false
    }

    public static func canonicalActiveProviderId(
        subscriptionId: String,
        kind: String
    ) -> String {
        let trimmedKind = kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if trimmedKind == googleVertexKind {
            return googleVertexMagpieProviderId
        }
        return subscriptionId.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

// MARK: - `D10-09` Bot Model Placeholder Normalization & Mismatch Detection

public enum FabushiMagpieBotModelPolicy: Sendable {
    /// Exact placeholder sentinels that represent "Follow Settings" (`""`).
    /// Strictly ZERO fake hardcoded model names (never `"gpt-5.4"` or `"claude-sonnet-4.6"`).
    public static let followSettingsSentinels: Set<String> = [
        "",
        "default",
        "默认",
        "默认模型",
        "auto",
        "hermes-local-v1",
        "local_hermes/hermes-local-v1",
    ]

    /// Normalizes a saved bot `preferred_model` string; returns `""` when it maps to Follow Settings.
    public static func normalizePreferredModel(_ raw: String?) -> String {
        guard let raw else { return "" }
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if followSettingsSentinels.contains(trimmed.lowercased())
            || followSettingsSentinels.contains(trimmed)
        {
            return ""
        }
        return trimmed
    }

    public static func isFollowingSettings(_ raw: String?) -> Bool {
        normalizePreferredModel(raw).isEmpty
    }

    /// Detects when a bot's non-empty override differs from the live Settings active models and `group/<id>` list.
    public static func detectModelMismatch(
        preferredModel: String?,
        availableModels: [String],
        routingGroupIds: [String]
    ) -> Bool {
        let normalized = normalizePreferredModel(preferredModel)
        guard !normalized.isEmpty else { return false }

        var validTargets = Set(
            availableModels
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
                .filter { !$0.isEmpty }
        )
        for gid in routingGroupIds {
            let cleanId = gid.trimmingCharacters(in: .whitespacesAndNewlines)
                .replacingOccurrences(of: "group/", with: "")
            if !cleanId.isEmpty {
                validTargets.insert("group/\(cleanId.lowercased())")
            }
        }
        guard !validTargets.isEmpty else { return false }
        return !validTargets.contains(normalized.lowercased())
    }
}

// MARK: - Providers, Subscriptions & Dual-Shape `magpie.state` DTOs

public struct FabushiMagpieProviderDTO: Codable, Sendable, Equatable, Identifiable {
    public let providerId: String
    public let name: String
    public let apiMode: String
    public let baseUrl: String?
    public let defaultModel: String
    public let models: [String]
    public let enabled: Bool
    public let hasApiKey: Bool
    public let maskedKey: String?
    public let isActive: Bool
    public let presetId: String?
    public let regionId: String?
    public let isSubscription: Bool
    public let noKeyRequired: Bool
    public let routingStrategy: String?
    public let headers: [String: String]

    public var id: String { providerId }

    public enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case name
        case apiMode = "api_mode"
        case baseUrl = "base_url"
        case defaultModel = "default_model"
        case models
        case enabled
        case hasApiKey = "has_api_key"
        case maskedKey = "masked_key"
        case isActive = "is_active"
        case presetId = "preset_id"
        case regionId = "region_id"
        case isSubscription = "is_subscription"
        case noKeyRequired = "no_key_required"
        case routingStrategy = "routing_strategy"
        case headers
    }

    public init(
        providerId: String,
        name: String,
        apiMode: String = "chat_completions",
        baseUrl: String? = nil,
        defaultModel: String = "",
        models: [String] = [],
        enabled: Bool = true,
        hasApiKey: Bool = false,
        maskedKey: String? = nil,
        isActive: Bool = false,
        presetId: String? = nil,
        regionId: String? = nil,
        isSubscription: Bool = false,
        noKeyRequired: Bool = false,
        routingStrategy: String? = nil,
        headers: [String: String] = [:]
    ) {
        self.providerId = providerId
        self.name = name
        self.apiMode = apiMode
        self.baseUrl = baseUrl
        self.defaultModel = defaultModel
        self.models = models
        self.enabled = enabled
        self.hasApiKey = hasApiKey
        self.maskedKey = maskedKey
        self.isActive = isActive
        self.presetId = presetId
        self.regionId = regionId
        self.isSubscription = isSubscription
        self.noKeyRequired = noKeyRequired
        self.routingStrategy = routingStrategy
        self.headers = headers
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.providerId = try c.decode(String.self, forKey: .providerId)
        self.name = (try? c.decodeIfPresent(String.self, forKey: .name)) ?? providerId
        self.apiMode = (try? c.decodeIfPresent(String.self, forKey: .apiMode)) ?? "chat_completions"
        self.baseUrl = try? c.decodeIfPresent(String.self, forKey: .baseUrl)
        self.defaultModel = (try? c.decodeIfPresent(String.self, forKey: .defaultModel)) ?? ""
        self.models = (try? c.decodeIfPresent([String].self, forKey: .models)) ?? []
        self.enabled = (try? c.decodeIfPresent(Bool.self, forKey: .enabled)) ?? true
        self.hasApiKey = (try? c.decodeIfPresent(Bool.self, forKey: .hasApiKey)) ?? false
        self.maskedKey = try? c.decodeIfPresent(String.self, forKey: .maskedKey)
        self.isActive = (try? c.decodeIfPresent(Bool.self, forKey: .isActive)) ?? false
        self.presetId = try? c.decodeIfPresent(String.self, forKey: .presetId)
        self.regionId = try? c.decodeIfPresent(String.self, forKey: .regionId)
        self.isSubscription = (try? c.decodeIfPresent(Bool.self, forKey: .isSubscription)) ?? false
        self.noKeyRequired = (try? c.decodeIfPresent(Bool.self, forKey: .noKeyRequired)) ?? false
        self.routingStrategy = try? c.decodeIfPresent(String.self, forKey: .routingStrategy)
        self.headers = (try? c.decodeIfPresent([String: String].self, forKey: .headers)) ?? [:]
    }
}

public struct FabushiMagpieSubscriptionDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let kind: String
    public let label: String
    public let email: String?
    public let accountId: String?
    public let plan: String?
    public let active: Bool
    public let baseUrl: String?
    public let protocolName: String
    public let defaultModel: String?
    public let models: [String]
    public let sourcePath: String
    public let expiresAtMs: Int64?
    public let headers: [String: String]

    public enum CodingKeys: String, CodingKey {
        case id
        case kind
        case label
        case email
        case accountId = "account_id"
        case plan
        case active
        case baseUrl = "base_url"
        case protocolName = "protocol"
        case defaultModel = "default_model"
        case models
        case sourcePath = "source_path"
        case expiresAtMs = "expires_at_ms"
        case headers
    }

    public init(
        id: String,
        kind: String,
        label: String,
        email: String? = nil,
        accountId: String? = nil,
        plan: String? = nil,
        active: Bool = false,
        baseUrl: String? = nil,
        protocolName: String = "chat_completions",
        defaultModel: String? = nil,
        models: [String] = [],
        sourcePath: String = "",
        expiresAtMs: Int64? = nil,
        headers: [String: String] = [:]
    ) {
        self.id = id
        self.kind = kind
        self.label = label
        self.email = email
        self.accountId = accountId
        self.plan = plan
        self.active = active
        self.baseUrl = baseUrl
        self.protocolName = protocolName
        self.defaultModel = defaultModel
        self.models = models
        self.sourcePath = sourcePath
        self.expiresAtMs = expiresAtMs
        self.headers = headers
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try c.decode(String.self, forKey: .id)
        self.kind = (try? c.decodeIfPresent(String.self, forKey: .kind)) ?? ""
        self.label = (try? c.decodeIfPresent(String.self, forKey: .label)) ?? id
        self.email = try? c.decodeIfPresent(String.self, forKey: .email)
        self.accountId = try? c.decodeIfPresent(String.self, forKey: .accountId)
        self.plan = try? c.decodeIfPresent(String.self, forKey: .plan)
        self.active = (try? c.decodeIfPresent(Bool.self, forKey: .active)) ?? false
        self.baseUrl = try? c.decodeIfPresent(String.self, forKey: .baseUrl)
        self.protocolName = (try? c.decodeIfPresent(String.self, forKey: .protocolName)) ?? "chat_completions"
        self.defaultModel = try? c.decodeIfPresent(String.self, forKey: .defaultModel)
        self.models = (try? c.decodeIfPresent([String].self, forKey: .models)) ?? []
        self.sourcePath = (try? c.decodeIfPresent(String.self, forKey: .sourcePath)) ?? ""
        self.expiresAtMs = try? c.decodeIfPresent(Int64.self, forKey: .expiresAtMs)
        self.headers = (try? c.decodeIfPresent([String: String].self, forKey: .headers)) ?? [:]
    }

    public func isActiveFor(activeProvider: String) -> Bool {
        active || FabushiMagpieSubscriptionCanonicalizer.isSubscriptionActive(
            subscriptionId: id,
            kind: kind,
            activeProvider: activeProvider
        )
    }

    public func candidateModels(fallbackActiveModel: String) -> [String] {
        var list: [String] = []
        if let def = defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines), !def.isEmpty {
            list.append(def)
        }
        for model in models {
            let m = model.trimmingCharacters(in: .whitespacesAndNewlines)
            if !m.isEmpty && !list.contains(m) {
                list.append(m)
            }
        }
        let activeTrimmed = fallbackActiveModel.trimmingCharacters(in: .whitespacesAndNewlines)
        if !activeTrimmed.isEmpty && !list.contains(activeTrimmed) {
            list.append(activeTrimmed)
        }
        return list
    }
}

public struct FabushiMagpieRouteTraceDTO: Codable, Sendable, Equatable, Identifiable {
    public let timestampMs: Int64
    public let requestedModel: String
    public let resolvedProviderId: String
    public let resolvedModel: String
    public let strategy: String
    public let reason: String
    public let candidatesConsidered: Int
    public let maskingHits: Int

    public var id: String { "\(timestampMs):\(requestedModel):\(resolvedProviderId):\(resolvedModel)" }

    public enum CodingKeys: String, CodingKey {
        case timestampMs = "timestamp_ms"
        case requestedModel = "requested_model"
        case resolvedProviderId = "resolved_provider_id"
        case resolvedModel = "resolved_model"
        case strategy
        case reason
        case candidatesConsidered = "candidates_considered"
        case maskingHits = "masking_hits"
    }
}

// MARK: - Smart Routing Groups (`magpie_gateway/src/routing.rs`) & 4 Circuit Breaker States

public enum FabushiMagpieRoutingStrategy: String, CaseIterable, Codable, Sendable, Equatable, Identifiable {
    case smart
    case order
    case rotate
    case usage
    case pace
    case cheapest
    case drain
    case fastest

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .smart: return "Smart"
        case .order: return "Order"
        case .rotate: return "Rotate"
        case .usage: return "Usage"
        case .pace: return "Pace"
        case .cheapest: return "Cheapest"
        case .drain: return "Drain"
        case .fastest: return "Fastest"
        }
    }
}

public struct FabushiMagpieRoutingGroupMemberDTO: Codable, Sendable, Equatable, Identifiable {
    public let target: String
    public let weight: UInt32
    public let enabled: Bool
    public let inputCostPer1m: Double?
    public let outputCostPer1m: Double?
    public let free: Bool

    public var id: String { target }

    public enum CodingKeys: String, CodingKey {
        case target
        case weight
        case enabled
        case inputCostPer1m = "input_cost_per_1m"
        case outputCostPer1m = "output_cost_per_1m"
        case free
    }

    public init(
        target: String,
        weight: UInt32 = 1,
        enabled: Bool = true,
        inputCostPer1m: Double? = nil,
        outputCostPer1m: Double? = nil,
        free: Bool = false
    ) {
        self.target = target
        self.weight = weight
        self.enabled = enabled
        self.inputCostPer1m = inputCostPer1m
        self.outputCostPer1m = outputCostPer1m
        self.free = free
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.target = try c.decode(String.self, forKey: .target)
        self.weight = (try? c.decodeIfPresent(UInt32.self, forKey: .weight)) ?? 1
        self.enabled = (try? c.decodeIfPresent(Bool.self, forKey: .enabled)) ?? true
        self.inputCostPer1m = try? c.decodeIfPresent(Double.self, forKey: .inputCostPer1m)
        self.outputCostPer1m = try? c.decodeIfPresent(Double.self, forKey: .outputCostPer1m)
        self.free = (try? c.decodeIfPresent(Bool.self, forKey: .free)) ?? false
    }
}

public struct FabushiMagpieRoutingGroupDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let strategy: FabushiMagpieRoutingStrategy
    public let members: [FabushiMagpieRoutingGroupMemberDTO]
    public let intentFastModel: String?
    public let intentDeepModel: String?
    public let stickyTtlMs: Int64

    public var canonicalId: String {
        let trimmed = id.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.hasPrefix("group/") {
            return String(trimmed.dropFirst("group/".count))
        }
        return trimmed
    }

    public var qualifiedModelName: String {
        "group/\(canonicalId)"
    }

    public enum CodingKeys: String, CodingKey {
        case id
        case name
        case strategy
        case members
        case intentFastModel = "intent_fast_model"
        case intentDeepModel = "intent_deep_model"
        case stickyTtlMs = "sticky_ttl_ms"
    }

    public init(
        id: String,
        name: String,
        strategy: FabushiMagpieRoutingStrategy = .smart,
        members: [FabushiMagpieRoutingGroupMemberDTO] = [],
        intentFastModel: String? = nil,
        intentDeepModel: String? = nil,
        stickyTtlMs: Int64 = 300_000
    ) {
        let cleanId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        self.id = cleanId.hasPrefix("group/") ? String(cleanId.dropFirst("group/".count)) : cleanId
        self.name = name
        self.strategy = strategy
        self.members = members
        self.intentFastModel = intentFastModel
        self.intentDeepModel = intentDeepModel
        self.stickyTtlMs = stickyTtlMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let rawId = try c.decode(String.self, forKey: .id)
        let trimmed = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
        self.id = trimmed.hasPrefix("group/") ? String(trimmed.dropFirst("group/".count)) : trimmed
        self.name = (try? c.decodeIfPresent(String.self, forKey: .name)) ?? self.id
        if let strat = try? c.decode(FabushiMagpieRoutingStrategy.self, forKey: .strategy) {
            self.strategy = strat
        } else if let rawStrat = try? c.decode(String.self, forKey: .strategy),
                  let parsed = FabushiMagpieRoutingStrategy(rawValue: rawStrat.lowercased())
        {
            self.strategy = parsed
        } else {
            self.strategy = .smart
        }
        self.members = (try? c.decodeIfPresent([FabushiMagpieRoutingGroupMemberDTO].self, forKey: .members)) ?? []
        self.intentFastModel = try? c.decodeIfPresent(String.self, forKey: .intentFastModel)
        self.intentDeepModel = try? c.decodeIfPresent(String.self, forKey: .intentDeepModel)
        self.stickyTtlMs = (try? c.decodeIfPresent(Int64.self, forKey: .stickyTtlMs)) ?? 300_000
    }
}

public struct FabushiMagpieDeleteGroupRequest: Encodable, Sendable, Equatable {
    public let id: String
}

public struct FabushiMagpieDeleteGroupResponse: Codable, Sendable, Equatable {
    public let deleted: Bool
    public let id: String
}

public struct FabushiMagpieResolveRouteRequest: Encodable, Sendable, Equatable {
    public let target: String
    public let defaultProvider: String?
    public let sessionKey: String?
    public let prompt: String?
    public let messageCount: UInt64?
    public let toolCount: UInt64?
    public let nowMs: Int64?

    public enum CodingKeys: String, CodingKey {
        case target
        case defaultProvider = "default_provider"
        case sessionKey = "session_key"
        case prompt
        case messageCount = "message_count"
        case toolCount = "tool_count"
        case nowMs = "now_ms"
    }

    public init(
        target: String,
        defaultProvider: String? = "deepseek",
        sessionKey: String? = nil,
        prompt: String? = nil,
        messageCount: UInt64? = 1,
        toolCount: UInt64? = 0,
        nowMs: Int64? = nil
    ) {
        self.target = target
        self.defaultProvider = defaultProvider
        self.sessionKey = sessionKey
        self.prompt = prompt
        self.messageCount = messageCount
        self.toolCount = toolCount
        self.nowMs = nowMs
    }
}

public struct FabushiMagpieResolvedRouteDTO: Codable, Sendable, Equatable, Identifiable {
    public let providerId: String
    public let model: String
    public let strategy: FabushiMagpieRoutingStrategy
    public let reason: String

    public var id: String { "\(providerId)/\(model):\(strategy.rawValue)" }

    public enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case model
        case strategy
        case reason
    }
}

public enum FabushiMagpieCircuitBreakerState: String, Sendable, Equatable {
    case closed
    case open
    case halfOpen = "half_open"

    public var localizationKey: String {
        switch self {
        case .closed: return "m4.magpie.circuitClosed"
        case .open: return "m4.magpie.circuitOpen"
        case .halfOpen: return "m4.magpie.circuitHalfOpen"
        }
    }
}

public enum FabushiMagpieCircuitRestKind: String, CaseIterable, Codable, Sendable, Equatable {
    case quotaRest = "quota_rest"
    case creditRest = "credit_rest"
    case overloadRest = "overload_rest"
    case rateRest = "rate_rest"
}

/// Pure-Swift mirror of `magpie_gateway/src/routing.rs` circuit-breaker backoff ladder & `forbiddenAlike` rules.
public enum FabushiMagpieCircuitBreakerPolicy: Sendable {
    /// Exact 7-step credit rest ladder: `[30m, 1h, 2h, 4h, 8h, 24h, 7d]` in milliseconds.
    public static let creditRestLadderMs: [Int64] = [
        1_800_000,
        3_600_000,
        7_200_000,
        14_400_000,
        28_800_000,
        86_400_000,
        604_800_000,
    ]
    public static let overloadRestMs: Int64 = 30_000
    public static let rateBaseMs: Int64 = 5_000
    public static let rateMaxMs: Int64 = 1_800_000
    public static let defaultQuotaRestMs: Int64 = 3_600_000

    private static let forbiddenAlikeMarkers: [String] = [
        "safety",
        "policy",
        "content_filter",
        "invalid_request",
        "unsupported_country",
        "context_length",
        "model_not_found",
        "model not found",
        "prompt",
    ]

    public static func creditRestDurationMs(consecutiveFailures: Int) -> Int64 {
        let idx = min(max(0, consecutiveFailures), creditRestLadderMs.count - 1)
        return creditRestLadderMs[idx]
    }

    public static func rateRestDurationMs(consecutiveFailures: Int, retryAfterSecs: UInt64? = nil) -> Int64 {
        if let retrySecs = retryAfterSecs, retrySecs > 0 {
            return min(Int64(retrySecs) * 1_000, rateMaxMs)
        }
        let shift = min(max(0, consecutiveFailures), 10)
        let computed = rateBaseMs << shift
        return min(computed, rateMaxMs)
    }

    public static func isForbiddenAlike(statusCode: UInt16?, errorText: String) -> Bool {
        guard statusCode == 400 || statusCode == 403 else { return false }
        let lower = errorText.lowercased()
        return forbiddenAlikeMarkers.contains { lower.contains($0) }
    }

    public static func classifyRestKind(statusCode: UInt16?, errorText: String) -> FabushiMagpieCircuitRestKind? {
        if isForbiddenAlike(statusCode: statusCode, errorText: errorText) {
            return nil
        }
        let lower = errorText.lowercased()
        if statusCode == 402
            || lower.contains("credit")
            || lower.contains("insufficient_quota")
            || lower.contains("billing")
            || lower.contains("balance")
        {
            return .creditRest
        }
        if statusCode == 503
            || statusCode == 529
            || (statusCode == 429 && (lower.contains("overload") || lower.contains("capacity")))
        {
            return .overloadRest
        }
        if lower.contains("quota") {
            return .quotaRest
        }
        if statusCode == 429 || lower.contains("rate_limit") || lower.contains("rate limit") {
            return .rateRest
        }
        return nil
    }
}

// MARK: - PII Privacy Masking (`magpie.maskingUpdate`, `maskText`, `maskingPreview`, `unmaskText`)

public enum FabushiMagpieMaskingAction: String, CaseIterable, Codable, Sendable, Equatable, Identifiable {
    case mask
    case warn
    case block

    public var id: String { rawValue }
}

public struct FabushiMagpieMaskingConfigDTO: Codable, Sendable, Equatable {
    public var enabled: Bool
    public var maskSecrets: Bool
    public var maskPersonal: Bool
    public var customWords: [String]
    public var allowList: [String]
    public var defaultAction: FabushiMagpieMaskingAction
    public var kindActions: [String: FabushiMagpieMaskingAction]

    public enum CodingKeys: String, CodingKey {
        case enabled
        case maskSecrets = "mask_secrets"
        case maskPersonal = "mask_personal"
        case customWords = "custom_words"
        case allowList = "allow_list"
        case defaultAction = "default_action"
        case kindActions = "kind_actions"
    }

    public init(
        enabled: Bool = true,
        maskSecrets: Bool = true,
        maskPersonal: Bool = true,
        customWords: [String] = [],
        allowList: [String] = [],
        defaultAction: FabushiMagpieMaskingAction = .mask,
        kindActions: [String: FabushiMagpieMaskingAction] = [:]
    ) {
        self.enabled = enabled
        self.maskSecrets = maskSecrets
        self.maskPersonal = maskPersonal
        self.customWords = customWords
        self.allowList = allowList
        self.defaultAction = defaultAction
        self.kindActions = kindActions
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.enabled = (try? c.decodeIfPresent(Bool.self, forKey: .enabled)) ?? false
        self.maskSecrets = (try? c.decodeIfPresent(Bool.self, forKey: .maskSecrets)) ?? true
        self.maskPersonal = (try? c.decodeIfPresent(Bool.self, forKey: .maskPersonal)) ?? true
        self.customWords = (try? c.decodeIfPresent([String].self, forKey: .customWords)) ?? []
        self.allowList = (try? c.decodeIfPresent([String].self, forKey: .allowList)) ?? []
        self.defaultAction = (try? c.decodeIfPresent(FabushiMagpieMaskingAction.self, forKey: .defaultAction)) ?? .mask
        self.kindActions = (try? c.decodeIfPresent([String: FabushiMagpieMaskingAction].self, forKey: .kindActions)) ?? [:]
    }
}

public struct FabushiMagpieMaskingHitDTO: Codable, Sendable, Equatable, Identifiable {
    public let kind: String
    public let placeholder: String
    public let action: FabushiMagpieMaskingAction
    public let previewHint: String

    public var id: String { "\(kind):\(placeholder):\(previewHint)" }

    public enum CodingKeys: String, CodingKey {
        case kind
        case placeholder
        case action
        case previewHint = "preview_hint"
    }
}

public struct FabushiMagpieMaskResultDTO: Codable, Sendable, Equatable {
    public let maskedText: String
    public let hits: [FabushiMagpieMaskingHitDTO]
    public let blocked: Bool
    public let blockReason: String?

    public enum CodingKeys: String, CodingKey {
        case maskedText = "masked_text"
        case hits
        case blocked
        case blockReason = "block_reason"
    }
}

public struct FabushiMagpieMaskTextRequest: Encodable, Sendable, Equatable {
    public let text: String
    public let config: FabushiMagpieMaskingConfigDTO?

    public init(text: String, config: FabushiMagpieMaskingConfigDTO? = nil) {
        self.text = text
        self.config = config
    }
}

public struct FabushiMagpieUnmaskRequest: Encodable, Sendable, Equatable {
    public let text: String
}

public struct FabushiMagpieUnmaskResponse: Codable, Sendable, Equatable {
    public let text: String
}

// MARK: - Import (`magpie.importLink`, `magpie.importDotenv`)

public struct FabushiMagpieImportLinkRequest: Encodable, Sendable, Equatable {
    public let uri: String
}

public struct FabushiMagpieImportDotenvRequest: Encodable, Sendable, Equatable {
    public let text: String
}

public struct FabushiMagpieImportedProviderSpecDTO: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let presetId: String?
    public let regionId: String?
    public let apiKey: String?
    public let protocolKind: FabushiMagpieProtocolKind
    public let baseUrl: String
    public let chatUrl: String?
    public let responsesUrl: String?
    public let anthropicUrl: String?
    public let geminiUrl: String?
    public let defaultModel: String
    public let models: [String]
    public let catalog: String?
    public let website: String?
    public let keysUrl: String?

    public enum CodingKeys: String, CodingKey {
        case id
        case name
        case presetId = "preset_id"
        case regionId = "region_id"
        case apiKey = "api_key"
        case protocolKind = "protocol"
        case baseUrl = "base_url"
        case chatUrl = "chat_url"
        case responsesUrl = "responses_url"
        case anthropicUrl = "anthropic_url"
        case geminiUrl = "gemini_url"
        case defaultModel = "default_model"
        case models
        case catalog
        case website
        case keysUrl = "keys_url"
    }
}

// MARK: - Usage & Failure Recording (`magpie.recordUsage`, `recordFailure`, `usageSummary`)

/// Sends BOTH routing metrics fields (`target`, `tokens`, `latency_ms`, `now_ms`) AND all 10 required
/// `UsageRecord` fields so `magpie.usageSummary` accurately aggregates every recorded turn.
public struct FabushiMagpieUsageRecordRequest: Encodable, Sendable, Equatable {
    public let target: String
    public let tokens: UInt64
    public let latencyMs: UInt64?
    public let nowMs: Int64
    public let id: String
    public let sessionId: String
    public let timestampMs: Int64
    public let providerId: String
    public let model: String
    public let inputTokens: UInt64
    public let outputTokens: UInt64
    public let cacheReadTokens: UInt64
    public let reasoningTokens: UInt64
    public let ttftMs: UInt64?
    public let durationMs: UInt64?
    public let tps: Double?
    public let costUsd: Double

    public enum CodingKeys: String, CodingKey {
        case target
        case tokens
        case latencyMs = "latency_ms"
        case nowMs = "now_ms"
        case id
        case sessionId = "session_id"
        case timestampMs = "timestamp_ms"
        case providerId = "provider_id"
        case model
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case reasoningTokens = "reasoning_tokens"
        case ttftMs = "ttft_ms"
        case durationMs = "duration_ms"
        case tps
        case costUsd = "cost_usd"
    }

    public init(
        id: String = "u_\(UUID().uuidString.prefix(8))",
        sessionId: String,
        providerId: String,
        model: String,
        inputTokens: UInt64,
        outputTokens: UInt64,
        cacheReadTokens: UInt64 = 0,
        reasoningTokens: UInt64 = 0,
        costUsd: Double,
        ttftMs: UInt64? = nil,
        durationMs: UInt64? = nil,
        tps: Double? = nil,
        timestampMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000)
    ) {
        self.target = "\(providerId)/\(model)"
        self.tokens = inputTokens + outputTokens
        self.latencyMs = durationMs ?? ttftMs
        self.nowMs = timestampMs
        self.id = id
        self.sessionId = sessionId
        self.timestampMs = timestampMs
        self.providerId = providerId
        self.model = model
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.cacheReadTokens = cacheReadTokens
        self.reasoningTokens = reasoningTokens
        self.ttftMs = ttftMs
        self.durationMs = durationMs
        self.tps = tps
        self.costUsd = costUsd
    }
}

public struct FabushiMagpieRecordFailureRequest: Encodable, Sendable, Equatable {
    public let target: String
    public let statusCode: UInt16?
    public let retryAfterSecs: UInt64?
    public let nowMs: Int64?

    public enum CodingKeys: String, CodingKey {
        case target
        case statusCode = "status_code"
        case retryAfterSecs = "retry_after_secs"
        case nowMs = "now_ms"
    }

    public init(
        target: String,
        statusCode: UInt16? = 429,
        retryAfterSecs: UInt64? = 60,
        nowMs: Int64? = nil
    ) {
        self.target = target
        self.statusCode = statusCode
        self.retryAfterSecs = retryAfterSecs
        self.nowMs = nowMs
    }
}

public struct FabushiMagpieRecordedResponse: Codable, Sendable, Equatable {
    public let recorded: Bool
}

public struct FabushiMagpieUsageSummaryRequest: Encodable, Sendable, Equatable {
    public let sinceMs: Int64?

    public enum CodingKeys: String, CodingKey {
        case sinceMs = "since_ms"
    }

    public init(sinceMs: Int64? = nil) {
        self.sinceMs = sinceMs
    }
}

public struct FabushiMagpieUsageSummaryDTO: Codable, Sendable, Equatable {
    public let totalRecords: UInt64
    public let totalInputTokens: UInt64
    public let totalOutputTokens: UInt64
    public let totalCacheReadTokens: UInt64
    public let totalReasoningTokens: UInt64
    public let totalTokens: UInt64
    public let totalCostUsd: Double
    public let avgTtftMs: Double?
    public let avgDurationMs: Double?
    public let avgTps: Double?
    public let bySession: [String: UInt64]
    public let byProvider: [String: UInt64]
    public let byModel: [String: UInt64]

    public enum CodingKeys: String, CodingKey {
        case totalRecords = "total_records"
        case totalInputTokens = "total_input_tokens"
        case totalOutputTokens = "total_output_tokens"
        case totalCacheReadTokens = "total_cache_read_tokens"
        case totalReasoningTokens = "total_reasoning_tokens"
        case totalTokens = "total_tokens"
        case totalCostUsd = "total_cost_usd"
        case avgTtftMs = "avg_ttft_ms"
        case avgDurationMs = "avg_duration_ms"
        case avgTps = "avg_tps"
        case bySession = "by_session"
        case byProvider = "by_provider"
        case byModel = "by_model"
    }

    public static let empty = FabushiMagpieUsageSummaryDTO(
        totalRecords: 0,
        totalInputTokens: 0,
        totalOutputTokens: 0,
        totalCacheReadTokens: 0,
        totalReasoningTokens: 0,
        totalTokens: 0,
        totalCostUsd: 0,
        avgTtftMs: nil,
        avgDurationMs: nil,
        avgTps: nil,
        bySession: [:],
        byProvider: [:],
        byModel: [:]
    )
}

// MARK: - Dual-Shape `magpie.state` Request & Response

public struct FabushiMagpieStateRequest: Encodable, Sendable, Equatable {
    public let remote: Bool

    public init(remote: Bool) {
        self.remote = remote
    }
}

/// Decodes BOTH embedded local `magpie.state` (`presets_count`, `groups`, `usage`) AND
/// remote server `MagpieStateDto` (`providers`, `subscriptions`, `routing_groups`, `masking`, etc.).
public struct FabushiMagpieStateResponse: Codable, Sendable, Equatable {
    // Local shape
    public let presetsCount: Int?
    public let localGroups: [FabushiMagpieRoutingGroupDTO]
    public let localUsage: FabushiMagpieUsageSummaryDTO?

    // Remote `MagpieStateDto` shape
    public let presets: [FabushiMagpiePresetDTO]
    public let providers: [FabushiMagpieProviderDTO]
    public let activeProvider: String
    public let activeModel: String
    public let allowLocalFallback: Bool
    public let hasActiveSubscription: Bool
    public let hasConfiguredProvider: Bool
    public let needsProviderSelection: Bool
    public let subscriptions: [FabushiMagpieSubscriptionDTO]
    public let remoteRoutingGroups: [FabushiMagpieRoutingGroupDTO]
    public let routingStrategy: String
    public let masking: FabushiMagpieMaskingConfigDTO?
    public let recentRouteTraces: [FabushiMagpieRouteTraceDTO]

    public var isRemotePayload: Bool {
        presetsCount == nil && localUsage == nil
    }

    public var effectiveGroups: [FabushiMagpieRoutingGroupDTO] {
        remoteRoutingGroups.isEmpty ? localGroups : remoteRoutingGroups
    }

    public enum CodingKeys: String, CodingKey {
        case presetsCount = "presets_count"
        case localGroups = "groups"
        case localUsage = "usage"
        case presets
        case providers
        case activeProvider = "active_provider"
        case activeModel = "active_model"
        case allowLocalFallback = "allow_local_fallback"
        case hasActiveSubscription = "has_active_subscription"
        case hasConfiguredProvider = "has_configured_provider"
        case needsProviderSelection = "needs_provider_selection"
        case subscriptions
        case remoteRoutingGroups = "routing_groups"
        case routingStrategy = "routing_strategy"
        case masking
        case recentRouteTraces = "recent_route_traces"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.presetsCount = try? c.decodeIfPresent(Int.self, forKey: .presetsCount)
        self.localGroups = (try? c.decodeIfPresent([FabushiMagpieRoutingGroupDTO].self, forKey: .localGroups)) ?? []
        self.localUsage = try? c.decodeIfPresent(FabushiMagpieUsageSummaryDTO.self, forKey: .localUsage)

        self.presets = (try? c.decodeIfPresent([FabushiMagpiePresetDTO].self, forKey: .presets)) ?? []
        self.providers = (try? c.decodeIfPresent([FabushiMagpieProviderDTO].self, forKey: .providers)) ?? []
        self.activeProvider = (try? c.decodeIfPresent(String.self, forKey: .activeProvider)) ?? ""
        self.activeModel = (try? c.decodeIfPresent(String.self, forKey: .activeModel)) ?? ""
        self.allowLocalFallback = (try? c.decodeIfPresent(Bool.self, forKey: .allowLocalFallback)) ?? false
        self.hasActiveSubscription = (try? c.decodeIfPresent(Bool.self, forKey: .hasActiveSubscription)) ?? false
        self.hasConfiguredProvider = (try? c.decodeIfPresent(Bool.self, forKey: .hasConfiguredProvider)) ?? false
        self.needsProviderSelection = (try? c.decodeIfPresent(Bool.self, forKey: .needsProviderSelection)) ?? false
        self.subscriptions = (try? c.decodeIfPresent([FabushiMagpieSubscriptionDTO].self, forKey: .subscriptions)) ?? []
        self.remoteRoutingGroups = (try? c.decodeIfPresent([FabushiMagpieRoutingGroupDTO].self, forKey: .remoteRoutingGroups)) ?? []
        self.routingStrategy = (try? c.decodeIfPresent(String.self, forKey: .routingStrategy)) ?? "smart"
        self.masking = try? c.decodeIfPresent(FabushiMagpieMaskingConfigDTO.self, forKey: .masking)
        self.recentRouteTraces = (try? c.decodeIfPresent([FabushiMagpieRouteTraceDTO].self, forKey: .recentRouteTraces)) ?? []
    }
}

// MARK: - Zero-Knowledge Vault AI OAuth Record (`USER_VAULT_CATEGORY_AI_OAUTH == "ai_oauth"`)

public struct FabushiMagpieVaultOAuthRecord: Codable, Sendable, Equatable, Identifiable {
    public let itemKey: String
    public let providerId: String
    public let providerName: String
    public let subscriptionKind: String?
    public let protocolKind: String
    public let baseUrl: String?
    public let defaultModel: String
    public let models: [String]
    public let email: String?
    public let maskedTokenPreview: String
    public let revision: Int64
    public let updatedAtMs: Int64

    public var id: String { itemKey }

    public enum CodingKeys: String, CodingKey {
        case itemKey = "item_key"
        case providerId = "provider_id"
        case providerName = "provider_name"
        case subscriptionKind = "subscription_kind"
        case protocolKind = "protocol"
        case baseUrl = "base_url"
        case defaultModel = "default_model"
        case models
        case email
        case maskedTokenPreview = "masked_token_preview"
        case revision
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        itemKey: String,
        providerId: String,
        providerName: String,
        subscriptionKind: String? = nil,
        protocolKind: String = FabushiMagpieProtocolKind.chatCompletions.rawValue,
        baseUrl: String? = nil,
        defaultModel: String = "",
        models: [String] = [],
        email: String? = nil,
        maskedTokenPreview: String = "",
        revision: Int64 = 1,
        updatedAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000)
    ) {
        self.itemKey = itemKey
        self.providerId = providerId
        self.providerName = providerName
        self.subscriptionKind = subscriptionKind
        self.protocolKind = protocolKind
        self.baseUrl = baseUrl
        self.defaultModel = defaultModel
        self.models = models
        self.email = email
        self.maskedTokenPreview = maskedTokenPreview
        self.revision = revision
        self.updatedAtMs = updatedAtMs
    }

    /// Masks any secret value before display so plaintext tokens are never rendered in list rows.
    public static func maskSecretForDisplay(_ secret: String) -> String {
        let trimmed = secret.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        if trimmed.count <= 6 {
            return "***(\(trimmed.count)c)"
        }
        return "\(trimmed.prefix(3))***\(trimmed.suffix(2)) (\(trimmed.count)c)"
    }
}
