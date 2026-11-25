import SwiftUI

/// Compact Apple HIG floating Call / Talk Mode capsule returned by
/// `PlatformModulesProviding.makeCallAndTalkOverlayView`.
///
/// Tapping reopens the full `.calls` modal sheet without obstructing the tab bar or composer.
@MainActor
public struct FabushiCallPipOverlayView: View {
    public let callPip: CallPipOverlayState
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        callPip: CallPipOverlayState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) {
        self.callPip = callPip
        self.coordinator = coordinator
        self.router = router
    }

    public var body: some View {
        Button {
            router.navigate(to: .calls, coordinator: coordinator)
        } label: {
            HStack(spacing: 8) {
                Image(systemName: callPip.isMuted ? "mic.slash.fill" : "phone.connection.fill")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(callPip.isMuted ? theme.warningColor : theme.successColor)
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 1) {
                    Text(callPip.peerTitle)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(1)
                    Text(callPip.stateText)
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textSecondary)
                        .lineLimit(1)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .fabushiLiquidGlass(cornerRadius: 20, isInteractive: true)
            .fabushiMinimumTouchTarget()
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(callPip.peerTitle) · \(callPip.stateText)")
        .accessibilityIdentifier("fabushi.call.floatingPip")
    }
}
