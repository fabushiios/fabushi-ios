import Foundation
import XCTest
@testable import Fabushi

final class FabushiRustHostActorTests: XCTestCase, @unchecked Sendable {

    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_host_actor_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    func testLifecyclePingUnknownCommandEmptyChatsAndPostShutdownClosed() async throws {
        XCTAssertEqual(FabushiRPCNamespace.allCases.count, 33)

        let dir = try makeTempDir("lifecycle")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()

        // Before initialization: fail closed with notInitialized
        do {
            _ = try await host.ping(nonce: 1)
            XCTFail("Expected notInitialized before initialize()")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err, .notInitialized)
        }

        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            serverUrl: nil,
            httpTimeoutSecs: 10,
            testMode: true,
            iosSandboxStrict: true
        ))

        let ping = try await host.ping(nonce: 12345)
        XCTAssertTrue(ping.pong)
        XCTAssertEqual(ping.nonce, 12345)
        XCTAssertEqual(ping.serverUrl, FabushiHostConfig.defaultServerURL)
        XCTAssertGreaterThanOrEqual(ping.generation, 1)

        // Unknown command MUST fail closed with `unknown_command` (zero "acknowledged" fallback)
        do {
            let _: FabushiJSONValue = try await host.dispatch(method: "nonexistent.unknownMethod")
            XCTFail("Expected unknown_command")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err.code, "unknown_command")
            if case .unknownCommand(let method, _) = err {
                XCTAssertEqual(method, "nonexistent.unknownMethod")
            } else {
                XCTFail("Expected .unknownCommand case, got \(err)")
            }
        }

        // Fresh chats.list MUST return empty array (zero fake sample chats)
        struct LocalOnlyChatsParams: Encodable, Sendable {
            let local_only: Bool
        }
        let chats: [FabushiJSONValue] = try await host.dispatch(
            method: FabushiRPCNamespace.chats.method("list"),
            params: LocalOnlyChatsParams(local_only: true)
        )
        XCTAssertTrue(chats.isEmpty, "Fresh local store must return empty chats list, got \(chats)")

        // Shutdown closes host; subsequent dispatches fail closed with host_closed
        await host.shutdown()
        let isInitAfterShutdown = await host.isInitialized
        XCTAssertFalse(isInitAfterShutdown)

        do {
            _ = try await host.ping(nonce: 2)
            XCTFail("Expected host_closed after shutdown()")
        } catch let err as FabushiHostError {
            XCTAssertEqual(err.code, "host_closed")
        }
    }

    /// Verifies both:
    /// 1. Typed Swift decoding of Rust's `catch_unwind` `host_panic` envelope into `.rustPanic`.
    /// 2. Live C-ABI behavior: Release iOS staticlibs (`#[cfg(all(target_os = "ios", not(debug_assertions)))]`
    ///    in `source/host/src/host.rs`) intentionally force `test_mode = false`, so `debug.trigger_panic`
    ///    fails closed with `unknown_command` on Release CI staticlibs (while live Rust `catch_unwind` ->
    ///    `host_panic` is exercised in `rust-host-ffi-ubuntu` debug builds), and the host remains healthy.
    func testDebugTriggerPanicSurvivesCatchUnwindAndHostRemainsHealthy() async throws {
        // 1. Verify typed Swift error decoding of Rust catch_unwind `host_panic` envelope
        let decodedPanic = FabushiHostError.fromRPCError(
            FabushiRPCErrorPayload(code: "host_panic", message: "simulated rust panic", retryable: false),
            method: "debug.trigger_panic",
            requestId: "req-panic",
            requestedGeneration: 1,
            responseGeneration: 1
        )
        XCTAssertEqual(decodedPanic, .rustPanic(message: "simulated rust panic"))
        XCTAssertEqual(decodedPanic.code, "host_panic")

        // 2. Verify live C-ABI fail-closed behavior and post-call host health
        let dir = try makeTempDir("panic")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        do {
            let _: FabushiJSONValue = try await host.dispatch(method: "debug.trigger_panic")
            XCTFail("debug.trigger_panic must fail closed (unknown_command on Release iOS or host_panic on debug)")
        } catch let err as FabushiHostError {
            XCTAssertTrue(
                err.code == "unknown_command" || err.code == "host_panic",
                "Expected fail-closed unknown_command (Release iOS gate) or host_panic (debug catch_unwind), got \(err)"
            )
        }

        // Subsequent RPC MUST continue working normally
        let ping = try await host.ping(nonce: 777)
        XCTAssertTrue(ping.pong)
        XCTAssertEqual(ping.nonce, 777)

        await host.shutdown()
    }

    func testDesktopOnlyCapabilitiesFailClosedOnIOS() async throws {
        let dir = try makeTempDir("capabilities")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        let forbiddenDesktopMethods = [
            "coding.probeLocal",
            "remote.localCapture",
            "magpie.scanLocalSubscriptions",
            "fabushi.executeLocalTerminalTool",
        ]

        for method in forbiddenDesktopMethods {
            do {
                let _: FabushiJSONValue = try await host.dispatch(method: method)
                XCTFail("Expected capability_unavailable for desktop-only method \(method)")
            } catch let err as FabushiHostError {
                XCTAssertEqual(
                    err.code,
                    "capability_unavailable",
                    "Method \(method) must fail closed with capability_unavailable, got \(err)"
                )
            }
        }

        await host.shutdown()
    }

    func testRealEventStreamPollsNewMessageFromLocalSend() async throws {
        let dir = try makeTempDir("events")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            testMode: true,
            iosSandboxStrict: true
        ))

        let stream = host.makeEventStream(bufferCapacity: 1024)
        let listener = Task<[FabushiDomainEvent], Never> {
            var collected: [FabushiDomainEvent] = []
            for await event in stream {
                if case .newMessage(_, let chatId, _, _, _, _, _, _) = event.kind, chatId == 9001 {
                    collected.append(event)
                    if collected.count == 2 {
                        break
                    }
                }
            }
            return collected
        }

        struct SendMessageParams: Encodable, Sendable {
            let chat_id: Int64
            let text: String
            let local_only: Bool
        }

        let _: FabushiJSONValue = try await host.dispatch(
            method: FabushiRPCNamespace.messages.method("send"),
            params: SendMessageParams(chat_id: 9001, text: "First Rust polled event", local_only: true)
        )
        let _: FabushiJSONValue = try await host.dispatch(
            method: FabushiRPCNamespace.messages.method("send"),
            params: SendMessageParams(chat_id: 9001, text: "Second Rust polled event", local_only: true)
        )

        let events = await withTaskGroup(of: [FabushiDomainEvent]?.self) { group in
            group.addTask { await listener.value }
            group.addTask {
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                return nil
            }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
        listener.cancel()

        let unwrapped = try XCTUnwrap(events, "Timed out waiting for fabushi_host_poll_events")
        XCTAssertEqual(unwrapped.count, 2)
        XCTAssertGreaterThan(unwrapped[0].seq, 0)
        XCTAssertGreaterThan(unwrapped[1].seq, unwrapped[0].seq, "Event seq must be strictly monotonic")
        XCTAssertGreaterThanOrEqual(unwrapped[0].generation, 1)
        if case .newMessage(_, let chatId, _, let text, _, _, _, _) = unwrapped[0].kind {
            XCTAssertEqual(chatId, 9001)
            XCTAssertEqual(text, "First Rust polled event")
        } else {
            XCTFail("Expected .newMessage event, got \(unwrapped[0].kind)")
        }

        await host.shutdown()
    }

    func testConcurrentInitializeCoalescesWithoutLockBusyOrHandleLeak() async throws {
        let dir = try makeTempDir("concurrent_init")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        let config = FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            serverUrl: nil,
            httpTimeoutSecs: 10,
            testMode: true,
            iosSandboxStrict: true
        )

        // 4 concurrent initialize(config:) calls on the exact same appDataDir must coalesce
        // onto a single fabushi_host_create with zero Tantivy LockBusy and zero leaked handles.
        try await withThrowingTaskGroup(of: Void.self) { group in
            for _ in 0..<4 {
                group.addTask {
                    try await host.initialize(config: config)
                }
            }
            for try await _ in group {}
        }

        let isInit = await host.isInitialized
        XCTAssertTrue(isInit)
        let ping = try await host.ping(nonce: 404)
        XCTAssertTrue(ping.pong)
        XCTAssertEqual(ping.nonce, 404)

        // Single shutdown must drop the sole Rust handle and release all Tantivy/SQLite locks.
        await host.shutdown()
        let isInitAfter = await host.isInitialized
        XCTAssertFalse(isInitAfter)
        let genAfterShutdown = await host.latestObservedGeneration
        XCTAssertEqual(genAfterShutdown, 1)

        // Immediate cold relaunch on the exact same appDataDir proves zero leaked handle / LockBusy.
        let relaunched = FabushiRustHostActor()
        try await relaunched.initialize(config: config)
        let ping2 = try await relaunched.ping(nonce: 405)
        XCTAssertTrue(ping2.pong)
        XCTAssertEqual(ping2.nonce, 405)
        await relaunched.shutdown()
    }

    func testConcurrentShutdownDuringInitializeLeavesHostClosed() async throws {
        let dir = try makeTempDir("race_init_shutdown")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        let config = FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            serverUrl: nil,
            httpTimeoutSecs: 10,
            testMode: true,
            iosSandboxStrict: true
        )

        let initTask = Task<Void, Error> {
            try await host.initialize(config: config)
        }

        // Wait deterministically until initialize() has entered the actor so shutdown() races
        // with or immediately follows initialization (never preceding it).
        for _ in 0..<2_000 {
            let inFlight = await host.hasInFlightInitialization
            let initialized = await host.isInitialized
            if inFlight || initialized {
                break
            }
            await Task.yield()
        }

        // Two concurrent shutdown() callers must coalesce and wait until teardown completes.
        async let shutdown1: Void = host.shutdown()
        async let shutdown2: Void = host.shutdown()
        _ = await (shutdown1, shutdown2)
        _ = try? await initTask.value

        let isInitAfter = await host.isInitialized
        XCTAssertFalse(isInitAfter, "Host must remain closed after initialize()/shutdown() race")

        // Immediate fresh host on the exact same appDataDir proves zero zombie host / LockBusy.
        let freshHost = FabushiRustHostActor()
        try await freshHost.initialize(config: config)
        let ping = try await freshHost.ping(nonce: 501)
        XCTAssertTrue(ping.pong)
        await freshHost.shutdown()
    }

    func testShutdownDrainsInFlightRPCAndReleasesLocks() async throws {
        let server = try LoopbackHTTPTestServer()
        defer { server.stop() }
        server.setSlowHealthDelayMs(3_000)

        let dir = try makeTempDir("shutdown_rpc_drain")
        defer { try? FileManager.default.removeItem(at: dir) }

        let host1 = FabushiRustHostActor()
        let config = FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            serverUrl: server.baseURLString,
            httpTimeoutSecs: 10,
            testMode: true,
            iosSandboxStrict: true
        )
        try await host1.initialize(config: config)

        // Launch an in-flight slow HTTP RPC so rpcQueue holds a cloned Arc<FabushiAppHost>.
        let slowRPC = Task<FabushiJSONValue, Error> {
            try await host1.dispatch(method: FabushiRPCNamespace.auth.method("health"))
        }

        for _ in 0..<80 {
            if server.hitCount(for: "GET /health") > 0 { break }
            try? await Task.sleep(nanoseconds: 25_000_000)
        }
        XCTAssertGreaterThanOrEqual(server.hitCount(for: "GET /health"), 1)

        // shutdown() must cancel in-flight RPCs and drain BOTH pollGroup and rpcGroup before returning.
        await host1.shutdown()

        do {
            _ = try await slowRPC.value
            XCTFail("Expected in-flight RPC to abort when host shuts down")
        } catch {
            // Expected cancellation / host_closed error
        }

        // Immediate cold relaunch on the exact same appDataDir proves every Rust Arc<FabushiAppHost>
        // dropped before shutdown() returned (zero Tantivy IndexWriter LockBusy).
        let host2 = FabushiRustHostActor()
        try await host2.initialize(config: config)
        let ping = try await host2.ping(nonce: 601)
        XCTAssertTrue(ping.pong)
        XCTAssertEqual(ping.nonce, 601)
        await host2.shutdown()
    }
}
