import SwiftUI

/// Compact Apple HIG floating pill rendered by the shell's `makeRemoteControlledPillView`
/// whenever a remote control session is active (`D10-11` — replaces desktop floating
/// `RemoteControlledWindow`). Provides instant `<10ms` local emergency takeover & disconnect.
@MainActor
public struct FabushiRemoteControlledPillView: View {
    public let status: RemoteControlStatusState
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var isTakingOver: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(
        status: RemoteControlStatusState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) {
        self.status = status
        self.coordinator = coordinator
        self.router = router
    }

    public var body: some View {
        HStack(spacing: 10) {
            Circle()
                .fill(theme.warningColor)
                .frame(width: 8, height: 8)
                .accessibilityHidden(true)

            Button {
                router.navigate(to: .remoteComputer(nodeId: status.id), coordinator: coordinator)
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text(status.nodeName)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(1)
                    Text(status.controllerLabel)
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textSecondary)
                        .lineLimit(1)
                }
            }
            .fabushiMinimumTouchTarget()

            Button {
                performEmergencyTakeover()
            } label: {
                Text(FabushiPlatformStrings.text(.remoteControlledPillTakeover, locale: locale))
                    .fabushiFont(.caption)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(theme.destructiveColor, in: Capsule())
                    .foregroundStyle(Color.white)
            }
            .disabled(isTakingOver)
            .fabushiMinimumTouchTarget()
            .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.emergencyTakeoverButton)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .fabushiLiquidGlass(cornerRadius: 22, isInteractive: true)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.controlledBanner)
    }

    private func performEmergencyTakeover() {
        guard !isTakingOver else { return }
        isTakingOver = true
        Task { @MainActor in
            defer { isTakingOver = false }
            _ = try? await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("hubEmergencyTakeover"),
                params: FabushiEmptyParams()
            ) as FabushiHubEmergencyTakeoverResult
            _ = try? await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("sessionsDisconnectAll"),
                params: FabushiEmptyParams()
            ) as FabushiJSONValue
            router.activeRemoteControlStatus = nil
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.remoteEmergencyTakeoverSuccess, locale: locale),
                isError: false
            )
        }
    }
}
