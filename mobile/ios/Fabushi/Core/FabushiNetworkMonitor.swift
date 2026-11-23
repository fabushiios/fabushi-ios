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
    var monitor: NWPathMonitor?
    var lastSnapshot: FabushiNetworkSnapshot?
    var lastForwardedOnline: Bool?
}

/// Thread-safe Apple `Network.NWPathMonitor` bridge running on a dedicated serial utility queue.
/// Deduplicates reachability transitions and forwards connectivity state directly to
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
        let newTransfer = MonitorTransfer(monitor: NWPathMonitor())
        let previous = lock.withLock { state -> MonitorTransfer? in
            let old = state.monitor.map(MonitorTransfer.init(monitor:))
            state.monitor = newTransfer.monitor
            return old
        }
        previous?.monitor.cancel()

        newTransfer.monitor.pathUpdateHandler = { [weak self] path in
            guard let self else { return }
            let snapshot = FabushiNetworkSnapshot(path: path)
            let (snapshotChanged, shouldNotifyHost) = self.recordTransition(snapshot)
            guard snapshotChanged else { return }

            if shouldNotifyHost {
                Task {
                    _ = try? await host.setNetworkOnline(snapshot.isOnline)
                }
            }
            onTransition?(snapshot)
        }
        newTransfer.monitor.start(queue: queue)
    }

    public func stop() {
        let existing = lock.withLock { state -> MonitorTransfer? in
            let mon = state.monitor.map(MonitorTransfer.init(monitor:))
            state.monitor = nil
            return mon
        }
        existing?.monitor.cancel()
    }

    /// Applies an observed network snapshot directly to the Rust outbox state and records it.
    @discardableResult
    public func applyObservedSnapshot(
        _ snapshot: FabushiNetworkSnapshot,
        host: FabushiRustHostActor
    ) async throws -> FabushiNetworkStateResult {
        _ = recordTransition(snapshot)
        return try await host.setNetworkOnline(snapshot.isOnline)
    }

    private func recordTransition(
        _ snapshot: FabushiNetworkSnapshot
    ) -> (snapshotChanged: Bool, shouldNotifyHost: Bool) {
        lock.withLock { state in
            let snapshotChanged = state.lastSnapshot != snapshot
            let shouldNotifyHost = state.lastForwardedOnline != snapshot.isOnline
            if snapshotChanged {
                state.lastSnapshot = snapshot
            }
            if shouldNotifyHost {
                state.lastForwardedOnline = snapshot.isOnline
            }
            return (snapshotChanged, shouldNotifyHost)
        }
    }
}
