import Foundation

/// Fail-closed errors thrown when launch environment configuration requests an unsafe state-reset target.
public enum FabushiLaunchConfigurationError: LocalizedError, Sendable, Equatable {
    case unsafeResetStateDirectory(String)

    public var errorDescription: String? {
        switch self {
        case .unsafeResetStateDirectory(let path):
            return "Refusing to reset unsafe or production state directory: \(path)"
        }
    }
}

/// Deterministic parser for Fabushi iOS launch environment variables and CLI arguments
/// (`TEST_INFRA.md §4.2`). NEVER enables mocks, fake tokens, or synthetic sample chats.
public struct FabushiLaunchConfiguration: Sendable, Equatable {
    public let isUITestMode: Bool
    public let disableLegacyFeatureHostTest: Bool
    public let shouldUseLegacyProductionRenderer: Bool
    public let serverURL: String?
    public let invalidLaunchServerURL: String?
    public let appDataDirectory: URL?
    public let resetState: Bool
    public let themePreset: FabushiThemePreset?
    public let accentHex: String?
    public let invalidLaunchAccentHex: String?
    public let reduceMotion: Bool
    public let highContrast: Bool
    public let reduceMotionLaunchOverride: Bool?
    public let highContrastLaunchOverride: Bool?
    public let requirePasscode: Bool
    public let simulateCallPermissionDenied: Bool
    public let simulateReplayKitDenied: Bool
    public let preferredLocale: FabushiLocale?
    public let formFactorHint: String?

    public init(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        arguments: [String] = ProcessInfo.processInfo.arguments
    ) {
        let uiTestMode = environment["FABUSHI_UI_TEST_MODE"] == "1"
        let disableLegacyHost = environment["FABUSHI_FEATURE_HOST_TEST"] == "0"
        let hasLegacySmoke = environment["FABUSHI_FEATURE_HOST_SMOKE"] == "1"
        let hasLegacyFeatureTest = environment["FABUSHI_FEATURE_HOST_TEST"] == "1"
        let hasLegacyCISession = !(environment["FABUSHI_CI_ACCOUNT_SESSION_BASE64"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .isEmpty ?? true)

        self.isUITestMode = uiTestMode
        self.disableLegacyFeatureHostTest = disableLegacyHost
        self.shouldUseLegacyProductionRenderer = !uiTestMode && (
            hasLegacySmoke || hasLegacyFeatureTest || hasLegacyCISession
        )

        // 4-tier server URL precedence:
        // Tier 1: CLI `--server-url <url>` or `--server-url=<url>`
        // Tier 2: `GPUI_TELEGRAM_SERVER_URL` > `FABUSHI_E2E_SERVER_URL`
        // Tiers 3 & 4 (persisted host config & default `http://34.46.77.133:8080`) remain in host when `nil`.
        let rawServerURL = Self.extractCLIServerURL(from: arguments)
            ?? Self.nonEmptyEnv("GPUI_TELEGRAM_SERVER_URL", in: environment)
            ?? Self.nonEmptyEnv("FABUSHI_E2E_SERVER_URL", in: environment)

        if let rawServerURL {
            if let normalized = Self.validateAndNormalizeServerURL(rawServerURL) {
                self.serverURL = normalized
                self.invalidLaunchServerURL = nil
            } else {
                self.serverURL = nil
                self.invalidLaunchServerURL = rawServerURL
            }
        } else {
            self.serverURL = nil
            self.invalidLaunchServerURL = nil
        }

        if let rawDataDir = Self.nonEmptyEnv("FABUSHI_APP_DATA", in: environment) {
            self.appDataDirectory = URL(fileURLWithPath: rawDataDir, isDirectory: true)
        } else {
            self.appDataDirectory = nil
        }

        self.resetState = environment["FABUSHI_E2E_RESET_STATE"] == "1"

        if let rawTheme = Self.nonEmptyEnv("FABUSHI_UI_THEME", in: environment) {
            self.themePreset = FabushiThemePreset.fromName(rawTheme)
        } else {
            self.themePreset = nil
        }

        if let rawAccent = Self.nonEmptyEnv("FABUSHI_UI_ACCENT_HEX", in: environment) {
            if let rgb = FabushiColorHex.parse(rawAccent, allowShortRGB: false) {
                self.accentHex = FabushiColorHex.format(rgb)
                self.invalidLaunchAccentHex = nil
            } else {
                self.accentHex = nil
                self.invalidLaunchAccentHex = rawAccent
            }
        } else {
            self.accentHex = nil
            self.invalidLaunchAccentHex = nil
        }

        // Explicit `"1"`/`"0"` launch overrides are authoritative; under `FABUSHI_UI_TEST_MODE=1`,
        // omitted flags also default authoritatively to `false` so consecutive E2E cold restarts
        // (e.g. `T4_08`) never inherit stale persisted toggle states from an earlier launch.
        let explicitReduceMotion = Self.parseOptionalBoolEnv("FABUSHI_UI_REDUCE_MOTION", in: environment)
        let explicitHighContrast = Self.parseOptionalBoolEnv("FABUSHI_UI_HIGH_CONTRAST", in: environment)
        self.reduceMotionLaunchOverride = explicitReduceMotion ?? (uiTestMode ? false : nil)
        self.highContrastLaunchOverride = explicitHighContrast ?? (uiTestMode ? false : nil)
        self.reduceMotion = self.reduceMotionLaunchOverride ?? false
        self.highContrast = self.highContrastLaunchOverride ?? false

        self.requirePasscode = environment["FABUSHI_E2E_REQUIRE_PASSCODE"] == "1"
        self.simulateCallPermissionDenied = environment["FABUSHI_E2E_SIMULATE_CALL_PERMISSION_DENIED"] == "1"
        self.simulateReplayKitDenied = environment["FABUSHI_E2E_SIMULATE_REPLAYKIT_DENIED"] == "1"
        self.preferredLocale = Self.extractPreferredLocale(from: arguments)
        self.formFactorHint = Self.nonEmptyEnv("FABUSHI_E2E_FORM_FACTOR", in: environment)?.lowercased()
    }

    public static var current: FabushiLaunchConfiguration {
        FabushiLaunchConfiguration()
    }

    /// Resolves the target data directory, matching `FabushiCoordinator.start`'s default
    /// `Application Support/FabushiCore` path when `FABUSHI_APP_DATA` is unset.
    public func resolvedAppDataDirectory(fileManager: FileManager = .default) -> URL? {
        if let appDataDirectory {
            return appDataDirectory
        }
        return fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("FabushiCore", isDirectory: true)
    }

    /// Cleans an explicit isolated ephemeral state directory ONLY when `resetState == true`
    /// (`FABUSHI_E2E_RESET_STATE=1`) AND an explicit `FABUSHI_APP_DATA` path was provided.
    ///
    /// Fail-safe invariants:
    /// - When `resetState == false`, preserves all persisted files untouched (cold-restart durability).
    /// - When `appDataDirectory == nil`, performs a safe no-op and NEVER falls back to production
    ///   `Application Support/FabushiCore`.
    /// - Rejects root (`/`), `$HOME`, system roots, and production Application Support paths.
    public func prepareStateDirectoryIfNeeded(fileManager: FileManager = .default) throws {
        guard resetState, let targetDirectory = appDataDirectory else { return }

        if Self.isUnsafeResetDirectory(targetDirectory, fileManager: fileManager) {
            throw FabushiLaunchConfigurationError.unsafeResetStateDirectory(targetDirectory.path)
        }

        if fileManager.fileExists(atPath: targetDirectory.path) {
            let contents = try fileManager.contentsOfDirectory(
                at: targetDirectory,
                includingPropertiesForKeys: nil,
                options: []
            )
            for itemURL in contents {
                try fileManager.removeItem(at: itemURL)
            }
        }
        try fileManager.createDirectory(at: targetDirectory, withIntermediateDirectories: true)
    }

    /// Validates an HTTP(S) server URL strictly and trims trailing slashes.
    public static func validateAndNormalizeServerURL(_ raw: String) -> String? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              (scheme == "http" || scheme == "https"),
              let host = components.host,
              !host.isEmpty
        else {
            return nil
        }

        var normalized = trimmed
        while normalized.hasSuffix("/") && normalized.count > scheme.count + 3 {
            normalized.removeLast()
        }
        return normalized
    }

    private static func nonEmptyEnv(_ key: String, in environment: [String: String]) -> String? {
        guard let value = environment[key]?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty
        else {
            return nil
        }
        return value
    }

    private static func parseOptionalBoolEnv(_ key: String, in environment: [String: String]) -> Bool? {
        guard let raw = nonEmptyEnv(key, in: environment)?.lowercased() else {
            return nil
        }
        switch raw {
        case "1", "true", "yes":
            return true
        case "0", "false", "no":
            return false
        default:
            return nil
        }
    }

    private static func normalizedPath(_ rawPath: String) -> String {
        let standardized = URL(fileURLWithPath: rawPath).standardizedFileURL.path
        if standardized.count > 1, standardized.hasSuffix("/") {
            return String(standardized.dropLast())
        }
        return standardized
    }

    private static func candidateNormalizedPaths(for url: URL) -> Set<String> {
        var paths: Set<String> = [
            normalizedPath(url.standardizedFileURL.path),
            normalizedPath(url.resolvingSymlinksInPath().standardizedFileURL.path),
        ]
        if url.path == "/" {
            paths.insert("/")
        }
        return paths
    }

    private static func isUnsafeResetDirectory(_ targetDirectory: URL, fileManager: FileManager) -> Bool {
        let candidates = candidateNormalizedPaths(for: targetDirectory)
        guard !candidates.isEmpty else { return true }

        var forbiddenExactPaths: Set<String> = [
            "",
            "/",
            "/System",
            "/Applications",
            "/Library",
            "/Users",
            "/var",
            "/private",
            "/private/var",
            "/etc",
            "/usr",
            "/tmp",
            "/private/tmp",
        ]

        let homePath = NSHomeDirectory()
        if !homePath.isEmpty {
            forbiddenExactPaths.formUnion(candidateNormalizedPaths(for: URL(fileURLWithPath: homePath, isDirectory: true)))
        }
        if let envHome = ProcessInfo.processInfo.environment["HOME"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !envHome.isEmpty {
            forbiddenExactPaths.formUnion(candidateNormalizedPaths(for: URL(fileURLWithPath: envHome, isDirectory: true)))
        }

        // Exact temporary directory root is unsafe (subdirectories under temp are allowed).
        forbiddenExactPaths.formUnion(candidateNormalizedPaths(for: fileManager.temporaryDirectory))

        let protectedSearchPaths: [FileManager.SearchPathDirectory] = [
            .applicationSupportDirectory,
            .documentDirectory,
            .libraryDirectory,
            .cachesDirectory,
        ]
        for searchPath in protectedSearchPaths {
            for url in fileManager.urls(for: searchPath, in: .userDomainMask) {
                forbiddenExactPaths.formUnion(candidateNormalizedPaths(for: url))
            }
        }

        if !candidates.isDisjoint(with: forbiddenExactPaths) {
            return true
        }

        // Reject production `Application Support/FabushiCore` and any descendant inside it.
        for appSupportURL in fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask) {
            let productionCoreURL = appSupportURL.appendingPathComponent("FabushiCore", isDirectory: true)
            let productionPaths = candidateNormalizedPaths(for: productionCoreURL)
            for prodPath in productionPaths {
                for candidate in candidates {
                    if candidate == prodPath || candidate.hasPrefix(prodPath + "/") {
                        return true
                    }
                }
            }
        }

        return false
    }

    private static func extractCLIServerURL(from arguments: [String]) -> String? {
        var idx = 0
        while idx < arguments.count {
            let arg = arguments[idx]
            if arg == "--server-url", idx + 1 < arguments.count {
                let candidate = arguments[idx + 1].trimmingCharacters(in: .whitespacesAndNewlines)
                if !candidate.isEmpty {
                    return candidate
                }
            } else if arg.hasPrefix("--server-url=") {
                let candidate = String(arg.dropFirst("--server-url=".count))
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                if !candidate.isEmpty {
                    return candidate
                }
            }
            idx += 1
        }
        return nil
    }

    private static func extractPreferredLocale(from arguments: [String]) -> FabushiLocale? {
        var idx = 0
        while idx < arguments.count {
            let arg = arguments[idx]
            if arg == "-AppleLanguages", idx + 1 < arguments.count {
                let rawList = arguments[idx + 1]
                    .trimmingCharacters(in: CharacterSet(charactersIn: "()[]\"' \t\r\n"))
                let firstToken = rawList
                    .split(separator: ",")
                    .first
                    .map { String($0).trimmingCharacters(in: CharacterSet(charactersIn: "\"' \t\r\n")) }
                if let firstToken, !firstToken.isEmpty {
                    return FabushiLocale(normalizing: firstToken)
                }
            } else if arg == "-AppleLocale", idx + 1 < arguments.count {
                let token = arguments[idx + 1].trimmingCharacters(in: .whitespacesAndNewlines)
                if !token.isEmpty {
                    return FabushiLocale(normalizing: token)
                }
            }
            idx += 1
        }
        return nil
    }
}
