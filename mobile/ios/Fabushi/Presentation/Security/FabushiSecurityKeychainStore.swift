import CryptoKit
import Foundation
import Security

// MARK: - Keychain Errors & Models (Zero UserDefaults Secret Storage)

public enum FabushiSecurityKeychainError: LocalizedError, Sendable, Equatable {
    case invalidVaultRootKeyLength(Int)
    case encodingFailed
    case decodingFailed
    case randomGenerationFailed(OSStatus)
    case osStatus(OSStatus)

    public var errorDescription: String? {
        switch self {
        case .invalidVaultRootKeyLength(let count):
            return "Vault root key must be exactly 32 bytes (received \(count))."
        case .encodingFailed:
            return "Failed to encode security record for iOS Keychain."
        case .decodingFailed:
            return "Failed to decode security record from iOS Keychain."
        case .randomGenerationFailed(let status):
            return "OS CSPRNG failed with status \(status)."
        case .osStatus(let status):
            return "iOS Keychain operation failed (OSStatus: \(status))."
        }
    }
}

public struct FabushiPasscodeRecord: Codable, Sendable, Equatable {
    public var isEnabled: Bool
    public var saltB64: String
    public var hashB64: String
    public var iterations: Int
    public var biometricsEnabled: Bool
    public var autoLockTimeoutSeconds: Int
    public var failedAttempts: Int
    public var lockoutUntilEpochMs: Int64?

    public init(
        isEnabled: Bool,
        saltB64: String,
        hashB64: String,
        iterations: Int,
        biometricsEnabled: Bool = false,
        autoLockTimeoutSeconds: Int = 300,
        failedAttempts: Int = 0,
        lockoutUntilEpochMs: Int64? = nil
    ) {
        self.isEnabled = isEnabled
        self.saltB64 = saltB64
        self.hashB64 = hashB64
        self.iterations = iterations
        self.biometricsEnabled = biometricsEnabled
        self.autoLockTimeoutSeconds = autoLockTimeoutSeconds
        self.failedAttempts = failedAttempts
        self.lockoutUntilEpochMs = lockoutUntilEpochMs
    }
}

public struct FabushiStoredTotpAccount: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    public var label: String
    public var issuer: String
    public var secret: String
    public var digits: Int
    public var period: Int
    public var createdAtMs: Int64

    public init(
        id: String = UUID().uuidString,
        label: String,
        issuer: String = "",
        secret: String,
        digits: Int = 6,
        period: Int = 30,
        createdAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) {
        self.id = id
        self.label = label
        self.issuer = issuer
        self.secret = secret
        self.digits = digits
        self.period = period
        self.createdAtMs = createdAtMs
    }
}

// MARK: - Pure Swift CryptoKit PBKDF2-HMAC-SHA256 & Constant-Time Comparison

public enum FabushiPasscodeCrypto: Sendable {
    public static let productionIterations: Int = 100_000
    public static let testIterations: Int = 1_000
    public static let saltByteLength: Int = 16
    public static let derivedKeyLength: Int = 32

    public static func generateRandomBytes(count: Int) throws -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        let status = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        guard status == errSecSuccess else {
            throw FabushiSecurityKeychainError.randomGenerationFailed(status)
        }
        return Data(bytes)
    }

    /// RFC 2898 PBKDF2-HMAC-SHA256 producing a single 32-byte block (`dkLen == 32`).
    public static func derivePBKDF2SHA256(
        passcode: String,
        salt: Data,
        iterations: Int
    ) -> Data {
        let rounds = max(1, iterations)
        let key = SymmetricKey(data: Data(passcode.utf8))

        // Block index = 1 (big-endian UInt32: 0x00000001)
        var saltWithIndex = salt
        saltWithIndex.append(contentsOf: [0x00, 0x00, 0x00, 0x01])

        var u = Data(HMAC<SHA256>.authenticationCode(for: saltWithIndex, using: key))
        var result = [UInt8](u)

        if rounds > 1 {
            for _ in 1..<rounds {
                u = Data(HMAC<SHA256>.authenticationCode(for: u, using: key))
                let uBytes = [UInt8](u)
                for idx in 0..<result.count {
                    result[idx] ^= uBytes[idx]
                }
            }
        }
        return Data(result)
    }

    public static func constantTimeEqual(_ lhs: Data, _ rhs: Data) -> Bool {
        guard lhs.count == rhs.count else { return false }
        var diff: UInt8 = 0
        for (a, b) in zip(lhs, rhs) {
            diff |= (a ^ b)
        }
        return diff == 0
    }
}

// MARK: - Dedicated iOS Security.framework Keychain Store for M2b Passcode, Vault Root Keys & TOTP

/// Thread-safe `Security.framework` Keychain store exclusively for M2b Passcode Lock records,
/// per-account 32-byte E2EE `VaultRootKey` blobs, and per-account encrypted-at-rest TOTP entries.
///
/// Strictly enforces `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` and ZERO `UserDefaults` storage.
public final class FabushiSecurityKeychainStore: Sendable {
    public static let shared = FabushiSecurityKeychainStore()

    public static let defaultPasscodeService = "com.ombhrum.fabushi.passcode.v1"
    public static let defaultPasscodeAccount = "local-passcode-record-v1"
    public static let defaultVaultKeyService = "com.ombhrum.fabushi.vault-root-key.v1"
    public static let defaultTotpService = "com.ombhrum.fabushi.vault-totp.v1"

    public let passcodeService: String
    public let passcodeAccount: String
    public let vaultKeyService: String
    public let totpService: String

    public init(
        passcodeService: String = FabushiSecurityKeychainStore.defaultPasscodeService,
        passcodeAccount: String = FabushiSecurityKeychainStore.defaultPasscodeAccount,
        vaultKeyService: String = FabushiSecurityKeychainStore.defaultVaultKeyService,
        totpService: String = FabushiSecurityKeychainStore.defaultTotpService
    ) {
        self.passcodeService = passcodeService
        self.passcodeAccount = passcodeAccount
        self.vaultKeyService = vaultKeyService
        self.totpService = totpService
    }

    // MARK: - Local Passcode Record

    public func loadPasscodeRecord() throws -> FabushiPasscodeRecord? {
        guard let data = try readData(service: passcodeService, account: passcodeAccount) else {
            return nil
        }
        do {
            return try JSONDecoder().decode(FabushiPasscodeRecord.self, from: data)
        } catch {
            throw FabushiSecurityKeychainError.decodingFailed
        }
    }

    public func savePasscodeRecord(_ record: FabushiPasscodeRecord) throws {
        let data: Data
        do {
            data = try JSONEncoder().encode(record)
        } catch {
            throw FabushiSecurityKeychainError.encodingFailed
        }
        try writeData(data, service: passcodeService, account: passcodeAccount)
    }

    public func deletePasscodeRecord() throws {
        try deleteItem(service: passcodeService, account: passcodeAccount)
    }

    // MARK: - Per-Account 32-Byte VaultRootKey

    public func saveVaultRootKeyBase64(_ rootKeyB64: String, accountId: String) throws {
        let trimmed = rootKeyB64.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let rawKey = Data(base64Encoded: trimmed) else {
            throw FabushiSecurityKeychainError.invalidVaultRootKeyLength(0)
        }
        guard rawKey.count == 32 else {
            throw FabushiSecurityKeychainError.invalidVaultRootKeyLength(rawKey.count)
        }
        try writeData(rawKey, service: vaultKeyService, account: vaultKeyAccount(for: accountId))
    }

    public func loadVaultRootKeyBase64(accountId: String) throws -> String? {
        guard let rawKey = try readData(service: vaultKeyService, account: vaultKeyAccount(for: accountId)) else {
            return nil
        }
        guard rawKey.count == 32 else {
            throw FabushiSecurityKeychainError.invalidVaultRootKeyLength(rawKey.count)
        }
        return rawKey.base64EncodedString()
    }

    public func hasVaultRootKey(accountId: String) -> Bool {
        (try? loadVaultRootKeyBase64(accountId: accountId)) != nil
    }

    public func deleteVaultRootKey(accountId: String) throws {
        try deleteItem(service: vaultKeyService, account: vaultKeyAccount(for: accountId))
    }

    // MARK: - Per-Account Keychain-Protected TOTP Secrets

    public func loadTotpAccounts(accountId: String) throws -> [FabushiStoredTotpAccount] {
        guard let data = try readData(service: totpService, account: totpAccount(for: accountId)) else {
            return []
        }
        do {
            return try JSONDecoder().decode([FabushiStoredTotpAccount].self, from: data)
        } catch {
            throw FabushiSecurityKeychainError.decodingFailed
        }
    }

    public func saveTotpAccounts(_ accounts: [FabushiStoredTotpAccount], accountId: String) throws {
        let data: Data
        do {
            data = try JSONEncoder().encode(accounts)
        } catch {
            throw FabushiSecurityKeychainError.encodingFailed
        }
        try writeData(data, service: totpService, account: totpAccount(for: accountId))
    }

    public func deleteTotpAccounts(accountId: String) throws {
        try deleteItem(service: totpService, account: totpAccount(for: accountId))
    }

    /// Purges all account-scoped Keychain secrets (E2EE VaultRootKey + TOTP entries) on logout.
    public func purgeAccountSecrets(accountId: String) {
        try? deleteVaultRootKey(accountId: accountId)
        try? deleteTotpAccounts(accountId: accountId)
    }

    // MARK: - Low-Level SecItem Helpers

    private func vaultKeyAccount(for accountId: String) -> String {
        "vault-root-key-\(accountId)"
    }

    private func totpAccount(for accountId: String) -> String {
        "vault-totp-\(accountId)"
    }

    private func readData(service: String, account: String) throws -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw FabushiSecurityKeychainError.osStatus(status)
        }
        guard let data = item as? Data else {
            throw FabushiSecurityKeychainError.decodingFailed
        }
        return data
    }

    private func writeData(_ data: Data, service: String, account: String) throws {
        let addQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecValueData as String: data,
        ]
        let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
        if addStatus == errSecSuccess {
            return
        }
        if addStatus == errSecDuplicateItem {
            let matchQuery: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecAttrAccount as String: account,
            ]
            let updateAttributes: [String: Any] = [
                kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
                kSecValueData as String: data,
            ]
            let updateStatus = SecItemUpdate(matchQuery as CFDictionary, updateAttributes as CFDictionary)
            guard updateStatus == errSecSuccess else {
                throw FabushiSecurityKeychainError.osStatus(updateStatus)
            }
            return
        }
        throw FabushiSecurityKeychainError.osStatus(addStatus)
    }

    private func deleteItem(service: String, account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw FabushiSecurityKeychainError.osStatus(status)
        }
    }
}
