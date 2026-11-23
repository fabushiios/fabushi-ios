import Foundation
import Security

// MARK: - Fail-Closed Keychain Errors

public enum FabushiKeychainError: LocalizedError, Sendable, Equatable {
    case osStatus(OSStatus)
    case invalidStoredLength(expected: Int, actual: Int)
    case randomGenerationFailed(OSStatus)

    public var errorDescription: String? {
        switch self {
        case .osStatus(let status):
            return "iOS Keychain operation failed with OSStatus \(status)."
        case .invalidStoredLength(let expected, let actual):
            return "Stored Keychain secret had invalid byte length \(actual) (expected \(expected))."
        case .randomGenerationFailed(let status):
            return "SecRandomCopyBytes failed with OSStatus \(status)."
        }
    }
}

// MARK: - Production Security.framework Keychain Store (Zero Fallbacks)

/// Fail-closed iOS Keychain store for 32-byte cryptographically random storage passphrases
/// and per-account secret key material.
///
/// Guarantees:
/// - Uses `SecRandomCopyBytes(kSecRandomDefault, 32, ...)` — zero static/repeating byte keys.
/// - Stores items under `kSecClassGenericPassword` with
///   `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`.
/// - Handles concurrent `errSecDuplicateItem` insert races via atomic re-read.
/// - Propagates all unexpected `OSStatus` errors fail-closed (zero `""` or `UserDefaults` fallback).
public final class FabushiKeychainStore: Sendable {
    public static let defaultService = "com.ombhrum.fabushi.storage-passphrase.v1"
    public static let defaultAccount = "default"
    public static let passphraseByteLength = 32

    public static let shared = FabushiKeychainStore()

    public let service: String
    public let account: String

    public init(
        service: String = defaultService,
        account: String = defaultAccount
    ) {
        self.service = service
        self.account = account
    }

    // MARK: - Primary Storage Passphrase (Base64-Encoded 32 Random Bytes)

    public func loadOrCreateStoragePassphrase() throws -> String {
        let data = try loadOrCreateSecretBytes(accountKey: account)
        return data.base64EncodedString()
    }

    public func readPassphrase() throws -> String? {
        guard let data = try readSecretBytes(accountKey: account) else {
            return nil
        }
        return data.base64EncodedString()
    }

    public func deletePassphrase() throws {
        try deleteSecretBytes(accountKey: account)
    }

    // MARK: - Per-Account Scoped Secret Material

    public func loadOrCreateAccountKey(accountId: String) throws -> Data {
        try loadOrCreateSecretBytes(accountKey: scopedAccountKey(accountId))
    }

    public func hasAccountKey(accountId: String) throws -> Bool {
        try readSecretBytes(accountKey: scopedAccountKey(accountId)) != nil
    }

    public func deleteAccountKey(accountId: String) throws {
        try deleteSecretBytes(accountKey: scopedAccountKey(accountId))
    }

    // MARK: - Internal Fail-Closed Security.framework Primitives

    private func scopedAccountKey(_ accountId: String) -> String {
        "\(account).acct.\(accountId)"
    }

    private func readSecretBytes(accountKey: String) throws -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountKey,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw FabushiKeychainError.osStatus(status)
        }
        guard let data = result as? Data else {
            throw FabushiKeychainError.invalidStoredLength(
                expected: Self.passphraseByteLength,
                actual: 0
            )
        }
        guard data.count == Self.passphraseByteLength else {
            throw FabushiKeychainError.invalidStoredLength(
                expected: Self.passphraseByteLength,
                actual: data.count
            )
        }
        return data
    }

    private func loadOrCreateSecretBytes(accountKey: String) throws -> Data {
        if let existing = try readSecretBytes(accountKey: accountKey) {
            return existing
        }

        var bytes = [UInt8](repeating: 0, count: Self.passphraseByteLength)
        let randomStatus = bytes.withUnsafeMutableBytes { rawBuffer -> OSStatus in
            guard let baseAddress = rawBuffer.baseAddress else {
                return errSecParam
            }
            return SecRandomCopyBytes(kSecRandomDefault, rawBuffer.count, baseAddress)
        }
        guard randomStatus == errSecSuccess else {
            throw FabushiKeychainError.randomGenerationFailed(randomStatus)
        }

        let secretData = Data(bytes)
        let addQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountKey,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecValueData as String: secretData,
        ]

        let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
        if addStatus == errSecSuccess {
            return secretData
        }
        if addStatus == errSecDuplicateItem {
            // Concurrent writer won the race; re-read the authoritative stored item.
            if let raced = try readSecretBytes(accountKey: accountKey) {
                return raced
            }
        }
        throw FabushiKeychainError.osStatus(addStatus)
    }

    private func deleteSecretBytes(accountKey: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountKey,
        ]
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw FabushiKeychainError.osStatus(status)
        }
    }
}
