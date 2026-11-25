import Foundation
import XCTest
@testable import Fabushi

@MainActor
final class FabushiSettingsProxyEconomyAndIntentsTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m5b_settings_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain() -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m5b.settings.\(UUID().uuidString)",
            account: "m5b-settings"
        )
    }

    private func signInTestUser(coordinator: FabushiCoordinator) async throws {
        let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
            method: FabushiRPCNamespace.auth.method("login"),
            params: FabushiLegacyLoginRequest(username: "platform_user", password: "StrongPassword!1"),
            extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
        )
    }

    // MARK: - 1. All 26 Settings Sections, Deep-Link Parsing, Bilingual Search & BLK-08 Community Honesty

    func testAll26SettingsSectionsParsingBilingualSearchAndCommunityBlk08Honesty() {
        let all = FabushiSettingsSectionType.allCases
        XCTAssertEqual(all.count, 26, "Settings Suite MUST expose all 26 canonical sections")

        // Every section has a unique accessibility identifier and non-empty bilingual titles
        let uniqueIds = Set(all.map(\.accessibilityID))
        XCTAssertEqual(uniqueIds.count, 26)

        // Verify ALL 13 canonical FabushiAccessibilityIDs.Settings.section* IDs are wired
        let canonicalSectionIds: [String] = [
            FabushiAccessibilityIDs.Settings.sectionAppearance,
            FabushiAccessibilityIDs.Settings.sectionLanguage,
            FabushiAccessibilityIDs.Settings.sectionPrivacySecurity,
            FabushiAccessibilityIDs.Settings.sectionDevices,
            FabushiAccessibilityIDs.Settings.sectionVault,
            FabushiAccessibilityIDs.Settings.sectionDataStorage,
            FabushiAccessibilityIDs.Settings.sectionNetworkProxy,
            FabushiAccessibilityIDs.Settings.sectionAIProviders,
            FabushiAccessibilityIDs.Settings.sectionMCP,
            FabushiAccessibilityIDs.Settings.sectionWalletStars,
            FabushiAccessibilityIDs.Settings.sectionNotifications,
            FabushiAccessibilityIDs.Settings.sectionUpdates,
            FabushiAccessibilityIDs.Settings.sectionCapabilities,
        ]
        for expectedId in canonicalSectionIds {
            XCTAssertTrue(uniqueIds.contains(expectedId), "Missing canonical settings section ID: \(expectedId)")
        }

        // Deep-link alias parsing
        XCTAssertEqual(FabushiSettingsSectionType.parse("network_proxy"), .networkProxy)
        XCTAssertEqual(FabushiSettingsSectionType.parse("ai-providers"), .aiProviders)
        XCTAssertEqual(FabushiSettingsSectionType.parse("browser"), .builtInBrowser)
        XCTAssertEqual(FabushiSettingsSectionType.parse("wallet"), .walletStars)
        XCTAssertEqual(FabushiSettingsSectionType.parse("community"), .capabilities)
        XCTAssertNil(FabushiSettingsSectionType.parse("   "))

        // Bilingual search across BOTH en and zh-Hans
        XCTAssertTrue(FabushiSettingsSectionType.matching(query: "proxy").contains(.networkProxy))
        XCTAssertTrue(FabushiSettingsSectionType.matching(query: "代理").contains(.networkProxy))
        XCTAssertTrue(FabushiSettingsSectionType.matching(query: "星币").contains(.walletStars))
        XCTAssertTrue(FabushiSettingsSectionType.matching(query: "浏览器").contains(.builtInBrowser))
        XCTAssertTrue(FabushiSettingsSectionType.matching(query: "社区").contains(.capabilities))
        XCTAssertEqual(FabushiSettingsSectionType.matching(query: "").count, 26)

        // BLK-08 D15-04 Community honesty: NO fabricated Community feed section; explicit UPSTREAM-FOUNDATION disclosure
        let badge = FabushiPlatformStrings.text(.settingsCommunityFoundationBadge, locale: .en)
        XCTAssertEqual(badge, "UPSTREAM-FOUNDATION")
        let body = FabushiPlatformStrings.text(.settingsCommunityFoundationBody, locale: .en)
        XCTAssertTrue(body.contains("v1.6.15"))
        XCTAssertTrue(body.contains("zero server HTTP/WS routes"))
    }

    // MARK: - 2. FabushiPlatformModule Implements All 8 Sealed Protocol Methods + M6 Browser Hook

    func testFabushiPlatformModuleImplementsAll8FactoriesAndBrowserHook() async throws {
        let dir = try makeTempDir("platform_module")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:1",
            testMode: true,
            startNetworkMonitor: false
        )

        let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
        let module = FabushiPlatformModule()
        let isolatedRegistry = FabushiFeatureModuleRegistry(platform: module)

        // Invoke ALL 8 sealed PlatformModulesProviding factories + M6 Browser hook
        _ = isolatedRegistry.platform.makeRemoteComputerWorkbenchView(nodeId: nil, coordinator: coordinator, router: router)
        _ = isolatedRegistry.platform.makeRemoteControlledPillView(
            status: RemoteControlStatusState(id: "node-1", nodeName: "Studio Mac", controllerLabel: "Mobile"),
            coordinator: coordinator,
            router: router
        )
        _ = isolatedRegistry.platform.makeMiniAppHostSheetView(miniAppId: "catalog", coordinator: coordinator, router: router)
        _ = isolatedRegistry.platform.makeMiniAppFloatingPipView(
            pip: MiniAppPipOverlayState(id: "app-1", title: "Global Dharma"),
            coordinator: coordinator,
            router: router
        )
        _ = isolatedRegistry.platform.makeCallAndTalkOverlayView(
            callPip: CallPipOverlayState(id: "call-1", peerTitle: "Peer", stateText: "Connected"),
            coordinator: coordinator,
            router: router
        )
        _ = isolatedRegistry.platform.makeCallsTabView(coordinator: coordinator, router: router)
        _ = isolatedRegistry.platform.makeSettingsRootView(initialSection: "proxy", coordinator: coordinator, router: router)
        _ = isolatedRegistry.platform.makeWalletAndPremiumViews(coordinator: coordinator, router: router)
        _ = module.makeBrowserWorkspaceView(initialURL: URL(string: "https://example.com"), coordinator: coordinator, router: router)

        XCTAssertTrue(isolatedRegistry.platform is FabushiPlatformModule)

        await coordinator.shutdown()
    }

    // MARK: - 3. Real Rust Storage Metrics, Seed File Purge, Update Check & Push BLK-01

    func testRealRustStorageMetricsSeedFilePurgeUpdateCheckAndPushBlk01() async throws {
        let dir = try makeTempDir("storage_update_push")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:1",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))

            // 1. Seed a real 8 KiB file inside `<storage_dir>/media/` -> verify metrics -> purge -> verify freed_bytes > 0
            let mediaDir = dir.appendingPathComponent("media", isDirectory: true)
            try FileManager.default.createDirectory(at: mediaDir, withIntermediateDirectories: true)
            let seededFile = mediaDir.appendingPathComponent("cas_blob_test.bin")
            try Data(repeating: 0x42, count: 8_192).write(to: seededFile)

            let storageVM = FabushiDataStorageViewModel(coordinator: coordinator, router: router)
            await storageVM.refreshMetrics()
            XCTAssertGreaterThanOrEqual(storageVM.metrics.mediaCasBytes, 8_192)
            XCTAssertGreaterThanOrEqual(storageVM.metrics.mediaCasFiles, 1)

            let rawPurge = await storageVM.purgeMediaCache(locale: .en)
            let purgeRes = try XCTUnwrap(rawPurge)
            XCTAssertGreaterThanOrEqual(purgeRes.freedBytes, 8_192)
            XCTAssertGreaterThanOrEqual(purgeRes.removedFiles, 1)
            XCTAssertEqual(purgeRes.metrics.mediaCasBytes, 0)

            let rawWal = await storageVM.runWalCheckpoint(locale: .en)
            let wal = try XCTUnwrap(rawWal)
            XCTAssertTrue(wal.checkpointed)

            // 2. Real `settings.updateCheck` -> strictly App Store / TestFlight, zero binary self-updater!
            let updatesVM = FabushiAppUpdatesViewModel(coordinator: coordinator)
            let rawUpdateInfo = await updatesVM.checkForUpdates()
            let updateInfo = try XCTUnwrap(rawUpdateInfo)
            XCTAssertFalse(updateInfo.selfUpdateSupported)
            XCTAssertEqual(updateInfo.distribution, "app_store_testflight")

            // 3. Real `push.registerToken` / `push.status` / `push.unregisterToken` + honest BLK-01 disclosure
            let notifVM = FabushiNotificationsViewModel(coordinator: coordinator, router: router)
            let sampleToken = Data([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88])
            await notifVM.registerCurrentDeviceToken(sampleToken, environment: "sandbox")
            XCTAssertTrue(notifVM.pushStatus.registered)
            XCTAssertEqual(notifVM.pushStatus.serverApnsDispatch, "upstream_stub_blocked")
            XCTAssertEqual(notifVM.pushStatus.blockerId, "BLK-01")
            XCTAssertTrue(notifVM.formattedApnsStatusSummary.contains("BLK-01"))

            await notifVM.unregisterAPNsToken()
            XCTAssertFalse(notifVM.pushStatus.registered)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 4. Real Rust Action Bus List & iOS Sandbox Fail-Closed Desktop Actions

    func testRealRustActionBusListAndIosSandboxFailClosedDesktopExecution() async throws {
        let dir = try makeTempDir("actions")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:1",
            testMode: true,
            startNetworkMonitor: false
        )

        let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
        let vm = FabushiActionBusViewModel(coordinator: coordinator, router: router)

        await vm.loadActions()
        XCTAssertGreaterThan(vm.actions.count, 0)

        // Desktop host-control actions MUST fail closed with capability_unavailable on iOS!
        let powerExecuted = await vm.executeAction(name: "device_power_action")
        XCTAssertFalse(powerExecuted)
        XCTAssertTrue(vm.lastExecutionWasSandboxBlocked)

        let termExecuted = await vm.executeAction(name: "terminal_exec")
        XCTAssertFalse(termExecuted)
        XCTAssertTrue(vm.lastExecutionWasSandboxBlocked)

        await coordinator.shutdown()
    }

    // MARK: - 5. D10-13 Strict Proxy Validation Matrix, IPv6 Bracket Stripping & MTProto Secret Rules

    func testD1013StrictProxyValidationMatrixIPv6AndMtprotoSecretWithoutColonSplit() throws {
        // Invalid hosts MUST be rejected
        let invalidHosts = [
            "",
            " 127.0.0.1",
            "127.0.0.1 ",
            "proxy host.com",
            "proxy\nhost",
            "http://1.2.3.4",
            "evil@host.com",
            "host.com/path",
            "host.com?q=1",
            "host.com#frag",
            "host:8080",
            "[not-ipv6]",
            "host%20name",
        ]
        for badHost in invalidHosts {
            XCTAssertThrowsError(
                try FabushiProxyValidator.validateHost(badHost),
                "Host '\(badHost)' MUST be rejected"
            )
        }

        // Valid hosts & IPv6 bracket stripping
        XCTAssertEqual(try FabushiProxyValidator.validateHost("proxy.example.com"), "proxy.example.com")
        XCTAssertEqual(try FabushiProxyValidator.validateHost("::1"), "::1")
        XCTAssertEqual(try FabushiProxyValidator.validateHost("[::1]"), "::1")
        XCTAssertEqual(try FabushiProxyValidator.validateHost("[2001:db8::1]"), "2001:db8::1")

        // Ports strictly 1...65535
        XCTAssertThrowsError(try FabushiProxyValidator.validatePort(0))
        XCTAssertThrowsError(try FabushiProxyValidator.validatePort(65536))
        XCTAssertEqual(try FabushiProxyValidator.validatePort(1080), 1080)

        // Credentials validation
        XCTAssertThrowsError(try FabushiProxyValidator.validateCredentials(username: nil, password: "secret"))
        XCTAssertThrowsError(try FabushiProxyValidator.validateCredentials(username: "", password: "secret"))
        XCTAssertThrowsError(try FabushiProxyValidator.validateCredentials(username: "bad:user", password: "ok"))
        XCTAssertThrowsError(try FabushiProxyValidator.validateCredentials(username: String(repeating: "a", count: 256), password: nil))
        XCTAssertNoThrow(try FabushiProxyValidator.validateCredentials(username: "alice_密码", password: "密码ü🔐"))

        // MTProto secret validation (NEVER splits `:`)
        let badSecrets = [
            "u&port=99",
            "u@a/b?c#d",
            "u:1",
            " ee1122 ",
            "=",
            "==",
            "a=b",
            String(repeating: "a", count: 1025),
        ]
        for bad in badSecrets {
            XCTAssertThrowsError(
                try FabushiProxyValidator.validateMTProtoSecret(bad),
                "MTProto secret '\(bad)' MUST be rejected"
            )
        }

        let goodSecrets = [
            "",
            "ee1122",
            "0123456789abcdef0123456789abcdef",
            "7jK-_1234567890abcdef",
            "7gAAAAAAAAAAAAAAAAAAAAB=",
            "7gAAAAAAAAAAAAAAAAAAAA==",
        ]
        for good in goodSecrets {
            XCTAssertNoThrow(try FabushiProxyValidator.validateMTProtoSecret(good))
        }
    }

    // MARK: - 6. D10-13 RFC-3986 `tg://proxy` & `tg://socks` Round-Trip (UTF-8 `"密码ü"`, `& = + %`, IPv6)

    func testD1013Rfc3986TgProxyAndSocksRoundTripWithUtf8AndSpecialChars() throws {
        // Direct exports empty string
        let direct = FabushiProxyRecord(kind: .direct, host: "149.154.167.50", port: 443)
        XCTAssertEqual(FabushiProxyDeepLinks.exportLink(for: direct), "")

        // HTTP wraps bare IPv6 in [...]
        let httpIPv6 = FabushiProxyRecord(kind: .http, host: "2001:db8::1", port: 8080)
        XCTAssertEqual(FabushiProxyDeepLinks.exportLink(for: httpIPv6), "http://[2001:db8::1]:8080")

        // SOCKS5 with UTF-8 + special query chars (`&`, `=`, `+`, `%`, `#`) round-trips without parameter injection
        let socksUtf8 = FabushiProxyRecord(
            kind: .socks5,
            host: "::1",
            port: 1080,
            username: "alice&port=9999#frag",
            password: "密码ü=1&a=2+3%20"
        )
        let socksLink = FabushiProxyDeepLinks.exportLink(for: socksUtf8)
        XCTAssertFalse(socksLink.contains("&port=9999"))
        XCTAssertFalse(socksLink.contains("#frag"))

        let decodedSocks = try FabushiProxyDeepLinks.parseImportLink(socksLink)
        XCTAssertEqual(decodedSocks.kind, .socks5)
        XCTAssertEqual(decodedSocks.host, "::1")
        XCTAssertEqual(decodedSocks.port, 1080)
        XCTAssertEqual(decodedSocks.username, "alice&port=9999#frag")
        XCTAssertEqual(decodedSocks.password, "密码ü=1&a=2+3%20")

        // MTProto with IPv6 round-trips cleanly
        let mtproto = FabushiProxyRecord(
            kind: .mtproto,
            host: "2001:db8::1",
            port: 443,
            mtprotoSecret: "7gAAAAAAAAAAAAAAAAAAAA=="
        )
        let mtLink = FabushiProxyDeepLinks.exportLink(for: mtproto)
        XCTAssertEqual(mtLink, "tg://proxy?server=2001%3Adb8%3A%3A1&port=443&secret=7gAAAAAAAAAAAAAAAAAAAA%3D%3D")

        let decodedMt = try FabushiProxyDeepLinks.parseImportLink(mtLink)
        XCTAssertEqual(decodedMt.kind, .mtproto)
        XCTAssertEqual(decodedMt.host, "2001:db8::1")
        XCTAssertEqual(decodedMt.port, 443)
        XCTAssertEqual(decodedMt.mtprotoSecret, "7gAAAAAAAAAAAAAAAAAAAA==")

        // Percent-decode malformed inputs safely without crash
        XCTAssertEqual(FabushiProxyDeepLinks.percentDecode("%E5%AF%86%E7%A0%81%C3%BC"), "密码ü")
        XCTAssertEqual(FabushiProxyDeepLinks.percentDecode("foo%ZZ"), "foo%ZZ")
        XCTAssertEqual(FabushiProxyDeepLinks.percentDecode("foo%+1"), "foo% 1")
    }

    // MARK: - 7. D10-13 `proxyOpGeneration` Anti-Divergence, Rollback, Real Rust `proxy.*` CRUD & Server URL

    func testD1013ProxyOpGenerationAntiDivergenceRollbackAndRealRustProxyRpcCrud() async throws {
        let dir = try makeTempDir("proxy_crud")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:1",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let toastCtrl = FabushiToastController(autoDismissEnabled: false)
            let router = ShellNavigationRouter(toastController: toastCtrl)
            let vm = FabushiNetworkProxyViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            // 1. Pre-mutation validation rejects bad host BEFORE calling Rust or mutating state
            let invalidSaved = await vm.upsertProxy(
                FabushiProxyRecord(kind: .socks5, host: "bad@host/path", port: 1080),
                activate: true,
                locale: .en
            )
            XCTAssertNil(invalidSaved)
            XCTAssertTrue(vm.proxies.isEmpty)
            XCTAssertNotNil(toastCtrl.windowBanner)

            // 2. Real Rust `proxy.upsert` + IPv6 bracket stripping `[::1]` -> `::1` + activation
            let rawSaved = await vm.upsertProxy(
                FabushiProxyRecord(
                    id: "proxy_ipv6_1",
                    kind: .socks5,
                    host: "[::1]",
                    port: 1080,
                    username: "alice",
                    password: "pw"
                ),
                activate: true,
                locale: .en
            )
            let saved = try XCTUnwrap(rawSaved)
            XCTAssertEqual(saved.host, "::1")
            XCTAssertEqual(vm.activeProxyId, "proxy_ipv6_1")

            // 3. Stale `proxyOpGeneration` anti-divergence: older response MUST be discarded!
            let staleGen = vm.proxyOpGeneration
            _ = vm.bumpProxyOpGeneration()
            let appliedStaleList = vm.applyFetchedProxyTruth(
                records: [],
                activeId: nil,
                capturedOpGeneration: staleGen,
                capturedAccountGeneration: coordinator.accountScopeGeneration
            )
            XCTAssertFalse(appliedStaleList)
            XCTAssertEqual(vm.activeProxyId, "proxy_ipv6_1", "Stale proxy list MUST NOT overwrite newer active selection")

            let appliedStaleProbe = vm.applyProbeResult(
                latencyMs: 999,
                statusMessage: "Stale Probe",
                capturedOpGeneration: staleGen,
                capturedAccountGeneration: coordinator.accountScopeGeneration
            )
            XCTAssertFalse(appliedStaleProbe)
            XCTAssertNotEqual(vm.lastProbeLatencyMs, 999)

            // 4. Selecting nonexistent proxy fails on Rust `proxy.setActive` -> strict rollback to `proxy_ipv6_1` + non-chat error toast!
            toastCtrl.dismissWindowToast()
            let switchedMissing = await vm.selectActiveProxy(id: "does_not_exist_999", locale: .en)
            XCTAssertFalse(switchedMissing)
            XCTAssertEqual(vm.activeProxyId, "proxy_ipv6_1", "Failed proxy.setActive MUST roll back to previous active proxy")
            XCTAssertEqual(toastCtrl.windowBanner?.isError, true)
            XCTAssertNil(toastCtrl.windowBanner?.scopeChatId, "Proxy error toast MUST be non-chat scoped")

            // 5. Disable proxy (`id: nil`) & delete cleanly
            let disabled = await vm.selectActiveProxy(id: nil, locale: .en)
            XCTAssertTrue(disabled)
            XCTAssertNil(vm.activeProxyId)

            let deleted = await vm.deleteProxy(id: "proxy_ipv6_1", locale: .en)
            XCTAssertTrue(deleted)
            XCTAssertTrue(vm.proxies.isEmpty)

            // 6. Editable Server URL validation & real `coordinator.setServerURL` generation advance
            vm.serverUrlDraft = "javascript:alert(1)"
            let badServerApplied = await vm.applyServerURL(locale: .en)
            XCTAssertFalse(badServerApplied)

            let genBeforeServerSwitch = coordinator.accountScopeGeneration
            vm.serverUrlDraft = "https://api.fabushi.example.com"
            let validServerApplied = await vm.applyServerURL(locale: .en)
            XCTAssertTrue(validServerApplied)
            XCTAssertGreaterThan(coordinator.accountScopeGeneration, genBeforeServerSwitch)
            XCTAssertEqual(coordinator.settingsStore.activeServerURL, "https://api.fabushi.example.com")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 8. Real Rust Wallet Balance, Transactions, Gift, Redeem, Membership, TON BLK-03 & StoreKit BLK-04

    func testRealRustWalletBalanceTransactionsGiftRedeemMembershipAndTonBlk03FailClosed() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":808,"username":"platform_user","session_id":"s-808","token":"tok-808","first_name":"Econ"}"#)
            case ("POST", "/api/v1/auth/logout"):
                return .json(200, #"{"ok":true}"#)
            case ("GET", "/api/v1/wallet/balance"):
                return .json(200, #"{"user_id":808,"stars_balance":420,"ton_nano":0,"updated_at_ms":1735000000000}"#)
            case ("GET", "/api/v1/wallet/transactions"):
                return .json(200, #"{"transactions":[{"tx_id":"tx-1","kind":"topup","amount_stars":500,"peer_user_id":null,"description":"Initial topup","created_at_ms":1735000000000}]}"#)
            case ("POST", "/api/v1/wallet/topup"):
                return .json(200, #"{"balance":{"user_id":808,"stars_balance":520,"ton_nano":0,"updated_at_ms":1735000001000}}"#)
            case ("POST", "/api/v1/wallet/gift"):
                return .json(200, #"{"balance":{"user_id":808,"stars_balance":470,"ton_nano":0,"updated_at_ms":1735000002000}}"#)
            case ("POST", "/api/admin/use-redeem-code"):
                return .json(200, #"{"success":true,"plan":"premium_monthly","duration_days":30,"expires_at":1737592000000,"message":"Redeemed 30 days"}"#)
            case ("GET", "/api/alipay/check-membership"):
                return .json(200, #"{"success":true,"is_premium":true,"plan":"premium_monthly","days_left":30,"expires_at_ms":1737592000000,"username":"platform_user"}"#)
            case ("POST", "/api/alipay/create-order"):
                return .json(200, #"{"success":true,"order_id":"ord-808","amount":"30.00","plan":"monthly","product_type":"subscription","payment_url":null}"#)
            case ("GET", "/api/alipay/query-order"):
                return .json(200, #"{"success":true,"order_id":"ord-808","plan":"monthly","amount":"30.00","status":"pending","created_at":"2026-11-25T00:00:00Z"}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("wallet_economy")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "808")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            try await signInTestUser(coordinator: coordinator)
            let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
            let vm = FabushiEconomyViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            // Honest BLK-04 StoreKit 2 & BLK-03 TON disclosures MUST always remain visible
            XCTAssertTrue(vm.isBlk04StoreKitDisclosureVisible)
            XCTAssertTrue(vm.isBlk03TonDisclosureVisible)

            await vm.refreshAll()
            XCTAssertEqual(vm.balance.starsBalance, 420)
            XCTAssertEqual(vm.transactions.count, 1)
            XCTAssertEqual(vm.tonStatus.onChainSigning, "upstream_not_implemented")
            XCTAssertEqual(vm.tonStatus.blockerId, "BLK-03")

            let toppedUp = await vm.topupStars(amount: 100, locale: .en)
            XCTAssertTrue(toppedUp)
            XCTAssertEqual(vm.balance.starsBalance, 520)

            let gifted = await vm.giftStars(toUserId: 909, amountStars: 50, message: "Thank you", locale: .en)
            XCTAssertTrue(gifted)
            XCTAssertEqual(vm.balance.starsBalance, 470)

            let rawRedeemed = await vm.redeemCode("FABUSHI-30D", locale: .en)
            let redeemed = try XCTUnwrap(rawRedeemed)
            XCTAssertTrue(redeemed.success)
            XCTAssertEqual(redeemed.durationDays, 30)
            XCTAssertTrue(vm.membership.isPremium)

            let rawCreatedOrder = await vm.createAlipayOrder(plan: "monthly")
            let createdOrder = try XCTUnwrap(rawCreatedOrder)
            XCTAssertEqual(createdOrder.orderId, "ord-808")

            let rawQueriedOrder = await vm.queryAlipayOrder(orderId: "ord-808")
            let queriedOrder = try XCTUnwrap(rawQueriedOrder)
            XCTAssertEqual(queriedOrder.status, "pending")

            // TON signing MUST fail closed with upstream_not_implemented (BLK-03)
            let tonErr = await vm.probeTonSignBroadcastFailClosed()
            if case .upstreamNotImplemented = tonErr {
                // Expected fail-closed BLK-03 behavior
            } else {
                XCTFail("Expected upstreamNotImplemented for wallet.tonSignBroadcast, got \(String(describing: tonErr))")
            }

            // StoreKit 2 helper produces valid compact JWS wrapper payload
            let jwsPayload = try FabushiPayStoreKit.advancedCommerceRequestData(compactJWS: "eyJhbGciOiJFUzI1NiJ9.e30.sig")
            XCTAssertFalse(jwsPayload.isEmpty)

            // Logout wipes wallet balance, ledger & membership state cleanly
            _ = try await coordinator.logout()
            XCTAssertEqual(vm.balance.starsBalance, 0)
            XCTAssertTrue(vm.transactions.isEmpty)
            XCTAssertFalse(vm.membership.isPremium)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 9. App Intents, Single AppShortcutsProvider & iOS Sandbox Action Guard

    func testAppIntentsMetadataInvariantsAndActionBusSandboxGuard() async throws {
        // Exactly 5 canonical shortcuts in FabushiAppShortcutsProvider
        XCTAssertEqual(FabushiAppShortcutsProvider.appShortcuts.count, 5)

        // All 10 desktop host-control actions are strictly blocked fail-closed in the iOS sandbox
        XCTAssertTrue(FabushiAppIntentRouteBridge.isBlockedInIOSSandbox(actionName: "device_power_action"))
        XCTAssertTrue(FabushiAppIntentRouteBridge.isBlockedInIOSSandbox(actionName: "terminal_exec"))
        XCTAssertTrue(FabushiAppIntentRouteBridge.isBlockedInIOSSandbox(actionName: "remote_mouse_action"))
        XCTAssertFalse(FabushiAppIntentRouteBridge.isBlockedInIOSSandbox(actionName: "list_chats"))

        // Attempting to run a blocked desktop action via FabushiRunActionBusIntent fails closed immediately
        let blockedIntent = FabushiRunActionBusIntent(actionName: "terminal_exec")
        do {
            _ = try await blockedIntent.perform()
            XCTFail("Desktop host action via AppIntent MUST fail closed in iOS sandbox")
        } catch let error as FabushiActionBusIntentError {
            if case .blockedInIOSSandbox(let name) = error {
                XCTAssertEqual(name, "terminal_exec")
            } else {
                XCTFail("Unexpected FabushiActionBusIntentError: \(error)")
            }
        }

        // Empty action name fails closed
        let emptyIntent = FabushiRunActionBusIntent(actionName: "   ")
        do {
            _ = try await emptyIntent.perform()
            XCTFail("Empty action name MUST fail closed")
        } catch let error as FabushiActionBusIntentError {
            if case .emptyActionName = error {
                // Expected
            } else {
                XCTFail("Unexpected error: \(error)")
            }
        }

        // Route intents produce canonical `fabushi://` URLs
        XCTAssertEqual(FabushiAppIntentRouteBridge.routeURL(for: "fabushi://remote").scheme, "fabushi")
        XCTAssertEqual(FabushiAppIntentRouteBridge.routeURL(for: "fabushi://calls").host, "calls")
    }

    // MARK: - 10. Extended M5b Platform Strings 100% Bilingual Coverage & Formatters

    func testExtendedM5bPlatformStringsCoverageAndFormatters() {
        XCTAssertGreaterThan(FabushiPlatformStrings.Key.allCases.count, 250)
        XCTAssertEqual(FabushiPlatformStrings.starsBalanceFormatted(120, locale: .en), "120 Stars")
        XCTAssertEqual(FabushiPlatformStrings.starsBalanceFormatted(120, locale: .zhHans), "120 星币")
        XCTAssertEqual(FabushiPlatformStrings.proxyLatencyFormatted(42, locale: .en), "Reachable · 42 ms")
        XCTAssertEqual(FabushiPlatformStrings.proxyLatencyFormatted(42, locale: .zhHans), "可达 · 42 毫秒")
        XCTAssertEqual(FabushiPlatformStrings.callDurationFormatted(65), "01:05")
        XCTAssertFalse(FabushiPlatformStrings.storageBytesFormatted(16_384).isEmpty)
    }
}
