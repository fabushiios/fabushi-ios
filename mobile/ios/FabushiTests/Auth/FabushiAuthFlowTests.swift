import Foundation
import os
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif
import XCTest
@testable import Fabushi

@MainActor
final class FabushiAuthFlowTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m2b_auth_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychains() -> (FabushiKeychainStore, FabushiSecurityKeychainStore) {
        let uuid = UUID().uuidString
        let coreKC = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m2b.core.\(uuid)",
            account: "m2b-core-test"
        )
        let secKC = FabushiSecurityKeychainStore(
            passcodeService: "com.ombhrum.fabushi.tests.m2b.passcode.\(uuid)",
            passcodeAccount: "passcode-test",
            vaultKeyService: "com.ombhrum.fabushi.tests.m2b.vault.\(uuid)",
            totpService: "com.ombhrum.fabushi.tests.m2b.totp.\(uuid)"
        )
        return (coreKC, secKC)
    }

    // MARK: - Core Additive Hooks: dispatchAuthTransition & syncAuthoritativeSessionState + Lifecycle Epoch Fence

    func testCoordinatorDispatchAuthTransitionAndSyncLifecycleFencing() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":1001,"username":"alice_f01","session_id":"s-1001","token":"tok-1001","first_name":"Alice"}"#)
            case ("GET", "/api/v1/auth/sessions"):
                return .json(200, #"[{"session_id":"s-1001","device_name":"Fabushi iOS","platform":"iOS","ip_address":"127.0.0.1","created_at_ms":1700000000000,"last_active_ms":1700000001000,"is_current":true}]"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("transition")
        let (coreKC, _) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "1001")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            XCTAssertEqual(coordinator.accountScopeGeneration, 1)
            XCTAssertEqual(coordinator.authState, .unauthenticated)

            let envelope: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "alice_f01", password: "SecretPassword!1"),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )

            XCTAssertGreaterThan(envelope.generation, 1, "Rust finalize_authenticated_session must advance generation")
            XCTAssertEqual(coordinator.accountScopeGeneration, envelope.generation)
            XCTAssertEqual(coordinator.activeAccountId, "1001")
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 1001, username: "alice_f01", accountId: "1001")
            )
            XCTAssertTrue(try coreKC.hasAccountKey(accountId: "1001"))

            // Subsequent fenced RPC MUST succeed with zero stale_generation error!
            let sessions: [FabushiActiveDeviceSessionDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.session.method("list"),
                params: FabushiEmptyParams()
            )
            XCTAssertEqual(sessions.count, 1)
            XCTAssertEqual(sessions.first?.sessionId, "s-1001")

            let syncedGen = try await coordinator.syncAuthoritativeSessionState()
            XCTAssertEqual(syncedGen, envelope.generation)

            // Lifecycle epoch fence: shutdown rejects subsequent auth transitions with notStarted.
            await coordinator.shutdown()
            do {
                _ = try await coordinator.syncAuthoritativeSessionState()
                XCTFail("Expected FabushiCoordinatorError.notStarted after shutdown")
            } catch let error as FabushiCoordinatorError {
                XCTAssertEqual(error, .notStarted)
            }
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F01: Split-Key V2 Registration (Zero Plaintext Password on Wire + Flat Vault Envelope Put)

    func testF01SplitKeyV2RegistrationZeroPlaintextAndVaultEnvelopeBootstrap() async throws {
        let capturedEnvelope = OSAllocatedUnfairLock<String?>(initialState: nil)

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/register"):
                return .json(200, #"{"user_id":2001,"username":"t1_f01_v2user","session_id":"s-2001","token":"tok-2001","first_name":"V2User","auth_version":2}"#)
            case ("PUT", "/api/v1/user-vault/envelope"):
                capturedEnvelope.withLock { $0 = req.bodyString }
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("f01_register")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "2001")
            secKC.purgeAccountSecrets(accountId: "2001")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            #if canImport(UIKit)
            defer { UIPasteboard.general.items = [] }
            #endif
            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)
            var didCompleteCallback = false
            let vm = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: []),
                mode: .primaryGate,
                securityKeychain: secKC,
                onAuthenticated: {
                    didCompleteCallback = true
                }
            )
            vm.serverURLText = server.baseURLString
            vm.authTab = .register
            vm.usernameText = "t1_f01_v2user"
            vm.passwordText = "SplitKeyV2!Strong9"

            await vm.submitRegistration(theme: theme)

            XCTAssertNil(vm.errorMessage)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 2001, username: "t1_f01_v2user", accountId: "2001")
            )
            XCTAssertTrue(coordinator.vaultStore.isUnlocked)
            XCTAssertTrue(vm.lastCompletedVaultBootstrap)
            let emergencyKit = try XCTUnwrap(vm.generatedEmergencyKit)
            XCTAssertTrue(emergencyKit.hasPrefix("EK-"))
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "2001"))

            // D4 Option A: Registration holds Auth Gate until user explicitly backs up & acknowledges Emergency Kit
            XCTAssertTrue(vm.isAwaitingEmergencyKitBackup)
            XCTAssertTrue(router.isHoldingPrimaryAuthGate)
            XCTAssertFalse(didCompleteCallback)
            XCTAssertTrue(vm.passwordText.isEmpty, "Transient password must be scrubbed immediately")
            XCTAssertFalse(
                vm.acknowledgeEmergencyKitAndContinue(),
                "Acknowledge must fail closed while hasAcknowledgedEmergencyKitSaved == false"
            )
            XCTAssertTrue(router.isHoldingPrimaryAuthGate)

            XCTAssertTrue(vm.copyGeneratedEmergencyKitToSecurePasteboard())
            XCTAssertTrue(vm.didCopyEmergencyKit)
            #if canImport(UIKit)
            XCTAssertEqual(UIPasteboard.general.string, emergencyKit)
            #endif

            vm.hasAcknowledgedEmergencyKitSaved = true
            XCTAssertTrue(vm.acknowledgeEmergencyKitAndContinue())
            XCTAssertNil(vm.generatedEmergencyKit, "Emergency Kit must be wiped from memory after ack")
            XCTAssertFalse(vm.isAwaitingEmergencyKitBackup)
            XCTAssertFalse(router.isHoldingPrimaryAuthGate)
            XCTAssertTrue(didCompleteCallback)

            let registerReqs = server.requests(method: "POST", pathPrefix: "/api/v1/auth/register")
            XCTAssertEqual(registerReqs.count, 1)
            let registerBody = try XCTUnwrap(registerReqs.first?.bodyString)
            XCTAssertTrue(registerBody.contains("auth_verifier_b64"), "Split-Key V2 registration must transmit auth_verifier_b64")
            XCTAssertTrue(registerBody.contains("auth_salt_b64"), "Split-Key V2 registration must transmit auth_salt_b64")
            XCTAssertFalse(registerBody.contains("SplitKeyV2!Strong9"), "Plaintext password must NEVER hit the wire!")
            XCTAssertFalse(registerBody.contains("\"password\""), "password key must be omitted for Split-Key V2 registration")

            let uploadedEnvelope = try XCTUnwrap(capturedEnvelope.withLock { $0 })
            XCTAssertTrue(uploadedEnvelope.contains("split_key_v2"))
            XCTAssertTrue(uploadedEnvelope.contains("kdf_m_cost_kib"))
            XCTAssertTrue(uploadedEnvelope.contains("wrapped_root_key_b64"))

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F01: Split-Key V2 Login + Transparent Vault Unlock, Legacy V1 Silent Upgrade, Server URL Validation

    func testF01SplitKeyV2LoginLegacyV1UpgradeAndServerURLFailClosed() async throws {
        let storedSalt = OSAllocatedUnfairLock<String>(initialState: "")
        let storedEnvelopeJSON = OSAllocatedUnfairLock<String?>(initialState: nil)
        let legacyMode = OSAllocatedUnfairLock<Bool>(initialState: false)

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("GET", "/api/v1/auth/kdf-params"):
                if legacyMode.withLock({ $0 }) {
                    return .json(404, #"{"error":"not_found"}"#)
                }
                let salt = storedSalt.withLock { $0 }
                return .json(
                    200,
                    #"{"auth_version":2,"auth_salt_b64":"\#(salt)","kdf_params":{"m_cost_kib":16384,"t_cost":2,"p_cost":1}}"#
                )
            case ("POST", "/api/v1/auth/login"):
                if legacyMode.withLock({ $0 }) {
                    return .json(200, #"{"user_id":2003,"username":"legacy_v1","session_id":"s-2003","token":"tok-2003","first_name":"Legacy"}"#)
                }
                return .json(200, #"{"user_id":2002,"username":"split_login_user","session_id":"s-2002","token":"tok-2002","first_name":"Split","auth_version":2}"#)
            case ("GET", "/api/v1/user-vault/envelope"):
                if let env = storedEnvelopeJSON.withLock({ $0 }) {
                    return .json(200, "{\"envelope\":\(env)}")
                }
                return .json(200, #"{"envelope":null}"#)
            case ("PUT", "/api/v1/user-vault/envelope"):
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            case ("POST", "/api/v1/auth/upgrade-split-key"):
                return .json(200, #"{"ok":true,"auth_version":2}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("f01_login")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "2002")
            try? coreKC.deleteAccountKey(accountId: "2003")
            secKC.purgeAccountSecrets(accountId: "2002")
            secKC.purgeAccountSecrets(accountId: "2003")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            // Prepare a genuine Rust-generated Split-Key V2 envelope for user 2002 with production Argon2id strength
            let initResp: FabushiVaultInitSplitKeyResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("initSplitKey"),
                params: FabushiVaultInitSplitKeyRequest(
                    userId: 2002,
                    password: "MyV2Password!99",
                    saltB64: nil,
                    kdfParams: FabushiArgon2KdfParamsDTO(mCostKib: 16_384, tCost: 2, pCost: 1),
                    fastKdf: nil
                )
            )
            let flatEnv = FabushiVaultEnvelopeConverter.toFlat(initResp.envelope)
            let flatEnvData = try JSONEncoder().encode(flatEnv)
            storedSalt.withLock { $0 = initResp.authSaltB64 }
            storedEnvelopeJSON.withLock { $0 = String(decoding: flatEnvData, as: UTF8.self) }

            // Lock vault before login to prove Split-Key V2 login transparently re-unlocks it!
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiEmptyParams()
            )
            coordinator.vaultStore.setUnlocked(false)

            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)
            let vm = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: []),
                securityKeychain: secKC
            )

            // 1. Fail-closed invalid Server URL (T2_10)
            vm.serverURLText = "not-a-valid-url://bad"
            vm.usernameText = "split_login_user"
            vm.passwordText = "MyV2Password!99"
            await vm.submitLogin(theme: theme)
            XCTAssertNotNil(vm.serverURLError)
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertTrue(server.requests(method: "POST", pathPrefix: "/api/v1/auth/login").isEmpty)

            // 2. Valid Split-Key V2 login -> zero plaintext password on wire + transparent vault unlock
            vm.serverURLText = server.baseURLString
            await vm.submitLogin(theme: theme)
            XCTAssertNil(vm.serverURLError)
            XCTAssertNil(vm.errorMessage)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 2002, username: "split_login_user", accountId: "2002")
            )
            XCTAssertTrue(coordinator.vaultStore.isUnlocked)
            XCTAssertEqual(try secKC.loadVaultRootKeyBase64(accountId: "2002"), initResp.rootKeyB64)

            let loginReqBody = try XCTUnwrap(server.requests(method: "POST", pathPrefix: "/api/v1/auth/login").first?.bodyString)
            XCTAssertTrue(loginReqBody.contains("auth_verifier_b64"))
            XCTAssertFalse(loginReqBody.contains("MyV2Password!99"))

            // 3. Legacy V1 fallback + silent Split-Key V2 upgrade (fresh account with nil server envelope -> D4 Hybrid non-blocking Shell banner!)
            legacyMode.withLock { $0 = true }
            storedEnvelopeJSON.withLock { $0 = nil }
            _ = try await coordinator.clearLocalSession()
            vm.usernameText = "legacy_v1"
            vm.passwordText = "LegacyPass!42"
            await vm.submitLogin(theme: theme)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 2003, username: "legacy_v1", accountId: "2003")
            )
            let upgradeReqs = server.requests(method: "POST", pathPrefix: "/api/v1/auth/upgrade-split-key")
            XCTAssertEqual(upgradeReqs.count, 1)
            let upgradeBody = try XCTUnwrap(upgradeReqs.first?.bodyString)
            XCTAssertTrue(upgradeBody.contains("auth_verifier_b64"))
            XCTAssertTrue(upgradeBody.contains("auth_salt_b64"))
            XCTAssertEqual(server.requests(method: "PUT", pathPrefix: "/api/v1/user-vault/envelope").count, 1)
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "2003"))
            XCTAssertFalse(
                router.isHoldingPrimaryAuthGate,
                "Silent Legacy V1 login upgrade must not block shell entry"
            )
            XCTAssertTrue(
                router.pendingEmergencyKit?.hasPrefix("EK-") == true,
                "Silent Legacy V1 upgrade must surface one-time Emergency Kit in persistent Shell banner"
            )

            // Verify router.pendingEmergencyKit auto-clears on switchAccount to another account and on logout
            let switchedGen = try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 2002,
                username: "split_login_user",
                sessionId: "s-2002-sw",
                token: "tok-2002-sw"
            ))
            XCTAssertTrue(router.handleAccountScopeChange(generation: switchedGen, coordinator: coordinator))
            XCTAssertNil(
                router.pendingEmergencyKit,
                "Switching to another account must auto-clear prior account pendingEmergencyKit"
            )

            router.presentPendingEmergencyKit("EK-LOGOUT-CLEAR-2002", forAccountId: "2002")
            XCTAssertEqual(router.pendingEmergencyKit, "EK-LOGOUT-CLEAR-2002")
            let clearedGen = try await coordinator.clearLocalSession()
            XCTAssertTrue(router.handleAccountScopeChange(generation: clearedGen, coordinator: coordinator))
            XCTAssertNil(
                router.pendingEmergencyKit,
                "Logging out / clearing session must auto-clear pendingEmergencyKit"
            )

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F02: Phone SMS & Email OTP Verification + Split-Key V2 Email Registration

    func testF02PhoneSMSAndEmailOTPFlowsAndValidation() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/phone/send-code"):
                return .json(200, #"{"ok":true,"sent":true,"phone_code_hash":"hash-99","expires_at_ms":1900000000000,"resend_after_ms":60000,"debug_code":"123456"}"#)
            case ("POST", "/api/v1/auth/phone/verify"):
                return .json(200, #"{"user_id":3001,"username":"phone_user","session_id":"s-3001","token":"tok-3001","first_name":"Phone"}"#)
            case ("POST", "/api/v1/auth/email/send-code"):
                return .json(200, #"{"sent":true,"cooldown_seconds":45,"expires_in_seconds":600}"#)
            case ("POST", "/api/v1/auth/email/verify"):
                return .json(200, #"{"user_id":3002,"username":"email_user","session_id":"s-3002","token":"tok-3002","first_name":"Email"}"#)
            case ("POST", "/api/v1/auth/email/register"):
                return .json(200, #"{"user_id":3003,"username":"email_v2","session_id":"s-3003","token":"tok-3003","first_name":"EmailV2","auth_version":2}"#)
            case ("PUT", "/api/v1/user-vault/envelope"):
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("f02_otp")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "3001")
            try? coreKC.deleteAccountKey(accountId: "3002")
            try? coreKC.deleteAccountKey(accountId: "3003")
            secKC.purgeAccountSecrets(accountId: "3003")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)
            let vm = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                securityKeychain: secKC
            )
            vm.serverURLText = server.baseURLString

            // Invalid phone fails closed without hitting server
            vm.otpChannel = .phone
            vm.phoneText = "123"
            await vm.sendOTP(theme: theme)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertFalse(vm.otpCodeSent)
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/auth/phone").count, 0)

            // Valid E.164 phone SMS send -> too-short OTP rejected locally -> valid verify succeeds
            vm.phoneText = "+14155550168"
            await vm.sendOTP(theme: theme)
            XCTAssertNil(vm.errorMessage)
            XCTAssertTrue(vm.otpCodeSent)
            XCTAssertEqual(vm.phoneCodeHash, "hash-99")
            XCTAssertGreaterThan(vm.otpResendRemainingSeconds, 0)

            vm.otpCodeText = "12"
            await vm.verifyOTP(theme: theme)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/auth/phone/verify").count, 0)

            vm.otpCodeText = "123456"
            await vm.verifyOTP(theme: theme)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 3001, username: "phone_user", accountId: "3001")
            )
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/auth/phone").count, 2)

            // Invalid email fails closed without hitting server
            _ = try await coordinator.clearLocalSession()
            vm.otpChannel = .email
            vm.emailText = "bad-email"
            await vm.sendOTP(theme: theme)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/auth/email").count, 0)

            // Valid email send -> verify
            vm.emailText = "alice@example.com"
            await vm.sendOTP(theme: theme)
            XCTAssertNil(vm.errorMessage)
            vm.otpCodeText = "654321"
            await vm.verifyOTP(theme: theme)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 3002, username: "email_user", accountId: "3002")
            )

            // Email Split-Key V2 Registration (zero plaintext password on wire + D4 Emergency Kit hold)
            _ = try await coordinator.clearLocalSession()
            vm.authTab = .register
            vm.usernameText = "email_v2"
            vm.passwordText = "EmailSplitSecret!8"
            vm.emailText = "v2@example.com"
            vm.otpCodeText = "777888"
            await vm.verifyOTP(theme: theme)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 3003, username: "email_v2", accountId: "3003")
            )
            XCTAssertTrue(vm.isAwaitingEmergencyKitBackup)
            XCTAssertTrue(router.isHoldingPrimaryAuthGate)
            vm.hasAcknowledgedEmergencyKitSaved = true
            XCTAssertTrue(vm.acknowledgeEmergencyKitAndContinue())
            XCTAssertFalse(router.isHoldingPrimaryAuthGate)
            XCTAssertNil(vm.generatedEmergencyKit)

            let emailRegBody = try XCTUnwrap(server.requests(method: "POST", pathPrefix: "/api/v1/auth/email/register").first?.bodyString)
            XCTAssertTrue(emailRegBody.contains("auth_verifier_b64"))
            XCTAssertFalse(emailRegBody.contains("EmailSplitSecret!8"))

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F03: Alipay QR Start, CoreImage Rendering, Poll Completion & OAuth Callback Consumption

    func testF03AlipayQRPollingCoreImageAndOAuthDeepLinkCallback() async throws {
        let pollCounter = OSAllocatedUnfairLock<Int>(initialState: 0)

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/alipay/start"):
                return .json(200, #"{"attempt_id":"alipay_1","login_url":"https://auth.alipay.example/oauth?state=csrf_42","expires_at_ms":2900000000000}"#)
            case ("GET", "/api/v1/auth/alipay/poll/alipay_1"):
                let count = pollCounter.withLock { c -> Int in
                    c += 1
                    return c
                }
                if count == 1 {
                    return .json(200, #"{"status":"pending"}"#)
                }
                return .json(200, #"{"status":"completed","session":{"user_id":4001,"username":"alipay_alice","session_id":"s-4001","token":"tok-4001","first_name":"Alipay"}}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("f03_alipay")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "4001")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)
            let vm = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                securityKeychain: secKC,
                pollIntervalNanoseconds: 5_000_000_000 // manually trigger poll steps deterministically
            )
            vm.serverURLText = server.baseURLString

            await vm.startAlipayLogin(theme: theme)
            XCTAssertTrue(vm.isAlipayActive)
            XCTAssertEqual(vm.alipayAttemptId, "alipay_1")
            XCTAssertEqual(vm.alipayExpectedState, "csrf_42")
            XCTAssertNotNil(vm.alipayQRImage, "CoreImage QR image must render for Alipay login URL")

            // First poll is pending
            let firstPoll = await vm.pollAlipayOnce(theme: theme)
            XCTAssertEqual(firstPoll?.status, "pending")
            XCTAssertEqual(coordinator.authState, .unauthenticated)

            // Mismatched CSRF state callback URL MUST fail closed!
            let badCallbackURL = try XCTUnwrap(URL(string: "fabushi://auth/alipay?state=wrong_state&auth_code=xyz"))
            XCTAssertTrue(router.handleOpenURL(badCallbackURL, coordinator: coordinator))
            let consumedBad = try XCTUnwrap(router.consumeLastAuthCallbackURL())
            XCTAssertNil(router.lastAuthCallbackURL)
            let rejected = await vm.handleAuthCallbackURL(consumedBad, theme: theme)
            XCTAssertFalse(rejected)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertEqual(coordinator.authState, .unauthenticated)

            // Matching CSRF state callback URL consumes router URL & triggers completion poll!
            let goodCallbackURL = try XCTUnwrap(URL(string: "fabushi://auth/alipay?state=csrf_42&auth_code=ok_code"))
            XCTAssertTrue(router.handleOpenURL(goodCallbackURL, coordinator: coordinator))
            let consumedGood = try XCTUnwrap(router.consumeLastAuthCallbackURL())
            XCTAssertNil(router.lastAuthCallbackURL)
            let accepted = await vm.handleAuthCallbackURL(consumedGood, theme: theme)
            XCTAssertTrue(accepted)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 4001, username: "alipay_alice", accountId: "4001")
            )

            // Expired Alipay session stops polling cleanly
            vm.alipayAttemptId = "alipay_expired"
            vm.alipayExpiresAtMs = 1_000
            let expiredPoll = await vm.pollAlipayOnce(theme: theme)
            XCTAssertNil(expiredPoll)
            XCTAssertEqual(vm.alipayStatusText, theme.localized("hig.auth.alipayExpired"))

            vm.cancelActiveTasks()
            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F04: Passkey Challenge Wire Path & Cross-Device QR Device Pairing

    func testF04PasskeyChallengeWireAndQRDevicePairing() async throws {
        let pairAccepted = OSAllocatedUnfairLock<Bool>(initialState: false)

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/passkey/challenge"):
                return .json(200, #"{"challenge":"00112233445566778899aabbccddeeff0123456789abcdef0f1e2d3c4b5a6978","rp_id":"fabushi.app","timeout_ms":60000,"user_id":5001}"#)
            case ("POST", "/api/v1/auth/passkey/verify"):
                return .json(200, #"{"user_id":5001,"username":"passkey_alice","session_id":"s-5001","token":"tok-5001","first_name":"Passkey"}"#)
            case ("POST", "/api/v1/auth/passkeys"):
                return .json(201, #"{"credential_id":"cred_b64url_ok","user_id":5001,"public_key":"att_b64url_ok","sign_count":0,"device_name":"Fabushi iOS","created_at_ms":1700000000000,"last_used_at_ms":1700000000000}"#)
            case ("GET", "/api/v1/auth/passkeys"):
                return .json(200, #"{"passkeys":[{"credential_id":"cred_b64url_ok","user_id":5001,"public_key":"att_b64url_ok","sign_count":1,"device_name":"Fabushi iOS","created_at_ms":1700000000000,"last_used_at_ms":1700000005000}],"total":1}"#)
            case ("DELETE", "/api/v1/auth/passkeys/cred_b64url_ok"):
                return .json(200, #"{"deleted":true}"#)
            case ("POST", "/api/v1/auth/pair/init"):
                return .json(200, #"{"pair_token":"pair_tok_777","qr_url":"tg://login?token=pair_tok_777","expires_at_ms":2900000000000}"#)
            case ("POST", "/api/v1/auth/pair/accept"):
                pairAccepted.withLock { $0 = true }
                return .json(200, #"{"accepted":true}"#)
            case ("GET", "/api/v1/auth/pair/poll/pair_tok_777"):
                if pairAccepted.withLock({ $0 }) {
                    return .json(200, #"{"status":"completed","pair_token":"pair_tok_777","session":{"user_id":5002,"username":"qr_paired_user","session_id":"s-5002","token":"tok-5002","first_name":"Paired"}}"#)
                }
                return .json(200, #"{"status":"pending","pair_token":"pair_tok_777"}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("f04_qr_passkey")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "5001")
            try? coreKC.deleteAccountKey(accountId: "5002")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            // 1. Real Rust auth.passkeyChallenge wire round-trip + D8-1 hex64 / Base64URL / Base64 exact byte proof
            let challenge: FabushiPasskeyChallengeDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyChallenge"),
                params: FabushiPasskeyChallengeRequest(username: "alice")
            )
            XCTAssertEqual(challenge.rpId, "fabushi.app")
            XCTAssertEqual(challenge.userId, 5001)

            // D8-1: Upstream 64-char SHA-256 hex MUST decode to 32 raw bytes (NOT 48 bogus Base64 bytes!)
            let hexDecoded = try XCTUnwrap(FabushiWebAuthnPasskeyController.decodeChallengeData(challenge.challenge))
            XCTAssertEqual(hexDecoded.count, 32)
            XCTAssertEqual(
                hexDecoded,
                Data([
                    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
                    0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
                    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
                    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78,
                ])
            )
            // FIX-9: Uppercase 64-char hex decodes identically to 32 raw bytes
            let upperHexDecoded = try XCTUnwrap(
                FabushiWebAuthnPasskeyController.decodeChallengeData(
                    "00112233445566778899AABBCCDDEEFF0123456789ABCDEF0F1E2D3C4B5A6978"
                )
            )
            XCTAssertEqual(upperHexDecoded, hexDecoded)

            // FIX-9: 64-char non-hex Base64 falls through from decodeExactHex64 to exact 48 raw bytes
            let raw48 = Data((0..<48).map { UInt8(($0 * 7 + 3) & 0xFF) })
            let base64Len64 = raw48.base64EncodedString()
            XCTAssertEqual(base64Len64.count, 64)
            XCTAssertEqual(
                FabushiWebAuthnPasskeyController.decodeChallengeData(base64Len64),
                raw48
            )

            // Standard Base64 exact bytes
            XCTAssertEqual(
                FabushiWebAuthnPasskeyController.decodeChallengeData("dGVzdC1jaGFsbGVuZ2UtMzJi"),
                Data("test-challenge-32b".utf8)
            )
            // Unpadded Base64URL exact bytes
            XCTAssertEqual(
                FabushiWebAuthnPasskeyController.decodeChallengeData("YWJjLV8"),
                Data([0x61, 0x62, 0x63, 0x2d, 0x5f])
            )
            // Opaque non-Base64 UTF-8 token fallback
            XCTAssertEqual(
                FabushiWebAuthnPasskeyController.decodeChallengeData("pkchal_1_test"),
                Data("pkchal_1_test".utf8)
            )
            // Empty / whitespace / padding-only returns nil (FIX-9)
            XCTAssertNil(FabushiWebAuthnPasskeyController.decodeChallengeData("   "))
            XCTAssertNil(FabushiWebAuthnPasskeyController.decodeChallengeData("===="))

            // D8-2: Base64URL unpadded encoding strips '+', '/', '=' and round-trips losslessly
            let binarySample = Data([0xFB, 0xFF, 0xFE, 0x3E, 0x3F])
            let b64Url = FabushiWebAuthnPasskeyController.base64URLEncode(binarySample)
            XCTAssertFalse(b64Url.contains("+"))
            XCTAssertFalse(b64Url.contains("/"))
            XCTAssertFalse(b64Url.contains("="))
            XCTAssertEqual(FabushiWebAuthnPasskeyController.decodeChallengeData(b64Url), binarySample)

            // Real Rust passkeyVerify -> passkeyRegister -> passkeyList (upstream credential_id keyed) -> passkeyDelete
            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("passkeyVerify"),
                params: FabushiPasskeyVerifyRequest(
                    challenge: challenge.challenge,
                    credentialId: b64Url,
                    clientDataJson: b64Url,
                    authenticatorData: b64Url,
                    signature: b64Url,
                    userId: 5001
                ),
                extractIdentity: { $0.identityTuple }
            )
            let verifyBody = try XCTUnwrap(server.requests(method: "POST", pathPrefix: "/api/v1/auth/passkey/verify").first?.bodyString)
            XCTAssertTrue(verifyBody.contains("\"credential_id\":\"\(b64Url)\""))
            XCTAssertTrue(verifyBody.contains("\"client_data_json\""))
            XCTAssertTrue(verifyBody.contains("\"authenticator_data\""))
            XCTAssertTrue(verifyBody.contains("\"signature\""))

            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyRegister"),
                params: FabushiPasskeyRegisterRequest(
                    credentialId: "cred_b64url_ok",
                    publicKey: "att_b64url_ok",
                    deviceName: "Fabushi iOS"
                )
            )
            let rawPasskeys: [FabushiJSONValue] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyList"),
                params: FabushiEmptyParams()
            )
            let parsedPasskeys = rawPasskeys.compactMap(FabushiPasskeyRecordDTO.fromJSONValue)
            XCTAssertEqual(parsedPasskeys.count, 1)
            XCTAssertEqual(parsedPasskeys.first?.id, "cred_b64url_ok")
            XCTAssertEqual(parsedPasskeys.first?.credentialId, "cred_b64url_ok")
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyDelete"),
                params: FabushiPasskeyDeleteRequest(passkeyId: "cred_b64url_ok")
            )
            XCTAssertEqual(server.requests(method: "DELETE", pathPrefix: "/api/v1/auth/passkeys/cred_b64url_ok").count, 1)

            // 2. Real QR Pair Init -> CoreImage QR -> URI token parser -> Accept -> Poll completion
            _ = try await coordinator.clearLocalSession()
            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)
            let vm = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                securityKeychain: secKC,
                pollIntervalNanoseconds: 5_000_000_000
            )
            vm.serverURLText = server.baseURLString

            await vm.startQrPair(theme: theme)
            XCTAssertTrue(vm.isQrPairActive)
            XCTAssertEqual(vm.qrPairToken, "pair_tok_777")
            XCTAssertEqual(vm.qrPairURL, "tg://login?token=pair_tok_777")
            XCTAssertNotNil(vm.qrPairImage)
            XCTAssertEqual(
                FabushiAuthValidation.extractQrPairToken(from: "tg://login?token=pair_tok_777"),
                "pair_tok_777"
            )

            // Pending poll
            let pending = await vm.pollQrPairOnce(theme: theme)
            XCTAssertEqual(pending?.status, "pending")

            // Authenticated owner approves pairing token
            pairAccepted.withLock { $0 = true }

            // Poll completes session cleanly via dispatchAuthTransition
            let completed = await vm.pollQrPairOnce(theme: theme)
            XCTAssertEqual(completed?.status, "completed")
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 5002, username: "qr_paired_user", accountId: "5002")
            )

            // Also verify real authenticated auth.qrPairAccept RPC wire path
            let acceptResp: FabushiQrPairAcceptResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("qrPairAccept"),
                params: FabushiQrPairAcceptRequest(pairToken: "pair_tok_777")
            )
            XCTAssertTrue(acceptResp.accepted)

            vm.cancelActiveTasks()
            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F05: Cloud 2FA Gate, 2FA Management (Status/Set/Disable) & Active Device Sessions

    func testF05Cloud2FAGateManagementAndActiveDeviceSessions() async throws {
        let twoFactorEnabled = OSAllocatedUnfairLock<Bool>(initialState: true)
        let disableBodyCaptured = OSAllocatedUnfairLock<String?>(initialState: nil)

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("GET", "/api/v1/auth/kdf-params"):
                return .json(404, #"{"error":"not_found"}"#)
            case ("POST", "/api/v1/auth/login"):
                return .json(401, #"{"code":"two_factor_required","requires_2fa":true,"challenge_token":"tfa_chal_88","hint":"lotus"}"#)
            case ("POST", "/api/v1/auth/2fa/verify"):
                if req.bodyString.contains("WrongCloud2FA!") {
                    return .json(401, #"{"error":"invalid_2fa_password"}"#)
                }
                return .json(200, #"{"user_id":6001,"username":"t1_f05_2fa","session_id":"s-6001","token":"tok-6001","first_name":"TwoFA"}"#)
            case ("GET", "/api/v1/auth/2fa/status"):
                let enabled = twoFactorEnabled.withLock { $0 }
                return .json(200, #"{"has_password":\#(enabled),"hint":"lotus","recovery_email_pattern":"a***@example.com"}"#)
            case ("POST", "/api/v1/auth/2fa/set"):
                twoFactorEnabled.withLock { $0 = true }
                return .json(200, #"{"has_password":true,"hint":"orchid"}"#)
            case ("POST", "/api/v1/auth/2fa/disable"):
                disableBodyCaptured.withLock { $0 = req.bodyString }
                twoFactorEnabled.withLock { $0 = false }
                return .json(200, #"{"disabled":true}"#)
            case ("GET", "/api/v1/auth/sessions"):
                return .json(
                    200,
                    #"[{"session_id":"s-6001","device_name":"Fabushi iOS","platform":"iOS","ip_address":"127.0.0.1","created_at_ms":1700000000000,"last_active_ms":1700000010000,"is_current":true},{"session_id":"s-desktop","device_name":"MacBook Pro","platform":"macOS","ip_address":"10.0.0.2","created_at_ms":1699999000000,"last_active_ms":1700000005000,"is_current":false}]"#
                )
            case ("DELETE", "/api/v1/auth/sessions/s-desktop"):
                return .json(200, #"{"revoked":true}"#)
            case ("DELETE", "/api/v1/auth/sessions/others"):
                return .json(200, #"{"revoked_others":true}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("f05_2fa_sessions")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "6001")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)
            let vm = FabushiAuthViewModel(coordinator: coordinator, router: router, securityKeychain: secKC)
            vm.serverURLText = server.baseURLString
            vm.usernameText = "t1_f05_2fa"
            vm.passwordText = "Password2FA!1"

            // Login triggers structured 2FA challenge -> activates Cloud 2FA Gate
            await vm.submitLogin(theme: theme)
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertEqual(vm.activeTwoFactorChallenge?.challengeToken, "tfa_chal_88")
            XCTAssertEqual(vm.activeTwoFactorChallenge?.hint, "lotus")

            // Negative 2FA checks: plain 401 without challenge_token parses nil; empty & wrong password keep gate open!
            XCTAssertNil(
                FabushiTwoFactorChallengeParser.extract(
                    from: FabushiHostError.rpcError(code: "unauthorized", message: "HTTP 401 invalid credentials", retryable: false)
                )
            )
            vm.twoFactorPasswordText = ""
            await vm.submitTwoFactorVerification(theme: theme)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertNotNil(vm.activeTwoFactorChallenge)
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/auth/2fa/verify").count, 0)

            vm.twoFactorPasswordText = "WrongCloud2FA!"
            await vm.submitTwoFactorVerification(theme: theme)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertNotNil(vm.activeTwoFactorChallenge)
            XCTAssertEqual(coordinator.authState, .unauthenticated)

            // Verify Cloud 2FA password -> authenticates session
            vm.twoFactorPasswordText = "CloudSecret2FA!"
            await vm.submitTwoFactorVerification(theme: theme)
            XCTAssertNil(vm.activeTwoFactorChallenge)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 6001, username: "t1_f05_2fa", accountId: "6001")
            )

            // 2FA status / set / disable (non-empty current_password required!)
            let status: FabushiTwoFactorStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("twoFactorStatus"),
                params: FabushiEmptyParams()
            )
            XCTAssertTrue(status.hasPassword)
            XCTAssertEqual(status.hint, "lotus")

            let updated: FabushiTwoFactorStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("twoFactorSet"),
                params: FabushiTwoFactorSetRequest(
                    currentPassword: "CloudSecret2FA!",
                    newPassword: "NewCloudSecret!2",
                    hint: "orchid",
                    recoveryEmail: nil
                )
            )
            XCTAssertEqual(updated.hint, "orchid")

            let disabled: FabushiTwoFactorDisableResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("twoFactorDisable"),
                params: FabushiTwoFactorDisableRequest(currentPassword: "NewCloudSecret!2")
            )
            XCTAssertTrue(disabled.disabled)
            let disableBody = try XCTUnwrap(disableBodyCaptured.withLock { $0 })
            XCTAssertTrue(disableBody.contains("NewCloudSecret!2"))

            // Active Device Sessions list / revoke single / revoke others
            let sessions: [FabushiActiveDeviceSessionDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.session.method("list"),
                params: FabushiEmptyParams()
            )
            XCTAssertEqual(sessions.count, 2)
            XCTAssertEqual(sessions.filter(\.isCurrent).count, 1)

            struct RevokeParams: Encodable, Sendable {
                let sessionId: String
                enum CodingKeys: String, CodingKey { case sessionId = "session_id" }
            }
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.session.method("revoke"),
                params: RevokeParams(sessionId: "s-desktop")
            )
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.session.method("revokeOther"),
                params: FabushiEmptyParams()
            )
            XCTAssertEqual(server.requests(method: "DELETE", pathPrefix: "/api/v1/auth/sessions/").count, 2)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - Verbatim Accessibility ID & Module Registration Contract

    func testAuthAccessibilityIDsAndModuleRegistrationContract() {
        XCTAssertEqual(FabushiAccessibilityIDs.Auth.allIdentifiers.count, 28)
        XCTAssertEqual(
            Set(FabushiAccessibilityIDs.Auth.allIdentifiers).count,
            28,
            "All 28 Auth accessibility identifiers must be strictly unique"
        )
        XCTAssertEqual(FabushiAccessibilityIDs.Auth.root, "fabushi.auth.root")
        XCTAssertEqual(FabushiAccessibilityIDs.Auth.splitKeyBadge, "fabushi.auth.splitKeyBadge")
        XCTAssertEqual(FabushiAccessibilityIDs.Auth.twoFactorPasswordField, "fabushi.auth.twoFactorPasswordField")

        let registry = FabushiFeatureModuleRegistry()
        let authModule = FabushiAuthModule()
        registry.register(auth: authModule)
        XCTAssertTrue(registry.auth === authModule)

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: FabushiKeychainStore())
        let router = ShellNavigationRouter()
        let explicitLaunch = FabushiLaunchConfiguration(
            environment: ["GPUI_TELEGRAM_SERVER_URL": "https://auth-test.example"],
            arguments: []
        )
        let vm = FabushiAuthViewModel(
            coordinator: coordinator,
            router: router,
            launchConfiguration: explicitLaunch
        )
        XCTAssertEqual(vm.serverURLText, "https://auth-test.example")

        let fallbackVM = FabushiAuthViewModel(
            coordinator: coordinator,
            router: router,
            launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: [])
        )
        XCTAssertEqual(fallbackVM.serverURLText, coordinator.settingsStore.activeServerURL)
    }

    // MARK: - D4 & D3-Auth Regression: Emergency Kit Hold (Primary & AddAccount), Envelope Upload Rollback & Legacy Argon2id Login

    func testD4EmergencyKitBackupGateRequiresAckAndWipesMemoryAcrossRegisterAndAddAccount() async throws {
        let failRegister = OSAllocatedUnfairLock<Bool>(initialState: false)
        let failEnvelopePut = OSAllocatedUnfairLock<Bool>(initialState: false)
        let storedArgonEnvelopeJSON = OSAllocatedUnfairLock<String?>(initialState: nil)
        let storedSalt = OSAllocatedUnfairLock<String>(initialState: "")

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/register"):
                if failRegister.withLock({ $0 }) {
                    return .json(500, #"{"error":"server_error"}"#)
                }
                if req.bodyString.contains("add_acct_user") {
                    return .json(200, #"{"user_id":9102,"username":"add_acct_user","session_id":"s-9102","token":"tok-9102","first_name":"AddAcct","auth_version":2}"#)
                }
                return .json(200, #"{"user_id":9101,"username":"rollback_user","session_id":"s-9101","token":"tok-9101","first_name":"Rollback","auth_version":2}"#)
            case ("PUT", "/api/v1/user-vault/envelope"):
                if failEnvelopePut.withLock({ $0 }) {
                    return .json(500, #"{"error":"vault_storage_down"}"#)
                }
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            case ("GET", "/api/v1/auth/kdf-params"):
                let salt = storedSalt.withLock { $0 }
                return .json(
                    200,
                    #"{"auth_version":2,"auth_salt_b64":"\#(salt)","kdf_params":{"m_cost_kib":16384,"t_cost":2,"p_cost":1}}"#
                )
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":9103,"username":"argon_env_user","session_id":"s-9103","token":"tok-9103","first_name":"ArgonEnv","auth_version":2}"#)
            case ("GET", "/api/v1/user-vault/envelope"):
                if let env = storedArgonEnvelopeJSON.withLock({ $0 }) {
                    return .json(200, "{\"envelope\":\(env)}")
                }
                return .json(200, #"{"envelope":null}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("d4_emergency_kit")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            for uid in ["9101", "9102", "9103"] {
                try? coreKC.deleteAccountKey(accountId: uid)
                secKC.purgeAccountSecrets(accountId: uid)
            }
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)

            // 1. Failed registration (HTTP 500) MUST release onboarding hold so UI never deadlocks
            failRegister.withLock { $0 = true }
            let primaryVM = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                mode: .primaryGate,
                securityKeychain: secKC
            )
            primaryVM.serverURLText = server.baseURLString
            primaryVM.authTab = .register
            primaryVM.usernameText = "rollback_user"
            primaryVM.passwordText = "RollbackPass!1"
            await primaryVM.submitRegistration(theme: theme)
            XCTAssertNotNil(primaryVM.errorMessage)
            XCTAssertFalse(router.isHoldingPrimaryAuthGate)
            XCTAssertEqual(coordinator.authState, .unauthenticated)

            // 2. D3 Rollback on PUT /envelope HTTP 500: Rust vault MUST be locked via vault.lock rollback!
            failRegister.withLock { $0 = false }
            failEnvelopePut.withLock { $0 = true }
            await primaryVM.submitRegistration(theme: theme)
            XCTAssertEqual(coordinator.activeAccountId, "9101")
            XCTAssertFalse(coordinator.vaultStore.isUnlocked)
            XCTAssertFalse(primaryVM.lastCompletedVaultBootstrap)
            XCTAssertNil(primaryVM.generatedEmergencyKit)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "9101"))
            XCTAssertFalse(router.isHoldingPrimaryAuthGate)
            let rustStatusAfterFail: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("status"),
                params: FabushiEmptyParams()
            )
            XCTAssertFalse(
                rustStatusAfterFail.unlocked,
                "Failed envelopePut during registration MUST roll back Rust vault via vault.lock"
            )
            XCTAssertEqual(
                rustStatusAfterFail.hasEnvelope,
                false,
                "Failed envelopePut rollback MUST scrub orphan vault_envelope from Rust RAM (FIX-2)"
            )

            // 3. .addAccount registration preserves isAccountSwitcherPresented while holding for Emergency Kit ack
            failEnvelopePut.withLock { $0 = false }
            router.isAccountSwitcherPresented = true
            var addAccountCompleted = false
            let addVM = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                mode: .addAccount,
                securityKeychain: secKC,
                onAuthenticated: {
                    addAccountCompleted = true
                }
            )
            addVM.serverURLText = server.baseURLString
            addVM.authTab = .register
            addVM.usernameText = "add_acct_user"
            addVM.passwordText = "AddAccountPass!2"
            await addVM.submitRegistration(theme: theme)

            XCTAssertTrue(addVM.isAwaitingEmergencyKitBackup)
            XCTAssertTrue(router.isHoldingAddAccountGate)
            XCTAssertFalse(addAccountCompleted)
            // handleAccountScopeChange while held MUST NOT dismiss Account Switcher sheet!
            _ = router.handleAccountScopeChange(
                generation: coordinator.accountScopeGeneration,
                coordinator: coordinator
            )
            XCTAssertTrue(
                router.isAccountSwitcherPresented,
                "Account Switcher sheet must stay mounted while .addAccount Emergency Kit backup is held"
            )
            XCTAssertFalse(addVM.acknowledgeEmergencyKitAndContinue())
            addVM.hasAcknowledgedEmergencyKitSaved = true
            XCTAssertTrue(addVM.acknowledgeEmergencyKitAndContinue())
            XCTAssertNil(addVM.generatedEmergencyKit)
            XCTAssertFalse(router.isHoldingAddAccountGate)
            XCTAssertTrue(addAccountCompleted)

            // 4. D3-5: bootstrapVaultAfterSplitKeyLogin unlocks legacy `argon2id` envelope via unlockWithPassphrase & saves Keychain key
            _ = try await coordinator.clearLocalSession()
            struct InitNewParams: Encodable, Sendable {
                let userId: UInt64
                let passphrase: String
                let kdfParams: FabushiArgon2KdfParamsDTO
                enum CodingKeys: String, CodingKey {
                    case userId = "user_id"
                    case passphrase
                    case kdfParams = "kdf_params"
                }
            }
            struct InitNewResp: Decodable, Sendable {
                let envelope: FabushiVaultNestedEnvelopeDTO
                let rootKeyB64: String
                enum CodingKeys: String, CodingKey {
                    case envelope
                    case rootKeyB64 = "root_key_b64"
                }
            }
            let legacyInit: InitNewResp = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("initNew"),
                params: InitNewParams(
                    userId: 9103,
                    passphrase: "ArgonUserPass!9",
                    kdfParams: FabushiArgon2KdfParamsDTO(mCostKib: 16_384, tCost: 2, pCost: 1)
                )
            )
            let flatArgon = FabushiVaultEnvelopeConverter.toFlat(legacyInit.envelope)
            let flatArgonData = try JSONEncoder().encode(flatArgon)
            storedArgonEnvelopeJSON.withLock { $0 = String(decoding: flatArgonData, as: UTF8.self) }
            storedSalt.withLock { $0 = flatArgon.kdfSaltB64 }
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiEmptyParams()
            )
            coordinator.vaultStore.setUnlocked(false)

            let loginVM = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                mode: .primaryGate,
                securityKeychain: secKC
            )
            loginVM.serverURLText = server.baseURLString
            loginVM.usernameText = "argon_env_user"
            loginVM.passwordText = "ArgonUserPass!9"
            await loginVM.submitLogin(theme: theme)

            XCTAssertEqual(coordinator.activeAccountId, "9103")
            XCTAssertTrue(coordinator.vaultStore.isUnlocked)
            XCTAssertTrue(loginVM.lastCompletedVaultBootstrap)
            XCTAssertEqual(try secKC.loadVaultRootKeyBase64(accountId: "9103"), legacyInit.rootKeyB64)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - FIX-1 Regression: Legacy V1 Upgrade Non-Clobbering + Mismatched Split-Key & device_keychain_v2 Bootstrap

    func testFix1LegacyV1UpgradePreservesExistingEnvelopesAndMismatchedSplitKeyBootstrap() async throws {
        struct ScenarioState: Sendable {
            var legacyMode: Bool = true
            var activeUserId: UInt64 = 9201
            var activeUsername: String = "legacy_argon"
            var kdfSaltB64: String = ""
            var kdfTCost: UInt32 = 2
            var storedEnvelopeJSON: String? = nil
            var envelopePutCount: Int = 0
            var envelopeGetCount: Int = 0
        }
        let state = OSAllocatedUnfairLock<ScenarioState>(initialState: ScenarioState())

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("GET", "/api/v1/auth/kdf-params"):
                let s = state.withLock { $0 }
                if s.legacyMode {
                    return .json(404, #"{"error":"not_found"}"#)
                }
                return .json(
                    200,
                    #"{"auth_version":2,"auth_salt_b64":"\#(s.kdfSaltB64)","kdf_params":{"m_cost_kib":16384,"t_cost":\#(s.kdfTCost),"p_cost":1}}"#
                )
            case ("POST", "/api/v1/auth/login"):
                let s = state.withLock { $0 }
                let authVer = s.legacyMode ? 1 : 2
                return .json(
                    200,
                    #"{"user_id":\#(s.activeUserId),"username":"\#(s.activeUsername)","session_id":"s-\#(s.activeUserId)","token":"tok-\#(s.activeUserId)","first_name":"User","auth_version":\#(authVer)}"#
                )
            case ("POST", "/api/v1/auth/upgrade-split-key"):
                return .json(200, #"{"ok":true,"auth_version":2}"#)
            case ("GET", "/api/v1/user-vault/envelope"):
                let env = state.withLock { s -> String? in
                    s.envelopeGetCount += 1
                    return s.storedEnvelopeJSON
                }
                if let env {
                    return .json(200, "{\"envelope\":\(env)}")
                }
                return .json(200, #"{"envelope":null}"#)
            case ("PUT", "/api/v1/user-vault/envelope"):
                state.withLock { s in
                    s.envelopePutCount += 1
                    s.storedEnvelopeJSON = req.bodyString
                }
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("fix1_legacy_v1_non_clobber")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            for uid in ["9201", "9202", "9203", "9204"] {
                try? coreKC.deleteAccountKey(accountId: uid)
                secKC.purgeAccountSecrets(accountId: uid)
            }
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            struct InitNewParams: Encodable, Sendable {
                let userId: UInt64
                let passphrase: String
                let kdfParams: FabushiArgon2KdfParamsDTO
                enum CodingKeys: String, CodingKey {
                    case userId = "user_id"
                    case passphrase
                    case kdfParams = "kdf_params"
                }
            }
            struct InitNewResp: Decodable, Sendable {
                let envelope: FabushiVaultNestedEnvelopeDTO
                let rootKeyB64: String
                enum CodingKeys: String, CodingKey {
                    case envelope
                    case rootKeyB64 = "root_key_b64"
                }
            }

            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)
            let vm = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                mode: .primaryGate,
                securityKeychain: secKC
            )
            vm.serverURLText = server.baseURLString

            // Case A: Legacy V1 silent upgrade when server ALREADY holds an `argon2id` envelope
            let argonInit: InitNewResp = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("initNew"),
                params: InitNewParams(
                    userId: 9201,
                    passphrase: "LegacyArgon!2026",
                    kdfParams: FabushiArgon2KdfParamsDTO(mCostKib: 16_384, tCost: 2, pCost: 1)
                )
            )
            let flatArgonJSON = String(
                decoding: try JSONEncoder().encode(FabushiVaultEnvelopeConverter.toFlat(argonInit.envelope)),
                as: UTF8.self
            )
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiVaultLockRequest(clearEnvelope: true)
            )
            coordinator.vaultStore.setUnlocked(false)

            state.withLock { s in
                s.legacyMode = true
                s.activeUserId = 9201
                s.activeUsername = "legacy_argon"
                s.storedEnvelopeJSON = flatArgonJSON
                s.envelopeGetCount = 0
                s.envelopePutCount = 0
            }

            vm.usernameText = "legacy_argon"
            vm.passwordText = "LegacyArgon!2026"
            await vm.submitLogin(theme: theme)

            let countsA = state.withLock { ($0.envelopeGetCount, $0.envelopePutCount, $0.storedEnvelopeJSON) }
            XCTAssertEqual(countsA.0, 1, "Legacy V1 silent upgrade MUST issue GET /api/v1/user-vault/envelope first")
            XCTAssertEqual(countsA.1, 0, "Legacy V1 silent upgrade MUST NOT clobber existing argon2id cloud envelope")
            XCTAssertEqual(countsA.2, flatArgonJSON)
            XCTAssertTrue(coordinator.vaultStore.isUnlocked)
            XCTAssertTrue(vm.lastCompletedVaultBootstrap)
            XCTAssertEqual(try secKC.loadVaultRootKeyBase64(accountId: "9201"), argonInit.rootKeyB64)
            XCTAssertNil(router.pendingEmergencyKit, "Existing unlocked vault must not mint a replacement Emergency Kit")

            // Case B: Legacy V1 silent upgrade when server ALREADY holds a `device_keychain_v2` envelope
            _ = try await coordinator.clearLocalSession()
            let deviceKeychainEnv = #"{"key_version":2,"kdf_alg":"device_keychain_v2","kdf_salt_b64":"","kdf_m_cost_kib":65536,"kdf_t_cost":3,"kdf_p_cost":1,"wrapped_root_key_b64":"","recovery_wrapped_root_key_b64":"cmVjb3Zlcnk=","updated_at_ms":1700000000000}"#
            state.withLock { s in
                s.legacyMode = true
                s.activeUserId = 9202
                s.activeUsername = "legacy_dkc"
                s.storedEnvelopeJSON = deviceKeychainEnv
                s.envelopeGetCount = 0
                s.envelopePutCount = 0
            }

            vm.usernameText = "legacy_dkc"
            vm.passwordText = "LegacyDKC!2026"
            await vm.submitLogin(theme: theme)

            let countsB = state.withLock { ($0.envelopeGetCount, $0.envelopePutCount, $0.storedEnvelopeJSON) }
            XCTAssertEqual(countsB.0, 1)
            XCTAssertEqual(countsB.1, 0, "Legacy V1 silent upgrade MUST NOT overwrite existing device_keychain_v2 envelope")
            XCTAssertEqual(countsB.2, deviceKeychainEnv)
            XCTAssertFalse(coordinator.vaultStore.isUnlocked)
            XCTAssertFalse(vm.lastCompletedVaultBootstrap)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "9202"))
            XCTAssertNil(router.pendingEmergencyKit)

            // Case C: Split-Key V2 login with MISMATCHED envelope salt/KDF params (`paramsMatch == false` fallback to password)
            _ = try await coordinator.clearLocalSession()
            let splitInit: FabushiVaultInitSplitKeyResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("initSplitKey"),
                params: FabushiVaultInitSplitKeyRequest(
                    userId: 9203,
                    password: "MismatchSplit!99",
                    saltB64: nil,
                    kdfParams: FabushiArgon2KdfParamsDTO(mCostKib: 16_384, tCost: 2, pCost: 1),
                    fastKdf: nil
                )
            )
            let flatSplitJSON = String(
                decoding: try JSONEncoder().encode(FabushiVaultEnvelopeConverter.toFlat(splitInit.envelope)),
                as: UTF8.self
            )
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiVaultLockRequest(clearEnvelope: true)
            )
            coordinator.vaultStore.setUnlocked(false)

            // Server auth kdf-params intentionally use a DIFFERENT salt and t_cost than the stored vault envelope!
            let differentAuthSalt = Data(repeating: 0x5A, count: 16).base64EncodedString()
            XCTAssertNotEqual(differentAuthSalt, splitInit.authSaltB64)
            state.withLock { s in
                s.legacyMode = false
                s.activeUserId = 9203
                s.activeUsername = "split_mismatch"
                s.kdfSaltB64 = differentAuthSalt
                s.kdfTCost = 3
                s.storedEnvelopeJSON = flatSplitJSON
                s.envelopeGetCount = 0
                s.envelopePutCount = 0
            }

            vm.usernameText = "split_mismatch"
            vm.passwordText = "MismatchSplit!99"
            await vm.submitLogin(theme: theme)

            let countsC = state.withLock { ($0.envelopeGetCount, $0.envelopePutCount) }
            XCTAssertEqual(countsC.0, 1)
            XCTAssertEqual(countsC.1, 0)
            XCTAssertTrue(coordinator.vaultStore.isUnlocked)
            XCTAssertTrue(vm.lastCompletedVaultBootstrap)
            XCTAssertEqual(
                try secKC.loadVaultRootKeyBase64(accountId: "9203"),
                splitInit.rootKeyB64,
                "paramsMatch == false must fall back to password derivation and recover exact VaultRootKey"
            )

            // Case D: Split-Key V2 login encountering `device_keychain_v2` envelope leaves vault locked & cloud envelope intact
            _ = try await coordinator.clearLocalSession()
            state.withLock { s in
                s.legacyMode = false
                s.activeUserId = 9204
                s.activeUsername = "split_dkc"
                s.kdfSaltB64 = differentAuthSalt
                s.kdfTCost = 2
                s.storedEnvelopeJSON = deviceKeychainEnv
                s.envelopeGetCount = 0
                s.envelopePutCount = 0
            }

            vm.usernameText = "split_dkc"
            vm.passwordText = "SplitDKC!99"
            await vm.submitLogin(theme: theme)

            let countsD = state.withLock { ($0.envelopeGetCount, $0.envelopePutCount, $0.storedEnvelopeJSON) }
            XCTAssertEqual(countsD.0, 1)
            XCTAssertEqual(countsD.1, 0)
            XCTAssertEqual(countsD.2, deviceKeychainEnv)
            XCTAssertFalse(coordinator.vaultStore.isUnlocked)
            XCTAssertFalse(vm.lastCompletedVaultBootstrap)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "9204"))

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - FIX-4 Regression: .addAccount Teardown Kit Rescue, In-Flight Teardown & Unauth Hold Clearing

    func testFix4AddAccountTeardownRescuesEmergencyKitAndClearsHolds() async throws {
        let holdEnvelopePut = OSAllocatedUnfairLock<Bool>(initialState: false)
        let envelopePutArrived = OSAllocatedUnfairLock<Bool>(initialState: false)

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/register"):
                if req.bodyString.contains("inflight_add") {
                    return .json(200, #"{"user_id":9302,"username":"inflight_add","session_id":"s-9302","token":"tok-9302","first_name":"InFlight","auth_version":2}"#)
                }
                return .json(200, #"{"user_id":9301,"username":"teardown_add","session_id":"s-9301","token":"tok-9301","first_name":"Teardown","auth_version":2}"#)
            case ("PUT", "/api/v1/user-vault/envelope"):
                envelopePutArrived.withLock { $0 = true }
                let deadline = Date().addingTimeInterval(3.0)
                while holdEnvelopePut.withLock({ $0 }) && Date() < deadline {
                    Thread.sleep(forTimeInterval: 0.01)
                }
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("fix4_add_account_teardown")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            for uid in ["9301", "9302"] {
                try? coreKC.deleteAccountKey(accountId: uid)
                secKC.purgeAccountSecrets(accountId: uid)
            }
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)

            // 1. Unexpected .addAccount teardown while awaiting Emergency Kit backup rescues kit into Shell banner & releases hold
            router.isAccountSwitcherPresented = true
            let addVM = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                mode: .addAccount,
                securityKeychain: secKC
            )
            addVM.handleViewAppear()
            addVM.serverURLText = server.baseURLString
            addVM.authTab = .register
            addVM.usernameText = "teardown_add"
            addVM.passwordText = "TeardownPass!1"
            await addVM.submitRegistration(theme: theme)

            let expectedKit = try XCTUnwrap(addVM.generatedEmergencyKit)
            XCTAssertTrue(expectedKit.hasPrefix("EK-"))
            XCTAssertTrue(addVM.isAwaitingEmergencyKitBackup)
            XCTAssertTrue(router.isHoldingAddAccountGate)

            addVM.handleViewDisappear()
            XCTAssertNil(addVM.generatedEmergencyKit, "handleViewDisappear must wipe ViewModel copy after rescue")
            XCTAssertFalse(addVM.isAwaitingEmergencyKitBackup)
            XCTAssertFalse(router.isHoldingAddAccountGate, "handleViewDisappear must release .addAccount hold")
            XCTAssertEqual(
                router.pendingEmergencyKit,
                expectedKit,
                "Unexpected .addAccount teardown must rescue unacknowledged Emergency Kit into Shell banner"
            )
            router.acknowledgePendingEmergencyKit()

            // 2. In-flight teardown during registration must rescue generated kit & never leak hold
            holdEnvelopePut.withLock { $0 = true }
            envelopePutArrived.withLock { $0 = false }
            router.isAccountSwitcherPresented = true
            let inFlightVM = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                mode: .addAccount,
                securityKeychain: secKC
            )
            inFlightVM.handleViewAppear()
            inFlightVM.serverURLText = server.baseURLString
            inFlightVM.authTab = .register
            inFlightVM.usernameText = "inflight_add"
            inFlightVM.passwordText = "InFlightPass!2"

            let regTask = Task { @MainActor in
                await inFlightVM.submitRegistration(theme: theme)
            }
            let waitDeadline = Date().addingTimeInterval(5.0)
            while !envelopePutArrived.withLock({ $0 }) && Date() < waitDeadline {
                try await Task.sleep(nanoseconds: 10_000_000)
            }
            XCTAssertTrue(envelopePutArrived.withLock { $0 })
            // View disappears while PUT /envelope is still in flight
            inFlightVM.handleViewDisappear()
            holdEnvelopePut.withLock { $0 = false }
            await regTask.value

            XCTAssertNil(inFlightVM.generatedEmergencyKit)
            XCTAssertFalse(router.isHoldingAddAccountGate, "In-flight teardown must not re-hold detached ViewModel")
            XCTAssertTrue(
                router.pendingEmergencyKit?.hasPrefix("EK-") == true,
                "In-flight teardown must promote newly created Emergency Kit into Shell banner"
            )

            // 3. External .unauthenticated transition clears BOTH onboarding holds, Account Switcher & pendingEmergencyKit
            router.beginAuthOnboardingHold(isAddAccount: false)
            router.beginAuthOnboardingHold(isAddAccount: true)
            router.isAccountSwitcherPresented = true
            XCTAssertTrue(router.isHoldingPrimaryAuthGate)
            XCTAssertTrue(router.isHoldingAddAccountGate)

            let unauthGen = try await coordinator.clearLocalSession()
            XCTAssertTrue(router.handleAccountScopeChange(generation: unauthGen, coordinator: coordinator))
            XCTAssertFalse(router.isHoldingPrimaryAuthGate)
            XCTAssertFalse(router.isHoldingAddAccountGate)
            XCTAssertFalse(router.isAccountSwitcherPresented)
            XCTAssertNil(router.pendingEmergencyKit)

            await coordinator.shutdown()
        } catch {
            holdEnvelopePut.withLock { $0 = false }
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - FIX-6 Regression: Async Login Vault Bootstrap Must Not Re-Lock Passcode or Complete Stale Generation

    func testFix6SubmitLoginAsyncBootstrapDoesNotReLockPasscodeOrCompleteStaleGeneration() async throws {
        let holdEnvelopeGet = OSAllocatedUnfairLock<Bool>(initialState: false)
        let envelopeGetArrived = OSAllocatedUnfairLock<Bool>(initialState: false)
        let storedSalt = OSAllocatedUnfairLock<String>(initialState: "")
        let storedEnvelopeJSON = OSAllocatedUnfairLock<String?>(initialState: nil)

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("GET", "/api/v1/auth/kdf-params"):
                let salt = storedSalt.withLock { $0 }
                return .json(
                    200,
                    #"{"auth_version":2,"auth_salt_b64":"\#(salt)","kdf_params":{"m_cost_kib":16384,"t_cost":2,"p_cost":1}}"#
                )
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":9401,"username":"race_login_user","session_id":"s-9401","token":"tok-9401","first_name":"Race","auth_version":2}"#)
            case ("GET", "/api/v1/user-vault/envelope"):
                envelopeGetArrived.withLock { $0 = true }
                let deadline = Date().addingTimeInterval(3.0)
                while holdEnvelopeGet.withLock({ $0 }) && Date() < deadline {
                    Thread.sleep(forTimeInterval: 0.01)
                }
                if let env = storedEnvelopeJSON.withLock({ $0 }) {
                    return .json(200, "{\"envelope\":\(env)}")
                }
                return .json(200, #"{"envelope":null}"#)
            case ("PUT", "/api/v1/user-vault/envelope"):
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("fix6_login_async_race")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "9401")
            secKC.purgeAccountSecrets(accountId: "9401")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let initResp: FabushiVaultInitSplitKeyResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("initSplitKey"),
                params: FabushiVaultInitSplitKeyRequest(
                    userId: 9401,
                    password: "RaceLoginPass!1",
                    saltB64: nil,
                    kdfParams: FabushiArgon2KdfParamsDTO(mCostKib: 16_384, tCost: 2, pCost: 1),
                    fastKdf: nil
                )
            )
            let flatEnvJSON = String(
                decoding: try JSONEncoder().encode(FabushiVaultEnvelopeConverter.toFlat(initResp.envelope)),
                as: UTF8.self
            )
            storedSalt.withLock { $0 = initResp.authSaltB64 }
            storedEnvelopeJSON.withLock { $0 = flatEnvJSON }
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiVaultLockRequest(clearEnvelope: true)
            )
            coordinator.vaultStore.setUnlocked(false)

            // 1. User unlocks Passcode during async login vault bootstrap -> MUST NOT re-lock when bootstrap finishes!
            let router = ShellNavigationRouter(isPasscodeLockEnabled: true)
            router.isPasscodeLocked = false
            let theme = FabushiTheme(locale: .en)
            var callbackCount = 0
            let vm = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                mode: .primaryGate,
                securityKeychain: secKC,
                onAuthenticated: {
                    callbackCount += 1
                }
            )
            vm.serverURLText = server.baseURLString
            vm.usernameText = "race_login_user"
            vm.passwordText = "RaceLoginPass!1"

            holdEnvelopeGet.withLock { $0 = true }
            envelopeGetArrived.withLock { $0 = false }
            let loginTask1 = Task { @MainActor in
                await vm.submitLogin(theme: theme)
            }
            let deadline1 = Date().addingTimeInterval(5.0)
            while !envelopeGetArrived.withLock({ $0 }) && Date() < deadline1 {
                try await Task.sleep(nanoseconds: 10_000_000)
            }
            XCTAssertTrue(envelopeGetArrived.withLock { $0 })

            // Shell's .onChange(of: authState) fires immediately upon dispatchAuthTransition
            let loginGen1 = coordinator.accountScopeGeneration
            XCTAssertTrue(router.engagePasscodeLockIfEnabledForGeneration(loginGen1))
            XCTAssertTrue(router.isPasscodeLocked)
            // User unlocks passcode while vault bootstrap is still running
            _ = router.unlockPasscode(coordinator: coordinator)
            XCTAssertFalse(router.isPasscodeLocked)

            holdEnvelopeGet.withLock { $0 = false }
            await loginTask1.value

            XCTAssertFalse(
                router.isPasscodeLocked,
                "Post-login async vault bootstrap completion must NOT re-lock passcode after user unlocked (FIX-6)"
            )
            XCTAssertTrue(vm.lastCompletedVaultBootstrap)
            XCTAssertEqual(callbackCount, 1)

            // 2. Generation change (logout/clearLocalSession) during async bootstrap MUST abort stale completeAuthentication()!
            _ = try await coordinator.clearLocalSession()
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiVaultLockRequest(clearEnvelope: true)
            )
            callbackCount = 0
            vm.usernameText = "race_login_user"
            vm.passwordText = "RaceLoginPass!1"

            holdEnvelopeGet.withLock { $0 = true }
            envelopeGetArrived.withLock { $0 = false }
            let loginTask2 = Task { @MainActor in
                await vm.submitLogin(theme: theme)
            }
            let deadline2 = Date().addingTimeInterval(5.0)
            while !envelopeGetArrived.withLock({ $0 }) && Date() < deadline2 {
                try await Task.sleep(nanoseconds: 10_000_000)
            }
            XCTAssertTrue(envelopeGetArrived.withLock { $0 })

            // Session logs out while bootstrap is in flight
            _ = try await coordinator.clearLocalSession()
            holdEnvelopeGet.withLock { $0 = false }
            await loginTask2.value

            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertEqual(
                callbackCount,
                0,
                "Stale generation after logout during async bootstrap must NOT invoke onAuthenticatedCallback (FIX-6)"
            )
            XCTAssertNil(router.pendingEmergencyKit)

            await coordinator.shutdown()
        } catch {
            holdEnvelopeGet.withLock { $0 = false }
            await coordinator.shutdown()
            throw error
        }
    }
}
