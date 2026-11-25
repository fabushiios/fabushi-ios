import Foundation
import UniformTypeIdentifiers
#if canImport(UIKit)
import UIKit
#endif

/// Local-only, auto-expiring clipboard helper for secrets (Vault items, TOTP codes,
/// one-time Emergency Kits, and X25519 safety codes). Prevents Universal Clipboard (Handoff)
/// leakage and indefinite pasteboard retention.
public enum FabushiSecureClipboard: Sendable {
    public static let defaultExpirationSeconds: TimeInterval = 60

    public struct SensitiveCopyPayload: Sendable, Equatable {
        public let typeIdentifier: String
        public let value: String
        public let localOnly: Bool
        public let expirationDate: Date

        public init(
            typeIdentifier: String,
            value: String,
            localOnly: Bool,
            expirationDate: Date
        ) {
            self.typeIdentifier = typeIdentifier
            self.value = value
            self.localOnly = localOnly
            self.expirationDate = expirationDate
        }
    }

    public static func makeSensitivePayload(
        _ value: String,
        expirationSeconds: TimeInterval = defaultExpirationSeconds,
        now: Date = Date()
    ) -> SensitiveCopyPayload {
        let clampedSeconds = max(1, expirationSeconds)
        return SensitiveCopyPayload(
            typeIdentifier: UTType.plainText.identifier,
            value: value,
            localOnly: true,
            expirationDate: now.addingTimeInterval(clampedSeconds)
        )
    }

    #if canImport(UIKit)
    @MainActor
    @discardableResult
    public static func copySensitive(
        _ value: String,
        expirationSeconds: TimeInterval = defaultExpirationSeconds,
        pasteboard: UIPasteboard = .general,
        now: Date = Date()
    ) -> SensitiveCopyPayload {
        let payload = makeSensitivePayload(
            value,
            expirationSeconds: expirationSeconds,
            now: now
        )
        pasteboard.setItems(
            [[payload.typeIdentifier: payload.value]],
            options: [
                .localOnly: payload.localOnly,
                .expirationDate: payload.expirationDate,
            ]
        )
        return payload
    }
    #else
    @MainActor
    @discardableResult
    public static func copySensitive(
        _ value: String,
        expirationSeconds: TimeInterval = defaultExpirationSeconds,
        now: Date = Date()
    ) -> SensitiveCopyPayload {
        makeSensitivePayload(
            value,
            expirationSeconds: expirationSeconds,
            now: now
        )
    }
    #endif
}
