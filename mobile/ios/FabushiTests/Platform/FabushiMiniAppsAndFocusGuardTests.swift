import CryptoKit
import Foundation
import os
import WebKit
import XCTest
@testable import Fabushi

@MainActor
final class FabushiMiniAppsAndFocusGuardTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m5a_miniapp_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain() -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m5a.miniapp.\(UUID().uuidString)",
            account: "m5a-miniapp"
        )
    }

    private func signInTestUser(coordinator: FabushiCoordinator) async throws {
        let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
            method: FabushiRPCNamespace.auth.method("login"),
            params: FabushiLegacyLoginRequest(username: "miniapp_user", password: "StrongPassword!1"),
            extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
        )
    }

    // MARK: - 1. D10-12 `startMinimized: true` Direct PiP, In-Place Restore & Unified `minimize()` + Toast Reconcile

    func testStartMinimizedDirectPiPInPlaceRestoreAndUnifiedMinimizeToastReconcile() async throws {
        let dir = try makeTempDir("start_minimized_pip")
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
            let pool = FabushiMiniAppSessionPool()
            let vm = FabushiMiniAppHostViewModel(coordinator: coordinator, router: router, sessionPool: pool)
            defer {
                vm.detachLifecycleHooks()
                pool.teardownAll()
            }

            let app = FabushiMiniAppEntry(
                appId: "app-weather",
                botUserId: 7001,
                botUsername: "weather_bot",
                botDisplayName: "Weather",
                shortName: "forecast",
                title: "Weather Mini App",
                webUrl: "https://fabushi.ombhrum.com/miniapps/weather/",
                appTosAccepted: true
            )

            // 1. startMinimized: true -> mounts directly into PiP WITHOUT flashing sheet or grabbing focus!
            let launched = await vm.launchMiniApp(app, startMinimized: true, locale: .en)
            XCTAssertTrue(launched)
            XCTAssertNil(router.presentedMiniAppId, "startMinimized MUST NOT set presentedMiniAppId")
            XCTAssertEqual(router.activeMiniAppPip?.id, app.appKey)

            let pooled = try XCTUnwrap(pool.existingSession(generation: coordinator.accountScopeGeneration, appKey: app.appKey))
            XCTAssertFalse(pooled.webView.allowFirstResponder, "PiP background WebMCP MUST keep allowFirstResponder == false")
            XCTAssertFalse(pooled.webView.canBecomeFirstResponder)
            XCTAssertFalse(pooled.webView.becomeFirstResponder())
            XCTAssertEqual(pooled.loadCount, 1)

            // 2. In-place restore from PiP -> preserves same WebView with ZERO reload!
            vm.restoreFromPiP(appKey: app.appKey)
            XCTAssertEqual(router.presentedMiniAppId, app.appKey)
            XCTAssertEqual(pooled.loadCount, 1, "Restoring from PiP must reuse live WKWebView in-place without reloading")
            XCTAssertTrue(pooled.webView.allowFirstResponder)

            // 3. Seed chat-scoped toast + non-chat toast, then call unified minimize()
            toastCtrl.showChatToast("Chat scoped notice", chatId: 42)
            toastCtrl.showNonChatToast("Global platform notice")
            vm.minimize(reason: .openBotProfile, isCompact: true, locale: .en)

            XCTAssertEqual(vm.lastMinimizeReason, .openBotProfile)
            XCTAssertNil(router.presentedMiniAppId)
            XCTAssertEqual(router.activeMiniAppPip?.id, app.appKey)
            XCTAssertFalse(pooled.webView.allowFirstResponder, "Unified minimize() MUST revoke WebView first responder")
            XCTAssertGreaterThanOrEqual(pooled.webView.domBlurEvaluationCount, 1)
            XCTAssertEqual(toastCtrl.windowBanner?.message, "Global platform notice", "Chat-scoped toast purged while non-chat toast survives")
            XCTAssertNil(toastCtrl.windowBanner?.scopeChatId)

            // 4. Explicit Close tears down WebView cleanly
            vm.closeActiveSession(isCompact: true)
            XCTAssertTrue(pooled.webView.didTeardownSession)
            XCTAssertNil(router.activeMiniAppPip)
            XCTAssertEqual(pool.activeCount, 0)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 2. D10-12 `CreateManagedBotBox` `originBoxId` Close-Race Guard

    func testCreateManagedBotBoxOriginBoxIdCloseRaceGuard() async throws {
        let shouldFailNext = OSAllocatedUnfairLock(initialState: false)
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":601,"username":"miniapp_user","session_id":"s-601","token":"tok-601","first_name":"Maker"}"#)
            case ("POST", "/api/v1/botfather/nl-create"):
                if shouldFailNext.withLock({ $0 }) {
                    return .json(500, #"{"error":"nl_create_failed"}"#)
                }
                return .jsonObject(200, [
                    "bot": [
                        "bot_user_id": 9001,
                        "owner_user_id": 601,
                        "username": "habit_bot",
                        "display_name": "Habit Tracker",
                    ],
                    "bot_token": "tok-bot-9001",
                    "companion_chat_id": 900100,
                    "summary_markdown": "Created Habit Tracker",
                ])
            case ("GET", "/api/v1/miniapps"):
                return .jsonObject(200, [
                    "apps": [
                        [
                            "app_id": "app-habit",
                            "bot_user_id": 9001,
                            "bot_username": "habit_bot",
                            "bot_display_name": "Habit Tracker",
                            "short_name": "habits",
                            "title": "Habit Tracker",
                            "web_url": "https://fabushi.ombhrum.com/miniapps/habits/",
                            "app_tos_accepted": true,
                        ],
                    ],
                ])
            case ("GET", "/api/v1/miniapps/installed"), ("GET", "/api/v1/miniapps/recent"):
                return .json(200, #"{"apps":[]}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("create_bot_race")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "601")
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
            let toastCtrl = FabushiToastController(autoDismissEnabled: false)
            let router = ShellNavigationRouter(toastController: toastCtrl)
            let pool = FabushiMiniAppSessionPool()
            let vm = FabushiMiniAppHostViewModel(coordinator: coordinator, router: router, sessionPool: pool)
            defer {
                vm.detachLifecycleHooks()
                pool.teardownAll()
            }

            // Case A: User opens CreateManagedBotBox, dismisses it mid-flight, request succeeds
            let boxIdA = vm.openCreateManagedBotBox()
            vm.closeCreateManagedBotBox(boxId: boxIdA)
            let routeBefore = router.activeRoute

            let created = await vm.submitCreateManagedBot(
                prompt: "Create a daily habit tracker mini app",
                originBoxId: boxIdA,
                locale: .en
            )
            XCTAssertTrue(created)
            XCTAssertFalse(vm.lastCreatedBotRouteHijacked, "Dismissed modal MUST NOT hijack active route")
            XCTAssertEqual(router.activeRoute, routeBefore)
            XCTAssertNil(vm.createBotModalError)
            XCTAssertEqual(vm.catalogApps.count, 1, "Catalog must still refresh when generation matches")
            XCTAssertEqual(toastCtrl.windowBanner?.scopeChatId, nil)

            // Case B: User opens box B, dismisses box B, opens fresh box C; box B fails -> MUST NOT mutate box C's modal error!
            let boxIdB = vm.openCreateManagedBotBox()
            vm.closeCreateManagedBotBox(boxId: boxIdB)
            let boxIdC = vm.openCreateManagedBotBox()
            XCTAssertEqual(vm.activeCreateBoxId, boxIdC)

            shouldFailNext.withLock { $0 = true }
            toastCtrl.clearAll()
            let failed = await vm.submitCreateManagedBot(
                prompt: "Failing bot creation",
                originBoxId: boxIdB,
                locale: .en
            )
            XCTAssertFalse(failed)
            XCTAssertNil(vm.createBotModalError, "Stale failure from dismissed box B must NOT pollute reopened box C")
            XCTAssertTrue(toastCtrl.windowBanner?.isError == true)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 3. D12-01 Focus-Gated WKWebView Policy, Autofocus Blur & Teardown

    func testD1201FocusGatedWebViewPolicyDomBlurAndTeardown() {
        // Policy truth table
        XCTAssertTrue(
            FabushiWebViewFocusPolicy.shouldWebViewOwnKeyboard(
                FabushiWebViewFocusContext(isPresented: true)
            )
        )
        XCTAssertFalse(
            FabushiWebViewFocusPolicy.shouldWebViewOwnKeyboard(
                FabushiWebViewFocusContext(isPresented: false)
            )
        )
        XCTAssertFalse(
            FabushiWebViewFocusPolicy.shouldWebViewOwnKeyboard(
                FabushiWebViewFocusContext(isPresented: true, isMinimizedToPiP: true)
            )
        )
        XCTAssertFalse(
            FabushiWebViewFocusPolicy.shouldWebViewOwnKeyboard(
                FabushiWebViewFocusContext(isPresented: true, isBackgroundWebMcp: true)
            )
        )
        XCTAssertFalse(
            FabushiWebViewFocusPolicy.shouldWebViewOwnKeyboard(
                FabushiWebViewFocusContext(isPresented: true, hasBlockingModal: true)
            )
        )
        XCTAssertFalse(
            FabushiWebViewFocusPolicy.shouldWebViewOwnKeyboard(
                FabushiWebViewFocusContext(isPresented: true, isHostInputFocused: true)
            )
        )

        // Subclass runtime enforcement
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let webView = FabushiFocusGatedWKWebView(frame: CGRect(x: 0, y: 0, width: 320, height: 240), configuration: config)
        let navDelegate = FabushiFocusGatedNavigationDelegate()
        webView.navigationDelegate = navDelegate

        XCTAssertFalse(webView.allowFirstResponder)
        XCTAssertFalse(webView.canBecomeFirstResponder)
        XCTAssertFalse(webView.becomeFirstResponder())

        webView.passThroughTouchesWhenNonInteractive = true
        XCTAssertNil(webView.hitTest(CGPoint(x: 20, y: 20), with: nil))

        // didFinish MUST auto-blur DOM when !allowFirstResponder (blocks <input autofocus>)
        let blursBefore = webView.domBlurEvaluationCount
        navDelegate.webView(webView, didFinish: nil)
        XCTAssertEqual(webView.domBlurEvaluationCount, blursBefore + 1)

        // Weak handler registration + explicit teardownSession()
        @MainActor
        final class DummyHandler: NSObject, WKScriptMessageHandler {
            func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {}
        }
        let handler = DummyHandler()
        webView.registerWeakMessageHandler(handler, name: "testBridge")
        XCTAssertTrue(webView.registeredMessageHandlerNames.contains("testBridge"))

        webView.teardownSession()
        XCTAssertTrue(webView.didTeardownSession)
        XCTAssertFalse(webView.allowFirstResponder)
        XCTAssertTrue(webView.registeredMessageHandlerNames.isEmpty)
        XCTAssertNil(webView.navigationDelegate)
    }

    // MARK: - 4. Mandatory ToS Decline Blocking & Accept Flow

    func testTosDeclineBlocksLaunchAndAcceptProceeds() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":602,"username":"miniapp_user","session_id":"s-602","token":"tok-602","first_name":"Maker"}"#)
            case ("POST", "/api/v1/bots/8100/accept-app-tos"):
                return .json(200, #"{"ok":true}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("tos_gate")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "602")
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
            let router = ShellNavigationRouter()
            let pool = FabushiMiniAppSessionPool()
            let vm = FabushiMiniAppHostViewModel(coordinator: coordinator, router: router, sessionPool: pool)
            defer {
                vm.detachLifecycleHooks()
                pool.teardownAll()
            }

            let unacceptedApp = FabushiMiniAppEntry(
                appId: "app-thirdparty",
                botUserId: 8100,
                botUsername: "thirdparty_bot",
                shortName: "tool",
                title: "Third-Party Tool",
                webUrl: "https://fabushi.ombhrum.com/miniapps/tool/",
                appTosAccepted: false
            )

            // Launch triggers ToS modal, does not mount session
            let launchedBeforeTos = await vm.launchMiniApp(unacceptedApp, startMinimized: false, locale: .en)
            XCTAssertFalse(launchedBeforeTos)
            XCTAssertNotNil(vm.pendingTosPrompt)
            XCTAssertNil(vm.activeSession)

            // Decline blocks launch and displays tosBlockedMessage
            vm.declinePendingTos(locale: .en)
            XCTAssertNil(vm.pendingTosPrompt)
            XCTAssertNotNil(vm.tosBlockedMessage)
            XCTAssertNil(vm.activeSession)

            // Re-open & accept ToS -> calls /api/v1/bots/8100/accept-app-tos -> launches cleanly
            _ = await vm.launchMiniApp(unacceptedApp, startMinimized: false, locale: .en)
            let acceptedAndLaunched = await vm.acceptPendingTos(locale: .en)
            XCTAssertTrue(acceptedAndLaunched)
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/bots/8100/accept-app-tos").count, 1)
            XCTAssertNotNil(vm.activeSession)
            XCTAssertNil(vm.tosBlockedMessage)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 5. Fail-Closed `.fabapp` Ed25519 Verification (`BLK-06`)

    func testFabAppPackageVerifierValidAndTamperedFailClosed() throws {
        let signingKey = Curve25519.Signing.PrivateKey()
        let htmlBytes = Data("<!doctype html><html><body>Global Dharma</body></html>".utf8)
        let htmlHash = SHA256.hash(data: htmlBytes).map { String(format: "%02x", $0) }.joined()

        let unsignedManifestObj: [String: Any] = [
            "manifestVersion": 1,
            "packageClass": "catalog",
            "appId": "global-dharma",
            "publisherId": "ombhrum-official",
            "version": "1.0.0",
            "displayName": "Global Dharma",
            "entrypoint": "index.html",
            "runtime": "web",
            "minHostApi": 1,
            "offlineCore": true,
            "requestedCapabilities": [],
            "networkPolicy": [
                "mode": "deny",
                "allowedOrigins": [],
            ],
            "files": [
                "index.html": [
                    "sha256": htmlHash,
                    "bytes": htmlBytes.count,
                    "mime": "text/html",
                ],
            ],
            "signature": [
                "algorithm": "Ed25519",
                "keyId": "prod-k1",
                "value": "placeholder",
            ],
        ]
        let unsignedData = try JSONSerialization.data(withJSONObject: unsignedManifestObj, options: [.sortedKeys])
        let canonicalPayload = try FabAppPackageVerifier.canonicalSigningPayloadForTests(manifestData: unsignedData)
        let signature = try signingKey.signature(for: canonicalPayload)

        var signedManifestObj = unsignedManifestObj
        signedManifestObj["signature"] = [
            "algorithm": "Ed25519",
            "keyId": "prod-k1",
            "value": signature.base64EncodedString(),
        ]
        let signedManifestData = try JSONSerialization.data(withJSONObject: signedManifestObj, options: [.sortedKeys])

        let trust = FabAppCatalogTrust(
            appId: "global-dharma",
            publisherId: "ombhrum-official",
            keyId: "prod-k1",
            publicKey: signingKey.publicKey.rawRepresentation,
            revoked: false
        )

        // Valid signed bundle succeeds
        let verified = try FabAppPackageVerifier.verifyCatalogPackage(
            manifestData: signedManifestData,
            resources: ["index.html": htmlBytes],
            trust: trust,
            hostApi: 1
        )
        XCTAssertEqual(verified.appId, "global-dharma")
        XCTAssertTrue(verified.offlineCore)

        // Tampered file bytes MUST fail closed
        let tamperedBytes = Data("<!doctype html><html><body>Tampered!</body></html>".utf8)
        XCTAssertThrowsError(
            try FabAppPackageVerifier.verifyCatalogPackage(
                manifestData: signedManifestData,
                resources: ["index.html": tamperedBytes],
                trust: trust,
                hostApi: 1
            )
        )

        // Revoked trust key MUST fail closed
        let revokedTrust = FabAppCatalogTrust(
            appId: "global-dharma",
            publisherId: "ombhrum-official",
            keyId: "prod-k1",
            publicKey: signingKey.publicKey.rawRepresentation,
            revoked: true
        )
        XCTAssertThrowsError(
            try FabAppPackageVerifier.verifyCatalogPackage(
                manifestData: signedManifestData,
                resources: ["index.html": htmlBytes],
                trust: revokedTrust,
                hostApi: 1
            )
        )
    }

    // MARK: - 6. Real Rust 全球法布施 (`miniapp.dharma*`), URI/URL/HMAC & `webmcp.*` RPCs

    func testGlobalDharmaRustStateMachineSqliteAndMiniAppWebMcpRpcs() async throws {
        let dir = try makeTempDir("dharma_webmcp")
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
            let router = ShellNavigationRouter()
            let pool = FabushiMiniAppSessionPool()
            let vm = FabushiMiniAppHostViewModel(coordinator: coordinator, router: router, sessionPool: pool)
            defer {
                vm.detachLifecycleHooks()
                pool.teardownAll()
            }

            // 1. 全球法布施 bootstrap + state machine dispatch + practice record + leaderboard + embedded WASM asset
            await vm.loadGlobalDharma()
            XCTAssertNotNil(vm.dharmaBootstrap)
            XCTAssertNotNil(vm.dharmaSnapshot)
            XCTAssertNotNil(vm.dharmaLeaderboard)

            let wasmInfo = try XCTUnwrap(vm.dharmaWasmAssetInfo)
            XCTAssertTrue(wasmInfo.hasWasmMagic, "Embedded fabushi_miniapp.wasm must start with \\0asm magic")
            XCTAssertGreaterThan(wasmInfo.sizeBytes, 0)

            let startRes = await vm.dispatchDharmaCommand(type: "start_practice_session")
            XCTAssertNotNil(startRes)
            XCTAssertTrue(vm.isPracticeSessionActive)

            let chantRes = await vm.dispatchDharmaCommand(type: "increment_chant")
            XCTAssertNotNil(chantRes)
            XCTAssertEqual(vm.localChantCount, 1)

            let recordRes = await vm.recordDharmaPractice(
                sutra: "心经",
                durationMinutes: 15,
                chantCount: 7,
                notes: "M5a verification practice",
                locale: .en
            )
            XCTAssertNotNil(recordRes)

            // 2. miniapp.parseUri & miniapp.sanitizeCustomUrl
            let parsed = await vm.parseMiniAppUri("gpui-miniapp://dharma_bot/global?startapp=sutra")
            XCTAssertEqual(parsed?.valid, true)
            XCTAssertEqual(parsed?.botUsername, "dharma_bot")
            XCTAssertEqual(parsed?.shortName, "global")

            let safeUrl = await vm.sanitizeCustomUrl("https://fabushi.ombhrum.com/miniapps/dharma")
            XCTAssertEqual(safeUrl, "https://fabushi.ombhrum.com/miniapps/dharma")
            let blockedJsUrl = await vm.sanitizeCustomUrl("javascript:alert(1)")
            XCTAssertNil(blockedJsUrl)
            let blockedFileUrl = await vm.sanitizeCustomUrl("file:///etc/passwd")
            XCTAssertNil(blockedFileUrl)

            // 3. miniapp.verifyInitDataLocal with real Telegram WebApp HMAC-SHA256
            let botToken = "123456:TEST-BOT-TOKEN-XYZ"
            let dataCheckString = "auth_date=1735000000\nquery_id=AAHdF6IQAAAAAN0XohDhrOrc"
            let secretKey = HMAC<SHA256>.authenticationCode(
                for: Data(botToken.utf8),
                using: SymmetricKey(data: Data("WebAppData".utf8))
            )
            let hashBytes = HMAC<SHA256>.authenticationCode(
                for: Data(dataCheckString.utf8),
                using: SymmetricKey(data: Data(secretKey))
            )
            let validHashHex = hashBytes.map { String(format: "%02x", $0) }.joined()
            let validInitData = "query_id=AAHdF6IQAAAAAN0XohDhrOrc&auth_date=1735000000&hash=\(validHashHex)"
            let tamperedInitData = "query_id=AAHdF6IQAAAAAN0XohDhrOrc&auth_date=1735000001&hash=\(validHashHex)"

            let validCheck = await vm.verifyInitDataLocal(initData: validInitData, botToken: botToken)
            XCTAssertTrue(validCheck)
            let invalidCheck = await vm.verifyInitDataLocal(initData: tamperedInitData, botToken: botToken)
            XCTAssertFalse(invalidCheck)

            // 4. webmcp.setControlMode / getControlState / validateToolCall
            let setModeRes = await vm.setWebMcpControlMode(appKey: "dharma_bot/global", mode: .aiControlling)
            XCTAssertEqual(setModeRes?.mode, .aiControlling)
            let getModeRes = await vm.getWebMcpControlState(appKey: "dharma_bot/global")
            XCTAssertEqual(getModeRes?.mode, .aiControlling)

            let manifestJSON = """
            {
              "app_id": "global-dharma",
              "version": "1.0.0",
              "readable_state_schema": {"type": "object"},
              "actions": [
                {
                  "name": "increment_chant",
                  "description": "Increment chant counter",
                  "input_schema": {
                    "type": "object",
                    "required": ["count"],
                    "properties": {"count": {"type": "integer"}}
                  },
                  "output_schema": {"type": "object"},
                  "risk_level": "low",
                  "requires_approval": false
                }
              ]
            }
            """
            let validTool = await vm.validateWebMcpToolCall(
                manifestJSON: manifestJSON,
                action: "increment_chant",
                args: .object(["count": .int(1)])
            )
            if case .failure(let err) = validTool {
                XCTFail("Expected valid WebMCP tool call to pass, got error: \(err)")
            }
            let invalidTool = await vm.validateWebMcpToolCall(
                manifestJSON: manifestJSON,
                action: "increment_chant",
                args: .object([:])
            )
            if case .success = invalidTool {
                XCTFail("Expected missing required argument to fail WebMCP schema validation")
            }

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
