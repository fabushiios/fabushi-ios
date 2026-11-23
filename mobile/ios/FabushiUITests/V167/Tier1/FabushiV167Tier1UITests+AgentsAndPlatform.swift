import Foundation
import UIKit
import XCTest

/// Tier 1 Happy-Path UI E2E Suite — Agent Studio, Multi-Agent, Magpie, Coding, MCP,
/// Remote Computer, Vault, Mini Apps, Media, Calls/Talk, Search, E2EE, Economy,
/// Actions/Push, Apple HIG Design System, 26-Section Settings, Network/Proxy & Governance (F17..F34).
extension FabushiV167Tier1UITests {

    // MARK: - F17: Unified Hermes/Fabushi Agent Runtime, 29 Toolsets & 7-Tab Studio

    func testT1_F17_hermesAgentRuntime29ToolsetsAnd7TabStudio() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t1_f17_studio")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f17_studio", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabStudio, in: application)
        let studioRoot = requireElement(FabushiAccessibilityIDs.Studio.root, in: application)
        assertWithinWindowBounds(studioRoot, in: application)

        let sevenTabs = [
            FabushiAccessibilityIDs.Studio.tabOverview,
            FabushiAccessibilityIDs.Studio.tabKanban,
            FabushiAccessibilityIDs.Studio.tabSkills,
            FabushiAccessibilityIDs.Studio.tabMemory,
            FabushiAccessibilityIDs.Studio.tabCron,
            FabushiAccessibilityIDs.Studio.tabApprovals,
            FabushiAccessibilityIDs.Studio.tabSessions,
        ]
        for tabID in sevenTabs {
            tapControlWithHIGChecks(
                tabID,
                in: application,
                verifyMinTouchTarget: true,
                verifyWithinBounds: true,
                verifyVoiceOver: true
            )
        }

        tapControl(FabushiAccessibilityIDs.Studio.tabOverview, in: application)
        let toolsets = requireElement(FabushiAccessibilityIDs.Studio.toolsetList, in: application)
        assertWithinWindowBounds(toolsets, in: application)
        recordCheckpoint("T1_F17_seven_tab_agent_studio", passed: true, attachScreen: true)
    }

    // MARK: - F18: Multi-Agent Protocol (Floor Control, 5-Hop Loop Budget & Workspace)

    func testT1_F18_multiAgentFloorControlWorkspaceAndHumanInterrupt() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f18_multiagent")],
                chats: [FabushiLoopbackSeedChat(chatId: 118, title: "Multi-Agent Council", chatType: "group", memberCount: 4)]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f18_multiagent", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let textField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: textField, with: "@Hermes @Coder collaborate on architecture plan")
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let floorBanner = requireElement(FabushiAccessibilityIDs.Stage.floorControlBanner, in: application)
        assertVoiceOverReady(floorBanner)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Stage.floorInterruptButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        XCTAssertTrue(
            floorBanner.waitForNonExistence(timeout: 6),
            "F18 human interrupt must dismiss floorControlBanner"
        )

        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        let workspaceBadge = requireElement(FabushiAccessibilityIDs.Inspector.multiAgentWorkspaceBadge, in: application)
        assertVoiceOverReady(workspaceBadge)
        recordCheckpoint("T1_F18_multi_agent_floor_control", passed: true)
    }

    // MARK: - F19: Pure-Rust Magpie AI Gateway, Routing, PII Masking & Vault OAuth

    func testT1_F19_magpieAIGatewayRoutingPIIMaskingAndHealthProbe() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t1_f19_magpie")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f19_magpie", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionAIProviders, in: application)

        let magpieRoot = requireElement(FabushiAccessibilityIDs.Magpie.root, in: application)
        assertWithinWindowBounds(magpieRoot, in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Magpie.piiMaskingToggle,
            in: application,
            verifyMinTouchTarget: false,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Magpie.probeHealthButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let probeStatus = requireElement(FabushiAccessibilityIDs.Magpie.probeStatusLabel, in: application)
        assertVoiceOverReady(probeStatus)
        recordCheckpoint("T1_F19_magpie_gateway", passed: true)
    }

    // MARK: - F20: Third-Party Coding Agents (Claude Code, Codex, ACP) Mobile Client

    func testT1_F20_codingAgentsDiffCardCheckpointAndPRFlow() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f20_coding")],
                chats: [FabushiLoopbackSeedChat(chatId: 120, title: "@ClaudeCode", chatType: "bot", memberCount: 2)],
                messages: [
                    FabushiLoopbackSeedMessage(
                        chatId: 120,
                        text: "```diff\n--- a/Sources/Host.swift\n+++ b/Sources/Host.swift\n@@ -1,1 +1,1 @@\n-let v = \"1.6.7\"\n+let v = \"1.6.8\"\n```",
                        messageType: "coding_diff"
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f20_coding", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let diffCard = requireElement(FabushiAccessibilityIDs.Stage.codingDiffCard, in: application)
        assertWithinWindowBounds(diffCard, in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Stage.codingCheckpointButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )

        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        let hostProbe = requireElement(FabushiAccessibilityIDs.Inspector.codingHostProbeButton, in: application)
        assertMinimumTouchTarget44pt(hostProbe)
        assertVoiceOverReady(hostProbe)
        recordCheckpoint("T1_F20_coding_agents_mobile", passed: true)
    }

    // MARK: - F21: Official OAuth 2.1 Remote MCP Grants, PKCE/QR & Redacted Audit

    func testT1_F21_officialMCPOAuth21GrantsAndRedactedAuditLog() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t1_f21_mcp")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f21_mcp", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabStudio, in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Studio.mcpGrantsButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        let auditLog = requireElement(FabushiAccessibilityIDs.Studio.mcpAuditLogList, in: application)
        assertWithinWindowBounds(auditLog, in: application)
        assertVoiceOverReady(auditLog)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Studio.mcpApproveButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        XCTAssertFalse(
            auditLog.label.contains("Bearer ") || auditLog.label.contains("tok_"),
            "F21 MCP audit log must redact sensitive bearer tokens"
        )

        let state = try await client.fetchState(pathPrefix: "/api/v1/mcp")
        XCTAssertGreaterThanOrEqual(
            state.matchedRequestCount,
            1,
            "F21 MCP grants & audit flow must send >= 1 real /api/v1/mcp request to loopback"
        )
        recordCheckpoint("T1_F21_mcp_grants_and_audit", passed: true)
    }

    // MARK: - F22: UU-Remote Computer Workbench, Multi-PTY, Tailnet & <10ms Takeover

    func testT1_F22_remoteComputerWorkbenchPTYControlledBannerAndTakeover() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f22_remote")],
                remoteNodes: [
                    FabushiLoopbackSeedRemoteNode(
                        deviceId: "mac_studio_v168",
                        name: "Mac Studio v1.6.8",
                        online: true,
                        screenCapturePermission: "granted_active",
                        accessibilityDragPermission: "granted_active"
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f22_remote", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabRemote, in: application)
        let workbench = requireElement(FabushiAccessibilityIDs.Remote.workbench, in: application)
        assertWithinWindowBounds(workbench, in: application)
        tapControl(FabushiAccessibilityIDs.Remote.firstNodeRow, in: application)

        let streamCanvas = requireElement(FabushiAccessibilityIDs.Remote.streamCanvas, in: application)
        assertWithinWindowBounds(streamCanvas, in: application)
        let controlledBanner = requireElement(FabushiAccessibilityIDs.Remote.controlledBanner, in: application)
        assertVoiceOverReady(controlledBanner)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Remote.emergencyTakeoverButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )

        let pty = requireElement(FabushiAccessibilityIDs.Remote.ptyTerminal, in: application)
        assertWithinWindowBounds(pty, in: application)

        let state = try await client.fetchState(pathPrefix: "/api/v1/computer/")
        let takeoverCalls = state.recordedRequests.filter {
            $0.method == "POST" && $0.path.contains("emergency_takeover")
        }
        XCTAssertGreaterThanOrEqual(
            takeoverCalls.count,
            1,
            "F22 emergency takeover must POST /api/v1/computer/lease/emergency_takeover"
        )
        recordCheckpoint("T1_F22_remote_workbench_takeover", passed: true, attachScreen: true)
    }

    // MARK: - F23: Zero-Knowledge Split-Key V2 & X25519 Vault + RFC 6238 TOTP

    func testT1_F23_zeroKnowledgeVaultUnlockAndRFC6238TOTPDisplay() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f23_vault")],
                vaultItems: [
                    FabushiLoopbackSeedVaultItem(
                        category: "totp",
                        itemKey: "fabushi_primary",
                        ciphertextB64: "ZW5jcnlwdGVkX3RvdHA=",
                        nonceB64: "MTIzNDU2Nzg5MDEy"
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f23_vault", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionVault, in: application)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Settings.vaultUnlockButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let totpLabel = requireElement(FabushiAccessibilityIDs.Settings.vaultTotpCodeLabel, in: application)
        assertVoiceOverReady(totpLabel)
        recordCheckpoint("T1_F23_vault_and_totp", passed: true)
    }

    // MARK: - F24: External Mini Apps, Live WebMCP, Floating PiP & Global Dharma

    func testT1_F24_externalMiniAppToSWebMCPAndFloatingPiPRestore() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f24_miniapp")],
                miniapps: [
                    FabushiLoopbackSeedMiniApp(appId: "global_dharma", title: "全球法布施", tosAccepted: false)
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f24_miniapp", password: "FabushiV168!Pass")

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.MiniApp.launcherButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        tapControl(FabushiAccessibilityIDs.MiniApp.globalDharmaCard, in: application)

        let tosModal = requireElement(FabushiAccessibilityIDs.MiniApp.tosModal, in: application)
        assertWithinWindowBounds(tosModal, in: application)
        tapControl(FabushiAccessibilityIDs.MiniApp.tosAcceptButton, in: application)

        let webView = requireElement(FabushiAccessibilityIDs.MiniApp.webView, in: application, timeout: 15)
        assertWithinWindowBounds(webView, in: application)
        let webMcpBadge = requireElement(FabushiAccessibilityIDs.MiniApp.webMcpStatusBadge, in: application)
        assertVoiceOverReady(webMcpBadge)

        let minBtn = requireElement(FabushiAccessibilityIDs.MiniApp.minimizePiPButton, in: application)
        assertMinimumTouchTarget44pt(minBtn)
        minBtn.tap()
        let pipPill = requireElement(FabushiAccessibilityIDs.MiniApp.floatingPiPPill, in: application)
        assertMinimumTouchTarget44pt(pipPill)
        pipPill.tap()
        _ = requireElement(FabushiAccessibilityIDs.MiniApp.webView, in: application, timeout: 10)
        recordCheckpoint("T1_F24_miniapp_webmcp_pip", passed: true, attachScreen: true)
    }

    // MARK: - F25: Chunked Media CAS, HTTP 206, 5-Bit Waveform, Stickers, Stories & Instant View

    func testT1_F25_mediaCASWaveformStickersStoriesAndInstantView() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f25_media")],
                chats: [FabushiLoopbackSeedChat(chatId: 125, title: "Rich Media Room")],
                messages: [
                    FabushiLoopbackSeedMessage(
                        chatId: 125,
                        text: "Voice note waveform demo",
                        messageType: "audio_voice",
                        mediaFileId: "cas_v168_audio_demo"
                    ),
                    FabushiLoopbackSeedMessage(
                        chatId: 125,
                        text: "Instant View: /article/v168",
                        messageType: "instant_view",
                        instantViewUrl: "/article/v168"
                    ),
                ],
                mediaBlobs: [
                    FabushiLoopbackSeedMediaBlob(
                        fileId: "cas_v168_audio_demo",
                        dataB64: "RkFCVVNISV9WMTY4X01FRElBX0JMT0I="
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f25_media", password: "FabushiV168!Pass")

        let storiesStrip = requireElement(FabushiAccessibilityIDs.ChatList.storiesStrip, in: application)
        assertWithinWindowBounds(storiesStrip, in: application)

        openFirstChat(in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Composer.stickerPickerButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let stickerGrid = requireElement(FabushiAccessibilityIDs.Composer.stickerGrid, in: application)
        assertWithinWindowBounds(stickerGrid, in: application)
        tapControl(FabushiAccessibilityIDs.Composer.stickerPickerButton, in: application)

        let mediaCard = requireElement(FabushiAccessibilityIDs.Stage.mediaPlayerCard, in: application)
        assertVoiceOverReady(mediaCard)
        let waveform = requireElement(FabushiAccessibilityIDs.Stage.waveformView, in: application)
        assertWithinWindowBounds(waveform, in: application)
        let ivCard = requireElement(FabushiAccessibilityIDs.Stage.instantViewCard, in: application)
        assertVoiceOverReady(ivCard)
        recordCheckpoint("T1_F25_media_stories_stickers_iv", passed: true)
    }

    // MARK: - F26: 1:1 & Group Calls (LiveKit/CallKit/ReplayKit) & Talk Mode <60ms VAD

    func testT1_F26_callsReplayKitScreenSharePickerAndTalkModeVADOrb() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f26_calls")],
                chats: [FabushiLoopbackSeedChat(chatId: 126, title: "Voice & Video Call Room")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f26_calls", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Stage.callStartButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let callOverlay = requireElement(FabushiAccessibilityIDs.Call.overlay, in: application)
        assertWithinWindowBounds(callOverlay, in: application)

        let fingerprint = requireElement(FabushiAccessibilityIDs.Call.fingerprintBadge, in: application)
        assertVoiceOverReady(fingerprint)
        let muteBtn = requireElement(FabushiAccessibilityIDs.Call.muteButton, in: application)
        assertMinimumTouchTarget44pt(muteBtn)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Call.screenShareButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )

        let sharePicker = element(FabushiAccessibilityIDs.Call.screenSharePicker, in: application)
        let shareUnavailable = element(FabushiAccessibilityIDs.Call.screenShareUnavailableBanner, in: application)
        let shareHandled = sharePicker.waitForExistence(timeout: 5) || shareUnavailable.waitForExistence(timeout: 3)
        XCTAssertTrue(shareHandled, "F26 ReplayKit screen-share tap must present RPSystemBroadcastPickerView or honest unavailable banner")

        tapControl(FabushiAccessibilityIDs.Call.hangupButton, in: application)
        tapControl(FabushiAccessibilityIDs.Stage.talkModeButton, in: application)
        let talkOrb = requireElement(FabushiAccessibilityIDs.Call.talkOrb, in: application)
        assertWithinWindowBounds(talkOrb, in: application)
        tapControl(FabushiAccessibilityIDs.Call.talkStopButton, in: application)
        recordCheckpoint("T1_F26_calls_replaykit_talk", passed: true)
    }

    // MARK: - F27: Unified Tantivy Full-Text + Hybrid Memory + Offline FTS5 Search

    func testT1_F27_unifiedSearchCJKQueryAndResultNavigation() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f27_search")],
                chats: [FabushiLoopbackSeedChat(chatId: 127, title: "Dharma Archive")],
                messages: [FabushiLoopbackSeedMessage(chatId: 127, text: "全球法布施 Unified Search v1.6.8")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f27_search", password: "FabushiV168!Pass")

        let searchField = requireElement(FabushiAccessibilityIDs.ChatList.searchField, in: application)
        assertVoiceOverReady(searchField)
        replaceText(in: searchField, with: "全球法布施")

        let resultsList = requireElement(FabushiAccessibilityIDs.ChatList.searchResultsList, in: application)
        assertWithinWindowBounds(resultsList, in: application)
        let hitPredicate = NSPredicate(format: "label CONTAINS[cd] %@ OR label CONTAINS[cd] %@", "全球法布施", "Dharma Archive")
        let hit = resultsList.descendants(matching: .any).matching(hitPredicate).firstMatch
        XCTAssertTrue(hit.waitForExistence(timeout: 6), "F27 unified CJK search must return seeded '全球法布施' / 'Dharma Archive' hit")
        hit.tap()
        let stageRoot = requireElement(FabushiAccessibilityIDs.Stage.root, in: application, timeout: 8)
        assertWithinWindowBounds(stageRoot, in: application)
        recordCheckpoint("T1_F27_unified_cjk_search", passed: true)
    }

    // MARK: - F28: E2EE Messaging (Double Ratchet, Sender Keys & 4-Emoji Safety Number)

    func testT1_F28_e2eeSecretChatAndSafetyNumberVerificationGrid() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f28_e2ee")],
                chats: [FabushiLoopbackSeedChat(chatId: 128, title: "Encrypted Peer", chatType: "private", memberCount: 2)]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f28_e2ee", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let e2eeBadge = requireElement(FabushiAccessibilityIDs.Stage.e2eeVerifiedBadge, in: application)
        assertVoiceOverReady(e2eeBadge)
        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Inspector.e2eeSafetyNumberButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let emojiGrid = requireElement(FabushiAccessibilityIDs.Inspector.e2eeSafetyEmojiGrid, in: application)
        assertWithinWindowBounds(emojiGrid, in: application)
        assertVoiceOverReady(emojiGrid)
        recordCheckpoint("T1_F28_e2ee_safety_number", passed: true)
    }

    // MARK: - F29: Economy, Stars Ledger, Gifts, Premium & StoreKit 2 IAP/Restore

    func testT1_F29_economyStarsLedgerAndStoreKit2RestoreDisclosure() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t1_f29_wallet")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f29_wallet", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionWalletStars, in: application)

        let balance = requireElement(FabushiAccessibilityIDs.Settings.starsBalanceLabel, in: application)
        assertVoiceOverReady(balance)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Settings.storeKitRestoreButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let disclosure = requireElement(FabushiAccessibilityIDs.Settings.storeKitDisclosureBanner, in: application)
        assertVoiceOverReady(disclosure)
        recordCheckpoint("T1_F29_stars_and_storekit2", passed: true)
    }

    // MARK: - F30: Action Bus, App Intents / Siri Shortcuts, fabushi:// URLs & APNs Status

    func testT1_F30_actionBusDeepLinkRoutingAndAPNsStatusDisplay() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f30_actions")],
                chats: [FabushiLoopbackSeedChat(chatId: 130, title: "DeepLink Target")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f30_actions", password: "FabushiV168!Pass")

        openDeepLink("fabushi://chat/130", in: application)
        let stageRoot = requireElement(FabushiAccessibilityIDs.Stage.root, in: application, timeout: 10)
        assertWithinWindowBounds(stageRoot, in: application)

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionNotifications, in: application)
        let apnsStatus = requireElement(FabushiAccessibilityIDs.Settings.apnsStatusLabel, in: application)
        assertVoiceOverReady(apnsStatus)
        recordCheckpoint("T1_F30_action_bus_and_apns", passed: true)
    }

    // MARK: - F31: Apple HIG Design System (4 Themes, WCAG >138 Guard, Liquid Glass, zh-Hans/en)

    func testT1_F31_appleHIGDesignSystemThemesAndLocaleSwitch() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t1_f31_hig")])
        )
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "night", accentHex: "#007AFF"))
        signInIfNeeded(in: application, username: "t1_f31_hig", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionAppearance, in: application)

        let themes = [
            FabushiAccessibilityIDs.Settings.themeNightButton,
            FabushiAccessibilityIDs.Settings.themeTintedButton,
            FabushiAccessibilityIDs.Settings.themeDayButton,
            FabushiAccessibilityIDs.Settings.themeClassicButton,
        ]
        for themeID in themes {
            tapControlWithHIGChecks(
                themeID,
                in: application,
                verifyMinTouchTarget: true,
                verifyWithinBounds: true,
                verifyVoiceOver: true
            )
        }
        let contrastBadge = requireElement(FabushiAccessibilityIDs.Settings.accentContrastBadge, in: application)
        assertVoiceOverReady(contrastBadge)
        recordCheckpoint("T1_F31_hig_themes_wcag", passed: true)
    }

    // MARK: - F32: Full 26-Section Apple Settings Suite, Search & Real Storage Purge

    func testT1_F32_full26SectionSettingsSearchAndRealStoragePurge() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t1_f32_settings")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f32_settings", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        let settingsRoot = requireElement(FabushiAccessibilityIDs.Settings.root, in: application)
        assertWithinWindowBounds(settingsRoot, in: application)

        let searchField = requireElement(FabushiAccessibilityIDs.Settings.searchField, in: application)
        replaceText(in: searchField, with: "Storage")
        tapControl(FabushiAccessibilityIDs.Settings.sectionDataStorage, in: application)

        let metricsLabel = requireElement(FabushiAccessibilityIDs.Settings.storageMetricsLabel, in: application)
        assertVoiceOverReady(metricsLabel)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Settings.storagePurgeCacheButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        recordCheckpoint("T1_F32_settings_26_and_storage", passed: true)
    }

    // MARK: - F33: Network, 4-Tier Config Cascade, Proxy Probe & App Update Check

    func testT1_F33_networkProxyConfigurationProbeAndAppStoreUpdateCheck() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t1_f33_net")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f33_net", password: "FabushiV168!Pass")

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionNetworkProxy, in: application)

        let hostField = requireElement(FabushiAccessibilityIDs.Settings.proxyHostField, in: application)
        replaceText(in: hostField, with: "127.0.0.1")
        let portField = requireElement(FabushiAccessibilityIDs.Settings.proxyPortField, in: application)
        replaceText(in: portField, with: "8080")
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Settings.proxyProbeButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let statusLabel = requireElement(FabushiAccessibilityIDs.Settings.proxyStatusLabel, in: application)
        assertVoiceOverReady(statusLabel)

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionUpdates, in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Settings.updateCheckButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let updateLabel = requireElement(FabushiAccessibilityIDs.Settings.updateStatusLabel, in: application)
        assertVoiceOverReady(updateLabel)
        recordCheckpoint("T1_F33_network_proxy_updates", passed: true)
    }

    // MARK: - F34: Fail-Closed Governance, Anti-Stub & Honest Capability Ledger

    func testT1_F34_failClosedGovernanceAndHonestCapabilityLedgerUI() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t1_f34_gov")])
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f34_gov", password: "FabushiV168!Pass")

        assertNoSampleChats(in: application)
        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionCapabilities, in: application)
        assertCapabilityLedgerShowsCanonicalIDs(in: application)

        let state = try await client.fetchState()
        XCTAssertEqual(state.version, "1.6.8", "F34 loopback authority version must match v1.6.8")
        recordCheckpoint("T1_F34_honest_capability_ledger", passed: true)
    }
}
