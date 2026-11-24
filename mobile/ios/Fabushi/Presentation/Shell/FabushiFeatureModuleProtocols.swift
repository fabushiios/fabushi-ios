import Foundation
import SwiftUI

// MARK: - Parallel Feature Module Registration Protocols (`PROJECT.md § Interface Contracts #3`)

/// Implemented by Milestone M3 (`Presentation/Chats`, `Conversation`, `Composer`, `Inspector`, `Media`, `Contacts`).
@MainActor
public protocol MessagingModuleProviding: AnyObject {
    func makeChatSidebarView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeContactsView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeConversationStageView(
        chatId: Int64?,
        messageId: Int64?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeMultiSelectComposeView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeContextInspectorView(
        target: InspectorTarget,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeUniversalSearchView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeMediaViewerOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeStoriesOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeInstantViewOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView
}

/// Implemented by Milestone M4 (`Presentation/AgentStudio`, `Magpie`, `MCP`, `Coding`).
@MainActor
public protocol AgentStudioModuleProviding: AnyObject {
    func makeAgentStudioWorkspaceView(
        initialTab: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeMagpieAiProvidersSettingsView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeOfficialMcpConsoleView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeCodingAgentThreadAccessories(
        chatId: Int64,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeMultiAgentRoomAccessories(
        chatId: Int64,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView
}

/// Implemented by Milestone M5 (`Presentation/RemoteComputer`, `MiniApps`, `Calls`, `Settings`, `Economy`).
@MainActor
public protocol PlatformModulesProviding: AnyObject {
    func makeRemoteComputerWorkbenchView(
        nodeId: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeRemoteControlledPillView(
        status: RemoteControlStatusState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeMiniAppHostSheetView(
        miniAppId: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeMiniAppFloatingPipView(
        pip: MiniAppPipOverlayState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeCallAndTalkOverlayView(
        callPip: CallPipOverlayState,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeCallsTabView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeSettingsRootView(
        initialSection: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView

    func makeWalletAndPremiumViews(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView
}

/// Implemented by Sub-Milestone M2b (`Presentation/Auth/**`).
@MainActor
public protocol AuthModuleProviding: AnyObject {
    func makeAuthGateView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        launchConfiguration: FabushiLaunchConfiguration
    ) -> AnyView
}

/// Implemented by Sub-Milestone M2b (`Presentation/Security/**`).
@MainActor
public protocol SecurityModuleProviding: AnyObject {
    func makePasscodeLockOverlay(
        coordinator: FabushiCoordinator,
        launchConfiguration: FabushiLaunchConfiguration,
        onUnlock: @escaping () -> Void
    ) -> AnyView

    func makePrivacyBlurOverlay() -> AnyView

    func makeAccountSwitcherSheet(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        onDismiss: @escaping () -> Void
    ) -> AnyView

    func makeVaultSettingsView(
        coordinator: FabushiCoordinator
    ) -> AnyView

    func makeDevicesSettingsView(
        coordinator: FabushiCoordinator
    ) -> AnyView
}

// MARK: - Canonical Honest Capability Ledger (7 Tracked IDs — `TEST_INFRA.md §3.1`)

public enum FabushiCapabilityDisposition: String, Sendable, Equatable {
    case upstreamPlanned = "upstream_planned"
    case serverBlocked = "server_blocked"

    public var localizationKey: String {
        switch self {
        case .upstreamPlanned: return "hig.capabilities.upstreamPlanned"
        case .serverBlocked: return "hig.capabilities.serverBlocked"
        }
    }
}

public struct FabushiTrackedCapabilityItem: Identifiable, Sendable, Equatable {
    public let id: String
    public let disposition: FabushiCapabilityDisposition
    public let summary: String

    public init(id: String, disposition: FabushiCapabilityDisposition, summary: String) {
        self.id = id
        self.disposition = disposition
        self.summary = summary
    }
}

public enum FabushiCapabilityLedger: Sendable {
    public static let canonicalItems: [FabushiTrackedCapabilityItem] = [
        FabushiTrackedCapabilityItem(
            id: "OCRM-001",
            disposition: .upstreamPlanned,
            summary: "141-adapter OpenClaw external channel runtime (upstream planned)"
        ),
        FabushiTrackedCapabilityItem(
            id: "FBCP-001",
            disposition: .upstreamPlanned,
            summary: "Fabushi Control Plane D1 orchestration (upstream planned)"
        ),
        FabushiTrackedCapabilityItem(
            id: "TON_ONCHAIN_SIGNING",
            disposition: .upstreamPlanned,
            summary: "TON on-chain wallet transaction signing (upstream planned)"
        ),
        FabushiTrackedCapabilityItem(
            id: "TELEGRAM_PASSPORT",
            disposition: .upstreamPlanned,
            summary: "Telegram Passport identity vault (upstream planned)"
        ),
        FabushiTrackedCapabilityItem(
            id: "APNS_HTTP2_SERVER_DELIVERY",
            disposition: .serverBlocked,
            summary: "Server apns.rs HTTP/2 push delivery dispatcher (server blocked)"
        ),
        FabushiTrackedCapabilityItem(
            id: "WEBRTC_NAT_SFU_TRANSPORT",
            disposition: .serverBlocked,
            summary: "Server LiveKit/WebRTC NAT/SFU media transport (server blocked)"
        ),
        FabushiTrackedCapabilityItem(
            id: "STOREKIT_SERVER_BINDING",
            disposition: .serverBlocked,
            summary: "App Store StoreKit 2 server receipt/entitlement binding (server blocked)"
        ),
    ]
}
