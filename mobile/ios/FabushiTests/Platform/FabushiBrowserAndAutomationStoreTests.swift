import Foundation
import os
import XCTest
@testable import Fabushi

@MainActor
final class FabushiBrowserAndAutomationStoreTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m5a_browser_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain() -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m5a.browser.\(UUID().uuidString)",
            account: "m5a-browser"
        )
    }

    // MARK: - 1. URL Gate & Strict `app_id` Validation Matrix

    func testValidateWKNavigationURLAndStrictAppIDValidation() {
        // Allowed URLs
        XCTAssertNotNil(FabushiBrowserSecurity.validateWKNavigationURL("https://fabushi.ombhrum.com/app"))
        XCTAssertNotNil(FabushiBrowserSecurity.validateWKNavigationURL("http://127.0.0.1:8080/test"))

        // Rejected unsafe URLs (fail-closed)
        XCTAssertNil(FabushiBrowserSecurity.validateWKNavigationURL("javascript:alert(1)"))
        XCTAssertNil(FabushiBrowserSecurity.validateWKNavigationURL("file:///etc/passwd"))
        XCTAssertNil(FabushiBrowserSecurity.validateWKNavigationURL("data:text/html,<script>alert(1)</script>"))
        XCTAssertNil(FabushiBrowserSecurity.validateWKNavigationURL("blob:https://example.com/1234"))
        XCTAssertNil(FabushiBrowserSecurity.validateWKNavigationURL("https://example.com/\nnewline"))
        XCTAssertNil(FabushiBrowserSecurity.validateWKNavigationURL(""))

        // Strict app_id validation
        XCTAssertTrue(FabushiBrowserSecurity.validateAutomationAppID("app_daily-check_01"))
        XCTAssertFalse(FabushiBrowserSecurity.validateAutomationAppID(""))
        XCTAssertFalse(FabushiBrowserSecurity.validateAutomationAppID("installed"))
        XCTAssertFalse(FabushiBrowserSecurity.validateAutomationAppID("INSTALLED"))
        XCTAssertFalse(FabushiBrowserSecurity.validateAutomationAppID("../escape"))
        XCTAssertFalse(FabushiBrowserSecurity.validateAutomationAppID("bad/slash"))
        XCTAssertFalse(FabushiBrowserSecurity.validateAutomationAppID(String(repeating: "a", count: 129)))
    }

    // MARK: - 2. WK_DOM_RECORDER_JS Redaction, Semantic Locator Priority & BrowserControlGate

    func testDomRecorderSensitiveRedactionSemanticLocatorPriorityAndControlGate() async throws {
        // Verify JS recorder script includes sensitive field guards
        XCTAssertTrue(FabushiBrowserScripts.wkDomRecorderJS.contains("current-password"))
        XCTAssertTrue(FabushiBrowserScripts.wkDomRecorderJS.contains("one-time-code"))
        XCTAssertTrue(FabushiBrowserScripts.wkDomRecorderJS.contains("[REDACTED]"))

        // Verify password / OTP / token payload redaction
        let passwordPayload: [String: Any] = [
            "event_type": "type",
            "page_url": "https://example.com/login",
            "locator": ["test_id": "login-password", "name_attr": "password", "css_selector": "#pwd"],
            "value": "SuperSecret123!",
            "is_secret": false,
            "parameter_name_hint": "user_password",
        ]
        let parsedPwd = FabushiBrowserWebViewRunner.parseRecorderPayload(passwordPayload)
        if case .fill(let locator, let value, let isSecret, _) = parsedPwd {
            XCTAssertTrue(isSecret, "Password field MUST be flagged isSecret == true")
            XCTAssertEqual(value, "[REDACTED]", "Raw password text MUST NEVER be stored in trace")
            XCTAssertTrue(locator.highestPriorityStrategy.hasPrefix("test_id:"))
        } else {
            XCTFail("Expected .fill action")
        }

        // Verify semantic locator priority cascade: test_id > aria_label > role+text_anchor > name_attr > css
        let loc1 = FabushiBrowserSemanticLocator(testId: "submit-btn", ariaLabel: "Submit", role: "button", textAnchor: "Go", cssSelector: ".btn", nameAttr: "go")
        XCTAssertEqual(loc1.highestPriorityStrategy, "test_id:submit-btn")

        let loc2 = FabushiBrowserSemanticLocator(ariaLabel: "Submit", role: "button", textAnchor: "Go", cssSelector: ".btn", nameAttr: "go")
        XCTAssertEqual(loc2.highestPriorityStrategy, "aria_label:Submit")

        let loc3 = FabushiBrowserSemanticLocator(role: "button", textAnchor: "Go", cssSelector: ".btn", nameAttr: "go")
        XCTAssertEqual(loc3.highestPriorityStrategy, "role+text:button:Go")

        let loc4 = FabushiBrowserSemanticLocator(cssSelector: ".btn", nameAttr: "go")
        XCTAssertEqual(loc4.highestPriorityStrategy, "name_attr:go")

        let loc5 = FabushiBrowserSemanticLocator(cssSelector: ".btn")
        XCTAssertEqual(loc5.highestPriorityStrategy, "css:.btn")

        // Verify BrowserControlGate transitions
        let dir = try makeTempDir("control_gate")
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

        let router = ShellNavigationRouter()
        let vm = FabushiBrowserViewModel(coordinator: coordinator, router: router)
        defer { vm.detachLifecycleHooks() }

        XCTAssertEqual(vm.controlMode, .idle)
        vm.beginAiControl(step: 1, statusText: "Navigating")
        XCTAssertEqual(vm.controlMode, .aiControlling(activeStep: 1, statusText: "Navigating"))

        vm.requestUserTakeover(reason: "CAPTCHA")
        XCTAssertEqual(vm.controlMode, .waitingForTakeover(reason: "CAPTCHA"))

        vm.takeOverByUser()
        XCTAssertEqual(vm.controlMode, .userTakeover(pausedAtStep: 1))

        vm.resumeAiControl(statusText: "Continuing")
        XCTAssertEqual(vm.controlMode, .aiControlling(activeStep: 1, statusText: "Continuing"))

        vm.cancelAiControl()
        XCTAssertEqual(vm.controlMode, .cancelled)

        await coordinator.shutdown()
    }

    // MARK: - 3. Cookie Codec + 512 KiB Cap + Real Rust Vault (`category: "browser_cookie"`) + Locked Fail-Closed

    func testCookieCodecAndRealRustVaultBrowserCookieSealOpenAndLockedFailClosed() async throws {
        // 1. Parse CDP JSON, EditThisCookie JSON & Netscape cookies.txt
        let cdpJSON = #"[{"name":"sid","value":"abc123","domain":".example.com","path":"/","secure":true,"httpOnly":true}]"#
        let cdpParsed = try FabushiBrowserCookieCodec.parseCookies(from: cdpJSON)
        XCTAssertEqual(cdpParsed.count, 1)
        XCTAssertEqual(cdpParsed[0].name, "sid")

        let netscape = """
        # Netscape HTTP Cookie File
        .example.com\tTRUE\t/\tTRUE\t1893456000\tpref\tdark
        #HttpOnly_.example.com\tTRUE\t/\tTRUE\t1893456000\tsess\txyz987
        """
        let netscapeParsed = try FabushiBrowserCookieCodec.parseCookies(from: netscape)
        XCTAssertEqual(netscapeParsed.count, 2)
        XCTAssertTrue(netscapeParsed[1].httpOnly)

        // 2. Enforce <= 512 KiB Vault bundle cap
        let oversizedValue = String(repeating: "x", count: 600_000)
        let oversizedCookies = [FabushiBrowserCookieRecord(name: "big", value: oversizedValue, domain: ".example.com")]
        XCTAssertThrowsError(
            try FabushiBrowserCookieCodec.encodeVaultBundle(profileKey: "default", cookies: oversizedCookies)
        ) { err in
            guard case FabushiBrowserPreflightError.vaultBundleTooLarge = err else {
                XCTFail("Expected vaultBundleTooLarge, got \(err)")
                return
            }
        }

        // 3. Real Rust Vault initNew(fast_kdf: true) -> sealItem(category: "browser_cookie") -> openItem -> lock fail-closed
        let dir = try makeTempDir("vault_cookies")
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
            struct KdfParams: Encodable, Sendable {
                let mCostKib: UInt32
                let tCost: UInt32
                let pCost: UInt32
                enum CodingKeys: String, CodingKey {
                    case mCostKib = "m_cost_kib"
                    case tCost = "t_cost"
                    case pCost = "p_cost"
                }
            }
            struct VaultInitParams: Encodable, Sendable {
                let userId: UInt64
                let passphrase: String
                let kdfParams: KdfParams
                enum CodingKeys: String, CodingKey {
                    case userId = "user_id"
                    case passphrase
                    case kdfParams = "kdf_params"
                }
            }
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("initNew"),
                params: VaultInitParams(
                    userId: 777,
                    passphrase: "VaultMasterPassword!9",
                    kdfParams: KdfParams(mCostKib: 16_384, tCost: 2, pCost: 1)
                )
            )

            let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
            let vm = FabushiBrowserViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            let sealedBlob = try await vm.backupActiveProfileToVault(
                userId: 777,
                cookiesOverride: netscapeParsed,
                pushToServer: false,
                locale: .en
            )
            XCTAssertEqual(sealedBlob["category"]?.stringValue, "browser_cookie")
            XCTAssertNotNil(sealedBlob["ciphertext_b64"]?.stringValue)

            let restored = try await vm.restoreProfileFromVaultBlob(sealedBlob, userId: 777, locale: .en)
            XCTAssertEqual(restored, netscapeParsed)

            // Lock Vault -> backup MUST fail closed with unauthorized!
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("lock"),
                params: FabushiEmptyParams()
            )
            do {
                _ = try await vm.backupActiveProfileToVault(
                    userId: 777,
                    cookiesOverride: netscapeParsed,
                    pushToServer: false,
                    locale: .en
                )
                XCTFail("Backing up browser profile when Vault is locked must fail closed")
            } catch let hostErr as FabushiHostError {
                XCTAssertEqual(hostErr.code, "unauthorized")
            }

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 4. All 7 `/api/v1/automation-apps*` REST Endpoints, Zero-LLM Replay & Logout Cache Wipe

    func testAutomationAppStoreAllSevenRestEndpointsZeroLlmReplayAndLogoutCacheWipe() async throws {
        let cleanScript = FabushiAutomationScriptApp(
            appId: "app_daily_digest",
            slug: "daily-digest",
            name: "Daily Digest",
            version: "1.0.0",
            description: "Fetches daily digest without LLM tokens",
            category: "productivity",
            targetDomains: ["example.com"],
            parameters: [
                FabushiAutomationAppParameterDto(name: "topic", description: "Topic", required: true, defaultValue: "news", isSecret: false),
                FabushiAutomationAppParameterDto(name: "api_token", description: "Vault secret", required: true, defaultValue: nil, isSecret: true),
            ],
            steps: [
                .navigate(urlTemplate: "https://example.com/{{topic}}"),
                .fill(
                    locator: FabushiBrowserSemanticLocator(testId: "token-box"),
                    valueTemplate: "{{api_token}}",
                    secretVaultKey: "api_token"
                ),
                .extractText(
                    locator: FabushiBrowserSemanticLocator(testId: "headline"),
                    outputKey: "title"
                ),
            ]
        )
        let manifestJSONData = try JSONEncoder().encode(cleanScript)
        let manifestJSONString = String(decoding: manifestJSONData, as: UTF8.self)
        let manifestSha = try FabushiBrowserSecurity.computeManifestSHA256(cleanScript)
        XCTAssertEqual(manifestSha.count, 64)

        let installedState = OSAllocatedUnfairLock(initialState: false)
        let runCountState = OSAllocatedUnfairLock(initialState: UInt64(0))

        let server = try M2bLoopbackHTTPTestServer { req in
            let manifestObj: Any = (try? JSONSerialization.jsonObject(with: Data(manifestJSONString.utf8))) ?? [:]
            switch (req.method, req.path) {
            case ("GET", "/api/v1/automation-apps"):
                return .jsonObject(200, [
                    "apps": [[
                        "app_id": "app_daily_digest",
                        "slug": "daily-digest",
                        "author_user_id": 10,
                        "name": "Daily Digest",
                        "description": "Fetches daily digest",
                        "category": "productivity",
                        "target_domains": ["example.com"],
                        "latest_version": "1.0.0",
                        "step_count": 3,
                        "install_count": 42,
                        "run_count": 100,
                        "is_published": true,
                        "is_installed": installedState.withLock { $0 },
                    ]],
                    "total": 1,
                    "limit": 20,
                    "offset": 0,
                ])
            case ("GET", "/api/v1/automation-apps/installed"):
                let isInst = installedState.withLock { $0 }
                let list: [[String: Any]] = isInst ? [[
                    "app_id": "app_daily_digest",
                    "slug": "daily-digest",
                    "name": "Daily Digest",
                    "installed_version": "1.0.0",
                    "latest_version": "1.0.0",
                    "manifest": manifestObj,
                    "content_sha256": manifestSha,
                    "installed_at_ms": 1_735_000_000_000,
                    "local_run_count": runCountState.withLock { $0 },
                ]] : []
                return .jsonObject(200, ["apps": list, "total": list.count])
            case ("GET", "/api/v1/automation-apps/app_daily_digest"):
                return .jsonObject(200, [
                    "app_id": "app_daily_digest",
                    "slug": "daily-digest",
                    "author_user_id": 10,
                    "name": "Daily Digest",
                    "latest_version": "1.0.0",
                    "versions": ["1.0.0"],
                    "manifest": manifestObj,
                    "content_sha256": manifestSha,
                    "is_published": true,
                    "is_installed": installedState.withLock { $0 },
                ])
            case ("POST", "/api/v1/automation-apps"):
                return .jsonObject(201, [
                    "app_id": "app_daily_digest",
                    "slug": "daily-digest",
                    "author_user_id": 10,
                    "name": "Daily Digest",
                    "latest_version": "1.0.0",
                    "versions": ["1.0.0"],
                    "manifest": manifestObj,
                    "content_sha256": manifestSha,
                    "is_published": true,
                    "is_installed": false,
                ])
            case ("POST", "/api/v1/automation-apps/app_daily_digest/install"):
                installedState.withLock { $0 = true }
                return .jsonObject(200, [
                    "ok": true,
                    "app": [
                        "app_id": "app_daily_digest",
                        "slug": "daily-digest",
                        "name": "Daily Digest",
                        "installed_version": "1.0.0",
                        "latest_version": "1.0.0",
                        "manifest": manifestObj,
                        "content_sha256": manifestSha,
                        "installed_at_ms": 1_735_000_000_000,
                        "local_run_count": 0,
                    ],
                    "install_count": 43,
                    "newly_installed": true,
                ])
            case ("DELETE", "/api/v1/automation-apps/app_daily_digest/install"):
                installedState.withLock { $0 = false }
                return .jsonObject(200, ["ok": true, "app_id": "app_daily_digest", "removed": true])
            case ("POST", "/api/v1/automation-apps/app_daily_digest/runs"):
                let nextCount = runCountState.withLock { val -> UInt64 in
                    val += 1
                    return val
                }
                return .jsonObject(200, [
                    "ok": true,
                    "app_id": "app_daily_digest",
                    "run_count": nextCount,
                    "last_run_at_ms": 1_735_000_001_000,
                ])
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("store_7_rest")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
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
            let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
            let vm = FabushiBrowserViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            // 1. Publish
            let published = try await vm.publishScriptApp(cleanScript, changelog: "Initial release", locale: .en)
            XCTAssertEqual(published.appId, "app_daily_digest")

            // 2. Search
            vm.storeSearchQuery = "daily"
            await vm.searchStore()
            XCTAssertEqual(vm.storeSummaries.count, 1)

            // 3. Get Detail
            let detail = await vm.loadAppDetail(appId: "app_daily_digest")
            XCTAssertEqual(detail?.contentSha256, manifestSha)

            // 4. Install
            let installed = await vm.installApp(appId: "app_daily_digest", locale: .en)
            XCTAssertEqual(installed?.appId, "app_daily_digest")

            // 5. List Installed + verify offline cache persisted in real settings.localSet
            await vm.refreshInstalledApps()
            XCTAssertEqual(vm.installedApps.count, 1)
            let cachedBeforeLogout = await vm.offlineCache.loadInstalledApps()
            XCTAssertEqual(cachedBeforeLogout.count, 1)

            // 6. Zero-LLM deterministic replay + Record Run telemetry (secret NEVER echoed to replayLogs!)
            let vaultSecretValue = "LIVE_VAULT_SECRET_NEVER_LOG_98765"
            var observedParams: [String: String] = [:]
            let report = await vm.runScriptApp(
                cleanScript,
                parameterValues: ["topic": "swift"],
                vaultSecretResolver: { key in key == "api_token" ? vaultSecretValue : nil },
                stepExecutorOverride: { step, resolved, outputs in
                    observedParams = resolved
                    if case .extractText(_, let outputKey) = step {
                        outputs[outputKey] = "Swift 6 News"
                    }
                },
                recordTelemetry: true,
                locale: .en
            )
            XCTAssertTrue(report.success)
            XCTAssertFalse(report.usedLlm, "Deterministic script replay MUST use zero LLM tokens")
            XCTAssertEqual(report.stepsExecuted, 3)
            XCTAssertEqual(report.extractedOutputs["title"], "Swift 6 News")
            XCTAssertEqual(observedParams["api_token"], vaultSecretValue)
            XCTAssertFalse(
                vm.replayLogs.joined(separator: "\n").contains(vaultSecretValue),
                "Secret parameter values MUST NEVER be echoed into UI replay logs!"
            )
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/automation-apps/app_daily_digest/runs").count, 1)

            // 7. Uninstall
            let uninstalled = await vm.uninstallApp(appId: "app_daily_digest", locale: .en)
            XCTAssertTrue(uninstalled)
            XCTAssertTrue(vm.installedApps.isEmpty)

            // Re-seed cache then simulate logout / account scope reset -> offline cache MUST wipe!
            await vm.offlineCache.saveInstalledApps([try XCTUnwrap(installed)])
            let seeded = await vm.offlineCache.loadInstalledApps()
            XCTAssertEqual(seeded.count, 1)

            vm.handleAccountScopeReset(generation: coordinator.accountScopeGeneration + 1)
            for _ in 0..<40 {
                let remaining = await vm.offlineCache.loadInstalledApps()
                if remaining.isEmpty { break }
                try await Task.sleep(nanoseconds: 25_000_000)
            }
            let remainingAfterWipe = await vm.offlineCache.loadInstalledApps()
            XCTAssertTrue(remainingAfterWipe.isEmpty, "Account switch / logout MUST wipe Automation App Store offline cache")
            XCTAssertTrue(vm.runner.webView.didTeardownSession)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 5. Fail-Closed Secret-Leak Preflight Matrix

    func testFailClosedSecretLeakPreflightMatrix() {
        let leakSamples: [(label: String, value: String)] = [
            ("jwt", "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"),
            ("bearer", "Bearer supersecretbearervalue123456"),
            ("openai_key", "sk-liveSecretKeyValue9876543210"),
            ("github_pat", "ghp_1234567890abcdefghijklmnop"),
            ("hardcoded_password", "password=MyPlaintextPass123"),
            ("cookie_auth", "auth_token=deadbeef0123456789"),
            ("cookie_ct0", "ct0=abcdef1234567890"),
            ("private_key", "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQE..."),
        ]

        for sample in leakSamples {
            let leakyApp = FabushiAutomationScriptApp(
                appId: "app_leak_\(sample.label)",
                slug: "leak-test",
                name: "Leaky App",
                steps: [
                    .fill(
                        locator: FabushiBrowserSemanticLocator(testId: "note-box"),
                        valueTemplate: sample.value,
                        secretVaultKey: nil
                    ),
                ]
            )
            do {
                try FabushiBrowserSecurity.validateScriptAppSafeForPublish(leakyApp)
                XCTFail("Expected secret leak preflight to block \(sample.label)")
            } catch let err as FabushiBrowserPreflightError {
                guard case .secretLeakDetected(let findings) = err else {
                    XCTFail("Expected secretLeakDetected for \(sample.label), got \(err)")
                    return
                }
                XCTAssertFalse(findings.isEmpty)
                XCTAssertTrue(
                    findings.contains { $0.fieldPath == "app.steps[0].value_template" },
                    "Finding for \(sample.label) must identify exact offending step path"
                )
            } catch {
                XCTFail("Unexpected error for \(sample.label): \(error)")
            }
        }

        // Secret parameter carrying default_value MUST fail closed
        let defaultSecretApp = FabushiAutomationScriptApp(
            appId: "app_secret_default",
            slug: "secret-default",
            name: "Secret Default App",
            parameters: [
                FabushiAutomationAppParameterDto(
                    name: "login_password",
                    description: "Password",
                    required: true,
                    defaultValue: "leaked-default",
                    isSecret: true
                ),
            ],
            steps: [.navigate(urlTemplate: "https://example.com")]
        )
        XCTAssertThrowsError(try FabushiBrowserSecurity.validateScriptAppSafeForPublish(defaultSecretApp))
    }

    // MARK: - 6. `FabushiPlatformStrings` 100% Bilingual Coverage & Anti-Sample Hygiene

    func testPlatformStringsBilingualCoverageAndNoForbiddenSampleTitles() {
        let forbiddenPrefixes = ["fabushi.", "remote.", "miniapp.", "browser.", "call.", "settings.", "hig."]
        let forbiddenSamples = ["Saved Messages", "Fabushi Community", "Sample Chat", "Demo Bot Chat", "Chief of Staff"]

        XCTAssertGreaterThan(FabushiPlatformStrings.Key.allCases.count, 100)
        for key in FabushiPlatformStrings.Key.allCases {
            for locale in [FabushiLocale.en, FabushiLocale.zhHans] {
                let value = FabushiPlatformStrings.text(key, locale: locale)
                XCTAssertFalse(
                    value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                    "Missing localized value for \(key.rawValue) in \(locale.rawValue)"
                )
                for prefix in forbiddenPrefixes {
                    XCTAssertFalse(
                        value.lowercased().hasPrefix(prefix),
                        "Raw key prefix leaked for \(key.rawValue): \(value)"
                    )
                }
                for forbidden in forbiddenSamples {
                    XCTAssertFalse(
                        value.contains(forbidden),
                        "Forbidden sample title '\(forbidden)' found in \(key.rawValue): \(value)"
                    )
                }
            }
        }
    }
}
