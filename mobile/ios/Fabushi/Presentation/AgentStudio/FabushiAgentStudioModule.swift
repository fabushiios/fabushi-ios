import SwiftUI

/// Sealed Milestone M4 module provider implementing ALL 5 factories of `AgentStudioModuleProviding`.
///
/// Designed for strict 1-line M6 registration in `FabushiSceneRoot.swift`:
/// ```swift
/// FabushiFeatureModuleRegistry.shared.register(agentStudio: FabushiAgentStudioModule())
/// ```
/// Every factory method idempotently installs `logoutCleanupHub` onto `FabushiCoordinator`'s
/// departing-account and server-switch purgers so logout & account switching always wipe all
/// M4 ViewModel state (`D10-02` & `D10-10`).
@MainActor
public final class FabushiAgentStudioModule: AgentStudioModuleProviding {
    public let logoutCleanupHub: FabushiAgentStudioLogoutCleanupHub

    public convenience init() {
        self.init(logoutCleanupHub: FabushiAgentStudioLogoutCleanupHub())
    }

    public init(logoutCleanupHub: FabushiAgentStudioLogoutCleanupHub) {
        self.logoutCleanupHub = logoutCleanupHub
    }

    public func installLogoutCleanup(on coordinator: FabushiCoordinator) {
        logoutCleanupHub.install(on: coordinator)
    }

    public func makeAgentStudioWorkspaceView(
        initialTab: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        installLogoutCleanup(on: coordinator)
        let hub = logoutCleanupHub
        return AnyView(
            FabushiAgentStudioWorkspaceView(
                initialTab: initialTab,
                coordinator: coordinator,
                router: router,
                cleanupHub: hub,
                mcpConsoleBuilder: { coord, rtr in
                    AnyView(
                        FabushiOfficialMcpConsoleView(
                            coordinator: coord,
                            router: rtr,
                            cleanupHub: hub
                        )
                    )
                },
                codingAgentsPanelBuilder: { coord, rtr in
                    AnyView(
                        FabushiCodingAgentsPanelView(
                            coordinator: coord,
                            router: rtr,
                            cleanupHub: hub
                        )
                    )
                }
            )
        )
    }

    public func makeMagpieAiProvidersSettingsView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        installLogoutCleanup(on: coordinator)
        return AnyView(
            FabushiMagpieSettingsView(
                coordinator: coordinator,
                router: router,
                cleanupHub: logoutCleanupHub
            )
        )
    }

    public func makeOfficialMcpConsoleView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        installLogoutCleanup(on: coordinator)
        return AnyView(
            FabushiOfficialMcpConsoleView(
                coordinator: coordinator,
                router: router,
                cleanupHub: logoutCleanupHub
            )
        )
    }

    public func makeCodingAgentThreadAccessories(
        chatId: Int64,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        installLogoutCleanup(on: coordinator)
        return AnyView(
            FabushiCodingThreadAccessoriesView(
                chatId: chatId,
                coordinator: coordinator,
                router: router,
                cleanupHub: logoutCleanupHub
            )
        )
    }

    public func makeMultiAgentRoomAccessories(
        chatId: Int64,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        installLogoutCleanup(on: coordinator)
        return AnyView(
            FabushiMultiAgentRoomAccessoriesView(
                chatId: chatId,
                coordinator: coordinator,
                router: router,
                cleanupHub: logoutCleanupHub,
                isChatStageSurface: true
            )
        )
    }
}
