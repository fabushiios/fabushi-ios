import Foundation
import Observation
import SwiftUI
#if canImport(UIKit)
import UIKit

@MainActor
@Observable
internal final class FabushiSecurityOverlayWindowModel {
    var isPasscodeOverlayActive: Bool
    var isPrivacyBlurActive: Bool
    var coordinator: FabushiCoordinator
    var theme: FabushiTheme
    var router: ShellNavigationRouter
    var registry: FabushiFeatureModuleRegistry
    var launchConfiguration: FabushiLaunchConfiguration
    @ObservationIgnored var onUnlock: () -> Void

    init(
        isPasscodeOverlayActive: Bool,
        isPrivacyBlurActive: Bool,
        coordinator: FabushiCoordinator,
        theme: FabushiTheme,
        router: ShellNavigationRouter,
        registry: FabushiFeatureModuleRegistry,
        launchConfiguration: FabushiLaunchConfiguration,
        onUnlock: @escaping () -> Void
    ) {
        self.isPasscodeOverlayActive = isPasscodeOverlayActive
        self.isPrivacyBlurActive = isPrivacyBlurActive
        self.coordinator = coordinator
        self.theme = theme
        self.router = router
        self.registry = registry
        self.launchConfiguration = launchConfiguration
        self.onUnlock = onUnlock
    }
}

@MainActor
private struct FabushiSecurityOverlayWindowContentView: View {
    let model: FabushiSecurityOverlayWindowModel
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    var body: some View {
        ZStack {
            if model.isPasscodeOverlayActive {
                model.registry.security.makePasscodeLockOverlay(
                    coordinator: model.coordinator,
                    launchConfiguration: model.launchConfiguration,
                    onUnlock: model.onUnlock
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .zIndex(50)
            }

            if model.isPrivacyBlurActive {
                model.registry.security.makePrivacyBlurOverlay()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .zIndex(100)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .preferredColorScheme(model.theme.colorScheme)
        .tint(model.theme.accentColor)
        .environment(\.locale, model.theme.locale.foundationLocale)
        .environment(model.coordinator)
        .environment(model.theme)
        .environment(model.router)
        .environment(model.registry)
        .transaction { transaction in
            if model.theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion) {
                transaction.animation = nil
                transaction.disablesAnimations = true
            }
        }
    }
}

/// Manages a dedicated top-level `UIWindow` (`UIWindow.Level.alert + 1`) that presents
/// `FabushiPrivacyBlurOverlayView` and `FabushiPasscodeLockOverlayView` strictly above ALL
/// UIKit-presented `.sheet`s (`UITransitionView`) without dismissing or tearing down sheet `@State`.
@MainActor
public final class FabushiSecurityOverlayWindowCoordinator {
    public static let overlayWindowLevel = UIWindow.Level(rawValue: UIWindow.Level.alert.rawValue + 1)

    public private(set) var overlayWindow: UIWindow?
    private weak var hostWindow: UIWindow?
    private weak var previousKeyWindow: UIWindow?
    private let model: FabushiSecurityOverlayWindowModel

    public var isOverlayWindowVisible: Bool {
        guard let overlayWindow else { return false }
        return !overlayWindow.isHidden
    }

    public init(
        coordinator: FabushiCoordinator,
        theme: FabushiTheme,
        router: ShellNavigationRouter,
        registry: FabushiFeatureModuleRegistry,
        launchConfiguration: FabushiLaunchConfiguration,
        onUnlock: @escaping () -> Void
    ) {
        self.model = FabushiSecurityOverlayWindowModel(
            isPasscodeOverlayActive: false,
            isPrivacyBlurActive: false,
            coordinator: coordinator,
            theme: theme,
            router: router,
            registry: registry,
            launchConfiguration: launchConfiguration,
            onUnlock: onUnlock
        )
    }

    public func update(
        isPasscodeOverlayActive: Bool,
        isPrivacyBlurActive: Bool,
        hostWindow: UIWindow?,
        coordinator: FabushiCoordinator,
        theme: FabushiTheme,
        router: ShellNavigationRouter,
        registry: FabushiFeatureModuleRegistry,
        launchConfiguration: FabushiLaunchConfiguration,
        onUnlock: @escaping () -> Void
    ) {
        if model.isPasscodeOverlayActive != isPasscodeOverlayActive {
            model.isPasscodeOverlayActive = isPasscodeOverlayActive
        }
        if model.isPrivacyBlurActive != isPrivacyBlurActive {
            model.isPrivacyBlurActive = isPrivacyBlurActive
        }
        if model.coordinator !== coordinator {
            model.coordinator = coordinator
        }
        if model.theme !== theme {
            model.theme = theme
        }
        if model.router !== router {
            model.router = router
        }
        if model.registry !== registry {
            model.registry = registry
        }
        model.launchConfiguration = launchConfiguration
        model.onUnlock = onUnlock

        if let hostWindow {
            self.hostWindow = hostWindow
        }

        applyWindowPresentationState()
    }

    public func attachHostWindow(_ window: UIWindow?) {
        hostWindow = window
        applyWindowPresentationState()
    }

    public func teardown() {
        restorePreviousKeyWindowIfNeeded()
        if let existing = overlayWindow {
            existing.accessibilityViewIsModal = false
            existing.rootViewController?.view.accessibilityViewIsModal = false
            existing.isHidden = true
            existing.rootViewController = nil
            existing.windowScene = nil
        }
        overlayWindow = nil
        previousKeyWindow = nil
        hostWindow = nil
        model.isPasscodeOverlayActive = false
        model.isPrivacyBlurActive = false
    }

    private func applyWindowPresentationState() {
        let shouldShow = model.isPasscodeOverlayActive || model.isPrivacyBlurActive
        guard shouldShow else {
            hideOverlayWindow()
            return
        }

        guard let targetScene = hostWindow?.windowScene ?? Self.resolveActiveWindowScene() else {
            return
        }

        let window = ensureOverlayWindow(for: targetScene)
        window.windowLevel = Self.overlayWindowLevel
        window.accessibilityViewIsModal = true
        window.rootViewController?.view.accessibilityViewIsModal = true

        if model.isPasscodeOverlayActive {
            if !window.isKeyWindow {
                previousKeyWindow = targetScene.windows.first(where: { $0.isKeyWindow && $0 !== window })
                    ?? hostWindow
                window.makeKeyAndVisible()
            } else {
                window.isHidden = false
            }
        } else {
            // Privacy blur only: cover all sheets & snapshots without forcibly stealing keyboard focus
            // when returning to an unlocked session, while still blocking interaction on the overlay window.
            if window.isKeyWindow {
                restorePreviousKeyWindowIfNeeded()
            }
            window.isHidden = false
        }
    }

    private func ensureOverlayWindow(for scene: UIWindowScene) -> UIWindow {
        if let existing = overlayWindow, existing.windowScene === scene {
            return existing
        }

        if let stale = overlayWindow {
            stale.accessibilityViewIsModal = false
            stale.rootViewController?.view.accessibilityViewIsModal = false
            stale.isHidden = true
            stale.rootViewController = nil
            stale.windowScene = nil
            overlayWindow = nil
        }

        let window = UIWindow(windowScene: scene)
        window.windowLevel = Self.overlayWindowLevel
        window.backgroundColor = .clear
        let controller = UIHostingController(
            rootView: FabushiSecurityOverlayWindowContentView(model: model)
        )
        controller.view.backgroundColor = .clear
        window.rootViewController = controller
        overlayWindow = window
        return window
    }

    private func hideOverlayWindow() {
        restorePreviousKeyWindowIfNeeded()
        if let existing = overlayWindow {
            existing.accessibilityViewIsModal = false
            existing.rootViewController?.view.accessibilityViewIsModal = false
            existing.isHidden = true
        }
    }

    private func restorePreviousKeyWindowIfNeeded() {
        guard let existing = overlayWindow, existing.isKeyWindow else {
            previousKeyWindow = nil
            return
        }
        let target = previousKeyWindow ?? hostWindow
        previousKeyWindow = nil
        if let target, target !== existing, !target.isHidden {
            target.makeKey()
        }
    }

    private static func resolveActiveWindowScene() -> UIWindowScene? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        if let active = scenes.first(where: { $0.activationState == .foregroundActive }) {
            return active
        }
        if let inactive = scenes.first(where: { $0.activationState == .foregroundInactive }) {
            return inactive
        }
        return scenes.first
    }
}

/// SwiftUI anchor view that binds `FabushiSecurityOverlayWindowCoordinator` to the hosting `UIWindowScene`.
@MainActor
internal struct FabushiSecurityOverlayWindowHost: UIViewRepresentable {
    let isPasscodeOverlayActive: Bool
    let isPrivacyBlurActive: Bool
    let coordinator: FabushiCoordinator
    let theme: FabushiTheme
    let router: ShellNavigationRouter
    let registry: FabushiFeatureModuleRegistry
    let launchConfiguration: FabushiLaunchConfiguration
    let onUnlock: () -> Void

    final class AnchorView: UIView {
        var onWindowChange: (@MainActor (UIWindow?) -> Void)?

        override init(frame: CGRect) {
            super.init(frame: frame)
            isUserInteractionEnabled = false
            backgroundColor = .clear
            isOpaque = false
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("init(coder:) has not been implemented")
        }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            onWindowChange?(window)
        }
    }

    func makeCoordinator() -> FabushiSecurityOverlayWindowCoordinator {
        FabushiSecurityOverlayWindowCoordinator(
            coordinator: coordinator,
            theme: theme,
            router: router,
            registry: registry,
            launchConfiguration: launchConfiguration,
            onUnlock: onUnlock
        )
    }

    func makeUIView(context: Context) -> AnchorView {
        let view = AnchorView(frame: .zero)
        let windowCoordinator = context.coordinator
        view.onWindowChange = { attachedWindow in
            windowCoordinator.attachHostWindow(attachedWindow)
        }
        return view
    }

    func updateUIView(_ uiView: AnchorView, context: Context) {
        context.coordinator.update(
            isPasscodeOverlayActive: isPasscodeOverlayActive,
            isPrivacyBlurActive: isPrivacyBlurActive,
            hostWindow: uiView.window,
            coordinator: coordinator,
            theme: theme,
            router: router,
            registry: registry,
            launchConfiguration: launchConfiguration,
            onUnlock: onUnlock
        )
    }

    static func dismantleUIView(_ uiView: AnchorView, coordinator: FabushiSecurityOverlayWindowCoordinator) {
        uiView.onWindowChange = nil
        coordinator.teardown()
    }
}
#endif
