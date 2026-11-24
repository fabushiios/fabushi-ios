import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Split-Key V2 & Argon2id DTOs (Exact Match to gpui_telegram_proto::user_vault & handlers_core.rs)

public struct FabushiArgon2KdfParamsDTO: Codable, Sendable, Equatable {
    public let mCostKib: UInt32
    public let tCost: UInt32
    public let pCost: UInt32

    public enum CodingKeys: String, CodingKey {
        case mCostKib = "m_cost_kib"
        case tCost = "t_cost"
        case pCost = "p_cost"
    }

    public init(mCostKib: UInt32 = 65_536, tCost: UInt32 = 3, pCost: UInt32 = 1) {
        self.mCostKib = mCostKib
        self.tCost = tCost
        self.pCost = pCost
    }
}

public struct FabushiKdfParamsRequest: Encodable, Sendable, Equatable {
    public let usernameOrEmail: String

    public enum CodingKeys: String, CodingKey {
        case usernameOrEmail = "username_or_email"
    }

    public init(usernameOrEmail: String) {
        self.usernameOrEmail = usernameOrEmail
    }
}

public struct FabushiKdfParamsResponse: Codable, Sendable, Equatable {
    public let authVersion: Int32
    public let authSaltB64: String
    public let kdfParams: FabushiArgon2KdfParamsDTO

    public enum CodingKeys: String, CodingKey {
        case authVersion = "auth_version"
        case authSaltB64 = "auth_salt_b64"
        case kdfParams = "kdf_params"
    }
}

public struct FabushiDeriveSplitKeyRequest: Encodable, Sendable, Equatable {
    public let password: String
    public let saltB64: String?
    public let kdfParams: FabushiArgon2KdfParamsDTO?
    public let fastKdf: Bool?

    public enum CodingKeys: String, CodingKey {
        case password
        case saltB64 = "salt_b64"
        case kdfParams = "kdf_params"
        case fastKdf = "fast_kdf"
    }

    public init(
        password: String,
        saltB64: String? = nil,
        kdfParams: FabushiArgon2KdfParamsDTO? = nil,
        fastKdf: Bool? = nil
    ) {
        self.password = password
        self.saltB64 = saltB64
        self.kdfParams = kdfParams
        self.fastKdf = fastKdf
    }
}

public struct FabushiDeriveSplitKeyResponse: Codable, Sendable, Equatable {
    public let authSaltB64: String
    public let kdfParams: FabushiArgon2KdfParamsDTO
    public let authVerifierB64: String
    public let vaultKekB64: String

    public enum CodingKeys: String, CodingKey {
        case authSaltB64 = "auth_salt_b64"
        case kdfParams = "kdf_params"
        case authVerifierB64 = "auth_verifier_b64"
        case vaultKekB64 = "vault_kek_b64"
    }
}

/// Wire-compatible Split-Key V2 registration payload.
/// Intentionally keeps `password == nil` for V2 so zero plaintext password is ever serialized on the wire.
public struct FabushiSplitKeyRegisterRequest: Encodable, Sendable, Equatable {
    public let username: String
    public let password: String?
    public let firstName: String
    public let lastName: String?
    public let deviceName: String
    public let autoWelcome: Bool
    public let authVersion: Int32?
    public let authSaltB64: String?
    public let authVerifierB64: String?
    public let kdfParams: FabushiArgon2KdfParamsDTO?

    public enum CodingKeys: String, CodingKey {
        case username
        case password
        case firstName = "first_name"
        case lastName = "last_name"
        case deviceName = "device_name"
        case autoWelcome = "auto_welcome"
        case authVersion = "auth_version"
        case authSaltB64 = "auth_salt_b64"
        case authVerifierB64 = "auth_verifier_b64"
        case kdfParams = "kdf_params"
    }

    public init(
        username: String,
        password: String? = nil,
        firstName: String,
        lastName: String? = nil,
        deviceName: String = "Fabushi iOS",
        autoWelcome: Bool = false,
        authVersion: Int32? = 2,
        authSaltB64: String?,
        authVerifierB64: String?,
        kdfParams: FabushiArgon2KdfParamsDTO?
    ) {
        self.username = username
        self.password = password
        self.firstName = firstName
        self.lastName = lastName
        self.deviceName = deviceName
        self.autoWelcome = autoWelcome
        self.authVersion = authVersion
        self.authSaltB64 = authSaltB64
        self.authVerifierB64 = authVerifierB64
        self.kdfParams = kdfParams
    }
}

public struct FabushiSplitKeyEmailRegisterRequest: Encodable, Sendable, Equatable {
    public let email: String
    public let code: String
    public let username: String
    public let password: String?
    public let firstName: String
    public let lastName: String?
    public let deviceName: String
    public let authVersion: Int32?
    public let authSaltB64: String?
    public let authVerifierB64: String?
    public let kdfParams: FabushiArgon2KdfParamsDTO?

    public enum CodingKeys: String, CodingKey {
        case email
        case code
        case username
        case password
        case firstName = "first_name"
        case lastName = "last_name"
        case deviceName = "device_name"
        case authVersion = "auth_version"
        case authSaltB64 = "auth_salt_b64"
        case authVerifierB64 = "auth_verifier_b64"
        case kdfParams = "kdf_params"
    }

    public init(
        email: String,
        code: String,
        username: String,
        password: String? = nil,
        firstName: String,
        lastName: String? = nil,
        deviceName: String = "Fabushi iOS",
        authVersion: Int32? = 2,
        authSaltB64: String?,
        authVerifierB64: String?,
        kdfParams: FabushiArgon2KdfParamsDTO?
    ) {
        self.email = email
        self.code = code
        self.username = username
        self.password = password
        self.firstName = firstName
        self.lastName = lastName
        self.deviceName = deviceName
        self.authVersion = authVersion
        self.authSaltB64 = authSaltB64
        self.authVerifierB64 = authVerifierB64
        self.kdfParams = kdfParams
    }
}

public struct FabushiSplitKeyLoginRequest: Encodable, Sendable, Equatable {
    public let username: String
    public let password: String
    public let deviceName: String
    public let authVerifierB64: String

    public enum CodingKeys: String, CodingKey {
        case username
        case password
        case deviceName = "device_name"
        case authVerifierB64 = "auth_verifier_b64"
    }

    public init(
        username: String,
        deviceName: String = "Fabushi iOS",
        authVerifierB64: String
    ) {
        self.username = username
        self.password = ""
        self.deviceName = deviceName
        self.authVerifierB64 = authVerifierB64
    }
}

public struct FabushiLegacyLoginRequest: Encodable, Sendable, Equatable {
    public let username: String
    public let password: String
    public let deviceName: String

    public enum CodingKeys: String, CodingKey {
        case username
        case password
        case deviceName = "device_name"
    }

    public init(username: String, password: String, deviceName: String = "Fabushi iOS") {
        self.username = username
        self.password = password
        self.deviceName = deviceName
    }
}

public struct FabushiUpgradeSplitKeyRequest: Encodable, Sendable, Equatable {
    public let oldPassword: String
    public let authSaltB64: String
    public let authVerifierB64: String
    public let kdfParams: FabushiArgon2KdfParamsDTO

    public enum CodingKeys: String, CodingKey {
        case oldPassword = "old_password"
        case authSaltB64 = "auth_salt_b64"
        case authVerifierB64 = "auth_verifier_b64"
        case kdfParams = "kdf_params"
    }
}

public struct FabushiChangePasswordRequest: Encodable, Sendable, Equatable {
    public let oldSecret: String
    public let authVersion: Int32
    public let newAuthSaltB64: String
    public let newAuthVerifierB64: String
    public let newKdfParams: FabushiArgon2KdfParamsDTO

    public enum CodingKeys: String, CodingKey {
        case oldSecret = "old_secret"
        case authVersion = "auth_version"
        case newAuthSaltB64 = "new_auth_salt_b64"
        case newAuthVerifierB64 = "new_auth_verifier_b64"
        case newKdfParams = "new_kdf_params"
    }
}

public struct FabushiAuthCustodyUpdateResponse: Codable, Sendable, Equatable {
    public let ok: Bool
    public let authVersion: Int32

    public enum CodingKeys: String, CodingKey {
        case ok
        case authVersion = "auth_version"
    }
}

public struct FabushiAuthSessionResponse: Codable, Sendable, Equatable {
    public let userId: UInt64
    public let username: String
    public let sessionId: String
    public let token: String
    public let firstName: String?
    public let lastName: String?
    public let authVersion: Int32?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case sessionId = "session_id"
        case token
        case firstName = "first_name"
        case lastName = "last_name"
        case authVersion = "auth_version"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.userId = try container.decode(UInt64.self, forKey: .userId)
        self.username = try container.decodeIfPresent(String.self, forKey: .username) ?? ""
        self.sessionId = try container.decodeIfPresent(String.self, forKey: .sessionId) ?? ""
        self.token = try container.decodeIfPresent(String.self, forKey: .token) ?? ""
        self.firstName = try container.decodeIfPresent(String.self, forKey: .firstName)
        self.lastName = try container.decodeIfPresent(String.self, forKey: .lastName)
        self.authVersion = try container.decodeIfPresent(Int32.self, forKey: .authVersion)
    }

    public var identityTuple: (userId: UInt64, username: String)? {
        guard userId > 0 else { return nil }
        let resolved = username.trimmingCharacters(in: .whitespacesAndNewlines)
        return (userId, resolved.isEmpty ? "user_\(userId)" : resolved)
    }
}

// MARK: - Phone SMS & Email OTP DTOs (F02)

public struct FabushiPhoneSendCodeRequest: Encodable, Sendable, Equatable {
    public let phone: String
}

public struct FabushiPhoneSendCodeResponse: Codable, Sendable, Equatable {
    public let ok: Bool
    public let sent: Bool
    public let phoneCodeHash: String
    public let expiresAtMs: Int64
    public let resendAfterMs: Int64
    public let debugCode: String?

    public enum CodingKeys: String, CodingKey {
        case ok
        case sent
        case phoneCodeHash = "phone_code_hash"
        case expiresAtMs = "expires_at_ms"
        case resendAfterMs = "resend_after_ms"
        case debugCode = "debug_code"
    }
}

public struct FabushiPhoneVerifyRequest: Encodable, Sendable, Equatable {
    public let phone: String
    public let phoneCodeHash: String?
    public let code: String
    public let username: String?
    public let password: String?
    public let firstName: String?
    public let lastName: String?
    public let deviceName: String?

    public enum CodingKeys: String, CodingKey {
        case phone
        case phoneCodeHash = "phone_code_hash"
        case code
        case username
        case password
        case firstName = "first_name"
        case lastName = "last_name"
        case deviceName = "device_name"
    }

    public init(
        phone: String,
        phoneCodeHash: String?,
        code: String,
        username: String? = nil,
        password: String? = nil,
        firstName: String? = nil,
        lastName: String? = nil,
        deviceName: String? = "Fabushi iOS"
    ) {
        self.phone = phone
        self.phoneCodeHash = phoneCodeHash
        self.code = code
        self.username = username
        self.password = password
        self.firstName = firstName
        self.lastName = lastName
        self.deviceName = deviceName
    }
}

public struct FabushiEmailSendCodeRequest: Encodable, Sendable, Equatable {
    public let email: String
}

public struct FabushiEmailSendCodeResponse: Codable, Sendable, Equatable {
    public let sent: Bool
    public let cooldownSeconds: UInt32
    public let expiresInSeconds: UInt32
    public let debugCode: String?

    public enum CodingKeys: String, CodingKey {
        case sent
        case cooldownSeconds = "cooldown_seconds"
        case expiresInSeconds = "expires_in_seconds"
        case debugCode = "debug_code"
    }
}

public struct FabushiEmailVerifyRequest: Encodable, Sendable, Equatable {
    public let email: String
    public let code: String
}

// MARK: - Alipay QR & OAuth DTOs (F03)

public struct FabushiAlipayStartResponse: Codable, Sendable, Equatable {
    public let attemptId: String
    public let loginUrl: String
    public let expiresAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case attemptId = "attempt_id"
        case loginUrl = "login_url"
        case expiresAtMs = "expires_at_ms"
    }
}

public struct FabushiAlipayPollRequest: Encodable, Sendable, Equatable {
    public let attemptId: String

    public enum CodingKeys: String, CodingKey {
        case attemptId = "attempt_id"
    }
}

public struct FabushiAlipayUserDTO: Codable, Sendable, Equatable {
    public let userId: UInt64
    public let username: String?
    public let firstName: String?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case firstName = "first_name"
    }
}

public struct FabushiAlipayPollResponse: Codable, Sendable, Equatable {
    public let status: String
    public let token: String?
    public let user: FabushiAlipayUserDTO?
    public let session: FabushiAuthSessionResponse?
    public let userId: UInt64?
    public let username: String?

    public enum CodingKeys: String, CodingKey {
        case status
        case token
        case user
        case session
        case userId = "user_id"
        case username
    }

    public var isCompleted: Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if normalized == "complete" || normalized == "completed" || normalized == "confirmed" {
            return true
        }
        return session != nil
    }

    public var isExpired: Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "expired" || normalized == "cancelled" || normalized == "canceled"
    }

    public func resolvedIdentity() -> (userId: UInt64, username: String)? {
        if let session = session, let identity = session.identityTuple {
            return identity
        }
        if let uid = userId ?? user?.userId, uid > 0 {
            let name = username ?? user?.username ?? "user_\(uid)"
            let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
            return (uid, trimmed.isEmpty ? "user_\(uid)" : trimmed)
        }
        return nil
    }
}

// MARK: - Cross-Device QR Pairing & Passkey DTOs (F04)

public struct FabushiQrPairInitResponse: Codable, Sendable, Equatable {
    public let pairToken: String
    public let qrUrl: String
    public let expiresAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case pairToken = "pair_token"
        case qrUrl = "qr_url"
        case expiresAtMs = "expires_at_ms"
    }
}

public struct FabushiQrPairPollRequest: Encodable, Sendable, Equatable {
    public let pairToken: String

    public enum CodingKeys: String, CodingKey {
        case pairToken = "pair_token"
    }
}

public struct FabushiQrPairAcceptRequest: Encodable, Sendable, Equatable {
    public let pairToken: String

    public enum CodingKeys: String, CodingKey {
        case pairToken = "pair_token"
    }
}

public struct FabushiQrPairAcceptResponse: Codable, Sendable, Equatable {
    public let accepted: Bool
}

public struct FabushiQrPairVerifyRequest: Encodable, Sendable, Equatable {
    public let pairToken: String
    public let cloudPassword: String?

    public enum CodingKeys: String, CodingKey {
        case pairToken = "pair_token"
        case cloudPassword = "cloud_password"
    }
}

public struct FabushiQrPairUserDTO: Codable, Sendable, Equatable {
    public let userId: UInt64
    public let username: String?
    public let firstName: String?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case firstName = "first_name"
    }
}

public struct FabushiQrPairPollResponse: Codable, Sendable, Equatable {
    public let status: String
    public let pairToken: String?
    public let session: FabushiAuthSessionResponse?
    public let token: String?
    public let user: FabushiQrPairUserDTO?

    public enum CodingKeys: String, CodingKey {
        case status
        case pairToken = "pair_token"
        case session
        case token
        case user
    }

    public var requiresTwoFactor: Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "two_factor_required" || normalized == "requires_2fa" || normalized == "2fa_required"
    }

    public var isExpired: Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "expired" || normalized == "cancelled" || normalized == "denied"
    }

    public func resolvedIdentity() -> (userId: UInt64, username: String)? {
        if let session = session, let identity = session.identityTuple {
            return identity
        }
        if let user = user, user.userId > 0 {
            let trimmed = (user.username ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return (user.userId, trimmed.isEmpty ? "user_\(user.userId)" : trimmed)
        }
        return nil
    }
}

public struct FabushiPasskeyChallengeRequest: Encodable, Sendable, Equatable {
    public let username: String?
}

/// Prefixed with `Fabushi*` to avoid colliding with legacy `PasskeyChallenge`.
public struct FabushiPasskeyChallengeDTO: Codable, Sendable, Equatable {
    public let challenge: String
    public let rpId: String
    public let timeoutMs: UInt64?
    public let userId: UInt64?

    public enum CodingKeys: String, CodingKey {
        case challenge
        case rpId = "rp_id"
        case timeoutMs = "timeout_ms"
        case userId = "user_id"
    }
}

public struct FabushiPasskeyVerifyRequest: Encodable, Sendable, Equatable {
    public let challenge: String
    public let credentialId: String
    public let clientDataJson: String
    public let authenticatorData: String
    public let signature: String
    public let userId: UInt64?

    public enum CodingKeys: String, CodingKey {
        case challenge
        case credentialId = "credential_id"
        case clientDataJson = "client_data_json"
        case authenticatorData = "authenticator_data"
        case signature
        case userId = "user_id"
    }
}

public struct FabushiPasskeyRegisterRequest: Encodable, Sendable, Equatable {
    public let credentialId: String
    public let publicKey: String
    public let deviceName: String?

    public enum CodingKeys: String, CodingKey {
        case credentialId = "credential_id"
        case publicKey = "public_key"
        case deviceName = "device_name"
    }
}

public struct FabushiPasskeyDeleteRequest: Encodable, Sendable, Equatable {
    public let passkeyId: String

    public enum CodingKeys: String, CodingKey {
        case passkeyId = "passkey_id"
    }
}

public struct FabushiPasskeyRecordDTO: Identifiable, Sendable, Equatable {
    public let id: String
    public let credentialId: String
    public let deviceName: String
    public let createdAtMs: Int64?
    public let lastUsedAtMs: Int64?

    public init(
        id: String,
        credentialId: String,
        deviceName: String,
        createdAtMs: Int64?,
        lastUsedAtMs: Int64?
    ) {
        self.id = id
        self.credentialId = credentialId
        self.deviceName = deviceName
        self.createdAtMs = createdAtMs
        self.lastUsedAtMs = lastUsedAtMs
    }

    public static func fromJSONValue(_ value: FabushiJSONValue) -> FabushiPasskeyRecordDTO? {
        guard let obj = value.objectValue else { return nil }
        let credId = obj["credential_id"]?.stringValue
            ?? obj["passkey_id"]?.stringValue
            ?? obj["id"]?.stringValue
            ?? ""
        guard !credId.isEmpty else { return nil }
        let recordId = obj["id"]?.stringValue ?? obj["passkey_id"]?.stringValue ?? credId
        let deviceName = obj["device_name"]?.stringValue ?? "Passkey"
        return FabushiPasskeyRecordDTO(
            id: recordId,
            credentialId: credId,
            deviceName: deviceName,
            createdAtMs: obj["created_at_ms"]?.int64Value,
            lastUsedAtMs: obj["last_used_at_ms"]?.int64Value
        )
    }
}

// MARK: - Cloud 2FA & Active Session DTOs (F05)

public struct FabushiTwoFactorStatusResponse: Codable, Sendable, Equatable {
    public let hasPassword: Bool
    public let hint: String?
    public let recoveryEmailPattern: String?

    public enum CodingKeys: String, CodingKey {
        case hasPassword = "has_password"
        case hint
        case recoveryEmailPattern = "recovery_email_pattern"
    }
}

public struct FabushiTwoFactorSetRequest: Encodable, Sendable, Equatable {
    public let currentPassword: String?
    public let newPassword: String
    public let hint: String?
    public let recoveryEmail: String?

    public enum CodingKeys: String, CodingKey {
        case currentPassword = "current_password"
        case newPassword = "new_password"
        case hint
        case recoveryEmail = "recovery_email"
    }
}

public struct FabushiTwoFactorDisableRequest: Encodable, Sendable, Equatable {
    public let currentPassword: String

    public enum CodingKeys: String, CodingKey {
        case currentPassword = "current_password"
    }
}

public struct FabushiTwoFactorDisableResponse: Codable, Sendable, Equatable {
    public let disabled: Bool
}

public struct FabushiTwoFactorVerifyRequest: Encodable, Sendable, Equatable {
    public let challengeToken: String
    public let cloudPassword: String
    public let deviceName: String?

    public enum CodingKeys: String, CodingKey {
        case challengeToken = "challenge_token"
        case cloudPassword = "cloud_password"
        case deviceName = "device_name"
    }
}

public struct FabushiTwoFactorChallenge: Sendable, Equatable {
    public let challengeToken: String
    public let hint: String?
    public let isQrPair: Bool

    public init(challengeToken: String, hint: String?, isQrPair: Bool = false) {
        self.challengeToken = challengeToken
        self.hint = hint
        self.isQrPair = isQrPair
    }
}

/// Defensively extracts structured `two_factor_required` / `requires_2fa` challenge payloads
/// from Rust HTTP/RPC error messages without relying on unguarded string heuristics.
public enum FabushiTwoFactorChallengeParser: Sendable {
    private struct WireChallengePayload: Decodable {
        let code: String?
        let error: String?
        let requires2FA: Bool?
        let challengeToken: String?
        let hint: String?

        enum CodingKeys: String, CodingKey {
            case code
            case error
            case requires2FA = "requires_2fa"
            case challengeToken = "challenge_token"
            case hint
        }
    }

    public static func extract(from error: Error) -> FabushiTwoFactorChallenge? {
        let message: String
        if let hostError = error as? FabushiHostError {
            message = hostError.localizedDescription
        } else {
            message = error.localizedDescription
        }
        return extract(fromMessage: message)
    }

    public static func extract(fromMessage message: String) -> FabushiTwoFactorChallenge? {
        if let firstBrace = message.firstIndex(of: "{"),
           let lastBrace = message.lastIndex(of: "}"),
           firstBrace <= lastBrace {
            let jsonSlice = String(message[firstBrace...lastBrace])
            if let data = jsonSlice.data(using: .utf8),
               let payload = try? JSONDecoder().decode(WireChallengePayload.self, from: data) {
                let codeMatch = payload.code?.lowercased() == "two_factor_required"
                    || payload.error?.lowercased() == "two_factor_required"
                    || payload.requires2FA == true
                if codeMatch,
                   let token = payload.challengeToken?.trimmingCharacters(in: .whitespacesAndNewlines),
                   !token.isEmpty {
                    let cleanHint = payload.hint?.trimmingCharacters(in: .whitespacesAndNewlines)
                    return FabushiTwoFactorChallenge(
                        challengeToken: token,
                        hint: (cleanHint?.isEmpty == false) ? cleanHint : nil
                    )
                }
            }
        }
        return nil
    }
}

public struct FabushiActiveDeviceSessionDTO: Codable, Sendable, Equatable, Identifiable {
    public let sessionId: String
    public let deviceName: String
    public let platform: String?
    public let ipAddress: String?
    public let createdAtMs: Int64
    public let lastActiveMs: Int64
    public let isCurrent: Bool

    public var id: String { sessionId }

    public enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case deviceName = "device_name"
        case platform
        case ipAddress = "ip_address"
        case createdAtMs = "created_at_ms"
        case lastActiveMs = "last_active_ms"
        case isCurrent = "is_current"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.sessionId = try container.decode(String.self, forKey: .sessionId)
        self.deviceName = try container.decode(String.self, forKey: .deviceName)
        self.platform = try container.decodeIfPresent(String.self, forKey: .platform)
        self.ipAddress = try container.decodeIfPresent(String.self, forKey: .ipAddress)
        self.createdAtMs = try container.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
        self.lastActiveMs = try container.decodeIfPresent(Int64.self, forKey: .lastActiveMs) ?? self.createdAtMs
        self.isCurrent = try container.decodeIfPresent(Bool.self, forKey: .isCurrent) ?? false
    }
}

// MARK: - Shared Zero-Knowledge Vault Envelope DTOs (Flat <-> Nested Conversion)

/// Nested `UserVaultKeyEnvelope` used by local Rust `bot-browser` crypto RPCs
/// (`vault.initNew`, `vault.initSplitKey`, `vault.unlockWithPassphrase`, `vault.unlockWithSplitKey`, `vault.unlockWithRecoveryKey`).
public struct FabushiVaultNestedEnvelopeDTO: Codable, Sendable, Equatable {
    public let keyVersion: UInt32
    public let kdfAlg: String
    public let kdfSaltB64: String
    public let kdfParams: FabushiArgon2KdfParamsDTO
    public let wrappedRootKeyB64: String
    public let recoveryWrappedRootKeyB64: String?
    public let updatedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case keyVersion = "key_version"
        case kdfAlg = "kdf_alg"
        case kdfSaltB64 = "kdf_salt_b64"
        case kdfParams = "kdf_params"
        case wrappedRootKeyB64 = "wrapped_root_key_b64"
        case recoveryWrappedRootKeyB64 = "recovery_wrapped_root_key_b64"
        case updatedAtMs = "updated_at_ms"
    }
}

/// Flat `UserVaultKeyEnvelopeDto` used by server sync RPCs (`vault.envelopeGet`, `vault.envelopePut`).
public struct FabushiVaultFlatEnvelopeDTO: Codable, Sendable, Equatable {
    public let keyVersion: UInt32
    public let kdfAlg: String
    public let kdfSaltB64: String
    public let kdfMCostKib: UInt32
    public let kdfTCost: UInt32
    public let kdfPCost: UInt32
    public let wrappedRootKeyB64: String
    public let recoveryWrappedRootKeyB64: String?
    public let updatedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case keyVersion = "key_version"
        case kdfAlg = "kdf_alg"
        case kdfSaltB64 = "kdf_salt_b64"
        case kdfMCostKib = "kdf_m_cost_kib"
        case kdfTCost = "kdf_t_cost"
        case kdfPCost = "kdf_p_cost"
        case wrappedRootKeyB64 = "wrapped_root_key_b64"
        case recoveryWrappedRootKeyB64 = "recovery_wrapped_root_key_b64"
        case updatedAtMs = "updated_at_ms"
    }
}

public enum FabushiVaultEnvelopeConverter: Sendable {
    public static func toNested(_ flat: FabushiVaultFlatEnvelopeDTO) -> FabushiVaultNestedEnvelopeDTO {
        FabushiVaultNestedEnvelopeDTO(
            keyVersion: flat.keyVersion,
            kdfAlg: flat.kdfAlg,
            kdfSaltB64: flat.kdfSaltB64,
            kdfParams: FabushiArgon2KdfParamsDTO(
                mCostKib: flat.kdfMCostKib,
                tCost: flat.kdfTCost,
                pCost: flat.kdfPCost
            ),
            wrappedRootKeyB64: flat.wrappedRootKeyB64,
            recoveryWrappedRootKeyB64: flat.recoveryWrappedRootKeyB64,
            updatedAtMs: flat.updatedAtMs
        )
    }

    public static func toFlat(_ nested: FabushiVaultNestedEnvelopeDTO) -> FabushiVaultFlatEnvelopeDTO {
        FabushiVaultFlatEnvelopeDTO(
            keyVersion: nested.keyVersion,
            kdfAlg: nested.kdfAlg,
            kdfSaltB64: nested.kdfSaltB64,
            kdfMCostKib: nested.kdfParams.mCostKib,
            kdfTCost: nested.kdfParams.tCost,
            kdfPCost: nested.kdfParams.pCost,
            wrappedRootKeyB64: nested.wrappedRootKeyB64,
            recoveryWrappedRootKeyB64: nested.recoveryWrappedRootKeyB64,
            updatedAtMs: nested.updatedAtMs
        )
    }
}

public struct FabushiVaultEnvelopeResponseDTO: Codable, Sendable, Equatable {
    public let envelope: FabushiVaultFlatEnvelopeDTO?
}

public struct FabushiVaultInitSplitKeyRequest: Encodable, Sendable, Equatable {
    public let userId: UInt64
    public let password: String
    public let saltB64: String?
    public let kdfParams: FabushiArgon2KdfParamsDTO?
    public let fastKdf: Bool?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case password
        case saltB64 = "salt_b64"
        case kdfParams = "kdf_params"
        case fastKdf = "fast_kdf"
    }
}

public struct FabushiVaultInitSplitKeyResponse: Codable, Sendable, Equatable {
    public let unlocked: Bool
    public let authSaltB64: String
    public let authVerifierB64: String
    public let envelope: FabushiVaultNestedEnvelopeDTO
    public let emergencyKit: String
    public let rootKeyB64: String

    public enum CodingKeys: String, CodingKey {
        case unlocked
        case authSaltB64 = "auth_salt_b64"
        case authVerifierB64 = "auth_verifier_b64"
        case envelope
        case emergencyKit = "emergency_kit"
        case rootKeyB64 = "root_key_b64"
    }
}

public struct FabushiVaultUnlockWithSplitKeyRequest: Encodable, Sendable, Equatable {
    public let userId: UInt64
    public let envelope: FabushiVaultNestedEnvelopeDTO?
    public let vaultKekB64: String?
    public let password: String?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case envelope
        case vaultKekB64 = "vault_kek_b64"
        case password
    }
}

public struct FabushiVaultUnlockWithSplitKeyResponse: Codable, Sendable, Equatable {
    public let unlocked: Bool
    public let rootKeyB64: String

    public enum CodingKeys: String, CodingKey {
        case unlocked
        case rootKeyB64 = "root_key_b64"
    }
}

public struct FabushiVaultUnlockWithKeyBytesRequest: Encodable, Sendable, Equatable {
    public let rootKeyB64: String

    public enum CodingKeys: String, CodingKey {
        case rootKeyB64 = "root_key_b64"
    }
}

public struct FabushiVaultUnlockStatusResponse: Codable, Sendable, Equatable {
    public let unlocked: Bool
    public let hasEnvelope: Bool?

    public enum CodingKeys: String, CodingKey {
        case unlocked
        case hasEnvelope = "has_envelope"
    }
}

// MARK: - Pure CoreImage QR Code Renderer (Collision-Free `FabushiQRCodeRenderer`)

public enum FabushiQRCodeRenderer: Sendable {
    #if canImport(UIKit)
    public static func renderQRCodeImage(from payload: String, scale: CGFloat = 10) -> UIImage? {
        let trimmed = payload.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let data = trimmed.data(using: .utf8) else {
            return nil
        }
        let filter = CIFilter.qrCodeGenerator()
        filter.message = data
        filter.correctionLevel = "M"
        guard let outputImage = filter.outputImage else {
            return nil
        }
        let clampedScale = max(2, scale)
        let scaled = outputImage.transformed(by: CGAffineTransform(scaleX: clampedScale, y: clampedScale))
        let context = CIContext()
        guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else {
            return nil
        }
        return UIImage(cgImage: cgImage)
    }
    #endif
}

// MARK: - Auth Validation & Deep-Link Parsing Helpers

public struct FabushiCountryDialCode: Identifiable, Hashable, Sendable {
    public let id: String
    public let dialCode: String
    public let regionCode: String

    public init(dialCode: String, regionCode: String) {
        self.id = "\(regionCode)_\(dialCode)"
        self.dialCode = dialCode
        self.regionCode = regionCode
    }

    public static let commonCodes: [FabushiCountryDialCode] = [
        FabushiCountryDialCode(dialCode: "+86", regionCode: "CN"),
        FabushiCountryDialCode(dialCode: "+1", regionCode: "US"),
        FabushiCountryDialCode(dialCode: "+852", regionCode: "HK"),
        FabushiCountryDialCode(dialCode: "+886", regionCode: "TW"),
        FabushiCountryDialCode(dialCode: "+81", regionCode: "JP"),
        FabushiCountryDialCode(dialCode: "+44", regionCode: "GB"),
        FabushiCountryDialCode(dialCode: "+65", regionCode: "SG"),
        FabushiCountryDialCode(dialCode: "+49", regionCode: "DE"),
    ]
}

public enum FabushiAuthValidation: Sendable {
    public static func validateServerURL(_ raw: String) -> String? {
        FabushiLaunchConfiguration.validateAndNormalizeServerURL(raw)
    }

    public static func isValidEmail(_ raw: String) -> Bool {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              !trimmed.contains(" "),
              let atIndex = trimmed.firstIndex(of: "@"),
              atIndex > trimmed.startIndex
        else {
            return false
        }
        let domain = trimmed[trimmed.index(after: atIndex)...]
        guard domain.contains("."),
              !domain.hasPrefix("."),
              !domain.hasSuffix(".")
        else {
            return false
        }
        return true
    }

    /// Normalizes a phone number into E.164 format and verifies 7..20 digits.
    /// If `rawNumber` already starts with `+`, its explicit country prefix is preserved.
    public static func normalizeE164Phone(countryCode: String, rawNumber: String) -> String? {
        let trimmed = rawNumber.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        let combined: String
        if trimmed.hasPrefix("+") {
            combined = trimmed
        } else {
            let prefix = countryCode.trimmingCharacters(in: .whitespacesAndNewlines)
            let normalizedPrefix = prefix.hasPrefix("+") ? prefix : "+\(prefix)"
            combined = normalizedPrefix + trimmed
        }

        let digits = combined.filter(\.isNumber)
        guard digits.count >= 7, digits.count <= 20 else {
            return nil
        }
        return "+\(digits)"
    }

    /// Extracts pairing token from `tg://login?token=...`, `fabushi://pair?token=...`, or a raw token string.
    public static func extractQrPairToken(from raw: String) -> String? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if let components = URLComponents(string: trimmed),
           let tokenItem = components.queryItems?.first(where: { $0.name == "token" || $0.name == "pair_token" }),
           let tokenValue = tokenItem.value?.trimmingCharacters(in: .whitespacesAndNewlines),
           !tokenValue.isEmpty {
            return tokenValue
        }
        if !trimmed.contains(" "), trimmed.count >= 6 {
            return trimmed
        }
        return nil
    }

    public static func extractStateQueryParam(from urlString: String) -> String? {
        guard let components = URLComponents(string: urlString),
              let state = components.queryItems?.first(where: { $0.name == "state" })?.value?
                  .trimmingCharacters(in: .whitespacesAndNewlines),
              !state.isEmpty
        else {
            return nil
        }
        return state
    }

    public static func parseAlipayCallbackURL(_ url: URL) -> (state: String?, authCode: String?)? {
        guard url.scheme?.lowercased() == "fabushi" else { return nil }
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return nil
        }
        let host = (components.host ?? "").lowercased()
        let pathParts = url.pathComponents.filter { $0 != "/" && !$0.isEmpty }.map { $0.lowercased() }
        let isAuthCallback = host == "auth" || pathParts.first == "auth"
        guard isAuthCallback else { return nil }

        var state: String?
        var authCode: String?
        for item in components.queryItems ?? [] {
            let trimmed = item.value?.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let trimmed, !trimmed.isEmpty else { continue }
            if item.name == "state" {
                state = trimmed
            } else if item.name == "auth_code" || item.name == "code" {
                authCode = trimmed
            }
        }
        return (state, authCode)
    }
}
