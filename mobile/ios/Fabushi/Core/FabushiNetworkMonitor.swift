import Foundation
import Network
import os

// MARK: - Network Snapshot Models

public enum FabushiNetworkInterface: String, Sendable, Equatable {
    case wifi
    case cellular
    case wiredEthernet
    case loopback
    case other
    case none
}

public struct FabushiNetworkSnapshot: Sendable, Equatable {
    public let isOnline: Bool
    public let isExpensive: Bool
    public let isConstrained: Bool
    public let interface: FabushiNetworkInterface

    public init(
        isOnline: Bool,
        isExpensive: Bool = false,
        isConstrained: Bool = false,
        interface: FabushiNetworkInterface = .other
    ) {
        self.isOnline = isOnline
        self.isExpensive = isExpensive
        self.isConstrained = isConstrained
        self.interface = interface
    }

    init(path: NWPath) {
        self.isOnline = path.status == .satisfied
        self.isExpensive = path.isExpensive
        self.isConstrained = path.isConstrained
        if path.status != .satisfied {
            self.interface = .none
        } else if path.usesInterfaceType(.wifi) {
            self.interface = .wifi
        } else if path.usesInterfaceType(.cellular) {
            self.interface = .cellular
        } else if path.usesInterfaceType(.wiredEthernet) {
            self.interface = .wiredEthernet
        } else if path.usesInterfaceType(.loopback) {
            self.interface = .loopback
        } else {
            self.interface = .other
        }
    }
}

// MARK: - Sendable NWPathMonitor Wrapper

private struct MonitorTransfer: @unchecked Sendable {
    let monitor: NWPathMonitor
}

private struct MonitorState: @unchecked Sendable {
    var sessionToken: UInt64 = 0
    var monitor: NWPathMonitor?
    var lastSnapshot: FabushiNetworkSnapshot?
    var lastForwardedOnline: Bool?
    var onTransition: (@Sendable (FabushiNetworkSnapshot) -> Void)?
    var forwardContinuation: AsyncStream<Bool>.Continuation?
    var forwardTask: Task<Void, Never>?
}

/// Thread-safe Apple `Network.NWPathMonitor` bridge running on a dedicated serial utility queue.
/// Deduplicates reachability transitions, fences stale callbacks across `stop()`/`start()` via
/// a monotonic session token, and forwards connectivity changes in strict serial order to
/// `FabushiRustHostActor.setNetworkOnline(_:)` (`outbox.setNetworkState`).
public final class FabushiNetworkMonitor: Sendable {
    private let queue = DispatchQueue(
        label: "com.ombhrum.fabushi.network-monitor",
        qos: .utility
    )
    private let lock = OSAllocatedUnfairLock(initialState: MonitorState())

    public init() {}

    deinit {
        stop()
    }

    public var currentSnapshot: FabushiNetworkSnapshot? {
        lock.withLock { $0.lastSnapshot }
    }

    public func start(
        host: FabushiRustHostActor,
        onTransition: (@Sendable (FabushiNetworkSnapshot) -> Void)? = nil
    ) {
        _ = startSession(host: host, startPathMonitor: true, onTransition: onTransition)
    }

    @discardableResult
    internal func startSession(
        host: FabushiRustHostActor,
        startPathMonitor: Bool,
        onTransition: (@Sendable (FabushiNetworkSnapshot) -> Void)? = nil
    ) -> UInt64 {
        let newTransfer: MonitorTransfer? = startPathMonitor ? MonitorTransfer(monitor: NWPathMonitor()) : nil
        let (onlineStream, onlineContinuation) = AsyncStream<Bool>.makeStream(
            bufferingPolicy: .bufferingNewest(1)
        )
        let forwardTask = Task<Void, Never> {
            for await isOnline in onlineStream {
                guard !Task.isCancelled else { break }
                _ = try? await host.setNetworkOnline(isOnline)
            }
        }

        let (oldMonitor, oldCont, oldTask, activeToken) = lock.withLock {
            state -> (
                MonitorTransfer?,
                AsyncStream<Bool>.Continuation?,
                Task<Void, Never>?,
                UInt64
            ) in
            let prevMonitor = state.monitor.map(MonitorTransfer.init(monitor:))
            let prevCont = state.forwardContinuation
            let prevTask = state.forwardTask

            state.sessionToken &+= 1
            let token = state.sessionToken
            state.monitor = newTransfer?.monitor
            state.lastSnapshot = nil
            state.lastForwardedOnline = nil
            state.onTransition = onTransition
            state.forwardContinuation = onlineContinuation
            state.forwardTask = forwardTask
            return (prevMonitor, prevCont, prevTask, token)
        }

        oldMonitor?.monitor.cancel()
        oldCont?.finish()
        oldTask?.cancel()

        if let transfer = newTransfer {
            transfer.monitor.pathUpdateHandler = { [weak self] path in
                self?.processSnapshot(FabushiNetworkSnapshot(path: path), expectedToken: activeToken)
            }
            transfer.monitor.start(queue: queue)
        }

        return activeToken
    }

    public func stop() {
        let (existingMonitor, existingCont, existingTask) = lock.withLock {
            state -> (
                MonitorTransfer?,
                AsyncStream<Bool>.Continuation?,
                Task<Void, Never>?
            ) in
            let mon = state.monitor.map(MonitorTransfer.init(monitor:))
            let cont = state.forwardContinuation
            let task = state.forwardTask

            state.sessionToken &+= 1
            state.monitor = nil
            state.lastSnapshot = nil
            state.lastForwardedOnline = nil
            state.onTransition = nil
            state.forwardContinuation = nil
            state.forwardTask = nil
            return (mon, cont, task)
        }
        existingMonitor?.monitor.cancel()
        existingCont?.finish()
        existingTask?.cancel()
    }

    /// Finishes the current forwarding stream without cancelling in-flight delivery, awaits
    /// convergence to the latest queued reachability state, and resets monitor session state.
    internal func drainAndStopForTesting() async {
        let (existingMonitor, existingCont, existingTask) = lock.withLock {
            state -> (
                MonitorTransfer?,
                AsyncStream<Bool>.Continuation?,
                Task<Void, Never>?
            ) in
            let mon = state.monitor.map(MonitorTransfer.init(monitor:))
            let cont = state.forwardContinuation
            let task = state.forwardTask

            state.sessionToken &+= 1
            state.monitor = nil
            state.lastSnapshot = nil
            state.lastForwardedOnline = nil
            state.onTransition = nil
            state.forwardContinuation = nil
            state.forwardTask = nil
            return (mon, cont, task)
        }
        existingMonitor?.monitor.cancel()
        existingCont?.finish()
        await existingTask?.value
    }

    @discardableResult
    internal func processSnapshot(
        _ snapshot: FabushiNetworkSnapshot,
        expectedToken: UInt64
    ) -> Bool {
        let outcome = lock.withLock {
            state -> (
                accepted: Bool,
                continuation: AsyncStream<Bool>.Continuation?,
                transition: (@Sendable (FabushiNetworkSnapshot) -> Void)?
            ) in
            guard state.sessionToken == expectedToken else {
                return (false, nil, nil)
            }
            let snapshotChanged = state.lastSnapshot != snapshot
            let shouldNotifyHost = state.lastForwardedOnline != snapshot.isOnline
            if snapshotChanged {
                state.lastSnapshot = snapshot
            }
            if shouldNotifyHost {
                state.lastForwardedOnline = snapshot.isOnline
            }
            guard snapshotChanged || shouldNotifyHost else {
                return (true, nil, nil)
            }
            return (
                true,
                shouldNotifyHost ? state.forwardContinuation : nil,
                snapshotChanged ? state.onTransition : nil
            )
        }

        guard outcome.accepted else { return false }
        if let continuation = outcome.continuation {
            continuation.yield(snapshot.isOnline)
        }
        outcome.transition?(snapshot)
        return true
    }

    /// Applies an observed network snapshot directly to the Rust outbox state and records it.
    @discardableResult
    public func applyObservedSnapshot(
        _ snapshot: FabushiNetworkSnapshot,
        host: FabushiRustHostActor
    ) async throws -> FabushiNetworkStateResult {
        lock.withLock { state in
            state.lastSnapshot = snapshot
            state.lastForwardedOnline = snapshot.isOnline
        }
        return try await host.setNetworkOnline(snapshot.isOnline)
    }
}
