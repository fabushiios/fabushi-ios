import Foundation
import LocalAuthentication
import Observation
import SwiftUI

// MARK: - Biometrics & Passcode Policy Models

public enum FabushiBiometryKind: String, Sendable, Equatable {
    case none
    case touchID
    case faceID
    case opticID

    public var systemImage: String {
        switch self {
        case .faceID: return "faceid"
        case .touchID: return "touchid"
        case .opticID: return "opticid"
        case .none: return "lock.shield"
        }
    }

    public var localizationKey: String {
        switch self {
        case .faceID: return "hig.passcode.biometryFaceID"
        case .touchID: return "hig.passcode.biometryTouchID"
        case .opticID: return "hig.passcode.biometryOpticID"
        case .none: return "hig.passcode.biometryUnavailable"
        }
    }
}

public enum FabushiBiometricAuthError: LocalizedError, Sendable, Equatable {
    case unavailable(String)
    case cancelled
    case failed(String)

    public var errorDescription: String? {
        switch self {
        case .unavailable(let message): return message
        case .cancelled: return "Biometric authentication was cancelled."
        case .failed(let message): return message
        }
    }
}

public enum FabushiPasscodePolicyError: LocalizedError, Sendable, Equatable {
    case tooShort
    case confirmationMismatch
    case reauthenticationRequired
    case currentPasscodeIncorrect(failedAttempts: Int)
    case lockedOut(remainingSeconds: Int, failedAttempts: Int)
    case biometricVerificationFailed(String)

    public var errorDescription: String? {
        switch self {
        case .tooShort:
            return "Passcode must be at least 4 characters."
        case .confirmationMismatch:
            return "Passcode entries do not match."
        case .reauthenticationRequired:
            return "Current passcode or biometric verification is required."
        case .currentPasscodeIncorrect(let attempts):
            return "Current passcode is incorrect (\(attempts))."
        case .lockedOut(let remaining, let attempts):
            return "Too many failed attempts (\(attempts)). Try again in \(remaining)s."
        case .biometricVerificationFailed(let reason):
            return reason
        }
    }
}

public enum FabushiPasscodeReauthAuthorization: Sendable, Equatable {
    case currentPasscode(String)
    case biometrics(localizedReason: String)
}

public enum FabushiPasscodeVerifyResult: Sendable, Equatable {
    case unlocked
    case incorrect(failedAttempts: Int)
    case lockedOut(remainingSeconds: Int, failedAttempts: Int)
}

public enum FabushiAutoLockPreset: Int, CaseIterable, Identifiable, Sendable {
    case disabled = 0
    case oneMinute = 60
    case fiveMinutes = 300
    case oneHour = 3_600
    case fiveHours = 18_000

    public var id: Int { rawValue }

    public var localizationKey: String {
        switch self {
        case .disabled: return "hig.passcode.autoLockOff"
        case .oneMinute: return "hig.passcode.autoLock1Min"
        case .fiveMinutes: return "hig.passcode.autoLock5Min"
        case .oneHour: return "hig.passcode.autoLock1Hour"
        case .fiveHours: return "hig.passcode.autoLock5Hours"
        }
    }
}

// MARK: - Keychain + LocalAuthentication + Rust Argon2id Passcode Lock Controller (F06)

@MainActor
@Observable
public final class FabushiPasscodeLockController {
    private static var _sharedInstance: FabushiPasscodeLockController?

    /// Computed singleton avoids Swift 6.1.2 `emitLazyGlobalInitializer` crash on `@MainActor @Observable`.
    public static var shared: FabushiPasscodeLockController {
        if let existing = _sharedInstance {
            return existing
        }
        let instance = FabushiPasscodeLockController()
        _sharedInstance = instance
        return instance
    }

    nonisolated public static let minimumPasscodeLength: Int = 4
    nonisolated public static let autoLockPresets: [Int] = [0, 60, 300, 3_600, 18_000]

    @ObservationIgnored private let keychainStore: FabushiSecurityKeychainStore
    @ObservationIgnored private let useFastIterationsForTests: Bool
    @ObservationIgnored private let nowProvider: @Sendable () -> Date
    @ObservationIgnored private let taskBag = FabushiCancellableTaskBag()

    public private(set) var isPasscodeEnabled: Bool = false
    public private(set) var hasUnreadableKeychainRecord: Bool = false
    public private(set) var biometricsEnabled: Bool = false
    public private(set) var autoLockTimeoutSeconds: Int = FabushiAutoLockPreset.fiveMinutes.rawValue
    public private(set) var failedAttempts: Int = 0
    public private(set) var lockoutUntilEpochMs: Int64? = nil
    public private(set) var lastActivityDate: Date
    public private(set) var backgroundedAtDate: Date? = nil

    private struct RustPasscodeSetParams: Encodable, Sendable {
        let passcode: String
    }

    private struct RustPasscodeVerifyParams: Encodable, Sendable {
        let passcode: String
    }

    private struct RustPasscodeVerifyResult: Decodable, Sendable {
        let valid: Bool
        let configured: Bool
    }

    private struct RustPasscodeStatusResult: Decodable, Sendable {
        let configured: Bool
    }

    public init(
        keychainStore: FabushiSecurityKeychainStore = .shared,
        useFastIterationsForTests: Bool = false,
        nowProvider: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.keychainStore = keychainStore
        self.useFastIterationsForTests = useFastIterationsForTests
        self.nowProvider = nowProvider
        self.lastActivityDate = nowProvider()
        reloadFromKeychain()
    }

    deinit {
        taskBag.cancelAll()
    }

    public func reloadFromKeychain() {
        do {
            if let record = try keychainStore.loadPasscodeRecord() {
                hasUnreadableKeychainRecord = false
                isPasscodeEnabled = record.isEnabled
                biometricsEnabled = record.biometricsEnabled
                autoLockTimeoutSeconds = record.autoLockTimeoutSeconds
                failedAttempts = record.failedAttempts
                lockoutUntilEpochMs = record.lockoutUntilEpochMs
            } else {
                hasUnreadableKeychainRecord = false
                isPasscodeEnabled = false
                biometricsEnabled = false
                autoLockTimeoutSeconds = FabushiAutoLockPreset.fiveMinutes.rawValue
                failedAttempts = 0
                lockoutUntilEpochMs = nil
            }
        } catch {
            // Fail closed: a corrupted or unreadable Keychain record MUST NEVER downgrade
            // passcode lock posture to disabled on cold launch or reload.
            hasUnreadableKeychainRecord = true
            isPasscodeEnabled = true
        }
    }

    /// Reloads local Keychain passcode state and synchronizes with Rust `settings.passcodeStatus`.
    /// Fail-closed: if Keychain read fails (corrupted/unreadable record), preserves
    /// `isPasscodeEnabled = true` and `hasUnreadableKeychainRecord = true` without downgrading
    /// lock posture. If Keychain has no enabled record while Rust `host_meta` reports a configured
    /// passcode, sets `isPasscodeEnabled = true` so cold launch engages lock immediately and
    /// Settings displays the "Current Passcode" re-authentication field.
    @discardableResult
    public func syncPasscodeStatus(
        coordinator: FabushiCoordinator?,
        router: ShellNavigationRouter? = nil,
        requirePasscode: Bool = false
    ) async -> Bool {
        let wasRouterEnabled = router?.isPasscodeLockEnabled ?? false
        reloadFromKeychain()

        if !hasUnreadableKeychainRecord,
           !isPasscodeEnabled,
           let coordinator,
           coordinator.isStarted {
            do {
                let rustStatus: RustPasscodeStatusResult = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.settings.method("passcodeStatus"),
                    params: FabushiEmptyParams()
                )
                if rustStatus.configured {
                    isPasscodeEnabled = true
                }
            } catch {
                // Non-fatal during passive status sync; mutating operations (`setPasscode` /
                // `clearPasscode`) still propagate errors fail-closed via `hasConfiguredPasscode`.
            }
        }

        let shouldEnablePasscode = requirePasscode || isPasscodeEnabled
        if let router {
            router.isPasscodeLockEnabled = shouldEnablePasscode
            if shouldEnablePasscode && !wasRouterEnabled {
                router.engagePasscodeLockIfEnabled()
            }
        }
        return shouldEnablePasscode
    }

    nonisolated public static func lockoutDurationSeconds(forFailedAttempts attempts: Int) -> Int {
        switch attempts {
        case ..<3: return 0
        case 3: return 5
        case 4: return 15
        case 5: return 30
        default: return 60
        }
    }

    public var lockoutRemainingSeconds: Int {
        guard let untilMs = lockoutUntilEpochMs else { return 0 }
        let nowMs = currentEpochMs()
        guard untilMs > nowMs else { return 0 }
        return max(1, Int((untilMs - nowMs + 999) / 1_000))
    }

    // MARK: - Configure / Clear Passcode & Preferences

    /// Fail-closed check for whether a passcode is configured in Keychain, Rust `host_meta`, or memory.
    /// Always reads Keychain first so corrupted/unreadable Keychain records immediately propagate
    /// `FabushiSecurityKeychainError` instead of failing open or masking corruption.
    private func hasConfiguredPasscode(coordinator: FabushiCoordinator?) async throws -> Bool {
        let record = try keychainStore.loadPasscodeRecord()
        if let record, record.isEnabled {
            reloadFromKeychain()
            return true
        }
        if let coordinator, coordinator.isStarted {
            let rustStatus: RustPasscodeStatusResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeStatus"),
                params: FabushiEmptyParams()
            )
            if rustStatus.configured {
                isPasscodeEnabled = true
                return true
            }
        }
        if isPasscodeEnabled {
            return true
        }
        return false
    }

    private func enforceReauthentication(
        _ authorization: FabushiPasscodeReauthAuthorization?,
        coordinator: FabushiCoordinator?
    ) async throws {
        guard let authorization else {
            throw FabushiPasscodePolicyError.reauthenticationRequired
        }
        switch authorization {
        case .currentPasscode(let current):
            let result = await verifyPasscode(current, coordinator: coordinator)
            switch result {
            case .unlocked:
                return
            case .incorrect(let attempts):
                throw FabushiPasscodePolicyError.currentPasscodeIncorrect(failedAttempts: attempts)
            case .lockedOut(let remaining, let attempts):
                throw FabushiPasscodePolicyError.lockedOut(
                    remainingSeconds: remaining,
                    failedAttempts: attempts
                )
            }
        case .biometrics(let reason):
            guard biometricsEnabled else {
                throw FabushiPasscodePolicyError.biometricVerificationFailed(
                    "Biometric authentication is not enabled for passcode."
                )
            }
            let bioResult = await authenticateWithBiometrics(localizedReason: reason)
            switch bioResult {
            case .success:
                return
            case .failure(let error):
                throw FabushiPasscodePolicyError.biometricVerificationFailed(error.localizedDescription)
            }
        }
    }

    public func setPasscode(
        _ passcode: String,
        currentAuthorization: FabushiPasscodeReauthAuthorization? = nil,
        coordinator: FabushiCoordinator?
    ) async throws {
        let alreadyConfigured = try await hasConfiguredPasscode(coordinator: coordinator)
        if alreadyConfigured {
            guard currentAuthorization != nil else {
                throw FabushiPasscodePolicyError.reauthenticationRequired
            }
        }
        guard passcode.count >= Self.minimumPasscodeLength else {
            throw FabushiPasscodePolicyError.tooShort
        }
        if alreadyConfigured {
            try await enforceReauthentication(currentAuthorization, coordinator: coordinator)
        }

        let salt = try FabushiPasscodeCrypto.generateRandomBytes(count: FabushiPasscodeCrypto.saltByteLength)
        let iterations = useFastIterationsForTests
            ? FabushiPasscodeCrypto.testIterations
            : FabushiPasscodeCrypto.productionIterations
        let derivedHash = FabushiPasscodeCrypto.derivePBKDF2SHA256(
            passcode: passcode,
            salt: salt,
            iterations: iterations
        )

        let record = FabushiPasscodeRecord(
            isEnabled: true,
            saltB64: salt.base64EncodedString(),
            hashB64: derivedHash.base64EncodedString(),
            iterations: iterations,
            biometricsEnabled: biometricsEnabled,
            autoLockTimeoutSeconds: autoLockTimeoutSeconds,
            failedAttempts: 0,
            lockoutUntilEpochMs: nil
        )
        try keychainStore.savePasscodeRecord(record)

        hasUnreadableKeychainRecord = false
        isPasscodeEnabled = true
        failedAttempts = 0
        lockoutUntilEpochMs = nil
        backgroundedAtDate = nil
        lastActivityDate = nowProvider()

        if let coordinator, coordinator.isStarted {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeSet"),
                params: RustPasscodeSetParams(passcode: passcode)
            )
        }
    }

    public func clearPasscode(
        authorization: FabushiPasscodeReauthAuthorization? = nil,
        coordinator: FabushiCoordinator?
    ) async throws {
        if try await hasConfiguredPasscode(coordinator: coordinator) {
            try await enforceReauthentication(authorization, coordinator: coordinator)
        }
        try keychainStore.deletePasscodeRecord()
        hasUnreadableKeychainRecord = false
        isPasscodeEnabled = false
        biometricsEnabled = false
        failedAttempts = 0
        lockoutUntilEpochMs = nil
        backgroundedAtDate = nil

        if let coordinator, coordinator.isStarted {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeClear"),
                params: FabushiEmptyParams()
            )
        }
    }

    public func setBiometricsEnabled(_ enabled: Bool) throws {
        guard var record = try keychainStore.loadPasscodeRecord() else {
            biometricsEnabled = enabled
            return
        }
        record.biometricsEnabled = enabled
        try keychainStore.savePasscodeRecord(record)
        biometricsEnabled = enabled
    }

    public func setAutoLockTimeoutSeconds(_ seconds: Int) throws {
        let clamped = max(0, seconds)
        guard var record = try keychainStore.loadPasscodeRecord() else {
            autoLockTimeoutSeconds = clamped
            return
        }
        record.autoLockTimeoutSeconds = clamped
        try keychainStore.savePasscodeRecord(record)
        autoLockTimeoutSeconds = clamped
    }

    // MARK: - Fail-Closed Verification & Progressive Lockout

    public func verifyPasscode(
        _ passcode: String,
        coordinator: FabushiCoordinator?
    ) async -> FabushiPasscodeVerifyResult {
        let nowMs = currentEpochMs()
        if let untilMs = lockoutUntilEpochMs, untilMs > nowMs {
            let remaining = max(1, Int((untilMs - nowMs + 999) / 1_000))
            return .lockedOut(remainingSeconds: remaining, failedAttempts: failedAttempts)
        }

        let storedRecord: FabushiPasscodeRecord?
        let keychainReadFailed: Bool
        do {
            storedRecord = try keychainStore.loadPasscodeRecord()
            keychainReadFailed = false
        } catch {
            storedRecord = nil
            keychainReadFailed = true
            hasUnreadableKeychainRecord = true
            isPasscodeEnabled = true
        }
        var isValid = false

        if let record = storedRecord,
           record.isEnabled,
           let salt = Data(base64Encoded: record.saltB64),
           let expectedHash = Data(base64Encoded: record.hashB64),
           !salt.isEmpty,
           !expectedHash.isEmpty {
            let computed = FabushiPasscodeCrypto.derivePBKDF2SHA256(
                passcode: passcode,
                salt: salt,
                iterations: record.iterations
            )
            isValid = FabushiPasscodeCrypto.constantTimeEqual(computed, expectedHash)
        } else if !keychainReadFailed, let coordinator, coordinator.isStarted {
            // Fallback to Rust Argon2id verification only when Keychain read succeeded without an
            // enabled local PBKDF2 record (e.g. Rust-only host_meta configuration);
            // fails closed (`configured == false` -> `valid == false`) when unconfigured (`T2_11`).
            if let rustResult: RustPasscodeVerifyResult = try? await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeVerify"),
                params: RustPasscodeVerifyParams(passcode: passcode)
            ) {
                isValid = rustResult.configured && rustResult.valid
            }
        }

        if isValid {
            failedAttempts = 0
            lockoutUntilEpochMs = nil
            lastActivityDate = nowProvider()
            if !keychainReadFailed, var updated = storedRecord {
                updated.failedAttempts = 0
                updated.lockoutUntilEpochMs = nil
                try? keychainStore.savePasscodeRecord(updated)
            }
            return .unlocked
        }

        // Fail-closed increment & progressive backoff persistence
        let nextAttempts = failedAttempts + 1
        failedAttempts = nextAttempts
        let backoffSeconds = Self.lockoutDurationSeconds(forFailedAttempts: nextAttempts)
        let nextLockoutMs: Int64? = backoffSeconds > 0
            ? (nowMs + Int64(backoffSeconds) * 1_000)
            : nil
        lockoutUntilEpochMs = nextLockoutMs

        // CRITICAL fail-closed invariant (R2-FIX-2): NEVER overwrite an unreadable/corrupted
        // Keychain passcode item with a synthetic `isEnabled: false` lockout record.
        if !keychainReadFailed {
            if var updated = storedRecord {
                updated.failedAttempts = nextAttempts
                updated.lockoutUntilEpochMs = nextLockoutMs
                try? keychainStore.savePasscodeRecord(updated)
            } else {
                let lockoutRecord = FabushiPasscodeRecord(
                    isEnabled: false,
                    saltB64: "",
                    hashB64: "",
                    iterations: 0,
                    biometricsEnabled: false,
                    autoLockTimeoutSeconds: autoLockTimeoutSeconds,
                    failedAttempts: nextAttempts,
                    lockoutUntilEpochMs: nextLockoutMs
                )
                try? keychainStore.savePasscodeRecord(lockoutRecord)
            }
        }

        if backoffSeconds > 0 {
            return .lockedOut(remainingSeconds: backoffSeconds, failedAttempts: nextAttempts)
        }
        return .incorrect(failedAttempts: nextAttempts)
    }

    // MARK: - LocalAuthentication Biometrics (Zero Fake Success)

    nonisolated public static func queryBiometryAvailability() -> (
        available: Bool,
        kind: FabushiBiometryKind,
        errorMessage: String?
    ) {
        let context = LAContext()
        var error: NSError?
        let canEvaluate = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
        let kind: FabushiBiometryKind
        switch context.biometryType {
        case .faceID: kind = .faceID
        case .touchID: kind = .touchID
        case .opticID: kind = .opticID
        default: kind = .none
        }
        return (canEvaluate, kind, error?.localizedDescription)
    }

    nonisolated public static func evaluateBiometrics(
        localizedReason: String
    ) async -> Result<Void, FabushiBiometricAuthError> {
        let context = LAContext()
        var canEvalError: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &canEvalError) else {
            return .failure(
                .unavailable(canEvalError?.localizedDescription ?? "Biometric authentication is unavailable.")
            )
        }

        return await withCheckedContinuation { continuation in
            context.evaluatePolicy(
                .deviceOwnerAuthenticationWithBiometrics,
                localizedReason: localizedReason
            ) { success, evalError in
                if success {
                    continuation.resume(returning: .success(()))
                    return
                }
                if let laError = evalError as? LAError,
                   laError.code == .userCancel || laError.code == .systemCancel || laError.code == .appCancel {
                    continuation.resume(returning: .failure(.cancelled))
                    return
                }
                let message = evalError?.localizedDescription ?? "Biometric authentication failed."
                continuation.resume(returning: .failure(.failed(message)))
            }
        }
    }

    public func canEvaluateBiometrics() -> (
        available: Bool,
        kind: FabushiBiometryKind,
        errorMessage: String?
    ) {
        Self.queryBiometryAvailability()
    }

    public func authenticateWithBiometrics(
        localizedReason: String
    ) async -> Result<Void, FabushiBiometricAuthError> {
        let result = await Self.evaluateBiometrics(localizedReason: localizedReason)
        if case .success = result {
            failedAttempts = 0
            lockoutUntilEpochMs = nil
            lastActivityDate = nowProvider()
            do {
                if var updated = try keychainStore.loadPasscodeRecord() {
                    updated.failedAttempts = 0
                    updated.lockoutUntilEpochMs = nil
                    try? keychainStore.savePasscodeRecord(updated)
                }
            } catch {
                // Preserve unreadable/corrupted Keychain passcode record untouched.
            }
        }
        return result
    }

    // MARK: - Idle Auto-Lock Lifecycle

    public func recordUserActivity() {
        lastActivityDate = nowProvider()
    }

    @discardableResult
    public func recordUserActivityIfNeeded(minimumInterval: TimeInterval = 1.0) -> Bool {
        let now = nowProvider()
        let elapsed = now.timeIntervalSince(lastActivityDate)
        guard elapsed < 0 || elapsed >= max(0, minimumInterval) else {
            return false
        }
        lastActivityDate = now
        return true
    }

    public func shouldEngageIdleAutoLock() -> Bool {
        guard isPasscodeEnabled, autoLockTimeoutSeconds > 0 else { return false }
        let elapsed = nowProvider().timeIntervalSince(lastActivityDate)
        return elapsed < 0 || elapsed >= TimeInterval(autoLockTimeoutSeconds)
    }

    @discardableResult
    public func evaluateIdleAutoLock(router: ShellNavigationRouter) -> Bool {
        guard shouldEngageIdleAutoLock() else { return false }
        router.isPasscodeLockEnabled = true
        router.engagePasscodeLockIfEnabled()
        return true
    }

    public func handleScenePhaseChange(_ phase: ScenePhase, router: ShellNavigationRouter) {
        switch phase {
        case .background:
            if backgroundedAtDate == nil {
                backgroundedAtDate = nowProvider()
            }
            // Do NOT unconditionally engage passcode lock here.
            // Privacy blur shields the snapshot immediately via top-level overlay UIWindow,
            // while passcode locking strictly respects autoLockTimeoutSeconds (0 == Disabled / Never auto-lock).
        case .inactive:
            break
        case .active:
            if let backgrounded = backgroundedAtDate {
                backgroundedAtDate = nil
                let now = nowProvider()
                let backgroundElapsed = now.timeIntervalSince(backgrounded)
                let totalIdleElapsed = now.timeIntervalSince(lastActivityDate)
                let timeout = TimeInterval(autoLockTimeoutSeconds)
                if isPasscodeEnabled,
                   autoLockTimeoutSeconds > 0,
                   (backgroundElapsed < 0
                       || totalIdleElapsed < 0
                       || backgroundElapsed >= timeout
                       || totalIdleElapsed >= timeout) {
                    router.isPasscodeLockEnabled = true
                    router.engagePasscodeLockIfEnabled()
                } else if !router.isPasscodeLocked {
                    // Returning from a brief app switch (e.g. Alipay OAuth / SMS / 2FA code)
                    // within the remaining autoLockTimeoutSeconds budget (or when auto-lock is disabled)
                    // refreshes activity without locking out the user. Already-locked sessions stay locked.
                    recordUserActivity()
                }
            } else {
                _ = evaluateIdleAutoLock(router: router)
            }
        @unknown default:
            break
        }
    }

    public func startIdleMonitorIfNeeded(
        router: ShellNavigationRouter,
        pollIntervalNanoseconds: UInt64 = 5_000_000_000
    ) {
        let interval = max(100_000_000, pollIntervalNanoseconds)
        let task = Task<Void, Never> { @MainActor [weak self, weak router] in
            while let self, let router, !Task.isCancelled {
                try? await Task.sleep(nanoseconds: interval)
                guard !Task.isCancelled else { return }
                _ = self.evaluateIdleAutoLock(router: router)
            }
        }
        taskBag.replace(key: "idleMonitor", with: task)
    }

    public func stopIdleMonitor() {
        taskBag.cancel(key: "idleMonitor")
    }

    private func currentEpochMs() -> Int64 {
        Int64(nowProvider().timeIntervalSince1970 * 1_000)
    }
}
