//
//  AccountScopeManager.swift
//  Fabushi
//
//  Account Scope Generation Fence backed by canonical Rust host generation.* / session.* RPCs
//  and real iOS Security.framework Keychain secret storage (zero hardcoded accounts or static keys).
//

import Foundation
import Security

public enum FenceRejectionError: Error, Equatable, LocalizedError, Sendable {
    case staleGeneration(taskGen: UInt64, currentGen: UInt64)
    case accountNotFound(String)
    case generationOverflow
    case hostNotInitialized
    case keychainFailed(OSStatus)

    public var errorDescription: String? {
        switch self {
        case .staleGeneration(let taskGen, let currentGen):
            return "Task generation \(taskGen) was invalidated by generation advance to \(currentGen)"
        case .accountNotFound(let id):
            return "Account \(id) not found in Rust session store"
        case .generationOverflow:
            return "Account generation counter reached UInt64 maximum"
        case .hostNotInitialized:
            return "Fabushi Rust host is not initialized"
        case .keychainFailed(let status):
            return "iOS Keychain operation failed (OSStatus: \(status))"
        }
    }
}

public actor AccountScopeManager {
    public static let shared = AccountScopeManager(host: .shared, keychain: .shared)

    private let host: FabushiRustHostActor
    private let keychain: FabushiKeychainStore

    public private(set) var currentGeneration: UInt64 = 1
    public private(set) var activeAccountId: String? = nil

    public init(
        host: FabushiRustHostActor = .shared,
        keychain: FabushiKeychainStore = .shared
    ) {
        self.host = host
        self.keychain = keychain
    }

    /// Synchronizes local cached generation and active account ID from authoritative Rust host state.
    @discardableResult
    public func syncFromHost() async throws -> UInt64 {
        let gen = try await fetchHostGeneration()
        currentGeneration = gen
        let accounts = try await host.listAccounts(generation: gen)
        activeAccountId = accounts.activeAccountId
        return gen
    }

    public func getCurrentGeneration() async throws -> UInt64 {
        let gen = try await fetchHostGeneration()
        currentGeneration = gen
        return gen
    }

    public func getActiveAccountId() async -> String? {
        if let list = try? await host.listAccounts() {
            activeAccountId = list.activeAccountId
        }
        return activeAccountId
    }

    /// Installs an authenticated account session into Rust (`session.setAuthSession`),
    /// then provisions a real 32-byte random Keychain key for that account and updates generation.
    @discardableResult
    public func registerAccountSession(_ params: FabushiSessionAuthParams) async throws -> UInt64 {
        guard params.userId > 0 else {
            throw FabushiHostError.invalidArguments(message: "user_id must be greater than 0")
        }

        let accountId = String(params.userId)
        let result: FabushiSessionAuthResult
        do {
            let gen = try await fetchHostGeneration()
            result = try await host.setAuthSession(params, generation: gen)
        } catch let hostError as FabushiHostError {
            throw mapHostError(hostError, accountId: accountId)
        }

        let resolvedAccountId = String(result.userId)
        currentGeneration = result.generation
        activeAccountId = resolvedAccountId

        do {
            _ = try keychain.loadOrCreateAccountKey(accountId: resolvedAccountId)
        } catch let kcError as FabushiKeychainError {
            if case .osStatus(let status) = kcError {
                throw FenceRejectionError.keychainFailed(status)
            }
            throw kcError
        }

        return result.generation
    }

    /// Switches active account via authoritative Rust `session.switchAccount` and advances generation fence.
    @discardableResult
    public func switchAccount(to newAccountId: String) async throws -> UInt64 {
        let currentGen = try await fetchHostGeneration()
        guard currentGen < UInt64.max else {
            throw FenceRejectionError.generationOverflow
        }

        do {
            let result = try await host.switchAccount(accountId: newAccountId, generation: currentGen)
            _ = try keychain.loadOrCreateAccountKey(accountId: result.accountId)
            currentGeneration = result.generation
            activeAccountId = result.accountId
            return result.generation
        } catch let hostError as FabushiHostError {
            throw mapHostError(hostError, accountId: newAccountId)
        } catch let kcError as FabushiKeychainError {
            if case .osStatus(let status) = kcError {
                throw FenceRejectionError.keychainFailed(status)
            }
            throw kcError
        }
    }

    /// Advances Rust generation monotonically via `generation.bump`.
    @discardableResult
    public func bumpGeneration() async throws -> UInt64 {
        let next = try await host.bumpGeneration()
        currentGeneration = next
        return next
    }

    /// Sets Rust generation monotonically via `generation.set` (rejects regressions fail-closed).
    @discardableResult
    public func setGeneration(_ generation: UInt64) async throws -> UInt64 {
        let applied = try await host.setGeneration(generation)
        currentGeneration = applied
        return applied
    }

    /// Clears active Rust session (`session.clearSession`), increments epoch, and scrubs real Keychain account key.
    @discardableResult
    public func logout() async throws -> UInt64 {
        let currentGen = try await fetchHostGeneration()
        let listed = try? await host.listAccounts(generation: currentGen)
        let targetAccount = listed?.activeAccountId ?? activeAccountId

        let result: FabushiClearSessionResult
        do {
            result = try await host.clearSession(generation: currentGen)
        } catch let hostError as FabushiHostError {
            throw mapHostError(hostError, accountId: targetAccount ?? "")
        }

        activeAccountId = nil
        currentGeneration = result.generation

        if let targetAccount, !targetAccount.isEmpty {
            do {
                try keychain.deleteAccountKey(accountId: targetAccount)
            } catch let kcError as FabushiKeychainError {
                if case .osStatus(let status) = kcError {
                    throw FenceRejectionError.keychainFailed(status)
                }
                throw kcError
            }
        }

        return result.generation
    }

    /// Validates a captured generation against authoritative Rust `generation.validate`.
    public func validateGeneration(_ taskGen: UInt64) async throws -> Bool {
        let validation = try await host.validateGeneration(taskGen)
        currentGeneration = validation.current
        return validation.valid && taskGen == validation.current
    }

    /// Executes an async task fenced by authoritative Rust generation. If generation advanced
    /// while the task was running, drops the result and throws `FenceRejectionError.staleGeneration`.
    public func executeFenced<T: Sendable>(
        _ task: @Sendable (UInt64) async throws -> T
    ) async throws -> T {
        let capturedGen = try await fetchHostGeneration()
        currentGeneration = capturedGen

        let result = try await task(capturedGen)

        let postGen = try await fetchHostGeneration()
        currentGeneration = postGen
        guard postGen == capturedGen else {
            throw FenceRejectionError.staleGeneration(taskGen: capturedGen, currentGen: postGen)
        }
        return result
    }

    /// Checks whether a real 32-byte Keychain key currently exists for `accountId`.
    public func hasActiveKey(for accountId: String) throws -> Bool {
        do {
            return try keychain.hasAccountKey(accountId: accountId)
        } catch let kcError as FabushiKeychainError {
            if case .osStatus(let status) = kcError {
                throw FenceRejectionError.keychainFailed(status)
            }
            throw kcError
        }
    }

    // MARK: - Internal Helpers

    private func fetchHostGeneration() async throws -> UInt64 {
        do {
            return try await host.currentGeneration()
        } catch FabushiHostError.notInitialized {
            throw FenceRejectionError.hostNotInitialized
        }
    }

    private func mapHostError(_ error: FabushiHostError, accountId: String) -> Error {
        switch error {
        case .notFound:
            return FenceRejectionError.accountNotFound(accountId)
        case .staleGeneration(let requested, let current, _):
            return FenceRejectionError.staleGeneration(
                taskGen: requested ?? currentGeneration,
                currentGen: current
            )
        case .notInitialized:
            return FenceRejectionError.hostNotInitialized
        default:
            return error
        }
    }
}
