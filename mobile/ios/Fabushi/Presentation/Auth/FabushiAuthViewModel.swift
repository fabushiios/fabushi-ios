import Foundation
import Observation
import os
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Swift 6 Strictly-Typed Sendable Cooperative Task Bag

public final class FabushiCancellableTaskBag: Sendable {
    private let lock = OSAllocatedUnfairLock<[String: Task<Void, Never>]>(initialState: [:])

    public init() {}

    public func replace(key: String, with task: Task<Void, Never>) {
        let previous = lock.withLock { map -> Task<Void, Never>? in
            let old = map[key]
            map[key] = task
            return old
        }
        previous?.cancel()
    }

    public func cancel(key: String) {
        let task = lock.withLock { $0.removeValue(forKey: key) }
        task?.cancel()
    }

    public func cancelAll() {
        let tasks = lock.withLock { map -> [Task<Void, Never>] in
            let values = Array(map.values)
            map.removeAll()
            return values
        }
        for task in tasks {
            task.cancel()
        }
    }
}

// MARK: - Auth Wizard Modes & Channels

public enum FabushiAuthPresentationMode: Sendable {
    case primaryGate
    case addAccount
}

public enum FabushiAuthTabMode: String, CaseIterable, Sendable, Identifiable {
    case signIn
    case register

    public var id: String { rawValue }
}

public enum FabushiOTPChannel: String, CaseIterable, Sendable, Identifiable {
    case phone
    case email

    public var id: String { rawValue }
}

// MARK: - Unified Authentication ViewModel (F01–F05)

@MainActor
@Observable
public final class FabushiAuthViewModel {
    public let mode: FabushiAuthPresentationMode
    public let useFastKdfForTests: Bool
    public let pollIntervalNanoseconds: UInt64

    @ObservationIgnored private let coordinator: FabushiCoordinator
    @ObservationIgnored private let router: ShellNavigationRouter
    @ObservationIgnored private let launchConfiguration: FabushiLaunchConfiguration
    @ObservationIgnored private let securityKeychain: FabushiSecurityKeychainStore
    @ObservationIgnored private let passkeyController: FabushiWebAuthnPasskeyController
    @ObservationIgnored private let alipayWebController: FabushiAlipayWebAuthController
    @ObservationIgnored private let onAuthenticatedCallback: (() -> Void)?
    @ObservationIgnored private let taskBag = FabushiCancellableTaskBag()

    // Core F01 Form State
    public var serverURLText: String
    public var serverURLError: String?
    public var authTab: FabushiAuthTabMode = .signIn
    public var usernameText: String = ""
    public var passwordText: String = ""
    public var firstNameText: String = ""

    // F02 OTP State
    public var otpChannel: FabushiOTPChannel = .phone
    public var selectedDialCode: FabushiCountryDialCode = FabushiCountryDialCode.commonCodes[0]
    public var phoneText: String = ""
    public var emailText: String = ""
    public var otpCodeText: String = ""
    public var phoneCodeHash: String?
    public var otpCodeSent: Bool = false
    public var otpResendRemainingSeconds: Int = 0
    public var otpStatusMessage: String?

    // F03 Alipay QR & OAuth State
    public var alipayAttemptId: String?
    public var alipayLoginURL: String?
    public var alipayExpectedState: String?
    public var alipayExpiresAtMs: Int64?
    public var alipayStatusText: String?
    public var isAlipayActive: Bool = false
    #if canImport(UIKit)
    public var alipayQRImage: UIImage?
    #endif

    // F04 Cross-Device QR Pairing State
    public var qrPairToken: String?
    public var qrPairURL: String?
    public var qrPairExpiresAtMs: Int64?
    public var qrPairStatusText: String?
    public var isQrPairActive: Bool = false
    #if canImport(UIKit)
    public var qrPairImage: UIImage?
    #endif

    // F05 Cloud 2FA Gate State
    public var activeTwoFactorChallenge: FabushiTwoFactorChallenge?
    public var twoFactorPasswordText: String = ""

    // Global Feedback & One-Time Emergency Kit
    public var isSubmitting: Bool = false
    public var errorMessage: String?
    public var generatedEmergencyKit: String?
    public var lastCompletedVaultBootstrap: Bool = false

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        launchConfiguration: FabushiLaunchConfiguration = .current,
        mode: FabushiAuthPresentationMode = .primaryGate,
        securityKeychain: FabushiSecurityKeychainStore = .shared,
        useFastKdfForTests: Bool = false,
        pollIntervalNanoseconds: UInt64 = 2_000_000_000,
        onAuthenticated: (() -> Void)? = nil
    ) {
        self.coordinator = coordinator
        self.router = router
        self.launchConfiguration = launchConfiguration
        self.mode = mode
        self.securityKeychain = securityKeychain
        self.passkeyController = FabushiWebAuthnPasskeyController()
        self.alipayWebController = FabushiAlipayWebAuthController()
        self.useFastKdfForTests = useFastKdfForTests
        self.pollIntervalNanoseconds = max(50_000_000, pollIntervalNanoseconds)
        self.onAuthenticatedCallback = onAuthenticated
        self.serverURLText = launchConfiguration.serverURL ?? coordinator.settingsStore.activeServerURL
    }

    deinit {
        taskBag.cancelAll()
    }

    public func cancelActiveTasks() {
        taskBag.cancelAll()
        alipayWebController.cancel()
    }

    public func clearErrors() {
        serverURLError = nil
        errorMessage = nil
    }

    // MARK: - Server URL Validation & Synchronization

    public func applyServerURLIfNeeded(theme: FabushiTheme) async -> Bool {
        serverURLError = nil
        guard let normalized = FabushiAuthValidation.validateServerURL(serverURLText) else {
            serverURLError = theme.localized("hig.auth.invalidServerURL")
            return false
        }
        serverURLText = normalized

        guard normalized != coordinator.settingsStore.activeServerURL else {
            return true
        }
        do {
            _ = try await coordinator.setServerURL(normalized)
            return true
        } catch {
            serverURLError = error.localizedDescription
            return false
        }
    }

    // MARK: - F01: Password Login (Split-Key V2 + Legacy V1 Silent Upgrade)

    public func submitLogin(theme: FabushiTheme) async {
        clearErrors()
        guard await applyServerURLIfNeeded(theme: theme) else { return }

        let trimmedUsername = usernameText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedUsername.isEmpty, !passwordText.isEmpty else {
            errorMessage = theme.localized("hig.auth.missingCredentials")
            return
        }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            let kdfParamsResponse = try await fetchKdfParamsOrFallback(username: trimmedUsername)

            if let kdf = kdfParamsResponse, kdf.authVersion >= 2 {
                let derived: FabushiDeriveSplitKeyResponse = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("deriveSplitKey"),
                    params: FabushiDeriveSplitKeyRequest(
                        password: passwordText,
                        saltB64: kdf.authSaltB64,
                        kdfParams: kdf.kdfParams,
                        fastKdf: nil
                    )
                )

                let envelope: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                    method: FabushiRPCNamespace.auth.method("loginSplitKey"),
                    params: FabushiSplitKeyLoginRequest(
                        username: trimmedUsername,
                        deviceName: "Fabushi iOS",
                        authVerifierB64: derived.authVerifierB64
                    ),
                    extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
                )

                if let identity = envelope.result.identityTuple {
                    await bootstrapVaultAfterSplitKeyLogin(
                        userId: identity.userId,
                        password: passwordText,
                        vaultKekB64: derived.vaultKekB64,
                        saltB64: derived.authSaltB64,
                        kdfParams: derived.kdfParams
                    )
                }
                completeAuthentication()
            } else {
                let envelope: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                    method: FabushiRPCNamespace.auth.method("login"),
                    params: FabushiLegacyLoginRequest(
                        username: trimmedUsername,
                        password: passwordText,
                        deviceName: "Fabushi iOS"
                    ),
                    extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
                )

                if let identity = envelope.result.identityTuple {
                    await silentlyUpgradeLegacyV1ToSplitKeyV2(
                        userId: identity.userId,
                        oldPassword: passwordText
                    )
                }
                completeAuthentication()
            }
        } catch {
            handleAuthError(error, isQrPair: false)
        }
    }

    // MARK: - F01: Split-Key V2 Registration (Zero Plaintext Password on Wire)

    public func submitRegistration(theme: FabushiTheme) async {
        clearErrors()
        guard await applyServerURLIfNeeded(theme: theme) else { return }

        let trimmedUsername = usernameText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedUsername.isEmpty, !passwordText.isEmpty else {
            errorMessage = theme.localized("hig.auth.missingCredentials")
            return
        }

        let resolvedFirstName = firstNameText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? trimmedUsername
            : firstNameText.trimmingCharacters(in: .whitespacesAndNewlines)

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            let derived: FabushiDeriveSplitKeyResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("deriveSplitKey"),
                params: FabushiDeriveSplitKeyRequest(
                    password: passwordText,
                    saltB64: nil,
                    kdfParams: nil,
                    fastKdf: useFastKdfForTests ? true : nil
                )
            )

            let trimmedEmail = emailText.trimmingCharacters(in: .whitespacesAndNewlines)
            let trimmedOtp = otpCodeText.trimmingCharacters(in: .whitespacesAndNewlines)

            let envelope: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse>
            if !trimmedEmail.isEmpty && !trimmedOtp.isEmpty && FabushiAuthValidation.isValidEmail(trimmedEmail) {
                envelope = try await coordinator.dispatchAuthTransition(
                    method: FabushiRPCNamespace.auth.method("emailRegisterSplitKey"),
                    params: FabushiSplitKeyEmailRegisterRequest(
                        email: trimmedEmail,
                        code: trimmedOtp,
                        username: trimmedUsername,
                        password: nil,
                        firstName: resolvedFirstName,
                        lastName: nil,
                        deviceName: "Fabushi iOS",
                        authVersion: 2,
                        authSaltB64: derived.authSaltB64,
                        authVerifierB64: derived.authVerifierB64,
                        kdfParams: derived.kdfParams
                    ),
                    extractIdentity: { $0.identityTuple }
                )
            } else {
                envelope = try await coordinator.dispatchAuthTransition(
                    method: FabushiRPCNamespace.auth.method("registerSplitKey"),
                    params: FabushiSplitKeyRegisterRequest(
                        username: trimmedUsername,
                        password: nil,
                        firstName: resolvedFirstName,
                        lastName: nil,
                        deviceName: "Fabushi iOS",
                        autoWelcome: false,
                        authVersion: 2,
                        authSaltB64: derived.authSaltB64,
                        authVerifierB64: derived.authVerifierB64,
                        kdfParams: derived.kdfParams
                    ),
                    extractIdentity: { $0.identityTuple }
                )
            }

            if let identity = envelope.result.identityTuple {
                await initializeAndUploadSplitKeyVault(
                    userId: identity.userId,
                    password: passwordText,
                    saltB64: derived.authSaltB64,
                    kdfParams: derived.kdfParams
                )
            }
            completeAuthentication()
        } catch {
            handleAuthError(error, isQrPair: false)
        }
    }

    // MARK: - F02: Phone SMS & Email OTP Verification

    public func sendOTP(theme: FabushiTheme) async {
        clearErrors()
        otpStatusMessage = nil
        guard await applyServerURLIfNeeded(theme: theme) else { return }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            switch otpChannel {
            case .phone:
                guard let normalizedPhone = FabushiAuthValidation.normalizeE164Phone(
                    countryCode: selectedDialCode.dialCode,
                    rawNumber: phoneText
                ) else {
                    errorMessage = theme.localized("hig.auth.invalidPhone")
                    return
                }
                let response: FabushiPhoneSendCodeResponse = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.auth.method("phoneSendCode"),
                    params: FabushiPhoneSendCodeRequest(phone: normalizedPhone)
                )
                phoneCodeHash = response.phoneCodeHash
                otpCodeSent = true
                otpStatusMessage = theme.localized("hig.auth.otpSent")
                let cooldown = max(1, min(300, Int(response.resendAfterMs / 1_000)))
                startOtpCountdown(seconds: cooldown > 0 ? cooldown : 60)

            case .email:
                let trimmedEmail = emailText.trimmingCharacters(in: .whitespacesAndNewlines)
                guard FabushiAuthValidation.isValidEmail(trimmedEmail) else {
                    errorMessage = theme.localized("hig.auth.invalidEmail")
                    return
                }
                let response: FabushiEmailSendCodeResponse = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.auth.method("emailSendCode"),
                    params: FabushiEmailSendCodeRequest(email: trimmedEmail)
                )
                otpCodeSent = response.sent
                otpStatusMessage = theme.localized("hig.auth.otpSent")
                let cooldown = max(1, min(300, Int(response.cooldownSeconds)))
                startOtpCountdown(seconds: cooldown)
            }
        } catch {
            handleAuthError(error, isQrPair: false)
        }
    }

    public func verifyOTP(theme: FabushiTheme) async {
        clearErrors()
        guard await applyServerURLIfNeeded(theme: theme) else { return }

        let trimmedCode = otpCodeText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmedCode.count >= 4, trimmedCode.count <= 8 else {
            errorMessage = theme.localized("hig.auth.invalidOTPCode")
            return
        }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            switch otpChannel {
            case .phone:
                guard let normalizedPhone = FabushiAuthValidation.normalizeE164Phone(
                    countryCode: selectedDialCode.dialCode,
                    rawNumber: phoneText
                ) else {
                    errorMessage = theme.localized("hig.auth.invalidPhone")
                    return
                }
                let trimmedUsername = usernameText.trimmingCharacters(in: .whitespacesAndNewlines)
                let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                    method: FabushiRPCNamespace.auth.method("phoneVerify"),
                    params: FabushiPhoneVerifyRequest(
                        phone: normalizedPhone,
                        phoneCodeHash: phoneCodeHash,
                        code: trimmedCode,
                        username: trimmedUsername.isEmpty ? nil : trimmedUsername,
                        password: nil,
                        firstName: firstNameText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? nil
                            : firstNameText.trimmingCharacters(in: .whitespacesAndNewlines),
                        lastName: nil,
                        deviceName: "Fabushi iOS"
                    ),
                    extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
                )
                completeAuthentication()

            case .email:
                let trimmedEmail = emailText.trimmingCharacters(in: .whitespacesAndNewlines)
                guard FabushiAuthValidation.isValidEmail(trimmedEmail) else {
                    errorMessage = theme.localized("hig.auth.invalidEmail")
                    return
                }
                let trimmedUsername = usernameText.trimmingCharacters(in: .whitespacesAndNewlines)
                if authTab == .register, !trimmedUsername.isEmpty, !passwordText.isEmpty {
                    let derived: FabushiDeriveSplitKeyResponse = try await coordinator.dispatchFenced(
                        method: FabushiRPCNamespace.vault.method("deriveSplitKey"),
                        params: FabushiDeriveSplitKeyRequest(
                            password: passwordText,
                            saltB64: nil,
                            kdfParams: nil,
                            fastKdf: useFastKdfForTests ? true : nil
                        )
                    )
                    let resolvedFirst = firstNameText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        ? trimmedUsername
                        : firstNameText.trimmingCharacters(in: .whitespacesAndNewlines)
                    let envelope: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                        method: FabushiRPCNamespace.auth.method("emailRegisterSplitKey"),
                        params: FabushiSplitKeyEmailRegisterRequest(
                            email: trimmedEmail,
                            code: trimmedCode,
                            username: trimmedUsername,
                            password: nil,
                            firstName: resolvedFirst,
                            lastName: nil,
                            deviceName: "Fabushi iOS",
                            authVersion: 2,
                            authSaltB64: derived.authSaltB64,
                            authVerifierB64: derived.authVerifierB64,
                            kdfParams: derived.kdfParams
                        ),
                        extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
                    )
                    if let identity = envelope.result.identityTuple {
                        await initializeAndUploadSplitKeyVault(
                            userId: identity.userId,
                            password: passwordText,
                            saltB64: derived.authSaltB64,
                            kdfParams: derived.kdfParams
                        )
                    }
                    completeAuthentication()
                } else {
                    let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                        method: FabushiRPCNamespace.auth.method("emailVerify"),
                        params: FabushiEmailVerifyRequest(email: trimmedEmail, code: trimmedCode),
                        extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
                    )
                    completeAuthentication()
                }
            }
        } catch {
            handleAuthError(error, isQrPair: false)
        }
    }

    // MARK: - F03: Alipay QR Polling & Mobile OAuth Callback

    public func startAlipayLogin(theme: FabushiTheme) async {
        clearErrors()
        cancelAlipayLogin()
        guard await applyServerURLIfNeeded(theme: theme) else { return }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            let startResp: FabushiAlipayStartResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("alipayStart"),
                params: FabushiEmptyParams()
            )
            alipayAttemptId = startResp.attemptId
            alipayLoginURL = startResp.loginUrl
            alipayExpectedState = FabushiAuthValidation.extractStateQueryParam(from: startResp.loginUrl)
            alipayExpiresAtMs = startResp.expiresAtMs
            #if canImport(UIKit)
            alipayQRImage = FabushiQRCodeRenderer.renderQRCodeImage(from: startResp.loginUrl)
            #endif
            alipayStatusText = theme.localized("hig.auth.alipayWaiting")
            isAlipayActive = true

            startAlipayPollingLoop(attemptId: startResp.attemptId, theme: theme)
        } catch {
            handleAuthError(error, isQrPair: false)
        }
    }

    public func cancelAlipayLogin() {
        taskBag.cancel(key: "alipayPoll")
        alipayWebController.cancel()
        isAlipayActive = false
        alipayAttemptId = nil
        alipayLoginURL = nil
        alipayExpectedState = nil
        alipayExpiresAtMs = nil
        alipayStatusText = nil
        #if canImport(UIKit)
        alipayQRImage = nil
        #endif
    }

    public func launchAlipayMobileOAuth(theme: FabushiTheme) {
        guard let urlString = alipayLoginURL, let url = URL(string: urlString) else {
            return
        }
        alipayWebController.startOAuthSession(url: url, callbackScheme: "fabushi") { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let callbackURL):
                Task { @MainActor in
                    await self.handleAuthCallbackURL(callbackURL, theme: theme)
                }
            case .failure(let error):
                self.errorMessage = error.localizedDescription
            }
        }
    }

    @discardableResult
    public func handleAuthCallbackURL(_ url: URL, theme: FabushiTheme) async -> Bool {
        guard let parsed = FabushiAuthValidation.parseAlipayCallbackURL(url) else {
            return false
        }
        if let expected = alipayExpectedState,
           let incoming = parsed.state,
           expected != incoming {
            errorMessage = theme.localized("hig.auth.alipayStateMismatch")
            return false
        }
        if isAlipayActive, alipayAttemptId != nil {
            await pollAlipayOnce(theme: theme)
            return true
        }
        return false
    }

    @discardableResult
    public func pollAlipayOnce(theme: FabushiTheme) async -> FabushiAlipayPollResponse? {
        guard let attemptId = alipayAttemptId else { return nil }
        if let expiresAtMs = alipayExpiresAtMs,
           expiresAtMs > 0,
           Int64(Date().timeIntervalSince1970 * 1_000) > expiresAtMs {
            taskBag.cancel(key: "alipayPoll")
            alipayStatusText = theme.localized("hig.auth.alipayExpired")
            return nil
        }

        do {
            let envelope: FabushiRPCEnvelopeResult<FabushiAlipayPollResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("alipayPoll"),
                params: FabushiAlipayPollRequest(attemptId: attemptId),
                extractIdentity: { (res: FabushiAlipayPollResponse) in res.resolvedIdentity() }
            )
            let poll = envelope.result
            if poll.isCompleted {
                taskBag.cancel(key: "alipayPoll")
                alipayWebController.cancel()
                alipayStatusText = theme.localized("hig.auth.alipayConfirmed")
                if coordinator.authState == .unauthenticated {
                    _ = try? await coordinator.syncAuthoritativeSessionState()
                }
                completeAuthentication()
            } else if poll.isExpired {
                taskBag.cancel(key: "alipayPoll")
                alipayStatusText = theme.localized("hig.auth.alipayExpired")
            } else {
                alipayStatusText = theme.localized("hig.auth.alipayWaiting")
            }
            return poll
        } catch {
            if !Task.isCancelled {
                errorMessage = error.localizedDescription
            }
            return nil
        }
    }

    // MARK: - F04: Passkeys (WebAuthn / AuthenticationServices) & Cross-Device QR Pairing

    public func signInWithPasskey(theme: FabushiTheme) async {
        clearErrors()
        guard await applyServerURLIfNeeded(theme: theme) else { return }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            let trimmedUsername = usernameText.trimmingCharacters(in: .whitespacesAndNewlines)
            let challenge: FabushiPasskeyChallengeDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyChallenge"),
                params: FabushiPasskeyChallengeRequest(
                    username: trimmedUsername.isEmpty ? nil : trimmedUsername
                )
            )

            let assertion = try await passkeyController.performAssertion(
                rpId: challenge.rpId,
                challenge: challenge.challenge
            )

            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("passkeyVerify"),
                params: FabushiPasskeyVerifyRequest(
                    challenge: challenge.challenge,
                    credentialId: assertion.credentialIdB64,
                    clientDataJson: assertion.clientDataJsonB64,
                    authenticatorData: assertion.authenticatorDataB64,
                    signature: assertion.signatureB64,
                    userId: challenge.userId
                ),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )
            completeAuthentication()
        } catch let webAuthnError as FabushiWebAuthnError {
            switch webAuthnError {
            case .cancelled:
                errorMessage = theme.localized("hig.auth.passkeyCancelled")
            case .notEntitledOrUnavailable(let detail):
                errorMessage = "\(theme.localized("hig.auth.passkeyUnavailable")) (\(detail))"
            default:
                errorMessage = webAuthnError.localizedDescription
            }
        } catch {
            handleAuthError(error, isQrPair: false)
        }
    }

    public func startQrPair(theme: FabushiTheme) async {
        clearErrors()
        cancelQrPair()
        guard await applyServerURLIfNeeded(theme: theme) else { return }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            let initResp: FabushiQrPairInitResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("qrPairInit"),
                params: FabushiEmptyParams()
            )
            qrPairToken = initResp.pairToken
            qrPairURL = initResp.qrUrl
            qrPairExpiresAtMs = initResp.expiresAtMs
            #if canImport(UIKit)
            qrPairImage = FabushiQRCodeRenderer.renderQRCodeImage(from: initResp.qrUrl)
            #endif
            qrPairStatusText = theme.localized("hig.auth.qrPairWaiting")
            isQrPairActive = true

            startQrPairPollingLoop(pairToken: initResp.pairToken, theme: theme)
        } catch {
            handleAuthError(error, isQrPair: true)
        }
    }

    public func cancelQrPair() {
        taskBag.cancel(key: "qrPairPoll")
        isQrPairActive = false
        qrPairToken = nil
        qrPairURL = nil
        qrPairExpiresAtMs = nil
        qrPairStatusText = nil
        #if canImport(UIKit)
        qrPairImage = nil
        #endif
    }

    @discardableResult
    public func pollQrPairOnce(theme: FabushiTheme) async -> FabushiQrPairPollResponse? {
        guard let token = qrPairToken else { return nil }
        if let expiresAtMs = qrPairExpiresAtMs,
           expiresAtMs > 0,
           Int64(Date().timeIntervalSince1970 * 1_000) > expiresAtMs {
            taskBag.cancel(key: "qrPairPoll")
            qrPairStatusText = theme.localized("hig.auth.qrPairExpired")
            return nil
        }

        do {
            let envelope: FabushiRPCEnvelopeResult<FabushiQrPairPollResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("qrPairPoll"),
                params: FabushiQrPairPollRequest(pairToken: token),
                extractIdentity: { (res: FabushiQrPairPollResponse) in res.resolvedIdentity() }
            )
            let poll = envelope.result
            if poll.resolvedIdentity() != nil || (poll.token != nil && !(poll.token?.isEmpty ?? true)) {
                taskBag.cancel(key: "qrPairPoll")
                qrPairStatusText = theme.localized("hig.auth.qrPairConfirmed")
                if coordinator.authState == .unauthenticated {
                    _ = try? await coordinator.syncAuthoritativeSessionState()
                }
                completeAuthentication()
            } else if poll.requiresTwoFactor {
                taskBag.cancel(key: "qrPairPoll")
                activeTwoFactorChallenge = FabushiTwoFactorChallenge(
                    challengeToken: token,
                    hint: nil,
                    isQrPair: true
                )
            } else if poll.isExpired {
                taskBag.cancel(key: "qrPairPoll")
                qrPairStatusText = theme.localized("hig.auth.qrPairExpired")
            } else {
                qrPairStatusText = theme.localized("hig.auth.qrPairWaiting")
            }
            return poll
        } catch {
            handleAuthError(error, isQrPair: true)
            return nil
        }
    }

    // MARK: - F05: Cloud 2FA Verification Gate

    public func submitTwoFactorVerification(theme: FabushiTheme) async {
        guard let challenge = activeTwoFactorChallenge else { return }
        clearErrors()

        guard !twoFactorPasswordText.isEmpty else {
            errorMessage = theme.localized("hig.auth.twoFactorMissingPassword")
            return
        }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            if challenge.isQrPair {
                let envelope: FabushiRPCEnvelopeResult<FabushiQrPairPollResponse> = try await coordinator.dispatchAuthTransition(
                    method: FabushiRPCNamespace.auth.method("qrPairVerify"),
                    params: FabushiQrPairVerifyRequest(
                        pairToken: challenge.challengeToken,
                        cloudPassword: twoFactorPasswordText
                    ),
                    extractIdentity: { (res: FabushiQrPairPollResponse) in res.resolvedIdentity() }
                )
                if envelope.result.resolvedIdentity() == nil,
                   coordinator.authState == .unauthenticated {
                    _ = try? await coordinator.syncAuthoritativeSessionState()
                }
            } else {
                let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                    method: FabushiRPCNamespace.auth.method("twoFactorVerify"),
                    params: FabushiTwoFactorVerifyRequest(
                        challengeToken: challenge.challengeToken,
                        cloudPassword: twoFactorPasswordText,
                        deviceName: "Fabushi iOS"
                    ),
                    extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
                )
            }
            activeTwoFactorChallenge = nil
            twoFactorPasswordText = ""
            completeAuthentication()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    public func cancelTwoFactorChallenge() {
        activeTwoFactorChallenge = nil
        twoFactorPasswordText = ""
        errorMessage = nil
    }

    // MARK: - Private Split-Key V2 Vault Bootstrap & Polling Helpers

    private func fetchKdfParamsOrFallback(username: String) async throws -> FabushiKdfParamsResponse? {
        do {
            return try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("kdfParams"),
                params: FabushiKdfParamsRequest(usernameOrEmail: username)
            )
        } catch let hostError as FabushiHostError {
            switch hostError {
            case .notFound, .unknownCommand:
                return nil
            case .rpcError(_, let message, _) where message.contains("404") || message.contains("405"):
                return nil
            default:
                throw hostError
            }
        }
    }

    private func bootstrapVaultAfterSplitKeyLogin(
        userId: UInt64,
        password: String,
        vaultKekB64: String,
        saltB64: String,
        kdfParams: FabushiArgon2KdfParamsDTO
    ) async {
        let accountId = String(userId)
        do {
            let envelopeResp: FabushiVaultEnvelopeResponseDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("envelopeGet"),
                params: FabushiEmptyParams()
            )
            if let flatEnv = envelopeResp.envelope,
               flatEnv.kdfAlg == "split_key_v2" || flatEnv.kdfAlg == "argon2id" {
                let nested = FabushiVaultEnvelopeConverter.toNested(flatEnv)
                let unlocked: FabushiVaultUnlockWithSplitKeyResponse = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("unlockWithSplitKey"),
                    params: FabushiVaultUnlockWithSplitKeyRequest(
                        userId: userId,
                        envelope: nested,
                        vaultKekB64: vaultKekB64,
                        password: nil
                    )
                )
                if unlocked.unlocked {
                    try? securityKeychain.saveVaultRootKeyBase64(unlocked.rootKeyB64, accountId: accountId)
                    coordinator.vaultStore.setUnlocked(true)
                    lastCompletedVaultBootstrap = true
                }
            } else if envelopeResp.envelope == nil {
                await initializeAndUploadSplitKeyVault(
                    userId: userId,
                    password: password,
                    saltB64: saltB64,
                    kdfParams: kdfParams
                )
            }
        } catch {
            // Non-fatal: login succeeded even if server vault envelope endpoint is unavailable.
            lastCompletedVaultBootstrap = false
        }
    }

    private func initializeAndUploadSplitKeyVault(
        userId: UInt64,
        password: String,
        saltB64: String,
        kdfParams: FabushiArgon2KdfParamsDTO
    ) async {
        let accountId = String(userId)
        do {
            let initResp: FabushiVaultInitSplitKeyResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("initSplitKey"),
                params: FabushiVaultInitSplitKeyRequest(
                    userId: userId,
                    password: password,
                    saltB64: saltB64,
                    kdfParams: kdfParams,
                    fastKdf: useFastKdfForTests ? true : nil
                )
            )
            let flatEnvelope = FabushiVaultEnvelopeConverter.toFlat(initResp.envelope)
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("envelopePut"),
                params: flatEnvelope
            )
            try? securityKeychain.saveVaultRootKeyBase64(initResp.rootKeyB64, accountId: accountId)
            coordinator.vaultStore.setUnlocked(initResp.unlocked)
            generatedEmergencyKit = initResp.emergencyKit
            lastCompletedVaultBootstrap = true
        } catch {
            // Non-fatal if remote vault sync is offline; local key may still have been initialized.
            lastCompletedVaultBootstrap = false
        }
    }

    private func silentlyUpgradeLegacyV1ToSplitKeyV2(
        userId: UInt64,
        oldPassword: String
    ) async {
        do {
            let derived: FabushiDeriveSplitKeyResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("deriveSplitKey"),
                params: FabushiDeriveSplitKeyRequest(
                    password: oldPassword,
                    saltB64: nil,
                    kdfParams: nil,
                    fastKdf: useFastKdfForTests ? true : nil
                )
            )
            let _: FabushiAuthCustodyUpdateResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("upgradeSplitKey"),
                params: FabushiUpgradeSplitKeyRequest(
                    oldPassword: oldPassword,
                    authSaltB64: derived.authSaltB64,
                    authVerifierB64: derived.authVerifierB64,
                    kdfParams: derived.kdfParams
                )
            )
            await initializeAndUploadSplitKeyVault(
                userId: userId,
                password: oldPassword,
                saltB64: derived.authSaltB64,
                kdfParams: derived.kdfParams
            )
        } catch {
            // Silent upgrade is intentionally non-fatal for legacy servers.
        }
    }

    private func completeAuthentication() {
        cancelActiveTasks()
        passwordText = ""
        otpCodeText = ""
        twoFactorPasswordText = ""
        if mode == .primaryGate {
            router.engagePasscodeLockIfEnabled()
        }
        router.handleAccountScopeChange(
            generation: coordinator.accountScopeGeneration,
            coordinator: coordinator
        )
        if !router.isPasscodeLocked {
            router.replayPendingDeepLinkIfReady(coordinator: coordinator)
        }
        onAuthenticatedCallback?()
    }

    private func handleAuthError(_ error: Error, isQrPair: Bool) {
        if let challenge = FabushiTwoFactorChallengeParser.extract(from: error) {
            activeTwoFactorChallenge = FabushiTwoFactorChallenge(
                challengeToken: challenge.challengeToken,
                hint: challenge.hint,
                isQrPair: isQrPair
            )
            errorMessage = nil
            return
        }
        errorMessage = error.localizedDescription
    }

    private func startOtpCountdown(seconds: Int) {
        otpResendRemainingSeconds = max(0, seconds)
        guard otpResendRemainingSeconds > 0 else { return }
        let task = Task<Void, Never> { @MainActor [weak self] in
            while let self, !Task.isCancelled, self.otpResendRemainingSeconds > 0 {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard !Task.isCancelled else { return }
                if self.otpResendRemainingSeconds > 0 {
                    self.otpResendRemainingSeconds -= 1
                }
            }
        }
        taskBag.replace(key: "otpCountdown", with: task)
    }

    private func startAlipayPollingLoop(attemptId: String, theme: FabushiTheme) {
        let interval = pollIntervalNanoseconds
        let task = Task<Void, Never> { @MainActor [weak self] in
            while let self, !Task.isCancelled, self.isAlipayActive, self.alipayAttemptId == attemptId {
                try? await Task.sleep(nanoseconds: interval)
                guard !Task.isCancelled, self.isAlipayActive, self.alipayAttemptId == attemptId else { return }
                let poll = await self.pollAlipayOnce(theme: theme)
                if let poll, poll.isCompleted || poll.isExpired {
                    return
                }
            }
        }
        taskBag.replace(key: "alipayPoll", with: task)
    }

    private func startQrPairPollingLoop(pairToken: String, theme: FabushiTheme) {
        let interval = pollIntervalNanoseconds
        let task = Task<Void, Never> { @MainActor [weak self] in
            while let self, !Task.isCancelled, self.isQrPairActive, self.qrPairToken == pairToken {
                try? await Task.sleep(nanoseconds: interval)
                guard !Task.isCancelled, self.isQrPairActive, self.qrPairToken == pairToken else { return }
                let poll = await self.pollQrPairOnce(theme: theme)
                if let poll, poll.resolvedIdentity() != nil || poll.requiresTwoFactor || poll.isExpired {
                    return
                }
            }
        }
        taskBag.replace(key: "qrPairPoll", with: task)
    }
}
