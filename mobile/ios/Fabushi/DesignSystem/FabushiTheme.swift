import Foundation
import Observation
import os
import SwiftUI

// MARK: - Theme Preset (Exact Upstream v1.6.9 4-Theme Matrix)

/// The 4 canonical `bhrumom/gpui-telegram@57a62ef92be2` (`Fabushi v1.6.9`) color themes.
public enum FabushiThemePreset: String, CaseIterable, Codable, Sendable, Identifiable {
    case night = "Night"
    case tinted = "Tinted"
    case day = "Day"
    case classic = "Classic"

    public var id: String { rawValue }

    /// Case-insensitive upstream alias parser (`palette_schemes.rs`).
    public static func fromName(_ raw: String) -> FabushiThemePreset {
        let normalized = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        switch normalized {
        case "night", "night-default", "dark":
            return .night
        case "tinted", "night tinted", "night_tinted", "night-green", "midnight":
            return .tinted
        case "day", "day blue", "day_blue", "day-blue", "light":
            return .day
        case "classic", "day classic", "day_classic":
            return .classic
        default:
            return .night
        }
    }

    public var isDark: Bool {
        switch self {
        case .night, .tinted:
            return true
        case .day, .classic:
            return false
        }
    }

    public var isAppleStyle: Bool {
        switch self {
        case .night, .tinted, .day:
            return true
        case .classic:
            return false
        }
    }

    public var defaultAccentRGB: UInt32 {
        switch self {
        case .night, .tinted:
            return 0x0A84FF
        case .day:
            return 0x007AFF
        case .classic:
            return 0x40A7E3
        }
    }

    public var defaultAccentHex: String {
        FabushiColorHex.format(defaultAccentRGB)
    }

    public var defaultWallpaperID: String {
        switch self {
        case .night: return "default_night"
        case .tinted: return "default_tinted"
        case .day: return "default_day"
        case .classic: return "default_classic"
        }
    }

    /// Upstream theme cycle order (`Night -> Tinted -> Day -> Classic -> Night`).
    public var nextInCycle: FabushiThemePreset {
        switch self {
        case .night: return .tinted
        case .tinted: return .day
        case .day: return .classic
        case .classic: return .night
        }
    }

    public var localizationKey: String {
        switch self {
        case .night: return "hig.theme.night"
        case .tinted: return "hig.theme.tinted"
        case .day: return "hig.theme.day"
        case .classic: return "hig.theme.classic"
        }
    }

    public var accessibilityIdentifier: String {
        switch self {
        case .night: return FabushiAccessibilityIDs.Settings.themeNightButton
        case .tinted: return FabushiAccessibilityIDs.Settings.themeTintedButton
        case .day: return FabushiAccessibilityIDs.Settings.themeDayButton
        case .classic: return FabushiAccessibilityIDs.Settings.themeClassicButton
        }
    }
}

// MARK: - Wallpapers, Accent Swatches & Peer Palettes (Exact Upstream v1.6.9)

public struct FabushiWallpaper: Sendable, Equatable, Hashable, Identifiable {
    public let id: String
    public let nameKey: String
    public let fallbackName: String
    public let topHex: UInt32
    public let bottomHex: UInt32
    public let patternHex: UInt32
    public let isDark: Bool

    public init(
        id: String,
        nameKey: String,
        fallbackName: String,
        topHex: UInt32,
        bottomHex: UInt32,
        patternHex: UInt32,
        isDark: Bool
    ) {
        self.id = id
        self.nameKey = nameKey
        self.fallbackName = fallbackName
        self.topHex = topHex
        self.bottomHex = bottomHex
        self.patternHex = patternHex
        self.isDark = isDark
    }

    public var gradient: LinearGradient {
        LinearGradient(
            colors: [Color(fabushiHex: topHex), Color(fabushiHex: bottomHex)],
            startPoint: .top,
            endPoint: .bottom
        )
    }

    public static let presets: [FabushiWallpaper] = [
        FabushiWallpaper(
            id: "default_night",
            nameKey: "hig.wallpaper.defaultNight",
            fallbackName: "Apple Dark",
            topHex: 0x1E1E1E,
            bottomHex: 0x1E1E1E,
            patternHex: 0x2C2C2E,
            isDark: true
        ),
        FabushiWallpaper(
            id: "default_tinted",
            nameKey: "hig.wallpaper.defaultTinted",
            fallbackName: "Apple Midnight",
            topHex: 0x181C22,
            bottomHex: 0x181C22,
            patternHex: 0x242A33,
            isDark: true
        ),
        FabushiWallpaper(
            id: "default_day",
            nameKey: "hig.wallpaper.defaultDay",
            fallbackName: "Apple Light",
            topHex: 0xFFFFFF,
            bottomHex: 0xFFFFFF,
            patternHex: 0xE5E5EA,
            isDark: false
        ),
        FabushiWallpaper(
            id: "default_classic",
            nameKey: "hig.wallpaper.defaultClassic",
            fallbackName: "Telegram Classic",
            topHex: 0xD5E8C8,
            bottomHex: 0xB8D8A3,
            patternHex: 0x93BD7E,
            isDark: false
        ),
        FabushiWallpaper(
            id: "aurora_dusk",
            nameKey: "hig.wallpaper.auroraDusk",
            fallbackName: "Aurora Dusk",
            topHex: 0x181B2C,
            bottomHex: 0x231E36,
            patternHex: 0x363058,
            isDark: true
        ),
        FabushiWallpaper(
            id: "desert_sand",
            nameKey: "hig.wallpaper.desertSand",
            fallbackName: "Warm Sand",
            topHex: 0xF2E9DB,
            bottomHex: 0xE7D7C1,
            patternHex: 0xCCB698,
            isDark: false
        ),
        FabushiWallpaper(
            id: "monochrome_slate",
            nameKey: "hig.wallpaper.monochromeSlate",
            fallbackName: "Graphite Slate",
            topHex: 0x171A1F,
            bottomHex: 0x1F242B,
            patternHex: 0x2D343D,
            isDark: true
        ),
    ]

    public static func preset(id: String) -> FabushiWallpaper? {
        presets.first { $0.id == id }
    }

    public static func defaultFor(_ preset: FabushiThemePreset) -> FabushiWallpaper {
        self.preset(id: preset.defaultWallpaperID) ?? presets[0]
    }
}

public struct FabushiAccentSwatch: Sendable, Equatable, Hashable, Identifiable {
    public let id: String
    public let name: String
    public let rgb: UInt32

    public var hexString: String { FabushiColorHex.format(rgb) }

    public static let curated: [FabushiAccentSwatch] = [
        FabushiAccentSwatch(id: "system_blue", name: "System Blue", rgb: 0x007AFF),
        FabushiAccentSwatch(id: "sky_blue", name: "Sky Blue", rgb: 0x40A7E3),
        FabushiAccentSwatch(id: "teal_mint", name: "Teal Mint", rgb: 0x3FC1B0),
        FabushiAccentSwatch(id: "emerald", name: "Emerald", rgb: 0x34C759),
        FabushiAccentSwatch(id: "amber_gold", name: "Amber Gold", rgb: 0xE5A030),
        FabushiAccentSwatch(id: "coral_orange", name: "Coral Orange", rgb: 0xE86C52),
        FabushiAccentSwatch(id: "crimson_rose", name: "Crimson Rose", rgb: 0xE16794),
        FabushiAccentSwatch(id: "lavender", name: "Lavender", rgb: 0x8544D6),
    ]
}

public enum FabushiPeerColorPalette: Sendable {
    public static let nameColorsDay: [UInt32] = [
        0xC03D33, 0x4FAD2D, 0xD09306, 0x168ACD, 0x8544D6, 0xCD4073, 0x2996AD,
    ]
    public static let nameColorsNight: [UInt32] = [
        0xFB6169, 0x85DE85, 0xF3BC5C, 0x65BDF3, 0xB48BF2, 0xFF5694, 0x62D4E3,
    ]
    public static let nameColorsTinted: [UInt32] = [
        0xEC7577, 0x86D67F, 0xE4C054, 0x68C7F3, 0xB383F3, 0xE16794, 0x57C9E0,
    ]
    public static let avatarGradientTop: [UInt32] = [
        0xFF845E, 0x9AD164, 0xECC65B, 0x5CAFFA, 0xB694F9, 0xFF8AAC, 0x5BCBE3,
    ]
    public static let avatarGradientBottom: [UInt32] = [
        0xD45246, 0x46BA43, 0xD59F36, 0x408ACF, 0x6C61DF, 0xD95574, 0x359AD4,
    ]

    public static func colorIndex(forPeerId peerId: Int64) -> Int {
        Int(peerId.magnitude % 7)
    }

    public static func peerNameRGB(peerId: Int64, preset: FabushiThemePreset) -> UInt32 {
        let index = colorIndex(forPeerId: peerId)
        switch preset {
        case .night:
            return nameColorsNight[index]
        case .tinted:
            return nameColorsTinted[index]
        case .day, .classic:
            return nameColorsDay[index]
        }
    }

    public static func avatarGradientRGB(peerId: Int64) -> (top: UInt32, bottom: UInt32) {
        let index = colorIndex(forPeerId: peerId)
        return (avatarGradientTop[index], avatarGradientBottom[index])
    }

    public struct BotHueTriplet: Sendable, Equatable {
        public let bubbleFillRGB: UInt32
        public let pillBackgroundRGB: UInt32
        public let pillTextRGB: UInt32
    }

    public static func botHue(named rawName: String, isDark: Bool) -> BotHueTriplet {
        switch rawName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "red":
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0xCF2B35, pillBackgroundRGB: 0x36191C, pillTextRGB: 0xFF6463)
                : BotHueTriplet(bubbleFillRGB: 0xC82532, pillBackgroundRGB: 0xFFEAEC, pillTextRGB: 0xCD2834)
        case "green":
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0x0E9F5A, pillBackgroundRGB: 0x162E22, pillTextRGB: 0x00BA69)
                : BotHueTriplet(bubbleFillRGB: 0x0D9655, pillBackgroundRGB: 0xE7F9F0, pillTextRGB: 0x00854A)
        case "yellow", "warn":
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0xC8770C, pillBackgroundRGB: 0x352815, pillTextRGB: 0xE28600)
                : BotHueTriplet(bubbleFillRGB: 0xC2730B, pillBackgroundRGB: 0xFEF4E7, pillTextRGB: 0xAA6300)
        case "orange":
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0xCD5309, pillBackgroundRGB: 0x362115, pillTextRGB: 0xFE6600)
                : BotHueTriplet(bubbleFillRGB: 0xD2550A, pillBackgroundRGB: 0xFFF0E7, pillTextRGB: 0xC14C00)
        case "cyan":
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0x0E9886, pillBackgroundRGB: 0x162D2A, pillTextRGB: 0x00B7A2)
                : BotHueTriplet(bubbleFillRGB: 0x0D9381, pillBackgroundRGB: 0xE7F8F6, pillTextRGB: 0x008273)
        case "purple":
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0x7845D7, pillBackgroundRGB: 0x271F39, pillTextRGB: 0xA582FF)
                : BotHueTriplet(bubbleFillRGB: 0x7442D1, pillBackgroundRGB: 0xF4EEFE, pillTextRGB: 0x7547D0)
        case "magenta":
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0xCF297E, pillBackgroundRGB: 0x361A28, pillTextRGB: 0xFF55A4)
                : BotHueTriplet(bubbleFillRGB: 0xC82779, pillBackgroundRGB: 0xFEEBF5, pillTextRGB: 0xC62378)
        case "brown":
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0x8C5D31, pillBackgroundRGB: 0x2A221B, pillTextRGB: 0xC59E7D)
                : BotHueTriplet(bubbleFillRGB: 0x86592E, pillBackgroundRGB: 0xF5F0EB, pillTextRGB: 0x996B42)
        default:
            return isDark
                ? BotHueTriplet(bubbleFillRGB: 0x116DD2, pillBackgroundRGB: 0x162639, pillTextRGB: 0x4D9BFF)
                : BotHueTriplet(bubbleFillRGB: 0x126FD4, pillBackgroundRGB: 0xE7F2FF, pillTextRGB: 0x0B6DD3)
        }
    }
}

// MARK: - Bilingual Locale (`zh-Hans` & `en`)

public enum FabushiLocale: String, CaseIterable, Codable, Sendable, Identifiable {
    case zhHans = "zh-Hans"
    case en = "en"

    public var id: String { rawValue }

    /// Upstream `is_zh_lang` normalization (`zh`, `zh-CN`, `zh_CN`, `zh-Hans` -> `.zhHans`; else `.en`).
    public init(normalizing code: String) {
        let lower = code.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if lower == "zh" || lower.hasPrefix("zh-") || lower.hasPrefix("zh_") {
            self = .zhHans
        } else {
            self = .en
        }
    }

    public static func systemDefault() -> FabushiLocale {
        if let preferred = Locale.preferredLanguages.first, !preferred.isEmpty {
            return FabushiLocale(normalizing: preferred)
        }
        return FabushiLocale(normalizing: Locale.current.identifier)
    }

    public var foundationLocale: Locale {
        Locale(identifier: rawValue)
    }

    public var nativeDisplayName: String {
        switch self {
        case .zhHans: return "简体中文"
        case .en: return "English"
        }
    }

    /// Resolves a key strictly from the compiled Apple String Catalog (`Localizable.xcstrings` ->
    /// `Localizable.loctable`) for this exact locale, supporting both OS launch languages and live
    /// in-app `zh-Hans`/`en` switching.
    public func localized(_ key: String, bundle: Bundle = .main) -> String {
        if let value = FabushiLoctableCache.lookup(key: key, localeCode: rawValue, in: bundle) {
            return value
        }
        if let lprojPath = bundle.path(forResource: rawValue, ofType: "lproj"),
           let localizedBundle = Bundle(path: lprojPath) {
            let resolved = localizedBundle.localizedString(forKey: key, value: nil, table: "Localizable")
            if resolved != key {
                return resolved
            }
        }
        return String(
            localized: String.LocalizationValue(key),
            table: nil,
            bundle: bundle,
            locale: foundationLocale
        )
    }

    public func localizedFormat(_ key: String, bundle: Bundle = .main, _ arguments: CVarArg...) -> String {
        let format = localized(key, bundle: bundle)
        return String(format: format, locale: foundationLocale, arguments: arguments)
    }
}

private enum FabushiLoctableCache {
    private static let lock = OSAllocatedUnfairLock(initialState: [String: [String: [String: String]]]())

    static func lookup(key: String, localeCode: String, in bundle: Bundle) -> String? {
        let bundlePath = bundle.bundlePath
        let tables: [String: [String: String]] = lock.withLock { cache in
            if let existing = cache[bundlePath] {
                return existing
            }
            let loaded = loadTables(from: bundle)
            cache[bundlePath] = loaded
            return loaded
        }

        if let exact = tables[localeCode]?[key] {
            return exact
        }
        let normalizedTarget = localeCode.lowercased().replacingOccurrences(of: "_", with: "-")
        for (langKey, dict) in tables {
            if langKey.lowercased().replacingOccurrences(of: "_", with: "-") == normalizedTarget,
               let matched = dict[key] {
                return matched
            }
        }
        return nil
    }

    private static func loadTables(from bundle: Bundle) -> [String: [String: String]] {
        guard let url = bundle.url(forResource: "Localizable", withExtension: "loctable"),
              let data = try? Data(contentsOf: url),
              let plist = try? PropertyListSerialization.propertyList(
                  from: data,
                  options: [],
                  format: nil
              ),
              let root = plist as? [String: Any]
        else {
            return [:]
        }

        let langContainer = (root["localizations"] as? [String: Any]) ?? root
        var result: [String: [String: String]] = [:]
        for (langCode, rawTable) in langContainer {
            guard let tableDict = rawTable as? [String: Any] else { continue }
            var strings: [String: String] = [:]
            for (entryKey, entryVal) in tableDict {
                if let str = entryVal as? String {
                    strings[entryKey] = str
                }
            }
            if !strings.isEmpty {
                result[langCode] = strings
            }
        }
        return result
    }
}

// MARK: - Computed Exact v1.6.9 Palette Snapshot (`Sendable`)

public struct FabushiThemePalette: Sendable, Equatable {
    public let preset: FabushiThemePreset
    public let highContrast: Bool

    public let windowBackgroundRGB: UInt32
    public let navRailBackgroundRGB: UInt32
    public let surfaceSecondaryRGB: UInt32
    public let rowHoverRGB: UInt32
    public let sidebarActiveSelectionRGB: UInt32
    public let surfacePrimaryRGB: UInt32
    public let surfaceTopBarRGB: UInt32
    public let surfaceComposerRGB: UInt32
    public let surfaceInputRGB: UInt32
    public let surfaceElevatedRGB: UInt32
    public let surfaceModalRGB: UInt32
    public let surfaceMenuRGB: UInt32
    public let tooltipBackgroundRGB: UInt32
    public let tooltipForegroundRGB: UInt32

    public let bubbleIncomingRGB: UInt32
    public let bubbleOutgoingBaseRGB: UInt32
    public let bubbleOutgoingTopRGB: UInt32
    public let bubbleOutgoingBottomRGB: UInt32
    public let bubbleOutgoingTextRGB: UInt32
    public let bubbleOutgoingSecondaryTextRGB: UInt32

    public let textPrimaryRGB: UInt32
    public let textSecondaryRGB: UInt32
    public let textMutedRGB: UInt32
    public let textLinkRGB: UInt32
    public let borderSolidRGB: UInt32

    public let accentContrast: FabushiAccentContrastResult
    public let accentHoverRGB: UInt32
    public let destructiveRGB: UInt32
    public let successRGB: UInt32
    public let warningRGB: UInt32
    public let onlineIndicatorRGB: UInt32
    public let servicePillBackgroundRGB: UInt32
    public let servicePillForegroundRGB: UInt32

    public let separatorAlpha: Double
    public let hairlineBorderAlpha: Double
    public let specularHighlightAlpha: Double

    public static func resolve(
        preset: FabushiThemePreset,
        customAccentRGB: UInt32? = nil,
        highContrast: Bool = false
    ) -> FabushiThemePalette {
        let rawAccent: UInt32
        if let customAccentRGB {
            rawAccent = customAccentRGB & 0x00FF_FFFF
        } else {
            rawAccent = preset.defaultAccentRGB
        }
        let contrast = FabushiColorHex.applyWCAG138Guard(rgb: rawAccent, highContrast: highContrast)
        let isDefaultFamily = defaultAccentFamily(for: preset).contains(rawAccent)

        // Exact upstream `palette_presets.rs` base tokens
        let base: BaseSpec = BaseSpec.forPreset(preset)

        let dialogsActiveRGB: UInt32
        let msgOutBaseRGB: UInt32
        let msgOutTopRGB: UInt32
        let msgOutBottomRGB: UInt32
        let msgOutFgRGB: UInt32
        let msgOutSecondaryFgRGB: UInt32
        let textLinkRGB: UInt32
        let accentHoverRGB: UInt32

        if preset.isAppleStyle && !isDefaultFamily {
            let customFill = contrast.effectiveRGB
            dialogsActiveRGB = customFill
            msgOutBaseRGB = customFill
            msgOutTopRGB = FabushiColorHex.mixRGB(customFill, 0xFFFFFF, t: preset.isDark ? 0.10 : 0.16)
            msgOutBottomRGB = customFill
            msgOutFgRGB = contrast.foregroundRGB
            msgOutSecondaryFgRGB = FabushiColorHex.mixRGB(contrast.foregroundRGB, customFill, t: 0.16)
            textLinkRGB = preset.isDark
                ? FabushiColorHex.mixRGB(0xFFFFFF, rawAccent, t: 0.72)
                : FabushiColorHex.adjustRGB(rawAccent, delta: -22)
            accentHoverRGB = preset.isDark
                ? FabushiColorHex.adjustRGB(rawAccent, delta: 16)
                : FabushiColorHex.adjustRGB(rawAccent, delta: -18)
        } else if !preset.isAppleStyle && !isDefaultFamily {
            dialogsActiveRGB = rawAccent
            msgOutBaseRGB = base.msgOutBase
            msgOutTopRGB = base.msgOutTop
            msgOutBottomRGB = base.msgOutBottom
            msgOutFgRGB = base.msgOutFg
            msgOutSecondaryFgRGB = base.msgOutSecondaryFg
            textLinkRGB = base.textLink
            accentHoverRGB = FabushiColorHex.adjustRGB(rawAccent, delta: -18)
        } else {
            dialogsActiveRGB = base.dialogsActive
            msgOutBaseRGB = base.msgOutBase
            msgOutTopRGB = base.msgOutTop
            msgOutBottomRGB = base.msgOutBottom
            msgOutFgRGB = highContrast ? contrast.foregroundRGB : base.msgOutFg
            msgOutSecondaryFgRGB = base.msgOutSecondaryFg
            textLinkRGB = base.textLink
            accentHoverRGB = base.accentHover
        }

        let resolvedTextPrimary: UInt32
        let resolvedTextSecondary: UInt32
        let resolvedSeparatorAlpha: Double
        let resolvedHairlineAlpha: Double

        if highContrast {
            resolvedTextPrimary = preset.isDark ? 0xFFFFFF : 0x000000
            resolvedTextSecondary = preset.isDark ? 0xD8D8DF : 0x2C2C30
            resolvedSeparatorAlpha = preset.isDark ? 0.28 : 0.26
            resolvedHairlineAlpha = preset.isDark ? 0.34 : 0.32
        } else {
            resolvedTextPrimary = base.textPrimary
            resolvedTextSecondary = base.textSecondary
            resolvedSeparatorAlpha = preset.isDark ? 0.10 : 0.08
            resolvedHairlineAlpha = preset.isDark ? 0.10 : 0.12
        }

        return FabushiThemePalette(
            preset: preset,
            highContrast: highContrast,
            windowBackgroundRGB: base.windowBg,
            navRailBackgroundRGB: base.railBg,
            surfaceSecondaryRGB: base.dialogsBg,
            rowHoverRGB: base.dialogsHoverBg,
            sidebarActiveSelectionRGB: dialogsActiveRGB,
            surfacePrimaryRGB: base.stageBg,
            surfaceTopBarRGB: base.topbarBg,
            surfaceComposerRGB: base.composerBg,
            surfaceInputRGB: base.inputBg,
            surfaceElevatedRGB: base.cardBg,
            surfaceModalRGB: base.modalBg,
            surfaceMenuRGB: base.menuBg,
            tooltipBackgroundRGB: base.tooltipBg,
            tooltipForegroundRGB: base.tooltipFg,
            bubbleIncomingRGB: base.msgInBg,
            bubbleOutgoingBaseRGB: msgOutBaseRGB,
            bubbleOutgoingTopRGB: msgOutTopRGB,
            bubbleOutgoingBottomRGB: msgOutBottomRGB,
            bubbleOutgoingTextRGB: msgOutFgRGB,
            bubbleOutgoingSecondaryTextRGB: msgOutSecondaryFgRGB,
            textPrimaryRGB: resolvedTextPrimary,
            textSecondaryRGB: resolvedTextSecondary,
            textMutedRGB: base.msgInDateFg,
            textLinkRGB: textLinkRGB,
            borderSolidRGB: base.borderFg,
            accentContrast: contrast,
            accentHoverRGB: accentHoverRGB,
            destructiveRGB: base.danger,
            successRGB: base.success,
            warningRGB: base.warning,
            onlineIndicatorRGB: base.onlineDotFg,
            servicePillBackgroundRGB: base.servicePillBg,
            servicePillForegroundRGB: base.servicePillFg,
            separatorAlpha: resolvedSeparatorAlpha,
            hairlineBorderAlpha: resolvedHairlineAlpha,
            specularHighlightAlpha: preset.isDark ? 0.12 : 0.72
        )
    }

    private static func defaultAccentFamily(for preset: FabushiThemePreset) -> Set<UInt32> {
        switch preset {
        case .night:
            return [0x0A84FF, 0x007AFF, 0x116DD2, 0x1680F3, 0x1084FE, 0x1687FF, 0x2AABEE, 0x40A7E3, 0x5288C1, 0x3390EC]
        case .tinted:
            return [0x0A84FF, 0x007AFF, 0x116DD2, 0x1687FF, 0x1084FE, 0x1ABC9C, 0x3FC1B0, 0x009687]
        case .day:
            return [0x007AFF, 0x0A84FF, 0x1680F3, 0x126FD4, 0x1084FE, 0x1687FF, 0x2AABEE, 0x40A7E3, 0x419FD9, 0x2481CC, 0x5288C1]
        case .classic:
            return [0x2AABEE, 0x40A7E3, 0x419FD9]
        }
    }

    private struct BaseSpec {
        let windowBg: UInt32
        let railBg: UInt32
        let dialogsBg: UInt32
        let dialogsHoverBg: UInt32
        let dialogsActive: UInt32
        let stageBg: UInt32
        let topbarBg: UInt32
        let composerBg: UInt32
        let inputBg: UInt32
        let cardBg: UInt32
        let modalBg: UInt32
        let menuBg: UInt32
        let tooltipBg: UInt32
        let tooltipFg: UInt32
        let msgInBg: UInt32
        let msgOutBase: UInt32
        let msgOutTop: UInt32
        let msgOutBottom: UInt32
        let msgOutFg: UInt32
        let msgOutSecondaryFg: UInt32
        let msgInDateFg: UInt32
        let servicePillBg: UInt32
        let servicePillFg: UInt32
        let onlineDotFg: UInt32
        let borderFg: UInt32
        let textPrimary: UInt32
        let textSecondary: UInt32
        let textLink: UInt32
        let accentHover: UInt32
        let danger: UInt32
        let success: UInt32
        let warning: UInt32

        static func forPreset(_ preset: FabushiThemePreset) -> BaseSpec {
            switch preset {
            case .night:
                return BaseSpec(
                    windowBg: 0x121212, railBg: 0x0E0E0E, dialogsBg: 0x0E0E0E,
                    dialogsHoverBg: 0x2C2C2C, dialogsActive: 0x0A84FF, stageBg: 0x161616,
                    topbarBg: 0x121212, composerBg: 0x161616, inputBg: 0x1F1F1F,
                    cardBg: 0x1F1F1F, modalBg: 0x1F1F1F, menuBg: 0x1F1F1F,
                    tooltipBg: 0x1F1F1F, tooltipFg: 0xF5F5F5, msgInBg: 0x1F1F1F,
                    msgOutBase: 0x116DD2, msgOutTop: 0x1884FC, msgOutBottom: 0x116DD2,
                    msgOutFg: 0xFFFFFF, msgOutSecondaryFg: 0xD6EBFF, msgInDateFg: 0x8A8A8E,
                    servicePillBg: 0x2C2C2C, servicePillFg: 0x9E9EA4, onlineDotFg: 0x00BA69,
                    borderFg: 0x2C2C2C, textPrimary: 0xF5F5F5, textSecondary: 0xA3A3A8,
                    textLink: 0x4D9BFF, accentHover: 0x1884FC, danger: 0xFF6463,
                    success: 0x00BA69, warning: 0xE28600
                )
            case .tinted:
                return BaseSpec(
                    windowBg: 0x181C22, railBg: 0x181C22, dialogsBg: 0x181C22,
                    dialogsHoverBg: 0x242A33, dialogsActive: 0x0A84FF, stageBg: 0x181C22,
                    topbarBg: 0x181C22, composerBg: 0x181C22, inputBg: 0x242A33,
                    cardBg: 0x222831, modalBg: 0x1E232B, menuBg: 0x212730,
                    tooltipBg: 0x262D37, tooltipFg: 0xF5F7FA, msgInBg: 0x242A33,
                    msgOutBase: 0x0A84FF, msgOutTop: 0x1E8FFF, msgOutBottom: 0x0A84FF,
                    msgOutFg: 0xFFFFFF, msgOutSecondaryFg: 0xD6EBFF, msgInDateFg: 0x8E96A0,
                    servicePillBg: 0x242A33, servicePillFg: 0x8E96A0, onlineDotFg: 0x30D158,
                    borderFg: 0x303844, textPrimary: 0xF5F7FA, textSecondary: 0x949DA8,
                    textLink: 0x409CFF, accentHover: 0x2994FF, danger: 0xFF453A,
                    success: 0x30D158, warning: 0xE28600
                )
            case .day:
                return BaseSpec(
                    windowBg: 0xFCFCFC, railBg: 0xF5F5F5, dialogsBg: 0xF5F5F5,
                    dialogsHoverBg: 0xF1F1F1, dialogsActive: 0x1680F3, stageBg: 0xFFFFFF,
                    topbarBg: 0xFCFCFC, composerBg: 0xFFFFFF, inputBg: 0xF1F1F1,
                    cardBg: 0xFFFFFF, modalBg: 0xFFFFFF, menuBg: 0xFFFFFF,
                    tooltipBg: 0x1C1C1C, tooltipFg: 0xFFFFFF, msgInBg: 0xF1F1F1,
                    msgOutBase: 0x126FD4, msgOutTop: 0x1E86FC, msgOutBottom: 0x126FD4,
                    msgOutFg: 0xFFFFFF, msgOutSecondaryFg: 0xE5F2FF, msgInDateFg: 0x7A7A80,
                    servicePillBg: 0xF1F1F1, servicePillFg: 0x7A7A80, onlineDotFg: 0x10AB61,
                    borderFg: 0xE5E5E7, textPrimary: 0x1C1C1C, textSecondary: 0x636366,
                    textLink: 0x0B6DD3, accentHover: 0x126FD4, danger: 0xE72E3B,
                    success: 0x10AB61, warning: 0xAA6300
                )
            case .classic:
                return BaseSpec(
                    windowBg: 0xFFFFFF, railBg: 0xEDEDED, dialogsBg: 0xFFFFFF,
                    dialogsHoverBg: 0xF1F1F1, dialogsActive: 0x419FD9, stageBg: 0xD5E8C8,
                    topbarBg: 0xFFFFFF, composerBg: 0xFFFFFF, inputBg: 0xF1F1F1,
                    cardBg: 0xF6F8F5, modalBg: 0xFFFFFF, menuBg: 0xFFFFFF,
                    tooltipBg: 0xEEF2F5, tooltipFg: 0x5D6C80, msgInBg: 0xFFFFFF,
                    msgOutBase: 0xEFFDDE, msgOutTop: 0xEFFDDE, msgOutBottom: 0xEFFDDE,
                    msgOutFg: 0x000000, msgOutSecondaryFg: 0x6FAB69, msgInDateFg: 0xA0ACB6,
                    servicePillBg: 0x527C41, servicePillFg: 0xFFFFFF, onlineDotFg: 0x4DC920,
                    borderFg: 0xE4E8E2, textPrimary: 0x000000, textSecondary: 0x707579,
                    textLink: 0x168ACD, accentHover: 0x2E95D1, danger: 0xE53935,
                    success: 0x4B9432, warning: 0xE5A030
                )
            }
        }
    }
}

// MARK: - Canonical Reactive Theme Engine (`PROJECT.md § Interface Contracts #3`)

@MainActor
@Observable
public final class FabushiTheme {
    private static var _defaultInstance: FabushiTheme?

    public static var `default`: FabushiTheme {
        if let existing = _defaultInstance {
            return existing
        }
        let instance = FabushiTheme()
        _defaultInstance = instance
        return instance
    }

    nonisolated public static let minBubbleCornerRadius: CGFloat = 6
    nonisolated public static let maxBubbleCornerRadius: CGFloat = 18
    nonisolated public static let defaultBubbleCornerRadius: CGFloat = 17

    public enum PreferenceKey: Sendable {
        public static let themeMode = "ui.theme_mode"
        public static let accentHex = "ui.accent_hex"
        public static let bubbleRadiusPx = "ui.bubble_radius_px"
        public static let wallpaperId = "ui.wallpaper_id"
        public static let locale = "ui.locale"
        public static let reduceMotion = "ui.reduce_motion"
        public static let highContrast = "ui.high_contrast"
    }

    public private(set) var preset: FabushiThemePreset
    public private(set) var customAccentHex: String?
    public private(set) var bubbleCornerRadius: CGFloat
    public private(set) var wallpaper: FabushiWallpaper
    public private(set) var locale: FabushiLocale
    public private(set) var reduceMotionOverride: Bool
    public private(set) var highContrastOverride: Bool
    public private(set) var lastAccentValidationError: String?

    @ObservationIgnored private var hasLaunchThemeOverride = false
    @ObservationIgnored private var hasLaunchAccentOverride = false
    @ObservationIgnored private var hasLaunchLocaleOverride = false
    @ObservationIgnored private var hasLaunchReduceMotionOverride = false
    @ObservationIgnored private var hasLaunchHighContrastOverride = false

    public init(
        preset: FabushiThemePreset = .night,
        customAccentHex: String? = nil,
        bubbleCornerRadius: CGFloat = FabushiTheme.defaultBubbleCornerRadius,
        wallpaper: FabushiWallpaper? = nil,
        locale: FabushiLocale = FabushiLocale.systemDefault(),
        reduceMotionOverride: Bool = false,
        highContrastOverride: Bool = false
    ) {
        self.preset = preset
        if let customAccentHex, let rgb = FabushiColorHex.parse(customAccentHex, allowShortRGB: false) {
            self.customAccentHex = FabushiColorHex.format(rgb)
        } else {
            self.customAccentHex = nil
        }
        self.bubbleCornerRadius = Self.clampBubbleRadius(bubbleCornerRadius)
        self.wallpaper = wallpaper ?? FabushiWallpaper.defaultFor(preset)
        self.locale = locale
        self.reduceMotionOverride = reduceMotionOverride
        self.highContrastOverride = highContrastOverride
        self.lastAccentValidationError = nil
    }

    // MARK: - Computed Palette & Semantic Tokens

    public var palette: FabushiThemePalette {
        let customRGB = customAccentHex.flatMap { FabushiColorHex.parse($0, allowShortRGB: false) }
        return FabushiThemePalette.resolve(
            preset: preset,
            customAccentRGB: customRGB,
            highContrast: highContrastOverride
        )
    }

    public var colorScheme: ColorScheme {
        preset.isDark ? .dark : .light
    }

    public var surfacePrimary: Color { Color(fabushiHex: palette.surfacePrimaryRGB) }
    public var surfaceSecondary: Color { Color(fabushiHex: palette.surfaceSecondaryRGB) }
    public var surfaceElevated: Color { Color(fabushiHex: palette.surfaceElevatedRGB) }
    public var surfaceTopBar: Color { Color(fabushiHex: palette.surfaceTopBarRGB) }
    public var surfaceChrome: Color { Color(fabushiHex: palette.windowBackgroundRGB) }
    public var surfaceComposer: Color { Color(fabushiHex: palette.surfaceComposerRGB) }
    public var surfaceInput: Color { Color(fabushiHex: palette.surfaceInputRGB) }
    public var surfaceModal: Color { Color(fabushiHex: palette.surfaceModalRGB) }
    public var surfaceMenu: Color { Color(fabushiHex: palette.surfaceMenuRGB) }

    public var bubbleIncoming: Color { Color(fabushiHex: palette.bubbleIncomingRGB) }
    public var bubbleOutgoingTopHex: UInt32 { palette.bubbleOutgoingTopRGB }
    public var bubbleOutgoingBottomHex: UInt32 { palette.bubbleOutgoingBottomRGB }
    public var bubbleOutgoingGradient: LinearGradient {
        LinearGradient(
            colors: [
                Color(fabushiHex: palette.bubbleOutgoingTopRGB),
                Color(fabushiHex: palette.bubbleOutgoingBottomRGB),
            ],
            startPoint: .top,
            endPoint: .bottom
        )
    }
    public var bubbleOutgoingText: Color { Color(fabushiHex: palette.bubbleOutgoingTextRGB) }

    public var separatorHairline: Color {
        Color(fabushiHex: preset.isDark ? 0xFFFFFF : 0x1C1C1C, alpha: palette.separatorAlpha)
    }

    public var hairlineBorder: Color {
        Color(fabushiHex: preset.isDark ? 0xFFFFFF : 0x1C1C1C, alpha: palette.hairlineBorderAlpha)
    }

    public var specularHighlight: Color {
        Color(fabushiHex: 0xFFFFFF, alpha: palette.specularHighlightAlpha)
    }

    public var accentContrastResult: FabushiAccentContrastResult { palette.accentContrast }
    public var rawAccentHex: String { palette.accentContrast.rawHex }
    public var effectiveAccentHex: String { palette.accentContrast.effectiveHex }
    public var accentColor: Color { Color(fabushiHex: palette.accentContrast.effectiveRGB) }
    public var accentForegroundColor: Color { Color(fabushiHex: palette.accentContrast.foregroundRGB) }

    public var textPrimary: Color { Color(fabushiHex: palette.textPrimaryRGB) }
    public var textSecondary: Color { Color(fabushiHex: palette.textSecondaryRGB) }
    public var textMuted: Color { Color(fabushiHex: palette.textMutedRGB) }
    public var textLink: Color { Color(fabushiHex: palette.textLinkRGB) }

    public var destructiveColor: Color { Color(fabushiHex: palette.destructiveRGB) }
    public var successColor: Color { Color(fabushiHex: palette.successRGB) }
    public var warningColor: Color { Color(fabushiHex: palette.warningRGB) }
    public var tooltipBackground: Color { Color(fabushiHex: palette.tooltipBackgroundRGB) }
    public var tooltipForeground: Color { Color(fabushiHex: palette.tooltipForegroundRGB) }

    public func effectiveReduceMotion(systemReduceMotion: Bool) -> Bool {
        reduceMotionOverride || systemReduceMotion
    }

    public func effectiveHighContrast(systemContrast: ColorSchemeContrast) -> Bool {
        highContrastOverride || systemContrast == .increased
    }

    public func localized(_ key: String, bundle: Bundle = .main) -> String {
        locale.localized(key, bundle: bundle)
    }

    // MARK: - Mutations

    public func applyLaunchConfiguration(_ config: FabushiLaunchConfiguration) {
        if let launchPreset = config.themePreset {
            preset = launchPreset
            wallpaper = FabushiWallpaper.defaultFor(launchPreset)
            hasLaunchThemeOverride = true
        }
        if let launchAccent = config.accentHex {
            customAccentHex = launchAccent
            lastAccentValidationError = nil
            hasLaunchAccentOverride = true
        } else if let invalid = config.invalidLaunchAccentHex {
            lastAccentValidationError = invalid
        }
        if let launchLocale = config.preferredLocale {
            locale = launchLocale
            hasLaunchLocaleOverride = true
        }
        if config.reduceMotion {
            reduceMotionOverride = true
            hasLaunchReduceMotionOverride = true
        }
        if config.highContrast {
            highContrastOverride = true
            hasLaunchHighContrastOverride = true
        }
    }

    public func selectPreset(_ newPreset: FabushiThemePreset) {
        let wasUsingDefaultWallpaper = wallpaper.id == preset.defaultWallpaperID
        preset = newPreset
        if wasUsingDefaultWallpaper {
            wallpaper = FabushiWallpaper.defaultFor(newPreset)
        }
    }

    public func cyclePreset() {
        selectPreset(preset.nextInCycle)
    }

    /// Strict fail-closed `#RRGGBB` accent hex setter. Rejects invalid hex strings without
    /// mutating the currently active valid accent.
    @discardableResult
    public func applyCustomAccentHex(_ raw: String) -> Bool {
        guard let rgb = FabushiColorHex.parse(raw, allowShortRGB: false) else {
            lastAccentValidationError = localized("hig.appearance.invalidHexError")
            return false
        }
        customAccentHex = FabushiColorHex.format(rgb)
        lastAccentValidationError = nil
        return true
    }

    public func clearCustomAccent() {
        customAccentHex = nil
        lastAccentValidationError = nil
    }

    public func setBubbleCornerRadius(_ radius: CGFloat) {
        bubbleCornerRadius = Self.clampBubbleRadius(radius)
    }

    public func selectWallpaper(_ newWallpaper: FabushiWallpaper) {
        wallpaper = newWallpaper
    }

    public func setLocale(_ newLocale: FabushiLocale) {
        locale = newLocale
    }

    public func setReduceMotion(_ enabled: Bool) {
        reduceMotionOverride = enabled
    }

    public func setHighContrast(_ enabled: Bool) {
        highContrastOverride = enabled
    }

    // MARK: - Real Rust Host Persistence (`settings.localAll` / `settings.localSet`)

    private struct LocalSettingSetParams: Encodable, Sendable {
        let key: String
        let value: FabushiJSONValue
    }

    private struct LocalSettingSetResult: Decodable, Sendable {
        let key: String
        let saved: Bool
    }

    /// Loads persisted UI preferences from real `settings.localAll` SQLite `host_kv` storage,
    /// preserving any explicit launch-environment overrides passed on app start.
    public func loadPersistedPreferences(using coordinator: FabushiCoordinator) async {
        guard coordinator.isStarted else { return }
        do {
            let stored: [String: FabushiJSONValue] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("localAll"),
                params: FabushiEmptyParams()
            )
            applyPersistedValues(stored)
        } catch {
            // Non-fatal: keep active defaults/launch overrides if host storage has no saved preferences yet.
        }
    }

    /// Persists all current UI preferences to real `settings.localSet` SQLite `host_kv` storage.
    @discardableResult
    public func persistPreferences(using coordinator: FabushiCoordinator) async -> Bool {
        guard coordinator.isStarted else { return false }
        let entries: [(String, FabushiJSONValue)] = [
            (PreferenceKey.themeMode, .string(preset.rawValue)),
            (PreferenceKey.accentHex, customAccentHex.map { .string($0) } ?? .null),
            (PreferenceKey.bubbleRadiusPx, .int(Int64(bubbleCornerRadius.rounded()))),
            (PreferenceKey.wallpaperId, .string(wallpaper.id)),
            (PreferenceKey.locale, .string(locale.rawValue)),
            (PreferenceKey.reduceMotion, .bool(reduceMotionOverride)),
            (PreferenceKey.highContrast, .bool(highContrastOverride)),
        ]

        var allSaved = true
        for (key, value) in entries {
            do {
                let res: LocalSettingSetResult = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.settings.method("localSet"),
                    params: LocalSettingSetParams(key: key, value: value)
                )
                if !res.saved {
                    allSaved = false
                }
            } catch {
                allSaved = false
            }
        }
        return allSaved
    }

    private func applyPersistedValues(_ stored: [String: FabushiJSONValue]) {
        if !hasLaunchThemeOverride, let rawTheme = stored[PreferenceKey.themeMode]?.stringValue {
            selectPreset(FabushiThemePreset.fromName(rawTheme))
        }
        if !hasLaunchAccentOverride, let accentVal = stored[PreferenceKey.accentHex] {
            switch accentVal {
            case .null:
                customAccentHex = nil
            case .string(let raw) where !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty:
                if let rgb = FabushiColorHex.parse(raw, allowShortRGB: false) {
                    customAccentHex = FabushiColorHex.format(rgb)
                }
            default:
                break
            }
        }
        if let radius = stored[PreferenceKey.bubbleRadiusPx]?.doubleValue {
            setBubbleCornerRadius(CGFloat(radius))
        }
        if !hasLaunchThemeOverride,
           let wallpaperID = stored[PreferenceKey.wallpaperId]?.stringValue,
           let matched = FabushiWallpaper.preset(id: wallpaperID) {
            wallpaper = matched
        }
        if !hasLaunchLocaleOverride, let rawLocale = stored[PreferenceKey.locale]?.stringValue {
            locale = FabushiLocale(normalizing: rawLocale)
        }
        if !hasLaunchReduceMotionOverride, let rm = stored[PreferenceKey.reduceMotion]?.boolValue {
            reduceMotionOverride = rm
        }
        if !hasLaunchHighContrastOverride, let hc = stored[PreferenceKey.highContrast]?.boolValue {
            highContrastOverride = hc
        }
    }

    private static func clampBubbleRadius(_ value: CGFloat) -> CGFloat {
        min(max(value, minBubbleCornerRadius), maxBubbleCornerRadius)
    }
}
