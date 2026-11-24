import Foundation
import os

// MARK: - Internal Thread-Safe FFI Support Boxes

private struct SendableRawHandle: @unchecked Sendable {
    let pointer: UnsafeMutableRawPointer
}

private struct HostHandleState: @unchecked Sendable {
    var pointer: UnsafeMutableRawPointer?
    var everInitialized: Bool = false
    var closed: Bool = false
}

private final class HostHandleBox: Sendable {
    private let lock = OSAllocatedUnfairLock(initialState: HostHandleState())

    func current() -> SendableRawHandle? {
        lock.withLock { state in
            state.pointer.map(SendableRawHandle.init(pointer:))
        }
    }

    func status() -> (hasHandle: Bool, everInitialized: Bool, closed: Bool) {
        lock.withLock { state in
            (state.pointer != nil, state.everInitialized, state.closed)
        }
    }

    func install(_ handle: SendableRawHandle) {
        lock.withLock { state in
            state.pointer = handle.pointer
            state.everInitialized = true
            state.closed = false
        }
    }

    func takeAndMarkClosed() -> SendableRawHandle? {
        lock.withLock { state in
            let existing = state.pointer.map(SendableRawHandle.init(pointer:))
            state.pointer = nil
            if state.everInitialized {
                state.closed = true
            }
            return existing
        }
    }
}

private final class DispatchCancelState: Sendable {
    private enum Phase: Sendable {
        case pending
        case dispatched
        case finished
    }

    private struct State: Sendable {
        var phase: Phase = .pending
        var cancelled: Bool = false
    }

    private let lock = OSAllocatedUnfairLock(initialState: State())

    func beginDispatchIfNotCancelled() -> Bool {
        lock.withLock { state in
            if state.cancelled {
                state.phase = .finished
                return false
            }
            state.phase = .dispatched
            return true
        }
    }

    /// Marks the request cancelled. Returns `true` iff FFI dispatch is currently in flight
    /// and `fabushi_host_cancel` should be invoked on `cancelQueue`.
    func markCancelled() -> Bool {
        lock.withLock { state in
            state.cancelled = true
            return state.phase == .dispatched
        }
    }

    func markFinished() {
        lock.withLock { state in
            state.phase = .finished
        }
    }

    var isFinished: Bool {
        lock.withLock { $0.phase == .finished }
    }
}

private func dispatchCancelAttempt(
    attempt: Int,
    maxAttempts: Int,
    requestId: String,
    cancelState: DispatchCancelState,
    handleBox: HostHandleBox,
    rpcGroup: DispatchGroup,
    cancelQueue: DispatchQueue
) {
    if cancelState.isFinished {
        return
    }
    let didCancel: Bool? = {
        rpcGroup.enter()
        defer { rpcGroup.leave() }
        guard let handle = handleBox.current() else {
            return nil
        }
        return requestId.withCString { cReqId in
            fabushi_host_cancel(handle.pointer, cReqId)
        }
    }()
    guard let didCancel else {
        return
    }
    if didCancel || attempt + 1 >= maxAttempts {
        return
    }
    cancelQueue.asyncAfter(deadline: .now() + .milliseconds(5)) {
        dispatchCancelAttempt(
            attempt: attempt + 1,
            maxAttempts: maxAttempts,
            requestId: requestId,
            cancelState: cancelState,
            handleBox: handleBox,
            rpcGroup: rpcGroup,
            cancelQueue: cancelQueue
        )
    }
}

private final class PollControlBox: Sendable {
    private struct State: Sendable {
        var token: UInt64 = 0
        var active: Bool = false
    }

    private let lock = OSAllocatedUnfairLock(initialState: State())

    func beginSession() -> UInt64 {
        lock.withLock { state in
            state.token &+= 1
            state.active = true
            return state.token
        }
    }

    func stop() {
        lock.withLock { state in
            state.token &+= 1
            state.active = false
        }
    }

    func isSessionActive(_ token: UInt64) -> Bool {
        lock.withLock { state in
            state.active && state.token == token
        }
    }
}

private final class DomainEventBroadcaster: Sendable {
    private let lock = OSAllocatedUnfairLock<[UUID: AsyncStream<FabushiDomainEvent>.Continuation]>(
        initialState: [:]
    )

    func makeStream(bufferCapacity: Int = 1024) -> AsyncStream<FabushiDomainEvent> {
        let id = UUID()
        let capacity = max(1, bufferCapacity)
        return AsyncStream(bufferingPolicy: .bufferingNewest(capacity)) { continuation in
            lock.withLock { map in
                map[id] = continuation
            }
            continuation.onTermination = { [weak self] _ in
                self?.lock.withLock { map in
                    _ = map.removeValue(forKey: id)
                }
            }
        }
    }

    func yield(_ event: FabushiDomainEvent) {
        let continuations = lock.withLock { Array($0.values) }
        for continuation in continuations {
            continuation.yield(event)
        }
    }

    func finishAll() {
        let continuations = lock.withLock { map -> [AsyncStream<FabushiDomainEvent>.Continuation] in
            let values = Array(map.values)
            map.removeAll()
            return values
        }
        for continuation in continuations {
            continuation.finish()
        }
    }
}

private struct PollEventsResultPayload: Decodable, Sendable {
    let events: [FabushiDomainEvent.WireHostEvent]
}

// MARK: - Canonical Swift 6 Rust Host Actor

/// Canonical Swift 6 actor wrapping the verified M1b Rust C-ABI (`fabushi_host_*`).
///
/// Concurrency & Safety Invariants:
/// - Four dedicated off-cooperative-pool GCD queues (`lifecycleQueue`, concurrent `rpcQueue`,
///   serial `pollQueue`, high-priority `cancelQueue`) prevent Swift cooperative thread-pool
///   starvation and RPC head-of-line blocking.
/// - Refcounted handle teardown via `HostHandleBox` + `lifecycleQueue` guarantees zero UAF
///   and non-blocking `deinit`.
/// - Race-free `withTaskCancellationHandler` bridges Swift `Task.cancel()` directly to
///   `fabushi_host_cancel`.
/// - Event streaming polls `fabushi_host_poll_events` on `pollQueue` into bounded
///   `AsyncStream<FabushiDomainEvent>` fan-out subscribers (zero C-callback re-entrancy deadlocks).
public actor FabushiRustHostActor {
    public static let shared = FabushiRustHostActor()
    public static let defaultEventBufferCapacity = 1024
    public static let defaultPollBatchSize: UInt32 = 64
    public static let defaultPollTimeoutMs: UInt32 = 250

    private let handleBox = HostHandleBox()
    private let pollControl = PollControlBox()
    private let pollGroup = DispatchGroup()
    private let rpcGroup = DispatchGroup()
    private let eventBroadcaster = DomainEventBroadcaster()

    private let lifecycleQueue = DispatchQueue(
        label: "com.ombhrum.fabushi.host.lifecycle",
        qos: .userInitiated
    )
    private let rpcQueue = DispatchQueue(
        label: "com.ombhrum.fabushi.host.rpc",
        qos: .userInitiated,
        attributes: .concurrent
    )
    private let pollQueue = DispatchQueue(
        label: "com.ombhrum.fabushi.host.poll",
        qos: .utility
    )
    private let cancelQueue = DispatchQueue(
        label: "com.ombhrum.fabushi.host.cancel",
        qos: .userInteractive,
        attributes: .concurrent
    )

    private var lifecycleEpoch: UInt64 = 0
    private var lifecycleTaskCounter: UInt64 = 0
    private var inFlightInit: (id: UInt64, task: Task<Void, Error>)? = nil
    private var inFlightShutdown: (id: UInt64, task: Task<Void, Never>)? = nil

    public private(set) var latestObservedGeneration: UInt64 = 1
    public private(set) var activeConfig: FabushiHostConfig? = nil

    public init() {}

    deinit {
        pollControl.stop()
        eventBroadcaster.finishAll()
        if let raw = handleBox.takeAndMarkClosed() {
            lifecycleQueue.async {
                fabushi_host_destroy(raw.pointer)
            }
        }
    }

    // MARK: - Event Streams (Bounded Multi-Subscriber Fan-Out)

    public nonisolated var events: AsyncStream<FabushiDomainEvent> {
        eventBroadcaster.makeStream(bufferCapacity: Self.defaultEventBufferCapacity)
    }

    public nonisolated func makeEventStream(
        bufferCapacity: Int = defaultEventBufferCapacity
    ) -> AsyncStream<FabushiDomainEvent> {
        eventBroadcaster.makeStream(bufferCapacity: bufferCapacity)
    }

    public var isInitialized: Bool {
        handleBox.current() != nil
    }

    internal var hasInFlightInitialization: Bool {
        inFlightInit != nil
    }

    // MARK: - Lifecycle

    public func initialize(config: FabushiHostConfig) async throws {
        while true {
            if let shutdownEntry = inFlightShutdown {
                await shutdownEntry.task.value
                continue
            }
            if handleBox.current() != nil {
                return
            }
            if let initEntry = inFlightInit {
                try await initEntry.task.value
                return
            }

            lifecycleTaskCounter &+= 1
            let taskId = lifecycleTaskCounter
            let expectedEpoch = lifecycleEpoch
            let task = Task<Void, Error> {
                try await self.performInitialize(
                    config: config,
                    expectedEpoch: expectedEpoch,
                    taskId: taskId
                )
            }
            inFlightInit = (id: taskId, task: task)
            try await task.value
            return
        }
    }

    private func performInitialize(
        config: FabushiHostConfig,
        expectedEpoch: UInt64,
        taskId: UInt64
    ) async throws {
        defer {
            if inFlightInit?.id == taskId {
                inFlightInit = nil
            }
        }

        guard lifecycleEpoch == expectedEpoch else {
            throw FabushiHostError.hostClosed(message: "Rust host shut down before initialization started")
        }
        if handleBox.current() != nil {
            return
        }

        if let dir = config.appDataDir, !dir.isEmpty {
            let dirURL = URL(fileURLWithPath: dir, isDirectory: true)
            try FileManager.default.createDirectory(at: dirURL, withIntermediateDirectories: true)
        }

        let encoder = JSONEncoder()
        let configData: Data
        do {
            configData = try encoder.encode(config)
        } catch {
            throw FabushiHostError.serializationFailed(message: error.localizedDescription)
        }
        guard let configJSON = String(data: configData, encoding: .utf8) else {
            throw FabushiHostError.serializationFailed(message: "UTF-8 encoding failed for HostConfig")
        }

        let createdHandle: SendableRawHandle = try await withCheckedThrowingContinuation { continuation in
            lifecycleQueue.async {
                let rawPtr: UnsafeMutableRawPointer? = configJSON.withCString { cConfig in
                    fabushi_host_create(cConfig)
                }
                guard let rawPtr else {
                    // MUST read borrowed thread-local error immediately on the SAME thread and NEVER free it!
                    let errorMessage = fabushi_host_last_error().map { String(cString: $0) }
                        ?? "Unknown fabushi_host_create failure"
                    continuation.resume(throwing: FabushiHostError.initializationFailed(message: errorMessage))
                    return
                }
                continuation.resume(returning: SendableRawHandle(pointer: rawPtr))
            }
        }

        // If shutdown() arrived while fabushi_host_create ran on lifecycleQueue, destroy the
        // created handle immediately off-actor WITHOUT installing it or starting pollQueue.
        guard lifecycleEpoch == expectedEpoch else {
            await destroyRawHandleOffActor(createdHandle)
            throw FabushiHostError.hostClosed(message: "Rust host shut down during initialization")
        }

        handleBox.install(createdHandle)
        activeConfig = config

        let pollToken = pollControl.beginSession()
        startEventPollLoop(handle: createdHandle, pollToken: pollToken)

        if let gen = try? await currentGeneration() {
            if lifecycleEpoch == expectedEpoch {
                latestObservedGeneration = max(1, gen)
            }
        }

        if lifecycleEpoch != expectedEpoch {
            throw FabushiHostError.hostClosed(message: "Rust host shut down during initialization")
        }
    }

    private func destroyRawHandleOffActor(_ handle: SendableRawHandle) async {
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            lifecycleQueue.async {
                fabushi_host_destroy(handle.pointer)
                continuation.resume()
            }
        }
    }

    public func shutdown() async {
        lifecycleEpoch &+= 1
        if let shutdownEntry = inFlightShutdown {
            await shutdownEntry.task.value
            return
        }

        lifecycleTaskCounter &+= 1
        let taskId = lifecycleTaskCounter
        let task = Task<Void, Never> {
            await self.performShutdown(taskId: taskId)
        }
        inFlightShutdown = (id: taskId, task: task)
        await task.value
    }

    private func performShutdown(taskId: UInt64) async {
        defer {
            if inFlightShutdown?.id == taskId {
                inFlightShutdown = nil
            }
        }

        // Wait for any in-flight initialize() to settle first so it cannot resurrect a handle post-shutdown.
        while let initEntry = inFlightInit {
            _ = try? await initEntry.task.value
        }

        pollControl.stop()
        let removed = handleBox.takeAndMarkClosed()
        eventBroadcaster.finishAll()
        activeConfig = nil
        latestObservedGeneration = 1

        let pGroup = pollGroup
        let rGroup = rpcGroup
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            lifecycleQueue.async {
                if let removed {
                    fabushi_host_destroy(removed.pointer)
                }
                // Wait off the Swift cooperative pool until both pollQueue and all in-flight
                // rpcQueue dispatches exit and drop every Rust Arc<FabushiAppHost> clone
                // (releasing SQLite WAL & Tantivy IndexWriter file locks deterministically).
                pGroup.wait()
                rGroup.wait()
                continuation.resume()
            }
        }
    }

    // MARK: - Typed JSON-RPC Dispatch with Cooperative Cancellation

    public func dispatchEnvelope<Req: Encodable & Sendable, Res: Decodable & Sendable>(
        method: String,
        params: Req,
        generation: UInt64? = nil,
        accountId: String? = nil
    ) async throws -> FabushiRPCEnvelopeResult<Res> {
        if Task.isCancelled {
            throw CancellationError()
        }

        let status = handleBox.status()
        guard status.hasHandle else {
            if status.closed {
                throw FabushiHostError.hostClosed(message: "Rust host has been shut down")
            }
            throw FabushiHostError.notInitialized
        }

        let requestId = UUID().uuidString
        let requestEnvelope = FabushiRPCRequest(
            id: requestId,
            method: method,
            generation: generation,
            accountId: accountId,
            params: params
        )

        let requestData: Data
        do {
            requestData = try JSONEncoder().encode(requestEnvelope)
        } catch {
            throw FabushiHostError.serializationFailed(message: error.localizedDescription)
        }
        guard let requestJSON = String(data: requestData, encoding: .utf8) else {
            throw FabushiHostError.serializationFailed(message: "Failed to encode RPC request as UTF-8")
        }

        let capturedEpoch = lifecycleEpoch
        let rawResponse = try await dispatchRawJSON(requestId: requestId, requestJSON: requestJSON)
        guard let responseData = rawResponse.data(using: .utf8) else {
            throw FabushiHostError.invalidResponse(raw: rawResponse)
        }

        let decoded: FabushiRPCResponse<Res>
        do {
            decoded = try JSONDecoder().decode(FabushiRPCResponse<Res>.self, from: responseData)
        } catch {
            // Check whether Rust returned a valid fail-closed error envelope whose `result` is absent.
            if let errorOnly = try? JSONDecoder().decode(FabushiRPCResponse<FabushiJSONValue>.self, from: responseData),
               !errorOnly.ok,
               let errPayload = errorOnly.error
            {
                let respGen = errorOnly.generation ?? latestObservedGeneration
                if lifecycleEpoch == capturedEpoch, respGen > latestObservedGeneration {
                    latestObservedGeneration = respGen
                }
                throw FabushiHostError.fromRPCError(
                    errPayload,
                    method: method,
                    requestId: requestId,
                    requestedGeneration: generation,
                    responseGeneration: respGen
                )
            }
            throw FabushiHostError.invalidResponse(raw: rawResponse)
        }

        let respGeneration = decoded.generation ?? latestObservedGeneration
        if lifecycleEpoch == capturedEpoch, respGeneration > latestObservedGeneration {
            latestObservedGeneration = respGeneration
        }

        if decoded.ok, let result = decoded.result {
            return FabushiRPCEnvelopeResult(
                id: decoded.id ?? requestId,
                generation: respGeneration,
                result: result
            )
        }

        if let errPayload = decoded.error {
            throw FabushiHostError.fromRPCError(
                errPayload,
                method: method,
                requestId: requestId,
                requestedGeneration: generation,
                responseGeneration: respGeneration
            )
        }

        throw FabushiHostError.invalidResponse(raw: rawResponse)
    }

    public func dispatch<Req: Encodable & Sendable, Res: Decodable & Sendable>(
        method: String,
        params: Req,
        generation: UInt64? = nil,
        accountId: String? = nil
    ) async throws -> Res {
        let envelope: FabushiRPCEnvelopeResult<Res> = try await dispatchEnvelope(
            method: method,
            params: params,
            generation: generation,
            accountId: accountId
        )
        return envelope.result
    }

    public func dispatch<Res: Decodable & Sendable>(
        method: String,
        generation: UInt64? = nil,
        accountId: String? = nil
    ) async throws -> Res {
        try await dispatch(
            method: method,
            params: FabushiEmptyParams(),
            generation: generation,
            accountId: accountId
        )
    }

    // MARK: - Canonical Lifecycle & Domain RPC Convenience Methods

    public func ping(nonce: UInt64 = 0) async throws -> FabushiPingResult {
        struct Params: Encodable, Sendable {
            let nonce: UInt64
        }
        return try await dispatch(
            method: FabushiRPCNamespace.session.method("ping"),
            params: Params(nonce: nonce)
        )
    }

    public func wsStatus() async throws -> FabushiWsStatusResult {
        try await dispatch(method: FabushiRPCNamespace.session.method("wsStatus"))
    }

    public func currentGeneration() async throws -> UInt64 {
        let capturedEpoch = lifecycleEpoch
        let res: FabushiGenerationResult = try await dispatch(
            method: FabushiRPCNamespace.generation.method("current")
        )
        if lifecycleEpoch == capturedEpoch {
            latestObservedGeneration = max(latestObservedGeneration, res.generation)
        }
        return res.generation
    }

    public func bumpGeneration() async throws -> UInt64 {
        let capturedEpoch = lifecycleEpoch
        let res: FabushiGenerationResult = try await dispatch(
            method: FabushiRPCNamespace.generation.method("bump")
        )
        if lifecycleEpoch == capturedEpoch {
            latestObservedGeneration = max(latestObservedGeneration, res.generation)
        }
        return res.generation
    }

    public func setGeneration(_ generation: UInt64) async throws -> UInt64 {
        struct Params: Encodable, Sendable {
            let generation: UInt64
        }
        let capturedEpoch = lifecycleEpoch
        let res: FabushiGenerationResult = try await dispatch(
            method: FabushiRPCNamespace.generation.method("set"),
            params: Params(generation: generation)
        )
        if lifecycleEpoch == capturedEpoch {
            latestObservedGeneration = max(latestObservedGeneration, res.generation)
        }
        return res.generation
    }

    public func validateGeneration(_ generation: UInt64) async throws -> FabushiGenerationValidationResult {
        struct Params: Encodable, Sendable {
            let generation: UInt64
        }
        let capturedEpoch = lifecycleEpoch
        let res: FabushiGenerationValidationResult = try await dispatch(
            method: FabushiRPCNamespace.generation.method("validate"),
            params: Params(generation: generation)
        )
        if lifecycleEpoch == capturedEpoch {
            latestObservedGeneration = max(latestObservedGeneration, res.current)
        }
        return res
    }

    public func setAuthSession(
        _ params: FabushiSessionAuthParams,
        generation: UInt64? = nil
    ) async throws -> FabushiSessionAuthResult {
        try await dispatch(
            method: FabushiRPCNamespace.session.method("setAuthSession"),
            params: params,
            generation: generation
        )
    }

    public func switchAccount(
        accountId: String,
        generation: UInt64? = nil
    ) async throws -> FabushiSwitchAccountResult {
        struct Params: Encodable, Sendable {
            let account_id: String
        }
        return try await dispatch(
            method: FabushiRPCNamespace.session.method("switchAccount"),
            params: Params(account_id: accountId),
            generation: generation,
            accountId: accountId
        )
    }

    public func listAccounts(generation: UInt64? = nil) async throws -> FabushiListAccountsResult {
        try await dispatch(
            method: FabushiRPCNamespace.session.method("listAccounts"),
            generation: generation
        )
    }

    /// Calls `auth.logout` (server revocation + local cache wipe + generation bump).
    public func logout(generation: UInt64? = nil) async throws -> FabushiLogoutResult {
        try await dispatch(
            method: FabushiRPCNamespace.auth.method("logout"),
            generation: generation
        )
    }

    /// Calls `session.clearSession` (local-only logout/wipe + generation bump).
    public func clearSession(generation: UInt64? = nil) async throws -> FabushiClearSessionResult {
        try await dispatch(
            method: FabushiRPCNamespace.session.method("clearSession"),
            generation: generation
        )
    }

    public func setServerURL(
        _ serverUrl: String,
        generation: UInt64? = nil
    ) async throws -> FabushiServerUrlResult {
        struct Params: Encodable, Sendable {
            let server_url: String
        }
        return try await dispatch(
            method: FabushiRPCNamespace.session.method("setServerUrl"),
            params: Params(server_url: serverUrl),
            generation: generation
        )
    }

    public func getServerURL() async throws -> FabushiGetServerUrlResult {
        try await dispatch(method: FabushiRPCNamespace.session.method("getServerUrl"))
    }

    public func checkpointWAL(generation: UInt64? = nil) async throws -> FabushiWalCheckpointSummary {
        try await dispatch(
            method: FabushiRPCNamespace.settings.method("checkpointWal"),
            generation: generation
        )
    }

    public func setNetworkOnline(
        _ online: Bool,
        generation: UInt64? = nil
    ) async throws -> FabushiNetworkStateResult {
        struct Params: Encodable, Sendable {
            let online: Bool
        }
        return try await dispatch(
            method: FabushiRPCNamespace.outbox.method("setNetworkState"),
            params: Params(online: online),
            generation: generation
        )
    }

    public func outboxPendingCount(generation: UInt64? = nil) async throws -> FabushiOutboxPendingCountResult {
        try await dispatch(
            method: FabushiRPCNamespace.outbox.method("pendingCount"),
            generation: generation
        )
    }

    // MARK: - Internal Non-Blocking FFI Dispatch & Event Pump

    private func dispatchRawJSON(requestId: String, requestJSON: String) async throws -> String {
        let cancelState = DispatchCancelState()
        let box = handleBox
        let rQueue = rpcQueue
        let rGroup = rpcGroup
        let cQueue = cancelQueue

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<String, Error>) in
                rGroup.enter()
                rQueue.async {
                    defer { rGroup.leave() }
                    guard cancelState.beginDispatchIfNotCancelled() else {
                        continuation.resume(throwing: CancellationError())
                        return
                    }
                    defer { cancelState.markFinished() }

                    guard let handle = box.current() else {
                        continuation.resume(
                            throwing: FabushiHostError.hostClosed(message: "Rust host handle closed")
                        )
                        return
                    }

                    let responsePtr: UnsafeMutablePointer<CChar>? = requestJSON.withCString { cReq in
                        fabushi_host_dispatch(handle.pointer, cReq)
                    }
                    guard let responsePtr else {
                        continuation.resume(throwing: FabushiHostError.invalidResponse(raw: "<null>"))
                        return
                    }
                    defer { fabushi_host_free_string(responsePtr) }

                    let responseString = String(cString: responsePtr)
                    continuation.resume(returning: responseString)
                }
            }
        } onCancel: {
            let shouldInvokeRustCancel = cancelState.markCancelled()
            guard shouldInvokeRustCancel else { return }
            cQueue.async {
                // Bounded non-blocking retry closes the narrow race window between entering
                // C-ABI dispatch and Rust inserting `requestId` into `in_flight_cancels`.
                dispatchCancelAttempt(
                    attempt: 0,
                    maxAttempts: 40,
                    requestId: requestId,
                    cancelState: cancelState,
                    handleBox: box,
                    rpcGroup: rGroup,
                    cancelQueue: cQueue
                )
            }
        }
    }

    private func startEventPollLoop(handle: SendableRawHandle, pollToken: UInt64) {
        let control = pollControl
        let broadcaster = eventBroadcaster
        let group = pollGroup
        let batchSize = Self.defaultPollBatchSize
        let timeoutMs = Self.defaultPollTimeoutMs

        group.enter()
        pollQueue.async {
            defer { group.leave() }
            let decoder = JSONDecoder()
            while control.isSessionActive(pollToken) {
                let rawPtr = fabushi_host_poll_events(handle.pointer, batchSize, timeoutMs)
                guard let rawPtr else {
                    break
                }
                let rawString = String(cString: rawPtr)
                fabushi_host_free_string(rawPtr)

                guard let data = rawString.data(using: .utf8),
                      let envelope = try? decoder.decode(FabushiRPCResponse<PollEventsResultPayload>.self, from: data)
                else {
                    continue
                }

                if !envelope.ok {
                    if envelope.error?.code.lowercased() == "host_closed" {
                        break
                    }
                    continue
                }

                if let events = envelope.result?.events {
                    for wire in events {
                        broadcaster.yield(FabushiDomainEvent.fromWire(wire))
                    }
                }
            }
        }
    }
}
