import AppIntents
import Foundation
import UIKit

// MARK: - App Intents Route & Action Bus Bridge (`Feature #33`)

/// Notification name posted when a Fabushi App Intent requests an in-app `fabushi://` route.
public enum FabushiAppIntentRouteBridge: Sendable {
    public static let routeRequestedNotification = Notification.Name("com.ombhrum.fabushi.appIntentRouteRequested")
    public static let urlUserInfoKey = "url"

    /// Desktop host-control actions strictly blocked fail-closed in the iOS sandbox
    /// (mirroring `source/host/src/handlers_core.rs:1657-1673`).
    public static let iosSandboxBlockedActions: Set<String> = [
        "device_power_action",
        "device_connect",
        "remote_capture_screen",
        "remote_mouse",
        "remote_mouse_action",
        "remote_keyboard",
        "remote_keyboard_action",
        "remote_disconnect_all",
        "terminal_exec",
        "terminal_list",
    ]

    public static func isBlockedInIOSSandbox(actionName: String) -> Bool {
        iosSandboxBlockedActions.contains(actionName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())
    }

    public static func routeURL(for target: String) -> URL {
        URL(string: target) ?? URL(string: "fabushi://settings")!
    }

    @MainActor
    public static func dispatchRouteURL(_ url: URL) {
        NotificationCenter.default.post(
            name: routeRequestedNotification,
            object: nil,
            userInfo: [urlUserInfoKey: url]
        )
        if let destination = ShellDeepLinkParser.parse(url),
           case .route(let route) = destination
        {
            FabushiCoordinator.shared.activeRoute = route
        }
    }
}

// MARK: - Native Apple App Intents (Xcode 16.4 / Swift 6 Strict Concurrency Compliant)

public struct FabushiOpenRemoteHubIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open Remote Computers"
    public static let description = IntentDescription("Open the Fabushi Remote Computers hub and Tailnet workbench.")
    public static let openAppWhenRun: Bool = true

    @Parameter(title: "Node ID")
    public var nodeId: String?

    public init() {}

    public init(nodeId: String?) {
        self.nodeId = nodeId
    }

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        let target: String
        if let nodeId, !nodeId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            target = "fabushi://remote/\(nodeId.trimmingCharacters(in: .whitespacesAndNewlines))"
        } else {
            target = "fabushi://remote"
        }
        let url = FabushiAppIntentRouteBridge.routeURL(for: target)
        await MainActor.run {
            FabushiAppIntentRouteBridge.dispatchRouteURL(url)
        }
        return .result(dialog: "Opening Fabushi Remote Computers.")
    }
}

public struct FabushiOpenMiniAppsIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open Mini Apps"
    public static let description = IntentDescription("Open the Fabushi Mini Apps catalog or Global Dharma.")
    public static let openAppWhenRun: Bool = true

    @Parameter(title: "Mini App ID")
    public var miniAppId: String?

    public init() {}

    public init(miniAppId: String?) {
        self.miniAppId = miniAppId
    }

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        let trimmed = miniAppId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "catalog"
        let resolvedId = trimmed.isEmpty ? "catalog" : trimmed
        let url = FabushiAppIntentRouteBridge.routeURL(for: "fabushi://miniapp/\(resolvedId)")
        await MainActor.run {
            FabushiAppIntentRouteBridge.dispatchRouteURL(url)
        }
        return .result(dialog: "Opening Fabushi Mini Apps.")
    }
}

public struct FabushiOpenCallsIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open Calls & AI Talk"
    public static let description = IntentDescription("Open Fabushi Voice/Video Calls and Full-Duplex AI Talk Mode.")
    public static let openAppWhenRun: Bool = true

    public init() {}

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        let url = FabushiAppIntentRouteBridge.routeURL(for: "fabushi://calls")
        await MainActor.run {
            FabushiAppIntentRouteBridge.dispatchRouteURL(url)
        }
        return .result(dialog: "Opening Fabushi Calls and AI Talk Mode.")
    }
}

public struct FabushiOpenSettingsIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open Fabushi Settings"
    public static let description = IntentDescription("Open a specific section in Fabushi Settings.")
    public static let openAppWhenRun: Bool = true

    @Parameter(title: "Section")
    public var section: String?

    public init() {}

    public init(section: String?) {
        self.section = section
    }

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        let trimmed = section?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let target = trimmed.isEmpty ? "fabushi://settings" : "fabushi://settings/\(trimmed)"
        let url = FabushiAppIntentRouteBridge.routeURL(for: target)
        await MainActor.run {
            FabushiAppIntentRouteBridge.dispatchRouteURL(url)
        }
        return .result(dialog: "Opening Fabushi Settings.")
    }
}

public struct FabushiOpenBuiltInBrowserIntent: AppIntent {
    public static let title: LocalizedStringResource = "Open Built-in Browser"
    public static let description = IntentDescription("Open the Fabushi Built-in Interactive Browser & Automation App Store.")
    public static let openAppWhenRun: Bool = true

    public init() {}

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        // Routes cleanly through Settings Built-in Browser section until M6 wires `fabushi://browser`.
        let url = FabushiAppIntentRouteBridge.routeURL(for: "fabushi://settings/browser")
        await MainActor.run {
            FabushiAppIntentRouteBridge.dispatchRouteURL(url)
        }
        return .result(dialog: "Opening Fabushi Built-in Browser.")
    }
}

public enum FabushiActionBusIntentError: LocalizedError, Sendable {
    case emptyActionName
    case blockedInIOSSandbox(String)
    case hostNotStarted

    public var errorDescription: String? {
        switch self {
        case .emptyActionName:
            return "Action name cannot be empty."
        case .blockedInIOSSandbox(let name):
            return "Action '\(name)' requires desktop host-control capabilities unavailable in the iOS sandbox."
        case .hostNotStarted:
            return "Fabushi host is not started yet."
        }
    }
}

public struct FabushiRunActionBusIntent: AppIntent {
    public static let title: LocalizedStringResource = "Run Fabushi Action"
    public static let description = IntentDescription("Execute a typed Action Bus command in Fabushi with iOS sandbox protection.")
    public static let openAppWhenRun: Bool = false

    @Parameter(title: "Action Name", default: "")
    public var actionName: String

    public init() {}

    public init(actionName: String) {
        self.actionName = actionName
    }

    public func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let cleanName = actionName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanName.isEmpty else {
            throw FabushiActionBusIntentError.emptyActionName
        }
        guard !FabushiAppIntentRouteBridge.isBlockedInIOSSandbox(actionName: cleanName) else {
            throw FabushiActionBusIntentError.blockedInIOSSandbox(cleanName)
        }

        let jsonString = try await Self.executeActionOnMainActor(name: cleanName)
        return .result(value: jsonString, dialog: "Executed \(cleanName).")
    }

    @MainActor
    private static func executeActionOnMainActor(name: String) async throws -> String {
        let coordinator = FabushiCoordinator.shared
        guard coordinator.isStarted else {
            throw FabushiActionBusIntentError.hostNotStarted
        }

        struct ExecuteParams: Encodable, Sendable {
            let name: String
            let input: FabushiJSONValue
            let approved: Bool
        }

        let response: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.actions.method("execute"),
            params: ExecuteParams(name: name, input: .object([:]), approved: false)
        )
        return response.toJSONString()
    }
}

// MARK: - Single Canonical AppShortcutsProvider (Every phrase MUST include `\(.applicationName)`)

public struct FabushiAppShortcutsProvider: AppShortcutsProvider {
    public static var shortcutTileColor: ShortcutTileColor { .blue }

    public static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: FabushiOpenRemoteHubIntent(),
            phrases: [
                "Open remote computers in \(.applicationName)",
                "Show remote desktop in \(.applicationName)"
            ],
            shortTitle: "Remote Computers",
            systemImageName: "desktopcomputer"
        )
        AppShortcut(
            intent: FabushiOpenMiniAppsIntent(),
            phrases: [
                "Open mini apps in \(.applicationName)",
                "Launch Global Dharma in \(.applicationName)"
            ],
            shortTitle: "Mini Apps",
            systemImageName: "square.grid.2x2"
        )
        AppShortcut(
            intent: FabushiOpenCallsIntent(),
            phrases: [
                "Start AI Talk Mode in \(.applicationName)",
                "Open calls in \(.applicationName)"
            ],
            shortTitle: "Calls & AI Talk",
            systemImageName: "waveform.and.mic"
        )
        AppShortcut(
            intent: FabushiOpenBuiltInBrowserIntent(),
            phrases: [
                "Open built-in browser in \(.applicationName)",
                "Open automation apps in \(.applicationName)"
            ],
            shortTitle: "Built-in Browser",
            systemImageName: "safari"
        )
        AppShortcut(
            intent: FabushiOpenSettingsIntent(),
            phrases: [
                "Open settings in \(.applicationName)"
            ],
            shortTitle: "Settings",
            systemImageName: "gearshape"
        )
    }
}
