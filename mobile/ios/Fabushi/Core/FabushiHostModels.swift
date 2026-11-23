import Foundation
import Observation
import Security

// MARK: - Host Configuration (Exact M1b source/host/src/host.rs:55-66 Schema)

public struct FabushiHostConfig: Codable, Sendable, Equatable {
    public static let defaultServerURL = "http://34.46.77.133:8080"
    public static let defaultHttpTimeoutSecs: UInt64 = 10

    public var appDataDir: String?
    public var storagePassphrase: String?
    public var serverUrl: String?
    public var httpTimeoutSecs: UInt64?
    public var testMode: Bool
    public var iosSandboxStrict: Bool?

    public enum CodingKeys: String, CodingKey {
        case appDataDir = "app_data_dir"
        case storagePassphrase = "storage_passphrase"
        case serverUrl = "server_url"
        case httpTimeoutSecs = "http_timeout_secs"
        case testMode = "test_mode"
        case iosSandboxStrict = "ios_sandbox_strict"
    }

    public init(
        appDataDir: String? = nil,
        storagePassphrase: String? = nil,
        serverUrl: String? = nil,
        httpTimeoutSecs: UInt64? = defaultHttpTimeoutSecs,
        testMode: Bool = false,
        iosSandboxStrict: Bool? = true
    ) {
        self.appDataDir = appDataDir
        self.storagePassphrase = storagePassphrase
        self.serverUrl = serverUrl
        self.httpTimeoutSecs = httpTimeoutSecs
        self.testMode = testMode
        self.iosSandboxStrict = iosSandboxStrict
    }
}

// MARK: - Wired 33 Rust JSON-RPC Namespaces

public enum FabushiRPCNamespace: String, CaseIterable, Sendable, Hashable {
    case auth
    case vault
    case session
    case chats
    case folders
    case topics
    case contacts
    case messages
    case sync
    case outbox
    case e2ee
    case search
    case media
    case stories
    case stickers
    case fabushi
    case multiAgent = "multi_agent"
    case magpie
    case coding
    case mcp
    case computers
    case remote
    case tailnet
    case miniapp
    case webmcp
    case calls
    case talk
    case wallet
    case settings
    case proxy
    case push
    case actions
    case generation

    public func method(_ action: String) -> String {
        "\(rawValue).\(action)"
    }
}

// MARK: - Sendable JSON Value & Empty Params

public struct FabushiEmptyParams: Codable, Sendable, Equatable {
    public init() {}
}

public enum FabushiJSONValue: Codable, Sendable, Equatable {
    case null
    case bool(Bool)
    case int(Int64)
    case double(Double)
    case string(String)
    case array([FabushiJSONValue])
    case object([String: FabushiJSONValue])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let b = try? container.decode(Bool.self) {
            self = .bool(b)
        } else if let i = try? container.decode(Int64.self) {
            self = .int(i)
        } else if let u = try? container.decode(UInt64.self) {
            if u <= UInt64(Int64.max) {
                self = .int(Int64(u))
            } else {
                self = .double(Double(u))
            }
        } else if let d = try? container.decode(Double.self) {
            self = .double(d)
        } else if let s = try? container.decode(String.self) {
            self = .string(s)
        } else if let a = try? container.decode([FabushiJSONValue].self) {
            self = .array(a)
        } else if let o = try? container.decode([String: FabushiJSONValue].self) {
            self = .object(o)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Unsupported JSON value"
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case .bool(let b):
            try container.encode(b)
        case .int(let i):
            try container.encode(i)
        case .double(let d):
            try container.encode(d)
        case .string(let s):
            try container.encode(s)
        case .array(let a):
            try container.encode(a)
        case .object(let o):
            try container.encode(o)
        }
    }

    public var boolValue: Bool? {
        if case .bool(let v) = self { return v }
        return nil
    }

    public var int64Value: Int64? {
        switch self {
        case .int(let v): return v
        case .double(let v): return Int64(v)
        default: return nil
        }
    }

    public var uint64Value: UInt64? {
        switch self {
        case .int(let v) where v >= 0: return UInt64(v)
        case .double(let v) where v >= 0: return UInt64(v)
        default: return nil
        }
    }

    public var doubleValue: Double? {
        switch self {
        case .double(let v): return v
        case .int(let v): return Double(v)
        default: return nil
        }
    }

    public var stringValue: String? {
        if case .string(let v) = self { return v }
        return nil
    }

    public var arrayValue: [FabushiJSONValue]? {
        if case .array(let v) = self { return v }
        return nil
    }

    public var objectValue: [String: FabushiJSONValue]? {
        if case .object(let v) = self { return v }
        return nil
    }

    public subscript(key: String) -> FabushiJSONValue? {
        objectValue?[key]
    }

    public func toJSONString() -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        guard let data = try? encoder.encode(self),
              let string = String(data: data, encoding: .utf8)
        else {
            return "null"
        }
        return string
    }
}

// MARK: - JSON-RPC Envelopes

public struct FabushiRPCRequest<Params: Encodable & Sendable>: Encodable, Sendable {
    public let id: String
    public let method: String
    public let generation: UInt64?
    public let accountId: String?
    public let params: Params

    public enum CodingKeys: String, CodingKey {
        case id
        case method
        case generation
        case accountId = "account_id"
        case params
    }

    public init(
        id: String,
        method: String,
        generation: UInt64? = nil,
        accountId: String? = nil,
        params: Params
    ) {
        self.id = id
        self.method = method
        self.generation = generation
        self.accountId = accountId
        self.params = params
    }
}

public struct FabushiRPCErrorPayload: Codable, Sendable, Equatable {
    public let code: String
    public let message: String
    public let retryable: Bool

    public enum CodingKeys: String, CodingKey {
        case code
        case message
        case retryable
    }

    public init(code: String, message: String, retryable: Bool = false) {
        self.code = code
        self.message = message
        self.retryable = retryable
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.code = try container.decodeIfPresent(String.self, forKey: .code) ?? "internal_error"
        self.message = try container.decodeIfPresent(String.self, forKey: .message) ?? "Unknown RPC error"
        self.retryable = try container.decodeIfPresent(Bool.self, forKey: .retryable) ?? false
    }
}

public struct FabushiRPCResponse<Result: Decodable & Sendable>: Decodable, Sendable {
    public let id: String?
    public let ok: Bool
    public let generation: UInt64?
    public let result: Result?
    public let error: FabushiRPCErrorPayload?
}

public struct FabushiRPCEnvelopeResult<Result: Sendable>: Sendable {
    public let id: String
    public let generation: UInt64
    public let result: Result

    public init(id: String, generation: UInt64, result: Result) {
        self.id = id
        self.generation = generation
        self.result = result
    }
}

// MARK: - Typed Fail-Closed Error Taxonomy (Matches source/host/src/error.rs)

public enum FabushiHostError: LocalizedError, Sendable, Equatable {
    case notInitialized
    case initializationFailed(message: String)
    case serializationFailed(message: String)
    case invalidResponse(raw: String)
    case unknownCommand(method: String, message: String)
    case staleGeneration(requested: UInt64?, current: UInt64, message: String)
    case invalidArguments(message: String)
    case capabilityUnavailable(message: String)
    case upstreamNotImplemented(message: String)
    case cancelled(requestId: String, message: String)
    case rustPanic(message: String)
    case hostClosed(message: String)
    case unauthorized(message: String)
    case permissionDenied(message: String)
    case notFound(message: String)
    case conflict(message: String)
    case rateLimited(message: String, retryable: Bool)
    case timeout(message: String, retryable: Bool)
    case networkError(message: String, retryable: Bool)
    case cryptoError(message: String)
    case storageError(message: String)
    case keychainFailed(status: OSStatus)
    case rpcError(code: String, message: String, retryable: Bool)

    public static func fromRPCError(
        _ payload: FabushiRPCErrorPayload,
        method: String,
        requestId: String,
        requestedGeneration: UInt64?,
        responseGeneration: UInt64
    ) -> FabushiHostError {
        let normalizedCode = payload.code.lowercased()
        switch normalizedCode {
        case "unknown_command":
            return .unknownCommand(method: method, message: payload.message)
        case "stale_generation":
            return .staleGeneration(
                requested: requestedGeneration,
                current: responseGeneration,
                message: payload.message
            )
        case "invalid_arguments", "invalid_json", "invalid_utf8":
            return .invalidArguments(message: payload.message)
        case "capability_unavailable":
            return .capabilityUnavailable(message: payload.message)
        case "upstream_not_implemented":
            return .upstreamNotImplemented(message: payload.message)
        case "cancelled":
            return .cancelled(requestId: requestId, message: payload.message)
        case "host_panic":
            return .rustPanic(message: payload.message)
        case "host_closed", "handle_required":
            return .hostClosed(message: payload.message)
        case "unauthorized":
            return .unauthorized(message: payload.message)
        case "permission_denied":
            return .permissionDenied(message: payload.message)
        case "not_found":
            return .notFound(message: payload.message)
        case "conflict":
            return .conflict(message: payload.message)
        case "rate_limited":
            return .rateLimited(message: payload.message, retryable: payload.retryable)
        case "timeout":
            return .timeout(message: payload.message, retryable: payload.retryable)
        case "network_error", "proxy_error":
            return .networkError(message: payload.message, retryable: payload.retryable)
        case "crypto_error":
            return .cryptoError(message: payload.message)
        case "storage_error":
            return .storageError(message: payload.message)
        default:
            return .rpcError(code: normalizedCode, message: payload.message, retryable: payload.retryable)
        }
    }

    public var code: String {
        switch self {
        case .notInitialized: return "not_initialized"
        case .initializationFailed: return "initialization_failed"
        case .serializationFailed: return "serialization_error"
        case .invalidResponse: return "invalid_response"
        case .unknownCommand: return "unknown_command"
        case .staleGeneration: return "stale_generation"
        case .invalidArguments: return "invalid_arguments"
        case .capabilityUnavailable: return "capability_unavailable"
        case .upstreamNotImplemented: return "upstream_not_implemented"
        case .cancelled: return "cancelled"
        case .rustPanic: return "host_panic"
        case .hostClosed: return "host_closed"
        case .unauthorized: return "unauthorized"
        case .permissionDenied: return "permission_denied"
        case .notFound: return "not_found"
        case .conflict: return "conflict"
        case .rateLimited: return "rate_limited"
        case .timeout: return "timeout"
        case .networkError: return "network_error"
        case .cryptoError: return "crypto_error"
        case .storageError: return "storage_error"
        case .keychainFailed: return "keychain_error"
        case .rpcError(let code, _, _): return code
        }
    }

    public var isCancelled: Bool {
        if case .cancelled = self { return true }
        return false
    }

    public var isRetryable: Bool {
        switch self {
        case .rateLimited(_, let retryable),
             .timeout(_, let retryable),
             .networkError(_, let retryable),
             .rpcError(_, _, let retryable):
            return retryable
        default:
            return false
        }
    }

    public var errorDescription: String? {
        switch self {
        case .notInitialized:
            return "Fabushi Rust host is not initialized."
        case .initializationFailed(let msg):
            return "Fabushi Rust host initialization failed: \(msg)"
        case .serializationFailed(let msg):
            return "Failed to serialize JSON-RPC request: \(msg)"
        case .invalidResponse(let raw):
            return "Invalid JSON-RPC response from Rust host: \(raw)"
        case .unknownCommand(let method, let msg):
            return "Unknown command '\(method)': \(msg)"
        case .staleGeneration(_, let current, let msg):
            return "Stale generation (current: \(current)): \(msg)"
        case .invalidArguments(let msg):
            return "Invalid arguments: \(msg)"
        case .capabilityUnavailable(let msg):
            return "Capability unavailable on iOS: \(msg)"
        case .upstreamNotImplemented(let msg):
            return "Upstream not implemented: \(msg)"
        case .cancelled(_, let msg):
            return "Request cancelled: \(msg)"
        case .rustPanic(let msg):
            return "Rust host panic recovered safely: \(msg)"
        case .hostClosed(let msg):
            return "Fabushi Rust host is closed: \(msg)"
        case .unauthorized(let msg):
            return "Unauthorized: \(msg)"
        case .permissionDenied(let msg):
            return "Permission denied: \(msg)"
        case .notFound(let msg):
            return "Not found: \(msg)"
        case .conflict(let msg):
            return "Conflict: \(msg)"
        case .rateLimited(let msg, _):
            return "Rate limited: \(msg)"
        case .timeout(let msg, _):
            return "Timed out: \(msg)"
        case .networkError(let msg, _):
            return "Network error: \(msg)"
        case .cryptoError(let msg):
            return "Crypto error: \(msg)"
        case .storageError(let msg):
            return "Storage error: \(msg)"
        case .keychainFailed(let status):
            return "iOS Keychain access failed (OSStatus: \(status))."
        case .rpcError(let code, let msg, _):
            return "[\(code)] \(msg)"
        }
    }
}

// MARK: - Canonical Core RPC DTOs

public struct FabushiPingResult: Codable, Sendable, Equatable {
    public let pong: Bool
    public let nonce: UInt64
    public let timestamp: Int64
    public let serverUrl: String
    public let generation: UInt64

    public enum CodingKeys: String, CodingKey {
        case pong
        case nonce
        case timestamp
        case serverUrl = "server_url"
        case generation
    }
}

public struct FabushiGenerationResult: Codable, Sendable, Equatable {
    public let generation: UInt64
}

public struct FabushiGenerationValidationResult: Codable, Sendable, Equatable {
    public let valid: Bool
    public let requested: UInt64
    public let current: UInt64
}

public struct FabushiWalCheckpointSummary: Codable, Sendable, Equatable {
    public let checkpointed: Bool
    public let busy: Int32
    public let logFrames: Int32
    public let checkpointedFrames: Int32

    public enum CodingKeys: String, CodingKey {
        case checkpointed
        case busy
        case logFrames = "log_frames"
        case checkpointedFrames = "checkpointed_frames"
    }
}

public struct FabushiSessionAuthParams: Codable, Sendable, Equatable {
    public let userId: UInt64
    public let username: String
    public let sessionId: String
    public let token: String
    public let firstName: String?
    public let createdAt: String?
    public let expiresAt: String?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case sessionId = "session_id"
        case token
        case firstName = "first_name"
        case createdAt = "created_at"
        case expiresAt = "expires_at"
    }

    public init(
        userId: UInt64,
        username: String,
        sessionId: String,
        token: String,
        firstName: String? = nil,
        createdAt: String? = nil,
        expiresAt: String? = nil
    ) {
        self.userId = userId
        self.username = username
        self.sessionId = sessionId
        self.token = token
        self.firstName = firstName
        self.createdAt = createdAt
        self.expiresAt = expiresAt
    }
}

public struct FabushiSessionAuthResult: Codable, Sendable, Equatable {
    public let userId: UInt64
    public let generation: UInt64

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case generation
    }
}

public struct FabushiClearSessionResult: Codable, Sendable, Equatable {
    public let cleared: Bool
    public let generation: UInt64
}

public struct FabushiLogoutResult: Codable, Sendable, Equatable {
    public let loggedOut: Bool
    public let generation: UInt64

    public enum CodingKeys: String, CodingKey {
        case loggedOut = "logged_out"
        case generation
    }
}

public struct FabushiSwitchAccountResult: Codable, Sendable, Equatable {
    public let switched: Bool
    public let accountId: String
    public let userId: UInt64
    public let generation: UInt64

    public enum CodingKeys: String, CodingKey {
        case switched
        case accountId = "account_id"
        case userId = "user_id"
        case generation
    }
}

public struct FabushiAccountSummary: Codable, Sendable, Equatable, Identifiable {
    public let accountId: String
    public let userId: UInt64?
    public let username: String?
    public let firstName: String?
    public let active: Bool

    public var id: String { accountId }

    public enum CodingKeys: String, CodingKey {
        case accountId = "account_id"
        case userId = "user_id"
        case username
        case firstName = "first_name"
        case active
    }
}

public struct FabushiListAccountsResult: Codable, Sendable, Equatable {
    public let accounts: [FabushiAccountSummary]
    public let activeAccountId: String?

    public enum CodingKeys: String, CodingKey {
        case accounts
        case activeAccountId = "active_account_id"
    }
}

public struct FabushiServerUrlResult: Codable, Sendable, Equatable {
    public let serverUrl: String
    public let generation: UInt64

    public enum CodingKeys: String, CodingKey {
        case serverUrl = "server_url"
        case generation
    }
}

public struct FabushiGetServerUrlResult: Codable, Sendable, Equatable {
    public let serverUrl: String
    public let defaultServerUrl: String

    public enum CodingKeys: String, CodingKey {
        case serverUrl = "server_url"
        case defaultServerUrl = "default_server_url"
    }
}

public struct FabushiNetworkStateResult: Codable, Sendable, Equatable {
    public let online: Bool
}

public struct FabushiOutboxPendingCountResult: Codable, Sendable, Equatable {
    public let pendingCount: Int
    public let networkOnline: Bool

    public enum CodingKeys: String, CodingKey {
        case pendingCount = "pending_count"
        case networkOnline = "network_online"
    }
}

public struct FabushiWsStatusResult: Codable, Sendable, Equatable {
    public let connected: Bool
    public let status: String
    public let serverUrl: String
    public let deviceId: String
    public let userId: UInt64?

    public enum CodingKeys: String, CodingKey {
        case connected
        case status
        case serverUrl = "server_url"
        case deviceId = "device_id"
        case userId = "user_id"
    }
}

// MARK: - Coordinator & Shell State Models

public enum FabushiAuthState: Sendable, Equatable {
    case unauthenticated
    case authenticated(userId: UInt64, username: String, accountId: String)
}

public enum ShellRoute: Sendable, Equatable, Hashable {
    case chats(chatId: Int64?, messageId: Int64?)
    case compose
    case agentStudio(tab: String?)
    case calls
    case remoteComputer(nodeId: String?)
    case miniApp(id: String?)
    case settings(section: String?)
}

// MARK: - Canonical @MainActor @Observable Domain Store Anchors (PROJECT.md Contract #2)

@MainActor
@Observable
public final class ChatStore {
    public private(set) var lastObservedGeneration: UInt64 = 1
    public private(set) var lastMessageSeq: UInt64 = 0
    public private(set) var lastSyncGapChatId: Int64? = nil
    public private(set) var lastSyncGapFromPts: UInt64? = nil
    public private(set) var pendingOutboxEventsCount: Int = 0

    public init() {}

    public func handleEvent(_ event: FabushiDomainEvent) {
        switch event.kind {
        case .newMessage, .editMessage, .deleteMessages, .readHistory, .reactionUpdated:
            lastMessageSeq = max(lastMessageSeq, event.seq)
        case .syncGapDetected(let chatId, let fromPts):
            lastSyncGapChatId = chatId
            lastSyncGapFromPts = fromPts
        case .outboxStatusChanged:
            pendingOutboxEventsCount += 1
        default:
            break
        }
    }

    public func resetForAccountScopeChange(generation: UInt64) {
        lastObservedGeneration = generation
        lastMessageSeq = 0
        lastSyncGapChatId = nil
        lastSyncGapFromPts = nil
        pendingOutboxEventsCount = 0
    }
}

@MainActor
@Observable
public final class AgentStudioStore {
    public private(set) var lastObservedGeneration: UInt64 = 1
    public private(set) var taskStatesById: [String: String] = [:]
    public private(set) var activeFloorSpeakerByChat: [Int64: String] = [:]

    public init() {}

    public func handleEvent(_ event: FabushiDomainEvent) {
        switch event.kind {
        case .taskStateUpdated(let taskId, let state):
            taskStatesById[taskId] = state
        case .agentFloorChanged(let chatId, let currentSpeaker, _, _):
            if let currentSpeaker {
                activeFloorSpeakerByChat[chatId] = currentSpeaker
            } else {
                activeFloorSpeakerByChat.removeValue(forKey: chatId)
            }
        default:
            break
        }
    }

    public func resetForAccountScopeChange(generation: UInt64) {
        lastObservedGeneration = generation
        taskStatesById.removeAll()
        activeFloorSpeakerByChat.removeAll()
    }
}

@MainActor
@Observable
public final class RemoteComputerStore {
    public private(set) var lastObservedGeneration: UInt64 = 1
    public private(set) var lastEmergencyOverrideEpoch: UInt64? = nil
    public private(set) var lastDisconnectedSessionsCount: Int = 0

    public init() {}

    public func handleEvent(_ event: FabushiDomainEvent) {
        if case .remoteEmergencyTakeover(let disconnected, let epoch, _) = event.kind {
            lastEmergencyOverrideEpoch = epoch
            lastDisconnectedSessionsCount = disconnected
        }
    }

    public func resetForAccountScopeChange(generation: UInt64) {
        lastObservedGeneration = generation
        lastEmergencyOverrideEpoch = nil
        lastDisconnectedSessionsCount = 0
    }
}

@MainActor
@Observable
public final class MiniAppHostStore {
    public private(set) var lastObservedGeneration: UInt64 = 1
    public private(set) var activeControlMode: String? = nil
    public private(set) var activeAppTitle: String? = nil

    public init() {}

    public func handleEvent(_ event: FabushiDomainEvent) {
        if case .miniAppControlStatus(_, _, _, _, let appTitle, let mode, _, _) = event.kind {
            activeAppTitle = appTitle
            activeControlMode = mode
        }
    }

    public func resetForAccountScopeChange(generation: UInt64) {
        lastObservedGeneration = generation
        activeControlMode = nil
        activeAppTitle = nil
    }
}

@MainActor
@Observable
public final class CallSessionStore {
    public private(set) var lastObservedGeneration: UInt64 = 1
    public private(set) var activeCallId: String? = nil
    public private(set) var activeCallState: String? = nil

    public init() {}

    public func handleEvent(_ event: FabushiDomainEvent) {
        if case .callUpdated(let callId, let state, _) = event.kind {
            activeCallId = callId
            activeCallState = state
        }
    }

    public func resetForAccountScopeChange(generation: UInt64) {
        lastObservedGeneration = generation
        activeCallId = nil
        activeCallState = nil
    }
}

@MainActor
@Observable
public final class SettingsStore {
    public private(set) var lastObservedGeneration: UInt64 = 1
    public private(set) var activeServerURL: String = FabushiHostConfig.defaultServerURL
    public private(set) var wsConnectionStatus: String = "Disconnected"

    public init() {}

    public func updateServerURL(_ url: String) {
        activeServerURL = url
    }

    public func handleEvent(_ event: FabushiDomainEvent) {
        if case .connectionStateChanged(let status, let serverUrl) = event.kind {
            wsConnectionStatus = status
            if let serverUrl, !serverUrl.isEmpty {
                activeServerURL = serverUrl
            }
        }
    }

    public func resetForAccountScopeChange(generation: UInt64) {
        lastObservedGeneration = generation
        wsConnectionStatus = "Disconnected"
    }
}

@MainActor
@Observable
public final class VaultStore {
    public private(set) var lastObservedGeneration: UInt64 = 1
    public private(set) var isUnlocked: Bool = false

    public init() {}

    public func setUnlocked(_ unlocked: Bool) {
        isUnlocked = unlocked
    }

    public func resetForAccountScopeChange(generation: UInt64) {
        lastObservedGeneration = generation
        isUnlocked = false
    }
}
