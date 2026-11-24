import SwiftUI

/// Adaptive Apple Liquid Glass surface modifier (`#available(iOS 26, *)`) that compiles cleanly
/// with zero errors and zero warnings on both Xcode 16.4 (`Swift 6.1.2` / iOS 18.5 SDK) and
/// Xcode 26 (`Swift 6.2` / iOS 26 SDK), while honoring Reduce Transparency and High Contrast.
@MainActor
public struct FabushiLiquidGlassModifier: ViewModifier {
    public let cornerRadius: CGFloat
    public let isInteractive: Bool

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var colorSchemeContrast

    private var theme: FabushiTheme {
        injectedTheme ?? .default
    }

    private var useOpaqueHighContrastSurface: Bool {
        reduceTransparency || theme.effectiveHighContrast(systemContrast: colorSchemeContrast)
    }

    public init(cornerRadius: CGFloat = FabushiLayoutMetrics.modalCornerRadius, isInteractive: Bool = false) {
        self.cornerRadius = cornerRadius
        self.isInteractive = isInteractive
    }

    @ViewBuilder
    public func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)

        if useOpaqueHighContrastSurface {
            content
                .background(theme.surfaceElevated, in: shape)
                .overlay(
                    shape.strokeBorder(theme.hairlineBorder, lineWidth: 1.0)
                )
        } else {
            #if compiler(>=6.2) && os(iOS)
            if #available(iOS 26, *) {
                content
                    .glassEffect(isInteractive ? .regular.interactive() : .regular, in: shape)
            } else {
                fallbackMaterialSurface(content, shape: shape, enhancedSpecular: false)
            }
            #else
            if #available(iOS 26, *) {
                // Built with Xcode 16.4 (iOS 18.5 SDK) running on iOS 26+: adaptive translucent
                // material with specular Liquid Glass rim until built against iOS 26 SDK.
                fallbackMaterialSurface(content, shape: shape, enhancedSpecular: true)
            } else {
                fallbackMaterialSurface(content, shape: shape, enhancedSpecular: false)
            }
            #endif
        }
    }

    @ViewBuilder
    private func fallbackMaterialSurface(
        _ content: Content,
        shape: RoundedRectangle,
        enhancedSpecular: Bool
    ) -> some View {
        content
            .background {
                if theme.preset.isAppleStyle {
                    shape.fill(enhancedSpecular ? .ultraThinMaterial : .regularMaterial)
                } else {
                    shape.fill(theme.surfaceMenu)
                }
            }
            .overlay(
                shape.strokeBorder(
                    enhancedSpecular ? theme.specularHighlight : theme.hairlineBorder,
                    lineWidth: enhancedSpecular ? 0.75 : 0.5
                )
            )
    }
}

public extension View {
    /// Applies an adaptive Liquid Glass surface on iOS 26+ with graceful Apple material fallback
    /// on iOS 17–25 and opaque high-contrast surfaces when Reduce Transparency or High Contrast is on.
    func fabushiLiquidGlass(
        cornerRadius: CGFloat = FabushiLayoutMetrics.modalCornerRadius,
        isInteractive: Bool = false
    ) -> some View {
        modifier(FabushiLiquidGlassModifier(cornerRadius: cornerRadius, isInteractive: isInteractive))
    }
}
