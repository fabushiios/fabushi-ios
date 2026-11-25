import Foundation
import Observation

/// Generation-fenced ViewModel driving the Built-in Interactive Browser Workspace (`D11-06..D11-09`),
/// `BrowserControlGate` (Take Over / Resume AI / Cancel AI), DOM Recorder + Script Distillation,
/// Isolated Session Profiles + JSON/Netscape Cookie Import + E2EE Vault Backup/Restore (`browser_cookie`),
/// Automation App Store 7-endpoint client + offline cache, Zero-LLM Replay, and `D12-01` focus retention.
@MainActor
@Observable
public final class FabushiBrowserViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let runner: FabushiBrowserWebViewRunner
    public let storeTransport: any FabushiAutomationAppStoreTransport
    public let offlineCache: FabushiAutomationAppOfflineCache

    // Browser Navigation & Focus State
    public var addressBarText: String = "https://fabushi.ombhrum.com"
    public private(set) var currentURLString: String = ""
    public private(set) var pageTitle: String = ""
    public private(set) var canGoBack: Bool = false
    public private(set) var canGoForward: Bool = false
    public private(set) var isPageLoading: Bool = false
    public var selectedInspectorTab: FabushiBrowserInspectorTab = .copilot
    public var isInspectorSheetPresented: Bool = false

    // D11-06 BrowserControlGate State Machine
    public private(set) var controlMode: FabushiBrowserControlMode = .idle
    public private(set) var replayStepProgress: (current: Int, total: Int)? = nil
    public private(set) var replayLogs: [String] = []
    public private(set) var lastReplayReport: FabushiBrowserScriptReplayReport? = nil

    // Recorder & Distilled Script Draft
    public private(set) var isRecording: Bool = false
    public private(set) var currentTrace: FabushiBrowserActionTrace = FabushiBrowserActionTrace()
    public private(set) var distilledDraft: FabushiAutomationScriptApp? = nil
    public private(set) var lastSecretLeakFindings: [FabushiBrowserSecretLeakFinding] = []

    // Session Profiles, Cookies & E2EE Vault Sync (`browser_cookie`)
    public private(set) var profiles: [FabushiBrowserProfileRecord]
    public private(set) var activeProfileId: String
    public private(set) var activeProfileCookies: [FabushiBrowserCookieRecord] = []
    public private(set) var lastVaultBackupBlob: FabushiJSONValue? = nil

    // Automation App Store State
    public var storeSearchQuery: String = ""
    public private(set) var storeSummaries: [FabushiAutomationAppSummary] = []
    public private(set) var installedApps: [FabushiInstalledAutomationApp] = []
    public private(set) var selectedAppDetail: FabushiAutomationAppDetail? = nil
    public private(set) var isStoreLoading: Bool = false

    // Errors & Generation Fencing
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var observedGeneration: UInt64

    @ObservationIgnored private let purgerId = "m5.browser.\(UUID().uuidString)"
    @ObservationIgnored private var gateContinuations: [CheckedContinuation<Bool, Never>] = []

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        storeTransport: (any FabushiAutomationAppStoreTransport)? = nil,
        initialURL: URL? = nil
    ) {
        self.coordinator = coordinator
        self.router = router
        self.observedGeneration = coordinator.accountScopeGeneration

        let defaultProfile = FabushiBrowserProfileRecord(
            profileKey: "default",
            displayName: "Default",
            storeIdentifier: UUID(),
            isEphemeral: false
        )
        let ephemeralProfile = FabushiBrowserProfileRecord(
            profileKey: "ephemeral",
            displayName: "Ephemeral",
            storeIdentifier: nil,
            isEphemeral: true
        )
        self.profiles = [defaultProfile, ephemeralProfile]
        self.activeProfileId = defaultProfile.profileKey
        self.runner = FabushiBrowserWebViewRunner(profile: defaultProfile)

        if let storeTransport {
            self.storeTransport = storeTransport
        } else {
            self.storeTransport = FabushiURLSessionAutomationStoreTransport(
                baseURLProvider: { @Sendable in
                    await MainActor.run { coordinator.settingsStore.activeServerURL }
                }
            )
        }
        self.offlineCache = FabushiAutomationAppOfflineCache(coordinator: coordinator)

        if let initialURL {
            addressBarText = initialURL.absoluteString
        }

        wireRunnerCallbacks()
        installAccountResetHooks()
    }

    private func wireRunnerCallbacks() {
        runner.onNavigationChanged = { [weak self] url, title, back, forward, loading in
            guard let self else { return }
            if let urlStr = url?.absoluteString, !urlStr.isEmpty {
                self.currentURLString = urlStr
                self.addressBarText = urlStr
            }
            if let title, !title.isEmpty {
                self.pageTitle = title
            }
            self.canGoBack = back
            self.canGoForward = forward
            self.isPageLoading = loading
        }

        runner.onNavigationBlocked = { [weak self] raw in
            guard let self else { return }
            let msg = "Blocked unsafe navigation: \(raw)"
            self.lastErrorMessage = msg
            self.router.toastController.showNonChatToast(msg, isError: true)
        }

        runner.onRecordedAction = { [weak self] pageURL, action in
            self?.recordAction(pageURL: pageURL, action: action)
        }
    }

    // MARK: - Generation Fencing & Logout / Account-Switch Wipe

    private func installAccountResetHooks() {
        coordinator.registerAccountSecretsPurger(id: purgerId) { [weak self] _ in
            guard let self else { return }
            self.handleAccountScopeReset(generation: self.coordinator.accountScopeGeneration)
        }
        coordinator.registerServerSwitchSecretsPurger(id: purgerId) { [weak self] in
            guard let self else { return }
            self.handleAccountScopeReset(generation: self.coordinator.accountScopeGeneration)
        }
    }

    public func detachLifecycleHooks() {
        coordinator.unregisterAccountSecretsPurger(id: purgerId)
        coordinator.unregisterServerSwitchSecretsPurger(id: purgerId)
    }

    public func reconcileAccountScopeIfNeeded() {
        let currentGen = coordinator.accountScopeGeneration
        guard currentGen != observedGeneration else { return }
        handleAccountScopeReset(generation: currentGen)
    }

    public func handleAccountScopeReset(generation: UInt64) {
        observedGeneration = generation
        cancelAiControl()
        runner.teardownSession()
        isRecording = false
        currentTrace = FabushiBrowserActionTrace()
        distilledDraft = nil
        lastSecretLeakFindings = []
        activeProfileCookies = []
        lastVaultBackupBlob = nil
        storeSummaries = []
        installedApps = []
        selectedAppDetail = nil
        replayLogs = []
        replayStepProgress = nil
        lastReplayReport = nil
        lastErrorMessage = nil

        Task { @MainActor [offlineCache] in
            await offlineCache.wipeAll()
        }
    }

    // MARK: - Navigation & D12-01 Focus Retention

    @discardableResult
    public func navigateFromAddressBar(locale: FabushiLocale = .en) -> Bool {
        let raw = addressBarText.trimmingCharacters(in: .whitespacesAndNewlines)
        let candidate = (!raw.contains("://") && !raw.isEmpty && !raw.contains(":")) ? "https://\(raw)" : raw
        do {
            try runner.navigate(to: candidate)
            lastErrorMessage = nil
            return true
        } catch {
            let msg = FabushiPlatformStrings.text(.browserInvalidUrlError, locale: locale)
            lastErrorMessage = msg
            router.toastController.showNonChatToast(msg, isError: true)
            return false
        }
    }

    @discardableResult
    public func updateFocusPolicy(
        isViewportPresented: Bool,
        isURLBarFocused: Bool,
        isInspectorModalPresented: Bool,
        hasBlockingModal: Bool = false
    ) -> Bool {
        let context = FabushiWebViewFocusContext(
            isPresented: isViewportPresented,
            isMinimizedToPiP: false,
            isBackgroundWebMcp: false,
            hasBlockingModal: hasBlockingModal || isInspectorModalPresented || router.isPasscodeLocked,
            isHostInputFocused: isURLBarFocused
        )
        runner.enforceFocusPolicy(context: context)
        return FabushiWebViewFocusPolicy.shouldWebViewOwnKeyboard(context)
    }

    // MARK: - D11-06 BrowserControlGate State Machine

    public func beginAiControl(step: Int = 1, statusText: String = "Running automation") {
        controlMode = .aiControlling(activeStep: step, statusText: statusText)
    }

    public func requestUserTakeover(reason: String) {
        controlMode = .waitingForTakeover(reason: reason)
    }

    public func takeOverByUser() {
        let pausedStep: Int
        switch controlMode {
        case .aiControlling(let step, _):
            pausedStep = step
        case .waitingForTakeover:
            pausedStep = replayStepProgress?.current ?? 1
        case .userTakeover(let step):
            pausedStep = step
        case .idle, .cancelled:
            pausedStep = 0
        }
        controlMode = .userTakeover(pausedAtStep: pausedStep)
    }

    public func resumeAiControl(statusText: String = "Resuming AI") {
        let step: Int
        switch controlMode {
        case .userTakeover(let pausedStep):
            step = max(1, pausedStep)
        case .waitingForTakeover:
            step = replayStepProgress?.current ?? 1
        default:
            step = 1
        }
        controlMode = .aiControlling(activeStep: step, statusText: statusText)
        let waiters = gateContinuations
        gateContinuations.removeAll()
        for cont in waiters {
            cont.resume(returning: true)
        }
    }

    public func cancelAiControl() {
        controlMode = .cancelled
        let waiters = gateContinuations
        gateContinuations.removeAll()
        for cont in waiters {
            cont.resume(returning: false)
        }
    }

    public func resetControlGateToIdle() {
        controlMode = .idle
    }

    public func awaitGateProceed() async -> Bool {
        switch controlMode {
        case .cancelled:
            return false
        case .waitingForTakeover, .userTakeover:
            return await withCheckedContinuation { continuation in
                gateContinuations.append(continuation)
            }
        case .idle, .aiControlling:
            return true
        }
    }

    // MARK: - DOM Recorder & Secret-Free Script Distillation

    public func startRecording() {
        isRecording = true
        currentTrace = FabushiBrowserActionTrace(initialUrl: currentURLString.isEmpty ? nil : currentURLString)
        runner.setRecording(true)
    }

    public func stopRecording() {
        isRecording = false
        currentTrace.completedAtMs = Int64(Date().timeIntervalSince1970 * 1000)
        runner.setRecording(false)
    }

    public func clearTrace() {
        currentTrace = FabushiBrowserActionTrace(initialUrl: currentURLString.isEmpty ? nil : currentURLString)
        distilledDraft = nil
        lastSecretLeakFindings = []
    }

    public func recordAction(pageURL: String?, action: FabushiRecordedActionKind) {
        let nextSeq = currentTrace.actions.count + 1
        currentTrace.actions.append(
            FabushiRecordedBrowserAction(
                seq: nextSeq,
                pageUrl: pageURL,
                action: action
            )
        )
    }

    /// Compiles `currentTrace` into an `AutomationScriptApp` (`schema_version = 1`) and runs
    /// the fail-closed `validateScriptAppSafeForPublish` secret-leak scanner before returning.
    public func distillTraceToScript(
        appId: String? = nil,
        slug: String = "recorded-workflow",
        name: String = "Recorded Browser Workflow",
        description: String = "Distilled from Fabushi iOS interactive browser trace",
        category: String = "automation"
    ) throws -> FabushiAutomationScriptApp {
        var steps: [FabushiScriptStep] = []
        var parameters: [FabushiAutomationAppParameterDto] = []
        var seenParamNames = Set<String>()
        var domains = Set<String>()

        if let initial = currentTrace.initialUrl,
           let url = FabushiBrowserSecurity.validateWKNavigationURL(initial)
        {
            if let host = url.host { domains.insert(host) }
            if case .navigate = currentTrace.actions.first?.action {
                // First action already navigates
            } else {
                steps.append(.navigate(urlTemplate: initial))
            }
        }

        for entry in currentTrace.actions {
            if let pageUrl = entry.pageUrl,
               let parsed = URL(string: pageUrl),
               let host = parsed.host
            {
                domains.insert(host)
            }
            switch entry.action {
            case .navigate(let url):
                if let host = URL(string: url)?.host { domains.insert(host) }
                steps.append(.navigate(urlTemplate: url))
            case .click(let locator):
                steps.append(.click(locator: locator))
            case .fill(let locator, let value, let isSecret, let hint):
                if isSecret {
                    let rawName = (hint ?? locator.nameAttr ?? "secret_input_\(entry.seq)")
                        .lowercased()
                        .replacingOccurrences(of: #"[^a-z0-9_]"#, with: "_", options: .regularExpression)
                    let paramName = rawName.isEmpty ? "secret_input_\(entry.seq)" : rawName
                    if seenParamNames.insert(paramName).inserted {
                        parameters.append(
                            FabushiAutomationAppParameterDto(
                                name: paramName,
                                description: "Vault-backed secret input",
                                required: true,
                                defaultValue: nil,
                                isSecret: true
                            )
                        )
                    }
                    steps.append(
                        .fill(
                            locator: locator,
                            valueTemplate: "{{\(paramName)}}",
                            secretVaultKey: paramName
                        )
                    )
                } else {
                    steps.append(
                        .fill(
                            locator: locator,
                            valueTemplate: value,
                            secretVaultKey: nil
                        )
                    )
                }
            case .pressKey(let key):
                steps.append(.pressKey(key: key))
            case .scroll(let dx, let dy):
                steps.append(.scroll(deltaX: dx, deltaY: dy))
            }
        }

        let resolvedAppId = appId ?? "app_\(UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(12))"
        let draft = FabushiAutomationScriptApp(
            schemaVersion: FabushiAutomationScriptApp.schemaVersionV1,
            appId: resolvedAppId,
            slug: slug,
            name: name,
            version: "1.0.0",
            description: description,
            category: category,
            targetDomains: Array(domains).sorted(),
            parameters: parameters,
            steps: steps,
            requiredCapabilities: ["browser.wkwebview"],
            sourceTraceSummary: "Distilled \(currentTrace.actions.count) actions"
        )

        do {
            try FabushiBrowserSecurity.validateScriptAppSafeForPublish(draft)
            lastSecretLeakFindings = []
            distilledDraft = draft
            return draft
        } catch let preflightError as FabushiBrowserPreflightError {
            if case .secretLeakDetected(let findings) = preflightError {
                lastSecretLeakFindings = findings
            }
            throw preflightError
        }
    }

    // MARK: - Isolated Profiles, Cookie Import/Export & E2EE Vault Sync (`browser_cookie`)

    public func createNamedProfile(displayName: String) -> FabushiBrowserProfileRecord {
        let cleanName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = cleanName.isEmpty ? "Profile \(profiles.count + 1)" : cleanName
        let slug = name.lowercased().replacingOccurrences(of: #"[^a-z0-9_-]"#, with: "-", options: .regularExpression)
        let record = FabushiBrowserProfileRecord(
            profileKey: "\(slug)-\(UUID().uuidString.prefix(6))",
            displayName: name,
            storeIdentifier: UUID(),
            isEphemeral: false
        )
        profiles.append(record)
        selectProfile(record)
        return record
    }

    public func selectProfile(_ profile: FabushiBrowserProfileRecord) {
        activeProfileId = profile.profileKey
        runner.switchProfile(profile)
        Task { @MainActor in
            await refreshCookiesFromWebView()
        }
    }

    public func refreshCookiesFromWebView() async {
        activeProfileCookies = await runner.exportCookies()
    }

    @discardableResult
    public func importCookies(from rawText: String, locale: FabushiLocale = .en) async throws -> Int {
        let parsed = try FabushiBrowserCookieCodec.parseCookies(from: rawText)
        let count = await runner.importCookies(parsed)
        activeProfileCookies = await runner.exportCookies()
        if activeProfileCookies.isEmpty {
            activeProfileCookies = parsed
        }
        router.toastController.showNonChatToast(
            FabushiPlatformStrings.cookiesImportedSummary(count: count, locale: locale),
            isError: false
        )
        return count
    }

    public func clearActiveProfileData(locale: FabushiLocale = .en) async {
        await runner.clearWebsiteData()
        activeProfileCookies = []
        router.toastController.showNonChatToast(
            FabushiPlatformStrings.text(.browserClearCookiesAction, locale: locale),
            isError: false
        )
    }

    private func resolvedVaultUserId(overrideUserId: UInt64?) -> UInt64 {
        if let overrideUserId {
            return overrideUserId
        }
        if case .authenticated(let uid, _, _) = coordinator.authState {
            return uid
        }
        return 1
    }

    /// Seals the active profile's cookies into the unlocked M2 Zero-Knowledge Credential Vault
    /// using EXACT `category: "browser_cookie"` and enforcing the `<= 512 KiB` bundle cap.
    /// Fails closed when the Vault is locked!
    @discardableResult
    public func backupActiveProfileToVault(
        userId overrideUserId: UInt64? = nil,
        cookiesOverride: [FabushiBrowserCookieRecord]? = nil,
        pushToServer: Bool = false,
        locale: FabushiLocale = .en
    ) async throws -> FabushiJSONValue {
        reconcileAccountScopeIfNeeded()
        let cookiesToSeal: [FabushiBrowserCookieRecord]
        if let cookiesOverride {
            cookiesToSeal = cookiesOverride
        } else {
            let live = await runner.exportCookies()
            cookiesToSeal = live.isEmpty ? activeProfileCookies : live
        }

        let plaintext = try FabushiBrowserCookieCodec.encodeVaultBundle(
            profileKey: activeProfileId,
            cookies: cookiesToSeal
        )

        let uid = resolvedVaultUserId(overrideUserId: overrideUserId)
        let itemKey = "browser_profile:\(activeProfileId)"

        struct SealParams: Encodable, Sendable {
            let userId: UInt64
            let category: String
            let itemKey: String
            let revision: Int64
            let plaintext: String
            enum CodingKeys: String, CodingKey {
                case userId = "user_id"
                case category
                case itemKey = "item_key"
                case revision
                case plaintext
            }
        }

        do {
            let sealedBlob: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.vault.method("sealItem"),
                params: SealParams(
                    userId: uid,
                    category: FabushiBrowserSecurity.vaultBrowserCookieCategory,
                    itemKey: itemKey,
                    revision: 1,
                    plaintext: plaintext
                )
            )
            lastVaultBackupBlob = sealedBlob

            if pushToServer {
                struct PushParams: Encodable, Sendable {
                    let category: String
                    let itemKey: String
                    let item: FabushiJSONValue
                    enum CodingKeys: String, CodingKey {
                        case category
                        case itemKey = "item_key"
                        case item
                    }
                }
                _ = try? await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.vault.method("syncPush"),
                    params: PushParams(
                        category: FabushiBrowserSecurity.vaultBrowserCookieCategory,
                        itemKey: itemKey,
                        item: sealedBlob
                    )
                ) as FabushiJSONValue
            }

            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.browserVaultBackupAction, locale: locale),
                isError: false
            )
            return sealedBlob
        } catch let hostErr as FabushiHostError where hostErr.code == "unauthorized" {
            let msg = FabushiPlatformStrings.text(.browserVaultLockedError, locale: locale)
            lastErrorMessage = msg
            router.toastController.showNonChatToast(msg, isError: true)
            throw hostErr
        }
    }

    /// Decrypts a sealed `"browser_cookie"` Vault blob via real `vault.openItem` and imports
    /// its cookies into the active profile's isolated `WKHTTPCookieStore`.
    @discardableResult
    public func restoreProfileFromVaultBlob(
        _ blob: FabushiJSONValue,
        userId overrideUserId: UInt64? = nil,
        locale: FabushiLocale = .en
    ) async throws -> [FabushiBrowserCookieRecord] {
        reconcileAccountScopeIfNeeded()
        let uid = resolvedVaultUserId(overrideUserId: overrideUserId)

        struct OpenParams: Encodable, Sendable {
            let userId: UInt64
            let blob: FabushiJSONValue
            enum CodingKeys: String, CodingKey {
                case userId = "user_id"
                case blob
            }
        }
        struct OpenResponse: Decodable, Sendable {
            let plaintextUtf8: String?
            let plaintextB64: String
            enum CodingKeys: String, CodingKey {
                case plaintextUtf8 = "plaintext_utf8"
                case plaintextB64 = "plaintext_b64"
            }
        }

        let opened: OpenResponse = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.vault.method("openItem"),
            params: OpenParams(userId: uid, blob: blob)
        )
        let plaintext: String
        if let utf8 = opened.plaintextUtf8 {
            plaintext = utf8
        } else if let data = Data(base64Encoded: opened.plaintextB64) {
            plaintext = String(decoding: data, as: UTF8.self)
        } else {
            throw FabushiBrowserPreflightError.invalidCookiePayload("Unable to decode opened Vault profile payload.")
        }

        let bundle = try FabushiBrowserCookieCodec.decodeVaultBundle(plaintext)
        _ = await runner.importCookies(bundle.cookies)
        activeProfileCookies = bundle.cookies
        router.toastController.showNonChatToast(
            FabushiPlatformStrings.text(.browserVaultRestoreAction, locale: locale),
            isError: false
        )
        return bundle.cookies
    }

    // MARK: - Automation App Store (7 Real REST Operations + Fail-Closed Publish + Offline Cache)

    public func searchStore(category: String? = nil, domain: String? = nil) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        isStoreLoading = true
        defer { isStoreLoading = false }

        do {
            let response = try await storeTransport.searchApps(
                query: FabushiListAutomationAppsQuery(
                    q: storeSearchQuery.isEmpty ? nil : storeSearchQuery,
                    category: category,
                    domain: domain
                )
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            storeSummaries = response.apps
            await offlineCache.saveCatalogSummaries(response.apps)
            lastErrorMessage = nil
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            let cached = await offlineCache.loadCatalogSummaries()
            if !cached.isEmpty {
                storeSummaries = cached
            }
            lastErrorMessage = error.localizedDescription
        }
    }

    public func refreshInstalledApps() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let response = try await storeTransport.listInstalledApps()
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            installedApps = response.apps
            await offlineCache.saveInstalledApps(response.apps)
            lastErrorMessage = nil
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            installedApps = await offlineCache.loadInstalledApps()
            lastErrorMessage = error.localizedDescription
        }
    }

    @discardableResult
    public func loadAppDetail(appId: String) async -> FabushiAutomationAppDetail? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let detail = try await storeTransport.getAppDetail(appId: appId)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            selectedAppDetail = detail
            return detail
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    @discardableResult
    public func installApp(appId: String, locale: FabushiLocale = .en) async -> FabushiInstalledAutomationApp? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let response = try await storeTransport.installApp(appId: appId)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if let idx = installedApps.firstIndex(where: { $0.appId == response.app.appId }) {
                installedApps[idx] = response.app
            } else {
                installedApps.append(response.app)
            }
            await offlineCache.saveInstalledApps(installedApps)
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.installed, locale: locale),
                isError: false
            )
            return response.app
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    @discardableResult
    public func uninstallApp(appId: String, locale: FabushiLocale = .en) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let response = try await storeTransport.uninstallApp(appId: appId)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            if response.ok {
                installedApps.removeAll { $0.appId == appId }
                await offlineCache.saveInstalledApps(installedApps)
            }
            return response.ok
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return false
        }
    }

    /// Publishes an `AutomationScriptApp` ONLY after passing fail-closed `validateScriptAppSafeForPublish`.
    @discardableResult
    public func publishScriptApp(
        _ app: FabushiAutomationScriptApp,
        changelog: String? = nil,
        isPublished: Bool = true,
        locale: FabushiLocale = .en
    ) async throws -> FabushiAutomationAppDetail {
        reconcileAccountScopeIfNeeded()
        do {
            try FabushiBrowserSecurity.validateScriptAppSafeForPublish(app)
            lastSecretLeakFindings = []
        } catch let preflightErr as FabushiBrowserPreflightError {
            if case .secretLeakDetected(let findings) = preflightErr {
                lastSecretLeakFindings = findings
            }
            let msg = "\(FabushiPlatformStrings.text(.browserStoreSecretLeakBlocked, locale: locale)): \(preflightErr.localizedDescription)"
            lastErrorMessage = msg
            router.toastController.showNonChatToast(msg, isError: true)
            throw preflightErr
        }

        let manifestJSON = try app.toJSONValue()
        let detail = try await storeTransport.publishApp(
            request: FabushiPublishAutomationAppRequest(
                manifest: manifestJSON,
                isPublished: isPublished,
                changelog: changelog
            )
        )
        selectedAppDetail = detail
        router.toastController.showNonChatToast(
            FabushiPlatformStrings.text(.browserStorePublishSuccess, locale: locale),
            isError: false
        )
        return detail
    }

    // MARK: - Zero-LLM Deterministic Replay (Honoring BrowserControlGate & Secret Redaction)

    /// Executes an `AutomationScriptApp` deterministically with **zero LLM tokens**,
    /// honoring `BrowserControlGate` pause/resume/cancel, resolving secret parameters without
    /// echoing their values into UI logs, and reporting telemetry via `recordRun`.
    @discardableResult
    public func runScriptApp(
        _ app: FabushiAutomationScriptApp,
        parameterValues: [String: String] = [:],
        vaultSecretResolver: (@MainActor (String) async -> String?)? = nil,
        stepExecutorOverride: (@MainActor (FabushiScriptStep, [String: String], inout [String: String]) async throws -> Void)? = nil,
        recordTelemetry: Bool = true,
        locale: FabushiLocale = .en
    ) async -> FabushiBrowserScriptReplayReport {
        reconcileAccountScopeIfNeeded()
        let startTime = Date()
        replayLogs = []
        lastReplayReport = nil

        var resolvedParams = parameterValues
        let secretParamNames = Set(app.parameters.filter(\.isSecret).map(\.name))

        for param in app.parameters {
            if param.isSecret {
                if resolvedParams[param.name] == nil, let resolver = vaultSecretResolver {
                    if let resolved = await resolver(param.name) {
                        resolvedParams[param.name] = resolved
                    }
                }
            } else if resolvedParams[param.name] == nil, let defaultVal = param.defaultValue {
                resolvedParams[param.name] = defaultVal
            }
            if param.required && (resolvedParams[param.name]?.isEmpty ?? true) {
                let errMsg = FabushiBrowserRunnerError.missingRequiredParameter(param.name).localizedDescription
                let report = FabushiBrowserScriptReplayReport(
                    appId: app.appId,
                    version: app.version,
                    success: false,
                    stepsExecuted: 0,
                    totalDurationMs: 0,
                    usedLlm: false,
                    error: errMsg
                )
                lastReplayReport = report
                replayLogs.append("Error: \(errMsg)")
                return report
            }
        }

        var extractedOutputs: [String: String] = [:]
        var executedCount = 0
        var finalError: String? = nil

        beginAiControl(step: 1, statusText: app.name)

        for (idx, step) in app.steps.enumerated() {
            let stepNum = idx + 1
            replayStepProgress = (stepNum, app.steps.count)

            if !(await awaitGateProceed()) {
                finalError = "Cancelled by user"
                break
            }
            controlMode = .aiControlling(activeStep: stepNum, statusText: app.name)

            // Log step execution NEVER echoing secret parameter values!
            replayLogs.append(redactedStepLog(step: step, index: stepNum, secretParamNames: secretParamNames))

            do {
                if let customExec = stepExecutorOverride {
                    try await customExec(step, resolvedParams, &extractedOutputs)
                } else {
                    try await runner.executeStep(
                        step,
                        resolvedParams: resolvedParams,
                        extractedOutputs: &extractedOutputs
                    )
                }
                executedCount += 1
            } catch let runnerErr as FabushiBrowserRunnerError {
                if case .humanCheckpointPaused(let reason) = runnerErr {
                    requestUserTakeover(reason: reason)
                    if await awaitGateProceed() {
                        executedCount += 1
                        continue
                    } else {
                        finalError = "Cancelled at checkpoint"
                        break
                    }
                }
                finalError = runnerErr.localizedDescription
                break
            } catch {
                finalError = error.localizedDescription
                break
            }
        }

        let elapsedMs = UInt64(max(0, Date().timeIntervalSince(startTime) * 1000.0))
        let success = finalError == nil && controlMode != .cancelled
        if controlMode != .cancelled {
            resetControlGateToIdle()
        }
        replayStepProgress = nil

        let report = FabushiBrowserScriptReplayReport(
            appId: app.appId,
            version: app.version,
            success: success,
            stepsExecuted: executedCount,
            totalDurationMs: elapsedMs,
            usedLlm: false,
            extractedOutputs: extractedOutputs,
            error: finalError
        )
        lastReplayReport = report

        if recordTelemetry && FabushiBrowserSecurity.validateAutomationAppID(app.appId) {
            _ = try? await storeTransport.recordRun(
                appId: app.appId,
                request: FabushiRecordAutomationRunRequest(
                    version: app.version,
                    success: success,
                    stepsExecuted: executedCount,
                    totalDurationMs: elapsedMs,
                    usedLlm: false,
                    error: finalError
                )
            )
        }

        router.toastController.showNonChatToast(
            success
                ? FabushiPlatformStrings.text(.browserStoreReplayComplete, locale: locale)
                : (finalError ?? "Replay failed"),
            isError: !success
        )
        return report
    }

    private func redactedStepLog(
        step: FabushiScriptStep,
        index: Int,
        secretParamNames: Set<String>
    ) -> String {
        switch step {
        case .navigate(let url):
            return "#\(index) navigate(\(url))"
        case .waitFor(let locator, let ms):
            return "#\(index) wait_for(\(locator.highestPriorityStrategy), \(ms)ms)"
        case .click(let locator):
            return "#\(index) click(\(locator.highestPriorityStrategy))"
        case .fill(let locator, let template, let vaultKey):
            let referencesSecret = vaultKey != nil || secretParamNames.contains { template.contains("{{\($0)}}") }
            return "#\(index) fill(\(locator.highestPriorityStrategy)) = \(referencesSecret ? "[REDACTED]" : template)"
        case .pressKey(let key):
            return "#\(index) press_key(\(key))"
        case .scroll(let dx, let dy):
            return "#\(index) scroll(\(Int(dx)), \(Int(dy)))"
        case .extractText(let locator, let outputKey):
            return "#\(index) extract_text(\(locator.highestPriorityStrategy) -> \(outputKey))"
        case .assertVisible(let locator):
            return "#\(index) assert_visible(\(locator.highestPriorityStrategy))"
        case .assertTextContains(let locator, _):
            return "#\(index) assert_text_contains(\(locator.highestPriorityStrategy))"
        case .humanCheckpoint(let reason):
            return "#\(index) human_checkpoint(\(reason))"
        }
    }
}
