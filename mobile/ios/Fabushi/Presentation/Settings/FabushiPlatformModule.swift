import SwiftUI

/// Concrete Milestone M5 provider implementing ALL 8 sealed `PlatformModulesProviding` factories
/// (wiring M5a Remote Computers / Mini Apps / PiP + M5b Calls / Talk Mode / 26-Section Settings /
/// Wallet & StoreKit 2) PLUS the public `makeBrowserWorkspaceView` hook ready for 1-line M6 wiring.
///
/// In M6, `FabushiSceneRoot.swift` registers this provider via:
/// `FabushiFeatureModuleRegistry.shared.register(platform: FabushiPlatformModule())`
@MainActor
public final class FabushiPlatformModule: PlatformModulesProviding {
    public init() {}

    public func makeRemoteComputerWorkbenchView(
        nodeId: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiRemoteComputerHubView(
                nodeId: nodeId,
                coordinator: coordinator,
                router: router
            )
        )
    }

    public func makeRemoteControlledPillView(
        status: RemoteControlStatusState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiRemoteControlledPillView(
                status: status,
                coordinator: coordinator,
                router: router
            )
        )
    }

    public func makeMiniAppHostSheetView(
        miniAppId: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiMiniAppHostSheetView(
                miniAppId: miniAppId,
                coordinator: coordinator,
                router: router,
                sessionPool: .shared
            )
        )
    }

    public func makeMiniAppFloatingPipView(
        pip: MiniAppPipOverlayState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiMiniAppFloatingPipView(
                pip: pip,
                coordinator: coordinator,
                router: router,
                sessionPool: .shared
            )
        )
    }

    public func makeCallAndTalkOverlayView(
        callPip: CallPipOverlayState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiCallPipOverlayView(
                callPip: callPip,
                coordinator: coordinator,
                router: router
            )
        )
    }

    public func makeCallsTabView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiCallsTabView(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public func makeSettingsRootView(
        initialSection: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiPlatformSettingsRootView(
                initialSection: initialSection,
                coordinator: coordinator,
                router: router
            )
        )
    }

    public func makeWalletAndPremiumViews(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiWalletAndPremiumView(
                coordinator: coordinator,
                router: router
            )
        )
    }

    /// Public M6 `D16-01` Built-in Interactive Browser & Automation App Store factory hook.
    public func makeBrowserWorkspaceView(
        initialURL: URL? = nil,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiBrowserWorkspaceView(
                initialURL: initialURL,
                coordinator: coordinator,
                router: router
            )
        )
    }
}
