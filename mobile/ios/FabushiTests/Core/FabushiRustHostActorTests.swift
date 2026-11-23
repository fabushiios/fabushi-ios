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

        // 2. Verify live C-ABI behavior: on Release iOS staticlibs, M1b security hardening
        // (source/host/src/host.rs:355) forces test_mode = false so debug.trigger_panic fails
        // closed with `unknown_command`; on debug staticlibs it triggers catch_unwind `host_panic`.
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
}
