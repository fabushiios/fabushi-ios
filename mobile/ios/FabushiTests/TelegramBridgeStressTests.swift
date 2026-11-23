import Foundation
import XCTest
@testable import Fabushi

final class TelegramBridgeStressTests: XCTestCase, @unchecked Sendable {

    struct PingParams: Encodable, Sendable {
        let nonce: Int
    }

    struct PingResponse: Decodable, Sendable {
        let pong: Bool
        let nonce: Int
        let timestamp: Int64
    }

    @MainActor
    func testConcurrentAsyncDispatchBursts() async throws {
        let runtime = TelegramBridgeRuntime()
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("test_bridge_burst_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        try await runtime.initialize(appDataDirectory: tempDir, isTestMode: true)

        let burstCount = 100
        var successes = 0

        await withTaskGroup(of: Result<PingResponse, Error>.self) { group in
            for i in 0..<burstCount {
                group.addTask {
                    do {
                        let res: PingResponse = try await runtime.dispatch(
                            command: "telegram.ping",
                            params: PingParams(nonce: i)
                        )
                        return .success(res)
                    } catch {
                        return .failure(error)
                    }
                }
            }

            for await result in group {
                if case .success(let resp) = result, resp.pong {
                    successes += 1
                }
            }
        }

        await runtime.shutdown()
        XCTAssertEqual(successes, burstCount, "Expected all \(burstCount) parallel dispatches to succeed without deadlock")
    }

    @MainActor
    func testHighFrequencyPushEventFlooding() async throws {
        let totalEvents = 5000
        let stream = TelegramEventStream(maxBufferedEvents: totalEvents)
        let eventSequence = stream.makeStream(bufferCapacity: totalEvents)

        let consumerTask = Task<Int, Never> {
            var count = 0
            for await _ in eventSequence {
                count += 1
                if count == totalEvents {
                    break
                }
            }
            return count
        }

        let workerCount = 4
        let eventsPerWorker = totalEvents / workerCount

        await withTaskGroup(of: Void.self) { group in
            for workerIdx in 0..<workerCount {
                group.addTask {
                    let start = workerIdx * eventsPerWorker
                    let end = start + eventsPerWorker
                    for i in start..<end {
                        let payload = "{\"id\":\(i),\"chat_id\":1001,\"sender_id\":2002,\"text\":\"Flooding #\(i)\",\"timestamp\":1700000000}"
                        stream.parseAndYield(type: "newMessage", json: payload)
                    }
                }
            }
        }

        stream.finish()
        let receivedCount = await consumerTask.value
        XCTAssertEqual(receivedCount, totalEvents, "Expected exactly \(totalEvents) events with zero drops")
    }

    @MainActor
    func testTaskCancellationResilience() async throws {
        let runtime = TelegramBridgeRuntime()
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("test_bridge_cancel_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        try await runtime.initialize(appDataDirectory: tempDir, isTestMode: true)

        let tasksCount = 50
        await withTaskGroup(of: Void.self) { group in
            for i in 0..<tasksCount {
                group.addTask {
                    let t = Task {
                        try await runtime.dispatch(
                            command: "telegram.ping",
                            params: PingParams(nonce: i)
                        ) as PingResponse
                    }
                    if i % 2 == 0 {
                        t.cancel()
                    }
                    _ = try? await t.value
                }
            }
        }

        // Post-cancellation verification: runtime remains operational
        var postSuccessCount = 0
        for i in 0..<20 {
            let resp: PingResponse = try await runtime.dispatch(
                command: "telegram.ping",
                params: PingParams(nonce: 1000 + i)
            )
            if resp.pong && resp.nonce == 1000 + i {
                postSuccessCount += 1
            }
        }
        await runtime.shutdown()
        XCTAssertEqual(postSuccessCount, 20, "All post-cancellation dispatches must succeed cleanly")
    }
}
