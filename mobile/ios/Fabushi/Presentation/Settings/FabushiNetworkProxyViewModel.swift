import Foundation
import Observation

/// Generation-fenced ViewModel driving Network, Server URL (`coordinator.setServerURL`),
/// and Strict Proxy Management (`D10-13`):
/// - Monotonic `proxyOpGeneration` incremented on EVERY proxy select/add/edit/delete/probe/disable op
/// - Discards stale `proxy.list` / `proxy.probeHealth` results carrying older `proxyOpGeneration` OR older `accountScopeGeneration`
/// - Pre-mutation `FabushiProxyValidator.validate(...)` BEFORE mutating UI or calling Rust RPCs
/// - Strict rollback to previous active proxy + `router.toastController.showNonChatToast(...)` on error/NotFound.
@MainActor
@Observable
public final class FabushiNetworkProxyViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    // Server URL state
    public var serverUrlDraft: String
    public private(set) var activeServerUrl: String

    // Proxy list & anti-divergence state
    public private(set) var proxies: [FabushiProxyRecord] = []
    public private(set) var activeProxyId: String? = nil
    public private(set) var proxyOpGeneration: UInt64 = 0
    public private(set) var probeStatusText: String = ""
    public private(set) var lastProbeLatencyMs: UInt32? = nil
    public private(set) var isProbing: Bool = false
    public private(set) var isLoading: Bool = false
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var observedAccountGeneration: UInt64

    // Form Drafts
    public var draftKind: FabushiProxyKind = .socks5
    public var draftHost: String = ""
    public var draftPort: String = "1080"
    public var draftUsername: String = ""
    public var draftPassword: String = ""
    public var draftMtprotoSecret: String = ""
    public var importLinkDraft: String = ""

    @ObservationIgnored private let purgerId = "m5.proxy.\(UUID().uuidString)"
    @ObservationIgnored private static let mtprotoStorageKey = "m5.settings.mtproto_proxies"

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        self.coordinator = coordinator
        self.router = router
        self.observedAccountGeneration = coordinator.accountScopeGeneration
        self.activeServerUrl = coordinator.settingsStore.activeServerURL
        self.serverUrlDraft = coordinator.settingsStore.activeServerURL
        installAccountResetHooks()
    }

    // MARK: - Lifecycle & Account-Switch Fencing

    private func installAccountResetHooks() {
        coordinator.registerAccountSecretsPurger(id: purgerId) { [weak self] _ in
            self?.resetForScopeChange()
        }
        coordinator.registerServerSwitchSecretsPurger(id: purgerId) { [weak self] in
            self?.resetForScopeChange()
        }
    }

    public func detachLifecycleHooks() {
        coordinator.unregisterAccountSecretsPurger(id: purgerId)
        coordinator.unregisterServerSwitchSecretsPurger(id: purgerId)
    }

    public func reconcileAccountScopeIfNeeded() {
        let current = coordinator.accountScopeGeneration
        guard current != observedAccountGeneration else { return }
        resetForScopeChange()
    }

    private func resetForScopeChange() {
        observedAccountGeneration = coordinator.accountScopeGeneration
        proxyOpGeneration &+= 1
        activeServerUrl = coordinator.settingsStore.activeServerURL
        serverUrlDraft = activeServerUrl
        proxies = []
        activeProxyId = nil
        lastProbeLatencyMs = nil
        probeStatusText = ""
        isProbing = false
        isLoading = false
        lastErrorMessage = nil
    }

    @discardableResult
    public func bumpProxyOpGeneration() -> UInt64 {
        proxyOpGeneration &+= 1
        return proxyOpGeneration
    }

    /// Applies an externally or asynchronously fetched proxy list ONLY if BOTH
    /// `proxyOpGeneration` and `accountScopeGeneration` still match.
    @discardableResult
    public func applyFetchedProxyTruth(
        records: [FabushiProxyRecord],
        activeId: String?,
        capturedOpGeneration: UInt64,
        capturedAccountGeneration: UInt64
    ) -> Bool {
        guard capturedOpGeneration == proxyOpGeneration,
              capturedAccountGeneration == coordinator.accountScopeGeneration
        else {
            return false
        }
        proxies = records
        activeProxyId = activeId
        return true
    }

    /// Applies an asynchronous health probe result ONLY if BOTH `proxyOpGeneration` and
    /// `accountScopeGeneration` still match.
    @discardableResult
    public func applyProbeResult(
        latencyMs: UInt32?,
        statusMessage: String,
        capturedOpGeneration: UInt64,
        capturedAccountGeneration: UInt64
    ) -> Bool {
        guard capturedOpGeneration == proxyOpGeneration,
              capturedAccountGeneration == coordinator.accountScopeGeneration
        else {
            return false
        }
        lastProbeLatencyMs = latencyMs
        probeStatusText = statusMessage
        isProbing = false
        return true
    }

    // MARK: - Editable Server URL (`FabushiLaunchConfiguration.validateAndNormalizeServerURL` + `coordinator.setServerURL`)

    @discardableResult
    public func applyServerURL(locale: FabushiLocale = .en) async -> Bool {
        reconcileAccountScopeIfNeeded()
        guard let normalized = FabushiLaunchConfiguration.validateAndNormalizeServerURL(serverUrlDraft) else {
            let msg = FabushiPlatformStrings.text(.settingsServerUrlInvalidToast, locale: locale)
            lastErrorMessage = msg
            router.toastController.showNonChatToast(msg, isError: true)
            return false
        }
        do {
            _ = try await coordinator.setServerURL(normalized)
            activeServerUrl = coordinator.settingsStore.activeServerURL
            serverUrlDraft = activeServerUrl
            observedAccountGeneration = coordinator.accountScopeGeneration
            lastErrorMessage = nil
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.settingsServerUrlUpdatedToast, locale: locale),
                isError: false
            )
            return true
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return false
        }
    }

    // MARK: - Load & Refresh (`proxy.list` + local MTProto profiles)

    public func refresh(locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        let opGen = bumpProxyOpGeneration()
        let accountGen = coordinator.accountScopeGeneration
        isLoading = true
        defer {
            if opGen == proxyOpGeneration {
                isLoading = false
            }
        }

        activeServerUrl = coordinator.settingsStore.activeServerURL
        if serverUrlDraft.isEmpty {
            serverUrlDraft = activeServerUrl
        }

        do {
            let rawList: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.proxy.method("list"),
                params: FabushiEmptyParams()
            )
            let activeId = rawList["active_proxy_id"]?.stringValue
            var parsed = Self.decodeProxyList(rawList)
            let mtprotoItems = await loadLocalMtprotoProxies()
            parsed.append(contentsOf: mtprotoItems)

            _ = applyFetchedProxyTruth(
                records: parsed,
                activeId: activeId,
                capturedOpGeneration: opGen,
                capturedAccountGeneration: accountGen
            )
            if activeId == nil && probeStatusText.isEmpty {
                probeStatusText = FabushiPlatformStrings.text(.settingsProxyStatusDirect, locale: locale)
            }
            lastErrorMessage = nil
        } catch {
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else { return }
            lastErrorMessage = error.localizedDescription
        }
    }

    // MARK: - Save / Upsert Proxy (Strict Pre-Mutation Validation)

    @discardableResult
    public func saveDraftProxy(activate: Bool = true, locale: FabushiLocale = .en) async -> FabushiProxyRecord? {
        let portInt = Int(draftPort.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
        let candidate = FabushiProxyRecord(
            kind: draftKind,
            host: draftHost,
            port: UInt16(clamping: max(0, portInt)),
            username: draftUsername.isEmpty ? nil : draftUsername,
            password: draftPassword.isEmpty ? nil : draftPassword,
            mtprotoSecret: draftMtprotoSecret
        )
        if draftKind != .direct && !(1...65535).contains(portInt) {
            let err = FabushiProxyValidationError.invalidPort(portInt)
            lastErrorMessage = err.localizedDescription
            router.toastController.showNonChatToast(err.localizedDescription, isError: true)
            return nil
        }
        return await upsertProxy(candidate, activate: activate, locale: locale)
    }

    @discardableResult
    public func upsertProxy(
        _ rawRecord: FabushiProxyRecord,
        activate: Bool = false,
        locale: FabushiLocale = .en
    ) async -> FabushiProxyRecord? {
        reconcileAccountScopeIfNeeded()

        // D10-13: Strict pre-mutation validation BEFORE mutating UI or invoking Rust RPCs!
        let validated: FabushiProxyRecord
        do {
            validated = try FabushiProxyValidator.validate(rawRecord)
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }

        let opGen = bumpProxyOpGeneration()
        let accountGen = coordinator.accountScopeGeneration
        let previousProxies = proxies
        let previousActiveId = activeProxyId

        if validated.kind == .mtproto {
            var localMt = await loadLocalMtprotoProxies()
            localMt.removeAll { $0.id == validated.id }
            var stored = validated
            stored.active = false
            localMt.append(stored)
            await saveLocalMtprotoProxies(localMt)
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else {
                return nil
            }
            proxies.removeAll { $0.id == stored.id }
            proxies.append(stored)
            lastErrorMessage = nil
            return stored
        }

        struct ProxyUpsertParams: Encodable, Sendable {
            let id: String
            let proxy_type: String
            let host: String
            let port: UInt16
            let username: String?
            let password: String?
            let fallback_to_direct: Bool
            let activate: Bool
        }

        do {
            let response: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.proxy.method("upsert"),
                params: ProxyUpsertParams(
                    id: validated.id,
                    proxy_type: validated.kind.rawValue, // Exact PascalCase: "Direct" | "Http" | "Socks5"
                    host: validated.host,
                    port: validated.port,
                    username: validated.username,
                    password: validated.password,
                    fallback_to_direct: validated.fallbackToDirect,
                    activate: activate
                )
            )
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else {
                return nil
            }
            let persistedId = response["id"]?.stringValue ?? validated.id
            let saved = FabushiProxyRecord(
                id: persistedId,
                kind: validated.kind,
                host: validated.host,
                port: validated.port,
                username: validated.username,
                password: validated.password,
                mtprotoSecret: "",
                fallbackToDirect: validated.fallbackToDirect,
                active: activate
            )
            proxies.removeAll { $0.id == saved.id }
            if activate {
                proxies = proxies.map { item in
                    var copy = item
                    copy.active = false
                    return copy
                }
                activeProxyId = saved.id
            }
            proxies.append(saved)
            lastErrorMessage = nil
            return saved
        } catch {
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else {
                return nil
            }
            // Strict rollback on failure!
            proxies = previousProxies
            activeProxyId = previousActiveId
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    // MARK: - Select Active Proxy (Strict Optimistic Rollback + `proxyOpGeneration`)

    @discardableResult
    public func selectActiveProxy(id: String?, locale: FabushiLocale = .en) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let opGen = bumpProxyOpGeneration()
        let accountGen = coordinator.accountScopeGeneration

        let previousActiveId = activeProxyId
        let previousProxies = proxies

        // Optimistic update
        activeProxyId = id
        proxies = proxies.map { item in
            var copy = item
            copy.active = (id != nil && item.id == id)
            return copy
        }

        struct SetActiveParams: Encodable, Sendable {
            let id: String?
        }

        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.proxy.method("setActive"),
                params: SetActiveParams(id: id)
            )
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else {
                return false
            }
            if id == nil {
                probeStatusText = FabushiPlatformStrings.text(.settingsProxyStatusDirect, locale: locale)
                lastProbeLatencyMs = nil
            }
            lastErrorMessage = nil
            return true
        } catch {
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else {
                return false
            }
            // Strict rollback to previous active proxy + non-chat error toast!
            activeProxyId = previousActiveId
            proxies = previousProxies
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return false
        }
    }

    // MARK: - Delete Proxy (`proxy.delete`)

    @discardableResult
    public func deleteProxy(id: String, locale: FabushiLocale = .en) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let opGen = bumpProxyOpGeneration()
        let accountGen = coordinator.accountScopeGeneration

        if let existing = proxies.first(where: { $0.id == id }), existing.kind == .mtproto {
            var localMt = await loadLocalMtprotoProxies()
            localMt.removeAll { $0.id == id }
            await saveLocalMtprotoProxies(localMt)
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else {
                return false
            }
            proxies.removeAll { $0.id == id }
            return true
        }

        struct DeleteParams: Encodable, Sendable {
            let id: String
        }

        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.proxy.method("delete"),
                params: DeleteParams(id: id)
            )
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else {
                return false
            }
            proxies.removeAll { $0.id == id }
            if activeProxyId == id {
                activeProxyId = nil
                probeStatusText = FabushiPlatformStrings.text(.settingsProxyStatusDirect, locale: locale)
            }
            lastErrorMessage = nil
            return true
        } catch {
            guard opGen == proxyOpGeneration, accountGen == coordinator.accountScopeGeneration else {
                return false
            }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return false
        }
    }

    // MARK: - Probe Proxy Health (`proxy.probeHealth`)

    @discardableResult
    public func probeDraftProxy(locale: FabushiLocale = .en) async -> UInt32? {
        let portInt = Int(draftPort.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
        if draftKind != .direct && !(1...65535).contains(portInt) {
            let err = FabushiProxyValidationError.invalidPort(portInt)
            lastErrorMessage = err.localizedDescription
            probeStatusText = err.localizedDescription
            router.toastController.showNonChatToast(err.localizedDescription, isError: true)
            return nil
        }
        let candidate = FabushiProxyRecord(
            kind: draftKind,
            host: draftHost,
            port: UInt16(clamping: max(0, portInt)),
            username: draftUsername.isEmpty ? nil : draftUsername,
            password: draftPassword.isEmpty ? nil : draftPassword,
            mtprotoSecret: draftMtprotoSecret
        )
        return await probeProxy(candidate, locale: locale)
    }

    @discardableResult
    public func probeProxy(_ rawRecord: FabushiProxyRecord, locale: FabushiLocale = .en) async -> UInt32? {
        reconcileAccountScopeIfNeeded()

        let validated: FabushiProxyRecord
        do {
            validated = try FabushiProxyValidator.validate(rawRecord)
        } catch {
            lastErrorMessage = error.localizedDescription
            probeStatusText = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }

        let opGen = bumpProxyOpGeneration()
        let accountGen = coordinator.accountScopeGeneration
        isProbing = true
        probeStatusText = FabushiPlatformStrings.text(.settingsProxyStatusProbing, locale: locale)

        struct ProbeParams: Encodable, Sendable {
            let proxy_type: String
            let host: String
            let port: UInt16
            let username: String?
            let password: String?
            let fallback_to_direct: Bool
        }

        let probeType = validated.kind == .mtproto ? FabushiProxyKind.socks5.rawValue : validated.kind.rawValue
        do {
            let res: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.proxy.method("probeHealth"),
                params: ProbeParams(
                    proxy_type: probeType,
                    host: validated.host,
                    port: validated.port,
                    username: validated.username,
                    password: validated.password,
                    fallback_to_direct: false
                )
            )
            let latency = UInt32(res["latency_ms"]?.uint64Value ?? 0)
            let statusMsg = FabushiPlatformStrings.proxyLatencyFormatted(latency, locale: locale)
            let applied = applyProbeResult(
                latencyMs: latency,
                statusMessage: statusMsg,
                capturedOpGeneration: opGen,
                capturedAccountGeneration: accountGen
            )
            guard applied else { return nil }
            if let idx = proxies.firstIndex(where: { $0.id == validated.id }) {
                proxies[idx].lastLatencyMs = latency
            }
            lastErrorMessage = nil
            return latency
        } catch {
            let statusMsg = "\(FabushiPlatformStrings.text(.settingsProxyStatusFailed, locale: locale)): \(error.localizedDescription)"
            _ = applyProbeResult(
                latencyMs: nil,
                statusMessage: statusMsg,
                capturedOpGeneration: opGen,
                capturedAccountGeneration: accountGen
            )
            if opGen == proxyOpGeneration && accountGen == coordinator.accountScopeGeneration {
                lastErrorMessage = error.localizedDescription
                router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            }
            return nil
        }
    }

    // MARK: - Import & Export RFC-3986 `tg://proxy` / `tg://socks` Links

    @discardableResult
    public func importProxyFromDeepLink(locale: FabushiLocale = .en) async -> FabushiProxyRecord? {
        do {
            let parsed = try FabushiProxyDeepLinks.parseImportLink(importLinkDraft)
            let saved = await upsertProxy(parsed, activate: parsed.kind.isRustHostSupported, locale: locale)
            if saved != nil {
                importLinkDraft = ""
            }
            return saved
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    public func copyShareLink(for record: FabushiProxyRecord, locale: FabushiLocale = .en) -> String {
        let link = FabushiProxyDeepLinks.exportLink(for: record)
        guard !link.isEmpty else { return "" }
        FabushiSecureClipboard.copySensitive(link)
        router.toastController.showNonChatToast(
            FabushiPlatformStrings.text(.copied, locale: locale),
            isError: false
        )
        return link
    }

    // MARK: - Private JSON & Local MTProto Helpers

    private static func decodeProxyList(_ value: FabushiJSONValue) -> [FabushiProxyRecord] {
        guard let arr = value["proxies"]?.arrayValue else { return [] }
        return arr.compactMap { item -> FabushiProxyRecord? in
            guard let id = item["id"]?.stringValue,
                  let typeRaw = item["proxy_type"]?.stringValue,
                  let kind = FabushiProxyKind.parse(typeRaw)
            else {
                return nil
            }
            let host = item["host"]?.stringValue ?? ""
            let port = UInt16(clamping: Int(item["port"]?.uint64Value ?? 0))
            let username = item["username"]?.stringValue
            let password = item["password"]?.stringValue
            let fallback = item["fallback_to_direct"]?.boolValue ?? false
            let active = item["active"]?.boolValue ?? false
            return FabushiProxyRecord(
                id: id,
                kind: kind,
                host: host,
                port: port,
                username: username,
                password: password,
                mtprotoSecret: "",
                fallbackToDirect: fallback,
                active: active
            )
        }
    }

    private func loadLocalMtprotoProxies() async -> [FabushiProxyRecord] {
        struct LocalGetParams: Encodable, Sendable {
            let key: String
        }
        guard let res: FabushiJSONValue = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.settings.method("localGet"),
            params: LocalGetParams(key: Self.mtprotoStorageKey)
        ),
            let raw = res["value"]?.stringValue,
            let data = raw.data(using: .utf8),
            let decoded = try? JSONDecoder().decode([FabushiProxyRecord].self, from: data)
        else {
            return []
        }
        return decoded
    }

    private func saveLocalMtprotoProxies(_ items: [FabushiProxyRecord]) async {
        guard let data = try? JSONEncoder().encode(items),
              let encoded = String(data: data, encoding: .utf8)
        else {
            return
        }
        struct LocalSetParams: Encodable, Sendable {
            let key: String
            let value: String
        }
        _ = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.settings.method("localSet"),
            params: LocalSetParams(key: Self.mtprotoStorageKey, value: encoded)
        ) as FabushiJSONValue
    }
}
