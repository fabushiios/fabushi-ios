import SwiftUI

// MARK: - Reduce-Motion-Safe Pulse Modifier (`D13-03`)

@MainActor
private struct FabushiSkeletonPulseModifier: ViewModifier {
    let baseOpacity: Double

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion
    @State private var isDimmed: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var reduceMotion: Bool {
        theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion)
    }

    func body(content: Content) -> some View {
        let resolvedOpacity: Double = reduceMotion
            ? baseOpacity * 0.82
            : (isDimmed ? baseOpacity * 0.48 : baseOpacity)

        content
            .opacity(resolvedOpacity)
            .onAppear {
                guard !reduceMotion else {
                    isDimmed = false
                    return
                }
                withAnimation(.easeInOut(duration: 1.2).repeatForever(autoreverses: true)) {
                    isDimmed = true
                }
            }
            .onChange(of: reduceMotion) { _, newValue in
                if newValue {
                    isDimmed = false
                } else {
                    withAnimation(.easeInOut(duration: 1.2).repeatForever(autoreverses: true)) {
                        isDimmed = true
                    }
                }
            }
    }
}

private extension View {
    func fabushiSkeletonPulse(baseOpacity: Double = 1.0) -> some View {
        modifier(FabushiSkeletonPulseModifier(baseOpacity: baseOpacity))
    }
}

// MARK: - 1. Cold-Cache 7-Row Dialog List Skeleton (`D13-03`)

@MainActor
public struct FabushiDialogListSkeletonView: View {
    public static let rowCount: Int = 7

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    private static let titleWidthFractions: [CGFloat] = [0.56, 0.44, 0.62, 0.48, 0.52, 0.40, 0.50]
    private static let subtitleWidthFractions: [CGFloat] = [0.82, 0.70, 0.88, 0.64, 0.76, 0.68, 0.72]

    public init() {}

    public var body: some View {
        VStack(spacing: 0) {
            ForEach(0..<Self.rowCount, id: \.self) { index in
                let fade = max(0.32, 1.0 - (Double(index) * 0.1))
                HStack(spacing: 12) {
                    Circle()
                        .fill(theme.surfaceInput)
                        .frame(width: 48, height: 48)

                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .fill(theme.surfaceInput)
                                .frame(width: 140 * Self.titleWidthFractions[index], height: 14)
                            Spacer()
                            RoundedRectangle(cornerRadius: 5, style: .continuous)
                                .fill(theme.surfaceInput)
                                .frame(width: 36, height: 10)
                        }

                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .fill(theme.surfaceInput.opacity(0.85))
                            .frame(width: 200 * Self.subtitleWidthFractions[index], height: 12)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .fabushiSkeletonPulse(baseOpacity: fade)

                if index < Self.rowCount - 1 {
                    Divider()
                        .overlay(theme.separatorHairline)
                        .padding(.leading, 76)
                }
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(theme.messagingString(.loadingChatsAccessibility))
        .accessibilityAddTraits(.updatesFrequently)
    }
}

// MARK: - 2. Collapsed iPad Rail 5-Avatar Skeleton (`D13-03`)

@MainActor
public struct FabushiCollapsedRailSkeletonView: View {
    public static let avatarCount: Int = 5

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init() {}

    public var body: some View {
        VStack(spacing: 14) {
            ForEach(0..<Self.avatarCount, id: \.self) { index in
                let fade = max(0.36, 1.0 - (Double(index) * 0.13))
                Circle()
                    .fill(theme.surfaceInput)
                    .frame(width: 44, height: 44)
                    .fabushiSkeletonPulse(baseOpacity: fade)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 12)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(theme.messagingString(.loadingChatsAccessibility))
        .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.railList)
    }
}

// MARK: - 3. Uncached Conversation 6-Bubble Transcript Skeleton (`D13-03`)

@MainActor
public struct FabushiTranscriptSkeletonView: View {
    public static let bubbleCount: Int = 6

    private struct BubbleSpec {
        let isOutgoing: Bool
        let width: CGFloat
        let height: CGFloat
    }

    private static let specs: [BubbleSpec] = [
        BubbleSpec(isOutgoing: false, width: 210, height: 44),
        BubbleSpec(isOutgoing: true, width: 170, height: 38),
        BubbleSpec(isOutgoing: false, width: 240, height: 58),
        BubbleSpec(isOutgoing: true, width: 220, height: 44),
        BubbleSpec(isOutgoing: false, width: 160, height: 38),
        BubbleSpec(isOutgoing: true, width: 200, height: 52),
    ]

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init() {}

    public var body: some View {
        VStack(spacing: 10) {
            Spacer(minLength: 0)
            ForEach(Array(Self.specs.enumerated()), id: \.offset) { index, spec in
                let fade = max(0.42, 0.52 + (Double(index) * 0.09))
                HStack {
                    if spec.isOutgoing {
                        Spacer(minLength: 48)
                    }
                    RoundedRectangle(cornerRadius: theme.bubbleCornerRadius, style: .continuous)
                        .fill(spec.isOutgoing ? theme.accentColor.opacity(0.22) : theme.bubbleIncoming)
                        .frame(width: spec.width, height: spec.height)
                        .fabushiSkeletonPulse(baseOpacity: fade)
                    if !spec.isOutgoing {
                        Spacer(minLength: 48)
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(theme.messagingString(.loadingMessagesAccessibility))
        .accessibilityAddTraits(.updatesFrequently)
    }
}

// MARK: - 4. iPad Empty Detail Stage Syncing Placeholder (`D13-03`)

@MainActor
public struct FabushiStageSyncingPlaceholderView: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init() {}

    public var body: some View {
        VStack(spacing: 12) {
            HStack(spacing: 10) {
                Image(systemName: "arrow.triangle.2.circlepath")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.accentColor)
                    .accessibilityHidden(true)
                Text(theme.messagingString(.syncingMessagesPill))
                    .fabushiFont(.subheadline)
                    .fontWeight(.medium)
                    .foregroundStyle(theme.textSecondary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.searchCornerRadius)
            .fabushiSkeletonPulse(baseOpacity: 0.95)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(theme.messagingString(.syncingMessagesPill))
    }
}
