import Foundation
import os
import Security
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif
import UniformTypeIdentifiers
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
            FabushiSecurityModule(securityKeychain: secKC).installLogoutSecretsPurger(on: coordinator)

            // 1. Sign in Account A (101) & seed VaultRootKey + TOTP + unlock Rust vault
            let envA: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "alice_acct", password: "PasswordA!1"),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )
            XCTAssertEqual(coordinator.activeAccountId, "101")
            let keyA = try FabushiPasscodeCrypto.generateRandomBytes(count: 32).base64EncodedString()
            try secKC.saveVaultRootKeyBase64(keyA, accountId: "101")
            try secKC.saveTotpAccounts(
                [FabushiStoredTotpAccount(label: "alice@ex", issuer: "Fabushi", secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")],
                accountId: "101"
            )
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "101"))

            // 2. Add Account B (202) WITHOUT logging out Account A!
            let envB: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "bob_acct", password: "PasswordB!2"),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )
            XCTAssertGreaterThan(envB.generation, envA.generation)
            XCTAssertEqual(coordinator.activeAccountId, "202")
            let keyB = try FabushiPasscodeCrypto.generateRandomBytes(count: 32).base64EncodedString()
            try secKC.saveVaultRootKeyBase64(keyB, accountId: "202")
            try secKC.saveTotpAccounts(
                [FabushiStoredTotpAccount(label: "bob@ex", issuer: "Fabushi", secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")],
                accountId: "202"
            )
            let _: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("unlockWithKeyBytes"),
                params: FabushiVaultUnlockWithKeyBytesRequest(rootKeyB64: keyB)
            )
            XCTAssertEqual(
                server.requests(method: "POST", pathPrefix: "/api/v1/auth/logout").count,
                0,
                "Adding a second account must preserve Account A's active server session (zero auth.logout calls)"
            )

            let accountsList = try await coordinator.host.listAccounts(generation: coordinator.accountScopeGeneration)
            XCTAssertEqual(accountsList.accounts.count, 2)
            XCTAssertEqual(accountsList.activeAccountId, "202")

            // 3. Atomic switch back to Account A -> advances generation, locks Rust vault & resets router scope
            router.isAccountSwitcherPresented = true
            let genAfterSwitch = try await coordinator.switchAccount(accountId: "101")
            XCTAssertGreaterThan(genAfterSwitch, envB.generation)
            XCTAssertEqual(
                coordinator.authState,
                .authenticated(userId: 101, username: "alice_acct", accountId: "101")
            )
            let rustStatusAfterSwitch: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("status"),
                params: FabushiEmptyParams()
            )
            XCTAssertFalse(
                rustStatusAfterSwitch.unlocked,
                "Switching from Account B to Account A must lock Rust vault in-memory key"
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

            // 5. D7: coordinator.logout() ITSELF scrubs Account A's Keychain secrets while preserving Account B!
            let genAfterLogout = try await coordinator.logout()
            XCTAssertGreaterThan(genAfterLogout, genAfterSwitch)
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertFalse(try coreKC.hasAccountKey(accountId: "101"))
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "101"))
            XCTAssertTrue(try secKC.loadTotpAccounts(accountId: "101").isEmpty)
            // Account B remains untouched
            XCTAssertTrue(try coreKC.hasAccountKey(accountId: "202"))
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "202"))
            XCTAssertEqual(try secKC.loadTotpAccounts(accountId: "202").count, 1)

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

            // 6. D5: Clear passcode rejects missing re-auth, then authorized clear wipes Keychain & Rust
            do {
                try await controller.clearPasscode(coordinator: coordinator)
                XCTFail("Expected FabushiPasscodePolicyError.reauthenticationRequired")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .reauthenticationRequired)
            }
            XCTAssertTrue(controller.isPasscodeEnabled)

            try await controller.clearPasscode(
                authorization: .currentPasscode("246810"),
                coordinator: coordinator
            )
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
        let storedEnvelopeJSON = OSAllocatedUnfairLock<String?>(initialState: nil)

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "POST", req.path == "/api/v1/auth/login" {
                return .json(200, #"{"user_id":7001,"username":"vault_alice","session_id":"s-7001","token":"tok-7001","first_name":"Vault"}"#)
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/envelope" {
                if let env = storedEnvelopeJSON.withLock({ $0 }) {
                    return .json(200, "{\"envelope\":\(env)}")
                }
                return .json(200, #"{"envelope":null}"#)
            }
            if req.method == "PUT", req.path == "/api/v1/user-vault/envelope" {
                storedEnvelopeJSON.withLock { $0 = req.bodyString }
                return .json(200, "{\"envelope\":\(req.bodyString)}")
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
                let category = (parts.first ?? "password").removingPercentEncoding ?? (parts.first ?? "password")
                let rawKey = parts.count > 1 ? parts[1] : "item"
                let itemKey = rawKey.removingPercentEncoding ?? rawKey
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
                let category = (parts.first ?? "password").removingPercentEncoding ?? (parts.first ?? "password")
                let rawKey = parts.count > 1 ? parts[1] : "item"
                let itemKey = rawKey.removingPercentEncoding ?? rawKey
                _ = serverItems.withLock { $0.removeValue(forKey: "\(category):\(itemKey)") }
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

            let vm = FabushiCredentialVaultViewModel(
                coordinator: coordinator,
                securityKeychain: secKC
            )
            await vm.unlockVault(passphraseOrPassword: "VaultMasterPass!7")
            XCTAssertTrue(vm.isUnlocked)
            XCTAssertTrue(coordinator.vaultStore.isUnlocked)
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "7001"))
            XCTAssertTrue(vm.pendingEmergencyKit?.hasPrefix("EK-") == true)
            vm.acknowledgeEmergencyKit()
            XCTAssertNil(vm.pendingEmergencyKit)
            XCTAssertEqual(server.requests(method: "PUT", pathPrefix: "/api/v1/user-vault/envelope").count, 1)

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
            // Origin mismatch MUST fail closed on decryptLocalOriginCredential
            do {
                _ = try await vm.decryptLocalOriginCredential(
                    credentialId: meta.credentialId,
                    origin: "https://evil.example"
                )
                XCTFail("Expected origin mismatch failure")
            } catch {
                // Expected fail-closed rejection
            }

            let deletedOriginCredential = try await vm.deleteLocalOriginCredential(credentialId: meta.credentialId)
            XCTAssertTrue(deletedOriginCredential)

            await vm.lockVault()
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertFalse(coordinator.vaultStore.isUnlocked)
            XCTAssertTrue(vm.totpAccounts.isEmpty)
            let rustLockedStatus: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("status"),
                params: FabushiEmptyParams()
            )
            XCTAssertFalse(rustLockedStatus.unlocked)

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
        let storedEnvelopeJSON = OSAllocatedUnfairLock<String?>(initialState: nil)

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "POST", req.path == "/api/v1/auth/login" {
                return .json(200, #"{"user_id":8001,"username":"sync_alice","session_id":"s-8001","token":"tok-8001","first_name":"Sync"}"#)
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/envelope" {
                if let env = storedEnvelopeJSON.withLock({ $0 }) {
                    return .json(200, "{\"envelope\":\(env)}")
                }
                return .json(200, #"{"envelope":null}"#)
            }
            if req.method == "PUT", req.path == "/api/v1/user-vault/envelope" {
                storedEnvelopeJSON.withLock { $0 = req.bodyString }
                return .json(200, "{\"envelope\":\(req.bodyString)}")
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

            let vm = FabushiCredentialVaultViewModel(
                coordinator: coordinator,
                securityKeychain: secKC
            )
            await vm.unlockVault(passphraseOrPassword: "SyncVaultPass!1")
            XCTAssertTrue(vm.isUnlocked)
            vm.acknowledgeEmergencyKit()
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

            // 3. Requester locks vault & removes Keychain root key:
            //    (a) Locked approver MUST fail closed with .vaultLocked
            //    (b) Bare unlockVault() MUST fail closed with .existingRemoteEnvelopeRequiresCredentials (ZERO random key!)
            //    (c) X25519 device sync open recovers the exact same 32-byte key!
            await vm.lockVault()
            try secKC.deleteVaultRootKey(accountId: "8001")
            do {
                _ = try await vm.approvePendingDeviceSync(pending)
                XCTFail("Expected FabushiVaultValidationError.vaultLocked on locked approver")
            } catch let err as FabushiVaultValidationError {
                XCTAssertEqual(err, .vaultLocked)
            }
            await vm.unlockVault()
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertEqual(vm.unlockPromptReason, .existingRemoteEnvelopeRequiresCredentials)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "8001"))

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

            // Negative TOTP URI & invalid Base32 checks leave Keychain clean
            XCTAssertNil(FabushiTotpURIInfo.parse("otpauth://hotp/Fabushi:alice?secret=\(secret)"))
            XCTAssertNil(FabushiTotpURIInfo.parse("otpauth://totp/Fabushi:alice?issuer=Fabushi"))
            XCTAssertNil(FabushiTotpURIInfo.parse("   "))
            do {
                try await vm.addTotpAccount(label: "bad", issuer: "Test", secretOrURI: "!!!INVALID_BASE32!!!")
                XCTFail("Expected invalid Base32 secret to fail closed")
            } catch {
                XCTAssertTrue(try secKC.loadTotpAccounts(accountId: "9001").isEmpty)
            }

            // 3. Honest Remote Obscura iOS sandbox disclosure in both en & zh-Hans
            let enNotice = FabushiLocale.en.localized("hig.vault.obscuraIosSandboxNotice", bundle: .main)
            let zhNotice = FabushiLocale.zhHans.localized("hig.vault.obscuraIosSandboxNotice", bundle: .main)
            XCTAssertTrue(enNotice.localizedCaseInsensitiveContains("desktop"))
            XCTAssertTrue(enNotice.localizedCaseInsensitiveContains("sandbox"))
            XCTAssertTrue(zhNotice.contains("沙盒"))
            XCTAssertTrue(zhNotice.contains("桌面"))

            // 4. FabushiSecurityModule registration & logout purger wiring contract
            let registry = FabushiFeatureModuleRegistry()
            let secModule = FabushiSecurityModule(
                passcodeController: FabushiPasscodeLockController(keychainStore: secKC),
                securityKeychain: secKC
            )
            registry.register(security: secModule)
            XCTAssertTrue(registry.security === secModule)
            let router = ShellNavigationRouter()
            _ = registry.security.makeAccountSwitcherSheet(coordinator: coordinator, router: router) {}
            XCTAssertEqual(FabushiAccessibilityIDs.Settings.vaultUnlockButton, "fabushi.settings.vaultUnlockButton")
            XCTAssertEqual(FabushiAccessibilityIDs.Settings.vaultTotpCodeLabel, "fabushi.settings.vaultTotpCodeLabel")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - D1 Regression: Passive UIWindow Recognizer, Throttling & Typing Activity Idle Prevention

    func testD1UserActivityObserverWindowRecognizerThrottlingAndIdlePrevention() async throws {
        let (_, secKC) = makeIsolatedKeychains()
        defer { try? secKC.deletePasscodeRecord() }

        let clock = OSAllocatedUnfairLock<Date>(initialState: Date(timeIntervalSince1970: 1_700_000_000))
        let controller = FabushiPasscodeLockController(
            keychainStore: secKC,
            useFastIterationsForTests: true,
            nowProvider: { clock.withLock { $0 } }
        )
        try await controller.setPasscode("246810", coordinator: nil)
        try controller.setAutoLockTimeoutSeconds(60)

        let router = ShellNavigationRouter(isPasscodeLockEnabled: true)
        router.isPasscodeLocked = false

        // Throttling verification
        clock.withLock { $0 = $0.addingTimeInterval(0.2) }
        XCTAssertFalse(controller.recordUserActivityIfNeeded(minimumInterval: 1.0))
        clock.withLock { $0 = $0.addingTimeInterval(1.0) }
        XCTAssertTrue(controller.recordUserActivityIfNeeded(minimumInterval: 1.0))
        let baselineActivity = controller.lastActivityDate

        #if canImport(UIKit)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        let observerCoord = FabushiUserActivityObserverView.Coordinator {
            _ = controller.recordUserActivityIfNeeded(minimumInterval: 1.0)
        }
        let anchor = FabushiUserActivityObserverView.AnchorView(frame: .zero)
        anchor.onWindowChange = { attachedWin in
            observerCoord.attach(to: attachedWin)
        }
        window.addSubview(anchor)
        defer { observerCoord.detach() }

        let recognizers = window.gestureRecognizers ?? []
        let attached = try XCTUnwrap(
            recognizers.first(where: { $0 is FabushiActivityObservationGestureRecognizer })
                as? FabushiActivityObservationGestureRecognizer,
            "FabushiActivityObservationGestureRecognizer must attach to hosting UIWindow"
        )
        XCTAssertFalse(attached.cancelsTouchesInView)
        XCTAssertFalse(attached.delaysTouchesBegan)
        XCTAssertFalse(attached.delaysTouchesEnded)
        XCTAssertFalse(attached.canPrevent(UITapGestureRecognizer()))
        XCTAssertFalse(attached.canBePrevented(by: UITapGestureRecognizer()))

        // FIX-9: Direct touchesBegan / touchesMoved / touchesEnded / pressesBegan + repeated attach idempotency + window swap
        clock.withLock { $0 = $0.addingTimeInterval(2.0) }
        attached.touchesBegan(Set(), with: UIEvent())
        XCTAssertGreaterThan(controller.lastActivityDate, baselineActivity)

        clock.withLock { $0 = $0.addingTimeInterval(2.0) }
        let beforeMoved = controller.lastActivityDate
        attached.touchesMoved(Set(), with: UIEvent())
        XCTAssertGreaterThan(controller.lastActivityDate, beforeMoved)

        clock.withLock { $0 = $0.addingTimeInterval(2.0) }
        let beforeEnded = controller.lastActivityDate
        attached.touchesEnded(Set(), with: UIEvent())
        XCTAssertGreaterThan(controller.lastActivityDate, beforeEnded)
        XCTAssertEqual(attached.state, .failed, "Passive observer must fail cleanly on touchesEnded without claiming gestures")

        clock.withLock { $0 = $0.addingTimeInterval(2.0) }
        let beforePress = controller.lastActivityDate
        attached.pressesBegan(Set(), with: UIPressesEvent())
        XCTAssertGreaterThan(controller.lastActivityDate, beforePress)

        // Repeated attach(to: sameWindow) must be strictly idempotent (single recognizer instance)
        observerCoord.attach(to: window)
        observerCoord.attach(to: window)
        let matchingAfterRepeated = (window.gestureRecognizers ?? []).filter {
            $0 is FabushiActivityObservationGestureRecognizer
        }
        XCTAssertEqual(matchingAfterRepeated.count, 1)
        XCTAssertTrue(matchingAfterRepeated.first === attached)

        // Window swap moves recognizer cleanly from window1 -> window2
        let window2 = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        observerCoord.attach(to: window2)
        XCTAssertFalse(
            (window.gestureRecognizers ?? []).contains(where: { $0 is FabushiActivityObservationGestureRecognizer })
        )
        XCTAssertTrue(
            (window2.gestureRecognizers ?? []).contains(where: { $0 === attached })
        )
        observerCoord.attach(to: window)

        // Advance +45s and simulate software keyboard typing notification -> resets idle timer!
        let beforeTyping = controller.lastActivityDate
        clock.withLock { $0 = $0.addingTimeInterval(45) }
        NotificationCenter.default.post(name: UITextField.textDidChangeNotification, object: nil)
        await Task.yield()
        try await Task.sleep(nanoseconds: 20_000_000)
        XCTAssertGreaterThan(controller.lastActivityDate, beforeTyping)

        // Advance another +45s (90s total, 45s since typing) -> MUST NOT lock yet!
        clock.withLock { $0 = $0.addingTimeInterval(45) }
        XCTAssertFalse(controller.evaluateIdleAutoLock(router: router))
        XCTAssertFalse(router.isPasscodeLocked)

        // Advance +16s with no activity (61s since typing) -> MUST lock!
        clock.withLock { $0 = $0.addingTimeInterval(16) }
        XCTAssertTrue(controller.evaluateIdleAutoLock(router: router))
        XCTAssertTrue(router.isPasscodeLocked)

        // Detach cleans up gesture recognizer from window (zero leaks)
        observerCoord.detach()
        XCTAssertFalse(
            (window.gestureRecognizers ?? []).contains(where: { $0 is FabushiActivityObservationGestureRecognizer })
        )

        // FIX-9: Coordinator deinit automatically removes NotificationCenter observers even without detach()
        let deinitCallbackCount = OSAllocatedUnfairLock<Int>(initialState: 0)
        do {
            let scopedWindow = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
            let scopedCoord = FabushiUserActivityObserverView.Coordinator {
                deinitCallbackCount.withLock { $0 += 1 }
            }
            scopedCoord.attach(to: scopedWindow)
        }
        NotificationCenter.default.post(name: UITextField.textDidChangeNotification, object: nil)
        await Task.yield()
        try await Task.sleep(nanoseconds: 20_000_000)
        XCTAssertEqual(
            deinitCallbackCount.withLock { $0 },
            0,
            "Deallocated Coordinator must remove text observers in deinit"
        )
        #endif
    }

    // MARK: - D2 Regression: Background ScenePhase Respects autoLockTimeoutSeconds & Preserves Locked Sessions

    func testD2BackgroundScenePhaseRespectsAutoLockTimeoutAndNeverUnlocksLockedSession() async throws {
        let (_, secKC) = makeIsolatedKeychains()
        defer { try? secKC.deletePasscodeRecord() }

        let clock = OSAllocatedUnfairLock<Date>(initialState: Date(timeIntervalSince1970: 1_700_010_000))
        let controller = FabushiPasscodeLockController(
            keychainStore: secKC,
            useFastIterationsForTests: true,
            nowProvider: { clock.withLock { $0 } }
        )
        try await controller.setPasscode("246810", coordinator: nil)
        try controller.setAutoLockTimeoutSeconds(60)

        let router = ShellNavigationRouter(isPasscodeLockEnabled: true)
        router.isPasscodeLocked = false

        // 1. Brief switch (< 60s): .background MUST NOT lock immediately; returning at +15s stays unlocked & refreshes activity
        controller.handleScenePhaseChange(.background, router: router)
        XCTAssertFalse(router.isPasscodeLocked)
        XCTAssertNotNil(controller.backgroundedAtDate)

        clock.withLock { $0 = $0.addingTimeInterval(15) }
        controller.handleScenePhaseChange(.active, router: router)
        XCTAssertFalse(router.isPasscodeLocked)
        XCTAssertNil(controller.backgroundedAtDate)
        XCTAssertEqual(controller.lastActivityDate, clock.withLock { $0 })
        XCTAssertFalse(controller.evaluateIdleAutoLock(router: router))

        // 2. Long background (>= 60s): locks immediately upon returning to .active
        controller.handleScenePhaseChange(.background, router: router)
        XCTAssertFalse(router.isPasscodeLocked)
        clock.withLock { $0 = $0.addingTimeInterval(60) }
        controller.handleScenePhaseChange(.active, router: router)
        XCTAssertTrue(router.isPasscodeLocked)

        // 3. Disabled (autoLockTimeoutSeconds == 0): never auto-locks even after 24h in background
        router.isPasscodeLocked = false
        try controller.setAutoLockTimeoutSeconds(0)
        controller.handleScenePhaseChange(.background, router: router)
        clock.withLock { $0 = $0.addingTimeInterval(86_400) }
        controller.handleScenePhaseChange(.active, router: router)
        XCTAssertFalse(router.isPasscodeLocked)
        XCTAssertFalse(controller.shouldEngageIdleAutoLock())

        // 4. Already locked session stays locked across .background -> .active (T2_11 invariant)
        router.isPasscodeLocked = true
        try controller.setAutoLockTimeoutSeconds(300)
        controller.handleScenePhaseChange(.background, router: router)
        clock.withLock { $0 = $0.addingTimeInterval(5) }
        controller.handleScenePhaseChange(.active, router: router)
        XCTAssertTrue(router.isPasscodeLocked)

        // 5. FIX-7: Pre-background idle time + background time >= timeout MUST lock on .active (55s idle + 20s bg >= 60s)
        router.isPasscodeLocked = false
        try controller.setAutoLockTimeoutSeconds(60)
        controller.recordUserActivity()
        clock.withLock { $0 = $0.addingTimeInterval(55) }
        controller.handleScenePhaseChange(.inactive, router: router)
        controller.handleScenePhaseChange(.background, router: router)
        XCTAssertFalse(router.isPasscodeLocked)
        clock.withLock { $0 = $0.addingTimeInterval(20) }
        controller.handleScenePhaseChange(.inactive, router: router)
        controller.handleScenePhaseChange(.active, router: router)
        XCTAssertTrue(
            router.isPasscodeLocked,
            "55s foreground idle + 20s background (75s >= 60s) must engage passcode lock on .active (FIX-7)"
        )

        // 6. FIX-7: Full .active -> .inactive -> .background -> .inactive -> .active brief switch (5s + 10s = 15s < 60s) stays unlocked
        router.isPasscodeLocked = false
        controller.recordUserActivity()
        clock.withLock { $0 = $0.addingTimeInterval(5) }
        controller.handleScenePhaseChange(.inactive, router: router)
        controller.handleScenePhaseChange(.background, router: router)
        clock.withLock { $0 = $0.addingTimeInterval(10) }
        controller.handleScenePhaseChange(.inactive, router: router)
        controller.handleScenePhaseChange(.active, router: router)
        XCTAssertFalse(router.isPasscodeLocked)
        XCTAssertEqual(controller.lastActivityDate, clock.withLock { $0 })

        // 7. FIX-7: Backward wall-clock jump fails closed both while foreground idle and while backgrounded
        clock.withLock { $0 = $0.addingTimeInterval(-10) }
        XCTAssertTrue(controller.shouldEngageIdleAutoLock())
        XCTAssertTrue(controller.evaluateIdleAutoLock(router: router))
        XCTAssertTrue(router.isPasscodeLocked)

        router.isPasscodeLocked = false
        clock.withLock { $0 = $0.addingTimeInterval(20) }
        controller.recordUserActivity()
        controller.handleScenePhaseChange(.background, router: router)
        clock.withLock { $0 = $0.addingTimeInterval(-5) }
        controller.handleScenePhaseChange(.active, router: router)
        XCTAssertTrue(
            router.isPasscodeLocked,
            "Backward wall-clock jump during background must fail closed to locked (FIX-7)"
        )
    }

    // MARK: - D3 Regression: Fail-Closed 4-Case Vault Unlock State Machine, Envelope Upload Rollback & Recovery

    func testD3VaultUnlockFailClosedStateMachineEnvelopeUploadAndRecovery() async throws {
        let serverEnvelopeJSON = OSAllocatedUnfairLock<String?>(initialState: nil)
        let failEnvelopePut = OSAllocatedUnfairLock<Bool>(initialState: false)
        let serverItems = OSAllocatedUnfairLock<[String: FabushiEncryptedVaultBlobDTO]>(initialState: [:])

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "POST", req.path == "/api/v1/auth/login" {
                return .json(200, #"{"user_id":7101,"username":"d3_vault_user","session_id":"s-7101","token":"tok-7101","first_name":"D3User"}"#)
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/envelope" {
                if let env = serverEnvelopeJSON.withLock({ $0 }) {
                    return .json(200, "{\"envelope\":\(env)}")
                }
                return .json(200, #"{"envelope":null}"#)
            }
            if req.method == "PUT", req.path == "/api/v1/user-vault/envelope" {
                if failEnvelopePut.withLock({ $0 }) {
                    return .json(500, #"{"error":"storage_unavailable"}"#)
                }
                serverEnvelopeJSON.withLock { $0 = req.bodyString }
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/items" {
                let items = serverItems.withLock { Array($0.values) }
                let resp = FabushiVaultSyncPullResponse(latestRevision: 1, items: items)
                let data = (try? JSONEncoder().encode(resp)) ?? Data("{\"latest_revision\":0,\"items\":[]}".utf8)
                return .json(200, String(decoding: data, as: UTF8.self))
            }
            if req.method == "PUT", req.path.hasPrefix("/api/v1/user-vault/items/") {
                let tail = String(req.path.dropFirst("/api/v1/user-vault/items/".count))
                let parts = tail.split(separator: "/", maxSplits: 1).map(String.init)
                let category = (parts.first ?? "password").removingPercentEncoding ?? "password"
                let itemKey = (parts.count > 1 ? parts[1] : "item").removingPercentEncoding ?? "item"
                let json = (try? JSONSerialization.jsonObject(with: req.body) as? [String: Any]) ?? [:]
                let blob = FabushiEncryptedVaultBlobDTO(
                    category: category,
                    itemKey: itemKey,
                    revision: 1,
                    nonceB64: (json["nonce_b64"] as? String) ?? "",
                    ciphertextB64: (json["ciphertext_b64"] as? String) ?? "",
                    deleted: false,
                    updatedAtMs: 1_700_000_000_000
                )
                serverItems.withLock { $0["\(category):\(itemKey)"] = blob }
                return .json(200, #"{"ok":true,"category":"\#(category)","item_key":"\#(itemKey)","revision":1,"updated_at_ms":1700000000000}"#)
            }
            return .json(404, #"{"error":"not_found"}"#)
        }
        defer { server.stop() }

        let dir = try makeTempDir("d3_vault_state_machine")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "7101")
            secKC.purgeAccountSecrets(accountId: "7101")
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
                params: FabushiLegacyLoginRequest(username: "d3_vault_user", password: "Pass!1"),
                extractIdentity: { $0.identityTuple }
            )

            let vm = FabushiCredentialVaultViewModel(
                coordinator: coordinator,
                securityKeychain: secKC
            )

            // Case B2: No remote envelope, empty Keychain, bare unlockVault() -> MUST fail closed (ZERO random key!)
            await vm.unlockVault()
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertEqual(vm.unlockPromptReason, .noRemoteEnvelopeRequiresInitialization)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "7101"))
            let statusB2: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("status"),
                params: FabushiEmptyParams()
            )
            XCTAssertFalse(statusB2.unlocked)

            // Case C2-Negative: envelopePut HTTP 500 MUST roll back Rust vault via vault.lock!
            failEnvelopePut.withLock { $0 = true }
            await vm.unlockVault(passphraseOrPassword: "NewVaultPass!9")
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertFalse(coordinator.vaultStore.isUnlocked)
            XCTAssertNil(vm.pendingEmergencyKit)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "7101"))
            let statusRollback: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("status"),
                params: FabushiEmptyParams()
            )
            XCTAssertFalse(
                statusRollback.unlocked,
                "Failed envelopePut MUST roll back Rust vault_unlocked_key via vault.lock"
            )
            XCTAssertEqual(
                statusRollback.hasEnvelope,
                false,
                "Failed envelopePut MUST also clear orphan vault_envelope in Rust RAM via clear_envelope: true (FIX-2)"
            )

            // Case C2-Positive: initSplitKey + envelopePut + Emergency Kit + Keychain + seal item
            failEnvelopePut.withLock { $0 = false }
            await vm.unlockVault(passphraseOrPassword: "NewVaultPass!9")
            XCTAssertTrue(vm.isUnlocked)
            let originalRootKey = try XCTUnwrap(try secKC.loadVaultRootKeyBase64(accountId: "7101"))
            let kit = try XCTUnwrap(vm.pendingEmergencyKit)
            XCTAssertTrue(kit.hasPrefix("EK-"))
            vm.acknowledgeEmergencyKit()
            XCTAssertNil(vm.pendingEmergencyKit)
            let uploadedEnv = try XCTUnwrap(serverEnvelopeJSON.withLock { $0 })
            XCTAssertTrue(uploadedEnv.contains("split_key_v2"))
            XCTAssertTrue(uploadedEnv.contains("kdf_m_cost_kib"))
            XCTAssertTrue(uploadedEnv.contains("recovery_wrapped_root_key_b64"))

            _ = try await vm.saveEncryptedItem(category: .password, itemKey: "proof_item", plaintext: "secret-proof-1")
            try await vm.addTotpAccount(
                label: "ext-lock@test",
                issuer: "Fabushi",
                secretOrURI: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
            )
            XCTAssertFalse(vm.items.isEmpty)
            XCTAssertFalse(vm.totpAccounts.isEmpty)
            XCTAssertFalse(vm.liveTotpCodes.isEmpty)

            // FIX-3: External Rust lock -> refreshVaultStatus() MUST scrub items, TOTP accounts & live codes
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiEmptyParams()
            )
            await vm.refreshVaultStatus()
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertTrue(vm.items.isEmpty, "External lock via refreshVaultStatus must scrub decrypted items")
            XCTAssertTrue(vm.totpAccounts.isEmpty, "External lock via refreshVaultStatus must scrub TOTP accounts")
            XCTAssertTrue(vm.liveTotpCodes.isEmpty, "External lock via refreshVaultStatus must scrub live TOTP codes")
            XCTAssertEqual(vm.remoteEncryptedTotpCount, 0)

            // Case A: Lock -> unlock from local Keychain without passphrase -> decrypts item
            await vm.lockVault()
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertTrue(vm.totpAccounts.isEmpty)
            await vm.unlockVault()
            XCTAssertTrue(vm.isUnlocked)
            XCTAssertEqual(vm.items.first(where: { $0.itemKey == "proof_item" })?.plaintext, "secret-proof-1")

            // Case B1: Existing remote envelope + empty local Keychain + bare unlockVault() -> MUST fail closed!
            await vm.lockVault()
            try secKC.deleteVaultRootKey(accountId: "7101")
            await vm.unlockVault()
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertEqual(vm.unlockPromptReason, .existingRemoteEnvelopeRequiresCredentials)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "7101"))

            // Case C1: Wrong password fails closed; right password unlocks & persists Keychain key
            await vm.unlockVault(passphraseOrPassword: "WrongPass!")
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "7101"))

            await vm.unlockVault(passphraseOrPassword: "NewVaultPass!9")
            XCTAssertTrue(vm.isUnlocked)
            XCTAssertEqual(try secKC.loadVaultRootKeyBase64(accountId: "7101"), originalRootKey)
            XCTAssertEqual(vm.items.first(where: { $0.itemKey == "proof_item" })?.plaintext, "secret-proof-1")

            // Case D: Emergency Kit recovery -> fails closed if remote envelope is nil; recovers & persists exact rootKeyB64 when present!
            await vm.lockVault()
            try secKC.deleteVaultRootKey(accountId: "7101")
            let savedEnv = serverEnvelopeJSON.withLock { $0 }
            serverEnvelopeJSON.withLock { $0 = nil }
            await vm.unlockVault(emergencyKit: kit)
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertNotNil(vm.errorMessage)
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "7101"))

            serverEnvelopeJSON.withLock { $0 = savedEnv }
            await vm.unlockVault(emergencyKit: kit)
            XCTAssertTrue(vm.isUnlocked)
            XCTAssertEqual(try secKC.loadVaultRootKeyBase64(accountId: "7101"), originalRootKey)
            XCTAssertEqual(vm.items.first(where: { $0.itemKey == "proof_item" })?.plaintext, "secret-proof-1")

            // Device-keychain-only cloud envelope rejects password unlock with clear guidance
            await vm.lockVault()
            try secKC.deleteVaultRootKey(accountId: "7101")
            serverEnvelopeJSON.withLock {
                $0 = #"{"key_version":2,"kdf_alg":"device_keychain_v2","kdf_salt_b64":"","kdf_m_cost_kib":65536,"kdf_t_cost":3,"kdf_p_cost":1,"wrapped_root_key_b64":"","recovery_wrapped_root_key_b64":"cmVjb3Zlcnk=","updated_at_ms":1700000000000}"#
            }
            await vm.unlockVault(passphraseOrPassword: "AnyPassword!1")
            XCTAssertFalse(vm.isUnlocked)
            XCTAssertEqual(vm.unlockPromptReason, .existingRemoteEnvelopeRequiresCredentials)
            XCTAssertTrue(vm.errorMessage?.localizedCaseInsensitiveContains("Emergency Kit") == true)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - D5 & D9 Regression: Mandatory Re-Auth on Passcode Change/Clear, Progressive Lockout & Alphanumeric Support

    func testD5AndD9PasscodeReauthEnforcementLockoutAndAlphanumericSupport() async throws {
        let dir = try makeTempDir("d5_d9_passcode")
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
            let clock = OSAllocatedUnfairLock<Date>(initialState: Date(timeIntervalSince1970: 1_700_020_000))
            let controller = FabushiPasscodeLockController(
                keychainStore: secKC,
                useFastIterationsForTests: true,
                nowProvider: { clock.withLock { $0 } }
            )

            // D9: Initial setup with alphanumeric passcode works in both Swift PBKDF2 & Rust Argon2id
            try await controller.setPasscode("Abc!9x", coordinator: coordinator)
            XCTAssertTrue(controller.isPasscodeEnabled)

            struct VerifyParams: Encodable, Sendable { let passcode: String }
            struct VerifyResp: Decodable, Sendable { let valid: Bool; let configured: Bool }
            let rustVerifyAlpha: VerifyResp = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeVerify"),
                params: VerifyParams(passcode: "Abc!9x")
            )
            XCTAssertTrue(rustVerifyAlpha.configured && rustVerifyAlpha.valid)

            // D5: Unauthorized overwrite & clear MUST throw .reauthenticationRequired
            do {
                try await controller.setPasscode("New!23", coordinator: coordinator)
                XCTFail("Expected .reauthenticationRequired on unauthorized overwrite")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .reauthenticationRequired)
            }
            do {
                try await controller.clearPasscode(coordinator: coordinator)
                XCTFail("Expected .reauthenticationRequired on unauthorized clear")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .reauthenticationRequired)
            }

            // Wrong current passcode triggers progressive lockout (1 -> 2 -> 3 locks out for 5s)
            for expectedAttempt in 1...2 {
                do {
                    try await controller.clearPasscode(
                        authorization: .currentPasscode("WrongPass"),
                        coordinator: coordinator
                    )
                    XCTFail("Expected .currentPasscodeIncorrect")
                } catch let err as FabushiPasscodePolicyError {
                    XCTAssertEqual(err, .currentPasscodeIncorrect(failedAttempts: expectedAttempt))
                }
            }
            do {
                try await controller.setPasscode(
                    "ZyX#42",
                    currentAuthorization: .currentPasscode("WrongPass"),
                    coordinator: coordinator
                )
                XCTFail("Expected .lockedOut on 3rd failed re-auth")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .lockedOut(remainingSeconds: 5, failedAttempts: 3))
            }

            // Even right current passcode is rejected while lockout timer is active
            do {
                try await controller.clearPasscode(
                    authorization: .currentPasscode("Abc!9x"),
                    coordinator: coordinator
                )
                XCTFail("Expected .lockedOut during active lockout window")
            } catch let err as FabushiPasscodePolicyError {
                if case .lockedOut(let remaining, let attempts) = err {
                    XCTAssertGreaterThan(remaining, 0)
                    XCTAssertEqual(attempts, 3)
                } else {
                    XCTFail("Expected .lockedOut, got \(err)")
                }
            }

            // Biometric re-auth when biometricsEnabled == false fails closed without modifying passcode
            do {
                try await controller.clearPasscode(
                    authorization: .biometrics(localizedReason: "Disable passcode"),
                    coordinator: coordinator
                )
                XCTFail("Expected .biometricVerificationFailed")
            } catch let err as FabushiPasscodePolicyError {
                if case .biometricVerificationFailed = err {
                    // Expected
                } else {
                    XCTFail("Expected .biometricVerificationFailed, got \(err)")
                }
            }
            XCTAssertTrue(controller.isPasscodeEnabled)

            // Advance clock +6s -> authorized overwrite to alphanumeric "ZyX#42" succeeds!
            clock.withLock { $0 = $0.addingTimeInterval(6) }
            try await controller.setPasscode(
                "ZyX#42",
                currentAuthorization: .currentPasscode("Abc!9x"),
                coordinator: coordinator
            )
            let oldRejected = await controller.verifyPasscode("Abc!9x", coordinator: coordinator)
            XCTAssertEqual(oldRejected, .incorrect(failedAttempts: 1))
            let newAccepted = await controller.verifyPasscode("ZyX#42", coordinator: coordinator)
            XCTAssertEqual(newAccepted, .unlocked)

            // Authorized clear wipes Keychain & Rust
            try await controller.clearPasscode(
                authorization: .currentPasscode("ZyX#42"),
                coordinator: coordinator
            )
            XCTAssertFalse(controller.isPasscodeEnabled)
            XCTAssertNil(try secKC.loadPasscodeRecord())
            let rustCleared: VerifyResp = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeVerify"),
                params: VerifyParams(passcode: "ZyX#42")
            )
            XCTAssertFalse(rustCleared.configured)
            XCTAssertFalse(rustCleared.valid)

            // FIX-8 Subcase A: Fresh controller with stale in-memory isPasscodeEnabled == false
            // while Keychain record exists MUST reject unauthenticated setPasscode & clearPasscode!
            let staleController = FabushiPasscodeLockController(
                keychainStore: secKC,
                useFastIterationsForTests: true,
                nowProvider: { clock.withLock { $0 } }
            )
            XCTAssertFalse(staleController.isPasscodeEnabled)
            try await controller.setPasscode("Keychain!77", coordinator: nil)
            XCTAssertFalse(staleController.isPasscodeEnabled)
            do {
                try await staleController.setPasscode("Bypass!1", coordinator: nil)
                XCTFail("Expected .reauthenticationRequired when Keychain already holds enabled passcode")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .reauthenticationRequired)
            }
            do {
                try await staleController.clearPasscode(coordinator: nil)
                XCTFail("Expected .reauthenticationRequired on stale controller clearPasscode")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .reauthenticationRequired)
            }
            try await staleController.clearPasscode(
                authorization: .currentPasscode("Keychain!77"),
                coordinator: nil
            )

            // FIX-8 Subcase B: Corrupted/undecodable Keychain record MUST fail closed (propagate decodingFailed, NOT bypass re-auth!)
            let corruptQuery: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: secKC.passcodeService,
                kSecAttrAccount as String: secKC.passcodeAccount,
                kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
                kSecValueData as String: Data("not-valid-json".utf8),
            ]
            XCTAssertEqual(SecItemAdd(corruptQuery as CFDictionary, nil), errSecSuccess)
            let corruptController = FabushiPasscodeLockController(
                keychainStore: secKC,
                useFastIterationsForTests: true,
                nowProvider: { clock.withLock { $0 } }
            )
            XCTAssertFalse(corruptController.isPasscodeEnabled)
            do {
                try await corruptController.setPasscode("Overwrite!9", coordinator: nil)
                XCTFail("Expected fail-closed Keychain error on corrupted passcode record")
            } catch let kcErr as FabushiSecurityKeychainError {
                XCTAssertEqual(kcErr, .decodingFailed)
            }
            do {
                try await corruptController.clearPasscode(coordinator: nil)
                XCTFail("Expected fail-closed Keychain error on clearPasscode with corrupted record")
            } catch let kcErr as FabushiSecurityKeychainError {
                XCTAssertEqual(kcErr, .decodingFailed)
            }
            try secKC.deletePasscodeRecord()

            // FIX-8 Subcase C: Rust-only configured passcode (no Keychain record) MUST enforce mandatory re-auth!
            struct RustSetParams: Encodable, Sendable { let passcode: String }
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeSet"),
                params: RustSetParams(passcode: "RustOnly!88")
            )
            let rustOnlyController = FabushiPasscodeLockController(
                keychainStore: secKC,
                useFastIterationsForTests: true,
                nowProvider: { clock.withLock { $0 } }
            )
            XCTAssertFalse(rustOnlyController.isPasscodeEnabled)
            XCTAssertNil(try secKC.loadPasscodeRecord())

            do {
                try await rustOnlyController.setPasscode("Hijack!1", coordinator: coordinator)
                XCTFail("Expected .reauthenticationRequired when Rust host_meta has configured passcode")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .reauthenticationRequired)
            }
            do {
                try await rustOnlyController.clearPasscode(coordinator: coordinator)
                XCTFail("Expected .reauthenticationRequired on clearPasscode for Rust-only passcode")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .reauthenticationRequired)
            }
            do {
                try await rustOnlyController.clearPasscode(
                    authorization: .currentPasscode("WrongRustPass"),
                    coordinator: coordinator
                )
                XCTFail("Expected .currentPasscodeIncorrect for wrong Rust-only passcode")
            } catch let err as FabushiPasscodePolicyError {
                XCTAssertEqual(err, .currentPasscodeIncorrect(failedAttempts: 1))
            }
            try await rustOnlyController.clearPasscode(
                authorization: .currentPasscode("RustOnly!88"),
                coordinator: coordinator
            )
            let rustAfterClear: VerifyResp = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeVerify"),
                params: VerifyParams(passcode: "RustOnly!88")
            )
            XCTAssertFalse(rustAfterClear.configured)

            // FIX-9: Bilingual localization verification for new passcode & vault status keys
            for key in [
                "hig.passcode.reauthRequiredError",
                "hig.passcode.biometricFailedError",
                "hig.vault.deviceSyncStatusLabel",
            ] {
                let enVal = FabushiLocale.en.localized(key, bundle: .main)
                let zhVal = FabushiLocale.zhHans.localized(key, bundle: .main)
                XCTAssertNotEqual(enVal, key)
                XCTAssertNotEqual(zhVal, key)
                XCTAssertNotEqual(enVal, zhVal)
            }

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - D6 Regression: Local-Only Auto-Expiring Secure Clipboard Enforcement

    func testD6SecureClipboardLocalOnlyAndExpirationEnforcement() async throws {
        let t0 = Date(timeIntervalSince1970: 1_700_050_000)
        let payload = FabushiSecureClipboard.makeSensitivePayload(
            "EK-TEST-1234",
            expirationSeconds: 60,
            now: t0
        )
        XCTAssertTrue(payload.localOnly)
        XCTAssertEqual(payload.typeIdentifier, UTType.plainText.identifier)
        XCTAssertEqual(payload.value, "EK-TEST-1234")
        XCTAssertEqual(payload.expirationDate, t0.addingTimeInterval(60))

        // Clamping non-positive expiration to >= 1s
        let clamped = FabushiSecureClipboard.makeSensitivePayload(
            "EK-CLAMP",
            expirationSeconds: 0,
            now: t0
        )
        XCTAssertEqual(clamped.expirationDate, t0.addingTimeInterval(1))

        #if canImport(UIKit)
        defer { UIPasteboard.general.items = [] }

        let copied = FabushiSecureClipboard.copySensitive("S3cr3t-TOTP-287082", expirationSeconds: 60)
        XCTAssertTrue(copied.localOnly)
        XCTAssertEqual(UIPasteboard.general.string, "S3cr3t-TOTP-287082")
        XCTAssertTrue(UIPasteboard.general.contains(pasteboardTypes: [UTType.plainText.identifier]))

        // Verify UIPasteboard actually honors `.expirationDate` (1s TTL clears automatically)
        FabushiSecureClipboard.copySensitive("ephemeral-secret", expirationSeconds: 1)
        XCTAssertEqual(UIPasteboard.general.string, "ephemeral-secret")

        var expired = false
        for _ in 0..<24 {
            try await Task.sleep(nanoseconds: 250_000_000)
            if UIPasteboard.general.string == nil {
                expired = true
                break
            }
        }
        XCTAssertTrue(
            expired,
            "UIPasteboard.general must clear expired sensitive items set via FabushiSecureClipboard"
        )
        #endif
    }

    // MARK: - D7-Swift Regression: Coordinator Logout Purger Hook, Idempotency, Offline Wipe & Stale-Generation Preservation

    func testD7CoordinatorLogoutHookPurgesKeychainSecretsAndRespectsStaleGeneration() async throws {
        let failLogout = OSAllocatedUnfairLock<Bool>(initialState: false)

        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                if req.bodyString.contains("bob_d7") {
                    return .json(200, #"{"user_id":202,"username":"bob_d7","session_id":"s-202","token":"tok-202","first_name":"Bob"}"#)
                }
                return .json(200, #"{"user_id":101,"username":"alice_d7","session_id":"s-101","token":"tok-101","first_name":"Alice"}"#)
            case ("POST", "/api/v1/auth/logout"):
                if failLogout.withLock({ $0 }) {
                    return .json(500, #"{"error":"offline_logout_error"}"#)
                }
                return .json(200, #"{"ok":true}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("d7_logout_hook")
        let (coreKC, secKC) = makeIsolatedKeychains()
        defer {
            try? coreKC.deletePassphrase()
            for uid in ["101", "202"] {
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
            // Register twice to prove keyed idempotency
            let secModule = FabushiSecurityModule(securityKeychain: secKC)
            secModule.installLogoutSecretsPurger(on: coordinator)
            secModule.installLogoutSecretsPurger(on: coordinator)

            // Sign in Account A (101) & Account B (202), seed Keychain secrets for BOTH
            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "alice_d7", password: "pwA"),
                extractIdentity: { $0.identityTuple }
            )
            try secKC.saveVaultRootKeyBase64(
                try FabushiPasscodeCrypto.generateRandomBytes(count: 32).base64EncodedString(),
                accountId: "101"
            )
            try secKC.saveTotpAccounts(
                [FabushiStoredTotpAccount(label: "alice@d7", issuer: "D7", secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")],
                accountId: "101"
            )

            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "bob_d7", password: "pwB"),
                extractIdentity: { $0.identityTuple }
            )
            try secKC.saveVaultRootKeyBase64(
                try FabushiPasscodeCrypto.generateRandomBytes(count: 32).base64EncodedString(),
                accountId: "202"
            )
            try secKC.saveTotpAccounts(
                [FabushiStoredTotpAccount(label: "bob@d7", issuer: "D7", secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")],
                accountId: "202"
            )

            // 1. Stale generation guard: MUST preserve active account "202" secrets when logout throws stale_generation!
            _ = try await coordinator.host.bumpGeneration()
            do {
                _ = try await coordinator.logout()
                XCTFail("Expected stale_generation on bumped host generation")
            } catch let hostErr as FabushiHostError {
                XCTAssertEqual(hostErr.code, "stale_generation")
            }
            XCTAssertTrue(try coreKC.hasAccountKey(accountId: "202"))
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "202"))
            XCTAssertEqual(try secKC.loadTotpAccounts(accountId: "202").count, 1)

            // 2. Normal direct coordinator.logout() (no AccountSwitcher): scrubs "202", preserves "101"!
            _ = try await coordinator.logout()
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertFalse(try coreKC.hasAccountKey(accountId: "202"))
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "202"))
            XCTAssertTrue(try secKC.loadTotpAccounts(accountId: "202").isEmpty)
            XCTAssertTrue(try coreKC.hasAccountKey(accountId: "101"))
            XCTAssertTrue(secKC.hasVaultRootKey(accountId: "101"))
            XCTAssertEqual(try secKC.loadTotpAccounts(accountId: "101").count, 1)

            // 3. Offline logout (HTTP 500 -> Rust wipes local session): reconciles unauthenticated & purges "101"!
            _ = try await coordinator.switchAccount(accountId: "101")
            failLogout.withLock { $0 = true }
            do {
                _ = try await coordinator.logout()
                XCTFail("Expected offline HTTP 500 error to propagate")
            } catch {
                // Expected server error after Rust local wipe
            }
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertFalse(try coreKC.hasAccountKey(accountId: "101"))
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "101"))
            XCTAssertTrue(try secKC.loadTotpAccounts(accountId: "101").isEmpty)

            // 4. FIX-8 / reviewer_2 note #4: Explicit clearLocalSession() MUST invoke registered account-secrets purger!
            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "alice_d7", password: "pwA"),
                extractIdentity: { $0.identityTuple }
            )
            try secKC.saveVaultRootKeyBase64(
                try FabushiPasscodeCrypto.generateRandomBytes(count: 32).base64EncodedString(),
                accountId: "101"
            )
            try secKC.saveTotpAccounts(
                [FabushiStoredTotpAccount(label: "clear@d7", issuer: "D7", secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")],
                accountId: "101"
            )
            _ = try await coordinator.clearLocalSession()
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertFalse(try coreKC.hasAccountKey(accountId: "101"))
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "101"))
            XCTAssertTrue(try secKC.loadTotpAccounts(accountId: "101").isEmpty)

            // 5. FIX-9 / challenger_2 LOW-1: setServerURL to a different server purges Server A vault/TOTP Keychain secrets
            // while preserving the device-level local passcode record!
            let passcodeCtrl = FabushiPasscodeLockController(
                keychainStore: secKC,
                useFastIterationsForTests: true
            )
            defer { try? secKC.deletePasscodeRecord() }
            try await passcodeCtrl.setPasscode("KeepMe!1", coordinator: nil)
            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "alice_d7", password: "pwA"),
                extractIdentity: { $0.identityTuple }
            )
            try secKC.saveVaultRootKeyBase64(
                try FabushiPasscodeCrypto.generateRandomBytes(count: 32).base64EncodedString(),
                accountId: "101"
            )
            try secKC.saveTotpAccounts(
                [FabushiStoredTotpAccount(label: "srvA@d7", issuer: "D7", secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")],
                accountId: "101"
            )
            _ = try await coordinator.setServerURL("http://127.0.0.2:9")
            XCTAssertEqual(coordinator.authState, .unauthenticated)
            XCTAssertFalse(try coreKC.hasAccountKey(accountId: "101"))
            XCTAssertFalse(secKC.hasVaultRootKey(accountId: "101"))
            XCTAssertTrue(try secKC.loadTotpAccounts(accountId: "101").isEmpty)
            XCTAssertNotNil(
                try secKC.loadPasscodeRecord(),
                "Switching server URL must preserve device-level passcode record while purging server-scoped vault keys"
            )

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - FIX-5 Regression: Top-Level Overlay UIWindow Covers Child Sheets & Cleans Up on Teardown

    func testFix5SecurityOverlayWindowCoversSheetsAndCleansUpOnTeardown() {
        #if canImport(UIKit)
        let coordinator = FabushiCoordinator(host: FabushiRustHostActor())
        let theme = FabushiTheme(locale: .en)
        let router = ShellNavigationRouter(isPasscodeLockEnabled: true)
        let registry = FabushiFeatureModuleRegistry()
        registry.register(security: FabushiSecurityModule())

        let overlayCoordinator = FabushiSecurityOverlayWindowCoordinator(
            coordinator: coordinator,
            theme: theme,
            router: router,
            registry: registry,
            launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: []),
            onUnlock: {}
        )
        defer { overlayCoordinator.teardown() }

        XCTAssertNil(overlayCoordinator.overlayWindow)
        XCTAssertFalse(overlayCoordinator.isOverlayWindowVisible)

        let activeScene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        guard activeScene != nil else {
            // Skip UIWindowScene presentation assertions only if no UIWindowScene exists in host environment
            return
        }

        // 1. Privacy blur active -> top-level overlay UIWindow mounts above UIWindow.Level.alert
        overlayCoordinator.update(
            isPasscodeOverlayActive: false,
            isPrivacyBlurActive: true,
            hostWindow: nil,
            coordinator: coordinator,
            theme: theme,
            router: router,
            registry: registry,
            launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: []),
            onUnlock: {}
        )
        XCTAssertTrue(overlayCoordinator.isOverlayWindowVisible)
        let blurWindow = overlayCoordinator.overlayWindow
        XCTAssertNotNil(blurWindow)
        XCTAssertGreaterThan(
            blurWindow?.windowLevel.rawValue ?? 0,
            UIWindow.Level.alert.rawValue,
            "Security overlay UIWindow must render strictly above UIWindow.Level.alert and all presented .sheets"
        )
        XCTAssertEqual(blurWindow?.accessibilityViewIsModal, true)

        // 2. Passcode lock active -> overlay window becomes key & modal above all sheets
        overlayCoordinator.update(
            isPasscodeOverlayActive: true,
            isPrivacyBlurActive: false,
            hostWindow: nil,
            coordinator: coordinator,
            theme: theme,
            router: router,
            registry: registry,
            launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: []),
            onUnlock: {}
        )
        XCTAssertTrue(overlayCoordinator.isOverlayWindowVisible)
        XCTAssertEqual(overlayCoordinator.overlayWindow?.isKeyWindow, true)
        XCTAssertEqual(overlayCoordinator.overlayWindow?.accessibilityViewIsModal, true)

        // 3. Unlock & active -> overlay window hides cleanly
        overlayCoordinator.update(
            isPasscodeOverlayActive: false,
            isPrivacyBlurActive: false,
            hostWindow: nil,
            coordinator: coordinator,
            theme: theme,
            router: router,
            registry: registry,
            launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: []),
            onUnlock: {}
        )
        XCTAssertFalse(overlayCoordinator.isOverlayWindowVisible)
        XCTAssertEqual(overlayCoordinator.overlayWindow?.accessibilityViewIsModal, false)

        // 4. Teardown releases UIWindow completely
        overlayCoordinator.teardown()
        XCTAssertNil(overlayCoordinator.overlayWindow)
        XCTAssertFalse(overlayCoordinator.isOverlayWindowVisible)
        #endif
    }
}
