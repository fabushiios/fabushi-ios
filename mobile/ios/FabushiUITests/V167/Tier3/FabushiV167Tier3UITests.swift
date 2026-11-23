import Foundation
import UIKit
import XCTest

/// Tier 3 Cross-Feature & Adaptive iPhone/iPad Form-Factor Pairwise UI E2E Suite (10 tests).
@MainActor
final class FabushiV167Tier3UITests: FabushiE2EUITestCase {

    // MARK: - 1. iPad 3-Column NavigationSplitView + .inspector + Rotation + Dynamic Type + Night

    func testT3_01_F07_F16_F31_ipadThreeColumnSplitViewInspectorRotationDynamicTypeNight() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t3_ipad_split")],
                chats: [FabushiLoopbackSeedChat(chatId: 301, title: "iPad 3-Col Workspace", chatType: "group", memberCount: 5)]
            )
        )
        let config = FabushiE2ELaunchConfig(
            theme: "night",
            preferredContentSizeCategory: "UICTContentSizeCategoryAccessibilityLarge"
        )
        let application = try launchFabushi(config)
        signInIfNeeded(in: application, username: "t3_ipad_split", password: "FabushiV168!Pass")

        setDeviceOrientation(.landscapeLeft)
        openFirstChat(in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Stage.inspectorToggleButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let inspector = requireElement(FabushiAccessibilityIDs.Inspector.drawer, in: application)
        assertWithinWindowBounds(inspector, in: application)

        if activeFormFactor() == .ipad {
            let stageRoot = requireElement(FabushiAccessibilityIDs.Stage.root, in: application)
            let splitView = requireElement(FabushiAccessibilityIDs.Shell.splitView, in: application)
            assertWithinWindowBounds(stageRoot, in: application)
            assertWithinWindowBounds(splitView, in: application)
            assertNonOverlapping(stageRoot, inspector)

            setDeviceOrientation(.portrait)
            XCTAssertTrue(
                stageRoot.waitForExistence(timeout: 5),
                "T3_01 F07/F16/F31 iPad stage must remain intact after landscape -> portrait rotation"
            )
        } else {
            setDeviceOrientation(.portrait)
            assertWithinWindowBounds(inspector, in: application)
            dismissInspectorIfPresented(in: application)
            let stageRoot = requireElement(FabushiAccessibilityIDs.Stage.root, in: application, timeout: 5)
            assertWithinWindowBounds(stageRoot, in: application)
        }
        recordCheckpoint("T3_01_ipad_3col_rotation_night", passed: true, attachScreen: true)
    }

    // MARK: - 2. iPhone TabView + NavigationStack + .sheet Inspector

    func testT3_02_F07_F16_F31_iphoneTabViewNavigationStackAndSheetInspector() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t3_iphone_stack")],
                chats: [FabushiLoopbackSeedChat(chatId: 302, title: "Compact iPhone Thread", chatType: "private", memberCount: 2)]
            )
        )
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "tinted"))
        signInIfNeeded(in: application, username: "t3_iphone_stack", password: "FabushiV168!Pass")

        if activeFormFactor() == .iphone {
            let tabBar = requireElement(FabushiAccessibilityIDs.Shell.tabBar, in: application)
            assertWithinWindowBounds(tabBar, in: application)
        }
        openFirstChat(in: application)
        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)

        let inspector = requireElement(FabushiAccessibilityIDs.Inspector.drawer, in: application)
        assertWithinWindowBounds(inspector, in: application)
        let userSection = requireElement(FabushiAccessibilityIDs.Inspector.profileUserSection, in: application)
        assertVoiceOverReady(userSection)

        let closeBtn = requireElement(FabushiAccessibilityIDs.Inspector.closeButton, in: application)
        assertMinimumTouchTarget44pt(closeBtn)
        closeBtn.tap()
        let closed = inspector.waitForNonExistence(timeout: 5)
        XCTAssertTrue(closed, "T3_02 F07/F16 closing sheet/drawer inspector must return cleanly to stage")
        recordCheckpoint("T3_02_iphone_tab_stack_sheet_inspector", passed: true)
    }

    // MARK: - 3. Account Switch + Active Sync + Vault Lock

    func testT3_03_F06_F11_F23_accountSwitchActiveSyncAndVaultLock() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [
                    FabushiLoopbackSeedUser(username: "t3_acct_alpha"),
                    FabushiLoopbackSeedUser(username: "t3_acct_beta"),
                ],
                chats: [FabushiLoopbackSeedChat(chatId: 303, title: "Alpha Sync Thread")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t3_acct_alpha", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let composerField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: composerField, with: "Sync check before account switch")
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        navigateBackToChatListIfCompact(in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Shell.accountSwitcherButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let sheet = requireElement(FabushiAccessibilityIDs.Shell.accountSwitcherSheet, in: application)
        assertWithinWindowBounds(sheet, in: application)
        dismissAccountSwitcherIfPresented(in: application)

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionVault, in: application)
        let vaultBtn = requireElement(FabushiAccessibilityIDs.Settings.vaultUnlockButton, in: application)
        assertMinimumTouchTarget44pt(vaultBtn)
        assertVoiceOverReady(vaultBtn)

        let state = try await client.fetchState()
        XCTAssertGreaterThanOrEqual(state.counts.messages, 1, "T3_03 F06/F11/F23 message must persist prior to account switch")
        recordCheckpoint("T3_03_account_switch_sync_vault", passed: true)
    }

    // MARK: - 4. Grok Multi-Select To: Compose + Multi-Agent Room + Floor Control

    func testT3_04_F08_F18_grokMultiSelectComposeMultiAgentRoomAndFloorControl() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t3_grok_multiagent")],
                chats: [
                    FabushiLoopbackSeedChat(chatId: 304, title: "@GrokBot", chatType: "bot", memberCount: 2),
                    FabushiLoopbackSeedChat(chatId: 305, title: "@HermesBot", chatType: "bot", memberCount: 2),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t3_grok_multiagent", password: "FabushiV168!Pass")

        tapControl(FabushiAccessibilityIDs.ChatList.composeButton, in: application)
        let toField = requireElement(FabushiAccessibilityIDs.Compose.toField, in: application)
        replaceText(in: toField, with: "Bot")
        tapControl(FabushiAccessibilityIDs.Compose.firstCandidate, in: application)
        tapControl(FabushiAccessibilityIDs.Compose.secondCandidate, in: application)

        let titleField = requireElement(FabushiAccessibilityIDs.Compose.groupTitleField, in: application)
        replaceText(in: titleField, with: "Grok + Hermes Council")
        let composerField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: composerField, with: "Coordinate release checklist")
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let floorBanner = requireElement(FabushiAccessibilityIDs.Stage.floorControlBanner, in: application, timeout: 12)
        assertVoiceOverReady(floorBanner)
        let interruptBtn = requireElement(FabushiAccessibilityIDs.Stage.floorInterruptButton, in: application)
        assertMinimumTouchTarget44pt(interruptBtn)
        interruptBtn.tap()
        XCTAssertTrue(
            floorBanner.waitForNonExistence(timeout: 6),
            "T3_04 interrupt must dismiss floorControlBanner"
        )
        recordCheckpoint("T3_04_grok_compose_multi_agent_floor", passed: true)
    }

    // MARK: - 5. Slash Command `/` + Tool Approval Dock + Streaming AI Turn

    func testT3_05_F15_F17_slashCommandToolApprovalDockAndStreamingAITurn() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t3_slash_approval")],
                chats: [FabushiLoopbackSeedChat(chatId: 306, title: "@HermesAgent", chatType: "bot", memberCount: 2)]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t3_slash_approval", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let field = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: field, with: "/")
        _ = requireElement(FabushiAccessibilityIDs.Composer.slashCommandPopover, in: application)
        tapControl(FabushiAccessibilityIDs.Composer.slashCommandFirstItem, in: application)
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let approvalDock = requireElement(FabushiAccessibilityIDs.Stage.toolApprovalDock, in: application, timeout: 10)
        assertWithinWindowBounds(approvalDock, in: application)
        let approveBtn = requireElement(FabushiAccessibilityIDs.Stage.toolApproveButton, in: application)
        let denyBtn = requireElement(FabushiAccessibilityIDs.Stage.toolDenyButton, in: application)
        assertMinimumTouchTarget44pt(approveBtn)
        assertMinimumTouchTarget44pt(denyBtn)
        assertNonOverlapping(approveBtn, denyBtn)
        let preApprovalLabel = element(FabushiAccessibilityIDs.Stage.messageBubbleLatest, in: application).label
        approveBtn.tap()

        XCTAssertTrue(
            approvalDock.waitForNonExistence(timeout: 6),
            "T3_05 F15/F17 approving tool dock must dismiss toolApprovalDock"
        )
        let progressed = await waitUntil(timeout: 10) {
            if self.element(FabushiAccessibilityIDs.Stage.streamingIndicator, in: application).exists {
                return true
            }
            let latest = self.element(FabushiAccessibilityIDs.Stage.messageBubbleLatest, in: application)
            return latest.exists && !latest.label.isEmpty && latest.label != preApprovalLabel
        }
        XCTAssertTrue(
            progressed,
            "T3_05 F15/F17 approving tool dock must trigger streaming indicator or render a new post-approval AI bubble"
        )
        recordCheckpoint("T3_05_slash_approval_streaming_turn", passed: true)
    }

    // MARK: - 6. Mini App WebMCP + Floating PiP Minimize/Restore + Background/Foreground

    func testT3_06_F24_F30_miniAppWebMCPFloatingPiPMinimizeRestoreAndBackgroundForeground() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t3_pip_bg")],
                miniapps: [
                    FabushiLoopbackSeedMiniApp(appId: "global_dharma", title: "全球法布施", tosAccepted: true)
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t3_pip_bg", password: "FabushiV168!Pass")

        tapControl(FabushiAccessibilityIDs.MiniApp.launcherButton, in: application)
        tapControl(FabushiAccessibilityIDs.MiniApp.globalDharmaCard, in: application)
        let webView = requireElement(FabushiAccessibilityIDs.MiniApp.webView, in: application, timeout: 15)
        assertWithinWindowBounds(webView, in: application)

        let minBtn = requireElement(FabushiAccessibilityIDs.MiniApp.minimizePiPButton, in: application)
        assertMinimumTouchTarget44pt(minBtn)
        minBtn.tap()
        let pipPill = requireElement(FabushiAccessibilityIDs.MiniApp.floatingPiPPill, in: application)
        assertWithinWindowBounds(pipPill, in: application)

        XCUIDevice.shared.press(.home)
        application.activate()

        let pipSurvived = pipPill.waitForExistence(timeout: 10)
        XCTAssertTrue(pipSurvived, "T3_06 F24/F30 Floating PiP pill must survive background/foreground cycle")
        pipPill.tap()
        let restoredWebView = requireElement(FabushiAccessibilityIDs.MiniApp.webView, in: application, timeout: 10)
        assertWithinWindowBounds(restoredWebView, in: application)
        recordCheckpoint("T3_06_miniapp_pip_bg_fg", passed: true)
    }

    // MARK: - 7. Remote Computer Workbench + PTY + Emergency Takeover + Controlled Pill + v1.6.8 Permission Prompt

    func testT3_07_F21_F22_remoteComputerWorkbenchPTYTakeoverControlledPillAndV168PermissionPrompt() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t3_remote_combo")],
                remoteNodes: [
                    FabushiLoopbackSeedRemoteNode(
                        deviceId: "mac_workstation_v168",
                        name: "Mac Studio Workstation",
                        online: true,
                        screenCapturePermission: "granted_needs_restart",
                        accessibilityDragPermission: "granted_active"
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t3_remote_combo", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabRemote, in: application)
        tapControl(FabushiAccessibilityIDs.Remote.firstNodeRow, in: application)

        let permModal = requireElement(FabushiAccessibilityIDs.Remote.permissionModal, in: application, timeout: 10)
        assertWithinWindowBounds(permModal, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Remote.permissionModalScreenStatus, in: application))
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Remote.permissionModalDismissButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        XCTAssertTrue(
            permModal.waitForNonExistence(timeout: 5),
            "T3_07 v1.6.8 permission modal must dismiss cleanly"
        )

        let controlledPill = requireElement(FabushiAccessibilityIDs.Remote.controlledBanner, in: application)
        assertVoiceOverReady(controlledPill)
        let pty = requireElement(FabushiAccessibilityIDs.Remote.ptyTerminal, in: application)
        assertWithinWindowBounds(pty, in: application)
        let takeover = requireElement(FabushiAccessibilityIDs.Remote.emergencyTakeoverButton, in: application)
        assertMinimumTouchTarget44pt(takeover)
        takeover.tap()

        let state = try await client.fetchState(pathPrefix: "/api/v1/computer/")
        let takeoverCalls = state.recordedRequests.filter {
            $0.method == "POST" && $0.path.contains("emergency_takeover")
        }
        XCTAssertGreaterThanOrEqual(
            takeoverCalls.count,
            1,
            "T3_07 F21/F22 emergency takeover must POST /api/v1/computer/lease/emergency_takeover"
        )
        recordCheckpoint("T3_07_remote_pty_takeover_v168_perm", passed: true)
    }

    // MARK: - 8. Calls + Talk Mode VAD Orb Barge-In

    func testT3_08_F26_F28_callsAndTalkModeVADOrbBargeIn() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t3_call_talk")],
                chats: [FabushiLoopbackSeedChat(chatId: 308, title: "Voice Call & Talk Room")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t3_call_talk", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        tapControl(FabushiAccessibilityIDs.Stage.callStartButton, in: application)
        let fingerprint = requireElement(FabushiAccessibilityIDs.Call.fingerprintBadge, in: application)
        assertVoiceOverReady(fingerprint)
        tapControl(FabushiAccessibilityIDs.Call.hangupButton, in: application)

        tapControl(FabushiAccessibilityIDs.Stage.talkModeButton, in: application)
        let talkOrb = requireElement(FabushiAccessibilityIDs.Call.talkOrb, in: application)
        assertWithinWindowBounds(talkOrb, in: application)
        let bargeIn = requireElement(FabushiAccessibilityIDs.Call.talkBargeInButton, in: application)
        assertMinimumTouchTarget44pt(bargeIn)
        bargeIn.tap()
        tapControl(FabushiAccessibilityIDs.Call.talkStopButton, in: application)
        recordCheckpoint("T3_08_calls_and_talk_barge_in", passed: true)
    }

    // MARK: - 9. Magpie Privacy Masking + Routing Fallback + Vault OAuth

    func testT3_09_F19_F23_magpiePrivacyMaskingRoutingFallbackAndVaultOAuth() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t3_magpie_vault")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t3_magpie_vault", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionAIProviders, in: application)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Magpie.piiMaskingToggle,
            in: application,
            verifyMinTouchTarget: false,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Magpie.routingRuleToggle,
            in: application,
            verifyMinTouchTarget: false,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        let vaultOAuthBtn = requireElement(FabushiAccessibilityIDs.Magpie.vaultOAuthSyncButton, in: application)
        assertMinimumTouchTarget44pt(vaultOAuthBtn)
        vaultOAuthBtn.tap()

        let breakerBadge = requireElement(FabushiAccessibilityIDs.Magpie.circuitBreakerBadge, in: application)
        assertVoiceOverReady(breakerBadge)
        recordCheckpoint("T3_09_magpie_pii_routing_vault_oauth", passed: true)
    }

    // MARK: - 10. Multi-Select Message Forward/Delete + Search + E2EE Safety Number

    func testT3_10_F13_F27_F28_multiSelectForwardDeleteSearchAndE2EESafetyNumber() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t3_msel_e2ee")],
                chats: [FabushiLoopbackSeedChat(chatId: 310, title: "Encrypted Ops Room", chatType: "private", memberCount: 2)],
                messages: [
                    FabushiLoopbackSeedMessage(chatId: 310, text: "Encrypted audit artifact alpha"),
                    FabushiLoopbackSeedMessage(chatId: 310, text: "Encrypted audit artifact beta"),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t3_msel_e2ee", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let bubble = requireElement(FabushiAccessibilityIDs.Stage.messageBubbleLatest, in: application)
        bubble.press(forDuration: 0.8)
        tapControl(FabushiAccessibilityIDs.Stage.forwardMessageAction, in: application)

        let toolbar = requireElement(FabushiAccessibilityIDs.Stage.multiSelectToolbar, in: application)
        assertWithinWindowBounds(toolbar, in: application)
        let fwdBtn = requireElement(FabushiAccessibilityIDs.Stage.multiSelectForwardButton, in: application)
        let delBtn = requireElement(FabushiAccessibilityIDs.Stage.multiSelectDeleteButton, in: application)
        assertMinimumTouchTarget44pt(fwdBtn)
        assertMinimumTouchTarget44pt(delBtn)
        assertNonOverlapping(fwdBtn, delBtn)

        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        tapControl(FabushiAccessibilityIDs.Inspector.e2eeSafetyNumberButton, in: application)
        let emojiGrid = requireElement(FabushiAccessibilityIDs.Inspector.e2eeSafetyEmojiGrid, in: application)
        assertWithinWindowBounds(emojiGrid, in: application)
        recordCheckpoint("T3_10_multiselect_search_e2ee", passed: true)
    }
}
