import Combine
import Foundation
import Security

// MARK: - Telegram Bridge Errors

public enum TelegramBridgeError: LocalizedError, Sendable, Equatable {
    case notInitialized
    case serializationFailed
    case invalidResponse(String)
    case serverError(code: String, message: String)
    case rustPanic(String)
    case keychainAccessFailed(OSStatus)

    public var errorDescription: String? {
        switch self {
        case .notInitialized:
            return "Fabushi Rust core runtime is not initialized."
        case .serializationFailed:
            return "Failed to serialize JSON-RPC command parameters."
        case .invalidResponse(let raw):
            return "Invalid or unparseable response from Rust runtime: \(raw)"
        case .serverError(let code, let message):
            return "[\(code)] \(message)"
        case .rustPanic(let message):
            return "Rust runtime panic: \(message)"
        case .keychainAccessFailed(let status):
            return "Failed to access iOS Keychain storage passphrase (status: \(status))."
        }
    }

    static func fromHostError(_ error: FabushiHostError) -> TelegramBridgeError {
        switch error {
        case .notInitialized:
            return .notInitialized
        case .serializationFailed:
            return .serializationFailed
        case .invalidResponse(let raw):
            return .invalidResponse(raw)
        case .rustPanic(let message):
            return .rustPanic(message)
        case .keychainFailed(let status):
            return .keychainAccessFailed(status)
        default:
            return .serverError(code: error.code, message: error.localizedDescription)
        }
    }
}

// MARK: - Swift 6 @MainActor Facade Over Canonical FabushiRustHostActor

/// Thin `@MainActor` facade bridging existing Presentation ViewModels onto the canonical
/// `FabushiRustHostActor` and fail-closed `FabushiKeychainStore` with zero mocks,
/// zero blocking main-thread sync queues, and full Swift 6 strict-concurrency compliance.
@MainActor
public final class TelegramBridgeRuntime: ObservableObject {
    public static let shared = TelegramBridgeRuntime(host: .shared, keychain: .shared)

    public let host: FabushiRustHostActor
    private let keychain: FabushiKeychainStore
    private let eventPipeline = TelegramEventStream()

    nonisolated(unsafe) private var eventForwardTask: Task<Void, Never>?

    public var eventStream: AsyncStream<TelegramEvent> {
        eventPipeline.stream
    }

    @Published public private(set) var isReady: Bool = false
    @Published public private(set) var connectionStatus: String = "Disconnected"

    public init(
        host: FabushiRustHostActor = FabushiRustHostActor(),
        keychain: FabushiKeychainStore = .shared
    ) {
        self.host = host
        self.keychain = keychain
    }

    deinit {
        eventForwardTask?.cancel()
        eventPipeline.finish()
    }

    // MARK: - Lifecycle Management

    public func initialize(
        appDataDirectory: URL? = nil,
        serverURL: String? = nil,
        isTestMode: Bool = false
    ) async throws {
        if await host.isInitialized {
            if !isReady {
                startEventForwarding()
                await refreshStatusFromHost()
                isReady = true
            }
            return
        }

        let directory = appDataDirectory ?? FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        )[0].appendingPathComponent("FabushiCore", isDirectory: true)

        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        let passphrase: String?
        if isTestMode {
            passphrase = nil
        } else {
            do {
                passphrase = try keychain.loadOrCreateStoragePassphrase()
            } catch let kcError as FabushiKeychainError {
                if case .osStatus(let status) = kcError {
                    throw TelegramBridgeError.keychainAccessFailed(status)
                }
                throw TelegramBridgeError.serverError(
                    code: "keychain_error",
                    message: kcError.localizedDescription
                )
            }
        }

        let config = FabushiHostConfig(
            appDataDir: directory.path,
            storagePassphrase: passphrase,
            serverUrl: serverURL,
            httpTimeoutSecs: FabushiHostConfig.defaultHttpTimeoutSecs,
            testMode: isTestMode,
            iosSandboxStrict: true
        )

        do {
            try await host.initialize(config: config)
        } catch let hostError as FabushiHostError {
            throw TelegramBridgeError.fromHostError(hostError)
        }

        startEventForwarding()
        await refreshStatusFromHost()
        isReady = true
    }

    public func shutdown() async {
        eventForwardTask?.cancel()
        eventForwardTask = nil
        isReady = false
        connectionStatus = "Disconnected"
        eventPipeline.finish()
        await host.shutdown()
    }

    // MARK: - Typed Command Dispatch

    public func dispatch<T: Decodable & Sendable, R: Encodable & Sendable>(
        command: String,
        params: R
    ) async throws -> T {
        do {
            return try await host.dispatch(method: command, params: params)
        } catch let hostError as FabushiHostError {
            throw TelegramBridgeError.fromHostError(hostError)
        }
    }

    public func dispatch<T: Decodable & Sendable>(command: String) async throws -> T {
        try await dispatch(command: command, params: FabushiEmptyParams())
    }

    @discardableResult
    public func dispatch(command: String) async throws -> FabushiJSONValue {
        try await dispatch(command: command, params: FabushiEmptyParams())
    }

    // MARK: - Internal Event Forwarding & Status Sync

    private func startEventForwarding() {
        eventForwardTask?.cancel()
        let domainStream = host.makeEventStream()
        eventForwardTask = Task { @MainActor [weak self] in
            for await domainEvent in domainStream {
                guard !Task.isCancelled, let self else { break }
                if case .connectionStateChanged(let status, _) = domainEvent.kind {
                    self.connectionStatus = status
                }
                self.eventPipeline.yield(domainEvent: domainEvent)
            }
        }
    }

    private func refreshStatusFromHost() async {
        if let ws = try? await host.wsStatus() {
            connectionStatus = ws.status
        } else {
            connectionStatus = "Disconnected"
        }
    }
}
