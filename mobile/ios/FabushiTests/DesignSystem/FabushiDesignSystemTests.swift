import Foundation
import SwiftUI
import XCTest
@testable import Fabushi

@MainActor
final class FabushiDesignSystemTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_ds_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    func testAllFourV169ThemePalettesMatchUpstreamHexMatrix() {
        let night = FabushiThemePalette.resolve(preset: .night)
        XCTAssertEqual(night.surfacePrimaryRGB, 0x161616)
        XCTAssertEqual(night.surfaceSecondaryRGB, 0x0E0E0E)
        XCTAssertEqual(night.surfaceTopBarRGB, 0x121212)
        XCTAssertEqual(night.surfaceElevatedRGB, 0x1F1F1F)
        XCTAssertEqual(night.bubbleIncomingRGB, 0x1F1F1F)
        XCTAssertEqual(night.bubbleOutgoingTopRGB, 0x1884FC)
        XCTAssertEqual(night.bubbleOutgoingBottomRGB, 0x116DD2)
        XCTAssertEqual(night.bubbleOutgoingTextRGB, 0xFFFFFF)
        XCTAssertEqual(night.accentContrast.effectiveHex, "#0A84FF")
        XCTAssertEqual(night.textPrimaryRGB, 0xF5F5F5)
        XCTAssertEqual(night.destructiveRGB, 0xFF6463)
        XCTAssertEqual(night.successRGB, 0x00BA69)

        let tinted = FabushiThemePalette.resolve(preset: .tinted)
        XCTAssertEqual(tinted.surfacePrimaryRGB, 0x181C22)
        XCTAssertEqual(tinted.surfaceSecondaryRGB, 0x181C22)
        XCTAssertEqual(tinted.surfaceElevatedRGB, 0x222831)
        XCTAssertEqual(tinted.bubbleIncomingRGB, 0x242A33)
        XCTAssertEqual(tinted.bubbleOutgoingTopRGB, 0x1E8FFF)
        XCTAssertEqual(tinted.bubbleOutgoingBottomRGB, 0x0A84FF)
        XCTAssertEqual(tinted.accentContrast.effectiveHex, "#0A84FF")
        XCTAssertEqual(tinted.destructiveRGB, 0xFF453A)
        XCTAssertEqual(tinted.successRGB, 0x30D158)

        let day = FabushiThemePalette.resolve(preset: .day)
        XCTAssertEqual(day.surfacePrimaryRGB, 0xFFFFFF)
        XCTAssertEqual(day.surfaceSecondaryRGB, 0xF5F5F5)
        XCTAssertEqual(day.surfaceTopBarRGB, 0xFCFCFC)
        XCTAssertEqual(day.bubbleIncomingRGB, 0xF1F1F1)
        XCTAssertEqual(day.bubbleOutgoingTopRGB, 0x1E86FC)
        XCTAssertEqual(day.bubbleOutgoingBottomRGB, 0x126FD4)
        XCTAssertEqual(day.accentContrast.effectiveHex, "#007AFF")
        XCTAssertEqual(day.textPrimaryRGB, 0x1C1C1C)
        XCTAssertEqual(day.destructiveRGB, 0xE72E3B)

        let classic = FabushiThemePalette.resolve(preset: .classic)
        XCTAssertEqual(classic.surfacePrimaryRGB, 0xD5E8C8)
        XCTAssertEqual(classic.surfaceSecondaryRGB, 0xFFFFFF)
        XCTAssertEqual(classic.bubbleIncomingRGB, 0xFFFFFF)
        XCTAssertEqual(classic.bubbleOutgoingTopRGB, 0xEFFDDE)
        XCTAssertEqual(classic.bubbleOutgoingBottomRGB, 0xEFFDDE)
        XCTAssertEqual(classic.bubbleOutgoingTextRGB, 0x000000)
        XCTAssertEqual(classic.accentContrast.rawHex, "#40A7E3")
        XCTAssertEqual(classic.accentContrast.effectiveHex, "#3EA2DC")
        XCTAssertTrue(classic.accentContrast.wasDarkened)
        XCTAssertFalse(classic.preset.isAppleStyle)

        // Upstream cycle order: Night -> Tinted -> Day -> Classic -> Night
        XCTAssertEqual(FabushiThemePreset.night.nextInCycle, .tinted)
        XCTAssertEqual(FabushiThemePreset.tinted.nextInCycle, .day)
        XCTAssertEqual(FabushiThemePreset.day.nextInCycle, .classic)
        XCTAssertEqual(FabushiThemePreset.classic.nextInCycle, .night)

        // Upstream case-insensitive alias parsing
        XCTAssertEqual(FabushiThemePreset.fromName("night-default"), .night)
        XCTAssertEqual(FabushiThemePreset.fromName("NIGHT_TINTED"), .tinted)
        XCTAssertEqual(FabushiThemePreset.fromName("day-blue"), .day)
        XCTAssertEqual(FabushiThemePreset.fromName("Day Classic"), .classic)
        XCTAssertEqual(FabushiThemePreset.fromName("unknown"), .night)
    }

    func testWCAG138LuminanceGuardGoldenVectorsAndHighContrastAA() {
        let blue = FabushiColorHex.applyWCAG138Guard(rgb: 0x007AFF)
        XCTAssertEqual(blue.effectiveHex, "#007AFF")
        XCTAssertFalse(blue.wasDarkened)
        XCTAssertEqual(blue.foregroundHex, "#FFFFFF")

        let darkBlue = FabushiColorHex.applyWCAG138Guard(rgb: 0x0A84FF)
        XCTAssertEqual(darkBlue.effectiveHex, "#0A84FF")
        XCTAssertFalse(darkBlue.wasDarkened)
        XCTAssertEqual(darkBlue.foregroundHex, "#FFFFFF")

        let yellow = FabushiColorHex.applyWCAG138Guard(rgb: 0xFFFF00)
        XCTAssertEqual(yellow.effectiveHex, "#A9A906")
        XCTAssertTrue(yellow.wasDarkened)
        XCTAssertEqual(yellow.darkenFactor, 0.36, accuracy: 0.0001)
        XCTAssertGreaterThanOrEqual(
            FabushiColorHex.contrastRatio(foreground: yellow.foregroundRGB, background: yellow.effectiveRGB),
            4.5
        )

        let white = FabushiColorHex.applyWCAG138Guard(rgb: 0xFFFFFF)
        XCTAssertEqual(white.effectiveHex, "#A9A9A9")
        XCTAssertTrue(white.wasDarkened)
        XCTAssertEqual(white.darkenFactor, 0.36, accuracy: 0.0001)
        XCTAssertGreaterThanOrEqual(
            FabushiColorHex.contrastRatio(foreground: white.foregroundRGB, background: white.effectiveRGB),
            4.5
        )

        // Exact boundary verification: luma == 138.0 untouched, luma == 139.0 darkened
        let atThreshold = FabushiColorHex.applyWCAG138Guard(rgb: 0x8A8A8A) // 138.0 exactly
        XCTAssertFalse(atThreshold.wasDarkened)
        XCTAssertEqual(atThreshold.effectiveRGB, 0x8A8A8A)

        let aboveThreshold = FabushiColorHex.applyWCAG138Guard(rgb: 0x8B8B8B) // 139.0
        XCTAssertTrue(aboveThreshold.wasDarkened)
        XCTAssertNotEqual(aboveThreshold.effectiveRGB, 0x8B8B8B)

        // High Contrast mode guarantees >= 4.5:1 WCAG AA contrast across all swatches & edge colors
        let testColors: [UInt32] = [0x000000, 0x111111, 0x007AFF, 0x0A84FF, 0x34C759, 0xE5A030, 0xFFFF00, 0xFFFFFF]
            + FabushiAccentSwatch.curated.map(\.rgb)
        for rgb in testColors {
            let hc = FabushiColorHex.applyWCAG138Guard(rgb: rgb, highContrast: true)
            let ratio = FabushiColorHex.contrastRatio(foreground: hc.foregroundRGB, background: hc.effectiveRGB)
            XCTAssertGreaterThanOrEqual(
                ratio,
                4.5,
                "High Contrast mode must guarantee >= 4.5:1 for \(FabushiColorHex.format(rgb)), got \(ratio)"
            )
        }
    }

    func testStrictHexParserFailClosedAndOptInShortRGB() {
        XCTAssertEqual(FabushiColorHex.parse("#007AFF"), 0x007AFF)
        XCTAssertEqual(FabushiColorHex.parse("007aff"), 0x007AFF)
        XCTAssertEqual(FabushiColorHex.parse("  #34C759  "), 0x34C759)

        // Strict fail-closed rejections (matching E2E T2_F31_04)
        XCTAssertNil(FabushiColorHex.parse("#123"))
        XCTAssertNil(FabushiColorHex.parse("#GGGGGG"))
        XCTAssertNil(FabushiColorHex.parse("not-a-color"))
        XCTAssertNil(FabushiColorHex.parse(""))
        XCTAssertNil(FabushiColorHex.parse("##007AFF"))
        XCTAssertNil(FabushiColorHex.parse("#1234567"))

        // Opt-in shorthand expansion
        XCTAssertEqual(FabushiColorHex.parse("#F0A", allowShortRGB: true), 0xFF00AA)
        XCTAssertEqual(FabushiColorHex.parse("abc", allowShortRGB: true), 0xAABBCC)

        // FabushiTheme fail-closed mutation check
        let theme = FabushiTheme(preset: .night)
        XCTAssertTrue(theme.applyCustomAccentHex("#34C759"))
        XCTAssertEqual(theme.customAccentHex, "#34C759")
        XCTAssertNil(theme.lastAccentValidationError)

        XCTAssertFalse(theme.applyCustomAccentHex("#123"))
        XCTAssertEqual(theme.customAccentHex, "#34C759", "Invalid hex must not corrupt active accent")
        XCTAssertNotNil(theme.lastAccentValidationError)

        XCTAssertFalse(theme.applyCustomAccentHex("not-a-color"))
        XCTAssertEqual(theme.customAccentHex, "#34C759")
    }

    func testCustomAccentPropagationAppleVsClassic() {
        let appleCustom = FabushiThemePalette.resolve(preset: .day, customAccentRGB: 0xFFFF00)
        XCTAssertEqual(appleCustom.accentContrast.effectiveHex, "#A9A906")
        XCTAssertEqual(appleCustom.bubbleOutgoingBottomRGB, 0xA9A906)
        XCTAssertEqual(appleCustom.sidebarActiveSelectionRGB, 0xA9A906)

        let classicCustom = FabushiThemePalette.resolve(preset: .classic, customAccentRGB: 0xE86C52)
        XCTAssertEqual(classicCustom.sidebarActiveSelectionRGB, 0xE86C52)
        XCTAssertEqual(classicCustom.bubbleOutgoingBaseRGB, 0xEFFDDE)
        XCTAssertEqual(classicCustom.textLinkRGB, 0x168ACD)
    }

    func testLaunchConfigurationParsingPrecedenceAndResetStateDirectory() throws {
        let dir = try makeTempDir("launch_reset")
        defer { try? FileManager.default.removeItem(at: dir) }

        let markerFile = dir.appendingPathComponent("persisted_marker.txt")
        try Data("keep-me".utf8).write(to: markerFile)

        // 1. CLI `--server-url` beats `GPUI_TELEGRAM_SERVER_URL` and `FABUSHI_E2E_SERVER_URL`
        let cliConfig = FabushiLaunchConfiguration(
            environment: [
                "FABUSHI_UI_TEST_MODE": "1",
                "FABUSHI_FEATURE_HOST_TEST": "0",
                "GPUI_TELEGRAM_SERVER_URL": "http://127.0.0.1:19001/",
                "FABUSHI_E2E_SERVER_URL": "http://127.0.0.1:19002/",
                "FABUSHI_APP_DATA": dir.path,
                "FABUSHI_E2E_RESET_STATE": "0",
                "FABUSHI_UI_THEME": "tinted",
                "FABUSHI_UI_ACCENT_HEX": "#ffff00",
                "FABUSHI_UI_REDUCE_MOTION": "1",
                "FABUSHI_UI_HIGH_CONTRAST": "1",
                "FABUSHI_E2E_REQUIRE_PASSCODE": "1",
                "FABUSHI_E2E_SIMULATE_CALL_PERMISSION_DENIED": "1",
                "FABUSHI_E2E_SIMULATE_REPLAYKIT_DENIED": "1",
                "FABUSHI_E2E_FORM_FACTOR": "ipad",
            ],
            arguments: [
                "Fabushi",
                "--server-url", "http://127.0.0.1:18080///",
                "-AppleLanguages", "(zh-Hans)",
                "-AppleLocale", "zh_CN",
            ]
        )

        XCTAssertTrue(cliConfig.isUITestMode)
        XCTAssertTrue(cliConfig.disableLegacyFeatureHostTest)
        XCTAssertFalse(cliConfig.shouldUseLegacyProductionRenderer)
        XCTAssertEqual(cliConfig.serverURL, "http://127.0.0.1:18080")
        XCTAssertEqual(cliConfig.appDataDirectory?.path, dir.path)
        XCTAssertFalse(cliConfig.resetState)
        XCTAssertEqual(cliConfig.themePreset, .tinted)
        XCTAssertEqual(cliConfig.accentHex, "#FFFF00")
        XCTAssertTrue(cliConfig.reduceMotion)
        XCTAssertTrue(cliConfig.highContrast)
        XCTAssertTrue(cliConfig.requirePasscode)
        XCTAssertTrue(cliConfig.simulateCallPermissionDenied)
        XCTAssertTrue(cliConfig.simulateReplayKitDenied)
        XCTAssertEqual(cliConfig.preferredLocale, .zhHans)
        XCTAssertEqual(cliConfig.formFactorHint, "ipad")

        // resetState == false MUST preserve persisted files for cold-restart durability
        try cliConfig.prepareStateDirectoryIfNeeded()
        XCTAssertTrue(FileManager.default.fileExists(atPath: markerFile.path))

        // 2. `--server-url=<url>` syntax & resetState == true cleans directory
        let resetConfig = FabushiLaunchConfiguration(
            environment: [
                "FABUSHI_APP_DATA": dir.path,
                "FABUSHI_E2E_RESET_STATE": "1",
                "FABUSHI_UI_ACCENT_HEX": "#123",
            ],
            arguments: [
                "Fabushi",
                "--server-url=https://fabushi.example.com/",
                "-AppleLanguages", "(en)",
            ]
        )
        XCTAssertEqual(resetConfig.serverURL, "https://fabushi.example.com")
        XCTAssertNil(resetConfig.accentHex)
        XCTAssertEqual(resetConfig.invalidLaunchAccentHex, "#123")
        XCTAssertEqual(resetConfig.preferredLocale, .en)

        try resetConfig.prepareStateDirectoryIfNeeded()
        XCTAssertFalse(FileManager.default.fileExists(atPath: markerFile.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: dir.path))

        // 3. Env precedence & legacy renderer routing matrix
        let envTier2 = FabushiLaunchConfiguration(
            environment: [
                "GPUI_TELEGRAM_SERVER_URL": "http://127.0.0.1:19100/",
                "FABUSHI_E2E_SERVER_URL": "http://127.0.0.1:19200/",
            ],
            arguments: []
        )
        XCTAssertEqual(envTier2.serverURL, "http://127.0.0.1:19100")

        let legacySmoke = FabushiLaunchConfiguration(
            environment: ["FABUSHI_FEATURE_HOST_SMOKE": "1"],
            arguments: []
        )
        XCTAssertTrue(legacySmoke.shouldUseLegacyProductionRenderer)

        let legacyCISessionOverriddenByUITest = FabushiLaunchConfiguration(
            environment: [
                "FABUSHI_UI_TEST_MODE": "1",
                "FABUSHI_CI_ACCOUNT_SESSION_BASE64": "eyJ0b2tlbiI6InRlc3QifQ==",
            ],
            arguments: []
        )
        XCTAssertFalse(legacyCISessionOverriddenByUITest.shouldUseLegacyProductionRenderer)
    }

    func testLocalizableXCStringsBilingualZhHansAndEnNoRawKeyLeaks() throws {
        let hanRegex = try NSRegularExpression(pattern: "\\p{Han}")
        let asciiLetterRegex = try NSRegularExpression(pattern: "[A-Za-z]")
        let forbiddenSampleTitles = [
            "Saved Messages",
            "Fabushi Community",
            "Sample Chat",
            "Demo Bot Chat",
            "Chief of Staff",
        ]

        let requiredKeys = [
            "hig.settings.appearance",
            "hig.settings.language",
            "hig.settings.privacySecurity",
            "hig.settings.capabilities",
            "hig.theme.night",
            "hig.theme.tinted",
            "hig.theme.day",
            "hig.theme.classic",
            "hig.tab.chats",
            "hig.tab.contacts",
            "hig.tab.studio",
            "hig.tab.remote",
            "hig.tab.calls",
            "hig.tab.settings",
            "hig.common.retry",
            "hig.common.close",
            "hig.common.cancel",
            "hig.common.back",
            "hig.offline.title",
            "hig.providerError.title",
            "hig.providerError.openProviders",
            "hig.appearance.contrastAdjusted",
            "hig.appearance.contrastPass",
            "hig.appearance.reduceMotion",
            "hig.appearance.highContrast",
            "hig.capabilities.upstreamPlanned",
            "hig.capabilities.serverBlocked",
        ]

        for key in requiredKeys {
            let zh = FabushiLocale.zhHans.localized(key, bundle: .main)
            let en = FabushiLocale.en.localized(key, bundle: .main)

            XCTAssertNotEqual(zh, key, "zh-Hans key \(key) fell back to raw key")
            XCTAssertNotEqual(en, key, "en key \(key) fell back to raw key")
            XCTAssertNotEqual(zh, en, "zh-Hans and en must differ for \(key)")

            let zhRange = NSRange(zh.startIndex..<zh.endIndex, in: zh)
            XCTAssertNotNil(
                hanRegex.firstMatch(in: zh, range: zhRange),
                "Expected Han characters in zh-Hans translation for \(key), got: \(zh)"
            )

            let enRange = NSRange(en.startIndex..<en.endIndex, in: en)
            XCTAssertNotNil(
                asciiLetterRegex.firstMatch(in: en, range: enRange),
                "Expected ASCII letters in en translation for \(key), got: \(en)"
            )
            XCTAssertNil(
                hanRegex.firstMatch(in: en, range: enRange),
                "Unexpected Han characters in en translation for \(key): \(en)"
            )

            for value in [zh, en] {
                for prefix in FabushiAccessibilityIDs.forbiddenRawLocalizationKeyPrefixes {
                    XCTAssertFalse(
                        value.lowercased().contains(prefix),
                        "Raw key prefix '\(prefix)' leaked in localized string '\(value)' for key \(key)"
                    )
                }
                for forbiddenSample in forbiddenSampleTitles {
                    XCTAssertFalse(
                        value.localizedCaseInsensitiveContains(forbiddenSample),
                        "Forbidden sample chat title '\(forbiddenSample)' appeared in localized string '\(value)'"
                    )
                }
            }
        }
    }

    func testAccessibilityIDsCatalogContract13NamespacesAnd281Identifiers() {
        XCTAssertEqual(FabushiAccessibilityIDs.allRequiredNamespaces.count, 13)
        XCTAssertEqual(FabushiAccessibilityIDs.Auth.allIdentifiers.count, 28)
        XCTAssertEqual(FabushiAccessibilityIDs.Shell.allIdentifiers.count, 28)
        XCTAssertEqual(FabushiAccessibilityIDs.ChatList.allIdentifiers.count, 26)
        XCTAssertEqual(FabushiAccessibilityIDs.Compose.allIdentifiers.count, 10)
        XCTAssertEqual(FabushiAccessibilityIDs.Stage.allIdentifiers.count, 36)
        XCTAssertEqual(FabushiAccessibilityIDs.Composer.allIdentifiers.count, 18)
        XCTAssertEqual(FabushiAccessibilityIDs.Inspector.allIdentifiers.count, 22)
        XCTAssertEqual(FabushiAccessibilityIDs.Studio.allIdentifiers.count, 17)
        XCTAssertEqual(FabushiAccessibilityIDs.Magpie.allIdentifiers.count, 9)
        XCTAssertEqual(FabushiAccessibilityIDs.Remote.allIdentifiers.count, 16)
        XCTAssertEqual(FabushiAccessibilityIDs.MiniApp.allIdentifiers.count, 14)
        XCTAssertEqual(FabushiAccessibilityIDs.Call.allIdentifiers.count, 14)
        XCTAssertEqual(FabushiAccessibilityIDs.Settings.allIdentifiers.count, 43)

        var seen = Set<String>()
        var total = 0
        for namespace in FabushiAccessibilityIDs.allRequiredNamespaces {
            let ids = FabushiAccessibilityIDs.allIdentifiersByNamespace[namespace] ?? []
            XCTAssertFalse(ids.isEmpty, "Namespace \(namespace) must have identifiers")
            for identifier in ids {
                XCTAssertTrue(
                    identifier.hasPrefix("\(namespace)."),
                    "Identifier \(identifier) must start with \(namespace)."
                )
                XCTAssertTrue(seen.insert(identifier).inserted, "Duplicate accessibility ID: \(identifier)")
                total += 1
            }
        }
        XCTAssertEqual(total, 281)
    }

    func testThemePersistenceRoundTripThroughRealRustHostLocalSettings() async throws {
        let dir = try makeTempDir("theme_persist")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.theme.\(UUID().uuidString)",
            account: "theme-test"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)

        do {
            let writerTheme = FabushiTheme(preset: .night, locale: .en)
            writerTheme.selectPreset(.tinted)
            XCTAssertTrue(writerTheme.applyCustomAccentHex("#FFFF00"))
            writerTheme.setBubbleCornerRadius(12)
            writerTheme.selectWallpaper(try XCTUnwrap(FabushiWallpaper.preset(id: "aurora_dusk")))
            writerTheme.setLocale(.zhHans)
            writerTheme.setReduceMotion(true)
            writerTheme.setHighContrast(true)

            let saved = await writerTheme.persistPreferences(using: coordinator)
            XCTAssertTrue(saved, "Real Rust settings.localSet must persist all UI preferences")

            // Reload into a fresh FabushiTheme without launch overrides
            let readerTheme = FabushiTheme(preset: .day, locale: .en)
            await readerTheme.loadPersistedPreferences(using: coordinator)
            XCTAssertEqual(readerTheme.preset, .tinted)
            XCTAssertEqual(readerTheme.customAccentHex, "#FFFF00")
            XCTAssertEqual(readerTheme.effectiveAccentHex, "#A9A906")
            XCTAssertEqual(readerTheme.bubbleCornerRadius, 12)
            XCTAssertEqual(readerTheme.wallpaper.id, "aurora_dusk")
            XCTAssertEqual(readerTheme.locale, .zhHans)
            XCTAssertTrue(readerTheme.reduceMotionOverride)
            XCTAssertTrue(readerTheme.highContrastOverride)

            // Verify explicit launch overrides take precedence over persisted values
            let launchOverrideTheme = FabushiTheme(preset: .night, locale: .zhHans)
            let launchConfig = FabushiLaunchConfiguration(
                environment: [
                    "FABUSHI_UI_THEME": "day",
                    "FABUSHI_UI_ACCENT_HEX": "#FFFFFF",
                ],
                arguments: ["-AppleLanguages", "(en)"]
            )
            launchOverrideTheme.applyLaunchConfiguration(launchConfig)
            await launchOverrideTheme.loadPersistedPreferences(using: coordinator)
            XCTAssertEqual(launchOverrideTheme.preset, .day)
            XCTAssertEqual(launchOverrideTheme.effectiveAccentHex, "#A9A9A9")
            XCTAssertEqual(launchOverrideTheme.locale, .en)

            // Verify UI-test-mode cold restart omits env flags and overrides stale persisted toggles to false
            let uiTestLaunchTheme = FabushiTheme(preset: .night, locale: .en)
            let uiTestLaunch2 = FabushiLaunchConfiguration(
                environment: [
                    "FABUSHI_UI_TEST_MODE": "1",
                    "FABUSHI_UI_THEME": "tinted",
                ],
                arguments: []
            )
            uiTestLaunchTheme.applyLaunchConfiguration(uiTestLaunch2)
            await uiTestLaunchTheme.loadPersistedPreferences(using: coordinator)
            XCTAssertFalse(uiTestLaunchTheme.reduceMotionOverride)
            XCTAssertFalse(uiTestLaunchTheme.highContrastOverride)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    func testPrepareStateDirectoryNeverWipesProductionWhenAppDataNil() throws {
        // 1. resetState == true with appDataDirectory == nil MUST be a safe no-op (never fallback to production)
        let nilDataDirReset = FabushiLaunchConfiguration(
            environment: ["FABUSHI_E2E_RESET_STATE": "1"],
            arguments: []
        )
        XCTAssertTrue(nilDataDirReset.resetState)
        XCTAssertNil(nilDataDirReset.appDataDirectory)
        XCTAssertNoThrow(try nilDataDirReset.prepareStateDirectoryIfNeeded())

        // 2. Root `/`, `$HOME`, `temporaryDirectory` root, and production `Application Support/FabushiCore`
        // MUST all be rejected fail-closed with `FabushiLaunchConfigurationError.unsafeResetStateDirectory`.
        var unsafePaths: [String] = [
            "/",
            NSHomeDirectory(),
            FileManager.default.temporaryDirectory.path,
        ]
        if let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first {
            unsafePaths.append(appSupport.path)
            unsafePaths.append(appSupport.appendingPathComponent("FabushiCore", isDirectory: true).path)
            unsafePaths.append(
                appSupport
                    .appendingPathComponent("FabushiCore", isDirectory: true)
                    .appendingPathComponent("nested_db", isDirectory: true)
                    .path
            )
        }

        for unsafePath in unsafePaths {
            let unsafeConfig = FabushiLaunchConfiguration(
                environment: [
                    "FABUSHI_APP_DATA": unsafePath,
                    "FABUSHI_E2E_RESET_STATE": "1",
                ],
                arguments: []
            )
            XCTAssertThrowsError(try unsafeConfig.prepareStateDirectoryIfNeeded()) { error in
                guard case FabushiLaunchConfigurationError.unsafeResetStateDirectory = error else {
                    XCTFail("Expected unsafeResetStateDirectory for '\(unsafePath)', got: \(error)")
                    return
                }
            }
        }

        // 3. Explicit "0"/"1" reduceMotion/highContrast overrides & localized invalid launch hex error
        let explicitZero = FabushiLaunchConfiguration(
            environment: [
                "FABUSHI_UI_REDUCE_MOTION": "0",
                "FABUSHI_UI_HIGH_CONTRAST": "0",
                "FABUSHI_UI_ACCENT_HEX": "fabushi.raw_env_leak",
            ],
            arguments: ["-AppleLanguages", "(en)"]
        )
        XCTAssertEqual(explicitZero.reduceMotionLaunchOverride, false)
        XCTAssertEqual(explicitZero.highContrastLaunchOverride, false)
        let theme = FabushiTheme(preset: .night, reduceMotionOverride: true, highContrastOverride: true)
        theme.applyLaunchConfiguration(explicitZero)
        XCTAssertFalse(theme.reduceMotionOverride)
        XCTAssertFalse(theme.highContrastOverride)
        let errText = try XCTUnwrap(theme.lastAccentValidationError)
        XCTAssertFalse(errText.contains("fabushi."), "Untrusted raw env input must never leak into UI error text")
    }

    func testHighContrastOutgoingBubbleInkMaintainsWCAGAAAcrossDefaultFamily() {
        // Explicit regression check for Night + #40A7E3 (default family) + highContrast == true
        let nightSkyHC = FabushiThemePalette.resolve(
            preset: .night,
            customAccentRGB: 0x40A7E3,
            highContrast: true
        )
        XCTAssertEqual(nightSkyHC.bubbleOutgoingBaseRGB, 0x116DD2)
        XCTAssertEqual(nightSkyHC.bubbleOutgoingTextRGB, 0xFFFFFF)
        XCTAssertGreaterThanOrEqual(
            FabushiColorHex.contrastRatio(
                foreground: nightSkyHC.bubbleOutgoingTextRGB,
                background: nightSkyHC.bubbleOutgoingBaseRGB
            ),
            4.5
        )

        // Exhaustive check across all 4 presets & representative default-family / custom accents
        let candidateAccents: [UInt32?] = [
            nil, 0x0A84FF, 0x007AFF, 0x40A7E3, 0x419FD9, 0x2AABEE, 0x3FC1B0, 0xFFFF00, 0xFFFFFF, 0xE86C52,
        ]
        for preset in FabushiThemePreset.allCases {
            for accent in candidateAccents {
                let palette = FabushiThemePalette.resolve(
                    preset: preset,
                    customAccentRGB: accent,
                    highContrast: true
                )
                let ratio = FabushiColorHex.contrastRatio(
                    foreground: palette.bubbleOutgoingTextRGB,
                    background: palette.bubbleOutgoingBaseRGB
                )
                XCTAssertGreaterThanOrEqual(
                    ratio,
                    4.5,
                    "High Contrast outgoing bubble ink must guarantee >= 4.5:1 for \(preset.rawValue) + \(accent.map(FabushiColorHex.format) ?? "default"), got \(ratio)"
                )
            }
        }
    }

    func testFullLocalizableCatalogBilingualParityAllKeys() throws {
        let hanRegex = try NSRegularExpression(pattern: "\\p{Han}")
        let asciiLetterRegex = try NSRegularExpression(pattern: "[A-Za-z]")
        let forbiddenSampleTitles = [
            "Saved Messages",
            "Fabushi Community",
            "Sample Chat",
            "Demo Bot Chat",
            "Chief of Staff",
        ]

        let allKeys = FabushiLocale.allCatalogKeys(in: .main)
        XCTAssertGreaterThanOrEqual(
            allKeys.count,
            101,
            "Expected all >= 101 compiled Localizable.xcstrings keys in Bundle.main, found \(allKeys.count)"
        )

        // Verify newly added Iteration 2 keys are present
        let mustIncludeKeys = [
            "hig.avatar.botBadge",
            "hig.status.starting",
            "hig.swatch.systemBlue",
            "hig.swatch.skyBlue",
            "hig.swatch.tealMint",
            "hig.swatch.emerald",
            "hig.swatch.amberGold",
            "hig.swatch.coralOrange",
            "hig.swatch.crimsonRose",
            "hig.swatch.lavender",
        ]
        for key in mustIncludeKeys {
            XCTAssertTrue(allKeys.contains(key), "Missing required catalog key '\(key)' in compiled bundle")
        }

        for key in allKeys.sorted() {
            let zh = FabushiLocale.zhHans.localized(key, bundle: .main)
            let en = FabushiLocale.en.localized(key, bundle: .main)

            XCTAssertFalse(zh.isEmpty, "Empty zh-Hans translation for \(key)")
            XCTAssertFalse(en.isEmpty, "Empty en translation for \(key)")
            XCTAssertNotEqual(zh, key, "zh-Hans key \(key) fell back to raw key")
            XCTAssertNotEqual(en, key, "en key \(key) fell back to raw key")
            XCTAssertNotEqual(zh, en, "zh-Hans and en must differ for \(key)")

            let zhRange = NSRange(zh.startIndex..<zh.endIndex, in: zh)
            XCTAssertNotNil(
                hanRegex.firstMatch(in: zh, range: zhRange),
                "Expected Han characters in zh-Hans translation for \(key), got: \(zh)"
            )

            let enRange = NSRange(en.startIndex..<en.endIndex, in: en)
            XCTAssertNotNil(
                asciiLetterRegex.firstMatch(in: en, range: enRange),
                "Expected ASCII letters in en translation for \(key), got: \(en)"
            )
            XCTAssertNil(
                hanRegex.firstMatch(in: en, range: enRange),
                "Unexpected Han characters in en translation for \(key): \(en)"
            )

            for value in [zh, en] {
                for prefix in FabushiAccessibilityIDs.forbiddenRawLocalizationKeyPrefixes {
                    XCTAssertFalse(
                        value.lowercased().contains(prefix),
                        "Raw key prefix '\(prefix)' leaked in localized string '\(value)' for key \(key)"
                    )
                }
                for forbiddenSample in forbiddenSampleTitles {
                    XCTAssertFalse(
                        value.localizedCaseInsensitiveContains(forbiddenSample),
                        "Forbidden sample chat title '\(forbiddenSample)' appeared in localized string '\(value)'"
                    )
                }
            }
        }
    }

    func testPrepareStateDirectoryRejectsCaseInsensitiveAPFSAndAncestorPaths() async throws {
        // 1. Case-insensitive APFS & ancestor path rejection (`M1`)
        let appSupport = try XCTUnwrap(
            FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
        )
        let libraryURL = appSupport.deletingLastPathComponent()
        let appContainerURL = libraryURL.deletingLastPathComponent()
        let simulatorParentURL = appContainerURL.deletingLastPathComponent()

        let unsafePaths: [String] = [
            appSupport.appendingPathComponent("fabushicore", isDirectory: true).path,
            appSupport.appendingPathComponent("FABUSHICORE", isDirectory: true)
                .appendingPathComponent("sub", isDirectory: true)
                .path,
            libraryURL.appendingPathComponent("application support", isDirectory: true)
                .appendingPathComponent("FaBuShIcOrE", isDirectory: true)
                .path,
            libraryURL.path,
            appContainerURL.path,
            simulatorParentURL.path,
            "/SYSTEM",
            "/Private/Tmp",
        ]

        for unsafePath in unsafePaths {
            let unsafeConfig = FabushiLaunchConfiguration(
                environment: [
                    "FABUSHI_APP_DATA": unsafePath,
                    "FABUSHI_E2E_RESET_STATE": "1",
                ],
                arguments: []
            )
            XCTAssertThrowsError(try unsafeConfig.prepareStateDirectoryIfNeeded()) { error in
                guard case FabushiLaunchConfigurationError.unsafeResetStateDirectory = error else {
                    XCTFail("Expected unsafeResetStateDirectory for '\(unsafePath)', got: \(error)")
                    return
                }
            }
        }

        // Verify a genuinely isolated temp directory STILL resets cleanly (non-vacuous guard)
        let safeDir = try makeTempDir("safe_reset_m1")
        defer { try? FileManager.default.removeItem(at: safeDir) }
        let sentinelFile = safeDir.appendingPathComponent("stale.txt")
        try Data("stale".utf8).write(to: sentinelFile)
        let safeConfig = FabushiLaunchConfiguration(
            environment: [
                "FABUSHI_APP_DATA": safeDir.path,
                "FABUSHI_E2E_RESET_STATE": "1",
            ],
            arguments: []
        )
        XCTAssertNoThrow(try safeConfig.prepareStateDirectoryIfNeeded())
        XCTAssertFalse(FileManager.default.fileExists(atPath: sentinelFile.path))

        // 2. Cancelled `persistPreferences` task aborts immediately without overwriting SQLite (`M4`)
        let persistDir = try makeTempDir("cancelled_persist_m4")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.theme.cancel.\(UUID().uuidString)",
            account: "theme-cancel-test"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: persistDir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(appDataDirectory: persistDir, testMode: true, startNetworkMonitor: false)

        do {
            let baselineTheme = FabushiTheme(preset: .day, locale: .en)
            baselineTheme.setBubbleCornerRadius(10)
            let baselineSaved = await baselineTheme.persistPreferences(using: coordinator)
            XCTAssertTrue(baselineSaved)

            let mutatedTheme = FabushiTheme(preset: .classic, locale: .zhHans)
            mutatedTheme.setBubbleCornerRadius(22)

            let cancelledTask = Task { @MainActor in
                await mutatedTheme.persistPreferences(using: coordinator)
            }
            cancelledTask.cancel()
            let cancelledSaved = await cancelledTask.value
            XCTAssertFalse(
                cancelledSaved,
                "Cancelled persistPreferences task must return false without writing out-of-order state"
            )

            let reloadedTheme = FabushiTheme(preset: .night, locale: .en)
            await reloadedTheme.loadPersistedPreferences(using: coordinator)
            XCTAssertEqual(reloadedTheme.preset, .day)
            XCTAssertEqual(reloadedTheme.bubbleCornerRadius, 10)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
