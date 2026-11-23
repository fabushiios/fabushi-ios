import Darwin
import Foundation
import os
import XCTest
@testable import Fabushi

// MARK: - Minimal POSIX 127.0.0.1:0 Loopback HTTP Server (Zero External Dependencies)

private final class LoopbackHTTPTestServer: Sendable {
    private struct ServerState: Sendable {
        var listenFD: Int32 = -1
        var stopped: Bool = false
        var slowHealthDelayMs: Int = 0
        var routeHits: [String: Int] = [:]
        var lastChatsAuthHeader: String? = nil
    }

    private let lock = OSAllocatedUnfairLock(initialState: ServerState())
    private let acceptQueue = DispatchQueue(label: "com.ombhrum.fabushi.test.loopback.accept", qos: .userInitiated)
    private let workerQueue = DispatchQueue(
        label: "com.ombhrum.fabushi.test.loopback.worker",
        qos: .userInitiated,
        attributes: .concurrent
    )

    public let port: UInt16

    init() throws {
        let fd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else {
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
        }

        var reuse: Int32 = 1
        Darwin.setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &reuse, socklen_t(MemoryLayout<Int32>.size))

        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = 0
        addr.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))

        let bindResult = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                Darwin.bind(fd, sockPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindResult == 0 else {
            let err = errno
            Darwin.close(fd)
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(err))
        }

        guard Darwin.listen(fd, 32) == 0 else {
            let err = errno
            Darwin.close(fd)
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(err))
        }

        var boundAddr = sockaddr_in()
        var boundLen = socklen_t(MemoryLayout<sockaddr_in>.size)
        let nameResult = withUnsafeMutablePointer(to: &boundAddr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                Darwin.getsockname(fd, sockPtr, &boundLen)
            }
        }
        guard nameResult == 0 else {
            let err = errno
            Darwin.close(fd)
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(err))
        }

        self.port = UInt16(bigEndian: boundAddr.sin_port)
        lock.withLock { $0.listenFD = fd }
        startAcceptLoop(listenFD: fd)
    }

    deinit {
        stop()
    }

    var baseURLString: String {
        "http://127.0.0.1:\(port)"
    }

    func setSlowHealthDelayMs(_ delayMs: Int) {
        lock.withLock { $0.slowHealthDelayMs = max(0, delayMs) }
    }

    func hitCount(for key: String) -> Int {
        lock.withLock { $0.routeHits[key, default: 0] }
    }

    func capturedChatsAuthHeader() -> String? {
        lock.withLock { $0.lastChatsAuthHeader }
    }

    func stop() {
        let fdToClose = lock.withLock { state -> Int32 in
            state.stopped = true
            let fd = state.listenFD
            state.listenFD = -1
            return fd
        }
        if fdToClose >= 0 {
            Darwin.shutdown(fdToClose, SHUT_RDWR)
            Darwin.close(fdToClose)
        }
    }

    private func startAcceptLoop(listenFD: Int32) {
        acceptQueue.async { [weak self] in
            while let self, !self.isStopped {
                var clientAddr = sockaddr_storage()
                var clientLen = socklen_t(MemoryLayout<sockaddr_storage>.size)
                let clientFD = withUnsafeMutablePointer(to: &clientAddr) { ptr in
                    ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                        Darwin.accept(listenFD, sockPtr, &clientLen)
                    }
                }
                if clientFD < 0 {
                    if self.isStopped { break }
                    continue
                }
                var noSigPipe: Int32 = 1
                Darwin.setsockopt(
                    clientFD,
                    SOL_SOCKET,
                    SO_NOSIGPIPE,
                    &noSigPipe,
                    socklen_t(MemoryLayout<Int32>.size)
                )
                self.workerQueue.async { [weak self] in
                    self?.handleConnection(clientFD)
                }
            }
        }
    }

    private var isStopped: Bool {
        lock.withLock { $0.stopped }
    }

    private func handleConnection(_ clientFD: Int32) {
        defer { Darwin.close(clientFD) }

        var buffer = [UInt8](repeating: 0, count: 16_384)
        let bytesRead = Darwin.recv(clientFD, &buffer, buffer.count, 0)
        guard bytesRead > 0 else { return }

        let requestText = String(decoding: buffer[0..<bytesRead], as: UTF8.self)
        let lines = requestText.components(separatedBy: "\r\n")
        guard let requestLine = lines.first else { return }
        let parts = requestLine.split(separator: " ")
        guard parts.count >= 2 else { return }
        let method = String(parts[0])
        let path = String(parts[1])
        let routeKey = "\(method) \(path)"

        var authHeader: String?
        for line in lines.dropFirst() {
            if line.lowercased().hasPrefix("authorization:") {
                authHeader = String(line.dropFirst("authorization:".count)).trimmingCharacters(in: .whitespaces)
            }
        }

        let capturedAuthHeader = authHeader
        let slowDelayMs = lock.withLock { state -> Int in
            state.routeHits[routeKey, default: 0] += 1
            if routeKey == "GET /api/v1/chats" {
                state.lastChatsAuthHeader = capturedAuthHeader
            }
            return state.slowHealthDelayMs
        }

        let statusLine: String
        let contentType: String
        let body: String

        switch routeKey {
        case "GET /health":
            if slowDelayMs > 0 {
                let slices = max(1, slowDelayMs / 25)
                for _ in 0..<slices {
                    if isStopped { return }
                    Thread.sleep(forTimeInterval: 0.025)
                }
            }
            statusLine = "HTTP/1.1 200 OK"
            contentType = "text/plain"
            body = "ok"

        case "GET /api/v1/status":
            statusLine = "HTTP/1.1 200 OK"
            contentType = "application/json"
            body = #"{"server":"fabushi-sim-loopback","status":"ok","version":"1.6.9","capabilities":["http","ws"]}"#

        case "POST /api/v1/auth/login":
            statusLine = "HTTP/1.1 200 OK"
            contentType = "application/json"
            body = #"{"user_id":77,"username":"alice_ios","session_id":"sess-sim-1","token":"tok-sim-secret","first_name":"Alice"}"#

        case "GET /api/v1/chats":
            statusLine = "HTTP/1.1 200 OK"
            contentType = "application/json"
            body = #"[{"chat_id":300,"title":"Simulator Loopback","chat_type":2,"unread_count":1}]"#

        default:
            statusLine = "HTTP/1.1 404 Not Found"
            contentType = "application/json"
            body = #"{"error":"not_found"}"#
        }

        let bodyBytes = Array(body.utf8)
        let responseHeader = "\(statusLine)\r\nContent-Type: \(contentType)\r\nContent-Length: \(bodyBytes.count)\r\nConnection: close\r\n\r\n"
        var responseBytes = Array(responseHeader.utf8)
        responseBytes.append(contentsOf: bodyBytes)

        responseBytes.withUnsafeBytes { rawBuf in
            guard let base = rawBuf.baseAddress else { return }
            var totalSent = 0
            while totalSent < rawBuf.count {
                let sent = Darwin.send(clientFD, base.advanced(by: totalSent), rawBuf.count - totalSent, 0)
                if sent <= 0 { break }
                totalSent += sent
            }
        }
    }
}

// MARK: - Loopback ATS, End-to-End Rust HttpClient & Cancellation Tests

final class FabushiLoopbackAndCancellationTests: XCTestCase, @unchecked Sendable {

    func testBundleATSEnforcesStrictPublicTransportWithLocalNetworkingEnabled() throws {
        let ats = try XCTUnwrap(
            Bundle.main.object(forInfoDictionaryKey: "NSAppTransportSecurity") as? [String: Any],
            "Bundle.main must define NSAppTransportSecurity"
        )
        XCTAssertEqual(ats["NSAllowsArbitraryLoads"] as? Bool, false, "NSAllowsArbitraryLoads must stay false")
        XCTAssertEqual(ats["NSAllowsLocalNetworking"] as? Bool, true, "NSAllowsLocalNetworking must be true")
    }

    func testDualURLSessionAndRustHttpClientLoopbackEndToEnd() async throws {
        let server = try LoopbackHTTPTestServer()
        defer { server.stop() }

        // 1. Apple URLSession loopback proof (actively exercises Info.plist NSAllowsLocalNetworking = true)
        let healthURL = try XCTUnwrap(URL(string: "\(server.baseURLString)/health"))
        let (urlSessionData, urlSessionResponse) = try await URLSession.shared.data(from: healthURL)
        let httpResponse = try XCTUnwrap(urlSessionResponse as? HTTPURLResponse)
        XCTAssertEqual(httpResponse.statusCode, 200)
        XCTAssertEqual(String(data: urlSessionData, encoding: .utf8), "ok")
        XCTAssertEqual(server.hitCount(for: "GET /health"), 1)

        // 2. Real Rust HttpClient end-to-end loopback proof across auth.health, auth.status, auth.login, chats.list
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_loopback_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            serverUrl: server.baseURLString,
            httpTimeoutSecs: 10,
            testMode: true,
            iosSandboxStrict: true
        ))

        struct HealthResult: Decodable, Sendable {
            let healthy: Bool
            let server_url: String
        }
        let health: HealthResult = try await host.dispatch(method: FabushiRPCNamespace.auth.method("health"))
        XCTAssertTrue(health.healthy)
        XCTAssertEqual(health.server_url, server.baseURLString)
        XCTAssertEqual(server.hitCount(for: "GET /health"), 2)

        struct StatusResult: Decodable, Sendable {
            let server: String
            let status: String
            let version: String
            let capabilities: [String]
        }
        let status: StatusResult = try await host.dispatch(method: FabushiRPCNamespace.auth.method("status"))
        XCTAssertEqual(status.server, "fabushi-sim-loopback")
        XCTAssertEqual(status.status, "ok")
        XCTAssertEqual(status.version, "1.6.9")
        XCTAssertEqual(server.hitCount(for: "GET /api/v1/status"), 1)

        struct LoginParams: Encodable, Sendable {
            let username: String
            let password: String
            let device_name: String
        }
        struct LoginResult: Decodable, Sendable {
            let user_id: UInt64
            let username: String
            let session_id: String
            let token: String
        }
        let login: LoginResult = try await host.dispatch(
            method: FabushiRPCNamespace.auth.method("login"),
            params: LoginParams(username: "alice_ios", password: "correct-password", device_name: "Fabushi iOS Sim")
        )
        XCTAssertEqual(login.user_id, 77)
        XCTAssertEqual(login.username, "alice_ios")
        XCTAssertEqual(login.token, "tok-sim-secret")
        XCTAssertEqual(server.hitCount(for: "POST /api/v1/auth/login"), 1)

        let genAfterLogin = try await host.currentGeneration()
        XCTAssertEqual(genAfterLogin, 2, "auth.login must advance host generation")

        struct ChatDTO: Decodable, Sendable {
            let chat_id: Int64
            let title: String
            let unread_count: Int
        }
        let chats: [ChatDTO] = try await host.dispatch(
            method: FabushiRPCNamespace.chats.method("list"),
            generation: genAfterLogin
        )
        XCTAssertEqual(chats.count, 1)
        XCTAssertEqual(chats[0].chat_id, 300)
        XCTAssertEqual(chats[0].title, "Simulator Loopback")
        XCTAssertEqual(server.hitCount(for: "GET /api/v1/chats"), 1)
        XCTAssertEqual(server.capturedChatsAuthHeader(), "Bearer tok-sim-secret")

        await host.shutdown()
    }

    func testSwiftTaskCancelAbortsSlowRustRPCWithoutHeadOfLineBlocking() async throws {
        let server = try LoopbackHTTPTestServer()
        defer { server.stop() }
        server.setSlowHealthDelayMs(5_000)

        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_cancel_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        let host = FabushiRustHostActor()
        try await host.initialize(config: FabushiHostConfig(
            appDataDir: dir.path,
            storagePassphrase: nil,
            serverUrl: server.baseURLString,
            httpTimeoutSecs: 15,
            testMode: true,
            iosSandboxStrict: true
        ))

        let startTime = Date()
        let slowTask = Task<FabushiJSONValue, Error> {
            try await host.dispatch(method: FabushiRPCNamespace.auth.method("health"))
        }

        // Wait briefly until the slow request hits the loopback server so Rust has registered in_flight_cancels
        for _ in 0..<40 {
            if server.hitCount(for: "GET /health") > 0 { break }
            try? await Task.sleep(nanoseconds: 25_000_000)
        }
        XCTAssertGreaterThanOrEqual(server.hitCount(for: "GET /health"), 1)

        // Concurrent session.ping MUST succeed immediately with ZERO head-of-line blocking!
        let pingStart = Date()
        let ping = try await host.ping(nonce: 555)
        let pingElapsed = Date().timeIntervalSince(pingStart)
        XCTAssertTrue(ping.pong)
        XCTAssertEqual(ping.nonce, 555)
        XCTAssertLessThan(pingElapsed, 0.75, "Concurrent RPC must not be head-of-line blocked by slow HTTP RPC")

        // Cancel slowTask -> must forward fabushi_host_cancel and abort in < 1.5s (NEVER waiting 5s!)
        let cancelTime = Date()
        slowTask.cancel()

        do {
            _ = try await slowTask.value
            XCTFail("Expected slow auth.health task to abort after cancellation")
        } catch is CancellationError {
            // Cancelled cleanly
        } catch let hostErr as FabushiHostError {
            XCTAssertTrue(hostErr.isCancelled, "Expected cancelled error, got \(hostErr)")
        } catch {
            XCTFail("Unexpected non-cancellation error: \(error)")
        }

        let cancelElapsed = Date().timeIntervalSince(cancelTime)
        let totalElapsed = Date().timeIntervalSince(startTime)
        XCTAssertLessThan(cancelElapsed, 1.5, "Cancellation must abort slow RPC in < 1.5s (actual: \(cancelElapsed)s)")
        XCTAssertLessThan(totalElapsed, 3.0, "Total duration must be far below 5s server delay (actual: \(totalElapsed)s)")

        await host.shutdown()
    }
}
