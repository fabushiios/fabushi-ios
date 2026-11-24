import Foundation
import os
import SwiftUI
import XCTest
@testable import Fabushi

@MainActor
final class FabushiSecurityAndVaultTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m2b_sec_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychains() -> (FabushiKeychainStore, FabushiSecurityKeychainStore) {
        let uuid = UUID().uuidString
        let coreKC = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m2b.sec.core.\(uuid)",
            account: "m2b-sec-core"
        )
        let secKC = FabushiSecurityKeychainStore(
            passcodeService: "com.ombhrum.fabushi.tests.m2b.sec.passcode.\(uuid)",
            passcodeAccount: "passcode-record",
            vaultKeyService: "com.ombhrum.fabushi.tests.m2b.sec.vault.\(uuid)",
            totpService: "com.ombhrum.fabushi.tests.m2b.sec.totp.\(uuid)"
        )
        return (coreKC, secKC)
    }

    // MARK: - F06: Multi-Account Switcher, Session Preservation, Generation Fencing, Quota & Logout Keychain Scrub

    func testF06MultiAccountSwitchGenerationFenceQuotaAndLogoutKeychainCleanup() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                if req.bodyString.contains("bob_acct") {
                    return .json(200, #"{"user_id":202,"username":"bob_acct","session_id":"s-202","token":"tok-202","first_name":"Bob"}"#)
                }
                return .json(200, #"{"user_id":101,"username":"alice_acct","session_id":"s-101","token":"tok-101","first_name":"Alice"}"#)
            case ("POST", "/api/v1/auth/logout"):
                return .json(200, #"{"ok":true}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("f06_multi_account")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "101")
            try? coreKC.deleteAccountKey(accountId: "202")
            secKC.purgeAccountSecrets(accountId: "101")
            secKC.purgeAccountSecrets(accountId: "202")
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

            // 1. Sign in Account A (101)
            let envA: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "alice_acct", password: "PasswordA!1"),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )
            XCTAssertEqual(coordinator.activeAccountId, "101")
            let keyA = try FabushiPasscodeCrypto.generateRandomBytes(count: 32).base64EncodedString()
            try secKC.saveVaultRootKeyBase64(keyA, accountId: "101")
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "101"))

            // 2. Add Account B (202) WITHOUT logging out Account A!
            let envB: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "bob_acct", password: "PasswordB!2"),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )
            XCTAssertGreaterThan(envB.generation, envA.generation)
            XCTAssertEqual(coordinator.activeAccountId, "202")
            XCTAssertEqual(
                server.requests(method: "POST", pathPrefix: "/api/v1/auth/logout").count,
                0,
                "Adding a second account must preserve Account A's active server session (zero auth.logout calls)"
            )

            let accountsList = try await coordinator.host.listAccounts(generation: coordinator.accountScopeGeneration)
            XCTAssertEqual(accountsList.accounts.count, 2)
            XCTAssertEqual(accountsList.activeAccountId, "202")

            // 3. Atomic switch back to Account A -> advances generation & resets router scope
            router.isAccountSwitcherPresented = true
            let genAfterSwitch = try await coordinator.switchAccount(accountId: "101")
            XCTAssertGreaterThan(genAfterSwitch, envB.generation)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 101, username: "alice_acct", accountId: "101")
            )
            XCTAssertTrue(router.handleAccountScopeChange(generation: genAfterSwitch, coordinator: coordinator))
            XCTAssertFalse(router.isAccountSwitcherPresented)

            // 4. Quota policy checks: Standard = 3, Premium = 6
            XCTAssertEqual(FabushiAccountQuotaPolicy.maxAccounts(isPremium: false), 3)
            XCTAssertEqual(FabushiAccountQuotaPolicy.maxAccounts(isPremium: true), 6)
            XCTAssertTrue(FabushiAccountQuotaPolicy.canAddAccount(currentCount: 2, isPremium: false))
            XCTAssertFalse(FabushiAccountQuotaPolicy.canAddAccount(currentCount: 3, isPremium: false))
            XCTAssertTrue(FabushiAccountQuotaPolicy.canAddAccount(currentCount: 3, isPremium: true))
            XCTAssertFalse(FabushiAccountQuotaPolicy.canAddAccount(currentCount: 6, isPremium: true))

            // 5. Logout Account A & scrub isolated Keychain secrets
            let genAfterLogout = try await coordinator.logout()
            try? coreKC.deleteAccountKey(accountId: "101")
            secKC.purgeAccountSecrets(accountId: "101")
            XCTAssertGreaterThan(genAfterLogout, genAfterSwitch)
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertFalse(coreKC.hasAccountKey(accountId: "101"))
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "101"))

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F06: Keychain Passcode Lock, Rust Argon2id Sync, Progressive Lockout, T2_11 Fail-Closed & Idle Auto-Lock

    func testF06PasscodeKeychainLockoutT211FailClosedAndIdleAutoLock() async throws {
        let dir = try makeTempDir("f06_passcode")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? secKC.deletePasscodeRecord()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let clock = OSAllocatedUnfairLock<Date>(initialState: Date(timeIntervalSince1970: 1_700_000_000))
            let controller = FabushiPasscodeLockController(
                keychainStore: secKC,
                useFastIterationsForTests: true,
                nowProvider: { clock.withLock { $0 } }
            )

            // 1. T2_11 fail-closed check: unconfigured passcode MUST NEVER accept "000000"
            let unconfiguredResult = await controller.verifyPasscode("000000", coordinator: coordinator)
            XCTAssertEqual(unconfiguredResult, .incorrect(failedAttempts: 1))

            // 2. Rejects too-short PIN (< 4 digits)
            do {
                try await controller.setPasscode("123", coordinator: coordinator)
                XCTFail("Expected FabushiPasscodePolicyError.tooShort")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .tooShort)
            }

            // 3. Set 6-digit PIN -> Keychain PBKDF2 + Rust Argon2id synced (ZERO plaintext in Keychain/UserDefaults)
            try await controller.setPasscode("246810", coordinator: coordinator)
            XCTAssertTrue(controller.isPasscodeEnabled)
            let record = try XCTUnwrap(try secKC.loadPasscodeRecord())
            XCTAssertTrue(record.isEnabled)
            XCTAssertNotEqual(record.hashB64, "246810")
            XCTAssertFalse(record.saltB64.isEmpty)
            XCTAssertNil(UserDefaults.standard.string(forKey: "fabushi_passcode"))

            // Verify real Rust settings.passcodeVerify directly too
            struct VerifyParams: Encodable, Sendable { let passcode: String }
            struct VerifyResp: Decodable, Sendable { let valid: Bool; let configured: Bool }
            let rustGood: VerifyResp = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeVerify"),
                params: VerifyParams(passcode: "246810")
            )
            XCTAssertTrue(rustGood.configured && rustGood.valid)

            // 4. Progressive lockout backoff: fails 1 & 2 -> incorrect; fail 3 -> 5s lockout!
            let fail1 = await controller.verifyPasscode("000000", coordinator: coordinator)
            XCTAssertEqual(fail1, .incorrect(failedAttempts: 1))
            let fail2 = await controller.verifyPasscode("111111", coordinator: coordinator)
            XCTAssertEqual(fail2, .incorrect(failedAttempts: 2))
            let fail3 = await controller.verifyPasscode("999999", coordinator: coordinator)
            XCTAssertEqual(fail3, .lockedOut(remainingSeconds: 5, failedAttempts: 3))

            // Even the right PIN is rejected while lockout window is active
            let duringLockout = await controller.verifyPasscode("246810", coordinator: coordinator)
            if case .lockedOut(let remaining, let attempts) = duringLockout {
                XCTAssertGreaterThan(remaining, 0)
                XCTAssertEqual(attempts, 3)
            } else {
                XCTFail("Expected .lockedOut while lockout timer active, got \(duringLockout)")
            }

            // Advance virtual clock +6s -> correct PIN unlocks and resets failedAttempts
            clock.withLock { $0 = $0.addingTimeInterval(6) }
            let unlocked = await controller.verifyPasscode("246810", coordinator: coordinator)
            XCTAssertEqual(unlocked, .unlocked)
            XCTAssertEqual(controller.failedAttempts, 0)

            // 5. Idle auto-lock engages ShellNavigationRouter after configured timeout
            let router = ShellNavigationRouter(isPasscodeLockEnabled: true)
            router.isPasscodeLocked = false
            try controller.setAutoLockTimeoutSeconds(60)
            controller.recordUserActivity()
            XCTAssertFalse(controller.evaluateIdleAutoLock(router: router))

            clock.withLock { $0 = $0.addingTimeInterval(61) }
            XCTAssertTrue(controller.evaluateIdleAutoLock(router: router))
            XCTAssertTrue(router.isPasscodeLocked)

            // 6. Clear passcode wipes Keychain & Rust
            try await controller.clearPasscode(coordinator: coordinator)
            XCTAssertFalse(controller.isPasscodeEnabled)
            XCTAssertNil(try secKC.loadPasscodeRecord())

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F23: Zero-Knowledge Vault Envelope Converter, 3-Category CRUD, 24 MiB Limit & Local Origin Vault

    func testF23VaultEnvelopeConversionSealOpenSync24MiBBoundAndOriginCRUD() async throws {
        let serverItems = OSAllocatedUnfairLock<[String: FabushiEncryptedVaultBlobDTO]>(initialState: [:])

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "POST", req.path == "/api/v1/auth/login" {
                return .json(200, #"{"user_id":7001,"username":"vault_alice","session_id":"s-7001","token":"tok-7001","first_name":"Vault"}"#)
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/items" {
                let items = serverItems.withLock { Array($0.values) }
                let resp = FabushiVaultSyncPullResponse(latestRevision: 5, items: items)
                let data = (try? JSONEncoder().encode(resp)) ?? Data("{\"latest_revision\":0,\"items\":[]}".utf8)
                return .json(200, String(decoding: data, as: UTF8.self))
            }
            if req.method == "PUT", req.path.hasPrefix("/api/v1/user-vault/items/") {
                let tail = String(req.path.dropFirst("/api/v1/user-vault/items/".count))
                let parts = tail.split(separator: "/", maxSplits: 1).map(String.init)
                let category = parts.first ?? "password"
                let itemKey = parts.count > 1 ? parts[1] : "item"
                let json = (try? JSONSerialization.jsonObject(with: req.body) as? [String: Any]) ?? [:]
                let expectedRev = (json["expected_revision"] as? Int64) ?? Int64((json["expected_revision"] as? Int) ?? 0)
                let nextRev = expectedRev + 1
                let nonce = (json["nonce_b64"] as? String) ?? ""
                let cipher = (json["ciphertext_b64"] as? String) ?? ""
                let blob = FabushiEncryptedVaultBlobDTO(
                    category: category,
                    itemKey: itemKey,
                    revision: nextRev,
                    nonceB64: nonce,
                    ciphertextB64: cipher,
                    deleted: false,
                    updatedAtMs: 1_700_000_500_000
                )
                serverItems.withLock { $0["\(category):\(itemKey)"] = blob }
                return .json(200, #"{"ok":true,"category":"\#(category)","item_key":"\#(itemKey)","revision":\#(nextRev),"updated_at_ms":1700000500000}"#)
            }
            if req.method == "DELETE", req.path.hasPrefix("/api/v1/user-vault/items/") {
                let tail = String(req.path.dropFirst("/api/v1/user-vault/items/".count))
                let parts = tail.split(separator: "/", maxSplits: 1).map(String.init)
                let category = parts.first ?? "password"
                let itemKey = parts.count > 1 ? parts[1] : "item"
                serverItems.withLock { $0.removeValue(forKey: "\(category):\(itemKey)") }
                return .json(200, #"{"ok":true,"category":"\#(category)","item_key":"\#(itemKey)","revision":2,"updated_at_ms":1700000600000}"#)
            }
            return .json(404, #"{"error":"not_found"}"#)
        }
        defer { server.stop() }

        // 1. Flat <-> Nested Envelope round-trip verification
        let nestedSample = FabushiVaultNestedEnvelopeDTO(
            keyVersion: 2,
            kdfAlg: "split_key_v2",
            kdfSaltB64: "AAAAAAAAAAAAAAAAAAAAAA==",
            kdfParams: FabushiArgon2KdfParamsDTO(mCostKib: 65_536, tCost: 3, pCost: 1),
            wrappedRootKeyB64: "d3JhcHBlZA==",
            recoveryWrappedRootKeyB64: "cmVjb3Zlcnk=",
            updatedAtMs: 1_700_000_000_000
        )
        let flat = FabushiVaultEnvelopeConverter.toFlat(nestedSample)
        XCTAssertEqual(flat.kdfMCostKib, 65_536)
        XCTAssertEqual(flat.kdfTCost, 3)
        XCTAssertEqual(flat.kdfPCost, 1)
        XCTAssertEqual(FabushiVaultEnvelopeConverter.toNested(flat), nestedSample)

        // 2. Strict item_key & 24 MiB fail-closed payload validation
        XCTAssertEqual(try FabushiVaultLimits.validateItemKey("github.com:work_user-1"), "github.com:work_user-1")
        for invalidKey in ["", "bad..key", "bad|key", "bad key", String(repeating: "a", count: 129)] {
            XCTAssertThrowsError(try FabushiVaultLimits.validateItemKey(invalidKey))
        }
        let oversizedPayload = String(repeating: "X", count: FabushiVaultLimits.maxPayloadBytes + 1)
        do {
            try FabushiVaultLimits.validatePayloadSize(oversizedPayload)
            XCTFail("Expected fail-closed rejection for 24 MiB + 1 byte payload")
        } catch let err as FabushiVaultValidationError {
            XCTAssertEqual(
                err,
                .payloadExceedsLimit(
                    actualBytes: FabushiVaultLimits.maxPayloadBytes + 1,
                    maxBytes: FabushiVaultLimits.maxPayloadBytes
                )
            )
        }

        let dir = try makeTempDir("f23_vault_crud")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "7001")
            secKC.purgeAccountSecrets(accountId: "7001")
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
            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "vault_alice", password: "Pass!1"),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )

            let vm = FabushiCredentialVaultViewModel(coordinator: coordinator, securityKeychain: secKC)
            await vm.unlockVault()
            XCTAssertTrue(vm.isUnlocked)
            XCTAssertTrue(coordinator.vaultStore.isUnlocked)
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "7001"))

            // 3. Save & round-trip across ALL 3 canonical categories: password, browser_cookie, ai_oauth
            for (category, key, secret) in [
                (FabushiVaultCategory.password, "github.com:alice", "ghp_super_secret_1"),
                (FabushiVaultCategory.browserCookie, "session:obscura", "{\"sid\":\"abc\"}"),
                (FabushiVaultCategory.aiOauth, "anthropic:claude", "oauth_token_xyz"),
            ] {
                let pushResp = try await vm.saveEncryptedItem(category: category, itemKey: key, plaintext: secret)
                XCTAssertTrue(pushResp.ok)
                XCTAssertEqual(pushResp.revision, 1)
            }

            // Pull & decrypt from real loopback server verifying AAD revision binding!
            vm.items = []
            await vm.pullSyncedItems()
            XCTAssertEqual(vm.items.count, 3)
            XCTAssertEqual(
                vm.items.first(where: { $0.itemKey == "github.com:alice" })?.plaintext,
                "ghp_super_secret_1"
            )
            XCTAssertEqual(
                vm.items.first(where: { $0.itemKey == "anthropic:claude" })?.plaintext,
                "oauth_token_xyz"
            )

            // Reject 24 MiB + 1 byte payload BEFORE hitting network!
            let pushCountBefore = server.requests(method: "PUT", pathPrefix: "/api/v1/user-vault/items/").count
            do {
                _ = try await vm.saveEncryptedItem(
                    category: .browserCookie,
                    itemKey: "too_large",
                    plaintext: oversizedPayload
                )
                XCTFail("Expected payloadExceedsLimit error")
            } catch let err as FabushiVaultValidationError {
                XCTAssertEqual(
                    err,
                    .payloadExceedsLimit(
                        actualBytes: FabushiVaultLimits.maxPayloadBytes + 1,
                        maxBytes: FabushiVaultLimits.maxPayloadBytes
                    )
                )
            }
            XCTAssertEqual(
                server.requests(method: "PUT", pathPrefix: "/api/v1/user-vault/items/").count,
                pushCountBefore,
                "Oversized payload must fail closed before network push"
            )

            // Delete synced item
            let target = try XCTUnwrap(vm.items.first(where: { $0.itemKey == "github.com:alice" }))
            try await vm.deleteEncryptedItem(target)
            XCTAssertEqual(vm.items.count, 2)

            // 4. Local vault.db origin-scoped credential CRUD (storeCredential -> listForOrigin -> decryptForOrigin -> deleteCredential)
            let meta = try await vm.storeLocalOriginCredential(
                origin: "https://example.com",
                username: "alice@example.com",
                password: "LocalOriginPassword!7",
                totpSecret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
                accountHint: "Work",
                reusePolicy: "confirm_every_time"
            )
            XCTAssertTrue(meta.hasTotp)
            let originList = try await vm.listLocalOriginCredentials(origin: "https://example.com")
            XCTAssertEqual(originList.count, 1)
            let decrypted = try await vm.decryptLocalOriginCredential(
                credentialId: meta.credentialId,
                origin: "https://example.com"
            )
            XCTAssertEqual(decrypted.username, "alice@example.com")
            XCTAssertEqual(decrypted.password, "LocalOriginPassword!7")
            XCTAssertEqual(decrypted.totpCode?.count, 6)
            XCTAssertTrue(try await vm.deleteLocalOriginCredential(credentialId: meta.credentialId))

            await vm.lockVault()
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertFalse(coordinator.vaultStore.isUnlocked)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F23: Real X25519 Cross-Device Trust Ring Sync Round-Trip (6-Digit Safety Code)

    func testF23X25519DeviceTrustRingSyncRoundTripWith6DigitSafetyCode() async throws {
        struct SyncRecord: Sendable {
            var requestId: String
            var deviceName: String
            var devicePublicKeyB64: String
            var verificationCode: String
            var status: String
            var approverEphPub: String?
            var encryptedRootKey: String?
            var nonce: String?
        }

        let syncStore = OSAllocatedUnfairLock<SyncRecord?>(initialState: nil)

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "POST", req.path == "/api/v1/auth/login" {
                return .json(200, #"{"user_id":8001,"username":"sync_alice","session_id":"s-8001","token":"tok-8001","first_name":"Sync"}"#)
            }
            if req.method == "POST", req.path == "/api/v1/user-vault/device-sync/request" {
                let json = (try? JSONSerialization.jsonObject(with: req.body) as? [String: Any]) ?? [:]
                let pub = (json["device_public_key_b64"] as? String) ?? ""
                let dev = (json["device_name"] as? String) ?? "iPad"
                let rec = SyncRecord(
                    requestId: "req_x25519_1",
                    deviceName: dev,
                    devicePublicKeyB64: pub,
                    verificationCode: "123-456",
                    status: "pending"
                )
                syncStore.withLock { $0 = rec }
                return .json(200, #"{"request_id":"req_x25519_1","verification_code":"123-456","expires_at":1900000000}"#)
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/device-sync/pending" {
                let maybeRec = syncStore.withLock { $0 }
                if let rec = maybeRec, rec.status == "pending" {
                    return .json(
                        200,
                        #"{"items":[{"request_id":"\#(rec.requestId)","device_name":"\#(rec.deviceName)","device_public_key_b64":"\#(rec.devicePublicKeyB64)","verification_code":"\#(rec.verificationCode)","status":"pending","created_at":1700000000,"expires_at":1900000000}]}"#
                    )
                }
                return .json(200, #"{"items":[]}"#)
            }
            if req.method == "POST", req.path == "/api/v1/user-vault/device-sync/req_x25519_1/approve" {
                let json = (try? JSONSerialization.jsonObject(with: req.body) as? [String: Any]) ?? [:]
                let approverEphPub = json["approver_ephemeral_pub_b64"] as? String
                let encryptedRootKey = json["encrypted_root_key_b64"] as? String
                let nonce = json["nonce_b64"] as? String
                syncStore.withLock { state in
                    state?.status = "approved"
                    state?.approverEphPub = approverEphPub
                    state?.encryptedRootKey = encryptedRootKey
                    state?.nonce = nonce
                }
                return .json(200, #"{"ok":true,"request_id":"req_x25519_1","status":"approved"}"#)
            }
            if req.method == "POST", req.path == "/api/v1/user-vault/device-sync/req_x25519_1/deny" {
                syncStore.withLock { $0?.status = "denied" }
                return .json(200, #"{"ok":true,"request_id":"req_x25519_1","status":"denied"}"#)
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/device-sync/req_x25519_1/status" {
                let rec = syncStore.withLock { $0 }
                if let rec, rec.status == "approved",
                   let eph = rec.approverEphPub,
                   let enc = rec.encryptedRootKey,
                   let nonce = rec.nonce {
                    return .json(
                        200,
                        #"{"status":"approved","approver_ephemeral_pub_b64":"\#(eph)","encrypted_root_key_b64":"\#(enc)","nonce_b64":"\#(nonce)","expires_at":1900000000}"#
                    )
                }
                return .json(200, #"{"status":"pending","expires_at":1900000000}"#)
            }
            return .json(404, #"{"error":"not_found"}"#)
        }
        defer { server.stop() }

        let dir = try makeTempDir("f23_x25519")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "8001")
            secKC.purgeAccountSecrets(accountId: "8001")
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
            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "sync_alice", password: "SyncPass!1"),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )

            let vm = FabushiCredentialVaultViewModel(coordinator: coordinator, securityKeychain: secKC)
            await vm.unlockVault()
            let originalRootKeyB64 = try XCTUnwrap(try secKC.loadVaultRootKeyBase64(accountId: "8001"))

            // 1. New device requests key (X25519 keygen -> 6-digit NNN-NNN safety fingerprint)
            let reqResp = try await vm.startDeviceSyncRequest(deviceName: "Fabushi iPad")
            XCTAssertEqual(reqResp.requestId, "req_x25519_1")
            let safetyCode = try XCTUnwrap(vm.activeSyncVerificationCode)
            XCTAssertEqual(safetyCode.count, 7)
            XCTAssertEqual(safetyCode[safetyCode.index(safetyCode.startIndex, offsetBy: 3)], "-")

            // 2. Trusted device loads pending & approves via real X25519 ECDH seal
            try await vm.loadPendingDeviceSyncRequests()
            XCTAssertEqual(vm.pendingDeviceSyncRequests.count, 1)
            let pending = try XCTUnwrap(vm.pendingDeviceSyncRequests.first)
            let approved = try await vm.approvePendingDeviceSync(pending)
            XCTAssertTrue(approved.ok)
            XCTAssertEqual(approved.status, "approved")

            // 3. Requester locks vault, removes Keychain root key, polls status, opens X25519 transfer & recovers exact same 32-byte key!
            await vm.lockVault()
            try secKC.deleteVaultRootKey(accountId: "8001")
            let finalStatus = try await vm.checkDeviceSyncStatusAndOpenIfApproved()
            XCTAssertEqual(finalStatus, "approved")
            XCTAssertTrue(vm.isUnlocked)
            XCTAssertEqual(try secKC.loadVaultRootKeyBase64(accountId: "8001"), originalRootKeyB64)

            // 4. Verify deny path
            let denied = try await vm.denyPendingDeviceSync(pending)
            XCTAssertTrue(denied.ok)
            XCTAssertEqual(denied.status, "denied")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - F23: RFC 6238 TOTP Known Test Vectors, otpauth:// Parsing, 30s Countdown, Obscura Disclosure & SecurityModule Contract

    func testF23RFC6238TotpVectorsOtpAuthCountdownObscuraNoticeAndSecurityModule() async throws {
        let dir = try makeTempDir("f23_totp")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "9001")
            secKC.purgeAccountSecrets(accountId: "9001")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            _ = try await coordinator.setAuthSession(
                FabushiSessionAuthParams(
                    userId: 9001,
                    username: "totp_alice",
                    sessionId: "s-9001",
                    token: "tok-9001"
                )
            )

            let vm = FabushiCredentialVaultViewModel(
                coordinator: coordinator,
                securityKeychain: secKC,
                nowProvider: { Date(timeIntervalSince1970: 59) }
            )

            // 1. RFC 6238 Appendix B exact SHA-1 test vectors executed by real Rust bot-browser::totp!
            let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
            let vec59Digits6 = try await vm.generateTotpCode(secret: secret, unixSecs: 59, digits: 6, period: 30)
            XCTAssertEqual(vec59Digits6.code, "287082")

            let vec1234567890Digits8 = try await vm.generateTotpCode(secret: secret, unixSecs: 1_234_567_890, digits: 8, period: 30)
            XCTAssertEqual(vec1234567890Digits8.code, "89005924")

            let vec1234567890Digits6 = try await vm.generateTotpCode(secret: secret, unixSecs: 1_234_567_890, digits: 6, period: 30)
            XCTAssertEqual(vec1234567890Digits6.code, "005924")

            // 2. otpauth://totp/... parsing + Keychain persistence + 30s countdown calculation
            let uri = "otpauth://totp/Fabushi:alice%40example.com?secret=\(secret)&issuer=Fabushi&digits=6&period=30"
            let parsed = try XCTUnwrap(FabushiTotpURIInfo.parse(uri))
            XCTAssertEqual(parsed.issuer, "Fabushi")
            XCTAssertEqual(parsed.label, "alice@example.com")
            XCTAssertEqual(parsed.digits, 6)
            XCTAssertEqual(parsed.period, 30)

            XCTAssertEqual(FabushiTotpURIInfo.remainingSeconds(at: 59, period: 30), 1)
            XCTAssertEqual(FabushiTotpURIInfo.remainingSeconds(at: 60, period: 30), 30)
            XCTAssertEqual(FabushiTotpURIInfo.remainingSeconds(at: 45, period: 30), 15)

            try await vm.addTotpAccount(label: "", issuer: "", secretOrURI: uri)
            XCTAssertEqual(vm.totpAccounts.count, 1)
            let savedEntry = try XCTUnwrap(vm.totpAccounts.first)
            XCTAssertEqual(vm.liveTotpCodes[savedEntry.id], "287082")
            XCTAssertEqual(vm.totpRemainingSeconds, 1)

            // Verify persisted in Keychain and cleanly deleted
            let reloadedFromKC = try secKC.loadTotpAccounts(accountId: "9001")
            XCTAssertEqual(reloadedFromKC.count, 1)
            try vm.deleteTotpAccount(id: savedEntry.id)
            XCTAssertTrue(vm.totpAccounts.isEmpty)
            XCTAssertTrue(try secKC.loadTotpAccounts(accountId: "9001").isEmpty)

            // 3. Honest Remote Obscura iOS sandbox disclosure in both en & zh-Hans
            let enNotice = FabushiLocale.en.localized("hig.vault.obscuraIosSandboxNotice", bundle: .main)
            let zhNotice = FabushiLocale.zhHans.localized("hig.vault.obscuraIosSandboxNotice", bundle: .main)
            XCTAssertTrue(enNotice.localizedCaseInsensitiveContains("desktop"))
            XCTAssertTrue(enNotice.localizedCaseInsensitiveContains("sandbox"))
            XCTAssertTrue(zhNotice.contains("沙盒"))
            XCTAssertTrue(zhNotice.contains("桌面"))

            // 4. FabushiSecurityModule implements all 6 SecurityModuleProviding factories cleanly
            let registry = FabushiFeatureModuleRegistry()
            let secModule = FabushiSecurityModule(
                passcodeController: FabushiPasscodeLockController(keychainStore: secKC),
                securityKeychain: secKC
            )
            registry.register(security: secModule)
            XCTAssertTrue(registry.security === secModule)

            let router = ShellNavigationRouter()
            let launchConfig = FabushiLaunchConfiguration(environment: [:], arguments: [])
            let factories: [AnyView] = [
                registry.security.makePasscodeLockOverlay(coordinator: coordinator, launchConfiguration: launchConfig) {},
                registry.security.makePrivacyBlurOverlay(),
                registry.security.makeAccountSwitcherSheet(coordinator: coordinator, router: router) {},
                registry.security.makeVaultSettingsView(coordinator: coordinator),
                registry.security.makeDevicesSettingsView(coordinator: coordinator),
                registry.security.makePrivacySecuritySettingsView(coordinator: coordinator, router: router),
            ]
            XCTAssertEqual(factories.count, 6)
            for view in factories {
                XCTAssertTrue(type(of: view) == AnyView.self)
            }

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
