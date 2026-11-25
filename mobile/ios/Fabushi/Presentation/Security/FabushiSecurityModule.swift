import SwiftUI

/// Production M2b Security module provider implementing ALL 6 factories of `SecurityModuleProviding`.
@MainActor
public final class FabushiSecurityModule: SecurityModuleProviding {
    private let passcodeController: FabushiPasscodeLockController
    private let securityKeychain: FabushiSecurityKeychainStore

    public init(
        passcodeController: FabushiPasscodeLockController = .shared,
        securityKeychain: FabushiSecurityKeychainStore = .shared
    ) {
        self.passcodeController = passcodeController
        self.securityKeychain = securityKeychain
    }

    public func makePasscodeLockOverlay(
        coordinator: FabushiCoordinator,
        launchConfiguration: FabushiLaunchConfiguration,
        onUnlock: @escaping () -> Void
    ) -> AnyView {
        AnyView(
            FabushiPasscodeLockOverlayView(
                coordinator: coordinator,
                launchConfiguration: launchConfiguration,
                passcodeController: passcodeController,
                onUnlock: onUnlock
            )
        )
    }

    public func makePrivacyBlurOverlay() -> AnyView {
        AnyView(FabushiPrivacyBlurOverlayView())
    }

    public func installLogoutSecretsPurger(on coordinator: FabushiCoordinator) {
        let keychain = securityKeychain
        coordinator.registerAccountSecretsPurger(id: "fabushi.security.keychain") { accountId in
            keychain.purgeAccountSecrets(accountId: accountId)
        }
        coordinator.registerServerSwitchSecretsPurger(id: "fabushi.security.keychain") {
            keychain.purgeAllAccountScopedSecrets()
        }
    }

    public func makeAccountSwitcherSheet(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        onDismiss: @escaping () -> Void
    ) -> AnyView {
        installLogoutSecretsPurger(on: coordinator)
        return AnyView(
            FabushiAccountSwitcherSheet(
                coordinator: coordinator,
                router: router,
                securityKeychain: securityKeychain,
                onDismiss: onDismiss
            )
        )
    }

    public func makeVaultSettingsView(
        coordinator: FabushiCoordinator
    ) -> AnyView {
        AnyView(
            FabushiCredentialVaultSettingsView(
                coordinator: coordinator,
                securityKeychain: securityKeychain
            )
        )
    }

    public func makeDevicesSettingsView(
        coordinator: FabushiCoordinator
    ) -> AnyView {
        AnyView(FabushiDevicesSettingsView(coordinator: coordinator))
    }

    public func makePrivacySecuritySettingsView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiPrivacySecuritySettingsView(
                coordinator: coordinator,
                router: router,
                passcodeController: passcodeController
            )
        )
    }
}
