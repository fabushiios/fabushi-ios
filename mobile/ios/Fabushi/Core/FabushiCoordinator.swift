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
final class BackgroundTaskGuard: Sendable {
    private struct State: Sendable {
        var id: UIBackgroundTaskIdentifier = .invalid
        var ended: Bool = false
    }

    private let lock = OSAllocatedUnfairLock(initialState: State())
    private let ender: @Sendable (UIBackgroundTaskIdentifier) -> Void

    init(
        ender: @escaping @Sendable (UIBackgroundTaskIdentifier) -> Void = {
            BackgroundTaskGuard.defaultSynchronousEnder($0)
        }
    ) {
        self.ender = ender
    }

    static func defaultSynchronousEnder(_ id: UIBackgroundTaskIdentifier) {
        if Thread.isMainThread {
            MainActor.assumeIsolated {
                UIApplication.shared.endBackgroundTask(id)
            }
        } else {
            DispatchQueue.main.sync {
                MainActor.assumeIsolated {
                    UIApplication.shared.endBackgroundTask(id)
                }
            }
        }
    }

    func store(_ newId: UIBackgroundTaskIdentifier) {
        let immediateEndId: UIBackgroundTaskIdentifier? = lock.withLock { state in
            if state.ended {
                return newId != .invalid ? newId : nil
            }
            state.id = newId
            return nil
        }
        if let immediateEndId {
            ender(immediateEndId)
        }
    }

    func endIfNeeded() {
        let idToEnd: UIBackgroundTaskIdentifier? = lock.withLock { state in
            state.ended = true
            let existing = state.id
            state.id = .invalid
            return existing != .invalid ? existing : nil
        }
        if let idToEnd {
            ender(idToEnd)
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
    @ObservationIgnored
    private var coordinatorLifecycleEpoch: UInt64 = 0
    @ObservationIgnored
    private var startTaskCounter: UInt64 = 0
    @ObservationIgnored
    private var inFlightStartId: UInt64 = 0
    @ObservationIgnored
    private var accountSecretsPurgers: [String: @MainActor (String) -> Void] = [:]
    @ObservationIgnored
    private var inFlightStartTask: Task<Void, Error>? = nil
    @ObservationIgnored
    private var shutdownTaskCounter: UInt64 = 0
    @ObservationIgnored
    private var inFlightShutdownId: UInt64 = 0
    @ObservationIgnored
    private var inFlightShutdownTask: Task<Void, Never>? = nil

    internal var hasInFlightStart: Bool {
        inFlightStartTask != nil
    }

    internal var hasInFlightShutdown: Bool {
        inFlightShutdownTask != nil
    }

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
        while true {
            if let shutdownTask = inFlightShutdownTask {
                await shutdownTask.value
                continue
            }
            if isStarted {
                return
            }
            if let existingTask = inFlightStartTask {
                try await existingTask.value
                return
            }

            let expectedEpoch = coordinatorLifecycleEpoch
            startTaskCounter &+= 1
            let taskId = startTaskCounter

            let task = Task<Void, Error> { @MainActor in
                try await self.performStart(
                    appDataDirectory: appDataDirectory,
                    serverURL: serverURL,
                    testMode: testMode,
                    startNetworkMonitor: startNetworkMonitor,
                    expectedEpoch: expectedEpoch,
                    taskId: taskId
                )
            }
            inFlightStartId = taskId
            inFlightStartTask = task
            try await task.value
            return
        }
    }

    private func performStart(
        appDataDirectory: URL?,
        serverURL: String?,
        testMode: Bool,
        startNetworkMonitor: Bool,
        expectedEpoch: UInt64,
        taskId: UInt64
    ) async throws {
        defer {
            if inFlightStartId == taskId {
                inFlightStartTask = nil
            }
        }

        do {
            guard coordinatorLifecycleEpoch == expectedEpoch else {
                throw FabushiHostError.hostClosed(message: "FabushiCoordinator shut down before start began")
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
            guard coordinatorLifecycleEpoch == expectedEpoch else {
                throw FabushiHostError.hostClosed(message: "FabushiCoordinator shut down during host initialization")
            }

            let liveGeneration = try await host.currentGeneration()
            guard coordinatorLifecycleEpoch == expectedEpoch else {
                throw FabushiHostError.hostClosed(message: "FabushiCoordinator shut down during generation sync")
            }

            let urlInfo = try? await host.getServerURL()
            guard coordinatorLifecycleEpoch == expectedEpoch else {
                throw FabushiHostError.hostClosed(message: "FabushiCoordinator shut down during server URL sync")
            }

            let accountsInfo = try? await host.listAccounts(generation: liveGeneration)
            guard coordinatorLifecycleEpoch == expectedEpoch else {
                throw FabushiHostError.hostClosed(message: "FabushiCoordinator shut down during account restoration")
            }

            // Commit all startup state atomically on @MainActor after the final await.
            let syncedGeneration = max(1, liveGeneration)
            accountScopeGeneration = syncedGeneration
            resetAllDomainStores(generation: syncedGeneration)
            if let urlInfo {
                settingsStore.updateServerURL(urlInfo.serverUrl)
            }
            if let accountsInfo {
                restoreAuthState(from: accountsInfo)
            } else {
                activeAccountId = nil
                authState = .unauthenticated
            }
            lastErrorMessage = nil
            isStarted = true

            startEventPump(capturedEpoch: expectedEpoch)

            if startNetworkMonitor {
                let capturedEpoch = expectedEpoch
                networkMonitor.start(host: host) { [weak self] snapshot in
                    Task { @MainActor [weak self] in
                        guard let self,
                              self.isStarted,
                              self.coordinatorLifecycleEpoch == capturedEpoch
                        else {
                            return
                        }
                        self.isOnline = self.networkMonitor.currentSnapshot?.isOnline ?? snapshot.isOnline
                    }
                }
            }
        } catch {
            eventPumpTask?.cancel()
            eventPumpTask = nil
            networkMonitor.stop()
            if coordinatorLifecycleEpoch == expectedEpoch {
                isStarted = false
                activeAccountId = nil
                authState = .unauthenticated
                lastErrorMessage = error.localizedDescription
            }
            throw error
        }
    }

    public func shutdown() async {
        coordinatorLifecycleEpoch &+= 1
        clearCoordinatorStateForShutdown()

        if let existingShutdown = inFlightShutdownTask {
            await existingShutdown.value
            return
        }

        shutdownTaskCounter &+= 1
        let taskId = shutdownTaskCounter
        let task = Task<Void, Never> { @MainActor in
            await self.performShutdown(taskId: taskId)
        }
        inFlightShutdownId = taskId
        inFlightShutdownTask = task
        await task.value
    }

    private func performShutdown(taskId: UInt64) async {
        defer {
            if inFlightShutdownId == taskId {
                inFlightShutdownTask = nil
            }
        }

        // Wait for any in-flight start() to settle first so it cannot resurrect a host post-shutdown.
        while let startTask = inFlightStartTask {
            _ = try? await startTask.value
        }

        eventPumpTask?.cancel()
        eventPumpTask = nil
        networkMonitor.stop()

        await host.shutdown()

        clearCoordinatorStateForShutdown()
    }

    // MARK: - Generation Fencing Invariant

    /// Executes an async operation fenced by both lifecycle epoch and `accountScopeGeneration`.
    /// Checks lifecycle + generation before AND after `await`, dropping stale completions without mutating state.
    public func executeFenced<T: Sendable>(
        _ operation: @Sendable (UInt64) async throws -> T
    ) async throws -> T {
        guard isStarted else {
            throw FabushiCoordinatorError.notStarted
        }
        let capturedEpoch = coordinatorLifecycleEpoch
        let capturedGen = accountScopeGeneration
        let value = try await operation(capturedGen)
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
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
        let capturedEpoch = coordinatorLifecycleEpoch
        let capturedGen = accountScopeGeneration
        let capturedAccount = activeAccountId
        let envelope: FabushiRPCEnvelopeResult<Res> = try await host.dispatchEnvelope(
            method: method,
            params: params,
            generation: capturedGen,
            accountId: capturedAccount
        )
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard accountScopeGeneration == capturedGen else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: capturedGen,
                current: accountScopeGeneration
            )
        }
        return envelope.result
    }

    // MARK: - Authoritative Account & Session Transitions

    /// Reconciles Swift coordinator generation and active account state from the authoritative
    /// Rust host, enforcing lifecycle epoch fencing and monotonic generation progression.
    @discardableResult
    public func syncAuthoritativeSessionState() async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let expectedEpoch = coordinatorLifecycleEpoch

        let syncedGeneration = try await host.currentGeneration()
        guard isStarted, coordinatorLifecycleEpoch == expectedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard syncedGeneration >= accountScopeGeneration else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: syncedGeneration,
                current: accountScopeGeneration
            )
        }
        if syncedGeneration > accountScopeGeneration {
            applyGenerationAdvance(syncedGeneration)
        }

        let accountsInfo = try await host.listAccounts(generation: syncedGeneration)
        guard isStarted, coordinatorLifecycleEpoch == expectedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard accountScopeGeneration == syncedGeneration else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: syncedGeneration,
                current: accountScopeGeneration
            )
        }

        restoreAuthState(from: accountsInfo)
        if case .authenticated(_, _, let activeId) = authState {
            _ = try? keychain.loadOrCreateAccountKey(accountId: activeId)
        }
        return accountScopeGeneration
    }

    /// Dispatches an `auth.*` RPC that may atomically establish a session and bump Rust's
    /// generation inside `finalize_authenticated_session`, adopting the authoritative generation,
    /// active account identity, and per-account Keychain key without double-calling `session.setAuthSession`.
    public func dispatchAuthTransition<Req: Encodable & Sendable, Res: Decodable & Sendable>(
        method: String,
        params: Req,
        extractIdentity: (Res) -> (userId: UInt64, username: String)?
    ) async throws -> FabushiRPCEnvelopeResult<Res> {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let capturedEpoch = coordinatorLifecycleEpoch
        let capturedGen = accountScopeGeneration
        let capturedAccount = activeAccountId

        let envelope: FabushiRPCEnvelopeResult<Res> = try await host.dispatchEnvelope(
            method: method,
            params: params,
            generation: capturedGen,
            accountId: capturedAccount
        )

        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard envelope.generation >= accountScopeGeneration else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: envelope.generation,
                current: accountScopeGeneration
            )
        }

        if let identity = extractIdentity(envelope.result), identity.userId > 0 {
            let accountId = String(identity.userId)
            let trimmedUsername = identity.username.trimmingCharacters(in: .whitespacesAndNewlines)
            let resolvedUsername = trimmedUsername.isEmpty ? "user_\(identity.userId)" : trimmedUsername
            applyGenerationAdvance(envelope.generation)
            activeAccountId = accountId
            authState = .authenticated(
                userId: identity.userId,
                username: resolvedUsername,
                accountId: accountId
            )
            _ = try? keychain.loadOrCreateAccountKey(accountId: accountId)
        } else if envelope.generation > accountScopeGeneration {
            await reconcileAuthoritativeSessionState(expectedEpoch: capturedEpoch)
            guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
                throw FabushiCoordinatorError.notStarted
            }
            if case .authenticated(_, _, let activeId) = authState {
                _ = try? keychain.loadOrCreateAccountKey(accountId: activeId)
            }
        }

        return envelope
    }

    @discardableResult
    public func setAuthSession(_ params: FabushiSessionAuthParams) async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let capturedEpoch = coordinatorLifecycleEpoch
        let res = try await host.setAuthSession(params, generation: accountScopeGeneration)
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard res.generation >= accountScopeGeneration else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: res.generation,
                current: accountScopeGeneration
            )
        }
        let accountId = String(res.userId)
        applyGenerationAdvance(res.generation)
        activeAccountId = accountId
        authState = .authenticated(userId: res.userId, username: params.username, accountId: accountId)
        return res.generation
    }

    @discardableResult
    public func switchAccount(accountId: String) async throws -> UInt64 {
        try await switchAccount(accountId: accountId, beforeUsernameLookup: nil)
    }

    @discardableResult
    internal func switchAccount(
        accountId: String,
        beforeUsernameLookup: (@MainActor () async -> Void)?
    ) async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let capturedEpoch = coordinatorLifecycleEpoch
        let res = try await host.switchAccount(accountId: accountId, generation: accountScopeGeneration)
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard res.generation >= accountScopeGeneration else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: res.generation,
                current: accountScopeGeneration
            )
        }
        applyGenerationAdvance(res.generation)
        activeAccountId = res.accountId
        // Set immediate consistent authState synchronously BEFORE suspending on listAccounts
        // so there is zero transient cross-account split-brain during the await.
        authState = .authenticated(userId: res.userId, username: res.accountId, accountId: res.accountId)

        if let beforeUsernameLookup {
            await beforeUsernameLookup()
            guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
                throw FabushiCoordinatorError.notStarted
            }
        }

        let accountsInfo = try? await host.listAccounts(generation: res.generation)
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard accountScopeGeneration == res.generation else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: res.generation,
                current: accountScopeGeneration
            )
        }
        if let resolvedUsername = accountsInfo?.accounts
            .first(where: { $0.accountId == res.accountId })?
            .username,
           !resolvedUsername.isEmpty
        {
            authState = .authenticated(
                userId: res.userId,
                username: resolvedUsername,
                accountId: res.accountId
            )
        }
        return res.generation
    }

    // MARK: - Account-Scoped Secrets Purge Hook (D7)

    public func registerAccountSecretsPurger(
        id: String = "default",
        _ purger: @escaping @MainActor (String) -> Void
    ) {
        accountSecretsPurgers[id] = purger
    }

    public func unregisterAccountSecretsPurger(id: String) {
        accountSecretsPurgers.removeValue(forKey: id)
    }

    private func purgeDepartingAccountSecrets(accountId: String?) {
        guard let accountId, !accountId.isEmpty else { return }
        try? keychain.deleteAccountKey(accountId: accountId)
        for purger in accountSecretsPurgers.values {
            purger(accountId)
        }
    }

    @discardableResult
    public func clearLocalSession() async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let capturedEpoch = coordinatorLifecycleEpoch
        let departingAccountId = activeAccountId
        let res = try await host.clearSession(generation: accountScopeGeneration)
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard res.generation >= accountScopeGeneration else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: res.generation,
                current: accountScopeGeneration
            )
        }
        applyGenerationAdvance(res.generation)
        activeAccountId = nil
        authState = .unauthenticated
        purgeDepartingAccountSecrets(accountId: departingAccountId)
        return res.generation
    }

    @discardableResult
    public func logout() async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let capturedEpoch = coordinatorLifecycleEpoch
        let departingAccountId = activeAccountId
        do {
            let res = try await host.logout(generation: accountScopeGeneration)
            guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
                throw FabushiCoordinatorError.notStarted
            }
            guard res.generation >= accountScopeGeneration else {
                throw FabushiCoordinatorError.staleGeneration(
                    captured: res.generation,
                    current: accountScopeGeneration
                )
            }
            applyGenerationAdvance(res.generation)
            activeAccountId = nil
            authState = .unauthenticated
            purgeDepartingAccountSecrets(accountId: departingAccountId)
            return res.generation
        } catch let coordinatorError as FabushiCoordinatorError {
            throw coordinatorError
        } catch let hostError as FabushiHostError where hostError.code == "stale_generation" {
            guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
                throw FabushiCoordinatorError.notStarted
            }
            // Rust rejected auth.logout pre-dispatch because another transition already
            // advanced generation; NEVER blindly wipe the newer active session or purge its secrets.
            await reconcileAuthoritativeSessionState(expectedEpoch: capturedEpoch)
            guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
                throw FabushiCoordinatorError.notStarted
            }
            throw hostError
        } catch {
            guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
                throw FabushiCoordinatorError.notStarted
            }
            // If remote logout failed (e.g. offline), Rust may have locally wiped the session
            // and bumped generation; reconcile authoritative generation and account state.
            await reconcileAuthoritativeSessionState(expectedEpoch: capturedEpoch)
            guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
                throw FabushiCoordinatorError.notStarted
            }
            if authState == .unauthenticated {
                purgeDepartingAccountSecrets(accountId: departingAccountId)
            }
            throw error
        }
    }

    @discardableResult
    public func setServerURL(_ serverUrl: String) async throws -> UInt64 {
        guard isStarted else { throw FabushiCoordinatorError.notStarted }
        let capturedEpoch = coordinatorLifecycleEpoch
        let res = try await host.setServerURL(serverUrl, generation: accountScopeGeneration)
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        guard res.generation >= accountScopeGeneration else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: res.generation,
                current: accountScopeGeneration
            )
        }
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
        let capturedEpoch = coordinatorLifecycleEpoch
        // Omit generation (nil) so device-wide SQLite WAL truncation (0xdead10cc protection)
        // is NEVER rejected with stale_generation during an account transition.
        let summary = try await host.checkpointWAL(generation: nil)
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            throw FabushiCoordinatorError.notStarted
        }
        lastWalCheckpoint = summary
        return summary
    }

    @discardableResult
    public func performBackgroundWALCheckpoint() async throws -> FabushiWalCheckpointSummary {
        #if canImport(UIKit)
        return try await performBackgroundWALCheckpoint(existingGuard: nil)
        #else
        return try await checkpointWAL()
        #endif
    }

    #if canImport(UIKit)
    private func acquireBackgroundWALGuard() -> BackgroundTaskGuard {
        let bgGuard = BackgroundTaskGuard()
        let taskId = UIApplication.shared.beginBackgroundTask(
            withName: "com.ombhrum.fabushi.wal-checkpoint"
        ) {
            bgGuard.endIfNeeded()
        }
        bgGuard.store(taskId)
        return bgGuard
    }

    @discardableResult
    private func performBackgroundWALCheckpoint(
        existingGuard: BackgroundTaskGuard?
    ) async throws -> FabushiWalCheckpointSummary {
        guard isStarted else {
            existingGuard?.endIfNeeded()
            throw FabushiCoordinatorError.notStarted
        }
        let bgGuard = existingGuard ?? acquireBackgroundWALGuard()
        defer { bgGuard.endIfNeeded() }
        return try await checkpointWAL()
    }
    #endif

    public func handleScenePhaseChange(_ phase: ScenePhase) {
        guard isStarted else { return }
        if phase == .background {
            let capturedEpoch = coordinatorLifecycleEpoch
            #if canImport(UIKit)
            // Acquire UIBackgroundTaskIdentifier SYNCHRONOUSLY before spawning the async Task
            // so iOS cannot suspend the process before WAL checkpoint begins (0xdead10cc).
            let bgGuard = acquireBackgroundWALGuard()
            Task { @MainActor [weak self] in
                guard let self, self.isStarted, self.coordinatorLifecycleEpoch == capturedEpoch else {
                    bgGuard.endIfNeeded()
                    return
                }
                do {
                    _ = try await self.performBackgroundWALCheckpoint(existingGuard: bgGuard)
                } catch {
                    if self.isStarted, self.coordinatorLifecycleEpoch == capturedEpoch {
                        self.lastErrorMessage = error.localizedDescription
                    }
                }
            }
            #else
            Task { @MainActor [weak self] in
                guard let self, self.isStarted, self.coordinatorLifecycleEpoch == capturedEpoch else { return }
                do {
                    _ = try await self.performBackgroundWALCheckpoint()
                } catch {
                    if self.isStarted, self.coordinatorLifecycleEpoch == capturedEpoch {
                        self.lastErrorMessage = error.localizedDescription
                    }
                }
            }
            #endif
        }
    }

    // MARK: - Internal Helpers

    private func clearCoordinatorStateForShutdown() {
        isStarted = false
        activeAccountId = nil
        authState = .unauthenticated
        accountScopeGeneration = 1
        eventPumpTask?.cancel()
        eventPumpTask = nil
        networkMonitor.stop()
        resetAllDomainStores(generation: 1)
    }

    private func resetAllDomainStores(generation: UInt64) {
        chatStore.resetForAccountScopeChange(generation: generation)
        agentStudioStore.resetForAccountScopeChange(generation: generation)
        remoteComputerStore.resetForAccountScopeChange(generation: generation)
        miniAppHostStore.resetForAccountScopeChange(generation: generation)
        callSessionStore.resetForAccountScopeChange(generation: generation)
        settingsStore.resetForAccountScopeChange(generation: generation)
        vaultStore.resetForAccountScopeChange(generation: generation)
    }

    @discardableResult
    private func applyGenerationAdvance(_ newGeneration: UInt64) -> Bool {
        guard newGeneration >= accountScopeGeneration else {
            return false
        }
        accountScopeGeneration = newGeneration
        resetAllDomainStores(generation: newGeneration)
        return true
    }

    private func reconcileAuthoritativeSessionState(expectedEpoch: UInt64? = nil) async {
        let capturedEpoch = expectedEpoch ?? coordinatorLifecycleEpoch
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            return
        }
        guard let synced = try? await host.currentGeneration() else {
            return
        }
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            return
        }
        guard synced >= accountScopeGeneration else {
            return
        }
        if synced > accountScopeGeneration {
            applyGenerationAdvance(synced)
        }
        guard let accountsInfo = try? await host.listAccounts(generation: synced) else {
            return
        }
        guard isStarted, coordinatorLifecycleEpoch == capturedEpoch else {
            return
        }
        if accountScopeGeneration == synced {
            restoreAuthState(from: accountsInfo)
        }
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

    private func startEventPump(capturedEpoch: UInt64) {
        eventPumpTask?.cancel()
        let stream = host.makeEventStream()
        eventPumpTask = Task { @MainActor [weak self] in
            for await event in stream {
                guard !Task.isCancelled,
                      let self,
                      self.isStarted,
                      self.coordinatorLifecycleEpoch == capturedEpoch
                else {
                    break
                }
                self.ingestDomainEvent(event)
            }
        }
    }

    /// Strict generation-fenced event intake: drops any event when not started or whose generation
    /// is strictly less than `accountScopeGeneration` (including generation 0 when active generation >= 1).
    @discardableResult
    internal func ingestDomainEvent(_ event: FabushiDomainEvent) -> Bool {
        guard isStarted, event.generation >= accountScopeGeneration else {
            return false
        }
        routeDomainEvent(event)
        return true
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
