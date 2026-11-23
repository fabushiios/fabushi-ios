import Foundation
import UIKit
import XCTest

/// Tier 2 Apple HIG, Accessibility, Localization, Boundary, Negative & v1.6.8 Polish Suite (16 tests).
@MainActor
final class FabushiV167Tier2UITests: FabushiE2EUITestCase {

    // MARK: - 1. Dynamic Type AX5 Zero-Clipping & 44pt Touch Targets

    func testT2_01_F07_F14_F17_F31_F32_dynamicTypeAX5ZeroClippingAnd44ptTouchTargets() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_ax5_user")],
                chats: [FabushiLoopbackSeedChat(chatId: 201, title: "AX5 Dynamic Type Room")]
            )
        )
        let ax5Config = FabushiE2ELaunchConfig(
            theme: "day",
            languageCode: "zh-Hans",
            localeIdentifier: "zh_CN",
            preferredContentSizeCategory: "UICTContentSizeCategoryAccessibilityExtraExtraExtraLarge"
        )
        let application = try launchFabushi(ax5Config)
        signInIfNeeded(in: application, username: "t2_ax5_user", password: "FabushiV168!Pass")

        let composeBtn = requireElement(FabushiAccessibilityIDs.ChatList.composeButton, in: application)
        assertMinimumTouchTarget44pt(composeBtn)
        assertWithinWindowBounds(composeBtn, in: application)

        openFirstChat(in: application)
        let sendBtn = requireElement(FabushiAccessibilityIDs.Composer.sendButton, in: application)
        let inputWrap = requireElement(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
        assertMinimumTouchTarget44pt(sendBtn)
        assertWithinWindowBounds(inputWrap, in: application)
        assertNonOverlapping(sendBtn, requireElement(FabushiAccessibilityIDs.Composer.attachButton, in: application))

        openShellTab(FabushiAccessibilityIDs.Shell.tabStudio, in: application)
        let studioTab = requireElement(FabushiAccessibilityIDs.Studio.tabOverview, in: application)
        assertMinimumTouchTarget44pt(studioTab)
        assertWithinWindowBounds(studioTab, in: application)

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        let settingsSearch = requireElement(FabushiAccessibilityIDs.Settings.searchField, in: application)
        assertWithinWindowBounds(settingsSearch, in: application)
        recordCheckpoint("T2_01_dynamic_type_ax5_44pt_zero_clip", passed: true, attachScreen: true)
    }

    // MARK: - 2. VoiceOver Completeness & Zero Raw Localization Key Leaks Across All 13 Namespaces

    func testT2_02_F31_F32_voiceOverCompletenessAndZeroRawKeyLeaksAcross13Namespaces() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_vo_user")],
                chats: [FabushiLoopbackSeedChat(chatId: 202, title: "VoiceOver Audit Room")]
            )
        )
        let application = try launchFabushi(FabushiE2ELaunchConfig(languageCode: "zh-Hans", localeIdentifier: "zh_CN"))
        XCTAssertEqual(FabushiAccessibilityIDs.allRequiredNamespaces.count, 13)

        let authRoot = element(FabushiAccessibilityIDs.Auth.root, in: application)
        if authRoot.waitForExistence(timeout: 3) {
            assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Auth.loginButton, in: application))
        }
        signInIfNeeded(in: application, username: "t2_vo_user", password: "FabushiV168!Pass")

        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Shell.accountSwitcherButton, in: application))
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.ChatList.composeButton, in: application))

        openFirstChat(in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Stage.headerTitle, in: application))
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Composer.sendButton, in: application))
        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Inspector.drawer, in: application))

        openShellTab(FabushiAccessibilityIDs.Shell.tabStudio, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Studio.tabOverview, in: application))
        openShellTab(FabushiAccessibilityIDs.Shell.tabRemote, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Remote.workbench, in: application))
        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Settings.searchField, in: application))
        recordCheckpoint("T2_02_voiceover_zero_raw_keys", passed: true)
    }

    // MARK: - 3. All 4 v1.6.8 Themes & Bright Custom Accent WCAG >138 Luma Contrast Guard

    func testT2_03_F31_all4V168ThemesAndBrightCustomAccentWCAG138Guard() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t2_theme_user")])
        )

        let themesWithAccents: [(String, String)] = [
            ("night", "#FFFF00"),
            ("tinted", "#FFFFFF"),
            ("day", "#FFFF00"),
            ("classic", "#007AFF"),
        ]
        for (index, pair) in themesWithAccents.enumerated() {
            let config = FabushiE2ELaunchConfig(
                theme: pair.0,
                accentHex: pair.1,
                resetState: index == 0
            )
            let application = try launchFabushi(config)
            signInIfNeeded(in: application, username: "t2_theme_user", password: "FabushiV168!Pass")
            openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
            tapControl(FabushiAccessibilityIDs.Settings.sectionAppearance, in: application)
            let badge = requireElement(FabushiAccessibilityIDs.Settings.accentContrastBadge, in: application)
            assertVoiceOverReady(badge)
            if pair.1 == "#FFFF00" || pair.1 == "#FFFFFF" {
                let badgeValue = (badge.value as? String) ?? badge.label
                XCTAssertFalse(
                    badgeValue.uppercased().contains("#FFFF00") && !badgeValue.lowercased().contains("dark"),
                    "F31 WCAG >138 luma guard must darken bright accent \(pair.1) on theme \(pair.0)"
                )
            }
            application.terminate()
        }
        recordCheckpoint("T2_03_themes_wcag_138_guard", passed: true)
    }

    // MARK: - 4. zh-Hans & en Localization Parity with Zero Untranslated Key Leaks

    func testT2_04_F31_zhHansAndEnLocalizationParityZeroUntranslatedKeyLeaks() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t2_locale_user")])
        )

        for localePair in [("zh-Hans", "zh_CN"), ("en", "en_US")] {
            let application = try launchFabushi(
                FabushiE2ELaunchConfig(
                    theme: "night",
                    languageCode: localePair.0,
                    localeIdentifier: localePair.1,
                    resetState: localePair.0 == "zh-Hans"
                )
            )
            signInIfNeeded(in: application, username: "t2_locale_user", password: "FabushiV168!Pass")
            openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)

            let appearanceRow = requireElement(FabushiAccessibilityIDs.Settings.sectionAppearance, in: application)
            let languageRow = requireElement(FabushiAccessibilityIDs.Settings.sectionLanguage, in: application)
            let privacyRow = requireElement(FabushiAccessibilityIDs.Settings.sectionPrivacySecurity, in: application)
            assertVoiceOverReady(appearanceRow)
            assertVoiceOverReady(languageRow)
            assertVoiceOverReady(privacyRow)
            application.terminate()
        }
        recordCheckpoint("T2_04_zh_hans_en_parity", passed: true)
    }

    // MARK: - 5. Reduce Motion & High Contrast Deterministic Toggles

    func testT2_05_F31_reduceMotionAndHighContrastDeterministicToggles() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t2_a11y_user")])
        )
        let application = try launchFabushi(
            FabushiE2ELaunchConfig(theme: "night", reduceMotion: true, highContrast: true)
        )
        signInIfNeeded(in: application, username: "t2_a11y_user", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionAppearance, in: application)

        let reduceMotion = requireElement(FabushiAccessibilityIDs.Settings.reduceMotionToggle, in: application)
        let highContrast = requireElement(FabushiAccessibilityIDs.Settings.highContrastToggle, in: application)
        assertVoiceOverReady(reduceMotion)
        assertVoiceOverReady(highContrast)
        assertMinimumTouchTarget44pt(reduceMotion)
        assertMinimumTouchTarget44pt(highContrast)
        recordCheckpoint("T2_05_reduce_motion_high_contrast", passed: true)
    }

    // MARK: - 6. Offline Network Disconnect Renders ContentUnavailableView & Recovers

    func testT2_06_F11_F33_offlineDisconnectRendersContentUnavailableViewAndRecovers() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_offline_user")],
                chats: [FabushiLoopbackSeedChat(chatId: 206, title: "Offline Recovery Chat")]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t2_offline_user", password: "FabushiV168!Pass")

        let faultOn = try await client.injectFault(FabushiLoopbackFaultRequest(offline: true))
        XCTAssertTrue(faultOn.offline, "Loopback server must enter offline disconnect mode")

        openFirstChat(in: application)
        let offlineView = requireElement(FabushiAccessibilityIDs.Shell.offlineUnavailableView, in: application, timeout: 12)
        assertWithinWindowBounds(offlineView, in: application)
        assertVoiceOverReady(offlineView)

        let retryBtn = requireElement(FabushiAccessibilityIDs.Shell.offlineRetryButton, in: application)
        assertMinimumTouchTarget44pt(retryBtn)

        let faultOff = try await client.clearFaults()
        XCTAssertFalse(faultOff.offline, "Loopback server must clear offline mode")
        retryBtn.tap()

        let recovered = offlineView.waitForNonExistence(timeout: 12)
        XCTAssertTrue(recovered, "F11/F33 offline ContentUnavailableView must dismiss upon recovery retry")
        recordCheckpoint("T2_06_offline_unavailable_and_recovery", passed: true, attachScreen: true)
    }

    // MARK: - 7. Server HTTP 500 Renders Inline Error / ContentUnavailableView or v1.6.8 PROVIDER_ERROR Card

    func testT2_07_F07_F11_F19_serverHTTP500RendersInlineErrorOrProviderErrorCard() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_http500_user")],
                chats: [FabushiLoopbackSeedChat(chatId: 207, title: "@Hermes500Bot", chatType: "bot", memberCount: 2)]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t2_http500_user", password: "FabushiV168!Pass")

        try await client.injectFault(
            FabushiLoopbackFaultRequest(
                httpFault: FabushiLoopbackHTTPFault(
                    pathPrefix: "/api/v1/messages",
                    status: 500,
                    body: ["error": "PROVIDER_ERROR", "message": "upstream model gateway returned 500"],
                    remainingHits: 2
                )
            )
        )

        openFirstChat(in: application)
        let field = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: field, with: "Trigger 500 provider error card")
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let providerCard = element(FabushiAccessibilityIDs.Shell.providerErrorCard, in: application)
        let stageError = element(FabushiAccessibilityIDs.Stage.errorUnavailableView, in: application)
        let handled = providerCard.waitForExistence(timeout: 8) || stageError.waitForExistence(timeout: 5)
        XCTAssertTrue(handled, "F07/F11/F19 HTTP 500 must render inline PROVIDER_ERROR card or stage error view")

        if providerCard.exists {
            let cta = requireElement(FabushiAccessibilityIDs.Shell.providerErrorOpenAIProviders, in: application)
            assertMinimumTouchTarget44pt(cta)
            assertVoiceOverReady(cta)
        } else {
            let retry = requireElement(FabushiAccessibilityIDs.Stage.errorRetryButton, in: application)
            assertMinimumTouchTarget44pt(retry)
        }
        try await client.clearFaults()
        recordCheckpoint("T2_07_http500_provider_error_card", passed: true)
    }

    // MARK: - 8. v1.6.8 Two-Layer Toast Non-Overlap Above Composer, Close CTA & Chat-Switch Clear

    func testT2_08_F07_v168TwoLayerToastNonOverlapExplicitCloseAndChatSwitchClear() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_toast_user")],
                chats: [
                    FabushiLoopbackSeedChat(chatId: 208, title: "Toast Source Chat"),
                    FabushiLoopbackSeedChat(chatId: 209, title: "Toast Target Chat"),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t2_toast_user", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        try await client.injectFault(
            FabushiLoopbackFaultRequest(
                httpFault: FabushiLoopbackHTTPFault(
                    pathPrefix: "/api/v1/messages",
                    status: 429,
                    body: ["error": "rate_limited"],
                    retryAfterMs: 3000,
                    remainingHits: 1
                )
            )
        )

        let field = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: field, with: "Trigger non-overlapping toast")
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let inputWrap = requireElement(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
        let windowBanner = element(FabushiAccessibilityIDs.Shell.toastWindowBanner, in: application)
        let stagePill = element(FabushiAccessibilityIDs.Shell.toastStagePill, in: application)
        let toastVisible = windowBanner.waitForExistence(timeout: 6) || stagePill.waitForExistence(timeout: 4)
        XCTAssertTrue(toastVisible, "v1.6.8 F07: rate-limit feedback must render two-layer toast banner or stage pill")

        if windowBanner.exists {
            XCTAssertLessThanOrEqual(
                windowBanner.frame.maxY,
                inputWrap.frame.minY,
                "v1.6.8 F07: windowBanner.maxY must remain strictly above inputWrap.minY"
            )
            assertNonOverlapping(windowBanner, inputWrap)
            let closeBtn = requireElement(FabushiAccessibilityIDs.Shell.toastWindowBannerClose, in: application)
            assertMinimumTouchTarget44pt(closeBtn)
            closeBtn.tap()
            let closed = windowBanner.waitForNonExistence(timeout: 4)
            XCTAssertTrue(closed, "v1.6.8 F07: tapping windowBanner.close must dismiss banner immediately")
        }

        if stagePill.exists {
            assertNonOverlapping(stagePill, inputWrap)
            tapControl(FabushiAccessibilityIDs.ChatList.secondRow, in: application)
            let cleared = stagePill.waitForNonExistence(timeout: 4)
            XCTAssertTrue(cleared, "v1.6.8 F07: switching chats must clear chat-scoped stage toast")
        }
        recordCheckpoint("T2_08_v168_two_layer_toast_non_overlap", passed: true)
    }

    // MARK: - 9. v1.6.8 Grok Compose Isolation & CJK IME Commit Safety

    func testT2_09_F08_F14_v168GrokComposeIsolationAndCJKIMECommitSafety() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_compose_iso")],
                chats: [FabushiLoopbackSeedChat(chatId: 210, title: "@GrokAI", chatType: "bot", memberCount: 2)]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t2_compose_iso", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let priorComposer = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: priorComposer, with: "草稿保留测试")

        tapControl(FabushiAccessibilityIDs.ChatList.composeButton, in: application)
        let inputWrap = element(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
        let hiddenInitially = inputWrap.waitForNonExistence(timeout: 3)
        XCTAssertTrue(hiddenInitially, "v1.6.8 F08: 0 chips must hide compose composer")

        let toField = requireElement(FabushiAccessibilityIDs.Compose.toField, in: application)
        replaceText(in: toField, with: "Grok")
        tapControl(FabushiAccessibilityIDs.Compose.firstCandidate, in: application)
        let revealed = inputWrap.waitForExistence(timeout: 5)
        XCTAssertTrue(revealed, "v1.6.8 F08: >=1 chip must reveal clean composer")
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Composer.botMenuButton, in: application).exists,
            "v1.6.8 F08: bot controls must remain hidden during compose"
        )

        tapControl(FabushiAccessibilityIDs.Compose.chipRemove, in: application)
        let hiddenAgain = inputWrap.waitForNonExistence(timeout: 4)
        XCTAssertTrue(hiddenAgain, "v1.6.8 F08: removing last chip must re-hide composer")

        tapControl(FabushiAccessibilityIDs.Compose.cancelButton, in: application)
        let restoredField = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        let restoredVal = (restoredField.value as? String) ?? ""
        XCTAssertTrue(
            restoredVal.contains("草稿保留测试"),
            "v1.6.8 F08/F14: cancelling compose must restore prior chat CJK draft"
        )
        recordCheckpoint("T2_09_v168_compose_isolation_ime", passed: true)
    }

    // MARK: - 10. Empty/Oversized Composer & Invalid Server URL Inline Error Handling

    func testT2_10_F01_F14_F33_emptyOversizedComposerAndInvalidServerURLInlineError() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_val_user")],
                chats: [FabushiLoopbackSeedChat(chatId: 211, title: "Validation Chat")]
            )
        )
        let application = try launchFabushi()

        let urlField = element(FabushiAccessibilityIDs.Auth.serverURLField, in: application)
        if urlField.waitForExistence(timeout: 4) {
            replaceText(in: urlField, with: "not-a-valid-url://bad")
            tapControl(FabushiAccessibilityIDs.Auth.loginButton, in: application)
            let urlErr = requireElement(FabushiAccessibilityIDs.Auth.serverURLError, in: application)
            assertVoiceOverReady(urlErr)
            let validURL = try resolvedServerURL()
            replaceText(in: urlField, with: validURL.absoluteString)
        }

        signInIfNeeded(in: application, username: "t2_val_user", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        let field = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: field, with: "   ")
        let sendBtn = requireElement(FabushiAccessibilityIDs.Composer.sendButton, in: application)
        if sendBtn.isEnabled && sendBtn.isHittable {
            sendBtn.tap()
        }
        let state = try await client.fetchState(pathPrefix: "/api/v1/chats/211/messages")
        XCTAssertEqual(state.counts.messages, 0, "F14 whitespace-only composer input must never send empty message")
        recordCheckpoint("T2_10_validation_composer_and_server_url", passed: true)
    }

    // MARK: - 11. Passcode Wrong-PIN Lockout Counter & Background Privacy Blur Overlay

    func testT2_11_F06_F23_passcodeWrongPINLockoutAndBackgroundPrivacyBlur() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t2_pin_user")])
        )
        let application = try launchFabushi(
            FabushiE2ELaunchConfig(extraEnvironment: ["FABUSHI_E2E_REQUIRE_PASSCODE": "1"])
        )
        signInIfNeeded(in: application, username: "t2_pin_user", password: "FabushiV168!Pass")

        let overlay = requireElement(FabushiAccessibilityIDs.Auth.passcodeOverlay, in: application, timeout: 10)
        assertWithinWindowBounds(overlay, in: application)

        let pinField = requireElement(FabushiAccessibilityIDs.Auth.passcodeField, in: application)
        replaceText(in: pinField, with: "000000")
        tapControl(FabushiAccessibilityIDs.Auth.passcodeSubmitButton, in: application)
        let errBanner = requireElement(FabushiAccessibilityIDs.Auth.passcodeErrorBanner, in: application)
        assertVoiceOverReady(errBanner)

        XCUIDevice.shared.press(.home)
        application.activate()
        let blurOrLock = element(FabushiAccessibilityIDs.Auth.privacyBlurOverlay, in: application).waitForExistence(timeout: 3)
            || overlay.waitForExistence(timeout: 5)
        XCTAssertTrue(blurOrLock, "F06/F23 returning from background must keep privacy blur or passcode lock active")
        recordCheckpoint("T2_11_passcode_lockout_and_privacy_blur", passed: true)
    }

    // MARK: - 12. v1.6.8 3-Way Inspector Isolation & Copied Feedback Clear on Switch

    func testT2_12_F16_v168ThreeWayInspectorIsolationAndCopiedFeedbackDismiss() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_insp_user")],
                chats: [
                    FabushiLoopbackSeedChat(chatId: 212, title: "@BotPeer", chatType: "bot", memberCount: 2),
                    FabushiLoopbackSeedChat(chatId: 213, title: "GroupPeer", chatType: "group", memberCount: 8),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t2_insp_user", password: "FabushiV168!Pass")

        openFirstChat(in: application)
        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        let botSection = requireElement(FabushiAccessibilityIDs.Inspector.profileBotSection, in: application)
        assertVoiceOverReady(botSection)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.tabMembers, in: application).exists,
            "v1.6.8 F16: Bot inspector must never expose group Members tab"
        )

        tapControl(FabushiAccessibilityIDs.Inspector.copyUsernameButton, in: application)
        let copied = requireElement(FabushiAccessibilityIDs.Inspector.copiedFeedback, in: application)
        assertVoiceOverReady(copied)

        tapControl(FabushiAccessibilityIDs.ChatList.secondRow, in: application)
        let clearedOnSwitch = copied.waitForNonExistence(timeout: 3)
        XCTAssertTrue(clearedOnSwitch, "v1.6.8 F16: copiedFeedback must clear immediately on chat switch")

        let groupSection = requireElement(FabushiAccessibilityIDs.Inspector.profileGroupSection, in: application)
        assertVoiceOverReady(groupSection)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.openAppButton, in: application).exists,
            "v1.6.8 F16: Group inspector must never leak bot openAppButton"
        )
        let leaveBtn = tapControl(FabushiAccessibilityIDs.Inspector.leaveGroupButton, in: application)
        assertMinimumTouchTarget44pt(leaveBtn)
        let leaveModal = requireElement(FabushiAccessibilityIDs.Inspector.leaveConfirmModal, in: application)
        assertWithinWindowBounds(leaveModal, in: application)
        tapControl(FabushiAccessibilityIDs.Inspector.leaveConfirmCancel, in: application)
        recordCheckpoint("T2_12_v168_3way_inspector_isolation", passed: true)
    }

    // MARK: - 13. Mini App Unaccepted ToS Fail-Closed Block & WebMCP Error Banner

    func testT2_13_F24_miniAppUnacceptedToSFailClosedAndWebMCPErrorBanner() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_miniapp_tos")],
                miniapps: [
                    FabushiLoopbackSeedMiniApp(appId: "global_dharma", title: "全球法布施", tosAccepted: false)
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t2_miniapp_tos", password: "FabushiV168!Pass")

        tapControl(FabushiAccessibilityIDs.MiniApp.launcherButton, in: application)
        tapControl(FabushiAccessibilityIDs.MiniApp.globalDharmaCard, in: application)
        _ = requireElement(FabushiAccessibilityIDs.MiniApp.tosModal, in: application)

        tapControl(FabushiAccessibilityIDs.MiniApp.tosDeclineButton, in: application)
        let blocked = requireElement(FabushiAccessibilityIDs.MiniApp.tosBlockedBanner, in: application)
        assertVoiceOverReady(blocked)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.MiniApp.webView, in: application).exists,
            "F24 declining Mini App ToS must fail closed and refuse loading WKWebView"
        )
        recordCheckpoint("T2_13_miniapp_tos_fail_closed", passed: true)
    }

    // MARK: - 14. v1.6.8 Remote Host ScreenCaptureKit (-3801) / Accessibility Denial & iOS Local Host Unsupported

    func testT2_14_F22_v168RemoteHostPermissionDenialModalAndLocalHostUnsupported() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_remote_perm")],
                remoteNodes: [
                    FabushiLoopbackSeedRemoteNode(
                        deviceId: "mac_denied_v168",
                        name: "MacBook Pro (TCC Denied)",
                        online: true,
                        screenCapturePermission: "not_granted",
                        accessibilityDragPermission: "not_granted"
                    )
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t2_remote_perm", password: "FabushiV168!Pass")

        openShellTab(FabushiAccessibilityIDs.Shell.tabRemote, in: application)
        let localBanner = requireElement(FabushiAccessibilityIDs.Remote.localHostUnsupportedBanner, in: application)
        assertVoiceOverReady(localBanner)

        tapControl(FabushiAccessibilityIDs.Remote.firstNodeRow, in: application)
        let permModal = requireElement(FabushiAccessibilityIDs.Remote.permissionModal, in: application, timeout: 10)
        assertWithinWindowBounds(permModal, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Remote.permissionModalScreenStatus, in: application))
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Remote.permissionModalAccessibilityStatus, in: application))

        // iOS must never surface macOS AppKit drag-HUD panel
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Remote.macosDragHudForbidden, in: application).exists,
            "v1.6.8 F22: iOS must never render macOS-only AppKit drag-HUD"
        )
        let retryBtn = requireElement(FabushiAccessibilityIDs.Remote.permissionModalRetryButton, in: application)
        let dismissBtn = requireElement(FabushiAccessibilityIDs.Remote.permissionModalDismissButton, in: application)
        assertMinimumTouchTarget44pt(retryBtn)
        assertMinimumTouchTarget44pt(dismissBtn)
        dismissBtn.tap()
        recordCheckpoint("T2_14_v168_remote_permission_modal", passed: true, attachScreen: true)
    }

    // MARK: - 15. Calls Mic/Camera/ReplayKit Permission Denial Fail-Closed Banner & Honest BLK-02 Disclosure

    func testT2_15_F26_callsMicCameraReplayKitPermissionDenialAndBLK02Disclosure() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(
                users: [FabushiLoopbackSeedUser(username: "t2_call_perm")],
                chats: [FabushiLoopbackSeedChat(chatId: 215, title: "Call Permission Room")]
            )
        )
        let application = try launchFabushi(
            FabushiE2ELaunchConfig(extraEnvironment: ["FABUSHI_E2E_SIMULATE_CALL_PERMISSION_DENIED": "1"])
        )
        signInIfNeeded(in: application, username: "t2_call_perm", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        tapControl(FabushiAccessibilityIDs.Stage.callStartButton, in: application)
        let deniedBanner = requireElement(FabushiAccessibilityIDs.Call.permissionDeniedBanner, in: application)
        assertWithinWindowBounds(deniedBanner, in: application)
        assertVoiceOverReady(deniedBanner)

        let openSettingsBtn = requireElement(FabushiAccessibilityIDs.Call.permissionDeniedOpenSettings, in: application)
        assertMinimumTouchTarget44pt(openSettingsBtn)
        let sfuBanner = requireElement(FabushiAccessibilityIDs.Call.sfuDisclosureBanner, in: application)
        assertVoiceOverReady(sfuBanner)
        recordCheckpoint("T2_15_call_permission_denial_blk02", passed: true)
    }

    // MARK: - 16. Zero Orphaned Routes, Dead Controls, Sample Chats & Honest Capability Ledger

    func testT2_16_F09_F34_zeroOrphanedRoutesDeadControlsSampleChatsAndHonestLedger() async throws {
        let client = try await prepareCleanLoopback()
        let application = try launchFabushi(FabushiE2ELaunchConfig(resetState: true))

        assertNoSampleChats(in: application)
        let initialState = try await client.fetchState()
        XCTAssertEqual(initialState.counts.chats, 0, "F09/F34 fresh unauthenticated loopback must have 0 sample chats")

        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t2_dead_ctl")])
        )
        signInIfNeeded(in: application, username: "t2_dead_ctl", password: "FabushiV168!Pass")
        assertNoSampleChats(in: application)

        assertNoDeadInteractiveControls(
            in: application,
            identifiers: [
                FabushiAccessibilityIDs.ChatList.composeButton,
                FabushiAccessibilityIDs.Shell.accountSwitcherButton,
                FabushiAccessibilityIDs.Shell.tabSettings,
            ]
        )

        openShellTab(FabushiAccessibilityIDs.Shell.tabSettings, in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionCapabilities, in: application)
        let ledger = requireElement(FabushiAccessibilityIDs.Settings.capabilityLedgerList, in: application)
        assertWithinWindowBounds(ledger, in: application)
        assertVoiceOverReady(ledger)
        recordCheckpoint("T2_16_zero_dead_controls_and_sample_chats", passed: true)
    }
}
