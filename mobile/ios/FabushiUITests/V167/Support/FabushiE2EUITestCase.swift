import Foundation
import UIKit
import XCTest

// MARK: - Top-Level Nonisolated Codable & Sendable Loopback DTOs

enum FabushiE2EFormFactor: String, Codable, Sendable {
    case iphone
    case ipad
}

struct FabushiE2ELaunchConfig: Sendable {
    var theme: String
    var accentHex: String?
    var languageCode: String
    var localeIdentifier: String
    var preferredContentSizeCategory: String?
    var reduceMotion: Bool
    var highContrast: Bool
    var resetState: Bool
    var extraEnvironment: [String: String]
    var extraArguments: [String]

    init(
        theme: String = "night",
        accentHex: String? = nil,
        languageCode: String = "zh-Hans",
        localeIdentifier: String = "zh_CN",
        preferredContentSizeCategory: String? = nil,
        reduceMotion: Bool = false,
        highContrast: Bool = false,
        resetState: Bool = true,
        extraEnvironment: [String: String] = [:],
        extraArguments: [String] = []
    ) {
        self.theme = theme
        self.accentHex = accentHex
        self.languageCode = languageCode
        self.localeIdentifier = localeIdentifier
        self.preferredContentSizeCategory = preferredContentSizeCategory
        self.reduceMotion = reduceMotion
        self.highContrast = highContrast
        self.resetState = resetState
        self.extraEnvironment = extraEnvironment
        self.extraArguments = extraArguments
    }
}

struct FabushiLoopbackHealth: Codable, Sendable {
    let ok: Bool
    let version: String?
    let tsMs: Int64?

    enum CodingKeys: String, CodingKey {
        case ok
        case version
        case tsMs = "ts_ms"
    }
}

struct FabushiLoopbackResetResponse: Codable, Sendable {
    let ok: Bool
    let reset: Bool
    let tsMs: Int64

    enum CodingKeys: String, CodingKey {
        case ok
        case reset
        case tsMs = "ts_ms"
    }
}

struct FabushiLoopbackSeedUser: Codable, Sendable {
    let username: String
    let firstName: String?
    let password: String?
    let authVersion: Int?
    let authSaltB64: String?
    let authVerifierB64: String?
    let twoFactorPassword: String?
    let twoFactorHint: String?
    let mintSession: Bool?
    let deviceName: String?

    enum CodingKeys: String, CodingKey {
        case username
        case firstName = "first_name"
        case password
        case authVersion = "auth_version"
        case authSaltB64 = "auth_salt_b64"
        case authVerifierB64 = "auth_verifier_b64"
        case twoFactorPassword = "two_factor_password"
        case twoFactorHint = "two_factor_hint"
        case mintSession = "mint_session"
        case deviceName = "device_name"
    }

    init(
        username: String,
        firstName: String? = nil,
        password: String? = "FabushiV168!Pass",
        authVersion: Int? = nil,
        authSaltB64: String? = nil,
        authVerifierB64: String? = nil,
        twoFactorPassword: String? = nil,
        twoFactorHint: String? = nil,
        mintSession: Bool? = true,
        deviceName: String? = "iOS E2E Simulator"
    ) {
        self.username = username
        self.firstName = firstName
        self.password = authVerifierB64 != nil ? nil : password
        self.authVersion = authVersion ?? (authVerifierB64 != nil ? 2 : 1)
        self.authSaltB64 = authSaltB64
        self.authVerifierB64 = authVerifierB64
        self.twoFactorPassword = twoFactorPassword
        self.twoFactorHint = twoFactorHint
        self.mintSession = mintSession
        self.deviceName = deviceName
    }

    static func splitKeyV2(
        username: String,
        firstName: String? = nil,
        authSaltB64: String = "QkJCQkJCQkJCQkJCQkJCQg==",
        authVerifierB64: String,
        twoFactorPassword: String? = nil,
        twoFactorHint: String? = nil,
        mintSession: Bool? = true,
        deviceName: String? = "iOS E2E Simulator"
    ) -> FabushiLoopbackSeedUser {
        FabushiLoopbackSeedUser(
            username: username,
            firstName: firstName,
            password: nil,
            authVersion: 2,
            authSaltB64: authSaltB64,
            authVerifierB64: authVerifierB64,
            twoFactorPassword: twoFactorPassword,
            twoFactorHint: twoFactorHint,
            mintSession: mintSession,
            deviceName: deviceName
        )
    }
}

struct FabushiLoopbackSeedChat: Codable, Sendable {
    let chatId: Int?
    let title: String
    let chatType: String?
    let memberCount: Int?
    let pinned: Bool?
    let unreadCount: Int?
    let isUnreadMarked: Bool?
    let botStarted: Bool?

    enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case title
        case chatType = "chat_type"
        case memberCount = "member_count"
        case pinned
        case unreadCount = "unread_count"
        case isUnreadMarked = "is_unread_marked"
        case botStarted = "bot_started"
    }

    init(
        chatId: Int? = nil,
        title: String,
        chatType: String? = "group",
        memberCount: Int? = 2,
        pinned: Bool? = nil,
        unreadCount: Int? = nil,
        isUnreadMarked: Bool? = nil,
        botStarted: Bool? = nil
    ) {
        self.chatId = chatId
        self.title = title
        self.chatType = chatType
        self.memberCount = memberCount
        self.pinned = pinned
        self.unreadCount = unreadCount
        self.isUnreadMarked = isUnreadMarked
        self.botStarted = botStarted
    }
}

struct FabushiLoopbackSeedMessage: Codable, Sendable {
    let chatId: Int
    let text: String
    let clientNonce: Int?
    let messageType: String?
    let mediaFileId: String?
    let instantViewUrl: String?

    enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case text
        case clientNonce = "client_nonce"
        case messageType = "message_type"
        case mediaFileId = "media_file_id"
        case instantViewUrl = "instant_view_url"
    }

    init(
        chatId: Int,
        text: String,
        clientNonce: Int? = nil,
        messageType: String? = nil,
        mediaFileId: String? = nil,
        instantViewUrl: String? = nil
    ) {
        self.chatId = chatId
        self.text = text
        self.clientNonce = clientNonce
        self.messageType = messageType
        self.mediaFileId = mediaFileId
        self.instantViewUrl = instantViewUrl
    }
}

struct FabushiLoopbackSeedMediaBlob: Codable, Sendable {
    let fileId: String
    let dataB64: String

    enum CodingKeys: String, CodingKey {
        case fileId = "file_id"
        case dataB64 = "data_b64"
    }
}

struct FabushiLoopbackSeedVaultItem: Codable, Sendable {
    let category: String
    let itemKey: String
    let ciphertextB64: String
    let nonceB64: String

    enum CodingKeys: String, CodingKey {
        case category
        case itemKey = "item_key"
        case ciphertextB64 = "ciphertext_b64"
        case nonceB64 = "nonce_b64"
    }
}

struct FabushiLoopbackSeedRemoteNode: Codable, Sendable {
    let deviceId: String
    let name: String
    let online: Bool
    let screenCapturePermission: String
    let accessibilityDragPermission: String

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case name
        case online
        case screenCapturePermission = "screen_capture_permission"
        case accessibilityDragPermission = "accessibility_drag_permission"
    }

    init(
        deviceId: String,
        name: String,
        online: Bool = true,
        screenCapturePermission: String = "granted_active",
        accessibilityDragPermission: String = "granted_active"
    ) {
        self.deviceId = deviceId
        self.name = name
        self.online = online
        self.screenCapturePermission = screenCapturePermission
        self.accessibilityDragPermission = accessibilityDragPermission
    }
}

struct FabushiLoopbackSeedMiniApp: Codable, Sendable {
    let appId: String
    let title: String
    let tosAccepted: Bool

    enum CodingKeys: String, CodingKey {
        case appId = "app_id"
        case title
        case tosAccepted = "tos_accepted"
    }

    init(appId: String, title: String, tosAccepted: Bool = false) {
        self.appId = appId
        self.title = title
        self.tosAccepted = tosAccepted
    }
}

struct FabushiLoopbackSeedAlipayConfirm: Codable, Sendable {
    let attemptId: String
    let userId: Int?

    enum CodingKeys: String, CodingKey {
        case attemptId = "attempt_id"
        case userId = "user_id"
    }

    init(attemptId: String, userId: Int? = nil) {
        self.attemptId = attemptId
        self.userId = userId
    }
}

struct FabushiLiveControlProbe: Sendable {
    let controlIdentifier: String
    let expectedElementIdentifier: String
    let dismissIdentifier: String?
    let timeout: TimeInterval

    init(
        controlIdentifier: String,
        expectedElementIdentifier: String,
        dismissIdentifier: String? = nil,
        timeout: TimeInterval = 8
    ) {
        self.controlIdentifier = controlIdentifier
        self.expectedElementIdentifier = expectedElementIdentifier
        self.dismissIdentifier = dismissIdentifier
        self.timeout = timeout
    }
}

struct FabushiLoopbackSeedRequest: Codable, Sendable {
    let users: [FabushiLoopbackSeedUser]?
    let chats: [FabushiLoopbackSeedChat]?
    let messages: [FabushiLoopbackSeedMessage]?
    let mediaBlobs: [FabushiLoopbackSeedMediaBlob]?
    let vaultItems: [FabushiLoopbackSeedVaultItem]?
    let journalFloors: [String: Int]?
    let remoteNodes: [FabushiLoopbackSeedRemoteNode]?
    let miniapps: [FabushiLoopbackSeedMiniApp]?
    let alipayConfirm: FabushiLoopbackSeedAlipayConfirm?

    enum CodingKeys: String, CodingKey {
        case users
        case chats
        case messages
        case mediaBlobs = "media_blobs"
        case vaultItems = "vault_items"
        case journalFloors = "journal_floors"
        case remoteNodes = "remote_nodes"
        case miniapps
        case alipayConfirm = "alipay_confirm"
    }

    init(
        users: [FabushiLoopbackSeedUser]? = nil,
        chats: [FabushiLoopbackSeedChat]? = nil,
        messages: [FabushiLoopbackSeedMessage]? = nil,
        mediaBlobs: [FabushiLoopbackSeedMediaBlob]? = nil,
        vaultItems: [FabushiLoopbackSeedVaultItem]? = nil,
        journalFloors: [String: Int]? = nil,
        remoteNodes: [FabushiLoopbackSeedRemoteNode]? = nil,
        miniapps: [FabushiLoopbackSeedMiniApp]? = nil,
        alipayConfirm: FabushiLoopbackSeedAlipayConfirm? = nil
    ) {
        self.users = users
        self.chats = chats
        self.messages = messages
        self.mediaBlobs = mediaBlobs
        self.vaultItems = vaultItems
        self.journalFloors = journalFloors
        self.remoteNodes = remoteNodes
        self.miniapps = miniapps
        self.alipayConfirm = alipayConfirm
    }
}

struct FabushiLoopbackSeededUserSummary: Codable, Sendable {
    let userId: Int
    let username: String
    let sessionId: String?
    let token: String?

    enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case sessionId = "session_id"
        case token
    }
}

struct FabushiLoopbackSeededChatSummary: Codable, Sendable {
    let chatId: Int
    let title: String

    enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case title
    }
}

struct FabushiLoopbackSeededMessageSummary: Codable, Sendable {
    let messageId: Int
    let chatId: Int
    let pts: Int

    enum CodingKeys: String, CodingKey {
        case messageId = "message_id"
        case chatId = "chat_id"
        case pts
    }
}

struct FabushiLoopbackSeedSummary: Codable, Sendable {
    let users: [FabushiLoopbackSeededUserSummary]
    let chats: [FabushiLoopbackSeededChatSummary]
    let messages: [FabushiLoopbackSeededMessageSummary]
    let mediaBlobIds: [String]
    let vaultItemCount: Int
    let remoteNodeIds: [String]
    let miniappIds: [String]

    enum CodingKeys: String, CodingKey {
        case users
        case chats
        case messages
        case mediaBlobIds = "media_blob_ids"
        case vaultItemCount = "vault_item_count"
        case remoteNodeIds = "remote_node_ids"
        case miniappIds = "miniapp_ids"
    }
}

struct FabushiLoopbackSeedResponse: Codable, Sendable {
    let ok: Bool
    let seeded: FabushiLoopbackSeedSummary
}

struct FabushiLoopbackHTTPFault: Codable, Sendable {
    let pathPrefix: String
    let method: String?
    let status: Int
    let body: [String: String]?
    let retryAfterMs: Int?
    let remainingHits: Int?

    enum CodingKeys: String, CodingKey {
        case pathPrefix = "path_prefix"
        case method
        case status
        case body
        case retryAfterMs = "retry_after_ms"
        case remainingHits = "remaining_hits"
    }

    init(
        pathPrefix: String,
        method: String? = nil,
        status: Int,
        body: [String: String]? = nil,
        retryAfterMs: Int? = nil,
        remainingHits: Int? = 1
    ) {
        self.pathPrefix = pathPrefix
        self.method = method
        self.status = status
        self.body = body
        self.retryAfterMs = retryAfterMs
        self.remainingHits = remainingHits
    }
}

struct FabushiLoopbackDelay: Codable, Sendable {
    let pathPrefix: String
    let delayMs: Int

    enum CodingKeys: String, CodingKey {
        case pathPrefix = "path_prefix"
        case delayMs = "delay_ms"
    }
}

struct FabushiLoopbackFaultRequest: Codable, Sendable {
    let clear: Bool?
    let offline: Bool?
    let httpFault: FabushiLoopbackHTTPFault?
    let httpFaults: [FabushiLoopbackHTTPFault]?
    let delay: FabushiLoopbackDelay?

    enum CodingKeys: String, CodingKey {
        case clear
        case offline
        case httpFault = "http_fault"
        case httpFaults = "http_faults"
        case delay
    }

    init(
        clear: Bool? = nil,
        offline: Bool? = nil,
        httpFault: FabushiLoopbackHTTPFault? = nil,
        httpFaults: [FabushiLoopbackHTTPFault]? = nil,
        delay: FabushiLoopbackDelay? = nil
    ) {
        self.clear = clear
        self.offline = offline
        self.httpFault = httpFault
        self.httpFaults = httpFaults
        self.delay = delay
    }
}

struct FabushiLoopbackFaultResponse: Codable, Sendable {
    let ok: Bool
    let offline: Bool
    let activeFaultCount: Int
    let activeDelayCount: Int

    enum CodingKeys: String, CodingKey {
        case ok
        case offline
        case activeFaultCount = "active_fault_count"
        case activeDelayCount = "active_delay_count"
    }
}

struct FabushiLoopbackRecordedRequest: Codable, Sendable {
    let method: String
    let path: String
    let query: String
    let bodySize: Int
    let bodyPreview: String
    let timestampMs: Int64

    enum CodingKeys: String, CodingKey {
        case method
        case path
        case query
        case bodySize = "body_size"
        case bodyPreview = "body_preview"
        case timestampMs = "timestamp_ms"
    }
}

struct FabushiLoopbackStateCounts: Codable, Sendable {
    let users: Int
    let activeSessions: Int
    let revokedTokens: Int
    let chats: Int
    let messages: Int
    let mediaBlobs: Int
    let vaultItems: Int
    let computerLeases: Int
    let remoteNodes: Int
    let miniapps: Int
    let tasks: Int

    enum CodingKeys: String, CodingKey {
        case users
        case activeSessions = "active_sessions"
        case revokedTokens = "revoked_tokens"
        case chats
        case messages
        case mediaBlobs = "media_blobs"
        case vaultItems = "vault_items"
        case computerLeases = "computer_leases"
        case remoteNodes = "remote_nodes"
        case miniapps
        case tasks
    }
}

struct FabushiLoopbackStateResponse: Codable, Sendable {
    let ok: Bool
    let version: String
    let offline: Bool
    let activeFaultCount: Int
    let recordedRequestCount: Int
    let matchedRequestCount: Int
    let recordedRequests: [FabushiLoopbackRecordedRequest]
    let counts: FabushiLoopbackStateCounts
    let chatIds: [Int]
    let activeSessionIds: [String]

    enum CodingKeys: String, CodingKey {
        case ok
        case version
        case offline
        case activeFaultCount = "active_fault_count"
        case recordedRequestCount = "recorded_request_count"
        case matchedRequestCount = "matched_request_count"
        case recordedRequests = "recorded_requests"
        case counts
        case chatIds = "chat_ids"
        case activeSessionIds = "active_session_ids"
    }
}

struct FabushiE2EStateEvidence: Codable, Sendable {
    let testName: String
    let formFactor: String
    let tier: String
    let sourceSHA: String
    let checkpoints: [String: Bool]
    let metadata: [String: String]

    enum CodingKeys: String, CodingKey {
        case testName = "test_name"
        case formFactor = "form_factor"
        case tier
        case sourceSHA = "source_sha"
        case checkpoints
        case metadata
    }
}

enum FabushiLoopbackClientError: Error, Sendable {
    case missingServerURL(String)
    case invalidEndpoint(String)
    case nonHTTPResponse
    case httpStatus(Int, String)
}

struct FabushiLoopbackClient: Sendable {
    let baseURL: URL

    private static let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 15
        config.timeoutIntervalForResource = 30
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: config)
    }()

    func checkHealth() async throws -> FabushiLoopbackHealth {
        try await sendRequest(method: "GET", path: "/healthz", bodyData: nil)
    }

    @discardableResult
    func resetState() async throws -> FabushiLoopbackResetResponse {
        let empty = Data("{}".utf8)
        return try await sendRequest(method: "POST", path: "/__e2e__/reset", bodyData: empty)
    }

    @discardableResult
    func seed(_ payload: FabushiLoopbackSeedRequest) async throws -> FabushiLoopbackSeedResponse {
        let encoder = JSONEncoder()
        let encoded = try encoder.encode(payload)
        return try await sendRequest(method: "POST", path: "/__e2e__/seed", bodyData: encoded)
    }

    @discardableResult
    func injectFault(_ payload: FabushiLoopbackFaultRequest) async throws -> FabushiLoopbackFaultResponse {
        let encoder = JSONEncoder()
        let encoded = try encoder.encode(payload)
        return try await sendRequest(method: "POST", path: "/__e2e__/fault", bodyData: encoded)
    }

    @discardableResult
    func clearFaults() async throws -> FabushiLoopbackFaultResponse {
        try await sendRequest(method: "DELETE", path: "/__e2e__/fault", bodyData: nil)
    }

    func fetchState(
        pathPrefix: String? = nil,
        includeBodies: Bool = true
    ) async throws -> FabushiLoopbackStateResponse {
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else {
            throw FabushiLoopbackClientError.invalidEndpoint(baseURL.absoluteString)
        }
        components.path = "/__e2e__/state"
        var queryItems = [
            URLQueryItem(name: "include_bodies", value: includeBodies ? "true" : "false")
        ]
        if let prefix = pathPrefix, !prefix.isEmpty {
            queryItems.append(URLQueryItem(name: "path_prefix", value: prefix))
        }
        components.queryItems = queryItems
        guard let url = components.url else {
            throw FabushiLoopbackClientError.invalidEndpoint("/__e2e__/state")
        }
        return try await execute(url: url, method: "GET", bodyData: nil)
    }

    private func sendRequest<Response: Decodable & Sendable>(
        method: String,
        path: String,
        bodyData: Data?
    ) async throws -> Response {
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else {
            throw FabushiLoopbackClientError.invalidEndpoint(baseURL.absoluteString)
        }
        components.path = path
        guard let url = components.url else {
            throw FabushiLoopbackClientError.invalidEndpoint(path)
        }
        return try await execute(url: url, method: method, bodyData: bodyData)
    }

    private func execute<Response: Decodable & Sendable>(
        url: URL,
        method: String,
        bodyData: Data?
    ) async throws -> Response {
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let data = bodyData {
            request.httpBody = data
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let (data, response) = try await Self.session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw FabushiLoopbackClientError.nonHTTPResponse
        }
        guard (200..<300).contains(http.statusCode) else {
            let preview = String(decoding: data, as: UTF8.self)
            throw FabushiLoopbackClientError.httpStatus(http.statusCode, preview)
        }
        let decoder = JSONDecoder()
        return try decoder.decode(Response.self, from: data)
    }
}

// MARK: - Canonical Swift 6.0 @MainActor Opaque-Box Base XCUITestCase

@MainActor
class FabushiE2EUITestCase: XCTestCase {
    var app: XCUIApplication?
    private var activeLoopbackClient: FabushiLoopbackClient?
    private var didModifyOrientation = false
    private var checkpoints: [String: Bool] = [:]
    private var metadata: [String: String] = [:]
    private var ephemeralDataDirectory: URL?

    override func setUp() async throws {
        try await super.setUp()
        continueAfterFailure = false
        didModifyOrientation = false
        checkpoints = [:]
        metadata = [:]
        activeLoopbackClient = nil
        ephemeralDataDirectory = nil
    }

    override func tearDown() async throws {
        if didModifyOrientation {
            XCUIDevice.shared.orientation = .portrait
            didModifyOrientation = false
        }
        if let client = activeLoopbackClient {
            _ = try? await client.clearFaults()
        }
        let evidence = FabushiE2EStateEvidence(
            testName: sanitizedTestName(),
            formFactor: activeFormFactor().rawValue,
            tier: resolvedTierName(),
            sourceSHA: resolvedSourceSHA(),
            checkpoints: checkpoints,
            metadata: metadata
        )
        emitEvidenceRecord(evidence)
        if app != nil {
            attachCheckpointScreenshot(named: "teardown_\(sanitizedTestName())")
        }
        app?.terminate()
        app = nil
        activeLoopbackClient = nil
        if let dataDir = ephemeralDataDirectory {
            try? FileManager.default.removeItem(at: dataDir)
            ephemeralDataDirectory = nil
        }
        try await super.tearDown()
    }

    // MARK: - Environment & Loopback Resolution (Fail-Closed, Zero XCTSkip)

    func resolvedServerURL(
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws -> URL {
        let env = ProcessInfo.processInfo.environment
        let raw = (env["FABUSHI_E2E_SERVER_URL"] ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty,
              let url = URL(string: raw),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              url.host != nil
        else {
            let reason = "FAIL-CLOSED: FABUSHI_E2E_SERVER_URL is missing or invalid ('\(raw)')"
            XCTFail(reason, file: file, line: line)
            throw FabushiLoopbackClientError.missingServerURL(reason)
        }
        return url
    }

    func resolvedTierName() -> String {
        let raw = ProcessInfo.processInfo.environment["FABUSHI_E2E_TIER"] ?? "unknown"
        return raw.isEmpty ? "unknown" : raw
    }

    func resolvedSourceSHA() -> String {
        let raw = ProcessInfo.processInfo.environment["FABUSHI_E2E_SOURCE_SHA"] ?? "unknown"
        return raw.isEmpty ? "unknown" : raw
    }

    func activeFormFactor() -> FabushiE2EFormFactor {
        let envValue = (ProcessInfo.processInfo.environment["FABUSHI_E2E_FORM_FACTOR"] ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        if let explicit = FabushiE2EFormFactor(rawValue: envValue) {
            return explicit
        }
        return UIDevice.current.userInterfaceIdiom == .pad ? .ipad : .iphone
    }

    func prepareCleanLoopback(
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws -> FabushiLoopbackClient {
        let url = try resolvedServerURL(file: file, line: line)
        let client = FabushiLoopbackClient(baseURL: url)
        activeLoopbackClient = client
        let resetResp = try await client.resetState()
        XCTAssertTrue(resetResp.ok && resetResp.reset, "Loopback /__e2e__/reset must succeed", file: file, line: line)
        let health = try await client.checkHealth()
        XCTAssertTrue(health.ok, "Loopback /healthz must return ok == true", file: file, line: line)
        metadata["server_url"] = url.absoluteString
        return client
    }

    // MARK: - Standardized Opaque-Box App Launch

    func launchFabushi(
        _ config: FabushiE2ELaunchConfig = FabushiE2ELaunchConfig(),
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws -> XCUIApplication {
        let serverURL = try resolvedServerURL(file: file, line: line)
        let dataDir = try ensureEphemeralDataDirectory(reset: config.resetState)

        let application = XCUIApplication()
        var env = application.launchEnvironment
        env["FABUSHI_UI_TEST_MODE"] = "1"
        env["FABUSHI_FEATURE_HOST_TEST"] = "0"
        env["FABUSHI_E2E_SERVER_URL"] = serverURL.absoluteString
        env["GPUI_TELEGRAM_SERVER_URL"] = serverURL.absoluteString
        env["FABUSHI_APP_DATA"] = dataDir.path
        env["FABUSHI_UI_THEME"] = config.theme
        env["FABUSHI_E2E_FORM_FACTOR"] = activeFormFactor().rawValue
        env["FABUSHI_E2E_TIER"] = resolvedTierName()
        env["FABUSHI_E2E_SOURCE_SHA"] = resolvedSourceSHA()
        if config.resetState {
            env["FABUSHI_E2E_RESET_STATE"] = "1"
        } else {
            env.removeValue(forKey: "FABUSHI_E2E_RESET_STATE")
        }
        if let accent = config.accentHex, !accent.isEmpty {
            env["FABUSHI_UI_ACCENT_HEX"] = accent
        }
        if config.reduceMotion {
            env["FABUSHI_UI_REDUCE_MOTION"] = "1"
        }
        if config.highContrast {
            env["FABUSHI_UI_HIGH_CONTRAST"] = "1"
        }
        for (key, value) in config.extraEnvironment {
            env[key] = value
        }
        application.launchEnvironment = env

        var args: [String] = [
            "--server-url", serverURL.absoluteString,
            "-AppleLanguages", "(\(config.languageCode))",
            "-AppleLocale", config.localeIdentifier,
        ]
        if let contentSize = config.preferredContentSizeCategory, !contentSize.isEmpty {
            args.append(contentsOf: ["-UIPreferredContentSizeCategoryName", contentSize])
        }
        args.append(contentsOf: config.extraArguments)
        application.launchArguments = args

        application.launch()
        self.app = application

        let inForeground = application.state == .runningForeground
            || application.wait(for: .runningForeground, timeout: 15)
        XCTAssertTrue(
            inForeground,
            "Fabushi app failed to reach runningForeground state",
            file: file,
            line: line
        )
        metadata["theme"] = config.theme
        metadata["locale"] = config.localeIdentifier
        return application
    }

    // MARK: - Element Query, Navigation & Input Helpers

    func element(_ identifier: String, in application: XCUIApplication) -> XCUIElement {
        application.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    func requireElement(
        _ identifier: String,
        in application: XCUIApplication,
        timeout: TimeInterval = 10,
        file: StaticString = #filePath,
        line: UInt = #line
    ) -> XCUIElement {
        let target = element(identifier, in: application)
        let appeared = target.waitForExistence(timeout: timeout)
        XCTAssertTrue(
            appeared,
            "Expected element '\(identifier)' to exist within \(timeout)s",
            file: file,
            line: line
        )
        return target
    }

    func tapControl(
        _ identifier: String,
        in application: XCUIApplication,
        timeout: TimeInterval = 10,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        tapControlWithHIGChecks(
            identifier,
            in: application,
            timeout: timeout,
            verifyMinTouchTarget: false,
            verifyWithinBounds: false,
            verifyVoiceOver: false,
            file: file,
            line: line
        )
    }

    func tapControlWithHIGChecks(
        _ identifier: String,
        in application: XCUIApplication,
        timeout: TimeInterval = 10,
        verifyMinTouchTarget: Bool = true,
        minTouchTarget: CGFloat = 44.0,
        verifyWithinBounds: Bool = true,
        verifyVoiceOver: Bool = false,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let control = requireElement(identifier, in: application, timeout: timeout, file: file, line: line)
        XCTAssertTrue(
            control.isEnabled,
            "Control '\(identifier)' must be enabled before tap",
            file: file,
            line: line
        )
        if !control.isHittable {
            let scrollable = application.scrollViews.firstMatch
            if scrollable.exists {
                _ = scrollToElement(control, in: scrollable, maxSwipes: 6)
            }
        }
        XCTAssertTrue(
            control.isHittable,
            "Control '\(identifier)' must be hittable before tap",
            file: file,
            line: line
        )
        if verifyMinTouchTarget {
            assertMinimumTouchTarget44pt(
                control,
                minPoints: minTouchTarget,
                requireHittable: true,
                file: file,
                line: line
            )
        }
        if verifyWithinBounds {
            assertWithinWindowBounds(control, in: application, file: file, line: line)
        }
        if verifyVoiceOver {
            assertVoiceOverReady(control, file: file, line: line)
        }
        control.tap()
    }

    func replaceText(in field: XCUIElement, with text: String) {
        field.tap()
        if let existing = field.value as? String, !existing.isEmpty {
            let placeholder = field.placeholderValue ?? ""
            if existing != placeholder {
                let deleteSequence = String(
                    repeating: XCUIKeyboardKey.delete.rawValue,
                    count: existing.count
                )
                field.typeText(deleteSequence)
            }
        }
        field.typeText(text)
    }

    func scrollToElement(
        _ target: XCUIElement,
        in container: XCUIElement,
        maxSwipes: Int = 6
    ) -> Bool {
        if target.exists && target.isHittable {
            return true
        }
        for _ in 0..<maxSwipes {
            container.swipeUp()
            if target.exists && target.isHittable {
                return true
            }
        }
        return target.exists
    }

    func waitUntil(
        timeout: TimeInterval = 10,
        pollMilliseconds: Int = 100,
        _ condition: @MainActor () -> Bool
    ) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() {
                return true
            }
            if Task.isCancelled {
                return false
            }
            try? await Task.sleep(for: .milliseconds(pollMilliseconds))
        }
        return condition()
    }

    func setDeviceOrientation(_ orientation: UIDeviceOrientation) {
        didModifyOrientation = true
        XCUIDevice.shared.orientation = orientation
    }

    func openDeepLink(
        _ urlString: String,
        in application: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        guard let url = URL(string: urlString) else {
            XCTFail("Invalid deep-link URL: \(urlString)", file: file, line: line)
            return
        }
        application.open(url)
    }

    func signInIfNeeded(
        in application: XCUIApplication,
        username: String,
        password: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let shellRoot = element(FabushiAccessibilityIDs.Shell.root, in: application)
        if shellRoot.waitForExistence(timeout: 3) {
            return
        }
        let userField = requireElement(
            FabushiAccessibilityIDs.Auth.usernameField,
            in: application,
            timeout: 10,
            file: file,
            line: line
        )
        replaceText(in: userField, with: username)
        let passField = requireElement(
            FabushiAccessibilityIDs.Auth.passwordField,
            in: application,
            timeout: 5,
            file: file,
            line: line
        )
        replaceText(in: passField, with: password)
        tapControl(FabushiAccessibilityIDs.Auth.loginButton, in: application, file: file, line: line)
        let shellVisible = shellRoot.waitForExistence(timeout: 15)
        XCTAssertTrue(
            shellVisible,
            "Authenticated shell root '\(FabushiAccessibilityIDs.Shell.root)' did not appear after login",
            file: file,
            line: line
        )
    }

    func openShellTab(
        _ tabIdentifier: String,
        in application: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let tab = element(tabIdentifier, in: application)
        if tab.waitForExistence(timeout: 5) && tab.isHittable {
            tab.tap()
            return
        }
        let backButton = element(FabushiAccessibilityIDs.Shell.backButton, in: application)
        if backButton.exists && backButton.isHittable {
            backButton.tap()
        } else {
            let navBack = application.navigationBars.buttons.firstMatch
            if navBack.exists && navBack.isHittable {
                navBack.tap()
            }
        }
        tapControl(tabIdentifier, in: application, timeout: 10, file: file, line: line)
    }

    func openFirstChat(
        in application: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        openChatRow(FabushiAccessibilityIDs.ChatList.firstRow, in: application, file: file, line: line)
    }

    func dismissSheetIfPresented(
        _ sheetIdentifier: String,
        dismissButtonID: String,
        in application: XCUIApplication,
        timeout: TimeInterval = 5,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let sheet = element(sheetIdentifier, in: application)
        guard sheet.exists else { return }
        tapControlWithHIGChecks(
            dismissButtonID,
            in: application,
            timeout: timeout,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: false,
            file: file,
            line: line
        )
        let dismissed = sheet.waitForNonExistence(timeout: timeout)
        XCTAssertTrue(
            dismissed,
            "Presented sheet '\(sheetIdentifier)' must dismiss after tapping '\(dismissButtonID)'",
            file: file,
            line: line
        )
    }

    func dismissInspectorIfPresented(
        in application: XCUIApplication,
        timeout: TimeInterval = 5,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        dismissSheetIfPresented(
            FabushiAccessibilityIDs.Inspector.drawer,
            dismissButtonID: FabushiAccessibilityIDs.Inspector.closeButton,
            in: application,
            timeout: timeout,
            file: file,
            line: line
        )
    }

    func dismissAccountSwitcherIfPresented(
        in application: XCUIApplication,
        timeout: TimeInterval = 5,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        dismissSheetIfPresented(
            FabushiAccessibilityIDs.Shell.accountSwitcherSheet,
            dismissButtonID: FabushiAccessibilityIDs.Shell.accountSwitcherCloseButton,
            in: application,
            timeout: timeout,
            file: file,
            line: line
        )
    }

    func navigateBackToChatListIfCompact(
        in application: XCUIApplication,
        timeout: TimeInterval = 8,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let chatList = element(FabushiAccessibilityIDs.ChatList.list, in: application)
        if activeFormFactor() == .ipad && chatList.exists && chatList.isHittable {
            return
        }
        dismissInspectorIfPresented(in: application, file: file, line: line)
        dismissAccountSwitcherIfPresented(in: application, file: file, line: line)
        if chatList.exists && chatList.isHittable {
            return
        }
        let shellBack = element(FabushiAccessibilityIDs.Shell.backButton, in: application)
        if shellBack.exists && shellBack.isHittable {
            shellBack.tap()
        } else {
            let navBack = application.navigationBars.buttons.firstMatch
            if navBack.exists && navBack.isHittable {
                navBack.tap()
            }
        }
        let visible = chatList.waitForExistence(timeout: timeout)
        XCTAssertTrue(
            visible,
            "ChatList '\(FabushiAccessibilityIDs.ChatList.list)' must be visible after compact back navigation",
            file: file,
            line: line
        )
    }

    func openChatRow(
        _ rowIdentifier: String,
        in application: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        navigateBackToChatListIfCompact(in: application, file: file, line: line)
        tapControl(rowIdentifier, in: application, file: file, line: line)
        _ = requireElement(FabushiAccessibilityIDs.Stage.root, in: application, file: file, line: line)
    }

    func reopenFirstChatIfComposerNotVisible(
        in application: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        if !element(FabushiAccessibilityIDs.Composer.textField, in: application).waitForExistence(timeout: 2) {
            openFirstChat(in: application, file: file, line: line)
        }
    }

    func openSettingsRoot(
        in application: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application, file: file, line: line)
        let sectionList = element(FabushiAccessibilityIDs.Settings.sectionList, in: application)
        if !sectionList.waitForExistence(timeout: 2) {
            let shellBack = element(FabushiAccessibilityIDs.Shell.backButton, in: application)
            if shellBack.exists && shellBack.isHittable {
                shellBack.tap()
            } else {
                let navBack = application.navigationBars.buttons.firstMatch
                if navBack.exists && navBack.isHittable {
                    navBack.tap()
                }
            }
        }
        _ = requireElement(FabushiAccessibilityIDs.Settings.root, in: application, file: file, line: line)
    }

    // MARK: - Apple HIG & Accessibility Assertion Oracles

    func assertMinimumTouchTarget44pt(
        _ target: XCUIElement,
        minPoints: CGFloat = 44.0,
        requireHittable: Bool = false,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertTrue(
            target.exists,
            "Touch-target element '\(target.identifier)' does not exist",
            file: file,
            line: line
        )
        let frame = target.frame
        XCTAssertFalse(
            frame.isNull || frame.isInfinite || frame.isEmpty,
            "Touch-target element '\(target.identifier)' has null/infinite/empty frame: \(frame)",
            file: file,
            line: line
        )
        XCTAssertTrue(
            frame.origin.x.isFinite && frame.origin.y.isFinite && frame.width.isFinite && frame.height.isFinite,
            "Touch-target element '\(target.identifier)' has non-finite frame coordinates: \(frame)",
            file: file,
            line: line
        )
        XCTAssertGreaterThanOrEqual(
            frame.width,
            minPoints,
            "Apple HIG 44pt width violation on '\(target.identifier)': width=\(frame.width)pt",
            file: file,
            line: line
        )
        XCTAssertGreaterThanOrEqual(
            frame.height,
            minPoints,
            "Apple HIG 44pt height violation on '\(target.identifier)': height=\(frame.height)pt",
            file: file,
            line: line
        )
        if requireHittable {
            XCTAssertTrue(
                target.isHittable,
                "Touch-target element '\(target.identifier)' must be hittable",
                file: file,
                line: line
            )
        }
    }

    func assertWithinWindowBounds(
        _ target: XCUIElement,
        in application: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertTrue(
            target.exists,
            "Element '\(target.identifier)' must exist for bounds check",
            file: file,
            line: line
        )
        let window = application.windows.firstMatch
        XCTAssertTrue(
            window.exists,
            "Application window must exist for bounds check of '\(target.identifier)'",
            file: file,
            line: line
        )
        let windowFrame = window.frame
        XCTAssertFalse(
            windowFrame.isNull || windowFrame.isInfinite || windowFrame.isEmpty
                || !windowFrame.origin.x.isFinite || !windowFrame.origin.y.isFinite
                || !windowFrame.width.isFinite || !windowFrame.height.isFinite,
            "Application window has invalid/empty frame: \(windowFrame)",
            file: file,
            line: line
        )
        let elemFrame = target.frame
        XCTAssertFalse(
            elemFrame.isNull || elemFrame.isInfinite || elemFrame.isEmpty
                || !elemFrame.origin.x.isFinite || !elemFrame.origin.y.isFinite
                || !elemFrame.width.isFinite || !elemFrame.height.isFinite,
            "Element '\(target.identifier)' has null/infinite/empty/non-finite frame: \(elemFrame)",
            file: file,
            line: line
        )
        XCTAssertGreaterThanOrEqual(
            elemFrame.minX,
            windowFrame.minX - 1.0,
            "Element '\(target.identifier)' clipped on leading edge: \(elemFrame.minX) < \(windowFrame.minX)",
            file: file,
            line: line
        )
        XCTAssertLessThanOrEqual(
            elemFrame.maxX,
            windowFrame.maxX + 1.0,
            "Element '\(target.identifier)' clipped on trailing edge: \(elemFrame.maxX) > \(windowFrame.maxX)",
            file: file,
            line: line
        )
        XCTAssertGreaterThanOrEqual(
            elemFrame.minY,
            windowFrame.minY - 1.0,
            "Element '\(target.identifier)' clipped on top edge: \(elemFrame.minY) < \(windowFrame.minY)",
            file: file,
            line: line
        )
        XCTAssertLessThanOrEqual(
            elemFrame.maxY,
            windowFrame.maxY + 1.0,
            "Element '\(target.identifier)' clipped on bottom edge: \(elemFrame.maxY) > \(windowFrame.maxY)",
            file: file,
            line: line
        )
    }

    func assertNonOverlapping(
        _ first: XCUIElement,
        _ second: XCUIElement,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertTrue(first.exists, "First element '\(first.identifier)' missing", file: file, line: line)
        XCTAssertTrue(second.exists, "Second element '\(second.identifier)' missing", file: file, line: line)
        let firstFrame = first.frame
        let secondFrame = second.frame
        XCTAssertFalse(
            firstFrame.isNull || firstFrame.isInfinite || firstFrame.isEmpty
                || !firstFrame.origin.x.isFinite || !firstFrame.origin.y.isFinite
                || !firstFrame.width.isFinite || !firstFrame.height.isFinite,
            "First element '\(first.identifier)' has empty/invalid frame for non-overlap check: \(firstFrame)",
            file: file,
            line: line
        )
        XCTAssertFalse(
            secondFrame.isNull || secondFrame.isInfinite || secondFrame.isEmpty
                || !secondFrame.origin.x.isFinite || !secondFrame.origin.y.isFinite
                || !secondFrame.width.isFinite || !secondFrame.height.isFinite,
            "Second element '\(second.identifier)' has empty/invalid frame for non-overlap check: \(secondFrame)",
            file: file,
            line: line
        )
        let overlap = firstFrame.intersection(secondFrame)
        let hasOverlap = !overlap.isNull && overlap.width > 0.5 && overlap.height > 0.5
        XCTAssertFalse(
            hasOverlap,
            "Elements '\(first.identifier)' and '\(second.identifier)' overlap by \(overlap.width)x\(overlap.height)pt",
            file: file,
            line: line
        )
    }

    func assertVoiceOverReady(
        _ target: XCUIElement,
        forbiddenKeyPrefixes: [String] = [
            "fabushi.",
            "auth.",
            "shell.",
            "chatlist.",
            "compose.",
            "stage.",
            "composer.",
            "inspector.",
            "studio.",
            "magpie.",
            "remote.",
            "miniapp.",
            "call.",
            "settings.",
        ],
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertTrue(
            target.exists,
            "VoiceOver element '\(target.identifier)' does not exist",
            file: file,
            line: line
        )
        let label = target.label.trimmingCharacters(in: .whitespacesAndNewlines)
        XCTAssertFalse(
            label.isEmpty,
            "VoiceOver accessibilityLabel is empty for identifier '\(target.identifier)'",
            file: file,
            line: line
        )
        XCTAssertFalse(
            target.identifier.isEmpty,
            "accessibilityIdentifier must not be empty",
            file: file,
            line: line
        )
        XCTAssertNotEqual(
            label,
            target.identifier,
            "VoiceOver label must be human-readable, not raw accessibilityIdentifier '\(target.identifier)'",
            file: file,
            line: line
        )
        let lowerLabel = label.lowercased()
        for prefix in forbiddenKeyPrefixes {
            XCTAssertFalse(
                lowerLabel.contains(prefix),
                "Raw localization key leak ('\(prefix)') detected in VoiceOver label '\(label)' for '\(target.identifier)'",
                file: file,
                line: line
            )
        }
    }

    func assertNoDeadInteractiveControls(
        in application: XCUIApplication,
        identifiers: [String],
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertFalse(identifiers.isEmpty, "Control identifier list must not be empty", file: file, line: line)
        for identifier in identifiers {
            let control = requireElement(identifier, in: application, timeout: 8, file: file, line: line)
            XCTAssertTrue(
                control.isEnabled,
                "Dead control detected (disabled): '\(identifier)'",
                file: file,
                line: line
            )
            assertMinimumTouchTarget44pt(control, requireHittable: true, file: file, line: line)
            assertWithinWindowBounds(control, in: application, file: file, line: line)
            assertVoiceOverReady(control, file: file, line: line)
        }
    }

    func assertControlIsLive(
        _ controlIdentifier: String,
        producesElement expectedIdentifier: String,
        dismissWith dismissIdentifier: String? = nil,
        in application: XCUIApplication,
        timeout: TimeInterval = 8,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        tapControlWithHIGChecks(
            controlIdentifier,
            in: application,
            timeout: timeout,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true,
            file: file,
            line: line
        )
        let expected = requireElement(expectedIdentifier, in: application, timeout: timeout, file: file, line: line)
        assertWithinWindowBounds(expected, in: application, file: file, line: line)
        if let dismissID = dismissIdentifier {
            tapControlWithHIGChecks(
                dismissID,
                in: application,
                timeout: timeout,
                verifyMinTouchTarget: true,
                verifyWithinBounds: true,
                verifyVoiceOver: false,
                file: file,
                line: line
            )
            let dismissed = expected.waitForNonExistence(timeout: timeout)
            XCTAssertTrue(
                dismissed,
                "Control '\(controlIdentifier)' destination '\(expectedIdentifier)' must dismiss via '\(dismissID)'",
                file: file,
                line: line
            )
        }
    }

    func assertNoDeadInteractiveControls(
        in application: XCUIApplication,
        probes: [FabushiLiveControlProbe],
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertFalse(probes.isEmpty, "Live control probe list must not be empty", file: file, line: line)
        for probe in probes {
            assertControlIsLive(
                probe.controlIdentifier,
                producesElement: probe.expectedElementIdentifier,
                dismissWith: probe.dismissIdentifier,
                in: application,
                timeout: probe.timeout,
                file: file,
                line: line
            )
        }
    }

    func assertNoSampleChats(
        in application: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let forbiddenTitles = [
            "Saved Messages",
            "Fabushi Community",
            "Sample Chat",
            "Demo Bot Chat",
            "Chief of Staff",
        ]
        for title in forbiddenTitles {
            let predicate = NSPredicate(format: "label CONTAINS[cd] %@", title)
            let leaked = application.descendants(matching: .any).matching(predicate).firstMatch
            XCTAssertFalse(
                leaked.exists,
                "Forbidden hardcoded sample chat '\(title)' rendered in live UI",
                file: file,
                line: line
            )
        }
    }

    static func computeWCAGAccentFill(_ hex: String) -> (effectiveHex: String, inputLuma: Double, adjusted: Bool)? {
        let cleaned = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        let stripped = cleaned.hasPrefix("#") ? String(cleaned.dropFirst()) : cleaned
        guard stripped.count == 6,
              stripped.allSatisfy(\.isHexDigit),
              let rgb = UInt32(stripped, radix: 16)
        else {
            return nil
        }
        let r = Int((rgb >> 16) & 0xFF)
        let g = Int((rgb >> 8) & 0xFF)
        let b = Int(rgb & 0xFF)
        let luma = 0.299 * Double(r) + 0.587 * Double(g) + 0.114 * Double(b)
        if luma <= 138.0 {
            return (String(format: "#%02X%02X%02X", r, g, b), luma, false)
        }
        let darkenT = min(max((luma - 138.0) / 145.0, 0.0), 0.36)
        func mix(_ channel: Int) -> Int {
            let blended = Double(channel) * (1.0 - darkenT) + Double(0x11) * darkenT
            return min(max(Int(blended.rounded()), 0), 255)
        }
        return (String(format: "#%02X%02X%02X", mix(r), mix(g), mix(b)), luma, true)
    }

    static func extractHexColors(from text: String) -> [String] {
        let chars = Array(text)
        var results: [String] = []
        var idx = 0
        while idx + 6 < chars.count {
            if chars[idx] == "#" {
                let slice = chars[(idx + 1)...(idx + 6)]
                if slice.allSatisfy(\.isHexDigit) {
                    let boundaryAfter = (idx + 7 == chars.count) || !chars[idx + 7].isHexDigit
                    if boundaryAfter {
                        results.append("#" + String(slice).uppercased())
                        idx += 7
                        continue
                    }
                }
            }
            idx += 1
        }
        return results
    }

    func isSwitchEnabled(_ toggle: XCUIElement) -> Bool {
        if let raw = toggle.value as? String {
            let normalized = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            return normalized == "1" || normalized == "true" || normalized == "on" || normalized == "enabled"
        }
        if let number = toggle.value as? NSNumber {
            return number.boolValue
        }
        return false
    }

    func assertCapabilityLedgerShowsCanonicalIDs(
        in application: XCUIApplication,
        requiredIDs: [String] = ["OCRM-001", "FBCP-001", "TON_ONCHAIN_SIGNING", "TELEGRAM_PASSPORT"],
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let ledger = requireElement(FabushiAccessibilityIDs.Settings.capabilityLedgerList, in: application, file: file, line: line)
        assertWithinWindowBounds(ledger, in: application, file: file, line: line)
        assertVoiceOverReady(ledger, file: file, line: line)
        for capID in requiredIDs {
            let pred = NSPredicate(format: "label CONTAINS[c] %@ OR identifier CONTAINS[c] %@", capID, capID)
            let row = ledger.descendants(matching: .any).matching(pred).firstMatch
            if !row.exists {
                _ = scrollToElement(row, in: ledger, maxSwipes: 4)
            }
            XCTAssertTrue(
                row.exists,
                "F34 honest capability ledger must visibly disclose canonical tracked ID '\(capID)'",
                file: file,
                line: line
            )
        }
    }

    // MARK: - Checkpoint Screenshots & Structured Evidence Output

    func attachCheckpointScreenshot(named name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func recordCheckpoint(_ name: String, passed: Bool = true, attachScreen: Bool = false) {
        checkpoints[name] = passed
        if attachScreen {
            attachCheckpointScreenshot(named: name)
        }
    }

    func emitEvidenceRecord(_ evidence: FabushiE2EStateEvidence) {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let encoded = try? encoder.encode(evidence) else {
            return
        }
        let line = "FABUSHI_E2E_STATE_BASE64=\(encoded.base64EncodedString())\n"
        try? FileHandle.standardOutput.write(contentsOf: Data(line.utf8))
    }

    // MARK: - Private Helpers

    private func sanitizedTestName() -> String {
        name
            .replacingOccurrences(of: "-[", with: "")
            .replacingOccurrences(of: "]", with: "")
            .replacingOccurrences(of: " ", with: "_")
    }

    private func ensureEphemeralDataDirectory(reset: Bool) throws -> URL {
        if !reset, let existing = ephemeralDataDirectory {
            return existing
        }
        if let existing = ephemeralDataDirectory {
            try? FileManager.default.removeItem(at: existing)
        }
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_e2e_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        ephemeralDataDirectory = dir
        return dir
    }
}
