import Foundation
import SwiftUI

/// Unified Adaptive Apple HIG Shell for Fabushi iOS (`v1.6.9`).
/// - **iPad (`horizontalSizeClass == .regular`)**: 3-column `NavigationSplitView` + collapsible sidebar rail
///   + trailing `320pt` `.inspector(isPresented:)`.
/// - **iPhone (`horizontalSizeClass == .compact`)**: Native `TabView` (Chats, Contacts, Agent Studio, Remote,
///   Settings) + `NavigationStack` + `.sheet` Inspector.
/// - **Lifecycle Gating**: Fail-closed startup retry (`FabushiStartupRetryView`) -> Auth Gate
///   (`AuthModuleProviding`) -> Authenticated Shell + Passcode Lock / Privacy Blur (`SecurityModuleProviding`)
///   + Floating PiP / Call / Remote overlays + Two-Layer Toast Host (`FabushiTwoLayerToastHost`).
@MainActor
public struct FabushiAdaptiveShellView: View {
    public let coordinator: FabushiCoordinator
    public let theme: FabushiTheme
    public let router: ShellNavigationRouter
    public let registry: FabushiFeatureModuleRegistry
    public let launchConfiguration: FabushiLaunchConfiguration
    public let startupErrorMessage: String?
    public let onRetryStartup: @MainActor () async -> Void

    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    @State private var isRetryingStartup = false
    @State private var didSyncLaunchPasscodePolicy = false
    @State private var persistentBannerHeight: CGFloat = 0

    public init(
        coordinator: FabushiCoordinator = .shared,
        theme: FabushiTheme,
        router: ShellNavigationRouter,
        registry: FabushiFeatureModuleRegistry = .shared,
        launchConfiguration: FabushiLaunchConfiguration = .current,
        startupErrorMessage: String? = nil,
        onRetryStartup: @escaping @MainActor () async -> Void = {}
    ) {
        self.coordinator = coordinator
        self.theme = theme
        self.router = router
        self.registry = registry
        self.launchConfiguration = launchConfiguration
        self.startupErrorMessage = startupErrorMessage
        self.onRetryStartup = onRetryStartup
    }

    private enum ActiveModalSheet: Identifiable {
        case accountSwitcher
        case calls
        case miniApp(String)
        case compactInspector(InspectorTarget)

        var id: String {
            switch self {
            case .accountSwitcher: return "accountSwitcher"
            case .calls: return "calls"
            case .miniApp(let appId): return "miniApp:\(appId)"
            case .compactInspector: return "compactInspector"
            }
        }
    }

    private var isPasscodeOverlayActive: Bool {
        coordinator.authState != .unauthenticated && router.isPasscodeLocked
    }

    private var isPrivacyBlurActive: Bool {
        scenePhase != .active && (router.isPasscodeLocked || router.isPasscodeLockEnabled)
    }

    private var hasPersistentTopBanner: Bool {
        coordinator.authState != .unauthenticated
            && (router.activeDeepLinkBannerText != nil || router.activeProviderErrorMessage != nil)
    }

    private var topToastInset: CGFloat {
        let baseNavInset: CGFloat = horizontalSizeClass == .compact ? 56 : 52
        guard hasPersistentTopBanner else {
            return baseNavInset
        }
        // Floor prevents a single-frame collision over persistent banners before GeometryReader
        // reports, while `persistentBannerHeight` tracks exact rendered height across default and AX5.
        let estimatedFloor: CGFloat =
            (router.activeDeepLinkBannerText != nil ? 68 : 0)
            + (router.activeProviderErrorMessage != nil ? 170 : 0)
        return max(persistentBannerHeight, estimatedFloor) + baseNavInset
    }

    private var activeModalSheetBinding: Binding<ActiveModalSheet?> {
        Binding(
            get: {
                // Strictly suppress modal `.sheet` presentations whenever Passcode Lock or
                // Privacy Blur is active so sheets never render above security overlays, while
                // preserving live Calls/MiniApp sheets across transient `.inactive` system prompts
                // when Passcode Lock is disabled.
                guard !router.isPasscodeLocked, !isPrivacyBlurActive else {
                    return nil
                }
                if router.isAccountSwitcherPresented {
                    return .accountSwitcher
                }
                if let miniAppId = router.presentedMiniAppId {
                    return .miniApp(miniAppId)
                }
                if router.isCallsSheetPresented {
                    return .calls
                }
                if horizontalSizeClass == .compact,
                   router.isInspectorPresented,
                   let target = router.inspectorTarget {
                    return .compactInspector(target)
                }
                return nil
            },
            set: { newValue in
                guard newValue == nil else { return }
                // Do not destroy modal sheet flags when transient Passcode Lock or Privacy Blur
                // suppresses presentation; unlocked user swipe-dismiss clears modal state cleanly.
                guard !router.isPasscodeLocked, !isPrivacyBlurActive else { return }
                router.dismissActiveModalSheet(
                    coordinator: coordinator,
                    dismissCompactInspector: horizontalSizeClass == .compact
                )
            }
        )
    }

    private var regularInspectorBinding: Binding<Bool> {
        Binding(
            get: {
                !router.isPasscodeLocked
                    && !isPrivacyBlurActive
                    && horizontalSizeClass == .regular
                    && router.isInspectorPresented
                    && router.inspectorTarget != nil
            },
            set: { presented in
                if !presented {
                    guard !router.isPasscodeLocked, !isPrivacyBlurActive else { return }
                    router.dismissInspector()
                }
            }
        )
    }

    public var body: some View {
        ZStack(alignment: .top) {
            rootLifecycleSurface
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .allowsHitTesting(!isPasscodeOverlayActive && !isPrivacyBlurActive)

            if !isPasscodeOverlayActive,
               !isPrivacyBlurActive,
               (router.toastController.stagePill != nil || router.toastController.windowBanner != nil) {
                FabushiTwoLayerToastHost(controller: router.toastController)
                    .padding(.top, topToastInset)
                    .zIndex(40)
            }

            if isPasscodeOverlayActive {
                registry.security.makePasscodeLockOverlay(
                    coordinator: coordinator,
                    launchConfiguration: launchConfiguration
                ) {
                    _ = router.unlockPasscode(coordinator: coordinator)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .transition(.opacity)
                .zIndex(50)
            }

            if isPrivacyBlurActive {
                registry.security.makePrivacyBlurOverlay()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .zIndex(100)
            }
        }
        .preferredColorScheme(theme.colorScheme)
        .tint(theme.accentColor)
        .environment(\.locale, theme.locale.foundationLocale)
        .environment(coordinator)
        .environment(theme)
        .environment(router)
        .environment(registry)
        .transaction { transaction in
            if theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion) {
                transaction.animation = nil
                transaction.disablesAnimations = true
            }
        }
        .onAppear {
            if !didSyncLaunchPasscodePolicy {
                didSyncLaunchPasscodePolicy = true
                if launchConfiguration.requirePasscode && !router.isPasscodeLockEnabled {
                    router.isPasscodeLockEnabled = true
                    router.engagePasscodeLockIfEnabled()
                }
            }
        }
        .onChange(of: coordinator.accountScopeGeneration) { _, newGeneration in
            router.handleAccountScopeChange(generation: newGeneration, coordinator: coordinator)
            if coordinator.authState != .unauthenticated, !router.isPasscodeLocked {
                router.replayPendingDeepLinkIfReady(coordinator: coordinator)
            }
        }
        .onChange(of: coordinator.activeRoute) { _, _ in
            router.syncFromCoordinator(coordinator)
        }
        .onChange(of: coordinator.authState) { oldState, newState in
            if newState == .unauthenticated {
                persistentBannerHeight = 0
                router.engagePasscodeLockIfEnabled()
            } else {
                if oldState == .unauthenticated {
                    router.engagePasscodeLockIfEnabled()
                }
                router.handleAccountScopeChange(
                    generation: coordinator.accountScopeGeneration,
                    coordinator: coordinator
                )
                if !router.isPasscodeLocked {
                    router.replayPendingDeepLinkIfReady(coordinator: coordinator)
                }
            }
        }
        .onChange(of: scenePhase) { _, phase in
            router.handleScenePhaseChange(phase)
            if phase == .background {
                router.engagePasscodeLockIfEnabled()
            }
        }
    }

    // MARK: - Lifecycle Gating Order

    @ViewBuilder
    private var rootLifecycleSurface: some View {
        let effectiveStartupError = startupErrorMessage ?? coordinator.lastErrorMessage
        if !coordinator.isStarted, let errorMessage = effectiveStartupError {
            FabushiStartupRetryView(
                errorMessage: errorMessage,
                isRetrying: isRetryingStartup
            ) {
                guard !isRetryingStartup else { return }
                isRetryingStartup = true
                Task {
                    await onRetryStartup()
                    isRetryingStartup = false
                }
            }
        } else if !coordinator.isStarted {
            VStack(spacing: 12) {
                ProgressView()
                    .controlSize(.large)
                Text(theme.localized("hig.status.starting"))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(theme.surfacePrimary)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(theme.localized("hig.status.starting"))
        } else if coordinator.authState == .unauthenticated {
            registry.auth.makeAuthGateView(
                coordinator: coordinator,
                router: router,
                launchConfiguration: launchConfiguration
            )
        } else {
            authenticatedShell
                .sheet(item: activeModalSheetBinding) { sheet in
                    switch sheet {
                    case .accountSwitcher:
                        registry.security.makeAccountSwitcherSheet(
                            coordinator: coordinator,
                            router: router
                        ) {
                            router.isAccountSwitcherPresented = false
                        }
                    case .calls:
                        registry.platform.makeCallsTabView(
                            coordinator: coordinator,
                            router: router
                        )
                    case .miniApp(let appId):
                        registry.platform.makeMiniAppHostSheetView(
                            miniAppId: appId,
                            coordinator: coordinator,
                            router: router
                        )
                    case .compactInspector(let target):
                        registry.messaging.makeContextInspectorView(
                            target: target,
                            coordinator: coordinator,
                            router: router
                        )
                    }
                }
        }
    }

    // MARK: - Authenticated Adaptive Shell

    @ViewBuilder
    private var authenticatedShell: some View {
        VStack(spacing: 0) {
            if hasPersistentTopBanner {
                topPersistentBanners
                    .background(
                        GeometryReader { proxy in
                            Color.clear
                                .preference(
                                    key: PersistentBannerHeightPreferenceKey.self,
                                    value: proxy.size.height
                                )
                        }
                    )
            }

            Group {
                if horizontalSizeClass == .regular {
                    regularIPadShell
                } else {
                    compactIPhoneShell
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onPreferenceChange(PersistentBannerHeightPreferenceKey.self) { height in
            persistentBannerHeight = hasPersistentTopBanner ? height : 0
        }
        .onChange(of: hasPersistentTopBanner) { _, isActive in
            if !isActive {
                persistentBannerHeight = 0
            }
        }
        .overlay(alignment: .bottomTrailing) {
            floatingOverlayStack
                .padding(.trailing, 16)
                .padding(.bottom, horizontalSizeClass == .compact ? 116 : 84)
        }
        .background(theme.surfacePrimary)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.root)
    }

    // MARK: - iPad Regular 3-Column Adaptive Shell

    @ViewBuilder
    private var regularIPadShell: some View {
        Group {
            if router.isSidebarCollapsed {
                HStack(spacing: 0) {
                    collapsedSidebarRail
                    Divider()
                    stageContainerView
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.splitView)
            } else {
                NavigationSplitView(columnVisibility: .constant(.all)) {
                    expandedSidebarView
                        .toolbar(removing: .sidebarToggle)
                        .navigationSplitViewColumnWidth(
                            min: FabushiLayoutMetrics.sidebarMinWidth,
                            ideal: FabushiLayoutMetrics.sidebarDefaultWidth,
                            max: FabushiLayoutMetrics.sidebarMaxWidth
                        )
                } detail: {
                    stageContainerView
                }
                .navigationSplitViewStyle(.balanced)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.splitView)
            }
        }
        .inspector(isPresented: regularInspectorBinding) {
            if let target = router.inspectorTarget {
                registry.messaging.makeContextInspectorView(
                    target: target,
                    coordinator: coordinator,
                    router: router
                )
                .inspectorColumnWidth(
                    min: FabushiLayoutMetrics.inspectorMinWidth,
                    ideal: FabushiLayoutMetrics.inspectorIdealWidth,
                    max: FabushiLayoutMetrics.inspectorMaxWidth
                )
            }
        }
    }

    private var collapsedSidebarRail: some View {
        VStack(spacing: 12) {
            Button {
                router.toggleSidebar()
            } label: {
                Image(systemName: "sidebar.leading")
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.localized("hig.shell.expandSidebar"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.sidebarCollapseToggle)

            Divider()

            ForEach(FabushiShellTab.allCases) { tab in
                let isSelected = router.activeTab == tab
                Button {
                    router.selectTab(tab, coordinator: coordinator)
                } label: {
                    Image(systemName: tab.systemImage)
                        .fabushiFont(.headline)
                        .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textSecondary)
                        .fabushiMinimumTouchTarget()
                        .background(
                            isSelected ? theme.accentColor : Color.clear,
                            in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                        )
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.localized(tab.localizationKey))
                .accessibilityIdentifier(tab.accessibilityIdentifier)
            }

            Spacer()

            accountSwitcherButton
        }
        .padding(.vertical, 12)
        .frame(width: FabushiLayoutMetrics.sidebarCollapsedWidth)
        .background(theme.surfaceSecondary)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.sidebarCollapsedRail)
    }

    private var expandedSidebarView: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                accountSwitcherButton
                FabushiConnectionStatusPill(
                    isOnline: coordinator.isOnline,
                    wsStatus: coordinator.settingsStore.wsConnectionStatus
                )
                Spacer(minLength: 4)
                Button {
                    router.toggleSidebar()
                } label: {
                    Image(systemName: "sidebar.leading")
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.localized("hig.shell.collapseSidebar"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.sidebarCollapseToggle)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(theme.surfaceTopBar)

            HStack(spacing: 4) {
                ForEach(FabushiShellTab.allCases) { tab in
                    let isSelected = router.activeTab == tab
                    Button {
                        router.selectTab(tab, coordinator: coordinator)
                    } label: {
                        Image(systemName: tab.systemImage)
                            .fabushiFont(.callout)
                            .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textSecondary)
                            .frame(maxWidth: .infinity)
                            .fabushiMinimumTouchTarget()
                            .background(
                                isSelected ? theme.accentColor : Color.clear,
                                in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                            )
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.localized(tab.localizationKey))
                    .accessibilityIdentifier(tab.accessibilityIdentifier)
                }
            }
            .padding(.horizontal, 8)
            .padding(.bottom, 8)
            .background(theme.surfaceTopBar)

            Divider()

            sidebarTabContent
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(theme.surfaceSecondary)
    }

    @ViewBuilder
    private var sidebarTabContent: some View {
        switch router.activeTab {
        case .chats:
            registry.messaging.makeChatSidebarView(coordinator: coordinator, router: router)
        case .contacts:
            registry.messaging.makeContactsView(coordinator: coordinator, router: router)
        case .studio, .remote, .settings:
            List {
                ForEach(FabushiShellTab.allCases) { tab in
                    Button {
                        router.selectTab(tab, coordinator: coordinator)
                    } label: {
                        Label(
                            theme.localized(tab.localizationKey),
                            systemImage: tab.systemImage
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(
                            router.activeTab == tab ? theme.accentColor : theme.textPrimary
                        )
                        .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                }
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
        }
    }

    /// Shared workspace route underneath modal `.calls`/`.miniApp` sheets so both iPad and
    /// compact iPhone preserve active conversation `@State` without duplicating live modal surfaces.
    private var resolvedWorkspaceRoute: ShellRoute {
        switch router.activeRoute {
        case .calls, .miniApp:
            return router.lastWorkspaceRoute
        default:
            return router.activeRoute
        }
    }

    @ViewBuilder
    private var stageContainerView: some View {
        NavigationStack {
            ZStack {
                switch resolvedWorkspaceRoute {
                case .compose:
                    registry.messaging.makeMultiSelectComposeView(
                        coordinator: coordinator,
                        router: router
                    )
                case .chats(let chatId, let messageId):
                    registry.messaging.makeConversationStageView(
                        chatId: chatId,
                        messageId: messageId,
                        coordinator: coordinator,
                        router: router
                    )
                case .agentStudio(let tab):
                    registry.agentStudio.makeAgentStudioWorkspaceView(
                        initialTab: tab,
                        coordinator: coordinator,
                        router: router
                    )
                case .remoteComputer(let nodeId):
                    registry.platform.makeRemoteComputerWorkbenchView(
                        nodeId: nodeId,
                        coordinator: coordinator,
                        router: router
                    )
                case .settings(let section):
                    registry.platform.makeSettingsRootView(
                        initialSection: section,
                        coordinator: coordinator,
                        router: router
                    )
                case .calls, .miniApp:
                    // Calls and Mini Apps are presented exclusively as modal sheets so the
                    // detail column never duplicates live WKWebView or Call surfaces underneath.
                    registry.messaging.makeConversationStageView(
                        chatId: nil,
                        messageId: nil,
                        coordinator: coordinator,
                        router: router
                    )
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.stageSlot)
        }
    }

    // MARK: - iPhone Compact TabView + NavigationStack Shell

    private var isCompactChatsDetailActive: Bool {
        if case .compose = resolvedWorkspaceRoute {
            return true
        }
        if case .chats(let chatId, _) = resolvedWorkspaceRoute, chatId != nil {
            return true
        }
        return false
    }

    private var compactIPhoneShell: some View {
        TabView(
            selection: Binding(
                get: { router.activeTab },
                set: { router.selectTab($0, coordinator: coordinator) }
            )
        ) {
            NavigationStack {
                Group {
                    if case .compose = resolvedWorkspaceRoute {
                        registry.messaging.makeMultiSelectComposeView(
                            coordinator: coordinator,
                            router: router
                        )
                    } else if case .chats(let chatId, let messageId) = resolvedWorkspaceRoute, chatId != nil {
                        ZStack {
                            registry.messaging.makeConversationStageView(
                                chatId: chatId,
                                messageId: messageId,
                                coordinator: coordinator,
                                router: router
                            )
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.stageSlot)
                    } else {
                        registry.messaging.makeChatSidebarView(
                            coordinator: coordinator,
                            router: router
                        )
                    }
                }
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        if isCompactChatsDetailActive {
                            Button {
                                router.navigateBackToChatList(coordinator: coordinator)
                            } label: {
                                HStack(spacing: 4) {
                                    Image(systemName: "chevron.backward")
                                        .fabushiFont(.headline)
                                        .accessibilityHidden(true)
                                    Text(theme.localized("hig.common.back"))
                                        .fabushiFont(.body)
                                        .lineLimit(1)
                                }
                                .foregroundStyle(theme.accentColor)
                                .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(theme.localized("hig.common.back"))
                            .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.backButton)
                        } else {
                            accountSwitcherButton
                        }
                    }
                    ToolbarItem(placement: .principal) {
                        FabushiConnectionStatusPill(
                            isOnline: coordinator.isOnline,
                            wsStatus: coordinator.settingsStore.wsConnectionStatus
                        )
                    }
                }
            }
            .tabItem {
                Label(
                    theme.localized(FabushiShellTab.chats.localizationKey),
                    systemImage: FabushiShellTab.chats.systemImage
                )
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.tabChats)
            }
            .tag(FabushiShellTab.chats)

            NavigationStack {
                registry.messaging.makeContactsView(coordinator: coordinator, router: router)
            }
            .tabItem {
                Label(
                    theme.localized(FabushiShellTab.contacts.localizationKey),
                    systemImage: FabushiShellTab.contacts.systemImage
                )
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.tabContacts)
            }
            .tag(FabushiShellTab.contacts)

            NavigationStack {
                registry.agentStudio.makeAgentStudioWorkspaceView(
                    initialTab: activeStudioTab,
                    coordinator: coordinator,
                    router: router
                )
            }
            .tabItem {
                Label(
                    theme.localized(FabushiShellTab.studio.localizationKey),
                    systemImage: FabushiShellTab.studio.systemImage
                )
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.tabStudio)
            }
            .tag(FabushiShellTab.studio)

            NavigationStack {
                registry.platform.makeRemoteComputerWorkbenchView(
                    nodeId: activeRemoteNodeId,
                    coordinator: coordinator,
                    router: router
                )
            }
            .tabItem {
                Label(
                    theme.localized(FabushiShellTab.remote.localizationKey),
                    systemImage: FabushiShellTab.remote.systemImage
                )
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.tabRemote)
            }
            .tag(FabushiShellTab.remote)

            NavigationStack {
                registry.platform.makeSettingsRootView(
                    initialSection: activeSettingsSection,
                    coordinator: coordinator,
                    router: router
                )
            }
            .tabItem {
                Label(
                    theme.localized(FabushiShellTab.settings.localizationKey),
                    systemImage: FabushiShellTab.settings.systemImage
                )
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.tabSettings)
            }
            .tag(FabushiShellTab.settings)
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.tabBar)
    }

    // MARK: - Shared Controls & Floating Overlay Slots

    private var accountSwitcherButton: some View {
        Button {
            router.isAccountSwitcherPresented = true
        } label: {
            Image(systemName: "person.crop.circle.fill")
                .fabushiFont(.title3)
                .foregroundStyle(theme.accentColor)
                .fabushiMinimumTouchTarget()
        }
        .buttonStyle(.plain)
        .accessibilityLabel(theme.localized("hig.accountSwitcher.button"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.accountSwitcherButton)
    }

    @ViewBuilder
    private var topPersistentBanners: some View {
        VStack(spacing: 8) {
            if let deepLinkText = router.activeDeepLinkBannerText {
                HStack(spacing: 8) {
                    Image(systemName: "link")
                        .foregroundStyle(theme.accentColor)
                        .accessibilityHidden(true)
                    Text("\(theme.localized("hig.deeplink.banner")): \(deepLinkText)")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Button {
                        router.dismissDeepLinkBanner()
                    } label: {
                        Image(systemName: "xmark")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.localized("hig.common.close"))
                }
                .padding(.leading, 12)
                .padding(.trailing, 4)
                .padding(.vertical, 4)
                .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius)
                .padding(.horizontal, 16)
                .accessibilityElement(children: .contain)
                .accessibilityLabel(theme.localized("hig.deeplink.banner"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.actionBusDeepLinkBanner)
            }

            if let providerError = router.activeProviderErrorMessage {
                FabushiProviderErrorCard(
                    detail: providerError,
                    onOpenAIProviders: {
                        router.dismissProviderErrorCard()
                        router.navigate(to: .settings(section: "aiProviders"), coordinator: coordinator)
                    },
                    onDismiss: {
                        router.dismissProviderErrorCard()
                    }
                )
                .padding(.horizontal, 16)
            }
        }
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity)
        .background(theme.surfaceTopBar)
        .overlay(alignment: .bottom) {
            Divider()
        }
    }

    @ViewBuilder
    private var floatingOverlayStack: some View {
        VStack(alignment: .trailing, spacing: 10) {
            if let remoteStatus = router.activeRemoteControlStatus {
                registry.platform.makeRemoteControlledPillView(
                    status: remoteStatus,
                    coordinator: coordinator,
                    router: router
                )
            }
            if let callPip = router.activeCallPip {
                registry.platform.makeCallAndTalkOverlayView(
                    callPip: callPip,
                    coordinator: coordinator,
                    router: router
                )
            }
            if let miniAppPip = router.activeMiniAppPip {
                registry.platform.makeMiniAppFloatingPipView(
                    pip: miniAppPip,
                    coordinator: coordinator,
                    router: router
                )
            }
        }
    }

    private var activeStudioTab: String? {
        if case .agentStudio(let tab) = resolvedWorkspaceRoute {
            return tab
        }
        return nil
    }

    private var activeRemoteNodeId: String? {
        if case .remoteComputer(let nodeId) = resolvedWorkspaceRoute {
            return nodeId
        }
        return nil
    }

    private var activeSettingsSection: String? {
        if case .settings(let section) = resolvedWorkspaceRoute {
            return section
        }
        return nil
    }
}

private struct PersistentBannerHeightPreferenceKey: PreferenceKey {
    static let defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}
