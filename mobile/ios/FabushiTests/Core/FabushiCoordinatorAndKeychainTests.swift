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
        async let start1: Void = coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        async let start2: Void = coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        async let start3: Void = coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        async let start4: Void = coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        _ = try await (start1, start2, start3, start4)
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

    func testShutdownCoalescesAndPreventsFalseStartOrOrphanedHost() async throws {
        // (a) Trace A: shutdown() racing coalesced shutdown() + concurrent start() on an already-started
        // coordinator (holding an in-flight slow loopback HTTP RPC so host.shutdown() actively drains).
        let server = try LoopbackHTTPTestServer()
        defer { server.stop() }
        server.setSlowHealthDelayMs(1_500)

        let dir1 = try makeTempDir("shutdown_coalesce_trace_a")
        let dir2 = try makeTempDir("shutdown_coalesce_trace_b")
        let dir3 = try makeTempDir("shutdown_fresh_gen_reset")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.shutdowncoalesce.\(UUID().uuidString)",
            account: "shutdowncoalesce"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir1)
            try? FileManager.default.removeItem(at: dir2)
            try? FileManager.default.removeItem(at: dir3)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(
            appDataDirectory: dir1,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )
        XCTAssertTrue(coordinator.isStarted)

        let gen101 = try await coordinator.setAuthSession(FabushiSessionAuthParams(
            userId: 101,
            username: "alice",
            sessionId: "sess-101",
            token: "tok-101"
        ))
        XCTAssertEqual(gen101, 2)
        coordinator.vaultStore.setUnlocked(true)
        XCTAssertTrue(coordinator.ingestDomainEvent(FabushiDomainEvent(
            seq: 700,
            generation: gen101,
            timestampMs: 1_700_000_000_000,
            type: "newMessage",
            rawPayloadJSON: "{}",
            kind: .newMessage(
                id: 700,
                chatId: 10,
                senderId: 101,
                text: "pre-shutdown message",
                timestamp: 1_700_000_000,
                replyTo: nil,
                clientNonce: nil,
                status: nil
            )
        )))
        XCTAssertTrue(coordinator.vaultStore.isUnlocked)
        XCTAssertEqual(coordinator.chatStore.lastMessageSeq, 700)

        let hostRef = coordinator.host
        let slowRPC = Task<FabushiJSONValue, Error> {
            try await hostRef.dispatch(method: FabushiRPCNamespace.auth.method("health"))
        }
        for _ in 0..<80 {
            if server.hitCount(for: "GET /health") > 0 { break }
            try? await Task.sleep(nanoseconds: 25_000_000)
        }
        XCTAssertGreaterThanOrEqual(server.hitCount(for: "GET /health"), 1)

        // Launch shutdown1 and confirm coordinator state + domain stores clear SYNCHRONOUSLY
        // before host.shutdown() even finishes draining.
        let shutdown1Entered = OSAllocatedUnfairLock<Bool>(initialState: false)
        let shutdown1 = Task { @MainActor in
            shutdown1Entered.withLock { $0 = true }
            await coordinator.shutdown()
        }
        for _ in 0..<2_000 {
            if shutdown1Entered.withLock({ $0 }) { break }
            await Task.yield()
        }
        XCTAssertTrue(shutdown1Entered.withLock { $0 })
        XCTAssertFalse(coordinator.isStarted, "shutdown() must clear isStarted synchronously before any await")
        XCTAssertNil(coordinator.activeAccountId)
        XCTAssertEqual(coordinator.authState, .unauthenticated)
        XCTAssertEqual(coordinator.accountScopeGeneration, 1)
        XCTAssertFalse(coordinator.vaultStore.isUnlocked)
        XCTAssertEqual(coordinator.chatStore.lastMessageSeq, 0)

        // Coalesce a second shutdown() before spawning concurrent re-start so shutdown2 cannot
        // trail behind restartTask regardless of scheduler timing.
        let shutdown2Entered = OSAllocatedUnfairLock<Bool>(initialState: false)
        let shutdown2 = Task { @MainActor in
            shutdown2Entered.withLock { $0 = true }
            await coordinator.shutdown()
        }
        for _ in 0..<2_000 {
            if shutdown2Entered.withLock({ $0 }) { break }
            await Task.yield()
        }
        XCTAssertTrue(shutdown2Entered.withLock { $0 })

        let restartTask = Task<Void, Error> { @MainActor in
            try await coordinator.start(
                appDataDirectory: dir1,
                serverURL: server.baseURLString,
                testMode: true,
                startNetworkMonitor: false
            )
        }

        await shutdown1.value
        await shutdown2.value
        _ = try? await slowRPC.value
        try await restartTask.value

        XCTAssertTrue(coordinator.isStarted, "Re-start after coalesced shutdown must converge to started")
        let hostInitAfterRestart = await coordinator.host.isInitialized
        XCTAssertTrue(hostInitAfterRestart)
        let pingAfterRestart = try await coordinator.host.ping(nonce: 801)
        XCTAssertTrue(pingAfterRestart.pong)
        XCTAssertEqual(pingAfterRestart.nonce, 801)
        XCTAssertEqual(coordinator.activeAccountId, "101")
        XCTAssertEqual(coordinator.authState, .authenticated(userId: 101, username: "alice", accountId: "101"))

        await coordinator.shutdown()
        let hostInitAfterPartAShutdown = await coordinator.host.isInitialized
        XCTAssertFalse(hostInitAfterPartAShutdown)

        // (b) Trace B: start() -> shutdown() -> concurrent re-start() verifying zero false-success start,
        // zero orphaned live host behind isStarted == false, clean domain store reset on shutdown(),
        // and clean post-re-start ping + setAuthSession.
        let coordinator2 = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)

        // Sub-case B1: start() interrupted by shutdown() leaves zero orphaned live host.
        let startInterrupt = Task<Void, Error> { @MainActor in
            try await coordinator2.start(appDataDirectory: dir2, testMode: true, startNetworkMonitor: false)
        }
        for _ in 0..<2_000 {
            if coordinator2.hasInFlightStart || coordinator2.isStarted { break }
            await Task.yield()
        }
        await coordinator2.shutdown()
        _ = try? await startInterrupt.value

        XCTAssertFalse(coordinator2.isStarted)
        let hostInitAfterB1 = await coordinator2.host.isInitialized
        XCTAssertFalse(hostInitAfterB1, "shutdown() racing start() must never leave an orphaned live host")
        XCTAssertNil(coordinator2.activeAccountId)
        XCTAssertEqual(coordinator2.authState, .unauthenticated)
        XCTAssertEqual(coordinator2.accountScopeGeneration, 1)

        // Sub-case B2: start() -> shutdown() -> TWO concurrent re-start() calls during shutdown drain.
        let startA = Task<Void, Error> { @MainActor in
            try await coordinator2.start(appDataDirectory: dir2, testMode: true, startNetworkMonitor: false)
        }
        for _ in 0..<2_000 {
            if coordinator2.hasInFlightStart || coordinator2.isStarted { break }
            await Task.yield()
        }

        let shutdownMidEntered = OSAllocatedUnfairLock<Bool>(initialState: false)
        let shutdownMid = Task { @MainActor in
            shutdownMidEntered.withLock { $0 = true }
            await coordinator2.shutdown()
        }
        for _ in 0..<2_000 {
            if shutdownMidEntered.withLock({ $0 }) { break }
            await Task.yield()
        }
        XCTAssertTrue(shutdownMidEntered.withLock { $0 })

        let restartB = Task<Void, Error> { @MainActor in
            try await coordinator2.start(appDataDirectory: dir2, testMode: true, startNetworkMonitor: false)
        }
        let restartC = Task<Void, Error> { @MainActor in
            try await coordinator2.start(appDataDirectory: dir2, testMode: true, startNetworkMonitor: false)
        }

        await shutdownMid.value
        _ = try? await startA.value
        try await restartB.value
        try await restartC.value

        XCTAssertTrue(coordinator2.isStarted, "shutdown() must never clobber concurrent re-start() into zombie isStarted == false")
        let hostInitAfterB2 = await coordinator2.host.isInitialized
        XCTAssertTrue(hostInitAfterB2)
        let pingAfterB2 = try await coordinator2.host.ping(nonce: 802)
        XCTAssertTrue(pingAfterB2.pong)
        XCTAssertEqual(pingAfterB2.nonce, 802)

        let gen303 = try await coordinator2.setAuthSession(FabushiSessionAuthParams(
            userId: 303,
            username: "charlie",
            sessionId: "sess-303",
            token: "tok-303"
        ))
        XCTAssertGreaterThanOrEqual(gen303, 2)
        XCTAssertEqual(coordinator2.accountScopeGeneration, gen303)
        XCTAssertEqual(coordinator2.activeAccountId, "303")
        XCTAssertEqual(coordinator2.authState, .authenticated(userId: 303, username: "charlie", accountId: "303"))

        // Shutdown and re-start on a brand-new empty data directory: accountScopeGeneration must
        // cleanly re-sync to 1 (never stuck on obsolete generation >= 2 from dir2).
        await coordinator2.shutdown()
        XCTAssertFalse(coordinator2.isStarted)
        XCTAssertEqual(coordinator2.accountScopeGeneration, 1)

        try await coordinator2.start(appDataDirectory: dir3, testMode: true, startNetworkMonitor: false)
        XCTAssertTrue(coordinator2.isStarted)
        XCTAssertEqual(coordinator2.accountScopeGeneration, 1)
        XCTAssertNil(coordinator2.activeAccountId)
        XCTAssertEqual(coordinator2.authState, .unauthenticated)

        let freshGen = try await coordinator2.setAuthSession(FabushiSessionAuthParams(
            userId: 304,
            username: "dana",
            sessionId: "sess-304",
            token: "tok-304"
        ))
        XCTAssertEqual(freshGen, 2)
        XCTAssertEqual(coordinator2.accountScopeGeneration, 2)
        XCTAssertEqual(coordinator2.activeAccountId, "304")
        await coordinator2.shutdown()
    }

    func testShutdownFencesInFlightSessionMutationsAgainstAuthStateResurrection() async throws {
        let dir = try makeTempDir("shutdown_fence_mutations")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.shutdownfence.\(UUID().uuidString)",
            account: "shutdownfence"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)

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

        // 1. Trace C1: switchAccount("101") interleaved with shutdown() inside beforeUsernameLookup
        // MUST throw FabushiCoordinatorError.notStarted (NOT silent success, NOT authState resurrection)
        // and leave all 7 domain stores cleanly reset.
        do {
            _ = try await coordinator.switchAccount(accountId: "101") {
                XCTAssertEqual(coordinator.activeAccountId, "101")
                let activeGen = coordinator.accountScopeGeneration
                coordinator.vaultStore.setUnlocked(true)
                XCTAssertTrue(coordinator.ingestDomainEvent(FabushiDomainEvent(
                    seq: 801,
                    generation: activeGen,
                    timestampMs: 1_700_000_000_001,
                    type: "newMessage",
                    rawPayloadJSON: "{}",
                    kind: .newMessage(
                        id: 801,
                        chatId: 10,
                        senderId: 101,
                        text: "in-flight switch msg",
                        timestamp: 1_700_000_001,
                        replyTo: nil,
                        clientNonce: nil,
                        status: nil
                    )
                )))
                XCTAssertTrue(coordinator.ingestDomainEvent(FabushiDomainEvent(
                    seq: 802,
                    generation: activeGen,
                    timestampMs: 1_700_000_000_002,
                    type: "taskStateUpdated",
                    rawPayloadJSON: "{}",
                    kind: .taskStateUpdated(taskId: "task-1", state: "running")
                )))
                XCTAssertTrue(coordinator.ingestDomainEvent(FabushiDomainEvent(
                    seq: 803,
                    generation: activeGen,
                    timestampMs: 1_700_000_000_003,
                    type: "remoteEmergencyTakeover",
                    rawPayloadJSON: "{}",
                    kind: .remoteEmergencyTakeover(disconnectedSessions: 2, overrideEpoch: 7, elapsedUs: 120)
                )))
                XCTAssertTrue(coordinator.ingestDomainEvent(FabushiDomainEvent(
                    seq: 804,
                    generation: activeGen,
                    timestampMs: 1_700_000_000_004,
                    type: "miniAppControlStatus",
                    rawPayloadJSON: "{}",
                    kind: .miniAppControlStatus(
                        botUsername: "bot",
                        shortName: "app",
                        chatId: 10,
                        botUserId: 99,
                        appTitle: "MiniApp",
                        mode: "active",
                        statusText: nil,
                        activeTool: nil
                    )
                )))
                XCTAssertTrue(coordinator.ingestDomainEvent(FabushiDomainEvent(
                    seq: 805,
                    generation: activeGen,
                    timestampMs: 1_700_000_000_005,
                    type: "callUpdated",
                    rawPayloadJSON: "{}",
                    kind: .callUpdated(callId: "call-1", state: "connected", payload: .null)
                )))
                XCTAssertTrue(coordinator.ingestDomainEvent(FabushiDomainEvent(
                    seq: 806,
                    generation: activeGen,
                    timestampMs: 1_700_000_000_006,
                    type: "connectionStateChanged",
                    rawPayloadJSON: "{}",
                    kind: .connectionStateChanged(status: "Connected", serverUrl: nil)
                )))
                await coordinator.shutdown()
            }
            XCTFail("Expected FabushiCoordinatorError.notStarted when shutdown() ran during switchAccount()")
        } catch let err as FabushiCoordinatorError {
            XCTAssertEqual(err, .notStarted)
        }

        XCTAssertFalse(coordinator.isStarted)
        XCTAssertNil(coordinator.activeAccountId)
        XCTAssertEqual(coordinator.authState, .unauthenticated)
        XCTAssertEqual(coordinator.accountScopeGeneration, 1)
        XCTAssertEqual(coordinator.chatStore.lastMessageSeq, 0)
        XCTAssertTrue(coordinator.agentStudioStore.taskStatesById.isEmpty)
        XCTAssertNil(coordinator.remoteComputerStore.lastEmergencyOverrideEpoch)
        XCTAssertNil(coordinator.miniAppHostStore.activeControlMode)
        XCTAssertNil(coordinator.callSessionStore.activeCallId)
        XCTAssertEqual(coordinator.settingsStore.wsConnectionStatus, "Disconnected")
        XCTAssertFalse(coordinator.vaultStore.isUnlocked)

        // Events ingested while shut down MUST be rejected fail-closed.
        XCTAssertFalse(coordinator.ingestDomainEvent(FabushiDomainEvent(
            seq: 807,
            generation: 1,
            timestampMs: 1_700_000_000_007,
            type: "newMessage",
            rawPayloadJSON: "{}",
            kind: .newMessage(
                id: 807,
                chatId: 10,
                senderId: 101,
                text: "post-shutdown event",
                timestamp: 1_700_000_007,
                replyTo: nil,
                clientNonce: nil,
                status: nil
            )
        )))
        XCTAssertEqual(coordinator.chatStore.lastMessageSeq, 0)

        // 2. executeFenced interleaved with shutdown() AND with shutdown() + immediate re-start()
        // MUST both throw FabushiCoordinatorError.notStarted via coordinatorLifecycleEpoch fencing.
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        do {
            _ = try await coordinator.executeFenced { _ in
                await coordinator.shutdown()
                return "stale_after_shutdown"
            }
            XCTFail("Expected FabushiCoordinatorError.notStarted when shutdown() ran during executeFenced()")
        } catch let err as FabushiCoordinatorError {
            XCTAssertEqual(err, .notStarted)
        }
        XCTAssertFalse(coordinator.isStarted)
        XCTAssertNil(coordinator.activeAccountId)
        XCTAssertEqual(coordinator.authState, .unauthenticated)

        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        do {
            _ = try await coordinator.executeFenced { _ in
                await coordinator.shutdown()
                try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
                return "stale_across_restart_epoch"
            }
            XCTFail("Expected FabushiCoordinatorError.notStarted across shutdown()+start() lifecycle epoch change")
        } catch let err as FabushiCoordinatorError {
            XCTAssertEqual(err, .notStarted)
        }
        XCTAssertTrue(coordinator.isStarted)
        await coordinator.shutdown()

        // 3. dispatchFenced interleaved with shutdown() on a real in-flight loopback RPC
        // MUST fail closed and never resurrect state.
        let server = try LoopbackHTTPTestServer()
        defer { server.stop() }
        server.setSlowHealthDelayMs(2_000)

        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )
        let fencedRPC = Task<FabushiJSONValue, Error> { @MainActor in
            try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("health"),
                params: FabushiEmptyParams()
            )
        }
        for _ in 0..<80 {
            if server.hitCount(for: "GET /health") > 0 { break }
            try? await Task.sleep(nanoseconds: 25_000_000)
        }
        XCTAssertGreaterThanOrEqual(server.hitCount(for: "GET /health"), 1)

        await coordinator.shutdown()
        do {
            _ = try await fencedRPC.value
            XCTFail("Expected dispatchFenced to fail closed when shutdown() ran mid-flight")
        } catch let coordError as FabushiCoordinatorError {
            XCTAssertEqual(coordError, .notStarted)
        } catch let hostError as FabushiHostError {
            XCTAssertTrue(
                hostError.code == "host_closed" || hostError.isCancelled,
                "Expected host_closed or cancelled host error, got \(hostError)"
            )
        }
        XCTAssertFalse(coordinator.isStarted)
        XCTAssertNil(coordinator.activeAccountId)
        XCTAssertEqual(coordinator.authState, .unauthenticated)

        // 4. setAuthSession interleaved with shutdown() MUST never resurrect activeAccountId/authState,
        // and calling setAuthSession while shut down MUST throw FabushiCoordinatorError.notStarted.
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        let setAuthEntered = OSAllocatedUnfairLock<Bool>(initialState: false)
        let setAuthTask = Task<UInt64, Error> { @MainActor in
            setAuthEntered.withLock { $0 = true }
            return try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 404,
                username: "dave",
                sessionId: "sess-404",
                token: "tok-404"
            ))
        }
        for _ in 0..<2_000 {
            if setAuthEntered.withLock({ $0 }) { break }
            await Task.yield()
        }
        XCTAssertTrue(setAuthEntered.withLock { $0 })

        await coordinator.shutdown()
        do {
            _ = try await setAuthTask.value
            // Acceptable only if setAuthSession completed entirely before shutdown() began;
            // shutdown() MUST still leave coordinator completely cleared with zero resurrection.
        } catch let coordError as FabushiCoordinatorError {
            XCTAssertEqual(coordError, .notStarted)
        } catch let hostError as FabushiHostError {
            XCTAssertTrue(
                hostError.code == "host_closed" || hostError.isCancelled,
                "Expected host_closed or cancelled host error, got \(hostError)"
            )
        }
        XCTAssertFalse(coordinator.isStarted)
        XCTAssertNil(coordinator.activeAccountId)
        XCTAssertEqual(coordinator.authState, .unauthenticated)
        XCTAssertFalse(coordinator.vaultStore.isUnlocked)

        do {
            _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 405,
                username: "eve",
                sessionId: "sess-405",
                token: "tok-405"
            ))
            XCTFail("Expected FabushiCoordinatorError.notStarted after shutdown()")
        } catch let err as FabushiCoordinatorError {
            XCTAssertEqual(err, .notStarted)
        }
        XCTAssertFalse(coordinator.isStarted)
        XCTAssertNil(coordinator.activeAccountId)
        XCTAssertEqual(coordinator.authState, .unauthenticated)
    }
}
