import Foundation

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

        self.reduceMotion = environment["FABUSHI_UI_REDUCE_MOTION"] == "1"
        self.highContrast = environment["FABUSHI_UI_HIGH_CONTRAST"] == "1"
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

    /// Cleans ephemeral local state ONLY when `resetState == true` (`FABUSHI_E2E_RESET_STATE=1`).
    /// Preserves all persisted files untouched on normal launches and cold restarts (`resetState == false`).
    public func prepareStateDirectoryIfNeeded(fileManager: FileManager = .default) throws {
        guard resetState else { return }
        guard let targetDirectory = resolvedAppDataDirectory(fileManager: fileManager) else { return }

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
