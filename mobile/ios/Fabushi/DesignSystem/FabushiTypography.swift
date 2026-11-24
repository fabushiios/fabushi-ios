import SwiftUI

/// Semantic Apple HIG Dynamic Type typography styles.
/// All M2a UI surfaces use system Dynamic Type text styles exclusively (ZERO fixed point sizes)
/// so text scales cleanly up to `accessibilityExtraExtraExtraLarge` (AX5) without clipping.
public enum FabushiTextStyle: Sendable, CaseIterable {
    case largeTitle
    case title2
    case title3
    case headline
    case subheadline
    case body
    case callout
    case footnote
    case caption
    case caption2
    case monospacedCaption

    public var font: Font {
        switch self {
        case .largeTitle:
            return .largeTitle.weight(.bold)
        case .title2:
            return .title2.weight(.semibold)
        case .title3:
            return .title3.weight(.semibold)
        case .headline:
            return .headline
        case .subheadline:
            return .subheadline
        case .body:
            return .body
        case .callout:
            return .callout
        case .footnote:
            return .footnote
        case .caption:
            return .caption
        case .caption2:
            return .caption2
        case .monospacedCaption:
            return .caption.monospaced()
        }
    }
}

/// Canonical Apple HIG + upstream `v1.6.9` adaptive shell geometry constants.
public enum FabushiLayoutMetrics: Sendable {
    public static let minTouchTarget: CGFloat = 44
    public static let sidebarDefaultWidth: CGFloat = 288
    public static let sidebarMinWidth: CGFloat = 240
    public static let sidebarMaxWidth: CGFloat = 360
    public static let sidebarCollapsedWidth: CGFloat = 68
    public static let inspectorMinWidth: CGFloat = 280
    public static let inspectorIdealWidth: CGFloat = 320
    public static let inspectorMaxWidth: CGFloat = 360
    public static let modalCornerRadius: CGFloat = 14
    public static let cardCornerRadius: CGFloat = 12
    public static let pillButtonCornerRadius: CGFloat = 10
    public static let searchCornerRadius: CGFloat = 18
}

public extension View {
    /// Applies a semantic Apple HIG Dynamic Type text style.
    func fabushiFont(_ style: FabushiTextStyle) -> some View {
        self.font(style.font)
    }

    /// Guarantees the Apple HIG minimum `44×44pt` hit target on interactive controls.
    func fabushiMinimumTouchTarget() -> some View {
        self
            .frame(
                minWidth: FabushiLayoutMetrics.minTouchTarget,
                minHeight: FabushiLayoutMetrics.minTouchTarget
            )
            .contentShape(Rectangle())
    }
}
