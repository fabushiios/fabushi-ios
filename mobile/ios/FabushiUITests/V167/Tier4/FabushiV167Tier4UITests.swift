import Foundation
import UIKit
import XCTest

/// Tier 4 Full Multi-Screen Real-World End-to-End User Journeys across iPhone & iPad Simulators (8 tests).
@MainActor
final class FabushiV167Tier4UITests: FabushiE2EUITestCase {

    // MARK: - Journey 1: Auth / Split-Key V2 -> Multi-Account -> Messaging & E2EE

    func testT4_01_F01_F06_F11_F28_authSplitKeyV2MultiAccountMessagingAndE2EEJourney() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [
                    FabushiLoopbackSeedUser(username: "t4_j1_secondary"),
                ],
                chats: [
                    FabushiLoopbackSeedChat(chatId: 401, title: "Encrypted Dharma Direct", chatType: "private", memberCount: 2)
                ]
            )
        )
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "night", resetState: true))
        assertNoSampleChats(in: application)

        // Register primary account through Split-Key V2 UI (zero plaintext password on wire)
        tapControl(FabushiAccessibilityIDs.Auth.registerTabButton, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Auth.splitKeyBadge, in: application))
        let regUserField = requireElement(FabushiAccessibilityIDs.Auth.usernameField, in: application)
        replaceText(in: regUserField, with: "t4_j1_primary")
        let regPassField = requireElement(FabushiAccessibilityIDs.Auth.passwordField, in: application)
        replaceText(in: regPassField, with: "SplitKeyV2!Journey1")
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Auth.registerSubmitButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        _ = requireElement(FabushiAccessibilityIDs.Shell.root, in: application, timeout: 15)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Shell.accountSwitcherButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let switcherSheet = requireElement(FabushiAccessibilityIDs.Shell.accountSwitcherSheet, in: application)
        assertWithinWindowBounds(switcherSheet, in: application)
        dismissAccountSwitcherIfPresented(in: application)
        openShellTab(FabushiAccessibilityIDs.Shell.tabChats, in: application)

        openFirstChat(in: application)
        let field = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: field, with: "End-to-end verified Split-Key V2 message")
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let delivered = await waitUntil(timeout: 10) {
            self.element(FabushiAccessibilityIDs.Stage.messageBubbleLatest, in: application).exists
        }
        XCTAssertTrue(delivered, "T4_01 message must appear in encrypted conversation stage")

        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        tapControl(FabushiAccessibilityIDs.Inspector.e2eeSafetyNumberButton, in: application)
        let safetyGrid = requireElement(FabushiAccessibilityIDs.Inspector.e2eeSafetyEmojiGrid, in: application)
        assertWithinWindowBounds(safetyGrid, in: application)

        let authState = try await client.fetchState(pathPrefix: "/api/v1/auth")
        let registerRequests = authState.recordedRequests.filter { $0.path == "/api/v1/auth/register" }
        XCTAssertGreaterThanOrEqual(registerRequests.count, 1, "T4_01 must POST /api/v1/auth/register")
        XCTAssertTrue(
            registerRequests.contains(where: { $0.bodyPreview.contains("auth_verifier_b64") }),
            "T4_01 Split-Key V2 registration must send derived auth_verifier_b64"
        )
        XCTAssertTrue(
            registerRequests.allSatisfy { !$0.bodyPreview.contains("SplitKeyV2!Journey1") },
            "T4_01 Split-Key V2 registration must never send plaintext password on the wire"
        )
        XCTAssertGreaterThanOrEqual(authState.counts.messages, 1, "T4_01 message must persist in loopback backend")
        recordCheckpoint("T4_01_auth_multiaccount_messaging_e2ee", passed: true, attachScreen: true)
    }

    // MARK: - Journey 2: Grok Multi-Select Bot Team Room -> Slash Command -> Approval

    func testT4_02_F08_F15_F17_F18_grokMultiSelectBotTeamRoomSlashCommandAndApprovalJourney() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t4_j2_team")],
                chats: [
                    FabushiLoopbackSeedChat(chatId: 402, title: "@GrokBot", chatType: "bot", memberCount: 2),
                    FabushiLoopbackSeedChat(chatId: 403, title: "@HermesBot", chatType: "bot", memberCount: 2),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t4_j2_team", password: "FabushiV168!Pass")

        tapControl(FabushiAccessibilityIDs.ChatList.composeButton, in: application)
        let toField = requireElement(FabushiAccessibilityIDs.Compose.toField, in: application)
        replaceText(in: toField, with: "Bot")
        tapControl(FabushiAccessibilityIDs.Compose.firstCandidate, in: application)
        tapControl(FabushiAccessibilityIDs.Compose.secondCandidate, in: application)

        let titleField = requireElement(FabushiAccessibilityIDs.Compose.groupTitleField, in: application)
        replaceText(in: titleField, with: "v1.6.8 Bot Team Room")
        let composer = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: composer, with: "/")
        _ = requireElement(FabushiAccessibilityIDs.Composer.slashCommandPopover, in: application)
        tapControl(FabushiAccessibilityIDs.Composer.slashCommandFirstItem, in: application)
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let approvalDock = requireElement(FabushiAccessibilityIDs.Stage.toolApprovalDock, in: application, timeout: 12)
        assertWithinWindowBounds(approvalDock, in: application)
        let approveBtn = requireElement(FabushiAccessibilityIDs.Stage.toolApproveButton, in: application)
        assertMinimumTouchTarget44pt(approveBtn)
        approveBtn.tap()
        XCTAssertTrue(
            approvalDock.waitForNonExistence(timeout: 6),
            "T4_02 approval dock must dismiss after approve"
        )

        let floorBanner = requireElement(FabushiAccessibilityIDs.Stage.floorControlBanner, in: application, timeout: 10)
        assertVoiceOverReady(floorBanner)
        recordCheckpoint("T4_02_grok_team_slash_approval", passed: true, attachScreen: true)
    }

    // MARK: - Journey 3: 7-Tab Hermes Studio + Magpie + Coding Agent Diff/PR

    func testT4_03_F17_F19_F20_F21_sevenTabHermesStudioMagpieAndCodingAgentDiffPRJourney() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t4_j3_studio")],
                chats: [FabushiLoopbackSeedChat(chatId: 404, title: "@CodexAgent", chatType: "bot", memberCount: 2)],
                messages: [
                    FabushiLoopbackSeedMessage(
                        chatId: 404,
                        text: "```diff\n--- a/Sources/Host.swift\n+++ b/Sources/Host.swift\n@@ -1,1 +1,1 @@\n-let v = \"1.6.7\"\n+let v = \"1.6.8\"\n```",
                        messageType: "coding_diff"
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t4_j3_studio", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabStudio, in: application)
        let studioRoot = requireElement(FabushiAccessibilityIDs.Studio.root, in: application)
        let studioTabBar = resolveHorizontalScrollContainer(
            preferredIdentifier: FabushiAccessibilityIDs.Studio.tabBar,
            inside: studioRoot,
            in: application
        )
        for tabID in [
            FabushiAccessibilityIDs.Studio.tabOverview,
            FabushiAccessibilityIDs.Studio.tabKanban,
            FabushiAccessibilityIDs.Studio.tabSkills,
            FabushiAccessibilityIDs.Studio.tabMemory,
            FabushiAccessibilityIDs.Studio.tabCron,
            FabushiAccessibilityIDs.Studio.tabApprovals,
            FabushiAccessibilityIDs.Studio.tabSessions,
        ] {
            if let studioTabBar {
                scrollToElementHorizontally(element(tabID, in: application), in: studioTabBar, maxSwipes: 6)
            }
            tapControl(tabID, in: application)
        }
        let codingPanel = requireElement(FabushiAccessibilityIDs.Studio.codingAgentsPanel, in: application)
        assertWithinWindowBounds(codingPanel, in: application)
        let prBtn = requireElement(FabushiAccessibilityIDs.Studio.codingAgentCreatePRButton, in: application)
        assertMinimumTouchTarget44pt(prBtn)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Studio.mcpGrantsButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        let mcpAudit = requireElement(FabushiAccessibilityIDs.Studio.mcpAuditLogList, in: application)
        assertWithinWindowBounds(mcpAudit, in: application)

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionAIProviders, in: application)
        let magpieRoot = requireElement(FabushiAccessibilityIDs.Magpie.root, in: application)
        assertWithinWindowBounds(magpieRoot, in: application)
        tapControl(FabushiAccessibilityIDs.Magpie.probeHealthButton, in: application)

        openShellTab(FabushiAccessibilityIDs.Shell.tabChats, in: application)
        openFirstChat(in: application)
        let diffCard = requireElement(FabushiAccessibilityIDs.Stage.codingDiffCard, in: application)
        assertWithinWindowBounds(diffCard, in: application)
        recordCheckpoint("T4_03_studio_magpie_coding_diff_pr", passed: true)
    }

    // MARK: - Journey 4: Remote Computer PTY + Emergency Takeover + v1.6.8 Screen-Capture Permission

    func testT4_04_F16_F22_F30_remoteComputerPTYEmergencyTakeoverAndV168ScreenPermissionJourney() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t4_j4_remote")],
                remoteNodes: [
                    FabushiLoopbackSeedRemoteNode(
                        deviceId: "mac_prod_v168",
                        name: "Mac Studio Production",
                        online: true,
                        screenCapturePermission: "not_granted",
                        accessibilityDragPermission: "not_granted"
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t4_j4_remote", password: "FabushiV168!Pass")

        openDeepLink("fabushi://remote/mac_prod_v168", in: application)
        let workbench = requireElement(FabushiAccessibilityIDs.Remote.workbench, in: application, timeout: 10)
        assertWithinWindowBounds(workbench, in: application)

        let permModal = requireElement(FabushiAccessibilityIDs.Remote.permissionModal, in: application, timeout: 10)
        assertWithinWindowBounds(permModal, in: application)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Remote.macosDragHudForbidden, in: application).exists,
            "T4_04 iOS remote controller must never render macOS AppKit drag-HUD"
        )
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Remote.permissionModalDismissButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        XCTAssertTrue(
            permModal.waitForNonExistence(timeout: 5),
            "T4_04 remote permission modal must dismiss cleanly"
        )

        let pty = requireElement(FabushiAccessibilityIDs.Remote.ptyTerminal, in: application)
        assertWithinWindowBounds(pty, in: application)
        let takeoverBtn = requireElement(FabushiAccessibilityIDs.Remote.emergencyTakeoverButton, in: application)
        assertMinimumTouchTarget44pt(takeoverBtn)
        takeoverBtn.tap()

        let state = try await client.fetchState(pathPrefix: "/api/v1/computer/")
        let takeoverCalls = state.recordedRequests.filter {
            $0.method == "POST" && $0.path.contains("emergency_takeover")
        }
        XCTAssertGreaterThanOrEqual(
            takeoverCalls.count,
            1,
            "T4_04 remote takeover must POST /api/v1/computer/lease/emergency_takeover"
        )
        recordCheckpoint("T4_04_remote_pty_takeover_v168_perm", passed: true, attachScreen: true)
    }

    // MARK: - Journey 5: 全球法布施 Mini App + Live WebMCP + Floating PiP + Cold-Restart Durability

    func testT4_05_F17_F24_F29_globalDharmaMiniAppLiveWebMCPFloatingPiPAndColdRestartDurability() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t4_j5_dharma")],
                miniapps: [
                    FabushiLoopbackSeedMiniApp(appId: "global_dharma", title: "全球法布施", tosAccepted: false)
                ]
            )
        )
        let firstLaunch = try launchFabushi(FabushiE2ELaunchConfig(resetState: true))
        signInIfNeeded(in: firstLaunch, username: "t4_j5_dharma", password: "FabushiV168!Pass")

        tapControl(FabushiAccessibilityIDs.MiniApp.launcherButton, in: firstLaunch)
        tapControl(FabushiAccessibilityIDs.MiniApp.globalDharmaCard, in: firstLaunch)
        let tosModal = requireElement(FabushiAccessibilityIDs.MiniApp.tosModal, in: firstLaunch)
        tapControl(FabushiAccessibilityIDs.MiniApp.tosAcceptButton, in: firstLaunch)
        XCTAssertTrue(
            tosModal.waitForNonExistence(timeout: 5),
            "T4_05 accepting Mini App ToS must dismiss tosModal"
        )

        let webView = requireElement(FabushiAccessibilityIDs.MiniApp.webView, in: firstLaunch, timeout: 15)
        assertWithinWindowBounds(webView, in: firstLaunch)
        let webMcpBadge = requireElement(FabushiAccessibilityIDs.MiniApp.webMcpStatusBadge, in: firstLaunch)
        assertVoiceOverReady(webMcpBadge)

        let tosState = try await client.fetchState(pathPrefix: "/api/v1/miniapps")
        XCTAssertGreaterThanOrEqual(
            tosState.matchedRequestCount,
            1,
            "T4_05 Mini App launch & ToS acceptance must reconcile with /api/v1/miniapps"
        )

        let minBtn = requireElement(FabushiAccessibilityIDs.MiniApp.minimizePiPButton, in: firstLaunch)
        assertMinimumTouchTarget44pt(minBtn)
        minBtn.tap()
        _ = requireElement(FabushiAccessibilityIDs.MiniApp.floatingPiPPill, in: firstLaunch)
        firstLaunch.terminate()

        // Cold restart WITHOUT resetState: verify accepted ToS & session persist cleanly
        let relaunched = try launchFabushi(FabushiE2ELaunchConfig(resetState: false))
        _ = requireElement(FabushiAccessibilityIDs.Shell.root, in: relaunched, timeout: 15)
        tapControl(FabushiAccessibilityIDs.MiniApp.launcherButton, in: relaunched)
        tapControl(FabushiAccessibilityIDs.MiniApp.globalDharmaCard, in: relaunched)
        let persistedWebView = requireElement(FabushiAccessibilityIDs.MiniApp.webView, in: relaunched, timeout: 15)
        assertWithinWindowBounds(persistedWebView, in: relaunched)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.MiniApp.tosModal, in: relaunched).exists,
            "T4_05 accepted Mini App ToS must persist across cold restart"
        )
        recordCheckpoint("T4_05_global_dharma_webmcp_pip_cold_restart", passed: true, attachScreen: true)
    }

    // MARK: - Journey 6: Calls + Talk Mode Barge-In

    func testT4_06_F12_F26_F28_callsReplayKitAndTalkModeBargeInFullJourney() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t4_j6_calls")],
                chats: [FabushiLoopbackSeedChat(chatId: 406, title: "Encrypted Call & Talk Room")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t4_j6_calls", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        tapControl(FabushiAccessibilityIDs.Stage.callStartButton, in: application)
        let callOverlay = requireElement(FabushiAccessibilityIDs.Call.overlay, in: application)
        assertWithinWindowBounds(callOverlay, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Call.fingerprintBadge, in: application))

        let muteBtn = requireElement(FabushiAccessibilityIDs.Call.muteButton, in: application)
        let camBtn = requireElement(FabushiAccessibilityIDs.Call.cameraButton, in: application)
        let hangupBtn = requireElement(FabushiAccessibilityIDs.Call.hangupButton, in: application)
        assertMinimumTouchTarget44pt(muteBtn)
        assertMinimumTouchTarget44pt(camBtn)
        assertMinimumTouchTarget44pt(hangupBtn)
        assertNonOverlapping(muteBtn, hangupBtn)
        hangupBtn.tap()

        tapControl(FabushiAccessibilityIDs.Stage.talkModeButton, in: application)
        let orb = requireElement(FabushiAccessibilityIDs.Call.talkOrb, in: application)
        assertWithinWindowBounds(orb, in: application)
        let bargeBtn = requireElement(FabushiAccessibilityIDs.Call.talkBargeInButton, in: application)
        assertMinimumTouchTarget44pt(bargeBtn)
        bargeBtn.tap()
        tapControl(FabushiAccessibilityIDs.Call.talkStopButton, in: application)
        recordCheckpoint("T4_06_calls_and_talk_barge_in", passed: true)
    }

    // MARK: - Journey 7: Full 26-Section Settings + StoreKit 2 + Proxy + Capability Ledger

    func testT4_07_F29_F32_F33_F34_full26SectionSettingsStoreKit2ProxyAndCapabilityLedgerJourney() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t4_j7_settings")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t4_j7_settings", password: "FabushiV168!Pass")

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionWalletStars, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Settings.starsBalanceLabel, in: application))
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Settings.storeKitDisclosureBanner, in: application))

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionNetworkProxy, in: application)
        let probeBtn = requireElement(FabushiAccessibilityIDs.Settings.proxyProbeButton, in: application)
        assertMinimumTouchTarget44pt(probeBtn)
        probeBtn.tap()

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionDataStorage, in: application)
        let purgeBtn = requireElement(FabushiAccessibilityIDs.Settings.storagePurgeCacheButton, in: application)
        assertMinimumTouchTarget44pt(purgeBtn)
        purgeBtn.tap()

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionCapabilities, in: application)
        assertCapabilityLedgerShowsCanonicalIDs(in: application)
        recordCheckpoint("T4_07_settings_26_storekit_proxy_ledger", passed: true)
    }

    // MARK: - Journey 8: Comprehensive Apple HIG iPhone+iPad A11y / zh-Hans+en / 4-Theme / AX5 Audit

    func testT4_08_F07_F31_F32_comprehensiveAppleHIGiPhoneIPadA11yLocale4ThemeAX5Audit() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t4_j8_hig_audit")],
                chats: [FabushiLoopbackSeedChat(chatId: 408, title: "Apple HIG Final Audit Room")]
            )
        )

        let auditMatrix: [(theme: String, accent: String, lang: String, locale: String, contentSize: String?)] = [
            ("night", "#FFFF00", "zh-Hans", "zh_CN", "UICTContentSizeCategoryAccessibilityExtraExtraExtraLarge"),
            ("tinted", "#FFFFFF", "en", "en_US", "UICTContentSizeCategoryAccessibilityLarge"),
            ("day", "#007AFF", "zh-Hans", "zh_CN", nil),
            ("classic", "#34C759", "en", "en_US", nil),
        ]

        for (index, item) in auditMatrix.enumerated() {
            let config = FabushiE2ELaunchConfig(
                theme: item.theme,
                accentHex: item.accent,
                languageCode: item.lang,
                localeIdentifier: item.locale,
                preferredContentSizeCategory: item.contentSize,
                reduceMotion: index == 0,
                highContrast: index == 1,
                resetState: index == 0
            )
            let application = try launchFabushi(config)
            signInIfNeeded(in: application, username: "t4_j8_hig_audit", password: "FabushiV168!Pass")
            if index >= 1 {
                navigateBackToChatListIfCompact(in: application)
            }
            assertNoSampleChats(in: application)

            let composeBtn = requireElement(FabushiAccessibilityIDs.ChatList.composeButton, in: application)
            assertMinimumTouchTarget44pt(composeBtn)
            assertWithinWindowBounds(composeBtn, in: application)
            assertVoiceOverReady(composeBtn)

            openFirstChat(in: application)
            let inputWrap = requireElement(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
            assertWithinWindowBounds(inputWrap, in: application)
            assertMinimumTouchTarget44pt(requireElement(FabushiAccessibilityIDs.Composer.sendButton, in: application))

            if activeFormFactor() == .ipad && index == 0 {
                setDeviceOrientation(.landscapeLeft)
                assertWithinWindowBounds(requireElement(FabushiAccessibilityIDs.Shell.splitView, in: application), in: application)
                setDeviceOrientation(.portrait)
            }
            if index < auditMatrix.count - 1 {
                application.terminate()
            }
        }
        recordCheckpoint("T4_08_comprehensive_apple_hig_audit", passed: true, attachScreen: true)
    }
}
