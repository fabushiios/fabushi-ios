import SwiftUI

/// Production M2b Auth module provider implementing M2a's `AuthModuleProviding` contract.
@MainActor
public final class FabushiAuthModule: AuthModuleProviding {
    public init() {}

    public func makeAuthGateView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        launchConfiguration: FabushiLaunchConfiguration
    ) -> AnyView {
        AnyView(
            FabushiAuthWizardView(
                coordinator: coordinator,
                router: router,
                launchConfiguration: launchConfiguration,
                mode: .primaryGate
            )
        )
    }
}
