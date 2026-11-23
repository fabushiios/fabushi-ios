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
                chats: [FabushiLoopbackSeedChat(chatId: 202, title: "VoiceOver Audit Room")],
                remoteNodes: [
                    FabushiLoopbackSeedRemoteNode(deviceId: "mac_vo_v168", name: "VoiceOver Mac Studio")
                ],
                miniapps: [
                    FabushiLoopbackSeedMiniApp(appId: "global_dharma", title: "全球法布施", tosAccepted: true)
                ]
            )
        )
        let application = try launchFabushi(FabushiE2ELaunchConfig(languageCode: "zh-Hans", localeIdentifier: "zh_CN"))

        // 1. fabushi.auth (fail-closed)
        let authRoot = requireElement(FabushiAccessibilityIDs.Auth.root, in: application)
        assertWithinWindowBounds(authRoot, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Auth.loginButton, in: application))

        signInIfNeeded(in: application, username: "t2_vo_user", password: "FabushiV168!Pass")

        // 2. fabushi.shell & 3. fabushi.chatlist
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Shell.accountSwitcherButton, in: application))
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.ChatList.composeButton, in: application))

        // 4. fabushi.compose
        tapControl(FabushiAccessibilityIDs.ChatList.composeButton, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Compose.toField, in: application))
        tapControl(FabushiAccessibilityIDs.Compose.cancelButton, in: application)

        // 5. fabushi.stage & 6. fabushi.composer
        openFirstChat(in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Stage.headerTitle, in: application))
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Composer.sendButton, in: application))

        // 7. fabushi.call
        tapControl(FabushiAccessibilityIDs.Stage.callStartButton, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Call.muteButton, in: application))
        tapControl(FabushiAccessibilityIDs.Call.hangupButton, in: application)
        XCTAssertTrue(
            element(FabushiAccessibilityIDs.Call.overlay, in: application).waitForNonExistence(timeout: 5),
            "T2_02 Call overlay must dismiss cleanly after hangup"
        )

        // 8. fabushi.inspector (dismiss sheet before tab switching on iPhone)
        tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Inspector.drawer, in: application))
        dismissInspectorIfPresented(in: application)
        navigateBackToChatListIfCompact(in: application)

        // 9. fabushi.miniapp
        tapControl(FabushiAccessibilityIDs.MiniApp.launcherButton, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.MiniApp.globalDharmaCard, in: application))
        tapControl(FabushiAccessibilityIDs.MiniApp.closeButton, in: application)

        // 10. fabushi.studio
        openShellTab(FabushiAccessibilityIDs.Shell.tabStudio, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Studio.tabOverview, in: application))

        // 11. fabushi.remote
        openShellTab(FabushiAccessibilityIDs.Shell.tabRemote, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Remote.workbench, in: application))

        // 12. fabushi.settings & 13. fabushi.magpie
        openSettingsRoot(in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Settings.searchField, in: application))
        tapControl(FabushiAccessibilityIDs.Settings.sectionAIProviders, in: application)
        assertVoiceOverReady(requireElement(FabushiAccessibilityIDs.Magpie.probeHealthButton, in: application))
        recordCheckpoint("T2_02_voiceover_zero_raw_keys", passed: true)
    }

    // MARK: - 3. All 4 v1.6.8 Themes & Bright Custom Accent WCAG >138 Luma Contrast Guard

    func testT2_03_F31_all4V168ThemesAndBrightCustomAccentWCAG138Guard() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t2_theme_user")])
        )

        let themesWithAccents: [(theme: String, rawAccent: String, expectedHex: String, adjusted: Bool)] = [
            ("night", "#FFFF00", "#A9A906", true),
            ("tinted", "#FFFFFF", "#A9A9A9", true),
            ("day", "#FFFF00", "#A9A906", true),
            ("classic", "#007AFF", "#007AFF", false),
        ]
        for (index, item) in themesWithAccents.enumerated() {
            let config = FabushiE2ELaunchConfig(
                theme: item.theme,
                accentHex: item.rawAccent,
                resetState: index == 0
            )
            let application = try launchFabushi(config)
            signInIfNeeded(in: application, username: "t2_theme_user", password: "FabushiV168!Pass")
            openSettingsRoot(in: application)
            tapControl(FabushiAccessibilityIDs.Settings.sectionAppearance, in: application)
            let badge = requireElement(FabushiAccessibilityIDs.Settings.accentContrastBadge, in: application)
            assertVoiceOverReady(badge)

            let rawAccent = item.rawAccent.uppercased()
            guard let expected = Self.computeWCAGAccentFill(rawAccent) else {
                XCTFail("Invalid test accent hex \(rawAccent)")
                return
            }
            XCTAssertEqual(
                expected.effectiveHex,
                item.expectedHex,
                "WCAG >138 reference formula mismatch for \(rawAccent)"
            )
            XCTAssertEqual(expected.adjusted, item.adjusted)

            let badgeValue = ((badge.value as? String) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let badgeLabel = badge.label.trimmingCharacters(in: .whitespacesAndNewlines)
            let valueHexes = Self.extractHexColors(from: badgeValue.uppercased())
            let labelHexes = Self.extractHexColors(from: badgeLabel.uppercased())
            let candidateHexes = !valueHexes.isEmpty ? valueHexes : labelHexes

            XCTAssertFalse(
                candidateHexes.isEmpty,
                "F31 Settings.accentContrastBadge must expose rendered #RRGGBB hex for \(rawAccent) on theme \(item.theme) (value='\(badgeValue)', label='\(badgeLabel)')"
            )

            if expected.adjusted {
                XCTAssertTrue(
                    candidateHexes.contains(expected.effectiveHex),
                    "F31 WCAG >138 luma guard expected darkened \(expected.effectiveHex) for \(rawAccent) on theme \(item.theme), got \(candidateHexes) (value='\(badgeValue)', label='\(badgeLabel)')"
                )
                let allowedHexes: Set<String> = [expected.effectiveHex, rawAccent]
                XCTAssertTrue(
                    Set(candidateHexes).isSubset(of: allowedHexes),
                    "F31 WCAG >138 luma guard rendered unexpected hexes \(candidateHexes) for \(rawAccent) on theme \(item.theme)"
                )
                if candidateHexes.count == 1 {
                    XCTAssertNotEqual(
                        candidateHexes[0],
                        rawAccent,
                        "F31 WCAG >138 luma guard must not leave bright accent \(rawAccent) unmodified on theme \(item.theme)"
                    )
                }
            } else {
                XCTAssertEqual(
                    Set(candidateHexes),
                    [rawAccent],
                    "F31 WCAG >138 guard must preserve compliant accent \(rawAccent) without false darkening on theme \(item.theme), got \(candidateHexes)"
                )
            }
            if index < themesWithAccents.count - 1 {
                application.terminate()
            }
        }
        recordCheckpoint("T2_03_themes_wcag_138_guard", passed: true)
    }

    // MARK: - 4. zh-Hans & en Localization Parity with Zero Untranslated Key Leaks

    func testT2_04_F31_zhHansAndEnLocalizationParityZeroUntranslatedKeyLeaks() async throws {
        let client = try await prepareCleanLoopback()
        try await client.seed(
            FabushiLoopbackSeedRequest(users: [FabushiLoopbackSeedUser(username: "t2_locale_user")])
        )

        var labelsByLocale: [String: [String]] = [:]
        let localePairs = [("zh-Hans", "zh_CN"), ("en", "en_US")]
        for (index, localePair) in localePairs.enumerated() {
            let application = try launchFabushi(
                FabushiE2ELaunchConfig(
                    theme: "night",
                    languageCode: localePair.0,
                    localeIdentifier: localePair.1,
                    resetState: index == 0
                )
            )
            signInIfNeeded(in: application, username: "t2_locale_user", password: "FabushiV168!Pass")
            openSettingsRoot(in: application)

            let appearanceRow = requireElement(FabushiAccessibilityIDs.Settings.sectionAppearance, in: application)
            let languageRow = requireElement(FabushiAccessibilityIDs.Settings.sectionLanguage, in: application)
            let privacyRow = requireElement(FabushiAccessibilityIDs.Settings.sectionPrivacySecurity, in: application)
            assertVoiceOverReady(appearanceRow)
            assertVoiceOverReady(languageRow)
            assertVoiceOverReady(privacyRow)
            labelsByLocale[localePair.0] = [appearanceRow.label, languageRow.label, privacyRow.label]
            if index < localePairs.count - 1 {
                application.terminate()
            }
        }
        guard let zhLabels = labelsByLocale["zh-Hans"], let enLabels = labelsByLocale["en"] else {
            XCTFail("T2_04 failed to capture both zh-Hans and en Settings labels")
            return
        }
        for i in 0..<zhLabels.count {
            XCTAssertNotEqual(
                zhLabels[i],
                enLabels[i],
                "F31 localization parity violated: Settings row \(i) rendered identical label '\(zhLabels[i])' for zh-Hans and en"
            )
            XCTAssertNotNil(
                zhLabels[i].range(of: "\\p{Han}", options: .regularExpression),
                "F31 zh-Hans localization must render Han/CJK text for Settings row \(i), got '\(zhLabels[i])'"
            )
            XCTAssertNotNil(
                enLabels[i].range(of: "[A-Za-z]", options: .regularExpression),
                "F31 en localization must render English text for Settings row \(i), got '\(enLabels[i])'"
            )
            XCTAssertNil(
                enLabels[i].range(of: "\\p{Han}", options: .regularExpression),
                "F31 en localization must not leak untranslated Han/CJK text in row \(i), got '\(enLabels[i])'"
            )
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

        openSettingsRoot(in: application)
        tapControl(FabushiAccessibilityIDs.Settings.sectionAppearance, in: application)

        let reduceMotion = requireElement(FabushiAccessibilityIDs.Settings.reduceMotionToggle, in: application)
        let highContrast = requireElement(FabushiAccessibilityIDs.Settings.highContrastToggle, in: application)
        let reduceMotionSwitch = assertToggleTouchTarget(reduceMotion, in: application)
        let highContrastSwitch = assertToggleTouchTarget(highContrast, in: application)

        XCTAssertTrue(
            isSwitchEnabled(reduceMotionSwitch, in: application),
            "F31 reduceMotionToggle must initialize ON when FABUSHI_UI_REDUCE_MOTION=1"
        )
        XCTAssertTrue(
            isSwitchEnabled(highContrastSwitch, in: application),
            "F31 highContrastToggle must initialize ON when FABUSHI_UI_HIGH_CONTRAST=1"
        )

        tapSwitch(reduceMotionSwitch, in: application)
        let rmDisabled = await waitUntil(timeout: 4) { !self.isSwitchEnabled(reduceMotionSwitch, in: application) }
        XCTAssertTrue(rmDisabled, "F31 tapping reduceMotionToggle must flip state to OFF")
        tapSwitch(reduceMotionSwitch, in: application)
        let rmReenabled = await waitUntil(timeout: 4) { self.isSwitchEnabled(reduceMotionSwitch, in: application) }
        XCTAssertTrue(rmReenabled, "F31 tapping reduceMotionToggle again must flip state back to ON")

        tapSwitch(highContrastSwitch, in: application)
        let hcDisabled = await waitUntil(timeout: 4) { !self.isSwitchEnabled(highContrastSwitch, in: application) }
        XCTAssertTrue(hcDisabled, "F31 tapping highContrastToggle must flip state to OFF")
        tapSwitch(highContrastSwitch, in: application)
        let hcReenabled = await waitUntil(timeout: 4) { self.isSwitchEnabled(highContrastSwitch, in: application) }
        XCTAssertTrue(hcReenabled, "F31 tapping highContrastToggle again must flip state back to ON")
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

        let faultBody = ["error": "PROVIDER_ERROR", "message": "upstream model gateway returned 500"]
        try await client.injectFault(
            FabushiLoopbackFaultRequest(
                httpFaults: [
                    FabushiLoopbackHTTPFault(
                        pathPrefix: "/api/v1/chats/207/messages",
                        method: "POST",
                        status: 500,
                        body: faultBody,
                        remainingHits: 2
                    ),
                    FabushiLoopbackHTTPFault(
                        pathPrefix: "/api/v1/messages",
                        method: "POST",
                        status: 500,
                        body: faultBody,
                        remainingHits: 2
                    ),
                    FabushiLoopbackHTTPFault(
                        pathPrefix: "/api/v1/agent",
                        method: "POST",
                        status: 500,
                        body: faultBody,
                        remainingHits: 2
                    ),
                ]
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
                httpFaults: [
                    FabushiLoopbackHTTPFault(
                        pathPrefix: "/api/v1/chats/208/messages",
                        method: "POST",
                        status: 429,
                        body: ["error": "rate_limited"],
                        retryAfterMs: 3000,
                        remainingHits: 2
                    ),
                    FabushiLoopbackHTTPFault(
                        pathPrefix: "/api/v1/messages",
                        method: "POST",
                        status: 429,
                        body: ["error": "rate_limited"],
                        retryAfterMs: 3000,
                        remainingHits: 2
                    ),
                ]
            )
        )

        let field = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: field, with: "Trigger non-overlapping toast")
        tapControl(FabushiAccessibilityIDs.Composer.sendButton, in: application)

        let inputWrap = requireElement(FabushiAccessibilityIDs.Composer.inputWrap, in: application)
        let windowBanner = element(FabushiAccessibilityIDs.Shell.toastWindowBanner, in: application)
        let stagePill = element(FabushiAccessibilityIDs.Shell.toastStagePill, in: application)
        let toastVisible = await waitUntil(timeout: 8) {
            windowBanner.exists || stagePill.exists
        }
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
        } else if stagePill.exists {
            XCTAssertLessThanOrEqual(
                stagePill.frame.maxY,
                inputWrap.frame.minY,
                "v1.6.8 F07: stagePill.maxY must remain strictly above inputWrap.minY"
            )
            assertNonOverlapping(stagePill, inputWrap)
            openChatRow(FabushiAccessibilityIDs.ChatList.secondRow, in: application)
            let cleared = stagePill.waitForNonExistence(timeout: 4)
            XCTAssertTrue(cleared, "v1.6.8 F07: switching chats must clear chat-scoped stage toast")
        } else {
            XCTFail("v1.6.8 F07: toast dismissed before non-overlap and dismissal verification")
        }
        try await client.clearFaults()
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

        navigateBackToChatListIfCompact(in: application)
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
        reopenFirstChatIfComposerNotVisible(in: application)
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

        // 1. Fail-closed invalid server URL inline error (no silent-skip `if`)
        let urlField = requireElement(FabushiAccessibilityIDs.Auth.serverURLField, in: application)
        replaceText(in: urlField, with: "not-a-valid-url://bad")
        tapControl(FabushiAccessibilityIDs.Auth.loginButton, in: application)
        let urlErr = requireElement(FabushiAccessibilityIDs.Auth.serverURLError, in: application)
        assertWithinWindowBounds(urlErr, in: application)
        assertVoiceOverReady(urlErr)
        let validURL = try resolvedServerURL()
        replaceText(in: urlField, with: validURL.absoluteString)

        signInIfNeeded(in: application, username: "t2_val_user", password: "FabushiV168!Pass")
        openFirstChat(in: application)

        // 2. Whitespace-only composer must never send
        let field = requireElement(FabushiAccessibilityIDs.Composer.textField, in: application)
        replaceText(in: field, with: "   ")
        let sendBtn = requireElement(FabushiAccessibilityIDs.Composer.sendButton, in: application)
        if sendBtn.isEnabled && sendBtn.isHittable {
            sendBtn.tap()
        }
        let afterEmptyState = try await client.fetchState(pathPrefix: "/api/v1/chats/211/messages")
        XCTAssertEqual(afterEmptyState.counts.messages, 0, "F14 whitespace-only composer input must never send empty message")

        // 3. Oversized (> 4096 chars) composer payload must surface Composer.validationErrorBanner & send 0 messages
        let oversizedPayload = String(repeating: "x", count: 4096) + "OVERFLOW9"
        await pasteText(in: field, with: oversizedPayload, in: application)
        if sendBtn.isEnabled && sendBtn.isHittable {
            sendBtn.tap()
        }
        let validationBanner = requireElement(FabushiAccessibilityIDs.Composer.validationErrorBanner, in: application)
        assertWithinWindowBounds(validationBanner, in: application)
        assertVoiceOverReady(validationBanner)

        let afterOversizeState = try await client.fetchState(pathPrefix: "/api/v1/chats/211/messages")
        XCTAssertEqual(
            afterOversizeState.counts.messages,
            0,
            "F14 oversized (>4096 chars) composer payload must fail closed and persist 0 messages"
        )
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
                    FabushiLoopbackSeedChat(chatId: 214, title: "DirectUserPeer", chatType: "private", memberCount: 2),
                ]
            )
        )
        let application = try launchFabushi()
        signInIfNeeded(in: application, username: "t2_insp_user", password: "FabushiV168!Pass")

        // Way 1: Bot Inspector
        openFirstChat(in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Stage.inspectorToggleButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let botSection = requireElement(FabushiAccessibilityIDs.Inspector.profileBotSection, in: application)
        assertVoiceOverReady(botSection)
        let openAppBtn = requireElement(FabushiAccessibilityIDs.Inspector.openAppButton, in: application)
        assertMinimumTouchTarget44pt(openAppBtn)
        assertVoiceOverReady(openAppBtn)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.tabMembers, in: application).exists,
            "v1.6.8 F16: Bot inspector must never expose group Members tab"
        )
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.leaveGroupButton, in: application).exists,
            "v1.6.8 F16: Bot inspector must never expose leaveGroupButton"
        )
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.blockContactButton, in: application).exists,
            "v1.6.8 F16: Bot inspector must never expose 1v1 blockContactButton"
        )

        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Inspector.copyUsernameButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let copied = requireElement(FabushiAccessibilityIDs.Inspector.copiedFeedback, in: application)
        assertVoiceOverReady(copied)

        // Switch to Way 2: Group Inspector (adaptive iPad side-by-side vs iPhone sheet)
        if activeFormFactor() == .ipad {
            tapControl(FabushiAccessibilityIDs.ChatList.secondRow, in: application)
            let clearedOnSwitch = copied.waitForNonExistence(timeout: 3)
            XCTAssertTrue(clearedOnSwitch, "v1.6.8 F16: copiedFeedback must clear immediately on chat switch")
        } else {
            dismissInspectorIfPresented(in: application)
            openChatRow(FabushiAccessibilityIDs.ChatList.secondRow, in: application)
            tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
            XCTAssertFalse(
                element(FabushiAccessibilityIDs.Inspector.copiedFeedback, in: application).exists,
                "v1.6.8 F16: copiedFeedback must not persist across chat switch"
            )
        }

        let groupSection = requireElement(FabushiAccessibilityIDs.Inspector.profileGroupSection, in: application)
        assertVoiceOverReady(groupSection)
        let membersTab = requireElement(FabushiAccessibilityIDs.Inspector.tabMembers, in: application)
        assertMinimumTouchTarget44pt(membersTab)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.openAppButton, in: application).exists,
            "v1.6.8 F16: Group inspector must never leak bot openAppButton"
        )
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.startSecretChatButton, in: application).exists,
            "v1.6.8 F16: Group inspector must never leak 1v1 startSecretChatButton"
        )
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.blockContactButton, in: application).exists,
            "v1.6.8 F16: Group inspector must never leak 1v1 blockContactButton"
        )
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Inspector.leaveGroupButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let leaveModal = requireElement(FabushiAccessibilityIDs.Inspector.leaveConfirmModal, in: application)
        assertWithinWindowBounds(leaveModal, in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Inspector.leaveConfirmCancel,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        XCTAssertTrue(
            leaveModal.waitForNonExistence(timeout: 5),
            "v1.6.8 F16: cancelling leave group modal must dismiss confirmation dialog"
        )

        // Way 3: 1v1 User Inspector
        openChatRow(FabushiAccessibilityIDs.ChatList.thirdRow, in: application)
        if !element(FabushiAccessibilityIDs.Inspector.drawer, in: application).exists {
            tapControl(FabushiAccessibilityIDs.Stage.inspectorToggleButton, in: application)
        }
        let userSection = requireElement(FabushiAccessibilityIDs.Inspector.profileUserSection, in: application)
        assertVoiceOverReady(userSection)
        let secretBtn = requireElement(FabushiAccessibilityIDs.Inspector.startSecretChatButton, in: application)
        assertMinimumTouchTarget44pt(secretBtn)
        assertVoiceOverReady(secretBtn)
        let blockBtn = requireElement(FabushiAccessibilityIDs.Inspector.blockContactButton, in: application)
        assertMinimumTouchTarget44pt(blockBtn)
        assertVoiceOverReady(blockBtn)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.tabMembers, in: application).exists,
            "v1.6.8 F16: 1v1 User inspector must never expose group Members tab"
        )
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.openAppButton, in: application).exists,
            "v1.6.8 F16: 1v1 User inspector must never expose bot openAppButton"
        )
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Inspector.leaveGroupButton, in: application).exists,
            "v1.6.8 F16: 1v1 User inspector must never expose leaveGroupButton"
        )
        dismissInspectorIfPresented(in: application)
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

        // Inject WebMCP fault, accept ToS, and verify MiniApp.webMcpErrorBanner surfaces cleanly
        let webMcpFaultBody = ["error": "webmcp_bridge_unavailable", "message": "injected WebMCP error"]
        try await client.injectFault(
            FabushiLoopbackFaultRequest(
                httpFaults: [
                    FabushiLoopbackHTTPFault(pathPrefix: "/api/v1/mcp", status: 500, body: webMcpFaultBody, remainingHits: 4),
                    FabushiLoopbackHTTPFault(pathPrefix: "/api/v1/webmcp", status: 500, body: webMcpFaultBody, remainingHits: 4),
                    FabushiLoopbackHTTPFault(pathPrefix: "/api/v1/miniapps/global_dharma/webmcp", status: 500, body: webMcpFaultBody, remainingHits: 4),
                ]
            )
        )
        if !element(FabushiAccessibilityIDs.MiniApp.globalDharmaCard, in: application).exists {
            tapControl(FabushiAccessibilityIDs.MiniApp.launcherButton, in: application)
        }
        tapControl(FabushiAccessibilityIDs.MiniApp.globalDharmaCard, in: application)
        _ = requireElement(FabushiAccessibilityIDs.MiniApp.tosModal, in: application)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.MiniApp.tosAcceptButton,
            in: application,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let webMcpError = requireElement(FabushiAccessibilityIDs.MiniApp.webMcpErrorBanner, in: application, timeout: 12)
        assertWithinWindowBounds(webMcpError, in: application)
        assertVoiceOverReady(webMcpError)
        try await client.clearFaults()
        recordCheckpoint("T2_13_miniapp_tos_fail_closed_and_webmcp_error", passed: true)
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
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Call.overlay, in: application).exists,
            "F26 mic/camera permission denial must fail closed and refuse presenting fake Call.overlay"
        )

        let openSettingsBtn = requireElement(FabushiAccessibilityIDs.Call.permissionDeniedOpenSettings, in: application)
        assertMinimumTouchTarget44pt(openSettingsBtn)
        let sfuBanner = requireElement(FabushiAccessibilityIDs.Call.sfuDisclosureBanner, in: application)
        assertVoiceOverReady(sfuBanner)

        application.terminate()
        let replayKitApp = try launchFabushi(
            FabushiE2ELaunchConfig(
                resetState: false,
                extraEnvironment: ["FABUSHI_E2E_SIMULATE_REPLAYKIT_DENIED": "1"]
            )
        )
        signInIfNeeded(in: replayKitApp, username: "t2_call_perm", password: "FabushiV168!Pass")
        openFirstChat(in: replayKitApp)
        tapControl(FabushiAccessibilityIDs.Stage.callStartButton, in: replayKitApp)
        _ = requireElement(FabushiAccessibilityIDs.Call.overlay, in: replayKitApp)
        tapControlWithHIGChecks(
            FabushiAccessibilityIDs.Call.screenShareButton,
            in: replayKitApp,
            verifyMinTouchTarget: true,
            verifyWithinBounds: true
        )
        let shareDenied = requireElement(FabushiAccessibilityIDs.Call.screenShareUnavailableBanner, in: replayKitApp, timeout: 8)
        assertWithinWindowBounds(shareDenied, in: replayKitApp)
        assertVoiceOverReady(shareDenied)
        XCTAssertFalse(
            element(FabushiAccessibilityIDs.Call.screenSharePicker, in: replayKitApp).exists,
            "F26 ReplayKit denial must not present broadcast picker"
        )
        tapControl(FabushiAccessibilityIDs.Call.hangupButton, in: replayKitApp)
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
            probes: [
                FabushiLiveControlProbe(
                    controlIdentifier: FabushiAccessibilityIDs.ChatList.composeButton,
                    expectedElementIdentifier: FabushiAccessibilityIDs.Compose.stage,
                    dismissIdentifier: FabushiAccessibilityIDs.Compose.cancelButton
                ),
                FabushiLiveControlProbe(
                    controlIdentifier: FabushiAccessibilityIDs.Shell.accountSwitcherButton,
                    expectedElementIdentifier: FabushiAccessibilityIDs.Shell.accountSwitcherSheet,
                    dismissIdentifier: FabushiAccessibilityIDs.Shell.accountSwitcherCloseButton
                ),
                FabushiLiveControlProbe(
                    controlIdentifier: FabushiAccessibilityIDs.Shell.tabSettings,
                    expectedElementIdentifier: FabushiAccessibilityIDs.Settings.root
                ),
            ]
        )

        tapControl(FabushiAccessibilityIDs.Settings.sectionCapabilities, in: application)
        assertCapabilityLedgerShowsCanonicalIDs(in: application)
        recordCheckpoint("T2_16_zero_dead_controls_and_sample_chats", passed: true)
    }
}
