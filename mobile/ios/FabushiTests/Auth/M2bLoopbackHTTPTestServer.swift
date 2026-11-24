import Darwin
import Foundation
import os

// MARK: - Self-Contained POSIX 127.0.0.1:0 Loopback HTTP Server for Real Rust C-ABI M2b XCTests

internal final class M2bLoopbackHTTPTestServer: Sendable {
    struct Request: Sendable {
        let method: String
        let path: String
        let query: String
        let headers: [String: String]
        let body: Data

        var bodyString: String {
            String(decoding: body, as: UTF8.self)
        }

        var fullTarget: String {
            query.isEmpty ? path : "\(path)?\(query)"
        }
    }

    struct Response: Sendable {
        let statusCode: Int
        let statusText: String
        let contentType: String
        let body: Data

        static func json(_ statusCode: Int = 200, _ jsonString: String) -> Response {
            Response(
                statusCode: statusCode,
                statusText: reasonPhrase(for: statusCode),
                contentType: "application/json",
                body: Data(jsonString.utf8)
            )
        }

        static func jsonObject(_ statusCode: Int = 200, _ object: Any) -> Response {
            let data = (try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])) ?? Data("{}".utf8)
            return Response(
                statusCode: statusCode,
                statusText: reasonPhrase(for: statusCode),
                contentType: "application/json",
                body: data
            )
        }

        private static func reasonPhrase(for statusCode: Int) -> String {
            switch statusCode {
            case 200: return "OK"
            case 201: return "Created"
            case 204: return "No Content"
            case 400: return "Bad Request"
            case 401: return "Unauthorized"
            case 403: return "Forbidden"
            case 404: return "Not Found"
            case 409: return "Conflict"
            case 429: return "Too Many Requests"
            default: return "Internal Server Error"
            }
        }
    }

    private struct State: Sendable {
        var listenFD: Int32 = -1
        var stopped: Bool = false
        var requests: [Request] = []
    }

    private let lock = OSAllocatedUnfairLock(initialState: State())
    private let handler: @Sendable (Request) -> Response
    private let acceptQueue = DispatchQueue(label: "com.ombhrum.fabushi.m2b.loopback.accept", qos: .userInitiated)
    private let workerQueue = DispatchQueue(
        label: "com.ombhrum.fabushi.m2b.loopback.worker",
        qos: .userInitiated,
        attributes: .concurrent
    )

    let port: UInt16

    init(handler: @escaping @Sendable (Request) -> Response) throws {
        self.handler = handler

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

        guard Darwin.listen(fd, 64) == 0 else {
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

    func recordedRequests() -> [Request] {
        lock.withLock { $0.requests }
    }

    func requests(method: String, pathPrefix: String) -> [Request] {
        lock.withLock { state in
            state.requests.filter { $0.method == method && $0.path.hasPrefix(pathPrefix) }
        }
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

    private var isStopped: Bool {
        lock.withLock { $0.stopped }
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

    private func handleConnection(_ clientFD: Int32) {
        defer { Darwin.close(clientFD) }

        var rawData = Data()
        var headerEndOffset: Int?
        var expectedContentLength = 0
        var chunk = [UInt8](repeating: 0, count: 16_384)
        let separator = Data([0x0D, 0x0A, 0x0D, 0x0A])

        while rawData.count < 2_000_000 {
            let n = Darwin.recv(clientFD, &chunk, chunk.count, 0)
            if n <= 0 { break }
            rawData.append(contentsOf: chunk[0..<n])

            if headerEndOffset == nil, let range = rawData.range(of: separator) {
                headerEndOffset = range.upperBound
                let headerText = String(decoding: rawData[..<range.lowerBound], as: UTF8.self)
                for line in headerText.components(separatedBy: "\r\n").dropFirst() {
                    if let colon = line.firstIndex(of: ":") {
                        let name = line[..<colon].trimmingCharacters(in: .whitespaces).lowercased()
                        let value = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
                        if name == "content-length", let length = Int(value) {
                            expectedContentLength = max(0, length)
                        }
                    }
                }
            }

            if let bodyStart = headerEndOffset, rawData.count - bodyStart >= expectedContentLength {
                break
            }
        }

        guard let bodyStart = headerEndOffset, bodyStart >= 4 else { return }
        let headerData = rawData[..<(bodyStart - 4)]
        let headerText = String(decoding: headerData, as: UTF8.self)
        let lines = headerText.components(separatedBy: "\r\n")
        guard let requestLine = lines.first else { return }
        let parts = requestLine.split(separator: " ")
        guard parts.count >= 2 else { return }

        let method = String(parts[0])
        let target = String(parts[1])
        let path: String
        let query: String
        if let qIdx = target.firstIndex(of: "?") {
            path = String(target[..<qIdx])
            query = String(target[target.index(after: qIdx)...])
        } else {
            path = target
            query = ""
        }

        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            if let colon = line.firstIndex(of: ":") {
                let name = line[..<colon].trimmingCharacters(in: .whitespaces).lowercased()
                let value = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
                headers[name] = value
            }
        }

        let bodySliceEnd = min(rawData.count, bodyStart + expectedContentLength)
        let bodyData = expectedContentLength > 0
            ? Data(rawData[bodyStart..<bodySliceEnd])
            : Data(rawData[bodyStart...])

        let request = Request(
            method: method,
            path: path,
            query: query,
            headers: headers,
            body: bodyData
        )
        lock.withLock { $0.requests.append(request) }

        let response = handler(request)
        let headerString = "HTTP/1.1 \(response.statusCode) \(response.statusText)\r\nContent-Type: \(response.contentType)\r\nContent-Length: \(response.body.count)\r\nConnection: close\r\n\r\n"
        var wireBytes = Data(headerString.utf8)
        wireBytes.append(response.body)

        wireBytes.withUnsafeBytes { rawBuf in
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
