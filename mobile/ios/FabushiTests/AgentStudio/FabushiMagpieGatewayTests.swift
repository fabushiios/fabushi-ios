import Foundation
import os
import XCTest
@testable import Fabushi

@MainActor
final class FabushiMagpieGatewayTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m4b_magpie_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychains(_ suffix: String) -> (FabushiKeychainStore, FabushiSecurityKeychainStore) {
        let token = UUID().uuidString
        let core = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m4b.magpie.core.\(suffix).\(token)",
            account: "m4b-magpie-core-\(suffix)"
        )
        let sec = FabushiSecurityKeychainStore(
            service: "com.ombhrum.fabushi.tests.m4b.magpie.sec.\(suffix).\(token)"
        )
        return (core, sec)
    }

    private func makeRouter() -> ShellNavigationRouter {
        ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
    }

    // MARK: - 1. Built-In Presets, Find Preset & Regional Endpoint Resolution

    func testPresetsFindAndResolveEndpoints() async throws {
        let dir = try makeTempDir("presets")
        let (coreKC, _) = makeIsolatedKeychains("presets")
        defer {
            try? coreKC.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let vm = FabushiMagpieViewModel(coordinator: coordinator, router: makeRouter())
            await vm.loadPresets()
            XCTAssertGreaterThanOrEqual(vm.presets.count, 40)
            XCTAssertTrue(vm.presets.contains(where: { $0.id == "deepseek" }))
            XCTAssertTrue(vm.presets.contains(where: { $0.id == "google-vertex" }))

            let zhipu = await vm.findPreset(id: "zhipu")
            XCTAssertEqual(zhipu?.id, "zhipu")

            let resolved = await vm.resolvePresetEndpoints(presetId: "zhipu", region: "coding")
            XCTAssertEqual(resolved?.presetId, "zhipu")
            XCTAssertTrue(resolved?.baseUrl.contains("open.bigmodel.cn") == true)

            await vm.probeProviderHealth(presetId: "zhipu", region: "coding")
            XCTAssertTrue(vm.probeStatusText.contains("open.bigmodel.cn"))
            XCTAssertTrue(vm.probeStatusText.contains("M6"))

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 2. Dual-Shape `magpie.state`, `D10-10` `google_vertex` Match & Single-Tap Instant Switch

    func testDualShapeStateGoogleVertexCanonicalMatchAndSingleTapInstantSwitch() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "GET", req.path == "/api/v1/hermes/magpie/state" {
                return .json(
                    200,
                    """
                    {
                      "presets": [],
                      "providers": [
                        {
                          "provider_id": "magpie_provider:google-vertex",
                          "name": "Google Vertex AI",
                          "api_mode": "gemini_generate_content",
                          "base_url": "https://us-central1-aiplatform.googleapis.com",
                          "default_model": "gemini-2.5-pro",
                          "models": ["gemini-2.5-pro", "gemini-2.5-flash"],
                          "enabled": true,
                          "has_api_key": true,
                          "is_active": true,
                          "preset_id": "google-vertex",
                          "is_subscription": true
                        }
                      ],
                      "active_provider": "magpie_provider:google-vertex",
                      "active_model": "gemini-2.5-pro",
                      "allow_local_fallback": false,
                      "has_active_subscription": true,
                      "has_configured_provider": true,
                      "needs_provider_selection": false,
                      "subscriptions": [
                        {
                          "id": "sub_vertex_primary",
                          "kind": "google_vertex",
                          "label": "Vertex Enterprise",
                          "email": "dev@example.com",
                          "active": false,
                          "protocol": "gemini_generate_content",
                          "default_model": "gemini-2.5-pro",
                          "models": ["gemini-2.5-pro", "gemini-2.5-flash"]
                        },
                        {
                          "id": "sub_claude_1",
                          "kind": "claude_code",
                          "label": "Claude Pro",
                          "active": false,
                          "protocol": "anthropic_messages",
                          "default_model": "claude-sonnet-4-5",
                          "models": ["claude-sonnet-4-5", "claude-opus-4-1"]
                        }
                      ],
                      "routing_groups": [],
                      "routing_strategy": "smart",
                      "masking": {
                        "enabled": true,
                        "mask_secrets": true,
                        "mask_personal": true,
                        "custom_words": [],
                        "allow_list": [],
                        "default_action": "mask",
                        "kind_actions": {}
                      },
                      "recent_route_traces": []
                    }
                    """
                )
            }
            return .json(404, #"{"error":"not_found"}"#)
        }
        defer { server.stop() }

        let dir = try makeTempDir("dual_state")
        let (coreKC, _) = makeIsolatedKeychains("dual_state")
        defer {
            try? coreKC.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let router = makeRouter()
            let vm = FabushiMagpieViewModel(coordinator: coordinator, router: router)

            // Unauthenticated -> local shape
            await vm.loadState(preferRemote: false)
            XCTAssertTrue(vm.providers.isEmpty)

            // Authenticated -> remote MagpieStateDto shape
            _ = try await coordinator.setAuthSession(
                FabushiSessionAuthParams(
                    userId: 401,
                    username: "magpie_user",
                    sessionId: "sess-401",
                    token: "tok-401",
                    firstName: "Magpie"
                )
            )
            await vm.loadState(preferRemote: true)
            XCTAssertEqual(vm.activeProvider, "magpie_provider:google-vertex")
            XCTAssertEqual(vm.activeModel, "gemini-2.5-pro")
            XCTAssertEqual(vm.subscriptions.count, 2)

            // Exact D10-10 canonical match (`google_vertex` <-> `magpie_provider:google-vertex`)
            let vertexSub = try XCTUnwrap(vm.subscriptions.first(where: { $0.id == "sub_vertex_primary" }))
            XCTAssertTrue(vertexSub.isActiveFor(activeProvider: vm.activeProvider))
            XCTAssertEqual(vm.selectedSubscriptionModels["sub_vertex_primary"], "gemini-2.5-pro")

            // Single-tap instant model switch on ALREADY-ACTIVE subscription
            let switchedImmediately = vm.selectSubscriptionModel(
                subscriptionId: "sub_vertex_primary",
                model: "gemini-2.5-flash"
            )
            XCTAssertTrue(switchedImmediately)
            XCTAssertEqual(vm.activeModel, "gemini-2.5-flash")
            XCTAssertEqual(vm.selectedSubscriptionModels["sub_vertex_primary"], "gemini-2.5-flash")

            // Tapping model chip on INACTIVE subscription only updates pending chip without switching activeModel
            let inactiveSwitched = vm.selectSubscriptionModel(
                subscriptionId: "sub_claude_1",
                model: "claude-opus-4-1"
            )
            XCTAssertFalse(inactiveSwitched)
            XCTAssertEqual(vm.selectedSubscriptionModels["sub_claude_1"], "claude-opus-4-1")
            XCTAssertEqual(vm.activeModel, "gemini-2.5-flash")

            // Explicit activation switches to Claude subscription with its selected chip model
            vm.activateSubscription(subscriptionId: "sub_claude_1")
            XCTAssertEqual(vm.activeProvider, "sub_claude_1")
            XCTAssertEqual(vm.activeModel, "claude-opus-4-1")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 3. `D10-09` Bot-Model Placeholder Sentinels & Mismatch Detector

    func testD1009BotModelPlaceholderSentinelsAndMismatchDetection() {
        for sentinel in ["", "   ", "default", "DEFAULT", "默认", "默认模型", "auto", "hermes-local-v1", "local_hermes/hermes-local-v1"] {
            XCTAssertEqual(
                FabushiMagpieBotModelPolicy.normalizePreferredModel(sentinel),
                "",
                "Sentinel '\(sentinel)' must normalize to Follow Settings"
            )
            XCTAssertTrue(FabushiMagpieBotModelPolicy.isFollowingSettings(sentinel))
            XCTAssertFalse(
                FabushiMagpieBotModelPolicy.detectModelMismatch(
                    preferredModel: sentinel,
                    availableModels: ["deepseek-chat"],
                    routingGroupIds: ["mix"]
                )
            )
        }

        XCTAssertEqual(
            FabushiMagpieBotModelPolicy.normalizePreferredModel("deepseek-chat"),
            "deepseek-chat"
        )
        XCTAssertFalse(
            FabushiMagpieBotModelPolicy.detectModelMismatch(
                preferredModel: "group/mix",
                availableModels: ["deepseek-chat"],
                routingGroupIds: ["mix"]
            )
        )
        XCTAssertTrue(
            FabushiMagpieBotModelPolicy.detectModelMismatch(
                preferredModel: "legacy-removed-model",
                availableModels: ["deepseek-chat", "gemini-2.5-pro"],
                routingGroupIds: ["mix"]
            )
        )
    }

    // MARK: - 4. Smart Routing Groups, Circuit Breaker, 10-Field Usage, PII Masking & Imports

    func testRoutingGroupsCircuitBreakerUsagePiiMaskingAndImports() async throws {
        let dir = try makeTempDir("routing_masking")
        let (coreKC, _) = makeIsolatedKeychains("routing_masking")
        defer {
            try? coreKC.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let vm = FabushiMagpieViewModel(coordinator: coordinator, router: makeRouter())

            // Pure-Swift Circuit Breaker Ladder & `forbiddenAlike` verification
            XCTAssertEqual(FabushiMagpieCircuitBreakerPolicy.creditRestLadderMs, [
                1_800_000, 3_600_000, 7_200_000, 14_400_000, 28_800_000, 86_400_000, 604_800_000,
            ])
            XCTAssertEqual(FabushiMagpieCircuitBreakerPolicy.creditRestDurationMs(consecutiveFailures: 0), 1_800_000)
            XCTAssertEqual(FabushiMagpieCircuitBreakerPolicy.creditRestDurationMs(consecutiveFailures: 6), 604_800_000)
            XCTAssertEqual(FabushiMagpieCircuitBreakerPolicy.overloadRestMs, 30_000)
            XCTAssertTrue(FabushiMagpieCircuitBreakerPolicy.isForbiddenAlike(statusCode: 403, errorText: "content_filter safety violation"))
            XCTAssertNil(FabushiMagpieCircuitBreakerPolicy.classifyRestKind(statusCode: 403, errorText: "content_filter safety violation"))
            XCTAssertEqual(FabushiMagpieCircuitBreakerPolicy.classifyRestKind(statusCode: 402, errorText: "insufficient_quota"), .creditRest)
            XCTAssertEqual(FabushiMagpieCircuitBreakerPolicy.classifyRestKind(statusCode: 529, errorText: "overloaded"), .overloadRest)
            XCTAssertEqual(FabushiMagpieCircuitBreakerPolicy.classifyRestKind(statusCode: 429, errorText: "rate_limit"), .rateRest)

            // Routing Group CRUD + `resolveRoute` + `recordFailure` circuit-breaker bypass
            let group = FabushiMagpieRoutingGroupDTO(
                id: "group/mix",
                name: "Primary Mix",
                strategy: .order,
                members: [
                    FabushiMagpieRoutingGroupMemberDTO(target: "openai/gpt-4o", weight: 1, enabled: true),
                    FabushiMagpieRoutingGroupMemberDTO(target: "deepseek/deepseek-chat", weight: 1, enabled: true),
                ]
            )
            XCTAssertEqual(group.canonicalId, "mix")
            XCTAssertEqual(group.qualifiedModelName, "group/mix")

            let saved = await vm.upsertRoutingGroup(group)
            XCTAssertEqual(saved?.canonicalId, "mix")
            await vm.loadRoutingGroups()
            XCTAssertTrue(vm.routingGroups.contains(where: { $0.canonicalId == "mix" }))

            let nowMs: Int64 = 1_750_000_000_000
            let beforeFailure = await vm.resolveRoute(target: "group/mix", nowMs: nowMs)
            XCTAssertEqual(beforeFailure.first?.providerId, "openai")
            XCTAssertEqual(beforeFailure.first?.model, "gpt-4o")

            let recordedFail = await vm.recordFailure(
                target: "openai/gpt-4o",
                statusCode: 429,
                retryAfterSecs: 60,
                errorText: "rate_limit",
                nowMs: nowMs
            )
            XCTAssertTrue(recordedFail)
            XCTAssertEqual(vm.circuitBreakerState, .open)

            let afterFailure = await vm.resolveRoute(target: "group/mix", nowMs: nowMs + 1_000)
            XCTAssertEqual(afterFailure.first?.providerId, "deepseek")
            XCTAssertEqual(afterFailure.first?.model, "deepseek-chat")

            // 10-field `recordUsage` -> `usageSummary`
            let recordedUsage = await vm.recordUsage(
                FabushiMagpieUsageRecordRequest(
                    id: "rec_1",
                    sessionId: "sess_m4",
                    providerId: "deepseek",
                    model: "deepseek-chat",
                    inputTokens: 1_000,
                    outputTokens: 500,
                    cacheReadTokens: 100,
                    reasoningTokens: 0,
                    costUsd: 0.0042,
                    ttftMs: 120,
                    durationMs: 950,
                    tps: 52.6,
                    timestampMs: nowMs
                )
            )
            XCTAssertTrue(recordedUsage)
            XCTAssertEqual(vm.usageSummary.totalRecords, 1)
            XCTAssertEqual(vm.usageSummary.totalTokens, 1_500)
            XCTAssertGreaterThan(vm.usageSummary.totalCostUsd, 0.0)

            // PII Privacy Masking: update -> maskText -> unmaskText -> preview (even when disabled!)
            let enabledCfg = FabushiMagpieMaskingConfigDTO(
                enabled: true,
                maskSecrets: true,
                maskPersonal: true,
                customWords: ["ProjectApollo"],
                allowList: [],
                defaultAction: .mask,
                kindActions: [:]
            )
            let updatedCfg = await vm.updateMaskingConfig(enabledCfg)
            XCTAssertEqual(updatedCfg?.enabled, true)

            let sampleSensitive = "Contact alice@corp.io about ProjectApollo using sk-proj-1234567890abcdefghij"
            let masked = try XCTUnwrap(await vm.maskText(sampleSensitive))
            XCTAssertFalse(masked.maskedText.contains("alice@corp.io"))
            XCTAssertFalse(masked.maskedText.contains("ProjectApollo"))
            XCTAssertFalse(masked.maskedText.contains("sk-proj-1234567890abcdefghij"))
            XCTAssertGreaterThanOrEqual(masked.hits.count, 3)

            let restored = await vm.unmaskText(masked.maskedText)
            XCTAssertEqual(restored, sampleSensitive)

            var disabledCfg = enabledCfg
            disabledCfg.enabled = false
            _ = await vm.updateMaskingConfig(disabledCfg)
            let preview = try XCTUnwrap(await vm.previewMasking(text: sampleSensitive))
            XCTAssertFalse(preview.maskedText.contains("alice@corp.io"))
            XCTAssertFalse(preview.hits.isEmpty)

            // Deep-link & `.env` import
            let importedLink = await vm.importFromLink("magpie://import?preset=deepseek&key=sk-test-1234567890abcdef")
            XCTAssertEqual(importedLink?.presetId, "deepseek")
            XCTAssertTrue(vm.providers.contains(where: { $0.presetId == "deepseek" }))

            let dotenvSpecs = await vm.importFromDotenv("OPENAI_API_KEY=sk-proj-1234567890abcdefghij\nANTHROPIC_API_KEY=sk-ant-1234567890abcdefghij\n")
            XCTAssertGreaterThanOrEqual(dotenvSpecs.count, 2)

            let deleted = await vm.deleteRoutingGroup(id: "mix")
            XCTAssertTrue(deleted)
            XCTAssertFalse(vm.routingGroups.contains(where: { $0.canonicalId == "mix" }))

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 5. `IOS-MAIN-INV-04` Fail-Closed Scan, Vault `ai_oauth` Sync, 3-Tier Errors & Logout Reset

    func testIosSandboxNoDotfileScanVaultAiOAuthSyncThreeTierErrorAndLogoutReset() async throws {
        let serverItems = OSAllocatedUnfairLock<[String: FabushiEncryptedVaultBlobDTO]>(initialState: [:])
        let storedEnvelopeJSON = OSAllocatedUnfairLock<String?>(initialState: nil)

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "POST", req.path == "/api/v1/auth/login" {
                return .json(200, #"{"user_id":7701,"username":"magpie_vault","session_id":"s-7701","token":"tok-7701","first_name":"Vault"}"#)
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/envelope" {
                if let env = storedEnvelopeJSON.withLock({ $0 }) {
                    return .json(200, "{\"envelope\":\(env)}")
                }
                return .json(200, #"{"envelope":null}"#)
            }
            if req.method == "PUT", req.path == "/api/v1/user-vault/envelope" {
                storedEnvelopeJSON.withLock { $0 = req.bodyString }
                return .json(200, "{\"envelope\":\(req.bodyString)}")
            }
            if req.method == "GET", req.path == "/api/v1/user-vault/items" {
                let items = serverItems.withLock { Array($0.values) }
                let resp = FabushiVaultSyncPullResponse(latestRevision: Int64(items.count), items: items)
                let data = (try? JSONEncoder().encode(resp)) ?? Data("{\"latest_revision\":0,\"items\":[]}".utf8)
                return .json(200, String(decoding: data, as: UTF8.self))
            }
            if req.method == "PUT", req.path.hasPrefix("/api/v1/user-vault/items/") {
                let tail = String(req.path.dropFirst("/api/v1/user-vault/items/".count))
                let parts = tail.split(separator: "/", maxSplits: 1).map(String.init)
                let category = (parts.first ?? "ai_oauth").removingPercentEncoding ?? "ai_oauth"
                let rawKey = parts.count > 1 ? parts[1] : "item"
                let itemKey = rawKey.removingPercentEncoding ?? rawKey
                let json = (try? JSONSerialization.jsonObject(with: req.body) as? [String: Any]) ?? [:]
                let expectedRev = (json["expected_revision"] as? Int64) ?? Int64((json["expected_revision"] as? Int) ?? 0)
                let nextRev = expectedRev + 1
                let nonce = (json["nonce_b64"] as? String) ?? ""
                let cipher = (json["ciphertext_b64"] as? String) ?? ""
                let blob = FabushiEncryptedVaultBlobDTO(
                    category: category,
                    itemKey: itemKey,
                    revision: nextRev,
                    nonceB64: nonce,
                    ciphertextB64: cipher,
                    deleted: false,
                    updatedAtMs: 1_750_000_000_000
                )
                serverItems.withLock { $0["\(category):\(itemKey)"] = blob }
                return .json(200, #"{"ok":true,"category":"\#(category)","item_key":"\#(itemKey)","revision":\#(nextRev),"updated_at_ms":1750000000000}"#)
            }
            return .json(404, #"{"error":"not_found"}"#)
        }
        defer { server.stop() }

        let dir = try makeTempDir("vault_oauth")
        let (coreKC, secKC) = makeIsolatedKeychains("vault_oauth")
        defer {
            try? coreKC.deletePassphrase()
            try? coreKC.deleteAccountKey(accountId: "7701")
            secKC.purgeAccountSecrets(accountId: "7701")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: coreKC)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let router = makeRouter()
            let hub = FabushiAgentStudioLogoutCleanupHub()
            let vm = FabushiMagpieViewModel(coordinator: coordinator, router: router, cleanupHub: hub)

            // 1. Strict IOS-MAIN-INV-04 fail-closed local dotfile scan
            let blockedLocally = await vm.probeLocalDotfileScanBoundary()
            XCTAssertTrue(blockedLocally)
            XCTAssertNotNil(vm.lastSandboxNotice)

            // 2. Authenticate & unlock Zero-Knowledge Split-Key V2 Vault
            let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
                method: FabushiRPCNamespace.auth.method("login"),
                params: FabushiLegacyLoginRequest(username: "magpie_vault", password: "Pass!1"),
                extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
            )
            let vaultVM = FabushiCredentialVaultViewModel(coordinator: coordinator, securityKeychain: secKC)
            await vaultVM.unlockVault(passphraseOrPassword: "MagpieVaultPass!9")
            XCTAssertTrue(vaultVM.isUnlocked)

            // 3. Save BYOK/OAuth provider -> seals & pushes to `category: "ai_oauth"` -> pull & decrypt via `syncVaultAiOAuth`
            let savedOk = await vm.saveProviderSpec(
                providerId: "google-vertex",
                name: "Google Vertex AI",
                protocolKind: .geminiGenerateContent,
                baseUrl: "https://us-central1-aiplatform.googleapis.com",
                defaultModel: "gemini-2.5-pro",
                apiKeyOrToken: "ya29.super_secret_oauth_token_123456",
                presetId: "google-vertex",
                subscriptionKind: FabushiMagpieSubscriptionKind.googleVertex.rawValue
            )
            XCTAssertTrue(savedOk)
            XCTAssertEqual(server.requests(method: "PUT", pathPrefix: "/api/v1/user-vault/items/ai_oauth/").count, 1)

            vm.vaultOAuthRecords = []
            vm.subscriptions = []
            let syncedCount = await vm.syncVaultAiOAuth()
            XCTAssertEqual(syncedCount, 1)
            XCTAssertEqual(vm.vaultOAuthRecords.first?.providerId, "google-vertex")
            XCTAssertEqual(vm.vaultOAuthRecords.first?.subscriptionKind, "google_vertex")
            XCTAssertFalse(vm.vaultOAuthRecords.first?.maskedTokenPreview.contains("super_secret") ?? true)
            XCTAssertTrue(vm.subscriptions.contains(where: { $0.kind == "google_vertex" }))

            // 4. D10-10 3-tier error routing: ONLY Tier 1 triggers Shell ProviderErrorCard CTA!
            vm.reportProviderError(rawMessage: "needs_provider_selection: no configured provider")
            XCTAssertEqual(vm.lastClassifiedError?.tier, .unconfiguredProvider)
            XCTAssertNotNil(router.activeProviderErrorMessage)

            vm.reportProviderError(rawMessage: "PROVIDER_ERROR: HTTP 429 provider rate limited")
            XCTAssertEqual(vm.lastClassifiedError?.tier, .providerRequestFailed)
            XCTAssertNil(router.activeProviderErrorMessage)

            vm.reportProviderError(rawMessage: "Local tool execution timed out unexpectedly")
            XCTAssertEqual(vm.lastClassifiedError?.tier, .localTurnFailed)
            XCTAssertNil(router.activeProviderErrorMessage)

            // 5. Logout cleanup wipes ALL sensitive Magpie state
            vm.importLinkDraft = "magpie://import?preset=openai&key=sk-secret"
            vm.maskingPreviewInput = "secret input"
            vm.probeStatusText = "verified"
            hub.resetForDepartingAccount("7701")

            XCTAssertTrue(vm.providers.isEmpty)
            XCTAssertTrue(vm.subscriptions.isEmpty)
            XCTAssertTrue(vm.selectedSubscriptionModels.isEmpty)
            XCTAssertTrue(vm.vaultOAuthRecords.isEmpty)
            XCTAssertEqual(vm.activeProvider, "")
            XCTAssertEqual(vm.activeModel, "")
            XCTAssertEqual(vm.importLinkDraft, "")
            XCTAssertEqual(vm.maskingPreviewInput, "")
            XCTAssertEqual(vm.probeStatusText, "")
            XCTAssertNil(vm.lastClassifiedError)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
