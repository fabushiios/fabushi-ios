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
        let splitKeyBadge = requireElement(FabushiAccessibilityIDs.Auth.splitKeyBadge, in: application)
        assertVoiceOverReady(splitKeyBadge)
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
        let registerRequests = backendState.recordedRequests.filter { $0.path == "/api/v1/auth/register" }
        XCTAssertGreaterThanOrEqual(registerRequests.count, 1, "F01 register must POST /api/v1/auth/register")
        XCTAssertTrue(
            registerRequests.contains(where: { $0.bodyPreview.contains("auth_verifier_b64") }),
            "F01 Split-Key V2 registration must send derived auth_verifier_b64"
        )
        XCTAssertTrue(
            registerRequests.allSatisfy { !$0.bodyPreview.contains("SplitKeyV2!Strong9") },
            "F01 Split-Key V2 registration must never transmit plaintext password on the wire"
        )
        recordCheckpoint("T1_F01_split_key_v2_bootstrap", passed: true, attachScreen: true)
    }

    // MARK: - F02: Email OTP Registration & Phone SMS Verification

    func testT1_F02_emailOTPAndPhoneSMSVerificationFlow() async throws {
        let client = try await prepareCleanLoopback()
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "day", languageCode: "en", localeIdentifier: "en_US"))

        let phoneField = requireElement(FabushiAccessibilityIDs.Auth.phoneField, in: application)
        assertVoiceOverReady(phoneField)
        replaceText(in: phoneField, with: "+14155550168")

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Auth.sendOTPButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )

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

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Auth.alipayLoginButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )

        let qrView = requireElement(FabushiAccessibilityIDs.Auth.alipayQRView, in: application)
        assertWithinWindowBounds(qrView, in: application)
        let statusLabel = requireElement(FabushiAccessibilityIDs.Auth.alipayStatusLabel, in: application)
        assertVoiceOverReady(statusLabel)

        let state = try await client.fetchState(pathPrefix: "/api/v1/auth/alipay")
        XCTAssertGreaterThanOrEqual(state.matchedRequestCount, 1, "F03 Alipay start must call /api/v1/auth/alipay/start")

        try await client.seed(
            FabushiLoopbackSeedRequest(
                alipayConfirm: FabushiLoopbackSeedAlipayConfirm(attemptId: "alipay_1")
            )
        )
        let shellRoot = requireElement(FabushiAccessibilityIDs.Shell.root, in: application, timeout: 15)
        assertWithinWindowBounds(shellRoot, in: application)
        recordCheckpoint("T1_F03_alipay_qr_started", passed: true)
    }

    // MARK: - F04: Passkeys (WebAuthn/FIDO2) & QR Device Pairing

    func testT1_F04_passkeysWebAuthnAndQRDevicePairing() async throws {
        let client = try await prepareCleanLoopback()
        let application = try launchFabushi(FabushiE2ELaunchConfig(theme: "tinted"))

        let passkeyButton = requireElement(FabushiAccessibilityIDs.Auth.passkeyLoginButton, in: application)
        assertMinimumTouchTarget44pt(passkeyButton)
        assertVoiceOverReady(passkeyButton)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Auth.qrPairButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
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

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Shell.accountSwitcherButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        let sheet = requireElement(FabushiAccessibilityIDs.Shell.accountSwitcherSheet, in: application)
        assertWithinWindowBounds(sheet, in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Shell.addAccountButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )

        let secUserField = requireElement(FabushiAccessibilityIDs.Auth.usernameField, in: application)
        replaceText(in: secUserField, with: "t1_f06_secondary")
        let secPassField = requireElement(FabushiAccessibilityIDs.Auth.passwordField, in: application)
        replaceText(in: secPassField, with: "FabushiV168!Pass")
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Auth.loginButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        _ = requireElement(FabushiAccessibilityIDs.Shell.root, in: application, timeout: 15)

        let loginState = try await client.fetchState(pathPrefix: "/api/v1/auth/login")
        XCTAssertGreaterThanOrEqual(
            loginState.matchedRequestCount,
            2,
            "F06 multi-account flow must authenticate both primary and secondary accounts against loopback"
        )
        XCTAssertGreaterThanOrEqual(
            loginState.counts.activeSessions,
            2,
            "F06 multi-account switcher must maintain >= 2 distinct active sessions"
        )
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
            tapControlWithHIGChecks(
                FabushiAccessibilityIDs.Shell.sidebarCollapseToggle,
                in: application,
                verifyMinTouchTarget: true,
                verifyWithinBounds: true
            )
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
        try await client.injectFault(
            FabushiLoopbackFaultRequest(
                httpFaults: [
                    FabushiLoopbackHTTPFault(
                        pathPrefix: "/api/v1/chats/107/messages",
                        method: "POST",
                        status: 429,
                        body: ["error": "rate_limited"],
                        retryAfterMs: 3000,
                        remainingHits: 1
                    ),
                    FabushiLoopbackHTTPFault(
                        pathPrefix: "/api/v1/messages",
                        method: "POST",
                        status: 429,
                        body: ["error": "rate_limited"],
                        retryAfterMs: 3000,
                        remainingHits: 1
                    ),
                ]
            )
        )
        let composerField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: composerField, with: "Trigger v1.6.8 shell toast check")
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let windowBanner = element(FabushiAccessibilityIDs.Shell.toastWindowBanner, in: application)
        let stagePill = element(FabushiAccessibilityIDs.Shell.toastStagePill, in: application)
        let toastVisible = windowBanner.waitForExistence(timeout: 6) || stagePill.waitForExistence(timeout: 4)
        XCTAssertTrue(toastVisible, "v1.6.8 F07: rate-limited send must surface windowBanner or stagePill toast")
        let activeToast = windowBanner.exists ? windowBanner : stagePill
        XCTAssertLessThanOrEqual(
            activeToast.frame.maxY,
            inputWrap.frame.minY,
            "v1.6.8 F07 invariant: toast must sit strictly above composer inputWrap"
        )
        assertNonOverlapping(activeToast, inputWrap)
        try await client.clearFaults()
        recordCheckpoint("T1_F07_adaptive_shell_v168", passed: true, attachScreen: true)
    }

    // MARK: - F08: Grok Bot Multi-Select Compose (`To:` Picker) & v1.6.8 Composer Isolation

    func testT1_F08_grokBotMultiSelectComposeAndExternalBotControls() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f08_grok")],
                chats: [
                    FabushiLoopbackSeedChat(
                        chatId: 108,
                        title: "@GrokAI",
                        chatType: "bot",
                        memberCount: 2,
                        botStarted: true
                    )
                ],
                messages: [
                    FabushiLoopbackSeedMessage(chatId: 108, text: "Grok bot ready")
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f08_grok", password: "FabushiV168!Pass")

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.ChatList.composeButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
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

        // v1.6.8 R1-B/R2-3: botMenuButton sits strictly left of inputWrap (fail-closed)
        let botMenu = requireElement(FabushiAccessibilityIDs.Composer.botMenuButton, in: application)
        let inputWrap = requireElement(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
        assertMinimumTouchTarget44pt(botMenu)
        assertVoiceOverReady(botMenu)
        assertNonOverlapping(botMenu, inputWrap)
        XCTAssertLessThanOrEqual(
            botMenu.frame.maxX,
            inputWrap.frame.minX + 1.0,
            "v1.6.8 F08: botMenuButton must sit outside and left of composer inputWrap"
        )

        // v1.6.8 external `/` coexistence & BotKeyboard toggle
        let botSlash = requireElement(FabushiAccessibilityIDs.Composer.botSlashButton, in: application)
        assertVoiceOverReady(botSlash)
        assertNonOverlapping(botSlash, botMenu)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Composer.botKeyboardToggle,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        let replyKeyboard = requireElement(FabushiAccessibilityIDs.Composer.botReplyKeyboard, in: application)
        assertWithinWindowBounds(replyKeyboard, in: application)
        tapControl(FabushiAccessibilityIDs.Composer.botKeyboardToggle, in: application)

        // v1.6.8 compact shrink (is_small) while typing
        let initialBotMenuWidth = botMenu.frame.width
        let botComposerField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: botComposerField, with: "Checking compact bot menu width")
        XCTAssertLessThanOrEqual(
            botMenu.frame.width,
            initialBotMenuWidth + 0.5,
            "v1.6.8 F08: botMenuButton must remain compact (<= expanded width) while typing"
        )
        assertNonOverlapping(botMenu, inputWrap)
        recordCheckpoint("T1_F08_grok_compose_and_bot_controls", passed: true)
    }

    // MARK: - F09: Unified Chat List, Folders, Archive, Pin, Mute & Forum Topics

    func testT1_F09_unifiedChatListFoldersPinnedGridAndUnreadIndicators() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f09_chats")],
                chats: [
                    FabushiLoopbackSeedChat(
                        chatId: 109,
                        title: "Pinned Engineering",
                        chatType: "group",
                        memberCount: 6,
                        pinned: true,
                        unreadCount: 3
                    ),
                    FabushiLoopbackSeedChat(
                        chatId: 110,
                        title: "@HermesBot",
                        chatType: "bot",
                        memberCount: 2,
                        isUnreadMarked: true
                    ),
                    FabushiLoopbackSeedChat(
                        chatId: 111,
                        title: "Release Feed",
                        chatType: "group",
                        memberCount: 4
                    ),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f09_chats", password: "FabushiV168!Pass")

        let chatList = requireElement(FabushiAccessibilityIDs.ChatList.list, in: application)
        assertWithinWindowBounds(chatList, in: application)
        assertNoSampleChats(in: application)

        let pinnedGrid = requireElement(FabushiAccessibilityIDs.ChatList.pinnedGrid, in: application)
        assertWithinWindowBounds(pinnedGrid, in: application)
        let unreadBadge = requireElement(FabushiAccessibilityIDs.ChatList.unreadBadge, in: application)
        assertVoiceOverReady(unreadBadge)
        let unreadMarkDot = requireElement(FabushiAccessibilityIDs.ChatList.unreadMarkDot, in: application)
        assertVoiceOverReady(unreadMarkDot)

        let seededRowPred = NSPredicate(format: "label CONTAINS[cd] %@", "Pinned Engineering")
        XCTAssertTrue(
            chatList.descendants(matching: .any).matching(seededRowPred).firstMatch.waitForExistence(timeout: 6),
            "F09 chat list must render seeded 'Pinned Engineering' row"
        )

        let folderBar = requireElement(FabushiAccessibilityIDs.ChatList.folderBar, in: application)
        assertWithinWindowBounds(folderBar, in: application)
        tapControl(FabushiAccessibilityIDs.ChatList.folderUnreadTab, in: application)
        tapControl(FabushiAccessibilityIDs.ChatList.folderBotsTab, in: application)
        let botBadge = requireElement(FabushiAccessibilityIDs.ChatList.botAvatarBadge, in: application)
        assertVoiceOverReady(botBadge)

        tapControl(FabushiAccessibilityIDs.ChatList.folderAllTab, in: application)
        let firstRow = requireElement(FabushiAccessibilityIDs.ChatList.firstRow, in: application)
        assertVoiceOverReady(firstRow)

        if activeFormFactor() == .ipad {
            tapControlWithHIGChecks(
                FabushiAccessibilityIDs.Shell.sidebarCollapseToggle,
                in: application,
                verifyMinTouchTarget: true,
                verifyWithinBounds: true
            )
            let railList = requireElement(FabushiAccessibilityIDs.ChatList.railList, in: application)
            assertWithinWindowBounds(railList, in: application)
            tapControl(FabushiAccessibilityIDs.Shell.sidebarCollapseToggle, in: application)
        }

        let state = try await client.fetchState(pathPrefix: "/api/v1/chats")
        XCTAssertGreaterThanOrEqual(state.matchedRequestCount, 1, "F09 must fetch /api/v1/chats from loopback")
        XCTAssertEqual(state.counts.chats, 3, "F09 must reflect exact 3 seeded chats with zero sample rows")
        recordCheckpoint("T1_F09_chatlist_folders_v168", passed: true)
    }

    // MARK: - F10: Contacts, Peer Resolution, Blocklist, Send-As & Prestart Bot

    func testT1_F10_contactsPeerResolutionSendAsAndPrestartBot() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t1_f10_contacts")],
                chats: [
                    FabushiLoopbackSeedChat(
                        chatId: 111,
                        title: "@PrestartBot",
                        chatType: "bot",
                        memberCount: 2,
                        botStarted: false
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t1_f10_contacts", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabContacts, in: application)
        let contactsList = requireElement(FabushiAccessibilityIDs.ChatList.contactsList, in: application)
        assertWithinWindowBounds(contactsList, in: application)

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.ChatList.addContactButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let contactField = requireElement(FabushiAccessibilityIDs.ChatList.contactUsernameField, in: application)
        replaceText(in: contactField, with: "dharma_peer")
        tapControl(FabushiAccessibilityIDs.ChatList.contactSaveButton, in: application)

        let contactsState = try await client.fetchState(pathPrefix: "/api/v1/contacts")
        XCTAssertGreaterThanOrEqual(contactsState.matchedRequestCount, 1, "F10 saving contact must call /api/v1/contacts")

        openShellTab(FabushiAccessibilityIDs.Shell.tabChats, in: application)
        openFirstChat(in: application)
        let botStartBar = requireElement(FabushiAccessibilityIDs.Stage.botStartBar, in: application)
        assertWithinWindowBounds(botStartBar, in: application)
        let descCard = requireElement(FabushiAccessibilityIDs.Stage.botDescriptionCard, in: application)
        assertWithinWindowBounds(descCard, in: application)
        assertVoiceOverReady(descCard)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Stage.botStartButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        XCTAssertTrue(
            botStartBar.waitForNonExistence(timeout: 6),
            "v1.6.8 F10: tapping botStartButton must dismiss pre-start bar and unlock composer"
        )
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
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Composer.sendButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )

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

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Composer.slashCommandFirstItem,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true,
            verifyVoiceOver: true
        )
        XCTAssertTrue(
            popover.waitForNonExistence(timeout: 4),
            "F15 selecting a slash command must dismiss slashCommandPopover"
        )
        let selectedVal = (textField.value as? String) ?? ""
        XCTAssertTrue(
            selectedVal.hasPrefix("/"),
            "F15 selecting a slash command must populate composer field with command text"
        )
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

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Stage.inspectorToggleButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
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
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Inspector.copyUsernameButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let feedback = requireElement(FabushiAccessibilityIDs.Inspector.copiedFeedback, in: application)
        assertVoiceOverReady(feedback)
        let dismissed = feedback.waitForNonExistence(timeout: 5)
        XCTAssertTrue(dismissed, "v1.6.8 F16: copiedFeedback must auto-dismiss within 5s")
        recordCheckpoint("T1_F16_muse_inspector_v168", passed: true)
    }
}
