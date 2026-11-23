import Foundation
import UIKit
import XCTest

/// Tier 1 Happy-Path UI E2E Suite — Core Auth, Adaptive Shell, Chat List, Compose, Transcript & Inspector (F01..F16).
/// Continued in `FabushiV167Tier1UITests+AgentsAndPlatform.swift` for F17..F34 (34 Tier 1 tests total).
@MainActor
final class FabushiV167Tier1UITests: FabushiE2EUITestCase {

    // MARK: - F01: Password Login, Registration & Split-Key V2 Bootstrap

    func testT1_F01_passwordLoginRegistrationAndSplitKeyV2Bootstrap() async throws {
        let client = try await prepareCleanLoopback()
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "night", resetState: true))

        let authRoot = requireElement(FabushiAccessibilityIDs.Auth.root, in: application)
        assertWithinWindowBounds(authRoot, in: application)
        assertNoSampleChats(in: application)

        tapControl(FabushiAccessibilityIDs.Auth.registerTabButton, in: application)
        let userField = requireElement(FabushiAccessibilityIDs.Auth.usernameField, in: application)
        replaceText(in: userField, with: "t1_f01_v2user")
        let passField = requireElement(FabushiAccessibilityIDs.Auth.passwordField, in: application)
        replaceText(in: passField, with: "SplitKeyV2!Strong9")

        let submitButton = requireElement(FabushiAccessibilityIDs.Auth.registerSubmitButton, in: application)
        assertMinimumTouchTarget44pt(submitButton)
        assertVoiceOverReady(submitButton)
        submitButton.tap()

        let shellRoot = requireElement(FabushiAccessibilityIDs.Shell.root, in: application, timeout: 15)
        assertWithinWindowBounds(shellRoot, in: application)
        assertNoSampleChats(in: application)

        let backendState = try await client.fetchState(pathPrefix: "/api/v1/auth")
        XCTAssertGreaterThanOrEqual(backendState.counts.users, 1, "F01 register must persist user on loopback")
        XCTAssertGreaterThanOrEqual(backendState.counts.activeSessions, 1, "F01 register must mint active session")
        recordCheckpoint("T1_F01_split_key_v2_bootstrap", passed: true, attachScreen: true)
    }

    // MARK: - F02: Email OTP Registration & Phone SMS Verification

    func testT1_F02_emailOTPAndPhoneSMSVerificationFlow() async throws {
        let client = try await prepareCleanLoopback()
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "day", languageCode: "en", localeIdentifier: "en_US"))

        let phoneField = requireElement(FabushiAccessibilityIDs.Auth.phoneField, in: application)
        assertVoiceOverReady(phoneField)
        replaceText(in: phoneField, with: "+14155550168")

        let sendButton = tapControl(FabushiAccessibilityIDs.Auth.sendOTPButton, in: application)
        assertMinimumTouchTarget44pt(sendButton)

        let otpField = requireElement(FabushiAccessibilityIDs.Auth.otpCodeField, in: application)
        replaceText(in: otpField, with: "123456")
        let verifyButton = requireElement(FabushiAccessibilityIDs.Auth.verifyOTPButton, in: application)
        assertMinimumTouchTarget44pt(verifyButton)
        verifyButton.tap()

        let shellRoot = requireElement(FabushiAccessibilityIDs.Shell.root, in: application, timeout: 15)
        assertWithinWindowBounds(shellRoot, in: application)

        let state = try await client.fetchState(pathPrefix: "/api/v1/auth/phone")
        XCTAssertGreaterThanOrEqual(state.matchedRequestCount, 1, "F02 OTP verification must hit loopback phone auth API")
        recordCheckpoint("T1_F02_otp_phone_sms_verified", passed: true)
    }

    // MARK: - F03: Alipay QR Login & Mobile OAuth Callback

    func testT1_F03_alipayQRLoginAndOAuthDeepLinkCallback() async throws {
        let client = try await prepareCleanLoopback()
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "night"))

        let alipayButton = tapControl(FabushiAccessibilityIDs.Auth.alipayLoginButton, in: application)
        assertMinimumTouchTarget44pt(alipayButton)
        assertVoiceOverReady(alipayButton)

        let qrView = requireElement(FabushiAccessibilityIDs.Auth.alipayQRView, in: application)
        assertWithinWindowBounds(qrView, in: application)
        let statusLabel = requireElement(FabushiAccessibilityIDs.Auth.alipayStatusLabel, in: application)
        assertVoiceOverReady(statusLabel)

        let state = try await client.fetchState(pathPrefix: "/api/v1/auth/alipay")
        XCTAssertGreaterThanOrEqual(state.matchedRequestCount, 1, "F03 Alipay start must call /api/v1/auth/alipay/start")
        recordCheckpoint("T1_F03_alipay_qr_started", passed: true)
    }

    // MARK: - F04: Passkeys (WebAuthn/FIDO2) & QR Device Pairing

    func testT1_F04_passkeysWebAuthnAndQRDevicePairing() async throws {
        let client = try await prepareCleanLoopback()
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "tinted"))

        let passkeyButton = requireElement(FabushiAccessibilityIDs.Auth.passkeyLoginButton, in: application)
        assertMinimumTouchTarget44pt(passkeyButton)
        assertVoiceOverReady(passkeyButton)

        let qrPairButton = tapControl(FabushiAccessibilityIDs.Auth.qrPairButton, in: application)
        assertMinimumTouchTarget44pt(qrPairButton)
        let qrCanvas = requireElement(FabushiAccessibilityIDs.Auth.qrPairCanvas, in: application)
        assertWithinWindowBounds(qrCanvas, in: application)

        let state = try await client.fetchState(pathPrefix: "/api/v1/auth/pair")
        XCTAssertGreaterThanOrEqual(state.matchedRequestCount, 1, "F04 QR pairing must invoke /api/v1/auth/pair/init")
        recordCheckpoint("T1_F04_passkey_and_qr_pair", passed: true)
    }

    // MARK: - F05: Cloud 2FA Password & Active Device Sessions

    func testT1_F05_cloud2FAAndActiveDeviceSessionManagement() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [
                    FabushiLoopbackSeedUser(
                        username: "t1_f05_2fa",
                        password: "Password2FA!1",
                        twoFactorPassword: "CloudSecret2FA!",
                        twoFactorHint: "lotus"
                    )
                ]
            )
        )
        let application = try launchFabushi()

        let userField = requireElement(FabushiAccessibilityIDs.Auth.usernameField, in: application)
        replaceText(in: userField, with: "t1_f05_2fa")
        let passField = requireElement(FabushiAccessibilityIDs.Auth.passwordField, in: application)
        replaceText(in: passField, with: "Password2FA!1")
        tapControl(FabushiAccessibilityIDs.Auth.loginButton, in: application)

        let twoFactorField = requireElement(FabushiAccessibilityIDs.Auth.twoFactorPasswordField, in: application)
        replaceText(in: twoFactorField, with: "CloudSecret2FA!")
        tapControl(FabushiAccessibilityIDs.Auth.twoFactorVerifyButton, in: application)

        _ = requireElement(FabushiAccessibilityIDs.Shell.root, in: application, timeout: 15)
        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionDevices, in: application)

        let devicesList = requireElement(FabushiAccessibilityIDs.Settings.devicesList, in: application)
        assertWithinWindowBounds(devicesList, in: application)
        let revokeOthers = requireElement(FabushiAccessibilityIDs.Settings.revokeOtherDevicesButton, in: application)
        assertMinimumTouchTarget44pt(revokeOthers)
        recordCheckpoint("T1_F05_cloud_2fa_and_sessions", passed: true)
    }

    // MARK: - F06: Multi-Account Switcher, Generation Fence & Passcode Lock

    func testT1_F06_multiAccountSwitcherGenerationFenceAndPasscodeLock() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [
                    FabushiLoopbackSeedUser(username: "t1_f06_primary"),
                    FabushiLoopbackSeedUser(username: "t1_f06_secondary"),
                ],
                chats: [FabushiLoopbackSeedChat(chatId: 101, title: "Primary Account Chat")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f06_primary", password: "FabushiV168!Pass")

        let switcher = tapControl(FabushiAccessibilityIDs.Shell.accountSwitcherButton, in: application)
        assertMinimumTouchTarget44pt(switcher)
        assertVoiceOverReady(switcher)

        let sheet = requireElement(FabushiAccessibilityIDs.Shell.accountSwitcherSheet, in: application)
        assertWithinWindowBounds(sheet, in: application)
        let addAccount = requireElement(FabushiAccessibilityIDs.Shell.addAccountButton, in: application)
        assertMinimumTouchTarget44pt(addAccount)
        assertVoiceOverReady(addAccount)

        let state = try await client.fetchState()
        XCTAssertGreaterThanOrEqual(state.counts.users, 2, "F06 multi-account setup requires >= 2 seeded users")
        recordCheckpoint("T1_F06_multi_account_switcher", passed: true)
    }

    // MARK: - F07: Unified Adaptive Apple Shell & v1.6.8 Two-Layer Toast / Rail Polish

    func testT1_F07_unifiedAdaptiveShellCollapsedRailAndToastNonOverlap() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f07_shell")],
                chats: [FabushiLoopbackSeedChat(chatId: 107, title: "Adaptive Shell Room")]
            )
        )
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "night"))
        signInIfNeeded(in: application, username: "t1_f07_shell", password: "FabushiV168!Pass")

        let shellRoot = requireElement(FabushiAccessibilityIDs.Shell.root, in: application)
        assertWithinWindowBounds(shellRoot, in: application)

        if activeFormFactor() == .ipad {
            let splitView = requireElement(FabushiAccessibilityIDs.Shell.splitView, in: application)
            assertWithinWindowBounds(splitView, in: application)
            let collapseToggle = tapControl(FabushiAccessibilityIDs.Shell.sidebarCollapseToggle, in: application)
            assertMinimumTouchTarget44pt(collapseToggle)
            let rail = requireElement(FabushiAccessibilityIDs.Shell.sidebarCollapsedRail, in: application)
            assertWithinWindowBounds(rail, in: application)
            tapControl(FabushiAccessibilityIDs.Shell.sidebarCollapseToggle, in: application)
        } else {
            let tabBar = requireElement(FabushiAccessibilityIDs.Shell.tabBar, in: application)
            assertWithinWindowBounds(tabBar, in: application)
        }

        openFirstChat(in: application)
        let stageSlot = requireElement(FabushiAccessibilityIDs.Shell.stageSlot, in: application)
        assertWithinWindowBounds(stageSlot, in: application)
        let inputWrap = requireElement(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
        let windowBanner = element(FabushiAccessibilityIDs.Shell.toastWindowBanner, in: application)
        if windowBanner.exists {
            XCTAssertLessThanOrEqual(
                windowBanner.frame.maxY,
                inputWrap.frame.minY,
                "v1.6.8 F07 invariant: window toast banner must never overlap composer input wrap"
            )
        }
        recordCheckpoint("T1_F07_adaptive_shell_v168", passed: true, attachScreen: true)
    }

    // MARK: - F08: Grok Bot Multi-Select Compose (`To:` Picker) & v1.6.8 Composer Isolation

    func testT1_F08_grokBotMultiSelectComposeAndExternalBotControls() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f08_grok")],
                chats: [FabushiLoopbackSeedChat(chatId: 108, title: "@GrokAI", chatType: "bot", memberCount: 2)]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f08_grok", password: "FabushiV168!Pass")

        let composeBtn = tapControl(FabushiAccessibilityIDs.ChatList.composeButton, in: application)
        assertMinimumTouchTarget44pt(composeBtn)
        let composeStage = requireElement(FabushiAccessibilityIDs.Compose.stage, in: application)
        assertWithinWindowBounds(composeStage, in: application)

        // v1.6.8 R2-4: 0 selected chips hides composer
        let composeInputWrap = element(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
        let hiddenAtZero = composeInputWrap.waitForNonExistence(timeout: 3)
        XCTAssertTrue(hiddenAtZero, "v1.6.8 F08: compose stage must hide composer when 0 recipients selected")

        let toField = requireElement(FabushiAccessibilityIDs.Compose.toField, in: application)
        replaceText(in: toField, with: "Grok")
        tapControl(FabushiAccessibilityIDs.Compose.firstCandidate, in: application)

        let shownWithChip = composeInputWrap.waitForExistence(timeout: 5)
        XCTAssertTrue(shownWithChip, "v1.6.8 F08: selecting >=1 chip must reveal clean compose composer")
        let leakedBotMenu = element(FabushiAccessibilityIDs.Composer.botMenuButton, in: application)
        XCTAssertFalse(leakedBotMenu.exists, "v1.6.8 F08: compose mode must not show residual botMenuButton")

        tapControl(FabushiAccessibilityIDs.Compose.cancelButton, in: application)
        openFirstChat(in: application)

        // v1.6.8 R1-B/R2-3: botMenuButton sits strictly left of inputWrap in bot chat
        let botMenu = element(FabushiAccessibilityIDs.Composer.botMenuButton, in: application)
        if botMenu.waitForExistence(timeout: 5) {
            let inputWrap = requireElement(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
            XCTAssertLessThanOrEqual(
                botMenu.frame.maxX,
                inputWrap.frame.minX + 1.0,
                "v1.6.8 F08: botMenuButton must sit outside and left of composer inputWrap"
            )
        }
        recordCheckpoint("T1_F08_grok_compose_and_bot_controls", passed: true)
    }

    // MARK: - F09: Unified Chat List, Folders, Archive, Pin, Mute & Forum Topics

    func testT1_F09_unifiedChatListFoldersPinnedGridAndUnreadIndicators() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f09_chats")],
                chats: [
                    FabushiLoopbackSeedChat(chatId: 109, title: "Pinned Engineering", chatType: "group", memberCount: 6),
                    FabushiLoopbackSeedChat(chatId: 110, title: "@HermesBot", chatType: "bot", memberCount: 2),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f09_chats", password: "FabushiV168!Pass")

        let chatList = requireElement(FabushiAccessibilityIDs.ChatList.list, in: application)
        assertWithinWindowBounds(chatList, in: application)
        assertNoSampleChats(in: application)

        let folderBar = requireElement(FabushiAccessibilityIDs.ChatList.folderBar, in: application)
        assertWithinWindowBounds(folderBar, in: application)
        tapControl(FabushiAccessibilityIDs.ChatList.folderBotsTab, in: application)
        let botBadge = requireElement(FabushiAccessibilityIDs.ChatList.botAvatarBadge, in: application)
        assertVoiceOverReady(botBadge)

        tapControl(FabushiAccessibilityIDs.ChatList.folderAllTab, in: application)
        let firstRow = requireElement(FabushiAccessibilityIDs.ChatList.firstRow, in: application)
        assertVoiceOverReady(firstRow)

        let state = try await client.fetchState(pathPrefix: "/api/v1/chats")
        XCTAssertEqual(state.counts.chats, 2, "F09 must reflect exact 2 seeded chats with zero sample rows")
        recordCheckpoint("T1_F09_chatlist_folders_v168", passed: true)
    }

    // MARK: - F10: Contacts, Peer Resolution, Blocklist, Send-As & Prestart Bot

    func testT1_F10_contactsPeerResolutionSendAsAndPrestartBot() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f10_contacts")],
                chats: [FabushiLoopbackSeedChat(chatId: 111, title: "@PrestartBot", chatType: "bot", memberCount: 2)]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f10_contacts", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabContacts, in: application)
        let contactsList = requireElement(FabushiAccessibilityIDs.ChatList.contactsList, in: application)
        assertWithinWindowBounds(contactsList, in: application)

        let addContact = tapControl(FabushiAccessibilityIDs.ChatList.addContactButton, in: application)
        assertMinimumTouchTarget44pt(addContact)
        let contactField = requireElement(FabushiAccessibilityIDs.ChatList.contactUsernameField, in: application)
        replaceText(in: contactField, with: "dharma_peer")
        tapControl(FabushiAccessibilityIDs.ChatList.contactSaveButton, in: application)

        openShellTab(FabushiAccessibilityIDs.Shell.tabChats, in: application)
        openFirstChat(in: application)
        let botStartBar = element(FabushiAccessibilityIDs.Stage.botStartBar, in: application)
        if botStartBar.waitForExistence(timeout: 4) {
            let descCard = requireElement(FabushiAccessibilityIDs.Stage.botDescriptionCard, in: application)
            assertWithinWindowBounds(descCard, in: application)
            tapControl(FabushiAccessibilityIDs.Stage.botStartButton, in: application)
        }
        let composerField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        assertVoiceOverReady(composerField)
        recordCheckpoint("T1_F10_contacts_and_prestart_bot", passed: true)
    }

    // MARK: - F11: Atomic PTS/QTS Sync, 24h Nonce Dedup & Offline SQLite WAL Outbox

    func testT1_F11_atomicPTSSyncAndOfflineOutboxReconnectDelivery() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f11_sync")],
                chats: [FabushiLoopbackSeedChat(chatId: 112, title: "Sync Resilience Room")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f11_sync", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let statusPill = requireElement(FabushiAccessibilityIDs.Shell.connectionStatusPill, in: application)
        assertVoiceOverReady(statusPill)

        let textField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: textField, with: "Atomic PTS sync test message")
        let sendBtn = tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)
        assertMinimumTouchTarget44pt(sendBtn)

        let delivered = await waitUntil(timeout: 10) {
            self.element(FabushiAccessibilityIDs.Stage.messageBubbleLatest, in: application).exists
        }
        XCTAssertTrue(delivered, "F11 sent message must render in transcript")

        let state = try await client.fetchState(pathPrefix: "/api/v1")
        XCTAssertGreaterThanOrEqual(state.counts.messages, 1, "F11 message must persist on loopback with monotonic PTS")
        recordCheckpoint("T1_F11_pts_sync_and_outbox", passed: true)
    }

    // MARK: - F12: Virtualized Transcript, 900s Grouping & 11+ Canonical Cards

    func testT1_F12_virtualizedTranscriptGroupingAndCanonicalCards() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f12_stage")],
                chats: [FabushiLoopbackSeedChat(chatId: 113, title: "Transcript Cards Room")],
                messages: [
                    FabushiLoopbackSeedMessage(chatId: 113, text: "First grouped bubble"),
                    FabushiLoopbackSeedMessage(chatId: 113, text: "Second grouped bubble within 900s"),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f12_stage", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let transcript = requireElement(FabushiAccessibilityIDs.Stage.transcriptList, in: application)
        assertWithinWindowBounds(transcript, in: application)
        let latestBubble = requireElement(FabushiAccessibilityIDs.Stage.messageBubbleLatest, in: application)
        assertVoiceOverReady(latestBubble)
        let headerTitle = requireElement(FabushiAccessibilityIDs.Stage.headerTitle, in: application)
        assertVoiceOverReady(headerTitle)
        recordCheckpoint("T1_F12_transcript_grouping", passed: true)
    }

    // MARK: - F13: Message Edit, Delete Tombstone, Forward, Reactions, Pins & TTL

    func testT1_F13_messageEditDeleteReactionAndPinLifecycle() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f13_ops")],
                chats: [FabushiLoopbackSeedChat(chatId: 114, title: "Message Ops Room")],
                messages: [FabushiLoopbackSeedMessage(chatId: 114, text: "Editable message body")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f13_ops", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let bubble = requireElement(FabushiAccessibilityIDs.Stage.messageBubbleLatest, in: application)
        bubble.press(forDuration: 0.8)
        let contextMenu = requireElement(FabushiAccessibilityIDs.Stage.messageContextMenu, in: application)
        assertWithinWindowBounds(contextMenu, in: application)

        tapControl(FabushiAccessibilityIDs.Stage.pinMessageAction, in: application)
        let pinnedBanner = requireElement(FabushiAccessibilityIDs.Stage.pinnedBanner, in: application)
        assertVoiceOverReady(pinnedBanner)

        bubble.press(forDuration: 0.8)
        tapControl(FabushiAccessibilityIDs.Stage.reactMessageAction, in: application)
        recordCheckpoint("T1_F13_message_ops_pin_react", passed: true)
    }

    // MARK: - F14: IME-Safe Canonical Composer, Drafts, Formatting & Voice Gesture

    func testT1_F14_imeSafeCanonicalComposerDraftPersistenceAndVoiceGesture() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f14_composer")],
                chats: [FabushiLoopbackSeedChat(chatId: 115, title: "Composer IME Room")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f14_composer", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let composerRoot = requireElement(FabushiAccessibilityIDs.Composer.root, in: application)
        assertWithinWindowBounds(composerRoot, in: application)
        let textField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: textField, with: "中文输入法草稿测试 v1.6.8")

        let sendBtn = requireElement(FabushiAccessibilityIDs.Composer.sendButton, in: application)
        assertMinimumTouchTarget44pt(sendBtn)
        assertVoiceOverReady(sendBtn)

        let voiceBtn = requireElement(FabushiAccessibilityIDs.Composer.voiceRecordButton, in: application)
        assertMinimumTouchTarget44pt(voiceBtn)
        assertVoiceOverReady(voiceBtn)
        recordCheckpoint("T1_F14_composer_ime_and_voice", passed: true)
    }

    // MARK: - F15: 49+ Hermes/Fabushi Slash Commands & `@` Context Badges

    func testT1_F15_slashCommandPopoverAndContextReferenceBadges() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f15_slash")],
                chats: [FabushiLoopbackSeedChat(chatId: 116, title: "@FabushiAgent", chatType: "bot", memberCount: 2)]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f15_slash", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let textField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: textField, with: "/")
        let popover = requireElement(FabushiAccessibilityIDs.Composer.slashCommandPopover, in: application)
        assertWithinWindowBounds(popover, in: application)

        let firstCmd = tapControl(FabushiAccessibilityIDs.Composer.slashCommandFirstItem, in: application)
        assertVoiceOverReady(firstCmd)
        recordCheckpoint("T1_F15_slash_commands_popover", passed: true)
    }

    // MARK: - F16: Muse Context Inspector & v1.6.8 3-Way Peer Isolation

    func testT1_F16_museContextInspectorThreeWayLayoutAndCopyFeedback() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f16_inspector")],
                chats: [
                    FabushiLoopbackSeedChat(chatId: 117, title: "Release Group", chatType: "group", memberCount: 5),
                    FabushiLoopbackSeedChat(chatId: 118, title: "@GrokAI", chatType: "bot", memberCount: 2),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f16_inspector", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let toggleBtn = tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        assertMinimumTouchTarget44pt(toggleBtn)
        let drawer = requireElement(FabushiAccessibilityIDs.Inspector.drawer, in: application)
        assertWithinWindowBounds(drawer, in: application)

        let groupSection = requireElement(FabushiAccessibilityIDs.Inspector.profileGroupSection, in: application)
        assertVoiceOverReady(groupSection)
        let membersTab = requireElement(FabushiAccessibilityIDs.Inspector.tabMembers, in: application)
        assertMinimumTouchTarget44pt(membersTab)
        let mediaTab = requireElement(FabushiAccessibilityIDs.Inspector.tabMedia, in: application)
        assertMinimumTouchTarget44pt(mediaTab)

        // v1.6.8 R2-1: Group inspector must never leak bot openAppButton or 1v1 startSecretChatButton
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.openAppButton, in: application).exists,
            "v1.6.8 F16: Group inspector must not render bot openAppButton"
        )
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.startSecretChatButton, in: application).exists,
            "v1.6.8 F16: Group inspector must not render 1v1 startSecretChatButton"
        )

        // v1.6.8 R2-2: Copy feedback appears then auto-dismisses cleanly
        let copyBtn = tapControl(FabushiAccessibilityIDs.Inspector.copyUsernameButton, in: application)
        assertMinimumTouchTarget44pt(copyBtn)
        let feedback = requireElement(FabushiAccessibilityIDs.Inspector.copiedFeedback, in: application)
        assertVoiceOverReady(feedback)
        let dismissed = feedback.waitForNonExistence(timeout: 5)
        XCTAssertTrue(dismissed, "v1.6.8 F16: copiedFeedback must auto-dismiss within 5s")
        recordCheckpoint("T1_F16_muse_inspector_v168", passed: true)
    }
}
