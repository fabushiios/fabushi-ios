import Foundation
import os
import SwiftUI
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
            let router = ShellNavigationRouter()
            let theme = FabushiTheme(locale: .en)
            let vm = FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: []),
                mode: .primaryGate,
                securityKeychain: secKC,
                useFastKdfForTests: true
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
            XCTAssertTrue(vm.generatedEmergencyKit?.hasPrefix("EK-") == true)
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "2001"))

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
                    #"{"auth_version":2,"auth_salt_b64":"\#(salt)","kdf_params":{"m_cost_kib":1024,"t_cost":1,"p_cost":1}}"#
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
            // Prepare a genuine Rust-generated Split-Key V2 envelope for user 2002
            let initResp: FabushiVaultInitSplitKeyResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("initSplitKey"),
                params: FabushiVaultInitSplitKeyRequest(
                    userId: 2002,
                    password: "MyV2Password!99",
                    saltB64: nil,
                    kdfParams: nil,
                    fastKdf: true
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
                securityKeychain: secKC,
                useFastKdfForTests: true
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

            // 3. Legacy V1 fallback + silent Split-Key V2 upgrade
            legacyMode.withLock { $0 = true }
            _ = try await coordinator.clearLocalSession()
            vm.usernameText = "legacy_v1"
            vm.passwordText = "LegacyPass!42"
            await vm.submitLogin(theme: theme)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 2003, username: "legacy_v1", accountId: "2003")
            )
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/auth/upgrade-split-key").count, 1)

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
                securityKeychain: secKC,
                useFastKdfForTests: true
            )
            vm.serverURLText = server.baseURLString

            // Invalid phone fails closed without hitting server
            vm.otpChannel = .phone
            vm.phoneText = "123"
            await vm.sendOTP(theme: theme)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertFalse(vm.otpCodeSent)

            // Valid E.164 phone SMS send -> verify
            vm.phoneText = "+14155550168"
            await vm.sendOTP(theme: theme)
            XCTAssertNil(vm.errorMessage)
            XCTAssertTrue(vm.otpCodeSent)
            XCTAssertEqual(vm.phoneCodeHash, "hash-99")
            XCTAssertGreaterThan(vm.otpResendRemainingSeconds, 0)

            vm.otpCodeText = "123456"
            await vm.verifyOTP(theme: theme)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 3001, username: "phone_user", accountId: "3001")
            )
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/auth/phone").count, 2)

            // Invalid email fails closed
            _ = try await coordinator.clearLocalSession()
            vm.otpChannel = .email
            vm.emailText = "bad-email"
            await vm.sendOTP(theme: theme)
            XCTAssertNotNil(vm.errorMessage)

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

            // Email Split-Key V2 Registration (zero plaintext password on wire)
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
                return .json(200, #"{"challenge":"dGVzdC1jaGFsbGVuZ2UtMzJi","rp_id":"fabushi.app","timeout_ms":60000,"user_id":5001}"#)
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
            // 1. Real Rust auth.passkeyChallenge wire round-trip + base64/base64url decode
            let challenge: FabushiPasskeyChallengeDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyChallenge"),
                params: FabushiPasskeyChallengeRequest(username: "alice")
            )
            XCTAssertEqual(challenge.rpId, "fabushi.app")
            XCTAssertEqual(challenge.userId, 5001)
            XCTAssertNotNil(FabushiWebAuthnPasskeyController.decodeChallengeData(challenge.challenge))
            XCTAssertNotNil(FabushiWebAuthnPasskeyController.decodeChallengeData("YWJjLV8"))

            // 2. Real QR Pair Init -> CoreImage QR -> URI token parser -> Accept -> Poll completion
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
        XCTAssertEqual(FabushiAccessibilityIDs.Auth.root, "fabushi.auth.root")
        XCTAssertEqual(FabushiAccessibilityIDs.Auth.splitKeyBadge, "fabushi.auth.splitKeyBadge")
        XCTAssertEqual(FabushiAccessibilityIDs.Auth.twoFactorPasswordField, "fabushi.auth.twoFactorPasswordField")

        let registry = FabushiFeatureModuleRegistry()
        let authModule = FabushiAuthModule()
        registry.register(auth: authModule)
        XCTAssertTrue(registry.auth === authModule)

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: FabushiKeychainStore())
        let router = ShellNavigationRouter()
        let view = registry.auth.makeAuthGateView(
            coordinator: coordinator,
            router: router,
            launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: [])
        )
        XCTAssertTrue(type(of: view) == AnyView.self)
    }
}
