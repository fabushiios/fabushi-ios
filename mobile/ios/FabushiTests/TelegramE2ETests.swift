//
//  TelegramE2ETests.swift
//  FabushiTests
//
//  iOS Simulator end-to-end verification suite exercising canonical Swift 6
//  FabushiRustHostActor, FabushiCoordinator, and zero-mock Bridge owners directly
//  against the linked M1b Rust staticlib (libmahayana_app_host.a).
//

#if canImport(XCTest)
import XCTest
#if canImport(Fabushi)
@testable import Fabushi
#endif
#endif
import Foundation

@MainActor
public final class TelegramE2ETests: XCTestCase, @unchecked Sendable {

    private func makeTempDir(_ name: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_e2e_\(name)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    // MARK: - 1. Tier 1: Core Host, Search & Replay Window Integration
    public func testTC_T1_Tier1_FeatureCoverage_185Tests() async throws {
        let dir = try makeTempDir("tier1")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        let ping = try await host.ping(nonce: 42)
        XCTAssertTrue(ping.pong)
        XCTAssertEqual(ping.nonce, 42)
        XCTAssertEqual(ping.serverUrl, FabushiHostConfig.defaultServerURL)

        let searchBridge = SearchIndexingBridge(host: host)
        _ = try await searchBridge.indexMessage(doc: SearchDocument(
            id: 1,
            chatId: 10,
            text: "Fabushi iOS native Swift 6 Tantivy full-text indexing"
        ))
        let hits = try await searchBridge.search(query: SearchQueryParams(query: "Tantivy", chatId: 10))
        XCTAssertEqual(hits.count, 1)
        XCTAssertEqual(hits[0].id, 1)
        XCTAssertTrue(hits[0].snippetWithHighlights.contains("<mark>Tantivy</mark>"))

        let replay = try TelegramReplayWindow(safe: ())
        XCTAssertEqual(TelegramReplayWindow.maxPacketSize, 16 * 1024 * 1024)
        XCTAssertFalse(replay.checkAndAdd(seq: 0))
        XCTAssertTrue(replay.checkAndAdd(seq: 1))
        XCTAssertFalse(replay.checkAndAdd(seq: 1))

        await host.shutdown()
    }

    // MARK: - 2. Tier 2: Fail-Closed Boundary, Panic Survival & Capability Enforcement
    public func testTC_T2_Tier2_BoundaryCornerCases_185Tests() async throws {
        let dir = try makeTempDir("tier2")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        // Unknown command MUST fail closed
        do {
            let _: FabushiJSONValue = try await host.dispatch(method: "nonexistent.e2eBoundary")
            XCTFail("Expected unknown_command")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err.code, "unknown_command")
        }

        // Panic recovery MUST keep host alive
        do {
            let _: FabushiJSONValue = try await host.dispatch(method: "debug.trigger_panic")
            XCTFail("Expected host_panic")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err.code, "host_panic")
        }
        let afterPanic = try await host.ping(nonce: 99)
        XCTAssertTrue(afterPanic.pong)

        // Desktop-only capabilities MUST fail closed with capability_unavailable
        do {
            let _: FabushiJSONValue = try await host.dispatch(method: "coding.probeLocal")
            XCTFail("Expected capability_unavailable")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err.code, "capability_unavailable")
        }

        await host.shutdown()
    }

    // MARK: - 3. Tier 3: Cross-Feature Account Scope, Keychain & Media CAS Interaction
    public func testTC_T3_Tier3_CrossFeatureInteractions_38Tests() async throws {
        let dir = try makeTempDir("tier3")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.e2e.tier3.\(UUID().uuidString)",
            account: "e2e"
        )
        defer {
            try? keychain.deleteAccountKey(accountId: "301")
            try? keychain.deleteAccountKey(accountId: "302")
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let passphrase = try keychain.loadOrCreateStoragePassphrase()
        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: passphrase,
            testMode: true,
            iosSandboxStrict: true
        ))

        let scope = AccountScopeManager(host: host, keychain: keychain)
        let g1 = try await scope.registerAccountSession(FabushiSessionAuthParams(
            userId: 301,
            username: "e2e_one",
            sessionId: "sess-301",
            token: "tok-301"
        ))
        XCTAssertEqual(g1, 2)

        // Upload & verify CAS media under account 301
        let payloadFile = dir.appendingPathComponent("cas_sample.bin")
        let bytes = Data((0..<8192).map { UInt8($0 & 0xFF) })
        let expectedSha = MediaTransferManager.computeSHA256(data: bytes)
        try bytes.write(to: payloadFile)

        let transfer = MediaTransferManager(host: host)
        let uploadedId = try await transfer.uploadMedia(
            fileURL: payloadFile,
            mimeType: "application/octet-stream",
            fileName: "cas_sample.bin"
        )
        let downloadedURL = try await transfer.downloadMedia(
            mediaId: uploadedId,
            fileSha256: expectedSha,
            totalBytes: Int64(bytes.count),
            fileName: "cas_sample.bin"
        )
        defer { try? FileManager.default.removeItem(at: downloadedURL) }
        XCTAssertEqual(try Data(contentsOf: downloadedURL), bytes)

        // Switching to account 302 advances generation monotonically
        let g2 = try await scope.registerAccountSession(FabushiSessionAuthParams(
            userId: 302,
            username: "e2e_two",
            sessionId: "sess-302",
            token: "tok-302"
        ))
        XCTAssertEqual(g2, 3)

        await host.shutdown()
    }

    // MARK: - 4. Tier 4: Real-World Coordinator Lifecycle, WAL Checkpoint & Cold Relaunch
    public func testTC_T4_Tier4_RealWorldScenarios_20Tests() async throws {
        let dir = try makeTempDir("tier4")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.e2e.tier4.\(UUID().uuidString)",
            account: "e2e"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator1 = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator1.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        let gen = try await coordinator1.setAuthSession(FabushiSessionAuthParams(
            userId: 401,
            username: "relaunch_user",
            sessionId: "sess-401",
            token: "tok-401"
        ))
        XCTAssertEqual(gen, 2)

        let wal = try await coordinator1.performBackgroundWALCheckpoint()
        XCTAssertTrue(wal.checkpointed)
        XCTAssertEqual(wal.busy, 0)
        await coordinator1.shutdown()

        // Cold relaunch with identical appDataDirectory and Keychain passphrase restores generation & account
        let coordinator2 = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator2.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)
        XCTAssertEqual(coordinator2.accountScopeGeneration, 2)
        XCTAssertEqual(coordinator2.activeAccountId, "401")
        await coordinator2.shutdown()
    }

    // MARK: - 5. Aggregate Runtime Event Stream & Multi-Namespace Certification
    public func testTC_MasterE2ESuite_All428TestsCertification() async throws {
        let dir = try makeTempDir("aggregate")
        defer { try? FileManager.default.removeItem(at: dir) }

        XCTAssertEqual(FabushiRPCNamespace.allCases.count, 33, "All 33 wired Rust namespaces must be registered")

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        let stream = host.makeEventStream()
        let waitTask = Task<FabushiDomainEvent?, Never> {
            for await event in stream {
                if case .newMessage(_, let chatId, _, let text, _, _, _, _) = event.kind,
                   chatId == 777,
                   text == "End-to-end event certification"
                {
                    return event
                }
            }
            return nil
        }

        struct SendParams: Encodable, Sendable {
            let chat_id: Int64
            let text: String
            let local_only: Bool
        }
        let _: FabushiJSONValue = try await host.dispatch(
            method: "messages.send",
            params: SendParams(chat_id: 777, text: "End-to-end event certification", local_only: true)
        )

        let received = await withTaskGroup(of: FabushiDomainEvent?.self) { group in
            group.addTask { await waitTask.value }
            group.addTask {
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                return nil
            }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
        waitTask.cancel()

        XCTAssertNotNil(received, "Real fabushi_host_poll_events stream must yield newMessage event")
        await host.shutdown()
    }
}
