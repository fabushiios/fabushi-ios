import Foundation
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
}
