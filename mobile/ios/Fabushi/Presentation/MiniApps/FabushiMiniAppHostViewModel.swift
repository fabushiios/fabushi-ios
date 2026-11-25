import Foundation
import Observation

/// Generation-fenced ViewModel driving External Mini Apps, WebMCP bidirectional control,
/// `D10-12` (`startMinimized`, unified `minimize()`, `CreateManagedBotBox` `originBoxId` race guard),
/// `D12-01` (`FabushiFocusGatedWKWebView` focus retention & teardown), and 全球法布施 (`miniapp.dharma*`).
@MainActor
@Observable
public final class FabushiMiniAppHostViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let sessionPool: FabushiMiniAppSessionPool

    // Catalog & Navigation State
    public var searchQuery: String = ""
    public var selectedFilter: FabushiMiniAppCatalogFilter = .all
    public private(set) var catalogApps: [FabushiMiniAppEntry] = []
    public private(set) var installedApps: [FabushiMiniAppEntry] = []
    public private(set) var recentApps: [FabushiMiniAppEntry] = []
    public private(set) var activeSession: FabushiMiniAppActiveSession? = nil
    public var isGlobalDharmaPresented: Bool = false
    public var isBrowserSheetPresented: Bool = false

    // ToS Gate & Security State
    public var pendingTosPrompt: FabushiMiniAppTosPromptState? = nil
    public private(set) var tosBlockedMessage: String? = nil
    public private(set) var webMcpErrorMessage: String? = nil
    public private(set) var lastMinimizeReason: FabushiMiniAppMinimizeReason? = nil

    // D10-12 CreateManagedBotBox Close-Race Guard (`originBoxId`)
    public private(set) var activeCreateBoxId: UUID? = nil
    public var isCreateBotSheetPresented: Bool = false
    public private(set) var isCreatingManagedBot: Bool = false
    public private(set) var createBotModalError: String? = nil
    public private(set) var lastCreatedBotRouteHijacked: Bool = false
    public private(set) var catalogRefreshCount: Int = 0

    // 全球法布施 (Global Dharma) Live Rust Core State
    public private(set) var dharmaBootstrap: FabushiJSONValue? = nil
    public private(set) var dharmaSnapshot: FabushiJSONValue? = nil
    public private(set) var dharmaLeaderboard: FabushiJSONValue? = nil
    public private(set) var dharmaWasmAssetInfo: FabushiMiniAppDharmaAssetInfo? = nil
    public private(set) var isGlobalDharmaPackageVerified: Bool = false
    public private(set) var globalDharmaVerificationError: String? = nil
    public private(set) var localChantCount: Int = 0
    public private(set) var isPracticeSessionActive: Bool = false

    // General Status
    public private(set) var isLoading: Bool = false
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var observedGeneration: UInt64

    @ObservationIgnored private let purgerId = "m5.miniapps.\(UUID().uuidString)"
    @ObservationIgnored nonisolated(unsafe) private var eventTask: Task<Void, Never>?

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        sessionPool: FabushiMiniAppSessionPool = .shared
    ) {
        self.coordinator = coordinator
        self.router = router
        self.sessionPool = sessionPool
        self.observedGeneration = coordinator.accountScopeGeneration
        installAccountResetHooks()
        startEventSubscription()
    }

    deinit {
        eventTask?.cancel()
    }

    public var filteredApps: [FabushiMiniAppEntry] {
        let base: [FabushiMiniAppEntry]
        switch selectedFilter {
        case .all:
            base = catalogApps
        case .installed:
            base = installedApps.isEmpty ? catalogApps.filter(\.isInstalled) : installedApps
        case .recent:
            base = recentApps.isEmpty ? catalogApps.filter(\.isRecentUsed) : recentApps
        }
        let q = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !q.isEmpty else { return base }
        return base.filter { app in
            app.title.lowercased().contains(q)
                || app.botUsername.lowercased().contains(q)
                || app.shortName.lowercased().contains(q)
                || app.description.lowercased().contains(q)
        }
    }

    public var activePooledSession: FabushiMiniAppSessionPool.PooledSession? {
        guard let activeSession else { return nil }
        return sessionPool.existingSession(
            generation: activeSession.generation,
            appKey: activeSession.app.appKey
        )
    }

    // MARK: - Generation Fencing & Account Switch Teardown (`D12-01`)

    private func installAccountResetHooks() {
        coordinator.registerAccountSecretsPurger(id: purgerId) { [weak self] departingAccountId in
            self?.resetStateForAccountScopeChange(
                generation: self?.coordinator.accountScopeGeneration ?? 0,
                departingAccountId: departingAccountId
            )
        }
        coordinator.registerServerSwitchSecretsPurger(id: purgerId) { [weak self] in
            self?.resetStateForAccountScopeChange(
                generation: self?.coordinator.accountScopeGeneration ?? 0,
                departingAccountId: nil
            )
        }
    }

    public func detachLifecycleHooks() {
        eventTask?.cancel()
        eventTask = nil
        coordinator.unregisterAccountSecretsPurger(id: purgerId)
        coordinator.unregisterServerSwitchSecretsPurger(id: purgerId)
    }

    public func reconcileAccountScopeIfNeeded() {
        let currentGen = coordinator.accountScopeGeneration
        guard currentGen != observedGeneration else { return }
        resetStateForAccountScopeChange(generation: currentGen, departingAccountId: nil)
    }

    public func resetStateForAccountScopeChange(
        generation: UInt64,
        departingAccountId: String?
    ) {
        observedGeneration = generation
        sessionPool.teardownAll(revokingAccountId: departingAccountId ?? coordinator.activeAccountId)
        catalogApps = []
        installedApps = []
        recentApps = []
        activeSession = nil
        isGlobalDharmaPresented = false
        isBrowserSheetPresented = false
        pendingTosPrompt = nil
        tosBlockedMessage = nil
        webMcpErrorMessage = nil
        activeCreateBoxId = nil
        isCreateBotSheetPresented = false
        isCreatingManagedBot = false
        createBotModalError = nil
        dharmaBootstrap = nil
        dharmaSnapshot = nil
        dharmaLeaderboard = nil
        isGlobalDharmaPackageVerified = false
        globalDharmaVerificationError = nil
        localChantCount = 0
        isPracticeSessionActive = false
        router.activeMiniAppPip = nil
        router.presentedMiniAppId = nil
    }

    private func startEventSubscription() {
        let stream = coordinator.host.makeEventStream()
        eventTask = Task { @MainActor [weak self] in
            for await event in stream {
                guard !Task.isCancelled, let self else { break }
                self.reconcileAccountScopeIfNeeded()
                guard event.generation >= self.coordinator.accountScopeGeneration else { continue }
                switch event.kind {
                case .miniAppControlStatus(let botUsername, let shortName, _, _, let appTitle, let modeRaw, let statusText, _):
                    let appKey = "\(botUsername.trimmingCharacters(in: CharacterSet(charactersIn: "@")))/\(shortName)"
                    let parsedMode = FabushiMiniAppControlMode(rawValue: modeRaw) ?? .idle
                    if var session = self.activeSession, session.app.appKey == appKey {
                        session.controlMode = parsedMode
                        session.statusText = statusText
                        self.activeSession = session
                        if let pip = self.router.activeMiniAppPip, pip.id == appKey {
                            self.router.activeMiniAppPip = MiniAppPipOverlayState(
                                id: appKey,
                                title: appTitle.isEmpty ? session.app.title : appTitle,
                                botUsername: botUsername,
                                statusText: statusText ?? parsedMode.displayTitle(locale: .en)
                            )
                        }
                    }
                case .miniAppWebMcpCommand(let payload):
                    self.handleIncomingWebMcpCommand(payload)
                default:
                    break
                }
            }
        }
    }

    private func handleIncomingWebMcpCommand(_ payload: FabushiJSONValue) {
        let operation = payload["operation"]?.stringValue ?? ""
        if operation == "request_takeover" {
            let note = payload["status_message"]?.stringValue
            if var session = activeSession {
                session.controlMode = .waitingForTakeover
                session.statusText = note
                activeSession = session
            }
        }
    }

    // MARK: - Catalog Loading & Initial Route Handling

    public func handleInitialMiniAppId(_ miniAppId: String?, locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        await refreshCatalog()

        guard let raw = miniAppId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !raw.isEmpty,
              raw.lowercased() != "catalog"
        else {
            return
        }

        let lower = raw.lowercased()
        if lower == "global_dharma" || lower == "global-dharma" {
            isGlobalDharmaPresented = true
            await loadGlobalDharma()
            return
        }

        if let active = activeSession,
           active.app.appId == raw || active.app.appKey == raw || active.app.shortName == raw
        {
            updateFocusPolicy(isPresented: true, isHostInputFocused: false)
            return
        }

        if let match = catalogApps.first(where: {
            $0.appId == raw || $0.appKey == raw || $0.shortName == raw || $0.botUsername == raw
        }) {
            _ = await launchMiniApp(match, startMinimized: false, locale: locale)
        }
    }

    public func refreshCatalog() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        isLoading = true
        defer { isLoading = false }

        struct ListParams: Encodable, Sendable {
            let query: String?
            let category: String?
        }

        let trimmedQuery = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let list: [FabushiMiniAppEntry] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.miniapp.method("list"),
                params: ListParams(query: trimmedQuery.isEmpty ? nil : trimmedQuery, category: nil)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            catalogApps = list
            catalogRefreshCount += 1
            lastErrorMessage = nil
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
        }

        if let installed: [FabushiMiniAppEntry] = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("listInstalled"),
            params: FabushiEmptyParams()
        ), coordinator.accountScopeGeneration == capturedGen {
            installedApps = installed
        }

        if let recent: [FabushiMiniAppEntry] = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("listRecent"),
            params: FabushiEmptyParams()
        ), coordinator.accountScopeGeneration == capturedGen {
            recentApps = recent
        }
    }

    // MARK: - D10-12 Launch (`startMinimized`), Mandatory ToS Gate & In-Place PiP Pool

    @discardableResult
    public func launchMiniApp(
        _ app: FabushiMiniAppEntry,
        startMinimized: Bool = false,
        customUrl: String? = nil,
        startParam: String? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        tosBlockedMessage = nil
        webMcpErrorMessage = nil

        // 1. Mandatory ToS Gate
        if !app.appTosAccepted {
            pendingTosPrompt = FabushiMiniAppTosPromptState(
                app: app,
                pendingStartMinimized: startMinimized,
                pendingCustomUrl: customUrl,
                pendingStartParam: startParam
            )
            updateFocusPolicy(isPresented: true, isHostInputFocused: false)
            return false
        }

        // 2. Sanitize Custom / Target URL Fail-Closed via Rust Host
        let candidateUrlRaw = (customUrl?.isEmpty == false ? customUrl! : app.webUrl)
        var resolvedURL: URL? = nil
        if !candidateUrlRaw.isEmpty {
            let sanitized = await sanitizeCustomUrl(candidateUrlRaw)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            guard let sanitized, let parsed = URL(string: sanitized) else {
                let msg = FabushiPlatformStrings.text(.miniAppInvalidUrlBlocked, locale: locale)
                webMcpErrorMessage = msg
                router.toastController.showNonChatToast(msg, isError: true)
                return false
            }
            resolvedURL = parsed
        }

        // 3. Call server launch context if available (fallback gracefully to local entry when offline/local)
        var companionChatId = app.directChatId
        struct LaunchParams: Encodable, Sendable {
            let botUsername: String
            let shortName: String
            let launchMode: FabushiMiniAppLaunchMode
            let startParam: String?
            let customUrl: String?
            enum CodingKeys: String, CodingKey {
                case botUsername = "bot_username"
                case shortName = "short_name"
                case launchMode = "launch_mode"
                case startParam = "start_param"
                case customUrl = "custom_url"
            }
        }
        if let ctx: FabushiMiniAppLaunchContext = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("launch"),
            params: LaunchParams(
                botUsername: app.botUsername,
                shortName: app.shortName,
                launchMode: .fullSize,
                startParam: startParam,
                customUrl: resolvedURL?.absoluteString
            )
        ), coordinator.accountScopeGeneration == capturedGen {
            companionChatId = ctx.companionChatId ?? companionChatId
        }

        guard coordinator.accountScopeGeneration == capturedGen else { return false }

        // 4. Acquire or reuse in-place pooled Focus-Gated WKWebView
        let pooled = sessionPool.acquireOrCreateSession(
            generation: capturedGen,
            appKey: app.appKey,
            accountId: coordinator.activeAccountId
        )
        if let resolvedURL {
            pooled.loadIfNeeded(resolvedURL)
        }

        let controlMode: FabushiMiniAppControlMode = startMinimized ? .aiControlling : .idle
        let session = FabushiMiniAppActiveSession(
            app: app,
            resolvedURL: resolvedURL,
            companionChatId: companionChatId,
            controlMode: controlMode,
            statusText: controlMode.displayTitle(locale: locale),
            isBackgroundWebMcp: startMinimized,
            generation: capturedGen
        )
        activeSession = session
        isGlobalDharmaPresented = false

        if startMinimized {
            // D10-12 + D12-01: Mount directly into floating PiP WITHOUT setting presentedMiniAppId
            // and strictly keeping allowFirstResponder == false!
            let context = FabushiWebViewFocusContext(
                isPresented: false,
                isMinimizedToPiP: true,
                isBackgroundWebMcp: true,
                hasBlockingModal: false,
                isHostInputFocused: false
            )
            pooled.webView.enforceFocusPolicy(context: context)
            router.activeMiniAppPip = MiniAppPipOverlayState(
                id: app.appKey,
                title: app.title,
                botUsername: app.botUsername,
                statusText: controlMode.displayTitle(locale: locale)
            )
            // Explicitly ensure presentedMiniAppId is NOT set for background PiP launch
            if router.presentedMiniAppId == app.appKey || router.presentedMiniAppId == app.appId {
                router.presentedMiniAppId = nil
            }
        } else {
            router.presentedMiniAppId = app.appKey
            updateFocusPolicy(isPresented: true, isHostInputFocused: false)
        }

        return true
    }

    @discardableResult
    public func acceptPendingTos(locale: FabushiLocale = .en) async -> Bool {
        guard let prompt = pendingTosPrompt else { return false }
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        struct AcceptParams: Encodable, Sendable {
            let botId: UInt64
            let appId: String?
            let allowWriteAccess: Bool
            enum CodingKeys: String, CodingKey {
                case botId = "bot_id"
                case appId = "app_id"
                case allowWriteAccess = "allow_write_access"
            }
        }

        if prompt.app.botUserId > 0 {
            do {
                let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.miniapp.method("acceptTos"),
                    params: AcceptParams(
                        botId: UInt64(prompt.app.botUserId),
                        appId: prompt.app.appId.isEmpty ? nil : prompt.app.appId,
                        allowWriteAccess: true
                    )
                )
            } catch {
                guard coordinator.accountScopeGeneration == capturedGen else { return false }
                lastErrorMessage = error.localizedDescription
                router.toastController.showNonChatToast(error.localizedDescription, isError: true)
                return false
            }
        }

        guard coordinator.accountScopeGeneration == capturedGen else { return false }
        let acceptedApp = prompt.app.withTosAccepted(true)
        if let idx = catalogApps.firstIndex(where: { $0.appKey == acceptedApp.appKey }) {
            catalogApps[idx] = acceptedApp
        }
        pendingTosPrompt = nil
        tosBlockedMessage = nil
        return await launchMiniApp(
            acceptedApp,
            startMinimized: prompt.pendingStartMinimized,
            customUrl: prompt.pendingCustomUrl,
            startParam: prompt.pendingStartParam,
            locale: locale
        )
    }

    public func declinePendingTos(locale: FabushiLocale = .en) {
        pendingTosPrompt = nil
        tosBlockedMessage = FabushiPlatformStrings.text(.miniAppTosBlockedMessage, locale: locale)
        if let pooled = activePooledSession {
            pooled.webView.enforceFocusPolicy(allowFirstResponder: false)
        }
    }

    // MARK: - D10-12 Unified `minimize(...)`, In-Place Restore & Explicit Close

    /// Single normalized minimize helper routed from Minimize button, Open Bot Profile,
    /// Open Mini App ToS, Open Telegram Link, Open Companion Chat, and sheet dismiss/Escape.
    public func minimize(
        reason: FabushiMiniAppMinimizeReason,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) {
        lastMinimizeReason = reason
        if var session = activeSession {
            if let pooled = sessionPool.existingSession(
                generation: session.generation,
                appKey: session.app.appKey
            ) {
                let pipFocusContext = FabushiWebViewFocusContext(
                    isPresented: false,
                    isMinimizedToPiP: true,
                    isBackgroundWebMcp: session.isBackgroundWebMcp,
                    hasBlockingModal: false,
                    isHostInputFocused: false
                )
                pooled.webView.enforceFocusPolicy(context: pipFocusContext)
            }
            session.statusText = session.controlMode.displayTitle(locale: locale)
            activeSession = session
            router.activeMiniAppPip = MiniAppPipOverlayState(
                id: session.app.appKey,
                title: session.app.title,
                botUsername: session.app.botUsername,
                statusText: session.statusText
            )
        }

        router.dismissActiveModalSheet(coordinator: coordinator)
        router.reconcileChatScopedToasts(coordinator: coordinator, isCompact: isCompact)
    }

    /// Restores an active minimized Mini App session in-place from `floatingPiPPill`
    /// WITHOUT reloading its pooled `FabushiFocusGatedWKWebView`.
    public func restoreFromPiP(appKey: String? = nil) {
        reconcileAccountScopeIfNeeded()
        let targetId = appKey ?? activeSession?.app.appKey ?? router.activeMiniAppPip?.id
        guard let targetId else { return }

        if var session = activeSession, session.app.appKey == targetId {
            session.isBackgroundWebMcp = false
            activeSession = session
        }

        router.presentedMiniAppId = targetId
        updateFocusPolicy(isPresented: true, isHostInputFocused: false)
    }

    /// Explicitly terminates the active Mini App session, calls `teardownSession()` on the pooled
    /// `WKWebView`, revokes WebMCP sessions, and clears PiP & sheet presentation.
    public func closeActiveSession(isCompact: Bool = true) {
        if let session = activeSession {
            sessionPool.teardownSession(
                generation: session.generation,
                appKey: session.app.appKey,
                accountId: coordinator.activeAccountId
            )
        } else if let pipId = router.activeMiniAppPip?.id {
            sessionPool.teardownSession(
                generation: coordinator.accountScopeGeneration,
                appKey: pipId,
                accountId: coordinator.activeAccountId
            )
        }
        activeSession = nil
        pendingTosPrompt = nil
        isGlobalDharmaPresented = false
        router.activeMiniAppPip = nil
        router.dismissActiveModalSheet(coordinator: coordinator)
        router.reconcileChatScopedToasts(coordinator: coordinator, isCompact: isCompact)
    }

    // MARK: - D12-01 Focus Policy Enforcement

    @discardableResult
    public func updateFocusPolicy(
        isPresented: Bool,
        isHostInputFocused: Bool
    ) -> Bool {
        guard let pooled = activePooledSession, let session = activeSession else {
            return false
        }
        let isMinimized = router.presentedMiniAppId == nil && router.activeMiniAppPip != nil
        let hasBlockingModal = pendingTosPrompt != nil
            || isCreateBotSheetPresented
            || isBrowserSheetPresented
            || router.isPasscodeLocked
            || router.isAccountSwitcherPresented
            || router.isCallsSheetPresented

        let context = FabushiWebViewFocusContext(
            isPresented: isPresented && router.presentedMiniAppId != nil,
            isMinimizedToPiP: isMinimized,
            isBackgroundWebMcp: session.isBackgroundWebMcp,
            hasBlockingModal: hasBlockingModal,
            isHostInputFocused: isHostInputFocused
        )
        return pooled.webView.enforceFocusPolicy(context: context)
    }

    // MARK: - D10-12 `CreateManagedBotBox` Close-Race Guard (`originBoxId: UUID`)

    @discardableResult
    public func openCreateManagedBotBox() -> UUID {
        let boxId = UUID()
        activeCreateBoxId = boxId
        createBotModalError = nil
        isCreateBotSheetPresented = true
        updateFocusPolicy(isPresented: true, isHostInputFocused: true)
        return boxId
    }

    public func closeCreateManagedBotBox(boxId: UUID? = nil) {
        if let boxId, activeCreateBoxId != boxId {
            return
        }
        activeCreateBoxId = nil
        isCreateBotSheetPresented = false
        createBotModalError = nil
        updateFocusPolicy(isPresented: true, isHostInputFocused: false)
    }

    /// Creates a managed bot/mini-app tagged with `originBoxId`.
    /// If the user dismisses the modal mid-flight (`activeCreateBoxId != originBoxId`),
    /// strictly suppresses route hijack and stale modal error mutation while refreshing
    /// the catalog (if `accountScopeGeneration` still matches) and emitting a non-chat toast.
    @discardableResult
    public func submitCreateManagedBot(
        prompt: String,
        originBoxId: UUID,
        avatarShape: String? = nil,
        avatarColor: String? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let trimmed = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        reconcileAccountScopeIfNeeded()

        let capturedGen = coordinator.accountScopeGeneration
        isCreatingManagedBot = true
        lastCreatedBotRouteHijacked = false
        if activeCreateBoxId == originBoxId {
            createBotModalError = nil
        }

        struct CreateNlParams: Encodable, Sendable {
            let prompt: String
            let avatarShape: String?
            let avatarColor: String?
            enum CodingKeys: String, CodingKey {
                case prompt
                case avatarShape = "avatar_shape"
                case avatarColor = "avatar_color"
            }
        }

        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("botCreateNl"),
                params: CreateNlParams(
                    prompt: trimmed,
                    avatarShape: avatarShape,
                    avatarColor: avatarColor
                )
            )
            isCreatingManagedBot = false

            // Account switched or logged out mid-flight -> drop completely!
            guard coordinator.accountScopeGeneration == capturedGen else {
                return false
            }

            await refreshCatalog()
            guard coordinator.accountScopeGeneration == capturedGen else {
                return false
            }

            if activeCreateBoxId == originBoxId && isCreateBotSheetPresented {
                activeCreateBoxId = nil
                isCreateBotSheetPresented = false
                createBotModalError = nil
                router.toastController.showNonChatToast(
                    FabushiPlatformStrings.text(.miniAppCreateBotSuccess, locale: locale),
                    isError: false
                )
            } else {
                // Modal was closed/replaced mid-flight: suppress route hijack & stale error mutation!
                lastCreatedBotRouteHijacked = false
                router.toastController.showNonChatToast(
                    FabushiPlatformStrings.text(.miniAppCreateBotBackgroundSuccess, locale: locale),
                    isError: false
                )
            }
            return true
        } catch {
            isCreatingManagedBot = false
            guard coordinator.accountScopeGeneration == capturedGen else {
                return false
            }

            let prefix = FabushiPlatformStrings.text(.miniAppCreateBotFailedPrefix, locale: locale)
            let msg = "\(prefix): \(error.localizedDescription)"
            if activeCreateBoxId == originBoxId && isCreateBotSheetPresented {
                // Only mutate modal error when the originating box is still open!
                createBotModalError = msg
            } else {
                // Closed/replaced mid-flight: DO NOT mutate modal error; emit non-chat toast only.
                router.toastController.showNonChatToast(msg, isError: true)
            }
            return false
        }
    }

    // MARK: - URI, URL Sanitization, InitData HMAC & WebMCP RPCs

    public func parseMiniAppUri(_ uri: String) async -> FabushiMiniAppParsedUri? {
        struct Params: Encodable, Sendable { let uri: String }
        return try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("parseUri"),
            params: Params(uri: uri)
        )
    }

    public func sanitizeCustomUrl(_ url: String) async -> String? {
        struct Params: Encodable, Sendable { let url: String }
        let response: FabushiMiniAppSanitizeUrlResponse? = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("sanitizeCustomUrl"),
            params: Params(url: url)
        )
        return response?.sanitizedUrl
    }

    public func verifyInitDataLocal(initData: String, botToken: String) async -> Bool {
        struct Params: Encodable, Sendable {
            let initData: String
            let botToken: String
            enum CodingKeys: String, CodingKey {
                case initData = "init_data"
                case botToken = "bot_token"
            }
        }
        let response: FabushiMiniAppVerifyInitDataResponse? = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("verifyInitDataLocal"),
            params: Params(initData: initData, botToken: botToken)
        )
        return response?.valid ?? false
    }

    public func setWebMcpControlMode(
        appKey: String,
        mode: FabushiMiniAppControlMode,
        locale: FabushiLocale = .en
    ) async -> FabushiMiniAppControlStateResponse? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let appKey: String
            let mode: FabushiMiniAppControlMode
            enum CodingKeys: String, CodingKey {
                case appKey = "app_key"
                case mode
            }
        }
        do {
            let response: FabushiMiniAppControlStateResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.webmcp.method("setControlMode"),
                params: Params(appKey: appKey, mode: mode)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if var session = activeSession, session.app.appKey == appKey {
                session.controlMode = response.mode
                session.statusText = response.mode.displayTitle(locale: locale)
                activeSession = session
            }
            webMcpErrorMessage = nil
            return response
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            webMcpErrorMessage = error.localizedDescription
            return nil
        }
    }

    public func getWebMcpControlState(appKey: String) async -> FabushiMiniAppControlStateResponse? {
        struct Params: Encodable, Sendable {
            let appKey: String
            enum CodingKeys: String, CodingKey { case appKey = "app_key" }
        }
        return try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.webmcp.method("getControlState"),
            params: Params(appKey: appKey)
        )
    }

    public func validateWebMcpToolCall(
        manifestJSON: String,
        action: String,
        args: FabushiJSONValue = .object([:])
    ) async -> Result<String, Error> {
        struct Params: Encodable, Sendable {
            let manifestJson: String
            let action: String
            let args: FabushiJSONValue
            enum CodingKeys: String, CodingKey {
                case manifestJson = "manifest_json"
                case action
                case args
            }
        }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.webmcp.method("validateToolCall"),
                params: Params(manifestJson: manifestJSON, action: action, args: args)
            )
            webMcpErrorMessage = nil
            return .success(action)
        } catch {
            webMcpErrorMessage = error.localizedDescription
            return .failure(error)
        }
    }

    // MARK: - Fail-Closed `.fabapp` Ed25519 Verification (Stratum A Reuse, `BLK-06`)

    internal func verifyOfflinePackage(
        manifestData: Data,
        resources: [String: Data],
        trust: FabAppCatalogTrust,
        hostApi: Int = 1
    ) -> Result<VerifiedFabAppPackage, Error> {
        do {
            let verified = try FabAppPackageVerifier.verifyCatalogPackage(
                manifestData: manifestData,
                resources: resources,
                trust: trust,
                hostApi: hostApi
            )
            isGlobalDharmaPackageVerified = true
            globalDharmaVerificationError = nil
            return .success(verified)
        } catch {
            isGlobalDharmaPackageVerified = false
            globalDharmaVerificationError = error.localizedDescription
            return .failure(error)
        }
    }

    // MARK: - 全球法布施 (Global Dharma) Real Rust State Machine & SQLite RPCs

    public func loadGlobalDharma() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        let userId: Int64
        let username: String
        if case .authenticated(let uid, let uname, _) = coordinator.authState {
            userId = Int64(uid)
            username = uname
        } else {
            userId = 1
            username = "practitioner"
        }

        struct BootstrapParams: Encodable, Sendable {
            let userId: Int64
            let username: String
            let displayName: String
            enum CodingKeys: String, CodingKey {
                case userId = "user_id"
                case username
                case displayName = "display_name"
            }
        }

        if let boot: FabushiJSONValue = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("dharmaBootstrap"),
            params: BootstrapParams(userId: userId, username: username, displayName: username)
        ), coordinator.accountScopeGeneration == capturedGen {
            dharmaBootstrap = boot
        }

        if let snap: FabushiJSONValue = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("dharmaStateSnapshot"),
            params: FabushiEmptyParams()
        ), coordinator.accountScopeGeneration == capturedGen {
            dharmaSnapshot = snap
        }

        struct LeaderboardParams: Encodable, Sendable {
            let userId: Int64
            let limit: Int
            enum CodingKeys: String, CodingKey {
                case userId = "user_id"
                case limit
            }
        }
        if let board: FabushiJSONValue = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("dharmaLeaderboard"),
            params: LeaderboardParams(userId: userId, limit: 20)
        ), coordinator.accountScopeGeneration == capturedGen {
            dharmaLeaderboard = board
        }

        _ = await verifyDharmaAsset(name: "fabushi_miniapp.wasm")
    }

    @discardableResult
    public func dispatchDharmaCommand(
        type: String,
        extraFields: [String: FabushiJSONValue] = [:]
    ) async -> FabushiMiniAppDharmaDispatchResponse? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        var payload = extraFields
        payload["type"] = .string(type)

        do {
            let response: FabushiMiniAppDharmaDispatchResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.miniapp.method("dharmaDispatch"),
                params: FabushiJSONValue.object(["command": .object(payload)])
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            dharmaSnapshot = response.snapshot
            if type == "increment_chant" {
                localChantCount += 1
            } else if type == "start_practice_session" {
                isPracticeSessionActive = true
            } else if type == "end_practice_session" {
                isPracticeSessionActive = false
            }
            return response
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }

    @discardableResult
    public func recordDharmaPractice(
        sutra: String,
        durationMinutes: UInt32 = 10,
        chantCount: UInt32 = 1,
        notes: String? = nil,
        locale: FabushiLocale = .en
    ) async -> FabushiJSONValue? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        let userId: Int64
        if case .authenticated(let uid, _, _) = coordinator.authState {
            userId = Int64(uid)
        } else {
            userId = 1
        }
        struct Params: Encodable, Sendable {
            let userId: Int64
            let sutra: String
            let durationMinutes: UInt32
            let chantCount: UInt32
            let notes: String?
            enum CodingKeys: String, CodingKey {
                case userId = "user_id"
                case sutra
                case durationMinutes = "duration_minutes"
                case chantCount = "chant_count"
                case notes
            }
        }
        do {
            let result: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.miniapp.method("dharmaRecordPractice"),
                params: Params(
                    userId: userId,
                    sutra: sutra,
                    durationMinutes: durationMinutes,
                    chantCount: chantCount,
                    notes: notes
                )
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.miniAppGlobalDharmaRecordPractice, locale: locale),
                isError: false
            )
            await loadGlobalDharma()
            return result
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    @discardableResult
    public func verifyDharmaAsset(name: String) async -> FabushiMiniAppDharmaAssetInfo? {
        struct Params: Encodable, Sendable { let name: String }
        let info: FabushiMiniAppDharmaAssetInfo? = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("dharmaAssetBytes"),
            params: Params(name: name)
        )
        if name == "fabushi_miniapp.wasm", let info {
            dharmaWasmAssetInfo = info
        }
        return info
    }
}
