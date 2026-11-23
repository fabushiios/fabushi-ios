import Foundation
import Observation
import os
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Coordinator Errors

public enum FabushiCoordinatorError: LocalizedError, Sendable, Equatable {
    case notStarted
    case staleGeneration(captured: UInt64, current: UInt64)

    public var errorDescription: String? {
        switch self {
        case .notStarted:
            return "FabushiCoordinator has not been started."
        case .staleGeneration(let captured, let current):
            return "Generation fence rejected stale completion (captured \(captured), current \(current))."
        }
    }
}

#if canImport(UIKit)
private final class BackgroundTaskGuard: Sendable {
    private let lock = OSAllocatedUnfairLock<UIBackgroundTaskIdentifier>(initialState: .invalid)

    func store(_ id: UIBackgroundTaskIdentifier) {
        lock.withLock { $0 = id }
    }

    func endIfNeeded() {
        let id = lock.withLock { current -> UIBackgroundTaskIdentifier in
            let existing = current
            current = .invalid
            return existing
        }
        guard id != .invalid else { return }
        Task { @MainActor in
            UIApplication.shared.endBackgroundTask(id)
        }
    }
}
#endif

// MARK: - Canonical App Coordinator (PROJECT.md Contract #2)

/// Single canonical `@MainActor @Observable` coordinator for Fabushi iOS.
///
/// Enforces:
/// - Fail-closed Keychain storage passphrase provisioning (zero static fallbacks).
/// - Authoritative Rust generation fencing (`executeFenced` drops stale async responses;
///   event pump discards events from older generations).
/// - `0xdead10cc` background suspension safety by wrapping `settings.checkpointWal` inside
///   `UIApplication.shared.beginBackgroundTask` / `endBackgroundTask`.
/// - Deduplicated `NWPathMonitor` bridging to Rust `outbox.setNetworkState`.
@MainActor
@Observable
public final class FabushiCoordinator {
    public static let shared = FabushiCoordinator()

    @ObservationIgnored public let host: FabushiRustHostActor
    @ObservationIgnored public let keychain: FabushiKeychainStore
    @ObservationIgnored public let networkMonitor: FabushiNetworkMonitor

    public private(set) var isStarted: Bool = false
    public private(set) var authState: FabushiAuthState = .unauthenticated
    public private(set) var accountScopeGeneration: UInt64 = 1
    public private(set) var activeAccountId: String? = nil
    public var activeRoute: ShellRoute = .chats(chatId: nil, messageId: nil)
    public private(set) var isOnline: Bool = true
    public private(set) var lastWalCheckpoint: FabushiWalCheckpointSummary? = nil
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var registeredAPNsDeviceTokenHex: String? = nil

    // Domain Store Anchors (PROJECT.md § Interface Contracts #2)
    public let chatStore = ChatStore()
    public let agentStudioStore = AgentStudioStore()
    public let remoteComputerStore = RemoteComputerStore()
    public let miniAppHostStore = MiniAppHostStore()
    public let callSessionStore = CallSessionStore()
    public let settingsStore = SettingsStore()
    public let vaultStore = VaultStore()

    @ObservationIgnored
    nonisolated(unsafe) private var eventPumpTask: Task<Void, Never>?

    public init(
        host: FabushiRustHostActor = .shared,
        keychain: FabushiKeychainStore = .shared,
        networkMonitor: FabushiNetworkMonitor = FabushiNetworkMonitor()
    ) {
        self.host = host
        self.keychain = keychain
        self.networkMonitor = networkMonitor
    }

    deinit {
        eventPumpTask?.cancel()
        networkMonitor.stop()
    }

    // MARK: - Lifecycle

    public func start(
        appDataDirectory: URL? = nil,
        serverURL: String? = nil,
        testMode: Bool = false,
        startNetworkMonitor: Bool = true
    ) async throws {
        if isStarted {
            return
        }

        let directory: URL
        if let appDataDirectory {
            directory = appDataDirectory
        } else {
            let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            directory = support.appendingPathComponent("FabushiCore", isDirectory: true)
        }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        // Fail-closed Keychain passphrase retrieval/creation (ZERO empty or static fallbacks!)
        let passphrase: String
        do {
            passphrase = try keychain.loadOrCreateStoragePassphrase()
        } catch let kcError as FabushiKeychainError {
            lastErrorMessage = kcError.localizedDescription
            if case .osStatus(let status) = kcError {
                throw FabushiHostError.keychainFailed(status: status)
            }
            throw kcError
        }

        let config = FabushiHostConfig(
            appDataDir: directory.path,
            storagePassphrase: passphrase,
            serverUrl: serverURL,
            httpTimeoutSecs: FabushiHostConfig.defaultHttpTimeoutSecs,
            testMode: testMode,
            iosSandboxStrict: true
        )

        try await host.initialize(config: config)

        let liveGeneration = try await host.currentGeneration()
        accountScopeGeneration = liveGeneration

        if let urlInfo = try? await host.getServerURL() {
            settingsStore.updateServerURL(urlInfo.serverUrl)
        }

        if let accountsInfo = try? await host.listAccounts(generation: liveGeneration) {
            restoreAuthState(from: accountsInfo)
        }

        startEventPump()

        if startNetworkMonitor {
            networkMonitor.start(host: host) { [weak self] snapshot in
                Task { @MainActor [weak self] in
                    self?.isOnline = snapshot.isOnline
                }
            }
        }

        lastErrorMessage = nil
        isStarted = true
    }

    public func shutdown() async {
        eventPumpTask?.cancel()
        eventPumpTask = nil
        networkMonitor.stop()
        await host.shutdown()
        isStarted = false
    }

    // MARK: - Generation Fencing Invariant

    /// Executes an async operation fenced by `accountScopeGeneration`.
    /// Checks generation before AND after `await`, dropping stale completions without mutating state.
    public func executeFenced<T: Sendable>(
        _ operation: @Sendable (UInt64) async throws -> T
    ) async throws -> T {
        guard isStarted else {
            throw FabushiCoordinatorError.notStarted
        }
        let capturedGen = accountScopeGeneration
        let value = try await operation(capturedGen)
        guard accountScopeGeneration == capturedGen else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: capturedGen,
                current: accountScopeGeneration
            )
        }
        return value
    }

    /// Dispatches a typed RPC stamped with `accountScopeGeneration` and fenced on return.
    public func dispatchFenced<Req: Encodable & Sendable, Res: Decodable & Sendable>(
        method: String,
        params: Req
    ) async throws -> Res {
        guard isStarted else {
            throw FabushiCoordinatorError.notStarted
        }
        let capturedGen = accountScopeGeneration
        let capturedAccount = activeAccountId
        let envelope: FabushiRPCEnvelopeResult<Res> = try await host.dispatchEnvelope(
            method: method,
            params: params,
            generation: capturedGen,
            accountId: capturedAccount
        )
        guard accountScopeGeneration == capturedGen else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: capturedGen,
                current: accountScopeGeneration
            )
        }
        return envelope.result
    }

    // MARK: - Authoritative Account & Session Transitions

    @discardableResult
    public func setAuthSession(_ params: FabushiSessionAuthParams) async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let res = try await host.setAuthSession(params, generation: accountScopeGeneration)
        let accountId = String(res.userId)
        applyGenerationAdvance(res.generation)
        activeAccountId = accountId
        authState = .authenticated(userId: res.userId, username: params.username, accountId: accountId)
        return res.generation
    }

    @discardableResult
    public func switchAccount(accountId: String) async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let res = try await host.switchAccount(accountId: accountId, generation: accountScopeGeneration)
        applyGenerationAdvance(res.generation)
        activeAccountId = res.accountId
        let username = (try? await host.listAccounts(generation: res.generation))?
            .accounts
            .first(where: { $0.accountId == res.accountId })?
            .username ?? res.accountId
        authState = .authenticated(userId: res.userId, username: username, accountId: res.accountId)
        return res.generation
    }

    @discardableResult
    public func clearLocalSession() async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let res = try await host.clearSession(generation: accountScopeGeneration)
        applyGenerationAdvance(res.generation)
        activeAccountId = nil
        authState = .unauthenticated
        return res.generation
    }

    @discardableResult
    public func logout() async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        do {
            let res = try await host.logout(generation: accountScopeGeneration)
            applyGenerationAdvance(res.generation)
            activeAccountId = nil
            authState = .unauthenticated
            return res.generation
        } catch {
            // Even if remote logout failed due to offline network, Rust locally wiped the session
            // and bumped generation; resync authoritative generation from Rust before rethrowing.
            if let synced = try? await host.currentGeneration(), synced != accountScopeGeneration {
                applyGenerationAdvance(synced)
                activeAccountId = nil
                authState = .unauthenticated
            }
            throw error
        }
    }

    @discardableResult
    public func setServerURL(_ serverUrl: String) async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let res = try await host.setServerURL(serverUrl, generation: accountScopeGeneration)
        applyGenerationAdvance(res.generation)
        activeAccountId = nil
        authState = .unauthenticated
        settingsStore.updateServerURL(res.serverUrl)
        return res.generation
    }

    // MARK: - APNs Device Token Forwarding

    public func registerAPNsDeviceToken(_ tokenData: Data, environment: String? = nil) async {
        let tokenHex = tokenData.map { String(format: "%02x", $0) }.joined()
        registeredAPNsDeviceTokenHex = tokenHex
        guard isStarted else { return }

        struct RegisterTokenParams: Encodable, Sendable {
            let token: String
            let environment: String?
        }

        let resolvedEnv: String
        if let environment {
            resolvedEnv = environment
        } else {
            #if DEBUG
            resolvedEnv = "sandbox"
            #else
            resolvedEnv = "production"
            #endif
        }

        _ = try? await host.dispatch(
            method: FabushiRPCNamespace.push.method("registerToken"),
            params: RegisterTokenParams(token: tokenHex, environment: resolvedEnv),
            generation: accountScopeGeneration,
            accountId: activeAccountId
        ) as FabushiJSONValue
    }

    // MARK: - 0xdead10cc Background Suspension WAL Checkpoint Safety

    @discardableResult
    public func checkpointWAL() async throws -> FabushiWalCheckpointSummary {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let summary = try await host.checkpointWAL(generation: accountScopeGeneration)
        lastWalCheckpoint = summary
        return summary
    }

    @discardableResult
    public func performBackgroundWALCheckpoint() async throws -> FabushiWalCheckpointSummary {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        #if canImport(UIKit)
        let bgGuard = BackgroundTaskGuard()
        let taskId = UIApplication.shared.beginBackgroundTask(
            withName: "com.ombhrum.fabushi.wal-checkpoint"
        ) {
            bgGuard.endIfNeeded()
        }
        bgGuard.store(taskId)
        defer { bgGuard.endIfNeeded() }
        #endif
        return try await checkpointWAL()
    }

    public func handleScenePhaseChange(_ phase: ScenePhase) {
        guard isStarted else { return }
        if phase == .background {
            Task { @MainActor [weak self] in
                guard let self, self.isStarted else { return }
                do {
                    _ = try await self.performBackgroundWALCheckpoint()
                } catch {
                    self.lastErrorMessage = error.localizedDescription
                }
            }
        }
    }

    // MARK: - Internal Helpers

    private func applyGenerationAdvance(_ newGeneration: UInt64) {
        accountScopeGeneration = newGeneration
        chatStore.resetForAccountScopeChange(generation: newGeneration)
        agentStudioStore.resetForAccountScopeChange(generation: newGeneration)
        remoteComputerStore.resetForAccountScopeChange(generation: newGeneration)
        miniAppHostStore.resetForAccountScopeChange(generation: newGeneration)
        callSessionStore.resetForAccountScopeChange(generation: newGeneration)
        settingsStore.resetForAccountScopeChange(generation: newGeneration)
        vaultStore.resetForAccountScopeChange(generation: newGeneration)
    }

    private func restoreAuthState(from list: FabushiListAccountsResult) {
        guard let activeId = list.activeAccountId, !activeId.isEmpty else {
            activeAccountId = nil
            authState = .unauthenticated
            return
        }
        activeAccountId = activeId
        if let summary = list.accounts.first(where: { $0.accountId == activeId }),
           let uid = summary.userId
        {
            authState = .authenticated(
                userId: uid,
                username: summary.username ?? activeId,
                accountId: activeId
            )
        } else if let uid = UInt64(activeId), uid > 0 {
            authState = .authenticated(userId: uid, username: activeId, accountId: activeId)
        } else {
            authState = .unauthenticated
        }
    }

    private func startEventPump() {
        eventPumpTask?.cancel()
        let stream = host.makeEventStream()
        eventPumpTask = Task { @MainActor [weak self] in
            for await event in stream {
                guard !Task.isCancelled, let self else { break }
                // Generation fencing invariant: drop events from any superseded generation.
                guard event.generation == 0 || event.generation >= self.accountScopeGeneration else {
                    continue
                }
                self.routeDomainEvent(event)
            }
        }
    }

    private func routeDomainEvent(_ event: FabushiDomainEvent) {
        chatStore.handleEvent(event)
        agentStudioStore.handleEvent(event)
        remoteComputerStore.handleEvent(event)
        miniAppHostStore.handleEvent(event)
        callSessionStore.handleEvent(event)
        settingsStore.handleEvent(event)
    }
}
