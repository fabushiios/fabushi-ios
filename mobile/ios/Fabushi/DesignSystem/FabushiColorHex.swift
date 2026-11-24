import Foundation
import SwiftUI

/// Output of the upstream `bhrumom/gpui-telegram` (`v1.6.9`) WCAG `>138` Rec. 601 luminance
/// contrast guard paired with WCAG 2.1 relative-luminance foreground ink selection.
public struct FabushiAccentContrastResult: Sendable, Equatable {
    public let rawHex: String
    public let effectiveHex: String
    public let rawRGB: UInt32
    public let effectiveRGB: UInt32
    public let rec601Luminance: Double
    public let wasDarkened: Bool
    public let darkenFactor: Double
    public let foregroundHex: String
    public let foregroundRGB: UInt32

    public init(
        rawHex: String,
        effectiveHex: String,
        rawRGB: UInt32,
        effectiveRGB: UInt32,
        rec601Luminance: Double,
        wasDarkened: Bool,
        darkenFactor: Double,
        foregroundHex: String,
        foregroundRGB: UInt32
    ) {
        self.rawHex = rawHex
        self.effectiveHex = effectiveHex
        self.rawRGB = rawRGB
        self.effectiveRGB = effectiveRGB
        self.rec601Luminance = rec601Luminance
        self.wasDarkened = wasDarkened
        self.darkenFactor = darkenFactor
        self.foregroundHex = foregroundHex
        self.foregroundRGB = foregroundRGB
    }
}

/// Pure, `Sendable` hex color parser, mixer, and WCAG contrast engine matching
/// `bhrumom/gpui-telegram@57a62ef92be2` (`palette_schemes.rs`, `palette_presets.rs`, `settings_chat.rs`).
public enum FabushiColorHex: Sendable {
    public static let wcag138Threshold: Double = 138.0
    public static let wcag138SlopeDenominator: Double = 145.0
    public static let wcag138MaxDarkenFactor: Double = 0.36
    public static let wcag138DarkenTargetRGB: UInt32 = 0x111111
    public static let wcagInkLuminanceMidpoint: Double = 0.179

    /// Parses an ASCII hex color string into a 24-bit RGB value (`0x000000...0xFFFFFF`).
    ///
    /// - Parameters:
    ///   - raw: Raw input string (whitespace trimmed, optional single leading `#`).
    ///   - allowShortRGB: Defaults to `false` (strict `#RRGGBB` fail-closed mode matching
    ///     upstream and E2E `T2_F31_04`). When `true`, expands 3-digit `#RGB` (`#F0A` -> `#FF00AA`).
    public static func parse(_ raw: String, allowShortRGB: Bool = false) -> UInt32? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        let stripped: Substring
        if trimmed.hasPrefix("#") {
            stripped = trimmed.dropFirst()
        } else {
            stripped = Substring(trimmed)
        }

        guard !stripped.isEmpty else { return nil }
        for scalar in stripped.unicodeScalars {
            guard isASCIIHexDigit(scalar) else { return nil }
        }

        if stripped.count == 6 {
            return UInt32(stripped, radix: 16).map { $0 & 0x00FF_FFFF }
        }

        if allowShortRGB, stripped.count == 3 {
            let chars = Array(stripped)
            let expanded = String([chars[0], chars[0], chars[1], chars[1], chars[2], chars[2]])
            return UInt32(expanded, radix: 16).map { $0 & 0x00FF_FFFF }
        }

        return nil
    }

    /// Formats a 24-bit RGB value into canonical uppercase `#RRGGBB`.
    public static func format(_ rgb: UInt32) -> String {
        let masked = rgb & 0x00FF_FFFF
        let r = (masked >> 16) & 0xFF
        let g = (masked >> 8) & 0xFF
        let b = masked & 0xFF
        return String(format: "#%02X%02X%02X", r, g, b)
    }

    /// Extracts 8-bit red, green, blue channels from a 24-bit RGB value.
    public static func components(_ rgb: UInt32) -> (r: UInt8, g: UInt8, b: UInt8) {
        let r = UInt8((rgb >> 16) & 0xFF)
        let g = UInt8((rgb >> 8) & 0xFF)
        let b = UInt8(rgb & 0xFF)
        return (r, g, b)
    }

    /// Exact upstream `mix_rgb(a, b, t)` per-channel linear blend with `.rounded()` clamping.
    public static func mixRGB(_ a: UInt32, _ b: UInt32, t: Double) -> UInt32 {
        let clampedT = min(max(t, 0.0), 1.0)
        func mixChannel(_ shift: UInt32) -> UInt32 {
            let ca = Double((a >> shift) & 0xFF)
            let cb = Double((b >> shift) & 0xFF)
            let blended = (ca + (cb - ca) * clampedT).rounded()
            return UInt32(min(max(Int(blended), 0), 255))
        }
        return (mixChannel(16) << 16) | (mixChannel(8) << 8) | mixChannel(0)
    }

    /// Exact upstream `adjust_rgb(hex, delta)` channel shift with `0...255` clamping.
    public static func adjustRGB(_ rgb: UInt32, delta: Int) -> UInt32 {
        func shiftChannel(_ shift: UInt32) -> UInt32 {
            let value = Int((rgb >> shift) & 0xFF) + delta
            return UInt32(min(max(value, 0), 255))
        }
        return (shiftChannel(16) << 16) | (shiftChannel(8) << 8) | shiftChannel(0)
    }

    /// Rec. 601 perceived luminance (`0...255`) used by upstream `ensure_white_text_contrast`.
    public static func rec601Luminance(_ rgb: UInt32) -> Double {
        let (r, g, b) = components(rgb)
        return 0.299 * Double(r) + 0.587 * Double(g) + 0.114 * Double(b)
    }

    /// WCAG 2.1 sRGB linearized relative luminance (`0.0...1.0`), matching upstream `wallpaper_label_ink`.
    public static func wcagRelativeLuminance(_ rgb: UInt32) -> Double {
        func linearize(_ channel: UInt8) -> Double {
            let s = Double(channel) / 255.0
            if s <= 0.03928 {
                return s / 12.92
            }
            return pow((s + 0.055) / 1.055, 2.4)
        }
        let (r, g, b) = components(rgb)
        return 0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)
    }

    /// WCAG 2.1 contrast ratio (`1.0...21.0`) between foreground and background colors.
    public static func contrastRatio(foreground: UInt32, background: UInt32) -> Double {
        let l1 = wcagRelativeLuminance(foreground)
        let l2 = wcagRelativeLuminance(background)
        let lighter = max(l1, l2)
        let darker = min(l1, l2)
        return (lighter + 0.05) / (darker + 0.05)
    }

    /// Upstream `wallpaper_label_ink` geometric-midpoint ink selector (`#18222D` vs `#FFFFFF`).
    public static func preferredInkHex(onBackground rgb: UInt32) -> UInt32 {
        wcagRelativeLuminance(rgb) > wcagInkLuminanceMidpoint ? 0x18222D : 0xFFFFFF
    }

    /// Applies the exact upstream `v1.6.9` WCAG `>138` luminance darkening guard and computes
    /// a legible foreground ink color.
    ///
    /// Golden vectors:
    /// - `#007AFF` -> `#007AFF` (`wasDarkened == false`)
    /// - `#0A84FF` -> `#0A84FF` (`wasDarkened == false`)
    /// - `#FFFF00` -> `#A9A906` (`wasDarkened == true`)
    /// - `#FFFFFF` -> `#A9A9A9` (`wasDarkened == true`)
    public static func applyWCAG138Guard(
        rgb: UInt32,
        highContrast: Bool = false
    ) -> FabushiAccentContrastResult {
        let masked = rgb & 0x00FF_FFFF
        let luma = rec601Luminance(masked)
        let effectiveRGB: UInt32
        let wasDarkened: Bool
        let darkenFactor: Double

        if luma <= wcag138Threshold {
            effectiveRGB = masked
            wasDarkened = false
            darkenFactor = 0.0
        } else {
            let t = min(
                max((luma - wcag138Threshold) / wcag138SlopeDenominator, 0.0),
                wcag138MaxDarkenFactor
            )
            effectiveRGB = mixRGB(masked, wcag138DarkenTargetRGB, t: t)
            wasDarkened = true
            darkenFactor = t
        }

        let effectiveRelLum = wcagRelativeLuminance(effectiveRGB)
        let foregroundRGB: UInt32
        if highContrast {
            // Geometric midpoint `0.179` with `#000000` vs `#FFFFFF` guarantees >= 4.58:1 WCAG AA
            // for every possible 24-bit RGB color.
            foregroundRGB = effectiveRelLum > wcagInkLuminanceMidpoint ? 0x000000 : 0xFFFFFF
        } else if wasDarkened {
            foregroundRGB = preferredInkHex(onBackground: effectiveRGB)
        } else if effectiveRelLum > 0.25 {
            foregroundRGB = preferredInkHex(onBackground: effectiveRGB)
        } else {
            foregroundRGB = 0xFFFFFF
        }

        return FabushiAccentContrastResult(
            rawHex: format(masked),
            effectiveHex: format(effectiveRGB),
            rawRGB: masked,
            effectiveRGB: effectiveRGB,
            rec601Luminance: luma,
            wasDarkened: wasDarkened,
            darkenFactor: darkenFactor,
            foregroundHex: format(foregroundRGB),
            foregroundRGB: foregroundRGB
        )
    }

    private static func isASCIIHexDigit(_ scalar: UnicodeScalar) -> Bool {
        switch scalar.value {
        case 0x30...0x39, 0x41...0x46, 0x61...0x66:
            return true
        default:
            return false
        }
    }
}

public extension Color {
    /// Initializes a `Color` from a 24-bit RGB hex value without colliding with legacy
    /// `private extension Color { init(hex:) }` overloads in existing files.
    init(fabushiHex rgb: UInt32, alpha: Double = 1.0) {
        let (r, g, b) = FabushiColorHex.components(rgb)
        self.init(
            .sRGB,
            red: Double(r) / 255.0,
            green: Double(g) / 255.0,
            blue: Double(b) / 255.0,
            opacity: alpha
        )
    }

    /// Initializes a `Color` from a hex string (`#RRGGBB` by default, optional `#RGB`).
    init?(fabushiHexString: String, allowShortRGB: Bool = false, alpha: Double = 1.0) {
        guard let parsed = FabushiColorHex.parse(fabushiHexString, allowShortRGB: allowShortRGB) else {
            return nil
        }
        self.init(fabushiHex: parsed, alpha: alpha)
    }
}
