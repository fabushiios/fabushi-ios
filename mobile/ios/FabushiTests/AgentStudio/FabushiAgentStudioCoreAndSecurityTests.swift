import Foundation
import SwiftUI
import UIKit
import XCTest
@testable import Fabushi

@MainActor
final class FabushiAgentStudioCoreAndSecurityTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m4a_studio_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain(_ suffix: String) -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m4a.studio.\(suffix).\(UUID().uuidString)",
            account: "m4a-\(suffix)"
        )
    }

    private func makeRouter() -> ShellNavigationRouter {
        ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
    }

    // MARK: - 1. Bilingual String Catalog Parity (100% of `FabushiAgentStudioStrings.allKeys`)

    func testBilingualStringCatalogParityAndZeroRawKeyLeaks() throws {
        let hanRegex = try NSRegularExpression(pattern: "\\p{Han}")
        let asciiLetterRegex = try NSRegularExpression(pattern: "[A-Za-z]")
        let forbiddenSampleTitles = [
            "Saved Messages",
            "Fabushi Community",
            "Sample Chat",
            "Demo Bot Chat",
            "Chief of Staff",
        ]

        let keys = FabushiAgentStudioStrings.allKeys
        XCTAssertGreaterThanOrEqual(keys.count, 100)

        for key in keys {
            XCTAssertTrue(key.hasPrefix("m4."), "M4 key '\(key)' must use 'm4.' prefix")
            let zh = FabushiAgentStudioStrings.text(key, locale: .zhHans)
            let en = FabushiAgentStudioStrings.text(key, locale: .en)

            XCTAssertFalse(zh.isEmpty, "Empty zh-Hans value for \(key)")
            XCTAssertFalse(en.isEmpty, "Empty en value for \(key)")
            XCTAssertNotEqual(zh, key, "zh-Hans fell back to raw key \(key)")
            XCTAssertNotEqual(en, key, "en fell back to raw key \(key)")
            XCTAssertNotEqual(zh, en, "zh-Hans and en must be distinct for \(key)")

            let zhRange = NSRange(zh.startIndex..<zh.endIndex, in: zh)
            XCTAssertNotNil(
                hanRegex.firstMatch(in: zh, range: zhRange),
                "Expected Han characters in zh-Hans for \(key), got: \(zh)"
            )

            let enRange = NSRange(en.startIndex..<en.endIndex, in: en)
            XCTAssertNotNil(
                asciiLetterRegex.firstMatch(in: en, range: enRange),
                "Expected ASCII letters in en for \(key), got: \(en)"
            )
            XCTAssertNil(
                hanRegex.firstMatch(in: en, range: enRange),
                "Unexpected Han characters in en for \(key): \(en)"
            )

            for value in [zh, en] {
                for prefix in FabushiAccessibilityIDs.forbiddenRawLocalizationKeyPrefixes {
                    XCTAssertFalse(
                        value.lowercased().contains(prefix),
                        "Forbidden prefix '\(prefix)' leaked in '\(value)' for key \(key)"
                    )
                }
                for sample in forbiddenSampleTitles {
                    XCTAssertFalse(
                        value.localizedCaseInsensitiveContains(sample),
                        "Forbidden sample title '\(sample)' found in '\(value)' for key \(key)"
                    )
                }
            }
        }
    }

    // MARK: - 2. `D11-05` 6-Stage Secret & JWT Trajectory Redactor

    func testSecretRedactorSixStagesMatchRustTrajectoryRules() {
        let redactor = FabushiStudioSecretRedactor(customLiterals: ["SUPER_CUSTOM_LITERAL_99"])

        // Stage 1: Custom literal
        let customRes = redactor.redactText("value=SUPER_CUSTOM_LITERAL_99 ok")
        XCTAssertEqual(customRes.redactedText, "value=[REDACTED:CUSTOM_SECRET] ok")
        XCTAssertEqual(customRes.findingsByCategory["custom_secret"], 1)

        // Stage 2: PEM private key block
        let pem = "before -----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY----- after"
        let pemRes = redactor.redactText(pem)
        XCTAssertEqual(pemRes.redactedText, "before [REDACTED:PRIVATE_KEY] after")
        XCTAssertEqual(pemRes.findingsByCategory["private_key"], 1)

        // Stage 3: Bearer / bearer / Basic auth headers
        let authRes = redactor.redactText("Authorization: Bearer abcdef1234567890 | bearer zyxwvuts9876 | Basic dXNlcjpwYXNzMTIz")
        XCTAssertTrue(authRes.redactedText.contains("Bearer [REDACTED:BEARER_TOKEN]"))
        XCTAssertTrue(authRes.redactedText.contains("bearer [REDACTED:BEARER_TOKEN]"))
        XCTAssertTrue(authRes.redactedText.contains("Basic [REDACTED:BASIC_AUTH]"))
        XCTAssertEqual(authRes.findingsByCategory["bearer_token"], 2)
        XCTAssertEqual(authRes.findingsByCategory["basic_auth"], 1)

        // Stage 4: All 15 prefixed keys
        let prefixedSamples: [(String, String)] = [
            ("sk-ant-1234567890abcdef", "[REDACTED:ANTHROPIC_KEY]"),
            ("sk-proj-1234567890abcdef", "[REDACTED:OPENAI_KEY]"),
            ("sk-1234567890abcdef1234", "[REDACTED:API_KEY]"),
            ("github_pat_1234567890abcdef", "[REDACTED:GITHUB_TOKEN]"),
            ("ghp_1234567890abcdef", "[REDACTED:GITHUB_TOKEN]"),
            ("gho_1234567890abcdef", "[REDACTED:GITHUB_TOKEN]"),
            ("ghs_1234567890abcdef", "[REDACTED:GITHUB_TOKEN]"),
            ("ghu_1234567890abcdef", "[REDACTED:GITHUB_TOKEN]"),
            ("xoxb-1234567890abc", "[REDACTED:SLACK_TOKEN]"),
            ("xoxp-1234567890abc", "[REDACTED:SLACK_TOKEN]"),
            ("xoxa-1234567890abc", "[REDACTED:SLACK_TOKEN]"),
            ("xoxr-1234567890abc", "[REDACTED:SLACK_TOKEN]"),
            ("AIzaSyA1234567890abcdefghij", "[REDACTED:GOOGLE_API_KEY]"),
            ("AKIAIOSFODNN7EXAMPLE", "[REDACTED:AWS_ACCESS_KEY]"),
            ("ASIAIOSFODNN7EXAMPLE", "[REDACTED:AWS_ACCESS_KEY]"),
        ]
        for (sample, marker) in prefixedSamples {
            let res = redactor.redactText("key=\(sample) done")
            XCTAssertEqual(res.redactedText, "key=\(marker) done", "Failed for prefixed token \(sample)")
        }

        // Stage 5: Raw JWT (`eyJ<p1>.<p2>.<p3>`)
        let rawJwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
        let jwtRes = redactor.redactText("token \(rawJwt) end")
        XCTAssertEqual(jwtRes.redactedText, "token [REDACTED:JWT] end")
        XCTAssertEqual(jwtRes.findingsByCategory["jwt"], 1)

        // Stage 6: Key-value secrets (incl. ct0, auth_token, refresh_token, session_id, set-cookie) + benign literals
        let kvInput = """
        "auth_token": "my_secret_cookie_val", "refresh_token": "ref_99887766", "session_id": "sid_11223344"
        set-cookie: ct0=9988776655443322; Path=/
        password=true token=false secret=null pwd=none my_ct0=shouldstay1234
        """
        let kvRes = redactor.redactText(kvInput)
        XCTAssertTrue(kvRes.redactedText.contains("\"auth_token\": \"[REDACTED:SECRET]\""))
        XCTAssertTrue(kvRes.redactedText.contains("\"refresh_token\": \"[REDACTED:SECRET]\""))
        XCTAssertTrue(kvRes.redactedText.contains("\"session_id\": \"[REDACTED:SECRET]\""))
        XCTAssertTrue(kvRes.redactedText.contains("ct0=[REDACTED:SECRET]"))
        XCTAssertTrue(kvRes.redactedText.contains("password=true"))
        XCTAssertTrue(kvRes.redactedText.contains("token=false"))
        XCTAssertTrue(kvRes.redactedText.contains("secret=null"))
        XCTAssertTrue(kvRes.redactedText.contains("pwd=none"))
        XCTAssertTrue(kvRes.redactedText.contains("my_ct0=shouldstay1234"))
    }

    // MARK: - 3. `D11-04` ScriptApp Publish Preflight Validator

    func testScriptAppPreflightValidatorAcceptsSafeAndRejectsLeaksAndMetadataUrls() {
        let safeApp = FabushiStudioAutomationScriptApp(
            schemaVersion: 1,
            appId: "app_weekly_report",
            slug: "weekly-report_1",
            name: "Weekly Report",
            version: "1.0.0",
            description: "Navigates to dashboard and extracts summary text",
            targetDomains: ["example.com"],
            parameters: [
                FabushiStudioScriptParameter(name: "org_slug", required: true, defaultValue: "acme", isSecret: false),
                FabushiStudioScriptParameter(name: "auth_token", required: true, defaultValue: nil, isSecret: true),
            ],
            steps: [
                .navigate(urlTemplate: "https://{{org_slug}}.example.com/reports"),
                .waitFor(locator: FabushiStudioSemanticLocator(testId: "report-root"), timeoutMs: 5_000),
                .fill(
                    locator: FabushiStudioSemanticLocator(nameAttr: "password"),
                    valueTemplate: "",
                    secretVaultKey: "example.com:password"
                ),
                .extractText(locator: FabushiStudioSemanticLocator(ariaLabel: "Summary"), outputKey: "summary"),
            ]
        )
        let validResult = FabushiStudioScriptAppPreflightValidator.validate(safeApp)
        XCTAssertTrue(validResult.isValid, "Expected safe app to pass, got issues: \(validResult.issues)")

        // Reject raw secret leak in step or description
        let leakedApp = FabushiStudioAutomationScriptApp(
            appId: "app_leak",
            slug: "app-leak",
            name: "Leak App",
            description: "Uses sk-ant-1234567890abcdef1234 directly",
            steps: [.navigate(urlTemplate: "https://example.com")]
        )
        let leakResult = FabushiStudioScriptAppPreflightValidator.validate(leakedApp)
        XCTAssertFalse(leakResult.isValid)
        XCTAssertTrue(leakResult.issues.contains { $0.kind == .secretLeakDetected })

        // Reject cloud metadata URLs (AWS & GCP) + embedded userinfo
        for badUrl in [
            "http://169.254.169.254/latest/meta-data/",
            "http://metadata.google.internal/computeMetadata/v1/",
            "https://user:pass@example.com/login",
        ] {
            let badUrlApp = FabushiStudioAutomationScriptApp(
                appId: "app_meta",
                slug: "app-meta",
                name: "Meta",
                steps: [.navigate(urlTemplate: badUrl)]
            )
            XCTAssertFalse(
                FabushiStudioScriptAppPreflightValidator.validate(badUrlApp).isValid,
                "Expected bad URL to fail preflight: \(badUrl)"
            )
        }

        // Reject non-secret sensitive parameter name & secret parameter with defaultValue & invalid slug
        let badParamsApp = FabushiStudioAutomationScriptApp(
            appId: "app_bad_param",
            slug: "Bad Slug!",
            name: "Bad",
            parameters: [
                FabushiStudioScriptParameter(name: "api_key", defaultValue: nil, isSecret: false),
                FabushiStudioScriptParameter(name: "user_secret", defaultValue: "hardcoded", isSecret: true),
            ],
            steps: [
                .fill(locator: FabushiStudioSemanticLocator(nameAttr: "password"), valueTemplate: "plain123", secretVaultKey: nil),
            ]
        )
        let badParamsResult = FabushiStudioScriptAppPreflightValidator.validate(badParamsApp)
        XCTAssertFalse(badParamsResult.isValid)
        XCTAssertTrue(badParamsResult.issues.contains { $0.path == "slug" })
        XCTAssertTrue(badParamsResult.issues.contains { $0.path == "parameters.api_key" && $0.kind == .secretLeakDetected })
        XCTAssertTrue(badParamsResult.issues.contains { $0.path == "parameters.user_secret" && $0.kind == .secretLeakDetected })
        XCTAssertTrue(badParamsResult.issues.contains { $0.path == "steps[0]" && $0.kind == .secretLeakDetected })
    }

    // MARK: - 4. `D10-10` 3-Tier AI Provider Error Classifier & `TaskWakeupTriggered` Guard

    func testThreeTierAiErrorClassifierAndWakeupGuard() {
        // Tier 1: Unconfigured Provider -> shows CTA
        let tier1 = FabushiMagpieErrorClassifier.classify(
            rawMessage: "No configured provider available for turn",
            needsProviderCta: false
        )
        XCTAssertEqual(tier1.tier, .unconfiguredProvider)
        XCTAssertTrue(tier1.showSettingsCTA)
        XCTAssertEqual(tier1.settingsActionCallback, "hermes:open_ai_providers")

        // Tier 2: Provider Request Failed (HTTP 401/429/rate-limit/auth/transport) -> NO misleading CTA + prefix strip
        let tier2Rate = FabushiMagpieErrorClassifier.classify(
            rawMessage: "\u{001B}[31mInternal error: PROVIDER_ERROR: Provider rate limited (HTTP 429)   retry later\u{001B}[0m"
        )
        XCTAssertEqual(tier2Rate.tier, .providerRequestFailed)
        XCTAssertFalse(tier2Rate.showSettingsCTA)
        XCTAssertNil(tier2Rate.settingsActionCallback)
        XCTAssertEqual(tier2Rate.detail(locale: .en), "Provider rate limited (HTTP 429) retry later")

        // "missing api key" combined with HTTP 401 marker must stay Tier 2, NOT Tier 1!
        let tier2Auth = FabushiMagpieErrorClassifier.classify(
            rawMessage: "HTTP 401 Unauthorized: missing api key in upstream authorization header"
        )
        XCTAssertEqual(tier2Auth.tier, .providerRequestFailed)
        XCTAssertFalse(tier2Auth.showSettingsCTA)

        // Tier 3: Local turn failed + 240-char truncation
        let longRaw = String(repeating: "x", count: 300)
        let tier3 = FabushiMagpieErrorClassifier.classify(rawMessage: longRaw)
        XCTAssertEqual(tier3.tier, .localTurnFailed)
        XCTAssertFalse(tier3.showSettingsCTA)
        XCTAssertEqual(tier3.detail(locale: .en).count, 240)
        XCTAssertTrue(tier3.detail(locale: .en).hasSuffix("…"))

        // Wakeup Guard (`D10-10`)
        XCTAssertEqual(
            FabushiStudioWakeupGuard.evaluate(.init(
                isAuthenticated: false,
                currentUserId: nil,
                requestedChatId: 100,
                knownChatIds: [100],
                botChatIds: [100]
            )),
            .block(reason: .unauthenticated)
        )
        XCTAssertEqual(
            FabushiStudioWakeupGuard.evaluate(.init(
                isAuthenticated: true,
                currentUserId: 1,
                requestedChatId: 999,
                knownChatIds: [100],
                botChatIds: [100]
            )),
            .block(reason: .chatNotFound)
        )
        XCTAssertEqual(
            FabushiStudioWakeupGuard.evaluate(.init(
                isAuthenticated: true,
                currentUserId: 1,
                requestedChatId: 100,
                knownChatIds: [100],
                botChatIds: [200]
            )),
            .block(reason: .targetNotBotChat)
        )
        XCTAssertEqual(
            FabushiStudioWakeupGuard.evaluate(.init(
                isAuthenticated: true,
                currentUserId: 1,
                requestedChatId: 100,
                knownChatIds: [100],
                botChatIds: [100]
            )),
            .allow(targetChatId: 100, resumePrompt: FabushiStudioWakeupGuard.canonicalResumePrompt)
        )
    }

    // MARK: - 5. Real Embedded Rust Host + Loopback HTTP End-to-End Studio RPCs (ZERO FFI Mocks)

    func testRealHostAgentStudioStatusToolsetsTasksKanbanAndSandboxFailClosed() async throws {
        let dir = try makeTempDir("status_tasks_kanban")
        defer { try? FileManager.default.removeItem(at: dir) }

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "GET" && req.path == "/api/v1/hermes/status" {
                return .json(
                    200,
                    """
                    {"healthy":true,"engine":"hermes_server","active_model":"claude-sonnet-4","active_provider":"anthropic","api_mode":"messages","toolsets_count":29,"tools_count":88,"skills_count":3,"cron_count":1,"sessions_count":2,"pending_approvals_count":0,"heartbeat_enabled":true,"allow_local_fallback":false,"has_active_subscription":true,"has_configured_provider":true,"needs_provider_selection":false}
                    """
                )
            }
            if req.method == "GET" && req.path == "/api/v1/fabushi-bots" {
                return .json(200, "[]")
            }
            return .json(404, "{\"error\":\"not_found\"}")
        }
        defer { server.stop() }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: makeIsolatedKeychain("status"))
        let router = makeRouter()
        let hub = FabushiAgentStudioLogoutCleanupHub()
        let vm = FabushiAgentStudioViewModel(coordinator: coordinator, router: router, cleanupHub: hub)

        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )
        do {
            _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 401,
                username: "studio_tester",
                sessionId: "sess-401",
                token: "tok-401",
                firstName: "Studio"
            ))

            // 1. Overview: Local + Remote status, >= 49 commands, 29 toolsets
            await vm.loadOverview()
            XCTAssertEqual(vm.localStatus?.engine, "hermes_embedded")
            XCTAssertEqual(vm.localStatus?.toolsetsCount, 29)
            XCTAssertEqual(vm.remoteStatus?.activeModel, "claude-sonnet-4")
            XCTAssertGreaterThanOrEqual(vm.commands.count, 40)
            XCTAssertEqual(vm.toolsetSummary?.totalToolsets, 29)

            // Verify loopback server actually received authenticated GET /api/v1/hermes/status
            let statusReqs = server.requests(method: "GET", pathPrefix: "/api/v1/hermes/status")
            XCTAssertEqual(statusReqs.count, 1)
            XCTAssertEqual(statusReqs.first?.headers["authorization"], "Bearer tok-401")

            // 2. Toolsets: safe preset succeeds; all 4 desktop-only toolsets + full preset fail closed
            let safeOk = await vm.applyToolsetPreset("safe")
            XCTAssertTrue(safeOk)
            for forbidden in ["terminal", "code_execution", "desktop_ui", "browser", "full"] {
                let ok = await vm.setToolsetEnabled(forbidden, enabled: true)
                XCTAssertFalse(ok, "Expected iOS sandbox to reject enabling '\(forbidden)'")
                XCTAssertNotNil(vm.sandboxNoticeMessage)
                if forbidden != "full" {
                    let enabledState = vm.toolsetSummary?.toolsets.first(where: { $0.id == forbidden })?.enabled ?? false
                    XCTAssertFalse(enabledState, "ViewModel must NEVER fake enabled state for '\(forbidden)'")
                }
            }

            // 3. Tasks FSM: PascalCase decode + lowercase snake_case transition + terminal rejection
            let createdTask = await vm.createTask(taskId: "task_m4_e2e")
            XCTAssertEqual(createdTask?.taskId, "task_m4_e2e")
            XCTAssertEqual(createdTask?.state, .pending)

            let runningTask = await vm.transitionTask(taskId: "task_m4_e2e", to: .running)
            XCTAssertEqual(runningTask?.state, .running)

            let completedTask = await vm.transitionTask(taskId: "task_m4_e2e", to: .completed)
            XCTAssertEqual(completedTask?.state, .completed)

            // Terminal state rejects subsequent transitions
            let invalidTransition = await vm.transitionTask(taskId: "task_m4_e2e", to: .running)
            XCTAssertNil(invalidTransition)

            // 4. Kanban: createBoard -> createCard -> listCards -> moveCard -> deleteCard
            let board = await vm.createKanbanBoard(name: "M4 Sprint", description: "Real SQLite board")
            let boardId = try XCTUnwrap(board?.id)
            let card = await vm.createKanbanCard(boardId: boardId, columnName: "Backlog", title: "Ship Agent Studio")
            let cardId = try XCTUnwrap(card?.id)
            XCTAssertEqual(card?.columnName, "Backlog")

            let moved = await vm.moveKanbanCard(cardId: cardId, targetColumn: "Done")
            XCTAssertEqual(moved?.columnName, "Done")

            await vm.loadKanbanCards(boardId: boardId)
            XCTAssertEqual(vm.kanbanCards.first(where: { $0.id == cardId })?.columnName, "Done")

            let deletedCard = await vm.deleteKanbanCard(cardId: cardId)
            XCTAssertTrue(deletedCard)
            XCTAssertFalse(vm.kanbanCards.contains(where: { $0.id == cardId }))

            // 5. Logout cleanup wipes all cached ViewModel state immediately
            _ = try await coordinator.clearLocalSession()
            XCTAssertTrue(vm.tasks.isEmpty)
            XCTAssertTrue(vm.kanbanBoards.isEmpty)
            XCTAssertTrue(vm.kanbanCards.isEmpty)
            XCTAssertNil(vm.localStatus)
            XCTAssertNil(vm.remoteStatus)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    func testRealHostSkillsMemoryCronSessionsAndValueBoundApprovals() async throws {
        let dir = try makeTempDir("skills_memory_cron_sessions_approvals")
        defer { try? FileManager.default.removeItem(at: dir) }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: makeIsolatedKeychain("full"))
        let router = makeRouter()
        let hub = FabushiAgentStudioLogoutCleanupHub()
        let vm = FabushiAgentStudioViewModel(coordinator: coordinator, router: router, cleanupHub: hub)

        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )
        do {
            // 1. Skills + Distilled `script_app` (`D11-04`)
            let distilledBody = """
            # Weekly Digest Script

            ## AutomationScriptApp
            - App ID: `app_weekly_digest`
            - Slug: `weekly-digest`
            - Version: `1.0.0`
            - Target Domains: example.com
            - Deterministic Steps: 1

            ## Parameters
            - `region` (required=true, secret=false): Target region
            """
            let savedSkill = await vm.createSkill(
                name: "weekly-digest",
                description: "Distilled zero-LLM digest workflow",
                body: distilledBody,
                platforms: ["macos", "linux"],
                toolsets: ["browser"],
                tags: ["browser", "script_app", "distilled"],
                version: "1.0.0",
                origin: .learned,
                trusted: true
            )
            let skill = try XCTUnwrap(savedSkill)
            XCTAssertTrue(skill.isDistilledScriptApp)
            let extractedApp = try XCTUnwrap(skill.extractAutomationScriptApp())
            XCTAssertEqual(extractedApp.slug, "weekly-digest")
            XCTAssertTrue(vm.validateScriptAppForPublish(extractedApp).isValid)

            let patched = await vm.patchSkill(name: "weekly-digest", oldText: "Target region", newText: "Region slug")
            XCTAssertTrue(patched?.body.contains("Region slug") ?? false)

            let deletedSkill = await vm.deleteSkill(name: "weekly-digest")
            XCTAssertTrue(deletedSkill)

            // 2. Memory (MEMORY.md 2200c / USER.md 1375c), Soul, Honcho & Quarantine
            await vm.loadMemoryAndSoul()
            XCTAssertEqual(vm.memoryState?.charLimit, 2_200)
            XCTAssertEqual(vm.userMemoryState?.charLimit, 1_375)

            let addedMem = await vm.mutateMemory(target: .memory, op: "add", entry: "Prefers strict Swift 6 concurrency")
            XCTAssertTrue(addedMem?.entries.contains("Prefers strict Swift 6 concurrency") ?? false)

            let savedSoul = await vm.saveSoul(markdown: "# Fabushi iOS\n\n- Role: Native Apple HIG Agent\n- Tone: Calm")
            XCTAssertTrue(savedSoul?.rawMarkdown.contains("Native Apple HIG Agent") ?? false)

            let honchoAns = await vm.queryHoncho(question: "What are my preferences?")
            XCTAssertNotNil(honchoAns)

            // 3. Cron: valid 5-field preview + invalid rejected + save/list/delete
            let validPreview = await vm.previewCron(schedule: "0 9 * * 1-5")
            XCTAssertEqual(validPreview?.valid, true)
            XCTAssertGreaterThan(validPreview?.summary.scheduledFiresNext7Days ?? 0, 0)

            let invalidPreview = await vm.previewCron(schedule: "not a cron")
            XCTAssertNil(invalidPreview)

            let savedCron = await vm.saveCronJob(name: "Weekday Brief", schedule: "0 9 * * 1-5", prompt: "Summarize PRs")
            let cronId = try XCTUnwrap(savedCron?.id)
            await vm.loadCronJobs()
            XCTAssertTrue(vm.cronJobs.contains(where: { $0.id == cronId }))
            let cronDeleted = await vm.deleteCronJob(jobId: cronId)
            XCTAssertTrue(cronDeleted)

            // 4. Sessions: create -> list -> get (present + nil for missing) -> search
            let createdSession = await vm.createSession(title: "M4 Audit Session")
            let sessionId = try XCTUnwrap(createdSession?.id)
            await vm.loadSessions()
            XCTAssertTrue(vm.sessions.contains(where: { $0.id == sessionId }))

            let foundSession = await vm.inspectSession(sessionId: sessionId)
            XCTAssertEqual(foundSession?.id, sessionId)
            let missingSession = await vm.inspectSession(sessionId: "nonexistent_session_xyz")
            XCTAssertNil(missingSession)

            await vm.searchSessions(query: "M4")

            // 5. Approvals: local mode MUST persist `approved_once` (NOT `pending`!) + single-use value-bound token
            let enqueued = await vm.enqueueApproval(
                sessionId: sessionId,
                command: "git push origin main",
                riskLevel: "high",
                patternName: "git_push"
            )
            let approvalId = try XCTUnwrap(enqueued?.id)
            XCTAssertEqual(enqueued?.status, .pending)

            let decided = await vm.decideApproval(approvalId: approvalId, decision: .approvedOnce)
            XCTAssertEqual(decided?.status, .approvedOnce, "Must send 'approved_once' so Rust does not revert to 'pending'")

            let issuedToken = await vm.issueValueBoundApproval(action: "file_write", target: "/workspace/a.swift")
            let token = try XCTUnwrap(issuedToken)
            XCTAssertFalse(token.spent)

            let consumedRaw = await vm.consumeValueBoundApproval(
                approvalId: token.approvalId,
                action: "file_write",
                target: "/workspace/a.swift"
            )
            let consumed = try XCTUnwrap(consumedRaw)
            XCTAssertTrue(consumed.spent)

            // Second replay MUST fail closed (`approval_rejected`)
            let replay = await vm.consumeValueBoundApproval(
                approvalId: token.approvalId,
                action: "file_write",
                target: "/workspace/a.swift"
            )
            XCTAssertNil(replay)

            // 6. Deep-link alias normalization & Workspace View construction
            XCTAssertEqual(FabushiStudioTab.fromDeepLinkAlias("tasks"), .kanban)
            XCTAssertEqual(FabushiStudioTab.fromDeepLinkAlias("soul"), .memory)
            XCTAssertEqual(FabushiStudioTab.fromDeepLinkAlias("heartbeat"), .cron)
            XCTAssertEqual(FabushiStudioTab.fromDeepLinkAlias("toolsets"), .overview)
            XCTAssertEqual(FabushiStudioTab.auxiliarySheetTarget(from: "mcp"), "mcp")

            let view = FabushiAgentStudioWorkspaceView(
                initialTab: "skills",
                coordinator: coordinator,
                router: router,
                cleanupHub: hub
            )
            let hosting = UIHostingController(rootView: view.environment(FabushiTheme.default))
            XCTAssertNotNil(hosting.view)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
