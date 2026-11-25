import CryptoKit
import Foundation
import os

// MARK: - Per-Account Frame-0 Messaging Snapshot Cache (`D13-01` / `D13-02` / TRAP #4)

/// Thread-safe, per-account disk snapshot cache stored under Application Support with
/// iOS `.completeUntilFirstUserAuthentication` file protection.
///
/// Security invariants:
/// - Keyed by SHA-256 of `(normalizedServerURL, accountId)` with zero plaintext PII in filenames.
/// - Enforces `maxCachedMessagesPerChat == 100` (`D13-01`).
/// - NEVER writes account chat data into `settings.localSet` (which is device-scoped!).
/// - Wired to `coordinator.registerAccountSecretsPurger` and `coordinator.registerServerSwitchSecretsPurger`.
public final class FabushiMessagingCacheStore: Sendable {
    public static let maxCachedMessagesPerChat: Int = 100
    public static let accountPurgerId: String = "m3-messaging-cache-account"
    public static let serverSwitchPurgerId: String = "m3-messaging-cache-server"

    public let baseDirectory: URL
    private let lock = OSAllocatedUnfairLock(initialState: ())

    public init(baseDirectory: URL? = nil) {
        if let baseDirectory {
            self.baseDirectory = baseDirectory
        } else {
            let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
                ?? FileManager.default.temporaryDirectory
            self.baseDirectory = support.appendingPathComponent("FabushiMessagingCache", isDirectory: true)
        }
    }

    // MARK: - Deterministic SHA-256 Keying

    public static func normalizeServerURL(_ serverURL: String) -> String {
        var trimmed = serverURL.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        while trimmed.hasSuffix("/") {
            trimmed.removeLast()
        }
        return trimmed
    }

    public static func sha256Hex(_ input: String) -> String {
        let digest = SHA256.hash(data: Data(input.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    public static func cacheKey(serverURL: String, accountId: String) -> String {
        let normalizedServer = normalizeServerURL(serverURL)
        let normalizedAccount = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        return sha256Hex("\(normalizedServer)|\(normalizedAccount)")
    }

    public func snapshotFileURL(serverURL: String, accountId: String) -> URL {
        let key = Self.cacheKey(serverURL: serverURL, accountId: accountId)
        let acctHash = Self.sha256Hex(accountId.trimmingCharacters(in: .whitespacesAndNewlines))
        return baseDirectory.appendingPathComponent("\(key)__acct_\(acctHash).json", isDirectory: false)
    }

    // MARK: - Synchronous Frame-0 Hydration & Bounded Write-Through

    public func loadSnapshot(serverURL: String, accountId: String) -> FabushiSyncSnapshot? {
        let trimmedAccount = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedAccount.isEmpty else { return nil }
        let fileURL = snapshotFileURL(serverURL: serverURL, accountId: trimmedAccount)

        return lock.withLock { _ in
            guard FileManager.default.fileExists(atPath: fileURL.path),
                  let data = try? Data(contentsOf: fileURL)
            else {
                return nil
            }
            guard var snapshot = try? JSONDecoder().decode(FabushiSyncSnapshot.self, from: data),
                  snapshot.version == FabushiSyncSnapshot.schemaVersion
            else {
                return nil
            }
            snapshot.messagesByChat = Self.enforceHistoryLimit(snapshot.messagesByChat)
            return snapshot
        }
    }

    @discardableResult
    public func saveSnapshot(
        _ snapshot: FabushiSyncSnapshot,
        serverURL: String,
        accountId: String
    ) -> Bool {
        let trimmedAccount = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedAccount.isEmpty else { return false }

        var bounded = snapshot
        bounded.version = FabushiSyncSnapshot.schemaVersion
        bounded.savedAtMs = FabushiWireDateParser.nowMs()
        bounded.messagesByChat = Self.enforceHistoryLimit(snapshot.messagesByChat)

        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let encoded = try? encoder.encode(bounded) else { return false }

        let fileURL = snapshotFileURL(serverURL: serverURL, accountId: trimmedAccount)
        return lock.withLock { _ in
            do {
                try ensureProtectedDirectoryLocked()
                var options: Data.WritingOptions = [.atomic]
                #if os(iOS)
                options.insert(.completeUntilFirstUserAuthentication)
                #endif
                try encoded.write(to: fileURL, options: options)
                #if os(iOS)
                try? FileManager.default.setAttributes(
                    [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                    ofItemAtPath: fileURL.path
                )
                #endif
                return true
            } catch {
                return false
            }
        }
    }

    public func purgeSnapshot(serverURL: String, accountId: String) {
        let trimmedAccount = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedAccount.isEmpty else { return }
        let fileURL = snapshotFileURL(serverURL: serverURL, accountId: trimmedAccount)
        lock.withLock { _ in
            try? FileManager.default.removeItem(at: fileURL)
        }
    }

    /// Removes ALL snapshot files associated with `accountId` across every server URL.
    public func purgeAccount(accountId: String) {
        let trimmedAccount = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedAccount.isEmpty else { return }
        let suffix = "__acct_\(Self.sha256Hex(trimmedAccount)).json"

        lock.withLock { _ in
            guard let files = try? FileManager.default.contentsOfDirectory(
                at: baseDirectory,
                includingPropertiesForKeys: nil
            ) else {
                return
            }
            for url in files where url.lastPathComponent.hasSuffix(suffix) {
                try? FileManager.default.removeItem(at: url)
            }
        }
    }

    /// Removes ALL snapshot files under `baseDirectory` (used on server URL switch).
    public func purgeAll() {
        lock.withLock { _ in
            guard let files = try? FileManager.default.contentsOfDirectory(
                at: baseDirectory,
                includingPropertiesForKeys: nil
            ) else {
                return
            }
            for url in files where url.pathExtension == "json" {
                try? FileManager.default.removeItem(at: url)
            }
        }
    }

    public func snapshotFileCount() -> Int {
        lock.withLock { _ in
            guard let files = try? FileManager.default.contentsOfDirectory(
                at: baseDirectory,
                includingPropertiesForKeys: nil
            ) else {
                return 0
            }
            return files.filter { $0.pathExtension == "json" }.count
        }
    }

    // MARK: - Helpers

    public static func enforceHistoryLimit(
        _ messagesByChat: [Int64: [FabushiMessageRecord]],
        limit: Int = maxCachedMessagesPerChat
    ) -> [Int64: [FabushiMessageRecord]] {
        let safeLimit = max(1, limit)
        var bounded: [Int64: [FabushiMessageRecord]] = [:]
        bounded.reserveCapacity(messagesByChat.count)
        for (chatId, messages) in messagesByChat {
            let sorted = messages.sorted { lhs, rhs in
                if lhs.dateMs != rhs.dateMs {
                    return lhs.dateMs < rhs.dateMs
                }
                return lhs.messageId < rhs.messageId
            }
            if sorted.count > safeLimit {
                bounded[chatId] = Array(sorted.suffix(safeLimit))
            } else {
                bounded[chatId] = sorted
            }
        }
        return bounded
    }

    private func ensureProtectedDirectoryLocked() throws {
        let fm = FileManager.default
        if !fm.fileExists(atPath: baseDirectory.path) {
            try fm.createDirectory(at: baseDirectory, withIntermediateDirectories: true)
        }
        #if os(iOS)
        try? fm.setAttributes(
            [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
            ofItemAtPath: baseDirectory.path
        )
        #endif
    }
}
