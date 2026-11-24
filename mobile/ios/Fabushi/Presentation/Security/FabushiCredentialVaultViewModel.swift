import Foundation
import Observation

// MARK: - Canonical Vault Categories & Fail-Closed Limits (Desktop v1.6.9 Parity)

public enum FabushiVaultCategory: String, CaseIterable, Codable, Sendable, Identifiable {
    case password = "password"
    case browserCookie = "browser_cookie"
    case aiOauth = "ai_oauth"

    public var id: String { rawValue }

    public var localizationKey: String {
        switch self {
        case .password: return "hig.vault.categoryPassword"
        case .browserCookie: return "hig.vault.categoryBrowserCookie"
        case .aiOauth: return "hig.vault.categoryAiOauth"
        }
    }

    public var systemImage: String {
        switch self {
        case .password: return "key.horizontal.fill"
        case .browserCookie: return "globe"
        case .aiOauth: return "sparkles"
        }
    }
}

public enum FabushiVaultValidationError: LocalizedError, Sendable, Equatable {
    case payloadExceedsLimit(actualBytes: Int, maxBytes: Int)
    case invalidItemKey(String)
    case vaultLocked
    case unauthenticated

    public var errorDescription: String? {
        switch self {
        case .payloadExceedsLimit(let actual, let maxBytes):
            return "Vault payload size (\(actual) bytes) exceeds the 24 MiB (\(maxBytes) bytes) security limit."
        case .invalidItemKey(let reason):
            return "Invalid vault item key: \(reason)"
        case .vaultLocked:
            return "Credential vault is locked."
        case .unauthenticated:
            return "Sign in before accessing the credential vault."
        }
    }
}

public enum FabushiVaultLimits: Sendable {
    /// Fail-closed 24 MiB payload ceiling matching server `MAX_VAULT_CIPHERTEXT_BYTES`.
    public static let maxPayloadBytes: Int = 24 * 1024 * 1024
    public static let maxItemKeyLength: Int = 128

    public static func validatePayloadSize(_ plaintext: String) throws {
        let byteCount = plaintext.utf8.count
        guard byteCount <= maxPayloadBytes else {
            throw FabushiVaultValidationError.payloadExceedsLimit(
                actualBytes: byteCount,
                maxBytes: maxPayloadBytes
            )
        }
    }

    public static func validateItemKey(_ rawKey: String) throws -> String {
        let key = rawKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty, key.utf8.count <= maxItemKeyLength else {
            throw FabushiVaultValidationError.invalidItemKey("length must be 1..=128 characters")
        }
        guard !key.contains(".."), !key.contains("|") else {
            throw FabushiVaultValidationError.invalidItemKey("must not contain '..' or '|'")
        }
        for scalar in key.unicodeScalars {
            let value = scalar.value
            let isUpper = value >= 0x41 && value <= 0x5A
            let isLower = value >= 0x61 && value <= 0x7A
            let isDigit = value >= 0x30 && value <= 0x39
            let isAllowedPunct = scalar == "." || scalar == "_" || scalar == ":" || scalar == "-"
            guard isUpper || isLower || isDigit || isAllowedPunct else {
                throw FabushiVaultValidationError.invalidItemKey(
                    "only ASCII letters, digits, '.', '_', ':', and '-' are permitted"
                )
            }
        }
        return key
    }
}

// MARK: - Vault Sync, Sealing, X25519 Device Trust Ring & TOTP DTOs

public struct FabushiEncryptedVaultBlobDTO: Codable, Sendable, Equatable {
    public let category: String
    public let itemKey: String
    public let revision: Int64
    public let nonceB64: String
    public let ciphertextB64: String
    public let deleted: Bool
    public let updatedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case category
        case itemKey = "item_key"
        case revision
        case nonceB64 = "nonce_b64"
        case ciphertextB64 = "ciphertext_b64"
        case deleted
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        category: String,
        itemKey: String,
        revision: Int64,
        nonceB64: String,
        ciphertextB64: String,
        deleted: Bool = false,
        updatedAtMs: Int64 = 0
    ) {
        self.category = category
        self.itemKey = itemKey
        self.revision = revision
        self.nonceB64 = nonceB64
        self.ciphertextB64 = ciphertextB64
        self.deleted = deleted
        self.updatedAtMs = updatedAtMs
    }
}

public struct FabushiVaultSealItemRequest: Encodable, Sendable, Equatable {
    public let userId: UInt64
    public let category: String
    public let itemKey: String
    public let revision: Int64
    public let plaintext: String

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case category
        case itemKey = "item_key"
        case revision
        case plaintext
    }
}

public struct FabushiVaultOpenItemRequest: Encodable, Sendable, Equatable {
    public let userId: UInt64
    public let blob: FabushiEncryptedVaultBlobDTO

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case blob
    }
}

public struct FabushiVaultOpenItemResponse: Codable, Sendable, Equatable {
    public let plaintextUtf8: String?
    public let plaintextB64: String

    public enum CodingKeys: String, CodingKey {
        case plaintextUtf8 = "plaintext_utf8"
        case plaintextB64 = "plaintext_b64"
    }
}

public struct FabushiVaultSyncPullRequest: Encodable, Sendable, Equatable {
    public let sinceRevision: Int64?
    public let category: String?

    public enum CodingKeys: String, CodingKey {
        case sinceRevision = "since_revision"
        case category
    }
}

public struct FabushiVaultSyncPullResponse: Codable, Sendable, Equatable {
    public let latestRevision: Int64
    public let items: [FabushiEncryptedVaultBlobDTO]

    public enum CodingKeys: String, CodingKey {
        case latestRevision = "latest_revision"
        case items
    }
}

public struct FabushiVaultSyncPushRequest: Encodable, Sendable, Equatable {
    public let category: String
    public let itemKey: String
    public let expectedRevision: Int64?
    public let nonceB64: String
    public let ciphertextB64: String
    public let deleted: Bool

    public enum CodingKeys: String, CodingKey {
        case category
        case itemKey = "item_key"
        case expectedRevision = "expected_revision"
        case nonceB64 = "nonce_b64"
        case ciphertextB64 = "ciphertext_b64"
        case deleted
    }
}

public struct FabushiVaultSyncPushResponse: Codable, Sendable, Equatable {
    public let ok: Bool
    public let category: String
    public let itemKey: String
    public let revision: Int64
    public let updatedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case ok
        case category
        case itemKey = "item_key"
        case revision
        case updatedAtMs = "updated_at_ms"
    }
}

public struct FabushiVaultSyncDeleteRequest: Encodable, Sendable, Equatable {
    public let category: String
    public let itemKey: String
    public let expectedRevision: Int64?

    public enum CodingKeys: String, CodingKey {
        case category
        case itemKey = "item_key"
        case expectedRevision = "expected_revision"
    }
}

public struct FabushiVaultUnlockPassphraseRequest: Encodable, Sendable, Equatable {
    public let userId: UInt64
    public let passphrase: String
    public let envelope: FabushiVaultNestedEnvelopeDTO?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case passphrase
        case envelope
    }
}

public struct FabushiVaultUnlockRecoveryRequest: Encodable, Sendable, Equatable {
    public let userId: UInt64
    public let recoveryKey: String
    public let envelope: FabushiVaultNestedEnvelopeDTO?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case recoveryKey = "recovery_key"
        case envelope
    }
}

public struct FabushiVaultDecryptedItem: Identifiable, Sendable, Equatable {
    public var id: String { "\(category):\(itemKey)" }
    public let category: String
    public let itemKey: String
    public let revision: Int64
    public let plaintext: String?
    public let updatedAtMs: Int64
    public let isOpaqueRemote: Bool
}

// MARK: - X25519 Device Trust Ring Sync DTOs

public struct FabushiDeviceSyncKeygenRequest: Encodable, Sendable, Equatable {
    public let pairId: String?

    public enum CodingKeys: String, CodingKey {
        case pairId = "pair_id"
    }
}

public struct FabushiDeviceSyncKeygenResponse: Codable, Sendable, Equatable {
    public let pairId: String
    public let publicKeyB64: String
    public let verificationCode: String

    public enum CodingKeys: String, CodingKey {
        case pairId = "pair_id"
        case publicKeyB64 = "public_key_b64"
        case verificationCode = "verification_code"
    }
}

public struct FabushiDeviceSyncRequestBodyDTO: Encodable, Sendable, Equatable {
    public let deviceName: String
    public let devicePublicKeyB64: String

    public enum CodingKeys: String, CodingKey {
        case deviceName = "device_name"
        case devicePublicKeyB64 = "device_public_key_b64"
    }
}

public struct FabushiDeviceSyncRequestResponseDTO: Codable, Sendable, Equatable {
    public let requestId: String
    public let verificationCode: String
    public let expiresAt: Int64

    public enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
        case verificationCode = "verification_code"
        case expiresAt = "expires_at"
    }
}

public struct FabushiPendingDeviceSyncItemDTO: Codable, Sendable, Equatable, Identifiable {
    public var id: String { requestId }
    public let requestId: String
    public let deviceName: String
    public let devicePublicKeyB64: String
    public let verificationCode: String
    public let status: String
    public let createdAt: Int64
    public let expiresAt: Int64

    public enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
        case deviceName = "device_name"
        case devicePublicKeyB64 = "device_public_key_b64"
        case verificationCode = "verification_code"
        case status
        case createdAt = "created_at"
        case expiresAt = "expires_at"
    }
}

public struct FabushiPendingDeviceSyncListResponseDTO: Codable, Sendable, Equatable {
    public let items: [FabushiPendingDeviceSyncItemDTO]
}

public struct FabushiDeviceKeyTransferDTO: Codable, Sendable, Equatable {
    public let approverEphemeralPubB64: String
    public let encryptedRootKeyB64: String
    public let nonceB64: String

    public enum CodingKeys: String, CodingKey {
        case approverEphemeralPubB64 = "approver_ephemeral_pub_b64"
        case encryptedRootKeyB64 = "encrypted_root_key_b64"
        case nonceB64 = "nonce_b64"
    }
}

public struct FabushiDeviceSyncSealRequestDTO: Encodable, Sendable, Equatable {
    public let requestId: String
    public let userId: UInt64
    public let targetDevicePubB64: String

    public enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
        case userId = "user_id"
        case targetDevicePubB64 = "target_device_pub_b64"
    }
}

public struct FabushiDeviceSyncSealResponseDTO: Codable, Sendable, Equatable {
    public let verificationCode: String
    public let transfer: FabushiDeviceKeyTransferDTO

    public enum CodingKeys: String, CodingKey {
        case verificationCode = "verification_code"
        case transfer
    }
}

public struct FabushiDeviceSyncApproveRequestDTO: Encodable, Sendable, Equatable {
    public let requestId: String
    public let approverEphemeralPubB64: String
    public let encryptedRootKeyB64: String
    public let nonceB64: String

    public enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
        case approverEphemeralPubB64 = "approver_ephemeral_pub_b64"
        case encryptedRootKeyB64 = "encrypted_root_key_b64"
        case nonceB64 = "nonce_b64"
    }
}

public struct FabushiDeviceSyncDenyRequestDTO: Encodable, Sendable, Equatable {
    public let requestId: String

    public enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
    }
}

public struct FabushiDeviceSyncActionResponseDTO: Codable, Sendable, Equatable {
    public let ok: Bool
    public let requestId: String
    public let status: String

    public enum CodingKeys: String, CodingKey {
        case ok
        case requestId = "request_id"
        case status
    }
}

public struct FabushiDeviceSyncStatusRequestDTO: Encodable, Sendable, Equatable {
    public let requestId: String

    public enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
    }
}

public struct FabushiDeviceSyncStatusResponseDTO: Codable, Sendable, Equatable {
    public let status: String
    public let approverEphemeralPubB64: String?
    public let encryptedRootKeyB64: String?
    public let nonceB64: String?
    public let expiresAt: Int64

    public enum CodingKeys: String, CodingKey {
        case status
        case approverEphemeralPubB64 = "approver_ephemeral_pub_b64"
        case encryptedRootKeyB64 = "encrypted_root_key_b64"
        case nonceB64 = "nonce_b64"
        case expiresAt = "expires_at"
    }
}

public struct FabushiDeviceSyncOpenRequestDTO: Encodable, Sendable, Equatable {
    public let pairId: String
    public let requestId: String
    public let userId: UInt64
    public let transfer: FabushiDeviceKeyTransferDTO

    public enum CodingKeys: String, CodingKey {
        case pairId = "pair_id"
        case requestId = "request_id"
        case userId = "user_id"
        case transfer
    }
}

public struct FabushiDeviceSyncOpenResponseDTO: Codable, Sendable, Equatable {
    public let unlocked: Bool
    public let rootKeyB64: String

    public enum CodingKeys: String, CodingKey {
        case unlocked
        case rootKeyB64 = "root_key_b64"
    }
}

// MARK: - RFC 6238 TOTP DTOs & `otpauth://` Parser

public struct FabushiTotpGenerateRequestDTO: Encodable, Sendable, Equatable {
    public let secret: String
    public let unixSecs: UInt64?
    public let digits: UInt64?
    public let period: UInt64?

    public enum CodingKeys: String, CodingKey {
        case secret
        case unixSecs = "unix_secs"
        case digits
        case period
    }
}

public struct FabushiTotpGenerateResponseDTO: Codable, Sendable, Equatable {
    public let code: String
    public let unixSecs: UInt64?
    public let digits: UInt32?
    public let period: UInt64?

    public enum CodingKeys: String, CodingKey {
        case code
        case unixSecs = "unix_secs"
        case digits
        case period
    }
}

public struct FabushiTotpURIInfo: Sendable, Equatable {
    public let label: String
    public let issuer: String
    public let secretOrURI: String
    public let digits: Int
    public let period: Int

    public static func parse(_ raw: String, fallbackLabel: String = "Authenticator") -> FabushiTotpURIInfo? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        if trimmed.lowercased().hasPrefix("otpauth://") {
            guard let components = URLComponents(string: trimmed),
                  components.host?.lowercased() == "totp"
            else {
                return nil
            }
            var query: [String: String] = [:]
            for item in components.queryItems ?? [] {
                if let value = item.value {
                    query[item.name.lowercased()] = value
                }
            }
            guard let secretParam = query["secret"], !secretParam.trimmingCharacters(in: .whitespaces).isEmpty else {
                return nil
            }
            let rawPath = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            let decodedPath = rawPath.removingPercentEncoding ?? rawPath
            let pathParts = decodedPath.split(separator: ":", maxSplits: 1).map(String.init)
            let pathIssuer = pathParts.count == 2 ? pathParts[0] : ""
            let label = (pathParts.last?.isEmpty == false) ? (pathParts.last ?? fallbackLabel) : fallbackLabel
            let issuer = query["issuer"] ?? pathIssuer
            let digits = Int(query["digits"] ?? "") ?? 6
            let period = max(1, Int(query["period"] ?? "") ?? 30)
            return FabushiTotpURIInfo(
                label: label,
                issuer: issuer,
                secretOrURI: trimmed,
                digits: min(max(digits, 6), 8),
                period: period
            )
        }

        return FabushiTotpURIInfo(
            label: fallbackLabel,
            issuer: "",
            secretOrURI: trimmed,
            digits: 6,
            period: 30
        )
    }

    public static func remainingSeconds(at unixSecs: Int64, period: Int = 30) -> Int {
        let safePeriod = max(1, period)
        let elapsed = Int(unixSecs % Int64(safePeriod))
        let remaining = safePeriod - elapsed
        return remaining > 0 ? remaining : safePeriod
    }
}

// MARK: - Zero-Knowledge Credential Vault & RFC 6238 TOTP ViewModel (F23 UI)

@MainActor
@Observable
public final class FabushiCredentialVaultViewModel {
    @ObservationIgnored private let coordinator: FabushiCoordinator
    @ObservationIgnored private let securityKeychain: FabushiSecurityKeychainStore
    @ObservationIgnored private let nowProvider: @Sendable () -> Date
    @ObservationIgnored private let taskBag = FabushiCancellableTaskBag()

    public var isUnlocked: Bool = false
    public var selectedCategory: FabushiVaultCategory = .password
    public var items: [FabushiVaultDecryptedItem] = []
    public var totpAccounts: [FabushiStoredTotpAccount] = []
    public var liveTotpCodes: [String: String] = [:]
    public var totpRemainingSeconds: Int = 30
    public var remoteEncryptedTotpCount: Int = 0

    // X25519 Device Sync state
    public var activeSyncPairId: String?
    public var activeSyncRequestId: String?
    public var activeSyncVerificationCode: String?
    public var activeSyncStatusMessage: String?
    public var pendingDeviceSyncRequests: [FabushiPendingDeviceSyncItemDTO] = []

    // Feedback
    public var isBusy: Bool = false
    public var errorMessage: String?
    public var statusMessage: String?

    public init(
        coordinator: FabushiCoordinator,
        securityKeychain: FabushiSecurityKeychainStore = .shared,
        nowProvider: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.coordinator = coordinator
        self.securityKeychain = securityKeychain
        self.nowProvider = nowProvider
        self.isUnlocked = coordinator.vaultStore.isUnlocked
    }

    deinit {
        taskBag.cancelAll()
    }

    public func stopTotpTicker() {
        taskBag.cancel(key: "totpTicker")
    }

    public var currentUserId: UInt64? {
        if case .authenticated(let uid, _, _) = coordinator.authState {
            return uid
        }
        return nil
    }

    public var currentAccountId: String? {
        if case .authenticated(_, _, let accountId) = coordinator.authState {
            return accountId
        }
        return coordinator.activeAccountId
    }

    public var filteredItems: [FabushiVaultDecryptedItem] {
        items.filter { $0.category == selectedCategory.rawValue }
    }

    // MARK: - Vault Lock & Unlock (Real Rust Crypto + Keychain VaultRootKey)

    public func refreshVaultStatus() async {
        guard coordinator.isStarted else { return }
        if let status: FabushiVaultUnlockStatusResponse = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("status"),
            params: FabushiEmptyParams()
        ) {
            isUnlocked = status.unlocked
            coordinator.vaultStore.setUnlocked(status.unlocked)
            if status.unlocked {
                await loadUnlockedVaultContents()
            }
        }
    }

    /// Unlocks the zero-knowledge vault using the account's Keychain-stored 32-byte `VaultRootKey`,
    /// an explicit Split-Key password / passphrase / Emergency Kit, or provisions a fresh 32-byte
    /// CSPRNG device-bound `VaultRootKey` into Keychain (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`)
    /// and unlocks real Rust `bot-browser` vault crypto via `vault.unlockWithKeyBytes`.
    public func unlockVault(
        passphraseOrPassword: String? = nil,
        emergencyKit: String? = nil
    ) async {
        errorMessage = nil
        statusMessage = nil
        guard coordinator.isStarted else {
            errorMessage = FabushiCoordinatorError.notStarted.localizedDescription
            return
        }
        guard let userId = currentUserId, let accountId = currentAccountId else {
            errorMessage = FabushiVaultValidationError.unauthenticated.localizedDescription
            return
        }

        isBusy = true
        defer { isBusy = false }

        do {
            let currentStatus: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("status"),
                params: FabushiEmptyParams()
            )
            if currentStatus.unlocked {
                isUnlocked = true
                coordinator.vaultStore.setUnlocked(true)
                await loadUnlockedVaultContents()
                return
            }

            if let kit = emergencyKit?.trimmingCharacters(in: .whitespacesAndNewlines), !kit.isEmpty {
                let envResp: FabushiVaultEnvelopeResponseDTO = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("envelopeGet"),
                    params: FabushiEmptyParams()
                )
                let nested = envResp.envelope.map(FabushiVaultEnvelopeConverter.toNested)
                let _: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("unlockWithRecoveryKey"),
                    params: FabushiVaultUnlockRecoveryRequest(
                        userId: userId,
                        recoveryKey: kit,
                        envelope: nested
                    )
                )
            } else if let secret = passphraseOrPassword, !secret.isEmpty {
                let envResp: FabushiVaultEnvelopeResponseDTO = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("envelopeGet"),
                    params: FabushiEmptyParams()
                )
                if let flatEnv = envResp.envelope {
                    let nested = FabushiVaultEnvelopeConverter.toNested(flatEnv)
                    if flatEnv.kdfAlg == "split_key_v2" {
                        let res: FabushiVaultUnlockWithSplitKeyResponse = try await coordinator.dispatchFenced(
                            method: FabushiRPCNamespace.vault.method("unlockWithSplitKey"),
                            params: FabushiVaultUnlockWithSplitKeyRequest(
                                userId: userId,
                                envelope: nested,
                                vaultKekB64: nil,
                                password: secret
                            )
                        )
                        try securityKeychain.saveVaultRootKeyBase64(res.rootKeyB64, accountId: accountId)
                    } else {
                        let _: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                            method: FabushiRPCNamespace.vault.method("unlockWithPassphrase"),
                            params: FabushiVaultUnlockPassphraseRequest(
                                userId: userId,
                                passphrase: secret,
                                envelope: nested
                            )
                        )
                    }
                } else {
                    let initRes: FabushiVaultInitSplitKeyResponse = try await coordinator.dispatchFenced(
                        method: FabushiRPCNamespace.vault.method("initSplitKey"),
                        params: FabushiVaultInitSplitKeyRequest(
                            userId: userId,
                            password: secret,
                            saltB64: nil,
                            kdfParams: nil,
                            fastKdf: nil
                        )
                    )
                    try securityKeychain.saveVaultRootKeyBase64(initRes.rootKeyB64, accountId: accountId)
                }
            } else if let storedRootKeyB64 = try securityKeychain.loadVaultRootKeyBase64(accountId: accountId) {
                let _: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("unlockWithKeyBytes"),
                    params: FabushiVaultUnlockWithKeyBytesRequest(rootKeyB64: storedRootKeyB64)
                )
            } else {
                // First-time device-bound Keychain VaultRootKey provisioning (real 32-byte CSPRNG -> Keychain -> Rust).
                let freshKeyData = try FabushiPasscodeCrypto.generateRandomBytes(count: 32)
                let freshKeyB64 = freshKeyData.base64EncodedString()
                try securityKeychain.saveVaultRootKeyBase64(freshKeyB64, accountId: accountId)
                let _: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("unlockWithKeyBytes"),
                    params: FabushiVaultUnlockWithKeyBytesRequest(rootKeyB64: freshKeyB64)
                )
            }

            isUnlocked = true
            coordinator.vaultStore.setUnlocked(true)
            await loadUnlockedVaultContents()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    public func lockVault() async {
        stopTotpTicker()
        items = []
        liveTotpCodes = [:]
        remoteEncryptedTotpCount = 0
        isUnlocked = false
        coordinator.vaultStore.setUnlocked(false)

        if coordinator.isStarted {
            let _: FabushiJSONValue? = try? await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiEmptyParams()
            )
        }
    }

    // MARK: - Synced Encrypted Items CRUD (24 MiB Bound + AAD Revision Binding)

    public func loadUnlockedVaultContents() async {
        if let accountId = currentAccountId {
            totpAccounts = (try? securityKeychain.loadTotpAccounts(accountId: accountId)) ?? []
        }
        await pullSyncedItems()
        await refreshAllTotpCodes()
        startTotpTickerIfNeeded()
    }

    public func pullSyncedItems() async {
        guard coordinator.isStarted, let userId = currentUserId else { return }

        do {
            let pullResp: FabushiVaultSyncPullResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("syncPull"),
                params: FabushiVaultSyncPullRequest(sinceRevision: nil, category: nil)
            )
            var decoded: [FabushiVaultDecryptedItem] = []
            var remoteTotpCount = 0

            for blob in pullResp.items where !blob.deleted {
                if blob.category == "totp" {
                    remoteTotpCount += 1
                    continue
                }
                if FabushiVaultCategory(rawValue: blob.category) != nil,
                   let opened: FabushiVaultOpenItemResponse = try? await coordinator.dispatchFenced(
                       method: FabushiRPCNamespace.vault.method("openItem"),
                       params: FabushiVaultOpenItemRequest(userId: userId, blob: blob)
                   ) {
                    decoded.append(
                        FabushiVaultDecryptedItem(
                            category: blob.category,
                            itemKey: blob.itemKey,
                            revision: blob.revision,
                            plaintext: opened.plaintextUtf8,
                            updatedAtMs: blob.updatedAtMs,
                            isOpaqueRemote: false
                        )
                    )
                } else {
                    decoded.append(
                        FabushiVaultDecryptedItem(
                            category: blob.category,
                            itemKey: blob.itemKey,
                            revision: blob.revision,
                            plaintext: nil,
                            updatedAtMs: blob.updatedAtMs,
                            isOpaqueRemote: true
                        )
                    )
                }
            }
            items = decoded
            remoteEncryptedTotpCount = remoteTotpCount
        } catch {
            // Non-fatal if remote sync pull fails offline; local unlocked vault remains accessible.
        }
    }

    public func saveEncryptedItem(
        category: FabushiVaultCategory,
        itemKey rawKey: String,
        plaintext: String
    ) async throws -> FabushiVaultSyncPushResponse {
        guard isUnlocked else {
            throw FabushiVaultValidationError.vaultLocked
        }
        guard let userId = currentUserId else {
            throw FabushiVaultValidationError.unauthenticated
        }

        // Fail-closed strict key & 24 MiB payload bound BEFORE calling Rust sealItem / syncPush!
        let validKey = try FabushiVaultLimits.validateItemKey(rawKey)
        try FabushiVaultLimits.validatePayloadSize(plaintext)

        let existingRevision = items
            .first(where: { $0.category == category.rawValue && $0.itemKey == validKey })?
            .revision ?? 0
        let nextRevision = existingRevision + 1

        let sealed: FabushiEncryptedVaultBlobDTO = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("sealItem"),
            params: FabushiVaultSealItemRequest(
                userId: userId,
                category: category.rawValue,
                itemKey: validKey,
                revision: nextRevision,
                plaintext: plaintext
            )
        )

        let pushResp: FabushiVaultSyncPushResponse = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("syncPush"),
            params: FabushiVaultSyncPushRequest(
                category: category.rawValue,
                itemKey: validKey,
                expectedRevision: existingRevision > 0 ? existingRevision : nil,
                nonceB64: sealed.nonceB64,
                ciphertextB64: sealed.ciphertextB64,
                deleted: false
            )
        )

        let updatedItem = FabushiVaultDecryptedItem(
            category: category.rawValue,
            itemKey: validKey,
            revision: pushResp.revision,
            plaintext: plaintext,
            updatedAtMs: pushResp.updatedAtMs,
            isOpaqueRemote: false
        )
        items.removeAll { $0.id == updatedItem.id }
        items.append(updatedItem)
        return pushResp
    }

    public func deleteEncryptedItem(_ item: FabushiVaultDecryptedItem) async throws {
        guard isUnlocked else {
            throw FabushiVaultValidationError.vaultLocked
        }
        let _: FabushiVaultSyncPushResponse = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("syncDelete"),
            params: FabushiVaultSyncDeleteRequest(
                category: item.category,
                itemKey: item.itemKey,
                expectedRevision: item.revision
            )
        )
        items.removeAll { $0.id == item.id }
    }

    // MARK: - Local Origin-Scoped Credential Vault (`vault.storeCredential`, `listForOrigin`, `decryptForOrigin`, `deleteCredential`)

    public struct LocalCredentialMetaDTO: Codable, Sendable, Equatable, Identifiable {
        public var id: String { credentialId }
        public let credentialId: String
        public let origin: String
        public let accountHint: String
        public let hasTotp: Bool
        public let reusePolicy: String
        public let createdAtMs: Int64
        public let lastUsedAtMs: Int64?

        public enum CodingKeys: String, CodingKey {
            case credentialId = "credential_id"
            case origin
            case accountHint = "account_hint"
            case hasTotp = "has_totp"
            case reusePolicy = "reuse_policy"
            case createdAtMs = "created_at_ms"
            case lastUsedAtMs = "last_used_at_ms"
        }
    }

    public struct DecryptedOriginCredentialDTO: Codable, Sendable, Equatable {
        public let credentialId: String
        public let origin: String
        public let username: String
        public let password: String
        public let hasTotp: Bool
        public let totpCode: String?

        public enum CodingKeys: String, CodingKey {
            case credentialId = "credential_id"
            case origin
            case username
            case password
            case hasTotp = "has_totp"
            case totpCode = "totp_code"
        }
    }

    private struct StoreCredentialParams: Encodable, Sendable {
        let origin: String
        let username: String
        let password: String
        let totpSecret: String?
        let accountHint: String?
        let reusePolicy: String?

        enum CodingKeys: String, CodingKey {
            case origin
            case username
            case password
            case totpSecret = "totp_secret"
            case accountHint = "account_hint"
            case reusePolicy = "reuse_policy"
        }
    }

    private struct ListForOriginParams: Encodable, Sendable {
        let origin: String
    }

    private struct DecryptForOriginParams: Encodable, Sendable {
        let credentialId: String
        let origin: String
        let frameOrigin: String?

        enum CodingKeys: String, CodingKey {
            case credentialId = "credential_id"
            case origin
            case frameOrigin = "frame_origin"
        }
    }

    private struct DeleteCredentialParams: Encodable, Sendable {
        let credentialId: String

        enum CodingKeys: String, CodingKey {
            case credentialId = "credential_id"
        }
    }

    private struct DeleteCredentialResult: Decodable, Sendable {
        let deleted: Bool
        let credentialId: String

        enum CodingKeys: String, CodingKey {
            case deleted
            case credentialId = "credential_id"
        }
    }

    public func storeLocalOriginCredential(
        origin: String,
        username: String,
        password: String,
        totpSecret: String? = nil,
        accountHint: String? = nil,
        reusePolicy: String? = nil
    ) async throws -> LocalCredentialMetaDTO {
        guard isUnlocked else { throw FabushiVaultValidationError.vaultLocked }
        try FabushiVaultLimits.validatePayloadSize(password)
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("storeCredential"),
            params: StoreCredentialParams(
                origin: origin,
                username: username,
                password: password,
                totpSecret: totpSecret,
                accountHint: accountHint,
                reusePolicy: reusePolicy
            )
        )
    }

    public func listLocalOriginCredentials(origin: String) async throws -> [LocalCredentialMetaDTO] {
        guard isUnlocked else { throw FabushiVaultValidationError.vaultLocked }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("listForOrigin"),
            params: ListForOriginParams(origin: origin)
        )
    }

    public func decryptLocalOriginCredential(
        credentialId: String,
        origin: String,
        frameOrigin: String? = nil
    ) async throws -> DecryptedOriginCredentialDTO {
        guard isUnlocked else { throw FabushiVaultValidationError.vaultLocked }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("decryptForOrigin"),
            params: DecryptForOriginParams(
                credentialId: credentialId,
                origin: origin,
                frameOrigin: frameOrigin
            )
        )
    }

    public func deleteLocalOriginCredential(credentialId: String) async throws -> Bool {
        guard isUnlocked else { throw FabushiVaultValidationError.vaultLocked }
        let res: DeleteCredentialResult = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("deleteCredential"),
            params: DeleteCredentialParams(credentialId: credentialId)
        )
        return res.deleted
    }

    // MARK: - RFC 6238 TOTP Authenticator (Real Rust `vault.totpGenerate`)

    public func generateTotpCode(
        secret: String,
        unixSecs: UInt64? = nil,
        digits: UInt64? = nil,
        period: UInt64? = nil
    ) async throws -> FabushiTotpGenerateResponseDTO {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("totpGenerate"),
            params: FabushiTotpGenerateRequestDTO(
                secret: secret,
                unixSecs: unixSecs,
                digits: digits,
                period: period
            )
        )
    }

    public func addTotpAccount(
        label: String,
        issuer: String = "",
        secretOrURI: String
    ) async throws {
        guard let accountId = currentAccountId else {
            throw FabushiVaultValidationError.unauthenticated
        }
        guard let parsed = FabushiTotpURIInfo.parse(
            secretOrURI,
            fallbackLabel: label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Authenticator" : label
        ) else {
            throw FabushiHostError.invalidArguments(message: "Invalid TOTP secret or otpauth:// URI.")
        }

        // Validate against real Rust RFC 6238 engine before persisting in Keychain.
        let nowSecs = UInt64(max(0, Int64(nowProvider().timeIntervalSince1970)))
        let generated = try await generateTotpCode(
            secret: parsed.secretOrURI,
            unixSecs: nowSecs,
            digits: UInt64(parsed.digits),
            period: UInt64(parsed.period)
        )

        let entry = FabushiStoredTotpAccount(
            label: label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? parsed.label : label,
            issuer: issuer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? parsed.issuer : issuer,
            secret: parsed.secretOrURI,
            digits: parsed.digits,
            period: parsed.period
        )
        totpAccounts.append(entry)
        try securityKeychain.saveTotpAccounts(totpAccounts, accountId: accountId)
        liveTotpCodes[entry.id] = generated.code
        totpRemainingSeconds = FabushiTotpURIInfo.remainingSeconds(at: Int64(nowSecs), period: entry.period)
        startTotpTickerIfNeeded()
    }

    public func deleteTotpAccount(id: String) throws {
        guard let accountId = currentAccountId else {
            throw FabushiVaultValidationError.unauthenticated
        }
        totpAccounts.removeAll { $0.id == id }
        liveTotpCodes.removeValue(forKey: id)
        try securityKeychain.saveTotpAccounts(totpAccounts, accountId: accountId)
    }

    public func refreshAllTotpCodes() async {
        guard coordinator.isStarted else { return }
        let nowSecs = Int64(nowProvider().timeIntervalSince1970)
        totpRemainingSeconds = FabushiTotpURIInfo.remainingSeconds(at: nowSecs, period: 30)

        for entry in totpAccounts {
            if let res = try? await generateTotpCode(
                secret: entry.secret,
                unixSecs: UInt64(max(0, nowSecs)),
                digits: UInt64(entry.digits),
                period: UInt64(entry.period)
            ) {
                liveTotpCodes[entry.id] = res.code
            }
        }
    }

    private func startTotpTickerIfNeeded() {
        guard isUnlocked else {
            stopTotpTicker()
            return
        }
        let task = Task<Void, Never> { @MainActor [weak self] in
            var lastWindow: Int64 = -1
            while let self, !Task.isCancelled, self.isUnlocked {
                let nowSecs = Int64(self.nowProvider().timeIntervalSince1970)
                let remaining = FabushiTotpURIInfo.remainingSeconds(at: nowSecs, period: 30)
                self.totpRemainingSeconds = remaining
                let currentWindow = nowSecs / 30
                if currentWindow != lastWindow {
                    lastWindow = currentWindow
                    await self.refreshAllTotpCodes()
                }
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
        taskBag.replace(key: "totpTicker", with: task)
    }

    // MARK: - X25519 Cross-Device Trust Ring Sync

    public func startDeviceSyncRequest(deviceName: String = "Fabushi iOS") async throws -> FabushiDeviceSyncRequestResponseDTO {
        let keygen: FabushiDeviceSyncKeygenResponse = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("deviceSyncKeygen"),
            params: FabushiDeviceSyncKeygenRequest(pairId: nil)
        )
        let reqResp: FabushiDeviceSyncRequestResponseDTO = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("deviceSyncRequest"),
            params: FabushiDeviceSyncRequestBodyDTO(
                deviceName: deviceName,
                devicePublicKeyB64: keygen.publicKeyB64
            )
        )
        activeSyncPairId = keygen.pairId
        activeSyncRequestId = reqResp.requestId
        activeSyncVerificationCode = keygen.verificationCode
        activeSyncStatusMessage = "pending"
        return reqResp
    }

    public func checkDeviceSyncStatusAndOpenIfApproved() async throws -> String {
        guard let requestId = activeSyncRequestId,
              let pairId = activeSyncPairId,
              let userId = currentUserId,
              let accountId = currentAccountId
        else {
            throw FabushiVaultValidationError.unauthenticated
        }

        let statusResp: FabushiDeviceSyncStatusResponseDTO = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("deviceSyncStatus"),
            params: FabushiDeviceSyncStatusRequestDTO(requestId: requestId)
        )
        activeSyncStatusMessage = statusResp.status

        if statusResp.status == "approved",
           let ephPub = statusResp.approverEphemeralPubB64,
           let encRoot = statusResp.encryptedRootKeyB64,
           let nonce = statusResp.nonceB64 {
            let openResp: FabushiDeviceSyncOpenResponseDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("deviceSyncOpen"),
                params: FabushiDeviceSyncOpenRequestDTO(
                    pairId: pairId,
                    requestId: requestId,
                    userId: userId,
                    transfer: FabushiDeviceKeyTransferDTO(
                        approverEphemeralPubB64: ephPub,
                        encryptedRootKeyB64: encRoot,
                        nonceB64: nonce
                    )
                )
            )
            try securityKeychain.saveVaultRootKeyBase64(openResp.rootKeyB64, accountId: accountId)
            isUnlocked = openResp.unlocked
            coordinator.vaultStore.setUnlocked(openResp.unlocked)
            await loadUnlockedVaultContents()
        }
        return statusResp.status
    }

    public func loadPendingDeviceSyncRequests() async throws {
        let resp: FabushiPendingDeviceSyncListResponseDTO = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("deviceSyncPending"),
            params: FabushiEmptyParams()
        )
        pendingDeviceSyncRequests = resp.items
    }

    public func approvePendingDeviceSync(_ item: FabushiPendingDeviceSyncItemDTO) async throws -> FabushiDeviceSyncActionResponseDTO {
        guard isUnlocked else {
            throw FabushiVaultValidationError.vaultLocked
        }
        guard let userId = currentUserId else {
            throw FabushiVaultValidationError.unauthenticated
        }
        let sealed: FabushiDeviceSyncSealResponseDTO = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("deviceSyncSeal"),
            params: FabushiDeviceSyncSealRequestDTO(
                requestId: item.requestId,
                userId: userId,
                targetDevicePubB64: item.devicePublicKeyB64
            )
        )
        let approved: FabushiDeviceSyncActionResponseDTO = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("deviceSyncApprove"),
            params: FabushiDeviceSyncApproveRequestDTO(
                requestId: item.requestId,
                approverEphemeralPubB64: sealed.transfer.approverEphemeralPubB64,
                encryptedRootKeyB64: sealed.transfer.encryptedRootKeyB64,
                nonceB64: sealed.transfer.nonceB64
            )
        )
        pendingDeviceSyncRequests.removeAll { $0.requestId == item.requestId }
        return approved
    }

    public func denyPendingDeviceSync(_ item: FabushiPendingDeviceSyncItemDTO) async throws -> FabushiDeviceSyncActionResponseDTO {
        let denied: FabushiDeviceSyncActionResponseDTO = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("deviceSyncDeny"),
            params: FabushiDeviceSyncDenyRequestDTO(requestId: item.requestId)
        )
        pendingDeviceSyncRequests.removeAll { $0.requestId == item.requestId }
        return denied
    }
}
