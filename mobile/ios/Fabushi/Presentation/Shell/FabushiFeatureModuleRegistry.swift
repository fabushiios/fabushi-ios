import Foundation
import Observation
import SwiftUI

// MARK: - Live Feature Module Registry

/// Central `@MainActor @Observable` registry allowing M2b, M3, M4, and M5 feature modules
/// to register independently into `FabushiAdaptiveShellView` with zero merge collisions.
@MainActor
@Observable
public final class FabushiFeatureModuleRegistry {
    private static var _sharedInstance: FabushiFeatureModuleRegistry?

    public static var shared: FabushiFeatureModuleRegistry {
        if let existing = _sharedInstance {
            return existing
        }
        let instance = FabushiFeatureModuleRegistry()
        _sharedInstance = instance
        return instance
    }

    public private(set) var messaging: any MessagingModuleProviding
    public private(set) var agentStudio: any AgentStudioModuleProviding
    public private(set) var platform: any PlatformModulesProviding
    public private(set) var auth: any AuthModuleProviding
    public private(set) var security: any SecurityModuleProviding

    public init(
        messaging: (any MessagingModuleProviding)? = nil,
        agentStudio: (any AgentStudioModuleProviding)? = nil,
        platform: (any PlatformModulesProviding)? = nil,
        auth: (any AuthModuleProviding)? = nil,
        security: (any SecurityModuleProviding)? = nil
    ) {
        self.messaging = messaging ?? DefaultMessagingModuleProvider()
        self.agentStudio = agentStudio ?? DefaultAgentStudioModuleProvider()
        self.platform = platform ?? DefaultPlatformModulesProvider()
        self.auth = auth ?? DefaultAuthModuleProvider()
        self.security = security ?? DefaultSecurityModuleProvider()
    }

    public func register(messaging provider: any MessagingModuleProviding) {
        self.messaging = provider
    }

    public func register(agentStudio provider: any AgentStudioModuleProviding) {
        self.agentStudio = provider
    }

    public func register(platform provider: any PlatformModulesProviding) {
        self.platform = provider
    }

    public func register(auth provider: any AuthModuleProviding) {
        self.auth = provider
    }

    public func register(security provider: any SecurityModuleProviding) {
        self.security = provider
    }

    public func resetToDefaults() {
        self.messaging = DefaultMessagingModuleProvider()
        self.agentStudio = DefaultAgentStudioModuleProvider()
        self.platform = DefaultPlatformModulesProvider()
        self.auth = DefaultAuthModuleProvider()
        self.security = DefaultSecurityModuleProvider()
    }
}

// MARK: - Honest Default Providers (ZERO synthetic sample chats, ZERO fake tokens/bypasses)

@MainActor
public final class DefaultMessagingModuleProvider: MessagingModuleProviding {
    public init() {}

    public func makeChatSidebarView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultChatSidebarSurface(router: router))
    }

    public func makeContactsView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultContactsSurface())
    }

    public func makeConversationStageView(
        chatId: Int64?,
        messageId: Int64?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultConversationStageSurface(chatId: chatId, router: router))
    }

    public func makeMultiSelectComposeView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultMultiSelectComposeSurface(coordinator: coordinator, router: router))
    }

    public func makeContextInspectorView(
        target: InspectorTarget,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultContextInspectorSurface(target: target, router: router))
    }

    public func makeUniversalSearchView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultMessagingUniversalSearchView())
    }

    public func makeMediaViewerOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(EmptyView())
    }

    public func makeStoriesOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(EmptyView())
    }

    public func makeInstantViewOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(EmptyView())
    }
}

@MainActor
public final class DefaultAgentStudioModuleProvider: AgentStudioModuleProviding {
    public init() {}

    public func makeAgentStudioWorkspaceView(
        initialTab: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultAgentStudioSurface())
    }

    public func makeMagpieAiProvidersSettingsView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultMagpieProvidersSurface())
    }

    public func makeOfficialMcpConsoleView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultAgentStudioSurface())
    }

    public func makeCodingAgentThreadAccessories(
        chatId: Int64,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(EmptyView())
    }

    public func makeMultiAgentRoomAccessories(
        chatId: Int64,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(EmptyView())
    }
}

@MainActor
public final class DefaultPlatformModulesProvider: PlatformModulesProviding {
    public init() {}

    public func makeRemoteComputerWorkbenchView(
        nodeId: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultRemoteWorkbenchSurface())
    }

    public func makeRemoteControlledPillView(
        status: RemoteControlStatusState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultRemoteControlledPillSurface(status: status))
    }

    public func makeMiniAppHostSheetView(
        miniAppId: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultMiniAppHostSurface(miniAppId: miniAppId, router: router))
    }

    public func makeMiniAppFloatingPipView(
        pip: MiniAppPipOverlayState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultMiniAppFloatingPipSurface(pip: pip, router: router))
    }

    public func makeCallAndTalkOverlayView(
        callPip: CallPipOverlayState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultCallPipSurface(callPip: callPip))
    }

    public func makeCallsTabView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(DefaultCallsDisclosureSurface())
    }

    public func makeSettingsRootView(
        initialSection: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiSettingsRootView(
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
        AnyView(DefaultCapabilityLedgerView())
    }
}

@MainActor
public final class DefaultAuthModuleProvider: AuthModuleProviding {
    public init() {}

    public func makeAuthGateView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        launchConfiguration: FabushiLaunchConfiguration
    ) -> AnyView {
        AnyView(
            DefaultAuthGateSurface(
                coordinator: coordinator,
                router: router,
                launchConfiguration: launchConfiguration
            )
        )
    }
}

@MainActor
public final class DefaultSecurityModuleProvider: SecurityModuleProviding {
    public init() {}

    public func makePasscodeLockOverlay(
        coordinator: FabushiCoordinator,
        launchConfiguration: FabushiLaunchConfiguration,
        onUnlock: @escaping () -> Void
    ) -> AnyView {
        AnyView(
            DefaultPasscodeLockSurface(
                coordinator: coordinator,
                onUnlock: onUnlock
            )
        )
    }

    public func makePrivacyBlurOverlay() -> AnyView {
        AnyView(DefaultPrivacyBlurSurface())
    }

    public func makeAccountSwitcherSheet(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        onDismiss: @escaping () -> Void
    ) -> AnyView {
        AnyView(
            DefaultAccountSwitcherSurface(
                coordinator: coordinator,
                router: router,
                onDismiss: onDismiss
            )
        )
    }

    public func makeVaultSettingsView(coordinator: FabushiCoordinator) -> AnyView {
        AnyView(DefaultVaultSettingsSurface(coordinator: coordinator))
    }

    public func makeDevicesSettingsView(coordinator: FabushiCoordinator) -> AnyView {
        AnyView(DefaultDevicesSettingsSurface(coordinator: coordinator))
    }
}

// MARK: - Default Messaging Surfaces (Honest, Zero Sample Chats)

@MainActor
private struct DefaultChatSidebarSurface: View {
    let router: ShellNavigationRouter
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(FabushiCoordinator.self) private var coordinator: FabushiCoordinator?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        VStack(spacing: 12) {
            HStack {
                Text(theme.localized("hig.tab.chats"))
                    .fabushiFont(.title2)
                    .foregroundStyle(theme.textPrimary)
                Spacer()
                Button {
                    if let coordinator {
                        router.navigate(to: .compose, coordinator: coordinator)
                    }
                } label: {
                    Image(systemName: "square.and.pencil")
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.localized("hig.common.compose"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.composeButton)
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)

            List {
                FabushiEmptyStateView(
                    title: theme.localized("hig.empty.chatsTitle"),
                    systemImage: "bubble.left.and.bubble.right",
                    descriptionText: theme.localized("hig.empty.chatsDescription"),
                    accessibilityID: FabushiAccessibilityIDs.ChatList.emptyState
                )
                .listRowBackground(Color.clear)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.list)
        }
        .background(theme.surfaceSecondary)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.root)
    }
}

@MainActor
private struct DefaultContactsSurface: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        List {
            FabushiEmptyStateView(
                title: theme.localized("hig.empty.contactsTitle"),
                systemImage: "person.2",
                descriptionText: theme.localized("hig.empty.contactsDescription"),
                accessibilityID: FabushiAccessibilityIDs.ChatList.contactsList
            )
            .listRowBackground(Color.clear)
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.tab.contacts"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.contactsList)
    }
}

@MainActor
private struct DefaultConversationStageSurface: View {
    let chatId: Int64?
    let router: ShellNavigationRouter
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        VStack(spacing: 16) {
            FabushiEmptyStateView(
                title: theme.localized("hig.tab.chats"),
                systemImage: "text.bubble",
                descriptionText: theme.localized("hig.empty.selectChat"),
                accessibilityID: FabushiAccessibilityIDs.Stage.root
            )
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .toolbar {
            if let chatId {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        router.openInspector(for: .chat(chatId: chatId))
                    } label: {
                        Image(systemName: "sidebar.trailing")
                            .fabushiMinimumTouchTarget()
                    }
                    .accessibilityLabel(theme.localized("hig.inspector.title"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.inspectorToggleButton)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.root)
    }
}

@MainActor
private struct DefaultMultiSelectComposeSurface: View {
    let coordinator: FabushiCoordinator
    let router: ShellNavigationRouter
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        VStack(spacing: 16) {
            HStack {
                Text(theme.localized("hig.common.compose"))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                Spacer()
                FabushiGlassPillButton(
                    title: theme.localized("hig.common.cancel"),
                    accessibilityID: FabushiAccessibilityIDs.Compose.cancelButton
                ) {
                    router.navigate(to: .chats(chatId: nil, messageId: nil), coordinator: coordinator)
                }
            }
            .padding()

            Spacer()

            FabushiEmptyStateView(
                title: theme.localized("hig.common.compose"),
                systemImage: "person.badge.plus",
                descriptionText: theme.localized("hig.empty.selectChat"),
                accessibilityID: FabushiAccessibilityIDs.Compose.candidateList
            )

            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.stage)
    }
}

@MainActor
private struct DefaultContextInspectorSurface: View {
    let target: InspectorTarget
    let router: ShellNavigationRouter
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        VStack(spacing: 16) {
            HStack {
                Text(theme.localized("hig.inspector.title"))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                Spacer()
                Button {
                    router.dismissInspector()
                } label: {
                    Image(systemName: "xmark")
                        .fabushiFont(.callout)
                        .foregroundStyle(theme.textSecondary)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.localized("hig.common.close"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.closeButton)
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)

            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    switch target {
                    case .bot(let botId, _):
                        FabushiSectionHeader(theme.localized("hig.inspector.title"))
                            .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.profileBotSection)
                        Text("#\(botId)")
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textSecondary)
                    case .group(let chatId):
                        FabushiSectionHeader(theme.localized("hig.inspector.title"))
                            .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.profileGroupSection)
                        Text("#\(chatId)")
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textSecondary)
                    case .user(let peerId):
                        FabushiSectionHeader(theme.localized("hig.inspector.title"))
                            .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.profileUserSection)
                        Text("#\(peerId)")
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textSecondary)
                    case .chat(let chatId):
                        FabushiSectionHeader(theme.localized("hig.inspector.title"))
                        Text("#\(chatId)")
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textSecondary)
                    }
                }
                .padding(16)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfaceSecondary)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(theme.localized("hig.inspector.title"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.drawer)
    }
}

@MainActor
private struct DefaultMessagingUniversalSearchView: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        FabushiEmptyStateView(
            title: theme.localized("hig.common.search"),
            systemImage: "magnifyingglass",
            descriptionText: theme.localized("hig.empty.selectChat"),
            accessibilityID: FabushiAccessibilityIDs.ChatList.searchResultsList
        )
        .background(theme.surfacePrimary)
    }
}

// MARK: - Default Agent Studio & Platform Surfaces

@MainActor
private struct DefaultAgentStudioSurface: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        FabushiEmptyStateView(
            title: theme.localized("hig.tab.studio"),
            systemImage: "sparkles.rectangle.stack",
            descriptionText: theme.localized("hig.module.studioSubtitle"),
            accessibilityID: FabushiAccessibilityIDs.Studio.root
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.tab.studio"))
    }
}

@MainActor
private struct DefaultMagpieProvidersSurface: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        FabushiEmptyStateView(
            title: theme.localized("hig.settings.aiProviders"),
            systemImage: "cpu",
            descriptionText: theme.localized("hig.module.studioSubtitle"),
            accessibilityID: FabushiAccessibilityIDs.Magpie.root
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.aiProviders"))
    }
}

@MainActor
private struct DefaultRemoteWorkbenchSurface: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "info.circle.fill")
                        .foregroundStyle(theme.accentColor)
                        .accessibilityHidden(true)
                    Text(theme.localized("hig.module.remoteHostUnsupported"))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textPrimary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(14)
                .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(theme.localized("hig.module.remoteHostUnsupported"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.localHostUnsupportedBanner)
            }
            .padding(16)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.tab.remote"))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.workbench)
    }
}

@MainActor
private struct DefaultRemoteControlledPillSurface: View {
    let status: RemoteControlStatusState
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "desktopcomputer")
                .foregroundStyle(theme.warningColor)
                .accessibilityHidden(true)
            Text("\(status.nodeName) · \(status.controllerLabel)")
                .fabushiFont(.caption)
                .foregroundStyle(theme.textPrimary)
                .lineLimit(1)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(status.nodeName) \(status.controllerLabel)")
        .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.controlledBanner)
    }
}

@MainActor
private struct DefaultMiniAppHostSurface: View {
    let miniAppId: String?
    let router: ShellNavigationRouter
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        NavigationStack {
            FabushiEmptyStateView(
                title: miniAppId ?? theme.localized("hig.app.name"),
                systemImage: "app.dashed",
                descriptionText: theme.localized("hig.module.miniAppSubtitle"),
                accessibilityID: FabushiAccessibilityIDs.MiniApp.root
            )
            .background(theme.surfacePrimary)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        router.presentedMiniAppId = nil
                    } label: {
                        Image(systemName: "xmark")
                            .fabushiMinimumTouchTarget()
                    }
                    .accessibilityLabel(theme.localized("hig.common.close"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.closeButton)
                }
            }
        }
    }
}

@MainActor
private struct DefaultMiniAppFloatingPipSurface: View {
    let pip: MiniAppPipOverlayState
    let router: ShellNavigationRouter
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        Button {
            router.presentedMiniAppId = pip.id
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "app.badge.fill")
                    .foregroundStyle(theme.accentColor)
                    .accessibilityHidden(true)
                Text(pip.title)
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .fabushiMinimumTouchTarget()
            .fabushiLiquidGlass(cornerRadius: 22, isInteractive: true)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(pip.title)
        .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.floatingPiPPill)
    }
}

@MainActor
private struct DefaultCallPipSurface: View {
    let callPip: CallPipOverlayState
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: callPip.isMuted ? "mic.slash.fill" : "phone.fill")
                .foregroundStyle(theme.successColor)
                .accessibilityHidden(true)
            Text("\(callPip.peerTitle) · \(callPip.stateText)")
                .fabushiFont(.caption)
                .foregroundStyle(theme.textPrimary)
                .lineLimit(1)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .fabushiLiquidGlass(cornerRadius: 22)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(callPip.peerTitle) \(callPip.stateText)")
        .accessibilityIdentifier(FabushiAccessibilityIDs.Call.overlay)
    }
}

@MainActor
private struct DefaultCallsDisclosureSurface: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        VStack(spacing: 16) {
            FabushiEmptyStateView(
                title: theme.localized("hig.tab.calls"),
                systemImage: "phone.fill",
                descriptionText: theme.localized("hig.module.callsDisclosure"),
                accessibilityID: FabushiAccessibilityIDs.Call.sfuDisclosureBanner
            )
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.tab.calls"))
    }
}

// MARK: - Canonical M2a Apple HIG Settings Surface (Real Theme/Accent/WCAG/Language/Capabilities)

@MainActor
public struct FabushiSettingsRootView: View {
    public let initialSection: String?
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(FabushiFeatureModuleRegistry.self) private var injectedRegistry: FabushiFeatureModuleRegistry?
    @State private var searchQuery: String = ""

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var registry: FabushiFeatureModuleRegistry { injectedRegistry ?? .shared }

    public init(
        initialSection: String? = nil,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) {
        self.initialSection = initialSection
        self.coordinator = coordinator
        self.router = router
    }

    private func matchesSearch(_ labels: [String]) -> Bool {
        let query = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else { return true }
        return labels.contains { $0.lowercased().contains(query) }
    }

    public var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(theme.textSecondary)
                    .accessibilityHidden(true)
                TextField(
                    theme.localized("hig.settings.searchPlaceholder"),
                    text: $searchQuery
                )
                .fabushiFont(.body)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityLabel(theme.localized("hig.settings.searchPlaceholder"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.searchField)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(
                theme.surfaceInput,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.searchCornerRadius, style: .continuous)
            )
            .padding(.horizontal, 16)
            .padding(.vertical, 8)

            List {
                if matchesSearch([
                    theme.localized("hig.settings.appearance"),
                    "appearance", "theme", "accent", "contrast", "motion", "外观", "主题",
                ]) {
                    NavigationLink {
                        FabushiAppearanceSettingsView(coordinator: coordinator)
                    } label: {
                        Label(
                            theme.localized("hig.settings.appearance"),
                            systemImage: "paintbrush.fill"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .accessibilityLabel(theme.localized("hig.settings.appearance"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionAppearance)
                }

                if matchesSearch([
                    theme.localized("hig.settings.language"),
                    "language", "locale", "chinese", "english", "语言",
                ]) {
                    NavigationLink {
                        FabushiLanguageSettingsView(coordinator: coordinator)
                    } label: {
                        Label(
                            theme.localized("hig.settings.language"),
                            systemImage: "globe"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .accessibilityLabel(theme.localized("hig.settings.language"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionLanguage)
                }

                if matchesSearch([
                    theme.localized("hig.settings.privacySecurity"),
                    "privacy", "security", "passcode", "vault", "devices", "隐私", "安全",
                ]) {
                    NavigationLink {
                        FabushiPrivacySecurityHubView(coordinator: coordinator)
                    } label: {
                        Label(
                            theme.localized("hig.settings.privacySecurity"),
                            systemImage: "lock.shield.fill"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .accessibilityLabel(theme.localized("hig.settings.privacySecurity"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionPrivacySecurity)
                }

                if matchesSearch([
                    theme.localized("hig.settings.devices"),
                    "devices", "sessions", "设备",
                ]) {
                    NavigationLink {
                        registry.security.makeDevicesSettingsView(coordinator: coordinator)
                    } label: {
                        Label(
                            theme.localized("hig.settings.devices"),
                            systemImage: "laptopcomputer.and.iphone"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .accessibilityLabel(theme.localized("hig.settings.devices"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionDevices)
                }

                if matchesSearch([
                    theme.localized("hig.settings.vault"),
                    "vault", "totp", "凭证",
                ]) {
                    NavigationLink {
                        registry.security.makeVaultSettingsView(coordinator: coordinator)
                    } label: {
                        Label(
                            theme.localized("hig.settings.vault"),
                            systemImage: "key.fill"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .accessibilityLabel(theme.localized("hig.settings.vault"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionVault)
                }

                if matchesSearch([
                    theme.localized("hig.settings.aiProviders"),
                    "ai", "providers", "magpie", "供应商",
                ]) {
                    NavigationLink {
                        registry.agentStudio.makeMagpieAiProvidersSettingsView(
                            coordinator: coordinator,
                            router: router
                        )
                    } label: {
                        Label(
                            theme.localized("hig.settings.aiProviders"),
                            systemImage: "cpu"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .accessibilityLabel(theme.localized("hig.settings.aiProviders"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionAIProviders)
                }

                if matchesSearch([
                    theme.localized("hig.settings.capabilities"),
                    "capabilities", "ledger", "ocrm", "fbcp", "能力",
                ]) {
                    NavigationLink {
                        DefaultCapabilityLedgerView()
                    } label: {
                        Label(
                            theme.localized("hig.settings.capabilities"),
                            systemImage: "checkmark.seal.fill"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .accessibilityLabel(theme.localized("hig.settings.capabilities"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionCapabilities)
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionList)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.tab.settings"))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.root)
    }
}

/// Real Apple HIG Appearance view wired to `FabushiTheme`, WCAG `>138` guard, and Rust `settings.localSet`.
@MainActor
public struct FabushiAppearanceSettingsView: View {
    public let coordinator: FabushiCoordinator
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var accentDraft: String = ""

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(coordinator: FabushiCoordinator) {
        self.coordinator = coordinator
    }

    public var body: some View {
        List {
            Section {
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 120), spacing: 10)],
                    spacing: 10
                ) {
                    ForEach(FabushiThemePreset.allCases) { preset in
                        let isSelected = theme.preset == preset
                        Button {
                            theme.selectPreset(preset)
                            Task { _ = await theme.persistPreferences(using: coordinator) }
                        } label: {
                            HStack(spacing: 8) {
                                Circle()
                                    .fill(Color(fabushiHex: preset.defaultAccentRGB))
                                    .frame(width: 16, height: 16)
                                    .accessibilityHidden(true)
                                Text(theme.localized(preset.localizationKey))
                                    .fabushiFont(.callout)
                                    .fontWeight(isSelected ? .semibold : .regular)
                                    .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textPrimary)
                                    .lineLimit(2)
                            }
                            .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget)
                            .padding(.horizontal, 10)
                            .background(
                                isSelected ? theme.accentColor : theme.surfaceInput,
                                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                            )
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(theme.localized(preset.localizationKey))
                        .accessibilityIdentifier(preset.accessibilityIdentifier)
                    }
                }
                .padding(.vertical, 4)
            } header: {
                FabushiSectionHeader(theme.localized("hig.appearance.colorTheme"))
            }

            Section {
                HStack(spacing: 8) {
                    TextField(
                        theme.localized("hig.appearance.accentHexPlaceholder"),
                        text: $accentDraft
                    )
                    .fabushiFont(.body)
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
                    .accessibilityLabel(theme.localized("hig.appearance.accentHexPlaceholder"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.accentHexField)

                    FabushiGlassPillButton(
                        title: theme.localized("hig.common.apply"),
                        accessibilityID: "fabushi.settings.accentApplyButton",
                        isProminent: true
                    ) {
                        if theme.applyCustomAccentHex(accentDraft) {
                            Task { _ = await theme.persistPreferences(using: coordinator) }
                        }
                    }
                }

                if let errorText = theme.lastAccentValidationError {
                    Text(errorText)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.destructiveColor)
                }

                // Exact E2E `T2_03` contract: accessibilityValue exposes ONLY the exact effective
                // `#RRGGBB` hex (`#A9A906`, `#A9A9A9`, `#007AFF`), paired with human-readable label.
                let contrastStatusLabel = theme.accentContrastResult.wasDarkened
                    ? theme.localized("hig.appearance.contrastAdjusted")
                    : theme.localized("hig.appearance.contrastPass")
                HStack(spacing: 10) {
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(theme.accentColor)
                        .frame(width: 28, height: 28)
                        .overlay(
                            Text("Aa")
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.accentForegroundColor)
                        )
                        .accessibilityHidden(true)

                    VStack(alignment: .leading, spacing: 2) {
                        Text(contrastStatusLabel)
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.textPrimary)
                        Text(theme.effectiveAccentHex)
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textSecondary)
                    }
                    Spacer()
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(contrastStatusLabel)
                .accessibilityValue(theme.effectiveAccentHex)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.accentContrastBadge)

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        ForEach(FabushiAccentSwatch.curated) { swatch in
                            Button {
                                accentDraft = swatch.hexString
                                if theme.applyCustomAccentHex(swatch.hexString) {
                                    Task { _ = await theme.persistPreferences(using: coordinator) }
                                }
                            } label: {
                                Circle()
                                    .fill(Color(fabushiHex: swatch.rgb))
                                    .frame(width: 32, height: 32)
                                    .overlay(
                                        Circle().stroke(
                                            theme.rawAccentHex == swatch.hexString ? theme.textPrimary : .clear,
                                            lineWidth: 2
                                        )
                                    )
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(swatch.name)
                        }
                    }
                }

                if theme.customAccentHex != nil {
                    Button(theme.localized("hig.appearance.resetAccent")) {
                        theme.clearCustomAccent()
                        accentDraft = theme.rawAccentHex
                        Task { _ = await theme.persistPreferences(using: coordinator) }
                    }
                    .fabushiFont(.callout)
                    .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                }
            } header: {
                FabushiSectionHeader(theme.localized("hig.appearance.accentColor"))
            }

            Section {
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Text(theme.localized("hig.appearance.bubbleRadius"))
                            .fabushiFont(.body)
                            .foregroundStyle(theme.textPrimary)
                        Spacer()
                        Text("\(Int(theme.bubbleCornerRadius.rounded())) pt")
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textSecondary)
                    }
                    Slider(
                        value: Binding(
                            get: { Double(theme.bubbleCornerRadius) },
                            set: { newValue in
                                theme.setBubbleCornerRadius(CGFloat(newValue))
                                Task { _ = await theme.persistPreferences(using: coordinator) }
                            }
                        ),
                        in: Double(FabushiTheme.minBubbleCornerRadius)...Double(FabushiTheme.maxBubbleCornerRadius),
                        step: 1
                    )
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        ForEach(FabushiWallpaper.presets) { item in
                            Button {
                                theme.selectWallpaper(item)
                                Task { _ = await theme.persistPreferences(using: coordinator) }
                            } label: {
                                VStack(spacing: 4) {
                                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                                        .fill(item.gradient)
                                        .frame(width: 56, height: 44)
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                                .stroke(
                                                    theme.wallpaper.id == item.id ? theme.accentColor : theme.hairlineBorder,
                                                    lineWidth: theme.wallpaper.id == item.id ? 2 : 0.5
                                                )
                                        )
                                    Text(theme.localized(item.nameKey))
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(theme.textSecondary)
                                        .lineLimit(1)
                                }
                                .frame(minWidth: FabushiLayoutMetrics.minTouchTarget, minHeight: FabushiLayoutMetrics.minTouchTarget)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(theme.localized(item.nameKey))
                        }
                    }
                }
            } header: {
                FabushiSectionHeader(theme.localized("hig.appearance.wallpaper"))
            }

            Section {
                Toggle(
                    isOn: Binding(
                        get: { theme.reduceMotionOverride },
                        set: { enabled in
                            theme.setReduceMotion(enabled)
                            Task { _ = await theme.persistPreferences(using: coordinator) }
                        }
                    )
                ) {
                    Text(theme.localized("hig.appearance.reduceMotion"))
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                }
                .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)
                .accessibilityLabel(theme.localized("hig.appearance.reduceMotion"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.reduceMotionToggle)

                Toggle(
                    isOn: Binding(
                        get: { theme.highContrastOverride },
                        set: { enabled in
                            theme.setHighContrast(enabled)
                            Task { _ = await theme.persistPreferences(using: coordinator) }
                        }
                    )
                ) {
                    Text(theme.localized("hig.appearance.highContrast"))
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                }
                .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)
                .accessibilityLabel(theme.localized("hig.appearance.highContrast"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.highContrastToggle)
            } header: {
                FabushiSectionHeader(theme.localized("hig.appearance.accessibilitySection"))
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.appearance"))
        .onAppear {
            accentDraft = theme.rawAccentHex
        }
    }
}

/// Real Apple HIG Language switcher wired to `FabushiTheme.locale` and Rust `settings.localSet`.
@MainActor
public struct FabushiLanguageSettingsView: View {
    public let coordinator: FabushiCoordinator
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(coordinator: FabushiCoordinator) {
        self.coordinator = coordinator
    }

    public var body: some View {
        List {
            Button {
                theme.setLocale(.zhHans)
                Task { _ = await theme.persistPreferences(using: coordinator) }
            } label: {
                HStack {
                    Text(theme.localized("hig.language.simplifiedChinese"))
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                    Spacer()
                    if theme.locale == .zhHans {
                        Image(systemName: "checkmark")
                            .foregroundStyle(theme.accentColor)
                            .accessibilityHidden(true)
                    }
                }
                .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.localized("hig.language.simplifiedChinese"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.localeZhHansButton)

            Button {
                theme.setLocale(.en)
                Task { _ = await theme.persistPreferences(using: coordinator) }
            } label: {
                HStack {
                    Text(theme.localized("hig.language.english"))
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                    Spacer()
                    if theme.locale == .en {
                        Image(systemName: "checkmark")
                            .foregroundStyle(theme.accentColor)
                            .accessibilityHidden(true)
                    }
                }
                .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.localized("hig.language.english"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.localeEnButton)
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.language"))
    }
}

@MainActor
private struct FabushiPrivacySecurityHubView: View {
    let coordinator: FabushiCoordinator
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(FabushiFeatureModuleRegistry.self) private var injectedRegistry: FabushiFeatureModuleRegistry?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var registry: FabushiFeatureModuleRegistry { injectedRegistry ?? .shared }

    var body: some View {
        List {
            NavigationLink {
                registry.security.makeVaultSettingsView(coordinator: coordinator)
            } label: {
                Label(theme.localized("hig.settings.vault"), systemImage: "key.fill")
                    .fabushiFont(.body)
                    .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
            }

            NavigationLink {
                registry.security.makeDevicesSettingsView(coordinator: coordinator)
            } label: {
                Label(theme.localized("hig.settings.devices"), systemImage: "laptopcomputer.and.iphone")
                    .fabushiFont(.body)
                    .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.privacySecurity"))
    }
}

/// Honest Capabilities Ledger disclosing all 7 canonical upstream-planned / server-blocked IDs.
@MainActor
public struct DefaultCapabilityLedgerView: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init() {}

    public var body: some View {
        List {
            Section {
                Text(theme.localized("hig.capabilities.subtitle"))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.capabilityLedgerBanner)
            }

            Section {
                ForEach(FabushiCapabilityLedger.canonicalItems) { item in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(item.id)
                                .fabushiFont(.headline)
                                .foregroundStyle(theme.textPrimary)
                            Spacer()
                            Text(theme.localized(item.disposition.localizationKey))
                                .fabushiFont(.caption)
                                .foregroundStyle(
                                    item.disposition == .serverBlocked
                                        ? theme.warningColor
                                        : theme.textSecondary
                                )
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(theme.surfaceInput, in: Capsule())
                        }
                        Text(item.summary)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textSecondary)
                    }
                    .padding(.vertical, 4)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("\(item.id), \(theme.localized(item.disposition.localizationKey)). \(item.summary)")
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.capabilities.title"))
        .accessibilityLabel(theme.localized("hig.capabilities.title"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.capabilityLedgerList)
    }
}

// MARK: - Default Auth & Security Surfaces (M2b Handoff Anchors — Zero Fake Login/Unlock)

@MainActor
private struct DefaultAuthGateSurface: View {
    let coordinator: FabushiCoordinator
    let router: ShellNavigationRouter
    let launchConfiguration: FabushiLaunchConfiguration

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var serverURLText: String = ""
    @State private var usernameText: String = ""
    @State private var passwordText: String = ""
    @State private var serverURLError: String?
    @State private var errorMessage: String?
    @State private var isSubmitting = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private struct LoginParams: Encodable, Sendable {
        let username: String
        let password: String
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    VStack(spacing: 8) {
                        Image(systemName: "paperplane.circle.fill")
                            .fabushiFont(.largeTitle)
                            .foregroundStyle(theme.accentColor)
                            .accessibilityHidden(true)
                        Text(theme.localized("hig.auth.title"))
                            .fabushiFont(.title2)
                            .foregroundStyle(theme.textPrimary)
                        Text(theme.localized("hig.auth.subtitle"))
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.textSecondary)
                            .multilineTextAlignment(.center)
                    }
                    .padding(.top, 24)

                    VStack(spacing: 12) {
                        TextField(
                            theme.localized("hig.auth.serverURLPlaceholder"),
                            text: $serverURLText
                        )
                        .fabushiFont(.body)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .padding(12)
                        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .accessibilityLabel(theme.localized("hig.auth.serverURLPlaceholder"))
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.serverURLField)

                        if let serverURLError {
                            Text(serverURLError)
                                .fabushiFont(.footnote)
                                .foregroundStyle(theme.destructiveColor)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .accessibilityLabel(serverURLError)
                                .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.serverURLError)
                        }

                        TextField(
                            theme.localized("hig.auth.usernamePlaceholder"),
                            text: $usernameText
                        )
                        .fabushiFont(.body)
                        .textContentType(.username)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .padding(12)
                        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .accessibilityLabel(theme.localized("hig.auth.usernamePlaceholder"))
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.usernameField)

                        SecureField(
                            theme.localized("hig.auth.passwordPlaceholder"),
                            text: $passwordText
                        )
                        .fabushiFont(.body)
                        .textContentType(.password)
                        .padding(12)
                        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .accessibilityLabel(theme.localized("hig.auth.passwordPlaceholder"))
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.passwordField)
                    }

                    if let errorMessage {
                        Text(errorMessage)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.destructiveColor)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .accessibilityLabel(errorMessage)
                            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.errorBanner)
                    }

                    FabushiGlassPillButton(
                        title: theme.localized("hig.auth.loginButton"),
                        systemImage: "arrow.right.circle.fill",
                        accessibilityID: FabushiAccessibilityIDs.Auth.loginButton,
                        isProminent: true
                    ) {
                        Task { await submitLogin() }
                    }
                    .disabled(isSubmitting)
                }
                .padding(24)
                .frame(maxWidth: 460)
                .frame(maxWidth: .infinity)
            }
            .background(theme.surfacePrimary)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.root)
        .onAppear {
            if serverURLText.isEmpty {
                serverURLText = launchConfiguration.serverURL ?? coordinator.settingsStore.activeServerURL
            }
        }
    }

    private func submitLogin() async {
        serverURLError = nil
        errorMessage = nil

        guard let normalizedURL = FabushiLaunchConfiguration.validateAndNormalizeServerURL(serverURLText) else {
            serverURLError = theme.localized("hig.auth.invalidServerURL")
            return
        }

        let trimmedUser = usernameText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedUser.isEmpty, !passwordText.isEmpty else {
            errorMessage = theme.localized("hig.auth.missingCredentials")
            return
        }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            if normalizedURL != coordinator.settingsStore.activeServerURL {
                _ = try await coordinator.setServerURL(normalizedURL)
            }
            let authResp: FabushiSessionAuthParams = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("login"),
                params: LoginParams(username: trimmedUser, password: passwordText)
            )
            _ = try await coordinator.setAuthSession(authResp)
            router.replayPendingDeepLinkIfReady(coordinator: coordinator)
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

@MainActor
private struct DefaultPasscodeLockSurface: View {
    let coordinator: FabushiCoordinator
    let onUnlock: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var passcodeInput = ""
    @State private var errorText: String?
    @State private var isVerifying = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private struct PasscodeVerifyParams: Encodable, Sendable {
        let passcode: String
    }

    private struct PasscodeVerifyResult: Decodable, Sendable {
        let valid: Bool
        let configured: Bool
    }

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "lock.fill")
                .fabushiFont(.largeTitle)
                .foregroundStyle(theme.accentColor)
                .accessibilityHidden(true)

            Text(theme.localized("hig.auth.passcodeTitle"))
                .fabushiFont(.title2)
                .foregroundStyle(theme.textPrimary)

            SecureField(
                theme.localized("hig.auth.passcodePlaceholder"),
                text: $passcodeInput
            )
            .fabushiFont(.body)
            .padding(12)
            .frame(maxWidth: 280)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityLabel(theme.localized("hig.auth.passcodePlaceholder"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.passcodeField)

            if let errorText {
                Text(errorText)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.destructiveColor)
                    .accessibilityLabel(errorText)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.passcodeErrorBanner)
            }

            FabushiGlassPillButton(
                title: theme.localized("hig.auth.passcodeUnlock"),
                systemImage: "lock.open.fill",
                accessibilityID: FabushiAccessibilityIDs.Auth.passcodeSubmitButton,
                isProminent: true
            ) {
                Task { await verifyPasscode() }
            }
            .disabled(isVerifying)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfaceModal)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(theme.localized("hig.auth.passcodeTitle"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.passcodeOverlay)
    }

    private func verifyPasscode() async {
        errorText = nil
        isVerifying = true
        defer { isVerifying = false }

        do {
            let result: PasscodeVerifyResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("passcodeVerify"),
                params: PasscodeVerifyParams(passcode: passcodeInput)
            )
            if result.valid {
                passcodeInput = ""
                onUnlock()
            } else {
                errorText = theme.localized("hig.auth.passcodeIncorrect")
            }
        } catch {
            errorText = theme.localized("hig.auth.passcodeIncorrect")
        }
    }
}

@MainActor
private struct DefaultPrivacyBlurSurface: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "eye.slash.fill")
                .fabushiFont(.largeTitle)
                .foregroundStyle(theme.textPrimary)
                .accessibilityHidden(true)
            Text(theme.localized("hig.auth.privacyProtected"))
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.ultraThickMaterial)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(theme.localized("hig.auth.privacyProtected"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.privacyBlurOverlay)
    }
}

@MainActor
private struct DefaultAccountSwitcherSurface: View {
    let coordinator: FabushiCoordinator
    let router: ShellNavigationRouter
    let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var accounts: [FabushiAccountSummary] = []

    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        NavigationStack {
            List {
                if accounts.isEmpty {
                    Text(theme.localized("hig.accountSwitcher.empty"))
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(Array(accounts.enumerated()), id: \.element.accountId) { idx, account in
                        Button {
                            Task {
                                if let newGen = try? await coordinator.switchAccount(accountId: account.accountId) {
                                    router.handleAccountScopeChange(generation: newGen, coordinator: coordinator)
                                    onDismiss()
                                }
                            }
                        } label: {
                            HStack {
                                Text(account.username ?? account.accountId)
                                    .fabushiFont(.body)
                                    .foregroundStyle(theme.textPrimary)
                                Spacer()
                                if account.active {
                                    Image(systemName: "checkmark")
                                        .foregroundStyle(theme.accentColor)
                                        .accessibilityHidden(true)
                                }
                            }
                            .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("fabushi.shell.accountSwitcher.row.\(idx)")
                    }
                }

                Section {
                    Button {
                        Task {
                            if let newGen = try? await coordinator.clearLocalSession() {
                                router.handleAccountScopeChange(generation: newGen, coordinator: coordinator)
                                onDismiss()
                            }
                        }
                    } label: {
                        Label(
                            theme.localized("hig.accountSwitcher.addAccount"),
                            systemImage: "person.badge.plus"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    }
                    .accessibilityLabel(theme.localized("hig.accountSwitcher.addAccount"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.addAccountButton)
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle(theme.localized("hig.accountSwitcher.title"))
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        onDismiss()
                    } label: {
                        Image(systemName: "xmark")
                            .fabushiMinimumTouchTarget()
                    }
                    .accessibilityLabel(theme.localized("hig.common.close"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.accountSwitcherCloseButton)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.accountSwitcherSheet)
        .task {
            if coordinator.isStarted,
               let result = try? await coordinator.host.listAccounts(
                   generation: coordinator.accountScopeGeneration
               ) {
                accounts = result.accounts
            }
        }
    }
}

@MainActor
private struct DefaultVaultSettingsSurface: View {
    let coordinator: FabushiCoordinator
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        FabushiEmptyStateView(
            title: theme.localized("hig.settings.vault"),
            systemImage: "key.fill",
            descriptionText: theme.localized("hig.settings.privacySecurity"),
            accessibilityID: FabushiAccessibilityIDs.Settings.sectionVault
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.vault"))
    }
}

@MainActor
private struct DefaultDevicesSettingsSurface: View {
    let coordinator: FabushiCoordinator
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        FabushiEmptyStateView(
            title: theme.localized("hig.settings.devices"),
            systemImage: "laptopcomputer.and.iphone",
            descriptionText: theme.localized("hig.settings.privacySecurity"),
            accessibilityID: FabushiAccessibilityIDs.Settings.devicesList
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.devices"))
    }
}
