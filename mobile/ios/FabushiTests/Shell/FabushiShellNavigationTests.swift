import Foundation
import SwiftUI
import XCTest
@testable import Fabushi

@MainActor
final class FabushiShellNavigationTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_shell_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    func testCanonicalFiveShellTabsAndBidirectionalRouteSync() {
        XCTAssertEqual(
            FabushiShellTab.allCases.map(\.accessibilityIdentifier),
            [
                "fabushi.shell.tab.chats",
                "fabushi.shell.tab.contacts",
                "fabushi.shell.tab.studio",
                "fabushi.shell.tab.remote",
                "fabushi.shell.tab.settings",
            ]
        )

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor())
        let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))

        router.selectTab(.studio, coordinator: coordinator)
        XCTAssertEqual(router.activeTab, .studio)
        XCTAssertEqual(router.activeRoute, .agentStudio(tab: nil))
        XCTAssertEqual(coordinator.activeRoute, .agentStudio(tab: nil))

        router.selectTab(.remote, coordinator: coordinator)
        XCTAssertEqual(router.activeTab, .remote)
        XCTAssertEqual(coordinator.activeRoute, .remoteComputer(nodeId: nil))

        router.selectTab(.settings, coordinator: coordinator)
        XCTAssertEqual(router.activeTab, .settings)
        XCTAssertEqual(coordinator.activeRoute, .settings(section: nil))

        router.navigate(to: .calls, coordinator: coordinator)
        XCTAssertTrue(router.isCallsSheetPresented)
        XCTAssertEqual(coordinator.activeRoute, .calls)

        router.navigate(to: .miniApp(id: "global_dharma"), coordinator: coordinator)
        XCTAssertEqual(router.presentedMiniAppId, "global_dharma")
        XCTAssertEqual(coordinator.activeRoute, .miniApp(id: "global_dharma"))

        // Reverse sync: coordinator.activeRoute -> router
        coordinator.activeRoute = .chats(chatId: 505, messageId: 909)
        router.syncFromCoordinator(coordinator)
        XCTAssertEqual(router.activeRoute, .chats(chatId: 505, messageId: 909))
        XCTAssertEqual(router.activeTab, .chats)
        XCTAssertEqual(router.activeChatId, 505)
    }

    func testDeepLinkParsingRoutingAndDeferredReplayAfterAuth() async throws {
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://chat/130?messageId=42"))),
            .route(.chats(chatId: 130, messageId: 42))
        )
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://compose"))),
            .route(.compose)
        )
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://contacts"))),
            .contacts
        )
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://studio/kanban"))),
            .route(.agentStudio(tab: "kanban"))
        )
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://calls"))),
            .route(.calls)
        )
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://remote/mac_prod_v168"))),
            .route(.remoteComputer(nodeId: "mac_prod_v168"))
        )
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://miniapp/global_dharma"))),
            .route(.miniApp(id: "global_dharma"))
        )
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://settings/appearance"))),
            .route(.settings(section: "appearance"))
        )
        XCTAssertEqual(
            ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "fabushi://auth/alipay?code=ok"))),
            .authCallback(action: "alipay", queryItems: ["code": "ok"])
        )
        XCTAssertNil(ShellDeepLinkParser.parse(try XCTUnwrap(URL(string: "https://example.com/chat/1"))))

        // Real coordinator deferred replay test
        let dir = try makeTempDir("deeplink_replay")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.shell.\(UUID().uuidString)",
            account: "shell-test"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)

        do {
            let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
            let remoteURL = try XCTUnwrap(URL(string: "fabushi://remote/mac_prod_v168"))

            // While unauthenticated, deep link is queued instead of bypassing auth gate
            XCTAssertTrue(router.handleOpenURL(remoteURL, coordinator: coordinator))
            XCTAssertEqual(router.pendingDeepLinkURL, remoteURL)
            XCTAssertEqual(router.activeRoute, .chats(chatId: nil, messageId: nil))

            // Authenticate via real coordinator session commit, then replay
            _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 777,
                username: "shell_deeplink_user",
                sessionId: "sess-dl-777",
                token: "tok-dl-777"
            ))
            XCTAssertTrue(router.replayPendingDeepLinkIfReady(coordinator: coordinator))
            XCTAssertNil(router.pendingDeepLinkURL)
            XCTAssertEqual(router.activeRoute, .remoteComputer(nodeId: "mac_prod_v168"))
            XCTAssertEqual(router.activeTab, .remote)
            XCTAssertEqual(router.activeDeepLinkBannerText, "fabushi://remote/mac_prod_v168")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    func testGenerationFencedAccountScopeResetRejectsStaleAndClearsTransientState() {
        let coordinator = FabushiCoordinator(host: FabushiRustHostActor())
        let toasts = FabushiToastController(autoDismissEnabled: false)
        let router = ShellNavigationRouter(toastController: toasts)

        router.navigate(to: .chats(chatId: 10, messageId: nil), coordinator: coordinator)
        router.openInspector(for: .bot(botId: 99, chatId: 10))
        router.isAccountSwitcherPresented = true
        router.isCallsSheetPresented = true
        router.presentedMiniAppId = "dharma"
        router.activeMiniAppPip = MiniAppPipOverlayState(id: "dharma", title: "Dharma")
        router.activeCallPip = CallPipOverlayState(id: "call-1", peerTitle: "Peer", stateText: "Active")
        router.activeRemoteControlStatus = RemoteControlStatusState(
            id: "node-1",
            nodeName: "Mac Studio",
            controllerLabel: "Connected"
        )
        router.activeDeepLinkBannerText = "fabushi://chat/10"
        router.activeProviderErrorMessage = "PROVIDER_ERROR"
        toasts.showStagePill("Copied", chatId: 10)
        toasts.showWindowToast("Rate limited", chatId: 10)

        // Stale generation (<= 1) MUST be ignored without mutating state
        XCTAssertFalse(router.handleAccountScopeChange(generation: 1, coordinator: coordinator))
        XCTAssertTrue(router.isInspectorPresented)
        XCTAssertNotNil(router.activeMiniAppPip)
        XCTAssertNotNil(toasts.stagePill)
        XCTAssertNotNil(toasts.windowBanner)

        // Newer generation (2) MUST clear every transient overlay, inspector, toast, and reset route
        XCTAssertTrue(router.handleAccountScopeChange(generation: 2, coordinator: coordinator))
        XCTAssertEqual(router.observedGeneration, 2)
        XCTAssertFalse(router.isInspectorPresented)
        XCTAssertNil(router.inspectorTarget)
        XCTAssertFalse(router.isAccountSwitcherPresented)
        XCTAssertFalse(router.isCallsSheetPresented)
        XCTAssertNil(router.presentedMiniAppId)
        XCTAssertNil(router.activeMiniAppPip)
        XCTAssertNil(router.activeCallPip)
        XCTAssertNil(router.activeRemoteControlStatus)
        XCTAssertNil(router.activeDeepLinkBannerText)
        XCTAssertNil(router.activeProviderErrorMessage)
        XCTAssertNil(toasts.stagePill)
        XCTAssertNil(toasts.windowBanner)
        XCTAssertEqual(router.activeRoute, .chats(chatId: nil, messageId: nil))
        XCTAssertEqual(coordinator.activeRoute, .chats(chatId: nil, messageId: nil))

        // Older generation (2) after advance MUST again be rejected
        XCTAssertFalse(router.handleAccountScopeChange(generation: 2, coordinator: coordinator))
    }

    func testTwoLayerToastScopingFifoDedupErrorDetectionAndChatSwitchPurge() {
        let controller = FabushiToastController(autoDismissEnabled: false)

        // Error auto-detection (upstream v1.6.8 / v1.6.9 tokens)
        XCTAssertTrue(FabushiToastController.isErrorText("PROVIDER_ERROR: quota exceeded"))
        XCTAssertTrue(FabushiToastController.isErrorText("Error: unauthorized"))
        XCTAssertTrue(FabushiToastController.isErrorText("发送失败"))
        XCTAssertTrue(FabushiToastController.isErrorText("发生错误"))
        XCTAssertTrue(FabushiToastController.isErrorText("Request failed"))
        XCTAssertFalse(FabushiToastController.isErrorText("Message copied"))

        // Layer 1 stage pill + generation fence
        controller.showStagePill("Copied\nline two", chatId: 100)
        XCTAssertEqual(controller.stagePill?.message, "Copied line two")
        let gen1 = controller.stageGeneration
        controller.showStagePill("Pinned", chatId: 100)
        XCTAssertGreaterThan(controller.stageGeneration, gen1)
        XCTAssertEqual(controller.stagePill?.message, "Pinned")

        // Layer 2 FIFO queue + consecutive duplicate spam dedup
        controller.showWindowToast("PROVIDER_ERROR: timeout", chatId: 100)
        controller.showWindowToast("PROVIDER_ERROR: timeout", chatId: 100) // Duplicate suppressed!
        XCTAssertEqual(controller.windowBanner?.isError, true)
        XCTAssertTrue(controller.windowQueue.isEmpty)

        controller.showWindowToast("Second chat 100 notice", chatId: 100)
        controller.showWindowToast("Global notice", chatId: nil)
        XCTAssertEqual(controller.windowQueue.count, 2)

        // Stale ID dismissal MUST NOT dismiss active banner
        controller.dismissWindowToast(id: UUID())
        XCTAssertEqual(controller.windowBanner?.message, "PROVIDER_ERROR: timeout")

        // Switching away from chat 100 purges BOTH active AND queued chat-100 toasts + stage pill,
        // promoting the unrelated global toast cleanly without leaking chat-100 queued items!
        controller.clearToasts(forChatId: 100)
        XCTAssertNil(controller.stagePill)
        XCTAssertEqual(controller.windowBanner?.message, "Global notice")
        XCTAssertNil(controller.windowBanner?.scopeChatId)
        XCTAssertTrue(controller.windowQueue.isEmpty)

        // Explicit close dismisses remaining banner
        controller.dismissWindowToast(id: controller.windowBanner?.id)
        XCTAssertNil(controller.windowBanner)

        // Router navigation from chat A -> chat B triggers synchronous chat-A toast purge
        let coordinator = FabushiCoordinator(host: FabushiRustHostActor())
        let router = ShellNavigationRouter(toastController: controller)
        router.navigate(to: .chats(chatId: 201, messageId: nil), coordinator: coordinator)
        controller.showStagePill("Chat 201 pill", chatId: 201)
        controller.showWindowToast("Chat 201 error", chatId: 201)

        router.navigate(to: .chats(chatId: 202, messageId: nil), coordinator: coordinator)
        XCTAssertNil(controller.stagePill)
        XCTAssertNil(controller.windowBanner)
    }

    func testFloatingOverlaysPersistAcrossScenePhaseTransitions() {
        let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
        let miniAppPip = MiniAppPipOverlayState(id: "global_dharma", title: "Global Dharma")
        let callPip = CallPipOverlayState(id: "call-42", peerTitle: "Alice", stateText: "01:12")
        let remoteStatus = RemoteControlStatusState(
            id: "mac_prod_v168",
            nodeName: "MacBook Pro",
            controllerLabel: "Controlled"
        )

        router.activeMiniAppPip = miniAppPip
        router.activeCallPip = callPip
        router.activeRemoteControlStatus = remoteStatus

        router.handleScenePhaseChange(.inactive)
        router.handleScenePhaseChange(.background)
        router.handleScenePhaseChange(.active)

        XCTAssertEqual(router.activeMiniAppPip, miniAppPip)
        XCTAssertEqual(router.activeCallPip, callPip)
        XCTAssertEqual(router.activeRemoteControlStatus, remoteStatus)
        XCTAssertEqual(router.lastScenePhase, .active)
    }

    func testFeatureModuleRegistryCustomRegistrationAndHonestCapabilityLedger() {
        let registry = FabushiFeatureModuleRegistry()
        let coordinator = FabushiCoordinator(host: FabushiRustHostActor())
        let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))

        let customMessaging = SpyMessagingProvider()
        let customStudio = SpyAgentStudioProvider()
        let customPlatform = SpyPlatformProvider()
        let customAuth = SpyAuthProvider()
        let customSecurity = SpySecurityProvider()

        registry.register(messaging: customMessaging)
        registry.register(agentStudio: customStudio)
        registry.register(platform: customPlatform)
        registry.register(auth: customAuth)
        registry.register(security: customSecurity)

        _ = registry.messaging.makeChatSidebarView(coordinator: coordinator, router: router)
        _ = registry.messaging.makeContactsView(coordinator: coordinator, router: router)
        _ = registry.messaging.makeConversationStageView(chatId: 1, messageId: nil, coordinator: coordinator, router: router)
        _ = registry.messaging.makeMultiSelectComposeView(coordinator: coordinator, router: router)
        _ = registry.messaging.makeContextInspectorView(target: .chat(chatId: 1), coordinator: coordinator, router: router)
        _ = registry.messaging.makeUniversalSearchView(coordinator: coordinator, router: router)
        XCTAssertEqual(customMessaging.callCount, 6)

        _ = registry.agentStudio.makeAgentStudioWorkspaceView(initialTab: "kanban", coordinator: coordinator, router: router)
        _ = registry.agentStudio.makeMagpieAiProvidersSettingsView(coordinator: coordinator, router: router)
        _ = registry.agentStudio.makeOfficialMcpConsoleView(coordinator: coordinator, router: router)
        XCTAssertEqual(customStudio.callCount, 3)

        _ = registry.platform.makeRemoteComputerWorkbenchView(nodeId: "n1", coordinator: coordinator, router: router)
        _ = registry.platform.makeMiniAppHostSheetView(miniAppId: "m1", coordinator: coordinator, router: router)
        _ = registry.platform.makeCallsTabView(coordinator: coordinator, router: router)
        _ = registry.platform.makeSettingsRootView(initialSection: nil, coordinator: coordinator, router: router)
        XCTAssertEqual(customPlatform.callCount, 4)

        _ = registry.auth.makeAuthGateView(coordinator: coordinator, router: router, launchConfiguration: .current)
        XCTAssertEqual(customAuth.callCount, 1)

        _ = registry.security.makePrivacyBlurOverlay()
        _ = registry.security.makeVaultSettingsView(coordinator: coordinator)
        _ = registry.security.makeDevicesSettingsView(coordinator: coordinator)
        XCTAssertEqual(customSecurity.callCount, 3)

        registry.resetToDefaults()
        XCTAssertTrue(registry.messaging is DefaultMessagingModuleProvider)
        XCTAssertTrue(registry.agentStudio is DefaultAgentStudioModuleProvider)
        XCTAssertTrue(registry.platform is DefaultPlatformModulesProvider)
        XCTAssertTrue(registry.auth is DefaultAuthModuleProvider)
        XCTAssertTrue(registry.security is DefaultSecurityModuleProvider)

        // Verify exact 7 canonical tracked capability IDs (`TEST_INFRA.md §3.1`)
        let ids = FabushiCapabilityLedger.canonicalItems.map(\.id)
        XCTAssertEqual(ids, [
            "OCRM-001",
            "FBCP-001",
            "TON_ONCHAIN_SIGNING",
            "TELEGRAM_PASSPORT",
            "APNS_HTTP2_SERVER_DELIVERY",
            "WEBRTC_NAT_SFU_TRANSPORT",
            "STOREKIT_SERVER_BINDING",
        ])
    }

    func testDeferredDeepLinkSurvivesDelayedAccountScopeGenerationChangeAfterLogin() async throws {
        let dir = try makeTempDir("deeplink_gen_fence")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.shell.genfence.\(UUID().uuidString)",
            account: "shell-genfence"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)

        do {
            let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
            let remoteURL = try XCTUnwrap(URL(string: "fabushi://remote/mac_prod_v168"))
            XCTAssertEqual(router.observedGeneration, 1)
            XCTAssertEqual(coordinator.accountScopeGeneration, 1)

            // Queue deep link pre-auth
            XCTAssertTrue(router.handleOpenURL(remoteURL, coordinator: coordinator))
            XCTAssertEqual(router.pendingDeepLinkURL, remoteURL)

            // Sign in advances coordinator.accountScopeGeneration (1 -> 2)
            _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 801,
                username: "gen_fence_user",
                sessionId: "sess-gf-801",
                token: "tok-gf-801"
            ))
            let postLoginGeneration = coordinator.accountScopeGeneration
            XCTAssertGreaterThan(postLoginGeneration, 1)

            // Replay reconciles generation fence strictly BEFORE navigating
            XCTAssertTrue(router.replayPendingDeepLinkIfReady(coordinator: coordinator))
            XCTAssertEqual(router.observedGeneration, postLoginGeneration)
            XCTAssertEqual(router.activeRoute, .remoteComputer(nodeId: "mac_prod_v168"))
            XCTAssertEqual(router.activeDeepLinkBannerText, "fabushi://remote/mac_prod_v168")

            // Delayed SwiftUI `.onChange(of: coordinator.accountScopeGeneration)` tick arrives AFTER replay
            XCTAssertFalse(
                router.handleAccountScopeChange(generation: postLoginGeneration, coordinator: coordinator),
                "Delayed generation tick must be rejected as already observed"
            )
            XCTAssertEqual(
                router.activeRoute,
                .remoteComputer(nodeId: "mac_prod_v168"),
                "Delayed accountScopeGeneration tick must NOT wipe replayed deep-link route"
            )
            XCTAssertEqual(
                coordinator.activeRoute,
                .remoteComputer(nodeId: "mac_prod_v168")
            )
            XCTAssertEqual(
                router.activeDeepLinkBannerText,
                "fabushi://remote/mac_prod_v168",
                "Delayed accountScopeGeneration tick must NOT wipe activeDeepLinkBannerText"
            )

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    func testDeferredDeepLinkBlockedWhilePasscodeLockedAndReplayedOnUnlock() async throws {
        let dir = try makeTempDir("deeplink_passcode")
        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.shell.passcode.\(UUID().uuidString)",
            account: "shell-passcode"
        )
        defer {
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        try await coordinator.start(appDataDirectory: dir, testMode: true, startNetworkMonitor: false)

        do {
            let router = ShellNavigationRouter(
                toastController: FabushiToastController(autoDismissEnabled: false),
                isPasscodeLockEnabled: true
            )
            XCTAssertTrue(router.isPasscodeLockEnabled)
            XCTAssertTrue(router.isPasscodeLocked)

            let miniAppURL = try XCTUnwrap(URL(string: "fabushi://miniapp/global_dharma"))
            XCTAssertTrue(router.handleOpenURL(miniAppURL, coordinator: coordinator))
            XCTAssertEqual(router.pendingDeepLinkURL, miniAppURL)

            // Authenticate while passcode lock is engaged
            _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 802,
                username: "passcode_lock_user",
                sessionId: "sess-pc-802",
                token: "tok-pc-802"
            ))

            // Replay MUST remain blocked while passcode-locked (zero modal sheet bypass!)
            XCTAssertFalse(router.replayPendingDeepLinkIfReady(coordinator: coordinator))
            XCTAssertEqual(router.pendingDeepLinkURL, miniAppURL)
            XCTAssertNil(router.presentedMiniAppId)
            XCTAssertEqual(router.activeRoute, .chats(chatId: nil, messageId: nil))

            // A deep link arriving while already authenticated & locked MUST also defer cleanly
            let settingsURL = try XCTUnwrap(URL(string: "fabushi://settings/ai_providers"))
            XCTAssertTrue(router.handleOpenURL(settingsURL, coordinator: coordinator))
            XCTAssertEqual(router.pendingDeepLinkURL, settingsURL)
            XCTAssertEqual(router.activeRoute, .chats(chatId: nil, messageId: nil))

            // Unlocking passcode replays deferred deep link AND reconciles generation cleanly
            XCTAssertTrue(router.unlockPasscode(coordinator: coordinator))
            XCTAssertFalse(router.isPasscodeLocked)
            XCTAssertNil(router.pendingDeepLinkURL)
            XCTAssertEqual(router.activeRoute, .settings(section: "ai_providers"))
            XCTAssertEqual(router.activeTab, .settings)
            XCTAssertEqual(router.activeDeepLinkBannerText, "fabushi://settings/ai_providers")

            // Runtime passcode lock hook re-engages cleanly when enabled, and no-ops when disabled
            router.engagePasscodeLockIfEnabled()
            XCTAssertTrue(router.isPasscodeLocked)
            router.isPasscodeLockEnabled = false
            router.isPasscodeLocked = false
            router.engagePasscodeLockIfEnabled()
            XCTAssertFalse(router.isPasscodeLocked)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    func testChatScopedToastsClearedWhenNavigatingToNonChatRoutesAndCompactBack() {
        let controller = FabushiToastController(autoDismissEnabled: false)
        let coordinator = FabushiCoordinator(host: FabushiRustHostActor())
        let router = ShellNavigationRouter(toastController: controller)

        // 1. Chat 100 -> Settings -> Chat 200 MUST purge Chat 100 active & queued toasts
        router.navigate(to: .chats(chatId: 100, messageId: nil), coordinator: coordinator)
        controller.showStagePill("Chat 100 pill", chatId: 100)
        controller.showWindowToast("Chat 100 active error", chatId: 100)
        controller.showWindowToast("Chat 100 queued notice", chatId: 100)
        controller.showWindowToast("Global system notice", chatId: nil)

        // Alternating duplicate `[A, B, A]` inside queue must also be deduplicated
        controller.showWindowToast("Chat 100 queued notice", chatId: 100)
        XCTAssertEqual(controller.windowQueue.count, 2)

        router.navigate(to: .settings(section: nil), coordinator: coordinator)
        XCTAssertNil(controller.stagePill, "Leaving chat 100 to Settings must clear chat-100 stage pill")
        XCTAssertEqual(controller.windowBanner?.message, "Global system notice")
        XCTAssertTrue(controller.windowQueue.isEmpty, "Chat 100 queued toasts must be purged on leaving chat")

        // Entering chat 200 sees ZERO leaked chat-100 toasts
        controller.dismissWindowToast(id: controller.windowBanner?.id)
        router.navigate(to: .chats(chatId: 200, messageId: nil), coordinator: coordinator)
        XCTAssertNil(controller.stagePill)
        XCTAssertNil(controller.windowBanner)

        // 2. Coordinator reverse sync to non-chat route (.agentStudio) also clears active chat toasts
        controller.showStagePill("Chat 200 pill", chatId: 200)
        controller.showWindowToast("Chat 200 toast", chatId: 200)
        coordinator.activeRoute = .agentStudio(tab: "kanban")
        router.syncFromCoordinator(coordinator)
        XCTAssertNil(controller.stagePill)
        XCTAssertNil(controller.windowBanner)

        // 3. Compact Back button (`navigateBackToChatList`) returns to chat list & clears toasts
        router.navigate(to: .chats(chatId: 300, messageId: nil), coordinator: coordinator)
        controller.showStagePill("Chat 300 pill", chatId: 300)
        controller.showWindowToast("Chat 300 error", chatId: 300)
        router.navigateBackToChatList(coordinator: coordinator)
        XCTAssertEqual(router.activeRoute, .chats(chatId: nil, messageId: nil))
        XCTAssertEqual(coordinator.activeRoute, .chats(chatId: nil, messageId: nil))
        XCTAssertNil(router.activeChatId)
        XCTAssertNil(controller.stagePill)
        XCTAssertNil(controller.windowBanner)

        // 4. Re-tapping already-active `.chats` tab pops back to chat list & clears toasts
        router.navigate(to: .chats(chatId: 400, messageId: nil), coordinator: coordinator)
        controller.showStagePill("Chat 400 pill", chatId: 400)
        router.selectTab(.chats, coordinator: coordinator)
        XCTAssertEqual(router.activeRoute, .chats(chatId: nil, messageId: nil))
        XCTAssertNil(controller.stagePill)

        // 5. Non-finite / non-positive durations fall back safely without crashing
        let autoController = FabushiToastController(autoDismissEnabled: true)
        autoController.showStagePill("NaN safe pill", chatId: nil, durationSeconds: .nan)
        XCTAssertEqual(autoController.stagePill?.message, "NaN safe pill")
        autoController.showWindowToast("Infinity safe toast", chatId: nil, durationSeconds: .infinity)
        XCTAssertEqual(autoController.windowBanner?.durationSeconds, FabushiToastController.defaultWindowDurationSeconds)
        autoController.clearAll()
    }

    func testModalCallsAndMiniAppSheetDismissalRestoresLastWorkspaceRoute() {
        let coordinator = FabushiCoordinator(host: FabushiRustHostActor())
        let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))

        // Workspace route #1: active chat 42 -> Calls modal sheet -> dismiss restores chat 42
        router.navigate(to: .chats(chatId: 42, messageId: 99), coordinator: coordinator)
        XCTAssertEqual(router.lastWorkspaceRoute, .chats(chatId: 42, messageId: 99))

        router.navigate(to: .calls, coordinator: coordinator)
        XCTAssertTrue(router.isCallsSheetPresented)
        XCTAssertNil(router.presentedMiniAppId)
        XCTAssertEqual(router.activeRoute, .calls)
        XCTAssertEqual(router.lastWorkspaceRoute, .chats(chatId: 42, messageId: 99))

        router.dismissActiveModalSheet(coordinator: coordinator)
        XCTAssertFalse(router.isCallsSheetPresented)
        XCTAssertEqual(router.activeRoute, .chats(chatId: 42, messageId: 99))
        XCTAssertEqual(coordinator.activeRoute, .chats(chatId: 42, messageId: 99))

        // Workspace route #2: Remote Computer -> MiniApp sheet -> dismiss restores Remote Computer
        router.navigate(to: .remoteComputer(nodeId: "mac_prod_v168"), coordinator: coordinator)
        XCTAssertEqual(router.lastWorkspaceRoute, .remoteComputer(nodeId: "mac_prod_v168"))

        router.navigate(to: .miniApp(id: "global_dharma"), coordinator: coordinator)
        XCTAssertEqual(router.presentedMiniAppId, "global_dharma")
        XCTAssertFalse(router.isCallsSheetPresented)
        XCTAssertEqual(router.activeRoute, .miniApp(id: "global_dharma"))
        XCTAssertEqual(router.lastWorkspaceRoute, .remoteComputer(nodeId: "mac_prod_v168"))

        router.dismissActiveModalSheet(coordinator: coordinator)
        XCTAssertNil(router.presentedMiniAppId)
        XCTAssertEqual(router.activeRoute, .remoteComputer(nodeId: "mac_prod_v168"))
        XCTAssertEqual(coordinator.activeRoute, .remoteComputer(nodeId: "mac_prod_v168"))
    }

    func testSettingsSectionDestinationParsingForDeepLinksAndProviderError() {
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("aiProviders"), .aiProviders)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("ai_providers"), .aiProviders)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("AI-Providers"), .aiProviders)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("magpie"), .aiProviders)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("appearance"), .appearance)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("THEME"), .appearance)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("language"), .language)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("locale"), .language)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("privacySecurity"), .privacySecurity)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("privacy_security"), .privacySecurity)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("privacy"), .privacySecurity)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("devices"), .devices)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("vault"), .vault)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("capabilities"), .capabilities)
        XCTAssertEqual(FabushiSettingsSectionDestination.parse("capability_ledger"), .capabilities)
        XCTAssertNil(FabushiSettingsSectionDestination.parse(nil))
        XCTAssertNil(FabushiSettingsSectionDestination.parse(""))
        XCTAssertNil(FabushiSettingsSectionDestination.parse("   "))
        XCTAssertNil(FabushiSettingsSectionDestination.parse("nonexistent_section"))
    }
}

// MARK: - Protocol Conformance Test Spies

@MainActor
private final class SpyMessagingProvider: MessagingModuleProviding {
    var callCount = 0
    func makeChatSidebarView(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeContactsView(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeConversationStageView(chatId: Int64?, messageId: Int64?, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeMultiSelectComposeView(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeContextInspectorView(target: InspectorTarget, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeUniversalSearchView(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeMediaViewerOverlay(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeStoriesOverlay(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeInstantViewOverlay(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
}

@MainActor
private final class SpyAgentStudioProvider: AgentStudioModuleProviding {
    var callCount = 0
    func makeAgentStudioWorkspaceView(initialTab: String?, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeMagpieAiProvidersSettingsView(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeOfficialMcpConsoleView(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeCodingAgentThreadAccessories(chatId: Int64, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeMultiAgentRoomAccessories(chatId: Int64, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
}

@MainActor
private final class SpyPlatformProvider: PlatformModulesProviding {
    var callCount = 0
    func makeRemoteComputerWorkbenchView(nodeId: String?, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeRemoteControlledPillView(status: RemoteControlStatusState, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeMiniAppHostSheetView(miniAppId: String?, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeMiniAppFloatingPipView(pip: MiniAppPipOverlayState, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeCallAndTalkOverlayView(callPip: CallPipOverlayState, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeCallsTabView(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeSettingsRootView(initialSection: String?, coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeWalletAndPremiumViews(coordinator: FabushiCoordinator, router: ShellNavigationRouter) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
}

@MainActor
private final class SpyAuthProvider: AuthModuleProviding {
    var callCount = 0
    func makeAuthGateView(coordinator: FabushiCoordinator, router: ShellNavigationRouter, launchConfiguration: FabushiLaunchConfiguration) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
}

@MainActor
private final class SpySecurityProvider: SecurityModuleProviding {
    var callCount = 0
    func makePasscodeLockOverlay(coordinator: FabushiCoordinator, launchConfiguration: FabushiLaunchConfiguration, onUnlock: @escaping () -> Void) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makePrivacyBlurOverlay() -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeAccountSwitcherSheet(coordinator: FabushiCoordinator, router: ShellNavigationRouter, onDismiss: @escaping () -> Void) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeVaultSettingsView(coordinator: FabushiCoordinator) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
    func makeDevicesSettingsView(coordinator: FabushiCoordinator) -> AnyView { callCount += 1; return AnyView(EmptyView()) }
}
