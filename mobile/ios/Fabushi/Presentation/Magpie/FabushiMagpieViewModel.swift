import Foundation
import Observation

@MainActor
@Observable
public final class FabushiMagpieViewModel {
    @ObservationIgnored public let coordinator: FabushiCoordinator
    @ObservationIgnored public let router: ShellNavigationRouter
    @ObservationIgnored public let cleanupHub: FabushiAgentStudioLogoutCleanupHub
    @ObservationIgnored public let oauthController: FabushiMagpieOAuthController
    @ObservationIgnored private let taskBag = FabushiCancellableTaskBag()
    @ObservationIgnored private let cleanupRegistrationId: String

    // Scope fencing
    public private(set) var lastObservedGeneration: UInt64
    public private(set) var lastObservedAccountId: String?

    // Core gateway state
    public var isLoading: Bool = false
    public var presets: [FabushiMagpiePresetDTO] = []
    public var providers: [FabushiMagpieProviderDTO] = []
    public var subscriptions: [FabushiMagpieSubscriptionDTO] = []
    public var selectedSubscriptionModels: [String: String] = [:]
    public var activeProvider: String = ""
    public var activeModel: String = ""
    public var needsProviderSelection: Bool = false
    public var recentRouteTraces: [FabushiMagpieRouteTraceDTO] = []

    // Routing & circuit breaker
    public var smartRoutingEnabled: Bool = true
    public var routingGroups: [FabushiMagpieRoutingGroupDTO] = []
    public var resolvedRoutes: [FabushiMagpieResolvedRouteDTO] = []
    public var circuitBreakerState: FabushiMagpieCircuitBreakerState = .closed
    public var lastCircuitRestKind: FabushiMagpieCircuitRestKind?
    public var lastCircuitFailureTarget: String?

    // PII Privacy Masking
    public var maskingConfig: FabushiMagpieMaskingConfigDTO = FabushiMagpieMaskingConfigDTO()
    public var maskingPreviewInput: String = ""
    public var maskingPreviewResult: FabushiMagpieMaskResultDTO?

    // Import & Usage
    public var importLinkDraft: String = ""
    public var importDotenvDraft: String = ""
    public var importedSpecs: [FabushiMagpieImportedProviderSpecDTO] = []
    public var usageSummary: FabushiMagpieUsageSummaryDTO = .empty

    // Zero-Knowledge Vault (`ai_oauth`) & Browser OAuth
    public var vaultUnlocked: Bool = false
    public var vaultOAuthRecords: [FabushiMagpieVaultOAuthRecord] = []
    public var oauthStatusMessage: String?
    public var lastOAuthCallback: FabushiMagpieOAuthCallbackPayload?

    // Health probe & honest disclosures
    public var isProbing: Bool = false
    public var probeStatusText: String = ""
    public var lastResolvedEndpoint: FabushiMagpieResolvedEndpointDTO?
    public var lastSandboxNotice: String?
    public var lastClassifiedError: FabushiMagpieClassifiedError?

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub(),
        oauthController: FabushiMagpieOAuthController? = nil
    ) {
        self.coordinator = coordinator
        self.router = router
        self.cleanupHub = cleanupHub
        self.oauthController = oauthController ?? FabushiMagpieOAuthController()
        self.lastObservedGeneration = coordinator.accountScopeGeneration
        self.lastObservedAccountId = coordinator.activeAccountId
        self.vaultUnlocked = coordinator.vaultStore.isUnlocked
        self.cleanupRegistrationId = "magpie.\(UUID().uuidString)"

        cleanupHub.registerResetHandler(id: cleanupRegistrationId) { [weak self] reason in
            self?.resetOnLogout(reason: reason)
        }
    }

    deinit {
        taskBag.cancelAll()
    }

    // MARK: - Account Scope & `D10-10` `reset_on_logout()`

    public func syncAccountScopeIfNeeded() {
        let currentGen = coordinator.accountScopeGeneration
        let currentAcct = coordinator.activeAccountId
        if currentGen != lastObservedGeneration || currentAcct != lastObservedAccountId {
            lastObservedGeneration = currentGen
            lastObservedAccountId = currentAcct
            resetOnLogout(reason: .generationAdvance(currentGen))
        }
    }

    public func resetOnLogout(reason: FabushiAgentStudioLogoutCleanupHub.ResetReason) {
        taskBag.cancelAll()
        oauthController.cancel()

        isLoading = false
        providers = []
        subscriptions = []
        selectedSubscriptionModels = [:]
        activeProvider = ""
        activeModel = ""
        needsProviderSelection = false
        recentRouteTraces = []

        routingGroups = []
        resolvedRoutes = []
        circuitBreakerState = .closed
        lastCircuitRestKind = nil
        lastCircuitFailureTarget = nil

        maskingConfig = FabushiMagpieMaskingConfigDTO()
        maskingPreviewInput = ""
        maskingPreviewResult = nil

        importLinkDraft = ""
        importDotenvDraft = ""
        importedSpecs = []
        usageSummary = .empty

        vaultUnlocked = false
        vaultOAuthRecords = []
        oauthStatusMessage = nil
        lastOAuthCallback = nil

        isProbing = false
        probeStatusText = ""
        lastResolvedEndpoint = nil
        lastSandboxNotice = nil
        lastClassifiedError = nil
        router.dismissProviderErrorCard()

        lastObservedGeneration = coordinator.accountScopeGeneration
        lastObservedAccountId = coordinator.activeAccountId
    }

    // MARK: - Initial Load & Refresh

    public func loadAll() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoading = true
        defer { isLoading = false }

        await loadPresets()
        await loadState()
        await loadRoutingGroups()
        await loadUsageSummary()
        await refreshVaultStatus()
    }

    public func loadPresets() async {
        guard coordinator.isStarted else { return }
        do {
            let response: FabushiMagpieListPresetsResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("listPresets"),
                params: FabushiEmptyParams()
            )
            presets = response.presets
        } catch {
            handleError(error)
        }
    }

    public func findPreset(id: String) async -> FabushiMagpiePresetDTO? {
        guard coordinator.isStarted else { return nil }
        do {
            return try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("findPreset"),
                params: FabushiMagpieFindPresetRequest(id: id)
            )
        } catch {
            handleError(error)
            return nil
        }
    }

    public func resolvePresetEndpoints(
        presetId: String,
        region: String? = nil,
        protocolKind: FabushiMagpieProtocolKind? = nil
    ) async -> FabushiMagpieResolvedEndpointDTO? {
        guard coordinator.isStarted else { return nil }
        do {
            let resolved: FabushiMagpieResolvedEndpointDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("resolvePresetEndpoints"),
                params: FabushiMagpieResolvePresetEndpointsRequest(
                    presetId: presetId,
                    region: region,
                    protocolKind: protocolKind
                )
            )
            lastResolvedEndpoint = resolved
            return resolved
        } catch {
            handleError(error)
            return nil
        }
    }

    /// Queries remote `magpie.state` when authenticated (falling back gracefully to local `magpie.state`
    /// if offline or unauthenticated) and synchronizes `D10-10` active subscription model selection.
    public func loadState(preferRemote: Bool = true) async {
        guard coordinator.isStarted else { return }
        let isAuthenticated: Bool
        if case .authenticated = coordinator.authState {
            isAuthenticated = true
        } else {
            isAuthenticated = false
        }

        do {
            let state: FabushiMagpieStateResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("state"),
                params: FabushiMagpieStateRequest(remote: preferRemote && isAuthenticated)
            )
            applyStateResponse(state)
        } catch {
            if preferRemote && isAuthenticated {
                // Fall back to embedded local state without fabricating remote providers
                do {
                    let localState: FabushiMagpieStateResponse = try await coordinator.dispatchFenced(
                        method: FabushiRPCNamespace.magpie.method("state"),
                        params: FabushiMagpieStateRequest(remote: false)
                    )
                    applyStateResponse(localState)
                } catch {
                    handleError(error)
                }
            } else {
                handleError(error)
            }
        }
    }

    private func applyStateResponse(_ state: FabushiMagpieStateResponse) {
        if state.isRemotePayload {
            providers = state.providers
            activeProvider = state.activeProvider
            activeModel = state.activeModel
            needsProviderSelection = state.needsProviderSelection
            subscriptions = state.subscriptions
            recentRouteTraces = state.recentRouteTraces
            if let remoteMasking = state.masking {
                maskingConfig = remoteMasking
            }
            if !state.remoteRoutingGroups.isEmpty {
                routingGroups = state.remoteRoutingGroups
            }
            syncSelectedSubscriptionModels(
                subscriptions: state.subscriptions,
                activeProvider: state.activeProvider,
                activeModel: state.activeModel
            )
        } else {
            if !state.localGroups.isEmpty {
                routingGroups = state.localGroups
            }
            if let usage = state.localUsage {
                usageSummary = usage
            }
        }
    }

    /// Exact `D10-10` active subscription synchronization including `google_vertex` <-> `magpie_provider:google-vertex`.
    public func syncSelectedSubscriptionModels(
        subscriptions subs: [FabushiMagpieSubscriptionDTO],
        activeProvider provider: String,
        activeModel model: String
    ) {
        let trimmedModel = model.trimmingCharacters(in: .whitespacesAndNewlines)
        for sub in subs {
            let isActive = sub.isActiveFor(activeProvider: provider)
            if isActive && !trimmedModel.isEmpty {
                selectedSubscriptionModels[sub.id] = trimmedModel
            } else if selectedSubscriptionModels[sub.id] == nil,
                      let def = sub.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines),
                      !def.isEmpty
            {
                selectedSubscriptionModels[sub.id] = def
            }
        }
    }

    // MARK: - `D10-10` Single-Tap Instant Active-Subscription Model Switch

    /// Tapping a model chip on a subscription always records the pending model selection.
    /// When the subscription is ALREADY active (`sub.active == true` or canonical `google_vertex` match)
    /// and the tapped model differs from `activeModel`, immediately switches the active model in one tap!
    @discardableResult
    public func selectSubscriptionModel(subscriptionId: String, model: String) -> Bool {
        let trimmedModel = model.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedModel.isEmpty else { return false }

        selectedSubscriptionModels[subscriptionId] = trimmedModel

        guard let sub = subscriptions.first(where: { $0.id == subscriptionId }) else {
            return false
        }

        let isAlreadyActive = sub.isActiveFor(activeProvider: activeProvider)
        if isAlreadyActive && activeModel != trimmedModel {
            activateSubscription(subscriptionId: subscriptionId, explicitModel: trimmedModel)
            return true
        }
        return false
    }

    public func activateSubscription(subscriptionId: String, explicitModel: String? = nil) {
        guard let idx = subscriptions.firstIndex(where: { $0.id == subscriptionId }) else {
            return
        }
        let target = subscriptions[idx]
        let chosenModel = (explicitModel ?? selectedSubscriptionModels[subscriptionId] ?? target.defaultModel ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let canonicalProviderId = FabushiMagpieSubscriptionCanonicalizer.canonicalActiveProviderId(
            subscriptionId: target.id,
            kind: target.kind
        )

        activeProvider = canonicalProviderId
        if !chosenModel.isEmpty {
            activeModel = chosenModel
            selectedSubscriptionModels[subscriptionId] = chosenModel
        }
        needsProviderSelection = false
        router.dismissProviderErrorCard()
        lastClassifiedError = nil

        subscriptions = subscriptions.map { sub in
            let nowActive = sub.id == subscriptionId
            return FabushiMagpieSubscriptionDTO(
                id: sub.id,
                kind: sub.kind,
                label: sub.label,
                email: sub.email,
                accountId: sub.accountId,
                plan: sub.plan,
                active: nowActive,
                baseUrl: sub.baseUrl,
                protocolName: sub.protocolName,
                defaultModel: nowActive && !chosenModel.isEmpty ? chosenModel : sub.defaultModel,
                models: sub.models,
                sourcePath: sub.sourcePath,
                expiresAtMs: sub.expiresAtMs,
                headers: sub.headers
            )
        }

        let toastText = FabushiAgentStudioStrings.text("m4.magpie.instantSwitchToast", locale: .en)
        router.toastController.showNonChatToast(toastText, isError: false)
    }

    // MARK: - Smart Routing Groups & Circuit Breaker

    public func loadRoutingGroups() async {
        guard coordinator.isStarted else { return }
        do {
            let groups: [FabushiMagpieRoutingGroupDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("listGroups"),
                params: FabushiEmptyParams()
            )
            routingGroups = groups
        } catch {
            handleError(error)
        }
    }

    @discardableResult
    public func upsertRoutingGroup(_ group: FabushiMagpieRoutingGroupDTO) async -> FabushiMagpieRoutingGroupDTO? {
        guard coordinator.isStarted else { return nil }
        let cleanId = group.canonicalId
        guard !cleanId.isEmpty else { return nil }

        let normalized = FabushiMagpieRoutingGroupDTO(
            id: cleanId,
            name: group.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? cleanId : group.name,
            strategy: group.strategy,
            members: group.members,
            intentFastModel: group.intentFastModel,
            intentDeepModel: group.intentDeepModel,
            stickyTtlMs: group.stickyTtlMs
        )

        do {
            let saved: FabushiMagpieRoutingGroupDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("upsertGroup"),
                params: normalized
            )
            if let idx = routingGroups.firstIndex(where: { $0.canonicalId == saved.canonicalId }) {
                routingGroups[idx] = saved
            } else {
                routingGroups.append(saved)
            }
            return saved
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func deleteRoutingGroup(id: String) async -> Bool {
        guard coordinator.isStarted else { return false }
        let cleanId = id.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "group/", with: "")
        do {
            let result: FabushiMagpieDeleteGroupResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("deleteGroup"),
                params: FabushiMagpieDeleteGroupRequest(id: cleanId)
            )
            if result.deleted {
                routingGroups.removeAll { $0.canonicalId == cleanId }
            }
            return result.deleted
        } catch {
            handleError(error)
            return false
        }
    }

    @discardableResult
    public func resolveRoute(
        target: String,
        defaultProvider: String = "deepseek",
        sessionKey: String? = nil,
        prompt: String? = nil,
        nowMs: Int64? = nil
    ) async -> [FabushiMagpieResolvedRouteDTO] {
        guard coordinator.isStarted else { return [] }
        let trimmed = target.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return [] }
        do {
            let routes: [FabushiMagpieResolvedRouteDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("resolveRoute"),
                params: FabushiMagpieResolveRouteRequest(
                    target: trimmed,
                    defaultProvider: defaultProvider,
                    sessionKey: sessionKey,
                    prompt: prompt,
                    nowMs: nowMs
                )
            )
            resolvedRoutes = routes
            if circuitBreakerState == .open && !routes.isEmpty {
                circuitBreakerState = .halfOpen
            }
            return routes
        } catch {
            handleError(error)
            return []
        }
    }

    @discardableResult
    public func recordFailure(
        target: String,
        statusCode: UInt16 = 429,
        retryAfterSecs: UInt64? = 60,
        errorText: String = "",
        nowMs: Int64? = nil
    ) async -> Bool {
        guard coordinator.isStarted else { return false }
        let trimmed = target.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }

        do {
            let response: FabushiMagpieRecordedResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("recordFailure"),
                params: FabushiMagpieRecordFailureRequest(
                    target: trimmed,
                    statusCode: statusCode,
                    retryAfterSecs: retryAfterSecs,
                    nowMs: nowMs
                )
            )
            if response.recorded {
                let restKind = FabushiMagpieCircuitBreakerPolicy.classifyRestKind(
                    statusCode: statusCode,
                    errorText: errorText
                )
                lastCircuitRestKind = restKind
                lastCircuitFailureTarget = trimmed
                circuitBreakerState = restKind == nil ? .closed : .open
            }
            return response.recorded
        } catch {
            handleError(error)
            return false
        }
    }

    // MARK: - Usage Metrics (`magpie.recordUsage`, `magpie.usageSummary`)

    @discardableResult
    public func recordUsage(_ request: FabushiMagpieUsageRecordRequest) async -> Bool {
        guard coordinator.isStarted else { return false }
        do {
            let response: FabushiMagpieRecordedResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("recordUsage"),
                params: request
            )
            if response.recorded {
                await loadUsageSummary()
            }
            return response.recorded
        } catch {
            handleError(error)
            return false
        }
    }

    public func loadUsageSummary(sinceMs: Int64? = nil) async {
        guard coordinator.isStarted else { return }
        do {
            let summary: FabushiMagpieUsageSummaryDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("usageSummary"),
                params: FabushiMagpieUsageSummaryRequest(sinceMs: sinceMs)
            )
            usageSummary = summary
        } catch {
            handleError(error)
        }
    }

    // MARK: - PII Privacy Masking (`magpie.maskingUpdate`, `maskText`, `maskingPreview`, `unmaskText`)

    @discardableResult
    public func updateMaskingConfig(_ updated: FabushiMagpieMaskingConfigDTO) async -> FabushiMagpieMaskingConfigDTO? {
        guard coordinator.isStarted else { return nil }
        do {
            let applied: FabushiMagpieMaskingConfigDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("maskingUpdate"),
                params: updated
            )
            maskingConfig = applied
            return applied
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func previewMasking(
        text: String,
        overrideConfig: FabushiMagpieMaskingConfigDTO? = nil
    ) async -> FabushiMagpieMaskResultDTO? {
        guard coordinator.isStarted else { return nil }
        guard !text.isEmpty else { return nil }
        do {
            let result: FabushiMagpieMaskResultDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("maskingPreview"),
                params: FabushiMagpieMaskTextRequest(text: text, config: overrideConfig)
            )
            maskingPreviewResult = result
            return result
        } catch {
            handleError(error)
            return nil
        }
    }

    public func maskText(
        _ text: String,
        overrideConfig: FabushiMagpieMaskingConfigDTO? = nil
    ) async -> FabushiMagpieMaskResultDTO? {
        guard coordinator.isStarted else { return nil }
        guard !text.isEmpty else { return nil }
        do {
            return try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("maskText"),
                params: FabushiMagpieMaskTextRequest(text: text, config: overrideConfig)
            )
        } catch {
            handleError(error)
            return nil
        }
    }

    public func unmaskText(_ maskedText: String) async -> String? {
        guard coordinator.isStarted else { return nil }
        guard !maskedText.isEmpty else { return nil }
        do {
            let resp: FabushiMagpieUnmaskResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("unmaskText"),
                params: FabushiMagpieUnmaskRequest(text: maskedText)
            )
            return resp.text
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - `.env` & Deep-Link Import (`magpie.importLink`, `magpie.importDotenv`)

    @discardableResult
    public func importFromLink(_ rawUri: String) async -> FabushiMagpieImportedProviderSpecDTO? {
        guard coordinator.isStarted else { return nil }
        let trimmed = rawUri.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        do {
            let spec: FabushiMagpieImportedProviderSpecDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("importLink"),
                params: FabushiMagpieImportLinkRequest(uri: trimmed)
            )
            importedSpecs.removeAll { $0.id == spec.id }
            importedSpecs.insert(spec, at: 0)
            mergeProviderFromImportedSpec(spec)
            importLinkDraft = ""
            return spec
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func importFromDotenv(_ rawDotenv: String) async -> [FabushiMagpieImportedProviderSpecDTO] {
        guard coordinator.isStarted else { return [] }
        let trimmed = rawDotenv.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return [] }
        do {
            let specs: [FabushiMagpieImportedProviderSpecDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("importDotenv"),
                params: FabushiMagpieImportDotenvRequest(text: trimmed)
            )
            for spec in specs {
                importedSpecs.removeAll { $0.id == spec.id }
                importedSpecs.append(spec)
                mergeProviderFromImportedSpec(spec)
            }
            if !specs.isEmpty {
                importDotenvDraft = ""
            }
            return specs
        } catch {
            handleError(error)
            return []
        }
    }

    private func mergeProviderFromImportedSpec(_ spec: FabushiMagpieImportedProviderSpecDTO) {
        let masked = spec.apiKey.map { FabushiMagpieVaultOAuthRecord.maskSecretForDisplay($0) }
        let provider = FabushiMagpieProviderDTO(
            providerId: spec.id,
            name: spec.name,
            apiMode: spec.protocolKind.rawValue,
            baseUrl: spec.baseUrl,
            defaultModel: spec.defaultModel,
            models: spec.models.isEmpty ? [spec.defaultModel].filter { !$0.isEmpty } : spec.models,
            enabled: true,
            hasApiKey: !(spec.apiKey?.isEmpty ?? true),
            maskedKey: masked,
            isActive: activeProvider == spec.id,
            presetId: spec.presetId,
            regionId: spec.regionId,
            isSubscription: false,
            noKeyRequired: false
        )
        providers.removeAll { $0.providerId == provider.providerId }
        providers.append(provider)
    }

    // MARK: - Honest BYOK Save (Encrypted Vault `ai_oauth`) & Honest Health Probe

    /// Saves a BYOK / OAuth provider specification into the Zero-Knowledge Vault (`category: "ai_oauth"`)
    /// when authenticated and unlocked, and updates in-session state cleanly without fabricating a
    /// server-side `magpie.saveProvider` write that awaits M6 host RPC.
    @discardableResult
    public func saveProviderSpec(
        providerId: String,
        name: String,
        protocolKind: FabushiMagpieProtocolKind,
        baseUrl: String,
        defaultModel: String,
        apiKeyOrToken: String,
        presetId: String? = nil,
        subscriptionKind: String? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let trimmedId = providerId.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedBase = baseUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedModel = defaultModel.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedSecret = apiKeyOrToken.trimmingCharacters(in: .whitespacesAndNewlines)

        guard !trimmedId.isEmpty, !trimmedName.isEmpty else { return false }

        let masked = FabushiMagpieVaultOAuthRecord.maskSecretForDisplay(trimmedSecret)
        let provider = FabushiMagpieProviderDTO(
            providerId: trimmedId,
            name: trimmedName,
            apiMode: protocolKind.rawValue,
            baseUrl: trimmedBase.isEmpty ? nil : trimmedBase,
            defaultModel: trimmedModel,
            models: trimmedModel.isEmpty ? [] : [trimmedModel],
            enabled: true,
            hasApiKey: !trimmedSecret.isEmpty,
            maskedKey: masked.isEmpty ? nil : masked,
            isActive: activeProvider.isEmpty,
            presetId: presetId,
            isSubscription: subscriptionKind != nil,
            noKeyRequired: trimmedSecret.isEmpty
        )
        providers.removeAll { $0.providerId == provider.providerId }
        providers.insert(provider, at: 0)
        if activeProvider.isEmpty {
            activeProvider = provider.providerId
            activeModel = trimmedModel
            needsProviderSelection = false
        }

        // Persist encrypted into Zero-Knowledge Vault (`ai_oauth`) whenever signed in and unlocked.
        guard case .authenticated(let userId, _, _) = coordinator.authState else {
            oauthStatusMessage = FabushiAgentStudioStrings.text("m4.magpie.savePendingM6Notice", locale: locale)
            return true
        }

        await refreshVaultStatus()
        guard vaultUnlocked else {
            oauthStatusMessage = FabushiAgentStudioStrings.text("m4.magpie.vaultLockedNotice", locale: locale)
            return true
        }

        do {
            let safeKey = try FabushiVaultLimits.validateItemKey("magpie:\(trimmedId)")
            var payloadDict: [String: Any] = [
                "provider_id": trimmedId,
                "provider_name": trimmedName,
                "protocol": protocolKind.rawValue,
                "base_url": trimmedBase,
                "default_model": trimmedModel,
                "models": trimmedModel.isEmpty ? [] : [trimmedModel],
            ]
            if let subscriptionKind, !subscriptionKind.isEmpty {
                payloadDict["subscription_kind"] = subscriptionKind
            }
            if !trimmedSecret.isEmpty {
                payloadDict["api_key"] = trimmedSecret
            }
            let payloadData = try JSONSerialization.data(withJSONObject: payloadDict, options: [.sortedKeys])
            let plaintext = String(decoding: payloadData, as: UTF8.self)
            try FabushiVaultLimits.validatePayloadSize(plaintext)

            let existingRev = vaultOAuthRecords.first(where: { $0.itemKey == safeKey })?.revision ?? 0
            let nextRev = existingRev + 1
            let sealed: FabushiEncryptedVaultBlobDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("sealItem"),
                params: FabushiVaultSealItemRequest(
                    userId: userId,
                    category: FabushiVaultCategory.aiOauth.rawValue,
                    itemKey: safeKey,
                    revision: nextRev,
                    plaintext: plaintext
                )
            )
            let pushResp: FabushiVaultSyncPushResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("syncPush"),
                params: FabushiVaultSyncPushRequest(
                    category: FabushiVaultCategory.aiOauth.rawValue,
                    itemKey: safeKey,
                    expectedRevision: existingRev > 0 ? existingRev : nil,
                    nonceB64: sealed.nonceB64,
                    ciphertextB64: sealed.ciphertextB64,
                    deleted: false
                )
            )
            let record = FabushiMagpieVaultOAuthRecord(
                itemKey: safeKey,
                providerId: trimmedId,
                providerName: trimmedName,
                subscriptionKind: subscriptionKind,
                protocolKind: protocolKind.rawValue,
                baseUrl: trimmedBase.isEmpty ? nil : trimmedBase,
                defaultModel: trimmedModel,
                models: trimmedModel.isEmpty ? [] : [trimmedModel],
                maskedTokenPreview: masked,
                revision: pushResp.revision,
                updatedAtMs: pushResp.updatedAtMs
            )
            vaultOAuthRecords.removeAll { $0.itemKey == safeKey }
            vaultOAuthRecords.insert(record, at: 0)
            oauthStatusMessage = FabushiAgentStudioStrings.text("m4.magpie.savePendingM6Notice", locale: locale)
            return pushResp.ok
        } catch {
            handleError(error)
            return false
        }
    }

    /// Honestly probes live endpoint resolution (`magpie.resolvePresetEndpoints`) and route health
    /// (`magpie.resolveRoute` + `magpie.state(remote: true)` when authenticated). NEVER fabricates
    /// fake 200 OK HTTP latency; explicitly discloses that server-side `magpie.probeProvider` awaits M6.
    public func probeProviderHealth(
        presetId: String? = nil,
        region: String? = nil,
        protocolKind: FabushiMagpieProtocolKind? = nil,
        locale: FabushiLocale = .en
    ) async {
        guard coordinator.isStarted else { return }
        isProbing = true
        defer { isProbing = false }

        var parts: [String] = []
        let targetPresetId = presetId
            ?? providers.first(where: { $0.isActive })?.presetId
            ?? providers.first?.presetId
            ?? presets.first?.id

        if let pid = targetPresetId, !pid.isEmpty,
           let resolved = await resolvePresetEndpoints(
               presetId: pid,
               region: region,
               protocolKind: protocolKind
           )
        {
            parts.append("\(resolved.displayName) -> \(resolved.baseUrl) [\(resolved.protocolKind.rawValue)]")
        }

        if let firstGroup = routingGroups.first {
            let routes = await resolveRoute(target: firstGroup.qualifiedModelName)
            if let lead = routes.first {
                parts.append("Route: \(lead.providerId)/\(lead.model) (\(lead.strategy.rawValue))")
            }
        }

        let disclaimer = FabushiAgentStudioStrings.text("m4.magpie.probePendingM6Notice", locale: locale)
        if parts.isEmpty {
            probeStatusText = disclaimer
        } else {
            probeStatusText = parts.joined(separator: " · ") + " · " + disclaimer
        }
    }

    // MARK: - Zero-Knowledge Vault AI OAuth Sync (`USER_VAULT_CATEGORY_AI_OAUTH == "ai_oauth"`)

    public func refreshVaultStatus() async {
        guard coordinator.isStarted else {
            vaultUnlocked = false
            return
        }
        do {
            let status: FabushiVaultUnlockStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("status"),
                params: FabushiEmptyParams()
            )
            vaultUnlocked = status.unlocked
        } catch {
            vaultUnlocked = coordinator.vaultStore.isUnlocked
        }
    }

    /// Syncs desktop AI OAuth credentials from the Zero-Knowledge Vault (`category: "ai_oauth"`)
    /// via `vault.syncPull` + `vault.openItem`. Strictly complies with `IOS-MAIN-INV-04` (zero dotfile scan).
    @discardableResult
    public func syncVaultAiOAuth(locale: FabushiLocale = .en) async -> Int {
        guard coordinator.isStarted else { return 0 }
        guard case .authenticated(let userId, _, _) = coordinator.authState else {
            oauthStatusMessage = FabushiAgentStudioStrings.text("m4.common.signInRequired", locale: locale)
            return 0
        }

        await refreshVaultStatus()
        guard vaultUnlocked else {
            oauthStatusMessage = FabushiAgentStudioStrings.text("m4.magpie.vaultLockedNotice", locale: locale)
            return 0
        }

        do {
            let pullResp: FabushiVaultSyncPullResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("syncPull"),
                params: FabushiVaultSyncPullRequest(
                    sinceRevision: nil,
                    category: FabushiVaultCategory.aiOauth.rawValue
                )
            )
            var decrypted: [FabushiMagpieVaultOAuthRecord] = []
            for blob in pullResp.items where !blob.deleted && blob.category == FabushiVaultCategory.aiOauth.rawValue {
                let opened: FabushiVaultOpenItemResponse = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("openItem"),
                    params: FabushiVaultOpenItemRequest(userId: userId, blob: blob)
                )
                let rawPlaintext: String
                if let utf8 = opened.plaintextUtf8 {
                    rawPlaintext = utf8
                } else if let data = Data(base64Encoded: opened.plaintextB64) {
                    rawPlaintext = String(decoding: data, as: UTF8.self)
                } else {
                    continue
                }
                let record = decodeVaultOAuthRecord(blob: blob, plaintext: rawPlaintext)
                decrypted.append(record)
            }
            vaultOAuthRecords = decrypted
            mergeVaultOAuthSubscriptions(decrypted)
            oauthStatusMessage = FabushiAgentStudioStrings.format(
                "m4.magpie.vaultSyncedFormat",
                locale: locale,
                Int32(clamping: decrypted.count)
            )
            router.toastController.showNonChatToast(oauthStatusMessage ?? "", isError: false)
            return decrypted.count
        } catch {
            handleError(error)
            return 0
        }
    }

    private func decodeVaultOAuthRecord(
        blob: FabushiEncryptedVaultBlobDTO,
        plaintext: String
    ) -> FabushiMagpieVaultOAuthRecord {
        if let data = plaintext.data(using: .utf8),
           let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        {
            let providerId = (obj["provider_id"] as? String) ?? blob.itemKey
            let providerName = (obj["provider_name"] as? String) ?? providerId
            let subKind = obj["subscription_kind"] as? String
            let proto = (obj["protocol"] as? String) ?? FabushiMagpieProtocolKind.chatCompletions.rawValue
            let baseUrl = obj["base_url"] as? String
            let defaultModel = (obj["default_model"] as? String) ?? ""
            let models = (obj["models"] as? [String]) ?? (defaultModel.isEmpty ? [] : [defaultModel])
            let email = obj["email"] as? String
            let secret = (obj["access_token"] as? String)
                ?? (obj["api_key"] as? String)
                ?? (obj["refresh_token"] as? String)
                ?? ""
            return FabushiMagpieVaultOAuthRecord(
                itemKey: blob.itemKey,
                providerId: providerId,
                providerName: providerName,
                subscriptionKind: subKind,
                protocolKind: proto,
                baseUrl: baseUrl,
                defaultModel: defaultModel,
                models: models,
                email: email,
                maskedTokenPreview: FabushiMagpieVaultOAuthRecord.maskSecretForDisplay(secret),
                revision: blob.revision,
                updatedAtMs: blob.updatedAtMs
            )
        }

        return FabushiMagpieVaultOAuthRecord(
            itemKey: blob.itemKey,
            providerId: blob.itemKey,
            providerName: blob.itemKey,
            maskedTokenPreview: FabushiMagpieVaultOAuthRecord.maskSecretForDisplay(plaintext),
            revision: blob.revision,
            updatedAtMs: blob.updatedAtMs
        )
    }

    private func mergeVaultOAuthSubscriptions(_ records: [FabushiMagpieVaultOAuthRecord]) {
        for rec in records {
            guard let kind = rec.subscriptionKind, !kind.isEmpty else { continue }
            let sub = FabushiMagpieSubscriptionDTO(
                id: rec.providerId,
                kind: kind,
                label: rec.providerName,
                email: rec.email,
                accountId: coordinator.activeAccountId,
                active: FabushiMagpieSubscriptionCanonicalizer.isSubscriptionActive(
                    subscriptionId: rec.providerId,
                    kind: kind,
                    activeProvider: activeProvider
                ),
                baseUrl: rec.baseUrl,
                protocolName: rec.protocolKind,
                defaultModel: rec.defaultModel.isEmpty ? nil : rec.defaultModel,
                models: rec.models,
                sourcePath: "vault:ai_oauth:\(rec.itemKey)"
            )
            subscriptions.removeAll { $0.id == sub.id }
            subscriptions.append(sub)
        }
        syncSelectedSubscriptionModels(
            subscriptions: subscriptions,
            activeProvider: activeProvider,
            activeModel: activeModel
        )
    }

    // MARK: - Strict `IOS-MAIN-INV-04` Fail-Closed Local Subscription Scan Probe

    /// Verifies that local home-directory dotfile scanning (`magpie.scanLocalSubscriptions`) fails closed
    /// on iOS with `capabilityUnavailable`, surfacing an honest sandbox notice instead of crashing.
    @discardableResult
    public func probeLocalDotfileScanBoundary(locale: FabushiLocale = .en) async -> Bool {
        guard coordinator.isStarted else { return false }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.magpie.method("scanLocalSubscriptions"),
                params: FabushiEmptyParams()
            )
            return false
        } catch let hostError as FabushiHostError {
            if case .capabilityUnavailable = hostError {
                lastSandboxNotice = FabushiAgentStudioStrings.text("m4.magpie.noDotfileScanNotice", locale: locale)
                return true
            }
            handleError(hostError)
            return false
        } catch {
            handleError(error)
            return false
        }
    }

    // MARK: - Browser OAuth via `ASWebAuthenticationSession`

    public func startBrowserOAuth(
        authorizeURLString: String,
        prefersEphemeralSession: Bool = false
    ) {
        guard let url = FabushiMagpieOAuthController.validateAuthorizeURL(authorizeURLString) else {
            oauthStatusMessage = FabushiMagpieOAuthError.invalidAuthorizeURL.localizedDescription
            router.toastController.showNonChatToast(
                FabushiMagpieOAuthError.invalidAuthorizeURL.localizedDescription,
                isError: true
            )
            return
        }

        oauthController.startOAuthSession(
            url: url,
            callbackScheme: FabushiMagpieOAuthController.defaultCallbackScheme,
            prefersEphemeralSession: prefersEphemeralSession
        ) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let payload):
                self.lastOAuthCallback = payload
                let summary = payload.code != nil
                    ? "OAuth authorization callback received (code redacted)."
                    : "OAuth callback completed."
                self.oauthStatusMessage = summary
                self.router.toastController.showNonChatToast(summary, isError: false)
            case .failure(let error):
                self.oauthStatusMessage = error.localizedDescription
                if error != .userCancelled {
                    self.router.toastController.showNonChatToast(error.localizedDescription, isError: true)
                }
            }
        }
    }

    // MARK: - `D10-10` 3-Tier Error Classification & Routing

    public func reportProviderError(rawMessage: String, needsProviderCta: Bool = false, locale: FabushiLocale = .en) {
        let classified = FabushiMagpieErrorClassifier.classify(
            rawMessage: rawMessage,
            needsProviderCta: needsProviderCta
        )
        lastClassifiedError = classified
        if classified.tier == .unconfiguredProvider {
            router.activeProviderErrorMessage = classified.detail(locale: locale)
        } else {
            // Tier 2 & Tier 3 MUST NOT trigger the misleading "Open AI Provider Settings" Shell CTA!
            router.dismissProviderErrorCard()
        }
        router.toastController.showNonChatToast(classified.toastMessage(locale: locale), isError: true)
    }

    public func dismissClassifiedError() {
        lastClassifiedError = nil
        router.dismissProviderErrorCard()
    }

    private func handleError(_ error: Error) {
        if let coordError = error as? FabushiCoordinatorError,
           case .staleGeneration = coordError
        {
            return
        }
        if let hostError = error as? FabushiHostError {
            if hostError.isCancelled {
                return
            }
            if case .staleGeneration = hostError {
                return
            }
            if case .capabilityUnavailable(let message) = hostError {
                lastSandboxNotice = message
                router.toastController.showNonChatToast(message, isError: true)
                return
            }
        }
        reportProviderError(rawMessage: error.localizedDescription)
    }
}
