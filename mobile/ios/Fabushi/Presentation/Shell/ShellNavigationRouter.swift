import Foundation
import Observation
import SwiftUI

// MARK: - Canonical 5-Tab Shell Identifiers (1:1 E2E Accessibility Contract)

/// Primary adaptive shell navigation destinations matching `FabushiAccessibilityIDs.Shell.tab*`.
public enum FabushiShellTab: String, CaseIterable, Hashable, Sendable, Identifiable {
    case chats
    case contacts
    case studio
    case remote
    case settings

    public var id: String { rawValue }

    public var accessibilityIdentifier: String {
        switch self {
        case .chats: return FabushiAccessibilityIDs.Shell.tabChats
        case .contacts: return FabushiAccessibilityIDs.Shell.tabContacts
        case .studio: return FabushiAccessibilityIDs.Shell.tabStudio
        case .remote: return FabushiAccessibilityIDs.Shell.tabRemote
        case .settings: return FabushiAccessibilityIDs.Shell.tabSettings
        }
    }

    public var systemImage: String {
        switch self {
        case .chats: return "bubble.left.and.bubble.right.fill"
        case .contacts: return "person.2.fill"
        case .studio: return "sparkles.rectangle.stack.fill"
        case .remote: return "desktopcomputer"
        case .settings: return "gearshape.fill"
        }
    }

    public var localizationKey: String {
        switch self {
        case .chats: return "hig.tab.chats"
        case .contacts: return "hig.tab.contacts"
        case .studio: return "hig.tab.studio"
        case .remote: return "hig.tab.remote"
        case .settings: return "hig.tab.settings"
        }
    }
}

// MARK: - Inspector & Floating Overlay Contracts (`PROJECT.md § Interface Contracts #3`)

public enum InspectorTarget: Equatable, Hashable, Sendable {
    case user(peerId: Int64)
    case group(chatId: Int64)
    case bot(botId: Int64, chatId: Int64?)
    case chat(chatId: Int64)
}

public struct MiniAppPipOverlayState: Identifiable, Sendable, Equatable {
    public let id: String
    public let title: String
    public let botUsername: String?
    public let statusText: String?

    public init(id: String, title: String, botUsername: String? = nil, statusText: String? = nil) {
        self.id = id
        self.title = title
        self.botUsername = botUsername
        self.statusText = statusText
    }
}

public struct CallPipOverlayState: Identifiable, Sendable, Equatable {
    public let id: String
    public let peerTitle: String
    public let stateText: String
    public let isMuted: Bool

    public init(id: String, peerTitle: String, stateText: String, isMuted: Bool = false) {
        self.id = id
        self.peerTitle = peerTitle
        self.stateText = stateText
        self.isMuted = isMuted
    }
}

public struct RemoteControlStatusState: Identifiable, Sendable, Equatable {
    public let id: String
    public let nodeName: String
    public let controllerLabel: String

    public init(id: String, nodeName: String, controllerLabel: String) {
        self.id = id
        self.nodeName = nodeName
        self.controllerLabel = controllerLabel
    }
}

// MARK: - `fabushi://` Deep Link Parser

public enum ShellDeepLinkDestination: Sendable, Equatable {
    case route(ShellRoute)
    case contacts
    case authCallback(action: String, queryItems: [String: String])
}

public enum ShellDeepLinkParser: Sendable {
    public static func parse(_ url: URL) -> ShellDeepLinkDestination? {
        guard let scheme = url.scheme?.lowercased(), scheme == "fabushi" else {
            return nil
        }

        let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        let host = (components?.host ?? url.host ?? "").lowercased()
        let pathParts = url.pathComponents.filter { $0 != "/" && !$0.isEmpty }
        var queryMap: [String: String] = [:]
        for item in components?.queryItems ?? [] {
            if let value = item.value {
                queryMap[item.name] = value
            }
        }

        let primarySegment: String
        let remainingSegments: [String]
        if !host.isEmpty {
            primarySegment = host
            remainingSegments = pathParts
        } else if let first = pathParts.first?.lowercased() {
            primarySegment = first
            remainingSegments = Array(pathParts.dropFirst())
        } else {
            return nil
        }

        switch primarySegment {
        case "chat", "chats":
            let chatId = remainingSegments.first.flatMap(Int64.init)
                ?? queryMap["chatId"].flatMap(Int64.init)
            let messageId = queryMap["messageId"].flatMap(Int64.init)
                ?? queryMap["message_id"].flatMap(Int64.init)
            return .route(.chats(chatId: chatId, messageId: messageId))

        case "compose", "new":
            return .route(.compose)

        case "contacts":
            return .contacts

        case "studio", "agent-studio", "agentstudio":
            let tab = remainingSegments.first ?? queryMap["tab"]
            return .route(.agentStudio(tab: tab))

        case "calls", "call":
            return .route(.calls)

        case "remote", "computer", "computers":
            let nodeId = remainingSegments.first ?? queryMap["nodeId"] ?? queryMap["node_id"]
            return .route(.remoteComputer(nodeId: nodeId))

        case "miniapp", "miniapps":
            let id = remainingSegments.first ?? queryMap["id"]
            return .route(.miniApp(id: id))

        case "settings":
            let section = remainingSegments.first ?? queryMap["section"]
            return .route(.settings(section: section))

        case "auth":
            let action = remainingSegments.first ?? "callback"
            return .authCallback(action: action, queryItems: queryMap)

        default:
            return nil
        }
    }
}

// MARK: - Unified Adaptive Shell Router

/// Canonical `@MainActor @Observable` router two-way synchronized with `FabushiCoordinator.activeRoute`,
/// managing adaptive sidebar/tab navigation, 3-way inspector state, floating PiP/call/remote overlays,
/// two-layer toast controller, `fabushi://` deep links, and generation-fenced account-switch resets.
@MainActor
@Observable
public final class ShellNavigationRouter {
    public var activeTab: FabushiShellTab = .chats
    public private(set) var activeRoute: ShellRoute = .chats(chatId: nil, messageId: nil)
    public private(set) var lastWorkspaceRoute: ShellRoute = .chats(chatId: nil, messageId: nil)
    public var isSidebarCollapsed: Bool = false

    public private(set) var inspectorTarget: InspectorTarget?
    public var isInspectorPresented: Bool = false {
        didSet {
            if !isInspectorPresented {
                inspectorTarget = nil
            }
        }
    }

    public var isAccountSwitcherPresented: Bool = false
    public var isCallsSheetPresented: Bool = false
    public var presentedMiniAppId: String?

    /// Runtime passcode lock policy and lock state (initialized from `launchConfiguration.requirePasscode`,
    /// writable by M2b Security when user enables/disables passcode at runtime).
    public var isPasscodeLockEnabled: Bool
    public var isPasscodeLocked: Bool

    public var activeMiniAppPip: MiniAppPipOverlayState?
    public var activeCallPip: CallPipOverlayState?
    public var activeRemoteControlStatus: RemoteControlStatusState?

    public var activeDeepLinkBannerText: String?
    public private(set) var pendingDeepLinkURL: URL?
    public private(set) var lastAuthCallbackURL: URL?
    public var activeProviderErrorMessage: String?

    public let toastController: FabushiToastController
    public private(set) var observedGeneration: UInt64 = 1
    public private(set) var lastScenePhase: ScenePhase = .active

    public init(
        toastController: FabushiToastController = FabushiToastController(),
        isPasscodeLockEnabled: Bool = false
    ) {
        self.toastController = toastController
        self.isPasscodeLockEnabled = isPasscodeLockEnabled
        self.isPasscodeLocked = isPasscodeLockEnabled
    }

    public var activeChatId: Int64? {
        if case .chats(let chatId, _) = activeRoute {
            return chatId
        }
        return nil
    }

    // MARK: - Navigation & Coordinator Bidirectional Sync

    public func selectTab(_ tab: FabushiShellTab, coordinator: FabushiCoordinator) {
        let wasAlreadyOnChatsTab = activeTab == .chats
        let previousChatId = activeChatId
        activeTab = tab

        let targetRoute: ShellRoute
        switch tab {
        case .chats:
            // Re-tapping the already-active Chats tab while inside a conversation or compose
            // pops back to the root chat list cleanly (matching native Apple HIG TabView behavior).
            if wasAlreadyOnChatsTab && (previousChatId != nil || activeRoute == .compose) {
                targetRoute = .chats(chatId: nil, messageId: nil)
            } else {
                targetRoute = .chats(chatId: previousChatId, messageId: nil)
            }
        case .contacts:
            // Contacts shares the Chats/Contacts messaging root without dropping active chat state
            // unless switching to a specific peer later.
            return
        case .studio:
            targetRoute = .agentStudio(tab: nil)
        case .remote:
            targetRoute = .remoteComputer(nodeId: nil)
        case .settings:
            targetRoute = .settings(section: nil)
        }

        navigate(to: targetRoute, coordinator: coordinator)
    }

    /// Pops from an active conversation or compose screen back to the root chat list,
    /// synchronously purging any chat-scoped Layer 1 / Layer 2 toasts.
    public func navigateBackToChatList(coordinator: FabushiCoordinator) {
        navigate(to: .chats(chatId: nil, messageId: nil), coordinator: coordinator)
    }

    public func navigate(to route: ShellRoute, coordinator: FabushiCoordinator) {
        let previousChatId = activeChatId
        clearChatScopedToastsIfLeavingChat(previousChatId: previousChatId, destination: route)

        activeRoute = route
        if coordinator.activeRoute != route {
            coordinator.activeRoute = route
        }
        applyTabAndModalState(for: route)
    }

    public func syncFromCoordinator(_ coordinator: FabushiCoordinator) {
        let incoming = coordinator.activeRoute
        guard incoming != activeRoute else { return }

        let previousChatId = activeChatId
        clearChatScopedToastsIfLeavingChat(previousChatId: previousChatId, destination: incoming)

        activeRoute = incoming
        applyTabAndModalState(for: incoming)
    }

    /// Dismisses active modal sheets and restores `activeRoute` & `coordinator.activeRoute`
    /// back to `lastWorkspaceRoute` whenever dismissing `.calls` or `.miniApp`.
    public func dismissActiveModalSheet(
        coordinator: FabushiCoordinator,
        dismissCompactInspector: Bool = false
    ) {
        isAccountSwitcherPresented = false
        presentedMiniAppId = nil
        isCallsSheetPresented = false
        if dismissCompactInspector {
            dismissInspector()
        }

        switch activeRoute {
        case .calls, .miniApp:
            let restored = lastWorkspaceRoute
            activeRoute = restored
            if coordinator.activeRoute != restored {
                coordinator.activeRoute = restored
            }
            applyTabAndModalState(for: restored)
        default:
            break
        }
    }

    public func openInspector(for target: InspectorTarget) {
        inspectorTarget = target
        isInspectorPresented = true
    }

    public func dismissInspector() {
        isInspectorPresented = false
        inspectorTarget = nil
    }

    public func toggleSidebar() {
        isSidebarCollapsed.toggle()
    }

    public func dismissDeepLinkBanner() {
        activeDeepLinkBannerText = nil
    }

    public func dismissProviderErrorCard() {
        activeProviderErrorMessage = nil
    }

    // MARK: - Runtime Passcode Lock Hooks (M2b Security Integration)

    public func engagePasscodeLockIfEnabled() {
        if isPasscodeLockEnabled {
            isPasscodeLocked = true
        }
    }

    @discardableResult
    public func unlockPasscode(coordinator: FabushiCoordinator) -> Bool {
        isPasscodeLocked = false
        return replayPendingDeepLinkIfReady(coordinator: coordinator)
    }

    // MARK: - Deep Link Handling (`fabushi://`)

    @discardableResult
    public func handleOpenURL(_ url: URL, coordinator: FabushiCoordinator) -> Bool {
        guard let destination = ShellDeepLinkParser.parse(url) else {
            return false
        }

        if case .authCallback = destination {
            lastAuthCallbackURL = url
            return true
        }

        let isReadyForShellNavigation = coordinator.isStarted
            && coordinator.authState != .unauthenticated
            && !isPasscodeLocked
        guard isReadyForShellNavigation else {
            pendingDeepLinkURL = url
            return true
        }

        pendingDeepLinkURL = nil
        switch destination {
        case .route(let route):
            navigate(to: route, coordinator: coordinator)
        case .contacts:
            activeTab = .contacts
        case .authCallback:
            break
        }

        activeDeepLinkBannerText = url.absoluteString
        return true
    }

    /// Replays a deferred deep link ONLY when started, authenticated, and NOT passcode-locked.
    /// Reconciles `coordinator.accountScopeGeneration` strictly BEFORE replaying so a delayed
    /// SwiftUI `.onChange(of: coordinator.accountScopeGeneration)` tick can never wipe the
    /// newly replayed route or deep-link banner.
    @discardableResult
    public func replayPendingDeepLinkIfReady(coordinator: FabushiCoordinator) -> Bool {
        guard coordinator.isStarted,
              coordinator.authState != .unauthenticated,
              !isPasscodeLocked
        else {
            return false
        }

        if coordinator.accountScopeGeneration > observedGeneration {
            _ = handleAccountScopeChange(
                generation: coordinator.accountScopeGeneration,
                coordinator: coordinator
            )
        }

        guard let pending = pendingDeepLinkURL else {
            return false
        }
        pendingDeepLinkURL = nil
        return handleOpenURL(pending, coordinator: coordinator)
    }

    // MARK: - Generation-Fenced Account Switch & Logout Reset

    /// Resets transient navigation, inspector, overlay, and toast state when `generation` advances.
    /// Rejects stale generations (`generation <= observedGeneration`) without mutating state.
    /// Preserves `pendingDeepLinkURL` so pre-auth / cold-start deep links can replay immediately after.
    @discardableResult
    public func handleAccountScopeChange(
        generation: UInt64,
        coordinator: FabushiCoordinator
    ) -> Bool {
        guard generation > observedGeneration else {
            return false
        }
        observedGeneration = generation

        toastController.clearAll()
        inspectorTarget = nil
        isInspectorPresented = false
        isAccountSwitcherPresented = false
        isCallsSheetPresented = false
        presentedMiniAppId = nil
        activeMiniAppPip = nil
        activeCallPip = nil
        activeRemoteControlStatus = nil
        activeDeepLinkBannerText = nil
        activeProviderErrorMessage = nil

        activeTab = .chats
        let defaultRoute: ShellRoute = .chats(chatId: nil, messageId: nil)
        activeRoute = defaultRoute
        lastWorkspaceRoute = defaultRoute
        if coordinator.activeRoute != defaultRoute {
            coordinator.activeRoute = defaultRoute
        }
        return true
    }

    /// Preserves floating PiP / Call / Remote Control overlays across background/foreground transitions (`T3_06`).
    public func handleScenePhaseChange(_ phase: ScenePhase) {
        lastScenePhase = phase
    }

    private func clearChatScopedToastsIfLeavingChat(
        previousChatId: Int64?,
        destination route: ShellRoute
    ) {
        guard let old = previousChatId else { return }
        if case .chats(let newChatId, _) = route, newChatId == old {
            return
        }
        toastController.clearToasts(forChatId: old)
    }

    private func applyTabAndModalState(for route: ShellRoute) {
        switch route {
        case .chats, .compose:
            activeTab = .chats
            lastWorkspaceRoute = route
        case .agentStudio:
            activeTab = .studio
            lastWorkspaceRoute = route
        case .remoteComputer:
            activeTab = .remote
            lastWorkspaceRoute = route
        case .settings:
            activeTab = .settings
            lastWorkspaceRoute = route
        case .calls:
            isCallsSheetPresented = true
            presentedMiniAppId = nil
        case .miniApp(let id):
            presentedMiniAppId = id ?? "catalog"
            isCallsSheetPresented = false
        }
    }
}
