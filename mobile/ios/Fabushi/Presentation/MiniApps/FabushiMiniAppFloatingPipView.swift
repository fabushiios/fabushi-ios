import SwiftUI

/// Compact draggable Apple HIG floating PiP capsule (`makeMiniAppFloatingPipView`) honoring
/// `D10-12` (in-place restore without reloading pooled `WKWebView`) and `D12-01` (PiP state
/// NEVER steals keyboard or IME focus from native host inputs).
@MainActor
public struct FabushiMiniAppFloatingPipView: View {
    public let pip: MiniAppPipOverlayState
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let sessionPool: FabushiMiniAppSessionPool

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion
    @State private var dragOffset: CGSize = .zero
    @State private var snappedOffset: CGSize = .zero

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(
        pip: MiniAppPipOverlayState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        sessionPool: FabushiMiniAppSessionPool = .shared
    ) {
        self.pip = pip
        self.coordinator = coordinator
        self.router = router
        self.sessionPool = sessionPool
    }

    public var body: some View {
        HStack(spacing: 10) {
            Button {
                // In-place restore without reloading pooled WKWebView
                router.presentedMiniAppId = pip.id
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "app.badge.fill")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.accentColor)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(pip.title)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textPrimary)
                            .lineLimit(1)
                        if let statusText = pip.statusText, !statusText.isEmpty {
                            Text(statusText)
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textSecondary)
                                .lineLimit(1)
                        }
                    }
                }
            }
            .fabushiMinimumTouchTarget()
            .accessibilityLabel("\(FabushiPlatformStrings.text(.miniAppRestorePiP, locale: locale)): \(pip.title)")

            Button {
                sessionPool.teardownSession(
                    generation: coordinator.accountScopeGeneration,
                    appKey: pip.id,
                    accountId: coordinator.activeAccountId
                )
                router.activeMiniAppPip = nil
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
            }
            .fabushiMinimumTouchTarget()
            .accessibilityLabel(FabushiPlatformStrings.text(.miniAppCloseSession, locale: locale))
            .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.closeButton)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .fabushiLiquidGlass(cornerRadius: 22, isInteractive: true)
        .offset(
            x: snappedOffset.width + dragOffset.width,
            y: snappedOffset.height + dragOffset.height
        )
        .gesture(
            DragGesture()
                .onChanged { value in
                    dragOffset = value.translation
                }
                .onEnded { value in
                    let rawX = snappedOffset.width + value.translation.width
                    let rawY = snappedOffset.height + value.translation.height
                    let clampedX = min(max(rawX, -220), 0)
                    let clampedY = min(max(rawY, -360), 0)
                    let snapX: CGFloat = clampedX < -110 ? -200 : 0
                    let target = CGSize(width: snapX, height: clampedY)
                    if theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion) {
                        snappedOffset = target
                        dragOffset = .zero
                    } else {
                        withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                            snappedOffset = target
                            dragOffset = .zero
                        }
                    }
                }
        )
        .onAppear {
            // Enforce D12-01 invariant on PiP mount: never allow first responder while minimized!
            if let pooled = sessionPool.existingSession(
                generation: coordinator.accountScopeGeneration,
                appKey: pip.id
            ) {
                pooled.webView.enforceFocusPolicy(
                    context: FabushiWebViewFocusContext(
                        isPresented: false,
                        isMinimizedToPiP: true,
                        isBackgroundWebMcp: false,
                        hasBlockingModal: false,
                        isHostInputFocused: false
                    )
                )
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.floatingPiPPill)
    }
}
