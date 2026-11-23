import Foundation
import XCTest
@testable import Fabushi

final class FabushiBridgeZeroMockTests: XCTestCase, @unchecked Sendable {

    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_bridge_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    func testSearchIndexingBridgeTantivyFTS5CJKBatchAndRebuild() async throws {
        let dir = try makeTempDir("search")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        let bridge = SearchIndexingBridge(host: host)
        let healthy = try await bridge.verifySchemaIntegrity()
        XCTAssertTrue(healthy)

        // Batch + rollback must NOT persist rolled-back docs
        try await bridge.batchIndex(docs: [
            SearchDocument(id: 1, chatId: 10, text: "ephemeral_rollback_token"),
        ])
        await bridge.rollbackPending()
        let rolledBackHits = try await bridge.search(query: SearchQueryParams(query: "ephemeral_rollback_token"))
        XCTAssertTrue(rolledBackHits.isEmpty)

        // Batch + commit indexes English & CJK messages into real SQLite FTS5 + Tantivy
        try await bridge.batchIndex(docs: [
            SearchDocument(id: 10, chatId: 100, text: "Rust Tantivy search verification message"),
            SearchDocument(id: 11, chatId: 100, text: "全球法布施原生苹果设计风格"),
            SearchDocument(id: 12, chatId: 200, text: "Separate chat Tantivy verification"),
        ])
        try await bridge.commit()

        let engHits = try await bridge.search(query: SearchQueryParams(query: "Tantivy", chatId: 100))
        XCTAssertEqual(engHits.count, 1)
        XCTAssertEqual(engHits[0].id, 10)
        XCTAssertTrue(engHits[0].snippetWithHighlights.contains("<mark>Tantivy</mark>"))

        let cjkHits = try await bridge.search(query: SearchQueryParams(query: "全球法布施", chatId: 100))
        XCTAssertEqual(cjkHits.count, 1)
        XCTAssertEqual(cjkHits[0].id, 11)
        XCTAssertTrue(cjkHits[0].snippetWithHighlights.contains("<mark>全球法布施</mark>"))

        let rebuiltCount = try await bridge.rebuildIndex()
        XCTAssertEqual(rebuiltCount, 3)

        await host.shutdown()
    }

    func testAccountScopeManagerHasZeroFakeDefaultsAndFailsClosedOnUnknownAccount() async throws {
        let dir = try makeTempDir("account_scope")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.scope.\(UUID().uuidString)",
            account: "scope"
        )
        defer {
            try? keychain.deleteAccountKey(accountId: "901")
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
        let initialAccount = await scope.getActiveAccountId()
        XCTAssertNil(initialAccount, "AccountScopeManager must default to nil active account (zero fake defaults)")

        // Switching to unknown account MUST fail closed with FenceRejectionError.accountNotFound
        do {
            _ = try await scope.switchAccount(to: "999999")
            XCTFail("Expected accountNotFound")
        } catch let err as FenceRejectionError {
            XCTAssertEqual(err, .accountNotFound("999999"))
        }

        // Registering real account provisions real Keychain key, logout scrubs it
        let gen = try await scope.registerAccountSession(FabushiSessionAuthParams(
            userId: 901,
            username: "real_user",
            sessionId: "sess-901",
            token: "tok-901"
        ))
        XCTAssertEqual(gen, 2)
        let hasKeyBefore = try await scope.hasActiveKey(for: "901")
        XCTAssertTrue(hasKeyBefore)

        let postLogoutGen = try await scope.logout()
        XCTAssertEqual(postLogoutGen, 3)
        let hasKeyAfter = try await scope.hasActiveKey(for: "901")
        XCTAssertFalse(hasKeyAfter)

        await host.shutdown()
    }

    func testMediaTransferManagerRealCASRoundTripShaMismatchAndConcurrencyThrottle() async throws {
        let dir = try makeTempDir("media_cas")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        let manager = MediaTransferManager(host: host)

        // 1. Real upload -> real CAS download round-trip
        let sampleData = Data((0..<700_000).map { UInt8(($0 &* 31) & 0xFF) }) // 2 chunks (512KB + remainder)
        let expectedSha = MediaTransferManager.computeSHA256(data: sampleData)
        let srcFile = dir.appendingPathComponent("roundtrip.bin")
        try sampleData.write(to: srcFile)

        let mediaId = try await manager.uploadMedia(
            fileURL: srcFile,
            mimeType: "application/octet-stream",
            fileName: "roundtrip.bin"
        )

        // Wrong expected SHA-256 MUST fail closed
        do {
            _ = try await manager.downloadMedia(
                mediaId: mediaId,
                fileSha256: String(repeating: "f", count: 64),
                totalBytes: Int64(sampleData.count),
                fileName: "corrupt.bin"
            )
            XCTFail("Expected sha256Mismatch")
        } catch let err as MediaTransferError {
            if case .sha256Mismatch = err {
                // Expected fail-closed verification
            } else {
                XCTFail("Expected sha256Mismatch, got \(err)")
            }
        }

        // Correct expected SHA-256 succeeds and caches atomically on disk
        let downloaded = try await manager.downloadMedia(
            mediaId: mediaId,
            fileSha256: expectedSha,
            totalBytes: Int64(sampleData.count),
            fileName: "roundtrip.bin"
        )
        defer { try? FileManager.default.removeItem(at: downloaded) }
        XCTAssertEqual(try Data(contentsOf: downloaded), sampleData)

        // 2. Enforce maxConcurrentTransfers == 4 across 8 concurrent uploads
        var uploadFiles: [URL] = []
        for idx in 0..<8 {
            let f = dir.appendingPathComponent("concurrent_\(idx).bin")
            try Data(repeating: UInt8(idx + 1), count: 600_000).write(to: f)
            uploadFiles.append(f)
        }

        let completedIds = try await withThrowingTaskGroup(of: String.self) { group in
            for (idx, fileURL) in uploadFiles.enumerated() {
                group.addTask {
                    try await manager.uploadMedia(
                        fileURL: fileURL,
                        mimeType: "application/octet-stream",
                        fileName: "concurrent_\(idx).bin"
                    )
                }
            }
            var ids: [String] = []
            for try await id in group {
                ids.append(id)
            }
            return ids
        }

        XCTAssertEqual(completedIds.count, 8)
        let peak = await manager.maxObservedConcurrentTransfers()
        XCTAssertGreaterThan(peak, 0)
        XCTAssertLessThanOrEqual(peak, MediaTransferManager.maxConcurrentTransfers)
        let activeAfter = await manager.currentActiveTransferCount()
        XCTAssertEqual(activeAfter, 0)

        await host.shutdown()
    }

    func testTelegramReplayWindowAndFixedEventStreamSchemas() async throws {
        // Real Rust C-ABI 128-bit sliding replay window + maxPacketSize
        XCTAssertEqual(TelegramReplayWindow.maxPacketSize, 16_777_216)
        let window = try TelegramReplayWindow(safe: ())
        XCTAssertFalse(window.checkAndAdd(seq: 0), "seq 0 must be rejected")
        XCTAssertTrue(window.checkAndAdd(seq: 10))
        XCTAssertFalse(window.checkAndAdd(seq: 10), "duplicate seq 10 must be rejected")
        XCTAssertTrue(window.checkAndAdd(seq: 5), "in-window out-of-order seq 5 must be accepted")
        XCTAssertFalse(window.checkAndAdd(seq: 5), "duplicate out-of-order seq 5 must be rejected")
        XCTAssertTrue(window.checkAndAdd(seq: 200), "advancing highest to 200 must succeed")
        XCTAssertFalse(window.checkAndAdd(seq: 50), "stale seq 50 (>128 behind 200) must be rejected")

        // Real M1b event JSON schemas parsed accurately
        let connEvent = TelegramEventStream.parseEvent(
            type: "connectionStateChanged",
            json: #"{"status":"Connected","server_url":"http://34.46.77.133:8080"}"#
        )
        XCTAssertEqual(connEvent, .connectionStateChanged(state: "Connected"))

        let reactionEvent = TelegramEventStream.parseEvent(
            type: "reactionUpdated",
            json: #"{"chat_id":10,"message_id":99,"user_id":5,"reactions":[{"emoji":"🙏","count":3,"chosen_by_me":true}]}"#
        )
        XCTAssertEqual(
            reactionEvent,
            .reactionUpdated(chatId: 10, messageId: 99, emoji: "🙏", count: 3, chosenByMe: true)
        )

        let statusEvent = TelegramEventStream.parseEvent(
            type: "userStatusChanged",
            json: #"{"user_id":42,"status":"Online"}"#
        )
        XCTAssertEqual(statusEvent, .userStatusChanged(userId: 42, isOnline: true))

        // Bounded buffering caps memory and retains newest events
        let boundedStream = TelegramEventStream(maxBufferedEvents: 4)
        let consumer = boundedStream.makeStream(bufferCapacity: 4)
        for i in 1...10 {
            boundedStream.yield(.ptsUpdate(pts: UInt64(i), ptsCount: 1))
        }
        boundedStream.finish()

        var drainedPts: [UInt64] = []
        for await ev in consumer {
            if case .ptsUpdate(let pts, _) = ev {
                drainedPts.append(pts)
            }
        }
        XCTAssertEqual(drainedPts, [7, 8, 9, 10], "Bounded .bufferingNewest(4) must retain newest 4 events")
    }
}
