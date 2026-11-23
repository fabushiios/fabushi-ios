import Foundation
import os
#if canImport(UIKit)
import UIKit
#endif
import XCTest
@testable import Fabushi

@MainActor
final class FabushiCoordinatorAndKeychainTests: XCTestCase, @unchecked Sendable {

    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_coord_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    func testKeychainStoreGenerates32ByteRandomPassphrasePersistsAndDeletesCleanly() throws {
        let store = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.keychain.\(UUID().uuidString)",
            account: "unit-test"
        )
        defer { try? store.deletePassphrase() }

        XCTAssertNil(try store.readPassphrase(), "Fresh Keychain service/account must start empty")

        let created = try store.loadOrCreateStoragePassphrase()
        let decoded = try XCTUnwrap(Data(base64Encoded: created))
        XCTAssertEqual(decoded.count, 32, "Keychain passphrase must be 32 cryptographically random bytes")
        XCTAssertNotEqual(decoded, Data(repeating: 0x00, count: 32))
        XCTAssertNotEqual(decoded, Data(repeating: 0xAA, count: 32))
        XCTAssertNotEqual(decoded, Data(repeating: 0xBB, count: 32))

        // Second call MUST return exact persisted value
        let reread = try store.loadOrCreateStoragePassphrase()
        XCTAssertEqual(reread, created)
        XCTAssertEqual(try store.readPassphrase(), created)

        // Delete removes item cleanly
        try store.deletePassphrase()
        XCTAssertNil(try store.readPassphrase())
    }

    func testCoordinatorGenerationFencingZeroUserGuardMultiAccountAndColdRelaunch() async throws {
        let dir = try makeTempDir("fencing_relaunch")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.coord.\(UUID().uuidString)",
            account: "coord"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        XCTAssertTrue(coordinator.isStarted)
        XCTAssertEqual(coordinator.accountScopeGeneration, 1)
        XCTAssertEqual(coordinator.authState, .unauthenticated)

        // user_id == 0 MUST fail closed with invalid_arguments
        do {
            _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 0,
                username: "zero_invalid",
                sessionId: "sess-zero",
                token: "tok-zero"
            ))
            XCTFail("Expected invalid_arguments for user_id == 0")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err.code, "invalid_arguments")
        }
        XCTAssertEqual(coordinator.accountScopeGeneration, 1)

        // Account 101 session -> generation advances to 2
        let gen101 = try await coordinator.setAuthSession(FabushiSessionAuthParams(
            userId: 101,
            username: "alice",
            sessionId: "sess-101",
            token: "tok-101-secret",
            firstName: "Alice"
        ))
        XCTAssertEqual(gen101, 2)
        XCTAssertEqual(coordinator.accountScopeGeneration, 2)
        XCTAssertEqual(coordinator.activeAccountId, "101")
        XCTAssertEqual(coordinator.authState, .authenticated(userId: 101, username: "alice", accountId: "101"))

        // Write local indexed message under account 101
        let search101 = SearchIndexingBridge(host: coordinator.host)
        _ = try await search101.indexMessage(doc: SearchDocument(
            id: 5001,
            chatId: 10,
            text: "Account 101 confidential secret note"
        ))
        let preSwitchHits = try await search101.search(query: SearchQueryParams(query: "confidential"))
        XCTAssertEqual(preSwitchHits.count, 1)

        // Account 202 session -> generation advances to 3 AND wipes account 101 local caches
        let gen202 = try await coordinator.setAuthSession(FabushiSessionAuthParams(
            userId: 202,
            username: "bob",
            sessionId: "sess-202",
            token: "tok-202-secret",
            firstName: "Bob"
        ))
        XCTAssertEqual(gen202, 3)
        XCTAssertEqual(coordinator.accountScopeGeneration, 3)
        XCTAssertEqual(coordinator.activeAccountId, "202")

        let postSwitchHits = try await search101.search(query: SearchQueryParams(query: "confidential"))
        XCTAssertTrue(postSwitchHits.isEmpty, "Cross-account local search data must be wiped on account switch")

        // Stale Rust RPC stamped with generation 1 MUST fail closed with stale_generation
        do {
            let _: FabushiJSONValue = try await coordinator.host.dispatch(
                method: FabushiRPCNamespace.session.method("listAccounts"),
                generation: 1
            )
            XCTFail("Expected stale_generation for generation: 1 when active is 3")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err.code, "stale_generation")
        }

        // Coordinator executeFenced MUST drop stale completion if generation advances mid-flight
        do {
            _ = try await coordinator.executeFenced { capturedGen in
                XCTAssertEqual(capturedGen, 3)
                _ = try await coordinator.switchAccount(accountId: "101")
                return "stale_payload"
            }
            XCTFail("Expected FabushiCoordinatorError.staleGeneration")
        } catch let err as FabushiCoordinatorError {
            XCTAssertEqual(err, .staleGeneration(captured: 3, current: 4))
        }
        XCTAssertEqual(coordinator.accountScopeGeneration, 4)
        XCTAssertEqual(coordinator.activeAccountId, "101")

        // Save a device-level local setting & checkpoint WAL before shutdown
        struct LocalSetParams: Encodable, Sendable {
            let key: String
            let value: String
        }
        let _: FabushiJSONValue = try await coordinator.host.dispatch(
            method: FabushiRPCNamespace.settings.method("localSet"),
            params: LocalSetParams(key: "theme_pref", value: "tinted"),
            generation: coordinator.accountScopeGeneration
        )
        let wal = try await coordinator.performBackgroundWALCheckpoint()
        XCTAssertTrue(wal.checkpointed)
        XCTAssertEqual(wal.busy, 0)
        XCTAssertEqual(coordinator.lastWalCheckpoint, wal)

        await coordinator.shutdown()

        // Cold relaunch on SAME appDataDirectory + SAME Keychain passphrase restores generation & decrypts AEAD accounts!
        let relaunched = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await relaunched.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        XCTAssertEqual(relaunched.accountScopeGeneration, 4)
        XCTAssertEqual(relaunched.activeAccountId, "101")

        // Switch back to AEAD-encrypted account 202 after cold relaunch -> proves Keychain passphrase decrypted stored token!
        let genAfterRelaunchSwitch = try await relaunched.switchAccount(accountId: "202")
        XCTAssertEqual(genAfterRelaunchSwitch, 5)
        XCTAssertEqual(relaunched.activeAccountId, "202")

        // Verify persisted setting survived WAL checkpoint + cold relaunch
        struct LocalGetParams: Encodable, Sendable { let key: String }
        struct LocalGetResult: Decodable, Sendable { let key: String; let value: String? }
        let savedSetting: LocalGetResult = try await relaunched.host.dispatch(
            method: FabushiRPCNamespace.settings.method("localGet"),
            params: LocalGetParams(key: "theme_pref"),
            generation: relaunched.accountScopeGeneration
        )
        XCTAssertEqual(savedSetting.value, "tinted")

        await relaunched.shutdown()
    }

    func testNetworkMonitorBridgesReachabilityToRustOutboxState() async throws {
        let dir = try makeTempDir("network_outbox")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        let monitor = FabushiNetworkMonitor()

        let offlineResult = try await monitor.applyObservedSnapshot(
            FabushiNetworkSnapshot(isOnline: false, interface: .none),
            host: host
        )
        XCTAssertFalse(offlineResult.online)
        let offlinePending = try await host.outboxPendingCount()
        XCTAssertFalse(offlinePending.networkOnline)

        let onlineResult = try await monitor.applyObservedSnapshot(
            FabushiNetworkSnapshot(isOnline: true, interface: .wifi),
            host: host
        )
        XCTAssertTrue(onlineResult.online)
        let onlinePending = try await host.outboxPendingCount()
        XCTAssertTrue(onlinePending.networkOnline)

        monitor.stop()
        await host.shutdown()
    }

    func testSwitchAccountFencesStaleAuthStateAndZeroGenerationEventsAreDropped() async throws {
        let dir = try makeTempDir("switch_fence")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.switchfence.\(UUID().uuidString)",
            account: "switchfence"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)

        // (c-1) Concurrent start() calls must coalesce cleanly without double-init LockBusy
        try await withThrowingTaskGroup(of: Void.self) { group in
            for _ in 0..<4 {
                group.addTask { @MainActor in
                    try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
                }
            }
            for try await _ in group {}
        }
        XCTAssertTrue(coordinator.isStarted)
        XCTAssertNil(coordinator.lastErrorMessage)

        // (c-2) Non-Keychain start() failure must populate lastErrorMessage and keep isStarted == false
        let blockingFile = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_blocking_file_\(UUID().uuidString)")
        try Data("not-a-directory".utf8).write(to: blockingFile)
        defer { try? FileManager.default.removeItem(at: blockingFile) }

        let failingCoordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        do {
            try await failingCoordinator.start(appDataDirectory: blockingFile, testMode: true, startNetworkMonitor: false)
            XCTFail("Expected start() to fail when appDataDirectory points to a regular file")
        } catch {
            // Expected filesystem failure
        }
        XCTAssertFalse(failingCoordinator.isStarted)
        XCTAssertNotNil(failingCoordinator.lastErrorMessage)

        // Seed accounts 101 (alice) and 202 (bob)
        _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
            userId: 101,
            username: "alice",
            sessionId: "sess-101",
            token: "tok-101"
        ))
        _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
            userId: 202,
            username: "bob",
            sessionId: "sess-202",
            token: "tok-202"
        ))
        XCTAssertEqual(coordinator.activeAccountId, "202")
        XCTAssertEqual(coordinator.authState, .authenticated(userId: 202, username: "bob", accountId: "202"))

        // (a-1) Deterministic B-02/B1-SEC proof: interleaving clearLocalSession() during switchAccount's
        // post-switch / pre-listAccounts suspension MUST throw staleGeneration and NEVER overwrite
        // authState back to account 101!
        do {
            _ = try await coordinator.switchAccount(accountId: "101") {
                // Immediately before suspension, authState must already reflect account 101 (zero transient split-brain)
                XCTAssertEqual(coordinator.activeAccountId, "101")
                XCTAssertEqual(
                    coordinator.authState,
                    .authenticated(userId: 101, username: "101", accountId: "101")
                )
                _ = try? await coordinator.clearLocalSession()
            }
            XCTFail("Expected staleGeneration when clearLocalSession() superseded switchAccount()")
        } catch let err as FabushiCoordinatorError {
            if case .staleGeneration = err {
                // Expected fenced rejection
            } else {
                XCTFail("Expected .staleGeneration, got \(err)")
            }
        }
        XCTAssertNil(coordinator.activeAccountId)
        XCTAssertEqual(coordinator.authState, .unauthenticated)

        // (a-2) Deterministic B-02/B1-SEC proof: interleaving switchAccount("202") during switchAccount("101")
        // MUST preserve account 202's authState cleanly.
        do {
            _ = try await coordinator.switchAccount(accountId: "101") {
                _ = try? await coordinator.switchAccount(accountId: "202")
            }
            XCTFail("Expected staleGeneration when switchAccount(202) superseded switchAccount(101)")
        } catch let err as FabushiCoordinatorError {
            if case .staleGeneration = err {
                // Expected fenced rejection
            } else {
                XCTFail("Expected .staleGeneration, got \(err)")
            }
        }
        XCTAssertEqual(coordinator.activeAccountId, "202")
        XCTAssertEqual(coordinator.authState, .authenticated(userId: 202, username: "bob", accountId: "202"))

        // (a-3) logout() rejected with stale_generation must reconcile authoritative Rust state
        // and MUST NOT blindly wipe the still-active account 202 session.
        let bumpedGen = try await coordinator.host.bumpGeneration()
        XCTAssertGreaterThan(bumpedGen, coordinator.accountScopeGeneration)
        do {
            _ = try await coordinator.logout()
            XCTFail("Expected stale_generation on outdated logout()")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err.code, "stale_generation")
        }
        XCTAssertEqual(coordinator.accountScopeGeneration, bumpedGen)
        XCTAssertEqual(coordinator.activeAccountId, "202")
        XCTAssertEqual(coordinator.authState, .authenticated(userId: 202, username: "bob", accountId: "202"))

        // (b) Synthetic generation-0 and stale-generation domain events MUST be dropped when accountScopeGeneration >= 1
        let activeGen = coordinator.accountScopeGeneration
        XCTAssertGreaterThanOrEqual(activeGen, 2)
        XCTAssertEqual(coordinator.chatStore.lastMessageSeq, 0)

        let genZeroEvent = FabushiDomainEvent(
            seq: 900,
            generation: 0,
            timestampMs: 1_700_000_000_000,
            type: "newMessage",
            rawPayloadJSON: "{}",
            kind: .newMessage(
                id: 1,
                chatId: 10,
                senderId: 202,
                text: "synthetic gen 0",
                timestamp: 1_700_000_000,
                replyTo: nil,
                clientNonce: nil,
                status: nil
            )
        )
        XCTAssertFalse(coordinator.ingestDomainEvent(genZeroEvent), "Generation 0 event must be dropped")
        XCTAssertEqual(coordinator.chatStore.lastMessageSeq, 0)

        let staleEvent = FabushiDomainEvent(
            seq: 901,
            generation: activeGen - 1,
            timestampMs: 1_700_000_000_001,
            type: "newMessage",
            rawPayloadJSON: "{}",
            kind: .newMessage(
                id: 2,
                chatId: 10,
                senderId: 202,
                text: "stale prior generation",
                timestamp: 1_700_000_001,
                replyTo: nil,
                clientNonce: nil,
                status: nil
            )
        )
        XCTAssertFalse(coordinator.ingestDomainEvent(staleEvent), "Stale-generation event must be dropped")
        XCTAssertEqual(coordinator.chatStore.lastMessageSeq, 0)

        let liveEvent = FabushiDomainEvent(
            seq: 902,
            generation: activeGen,
            timestampMs: 1_700_000_000_002,
            type: "newMessage",
            rawPayloadJSON: "{}",
            kind: .newMessage(
                id: 3,
                chatId: 10,
                senderId: 202,
                text: "current generation",
                timestamp: 1_700_000_002,
                replyTo: nil,
                clientNonce: nil,
                status: nil
            )
        )
        XCTAssertTrue(coordinator.ingestDomainEvent(liveEvent), "Current-generation event must be accepted")
        XCTAssertEqual(coordinator.chatStore.lastMessageSeq, 902)

        await coordinator.shutdown()
    }

    func testBackgroundTaskGuardSynchronousExpiryAndStoreOrdering() async throws {
        #if canImport(UIKit)
        // 1. Store-then-expire ordering: synchronous single invocation of endBackgroundTask, never double-ended.
        let ended1 = OSAllocatedUnfairLock<[UIBackgroundTaskIdentifier]>(initialState: [])
        let guard1 = BackgroundTaskGuard { id in
            ended1.withLock { $0.append(id) }
        }
        let taskId1 = UIBackgroundTaskIdentifier(rawValue: 41)
        guard1.store(taskId1)
        XCTAssertEqual(ended1.withLock { $0 }, [], "store() must not end an active background task")

        guard1.endIfNeeded()
        XCTAssertEqual(ended1.withLock { $0 }, [taskId1], "First endIfNeeded() must synchronously end taskId1")

        guard1.endIfNeeded()
        XCTAssertEqual(ended1.withLock { $0 }, [taskId1], "Second endIfNeeded() must not double-end taskId1")

        // 2. Expire-before-store ordering: expiration handler fires before store(taskId) -> store() synchronously ends once.
        let ended2 = OSAllocatedUnfairLock<[UIBackgroundTaskIdentifier]>(initialState: [])
        let guard2 = BackgroundTaskGuard { id in
            ended2.withLock { $0.append(id) }
        }
        guard2.endIfNeeded()
        XCTAssertEqual(ended2.withLock { $0 }, [], "Expiring before store() must not end .invalid")

        let taskId2 = UIBackgroundTaskIdentifier(rawValue: 42)
        guard2.store(taskId2)
        XCTAssertEqual(ended2.withLock { $0 }, [taskId2], "store() after expiry must synchronously end taskId2 once")

        guard2.endIfNeeded()
        XCTAssertEqual(ended2.withLock { $0 }, [taskId2], "Subsequent endIfNeeded() must not double-end taskId2")

        // 3. Invalid identifier must never invoke endBackgroundTask.
        let ended3 = OSAllocatedUnfairLock<[UIBackgroundTaskIdentifier]>(initialState: [])
        let guard3 = BackgroundTaskGuard { id in
            ended3.withLock { $0.append(id) }
        }
        guard3.store(.invalid)
        guard3.endIfNeeded()
        XCTAssertEqual(ended3.withLock { $0 }, [])
        #endif

        // 4. Device-wide WAL checkpoint uses generation: nil so truncation succeeds even if
        // Rust host generation advanced independently during an account transition.
        let dir = try makeTempDir("wal_gen_nil")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.wal.\(UUID().uuidString)",
            account: "wal"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        let bumped = try await coordinator.host.bumpGeneration()
        XCTAssertGreaterThan(bumped, coordinator.accountScopeGeneration)

        let walSummary = try await coordinator.performBackgroundWALCheckpoint()
        XCTAssertTrue(walSummary.checkpointed)
        XCTAssertEqual(walSummary.busy, 0)
        XCTAssertEqual(coordinator.lastWalCheckpoint, walSummary)

        await coordinator.shutdown()
    }

    func testNetworkMonitorSerializedBurstConvergesInOrderAndSessionFencesStop() async throws {
        let dir = try makeTempDir("network_burst")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        let monitor = FabushiNetworkMonitor()

        // Burst 1: rapid [false, true, false, true, false, true] must converge deterministically to true
        let token1 = monitor.startSession(host: host, startPathMonitor: false)
        for online in [false, true, false, true, false, true] {
            let accepted = monitor.processSnapshot(
                FabushiNetworkSnapshot(isOnline: online, interface: online ? .wifi : .none),
                expectedToken: token1
            )
            XCTAssertTrue(accepted)
        }
        await monitor.drainAndStopForTesting()
        let pendingAfterTrue = try await host.outboxPendingCount()
        XCTAssertTrue(pendingAfterTrue.networkOnline, "Serialized burst ending in true must leave Rust outbox online")

        // Stale token1 callback arriving after stop MUST be dropped without mutating Rust state
        XCTAssertFalse(monitor.processSnapshot(
            FabushiNetworkSnapshot(isOnline: false, interface: .none),
            expectedToken: token1
        ))
        let stillOnline = try await host.outboxPendingCount()
        XCTAssertTrue(stillOnline.networkOnline)

        // Burst 2: rapid [true, false, true, false, true, false] must converge deterministically to false
        let token2 = monitor.startSession(host: host, startPathMonitor: false)
        XCTAssertNotEqual(token2, token1)
        // Stale token1 MUST be dropped against active session token2
        XCTAssertFalse(monitor.processSnapshot(
            FabushiNetworkSnapshot(isOnline: true, interface: .wifi),
            expectedToken: token1
        ))

        for online in [true, false, true, false, true, false] {
            let accepted = monitor.processSnapshot(
                FabushiNetworkSnapshot(isOnline: online, interface: online ? .cellular : .none),
                expectedToken: token2
            )
            XCTAssertTrue(accepted)
        }
        await monitor.drainAndStopForTesting()
        let pendingAfterFalse = try await host.outboxPendingCount()
        XCTAssertFalse(pendingAfterFalse.networkOnline, "Serialized burst ending in false must leave Rust outbox offline")

        // Stale token2 callback arriving after stop MUST be dropped without mutating Rust state
        XCTAssertFalse(monitor.processSnapshot(
            FabushiNetworkSnapshot(isOnline: true, interface: .wifi),
            expectedToken: token2
        ))
        let stillOffline = try await host.outboxPendingCount()
        XCTAssertFalse(stillOffline.networkOnline)

        // Dedup reset across stop() -> start(): fresh host (defaults to online=true) MUST receive offline update
        // even though the previous monitor session ended in offline=false.
        let dir2 = try makeTempDir("network_restart")
        defer { try? FileManager.default.removeItem(at: dir2) }
        let host2 = FabushiRustHostActor()
        try await host2.initialize(config: FabushiHostConfig(
            appDataDir: dir2.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))
        let token3 = monitor.startSession(host: host2, startPathMonitor: false)
        XCTAssertTrue(monitor.processSnapshot(
            FabushiNetworkSnapshot(isOnline: false, interface: .none),
            expectedToken: token3
        ))
        await monitor.drainAndStopForTesting()
        let host2Pending = try await host2.outboxPendingCount()
        XCTAssertFalse(host2Pending.networkOnline)

        await host2.shutdown()
        await host.shutdown()
    }
}
