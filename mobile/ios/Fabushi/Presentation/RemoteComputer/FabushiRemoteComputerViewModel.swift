import Foundation
import Observation
import UIKit

/// Generation-fenced ViewModel driving the standalone UU-Remote Computer Hub (`D10-11`),
/// Interactive Remote Workbench (Desktop Stream + Multi-Session PTY + Files + Device Settings),
/// Quick Assist, Cloud PC CRUD, AI & MCP approvals, and Tailnet mesh status.
@MainActor
@Observable
public final class FabushiRemoteComputerViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    // Hub Navigation & Overlay State (Deterministic Back-Stack Unwind)
    public var selectedHubTab: FabushiRemoteHubTab = .myDevices
    public var activeWorkbenchNodeId: String? = nil
    public var selectedWorkbenchTab: FabushiRemoteWorkbenchTab = .desktop
    public var isPermissionModalPresented: Bool = false
    public var isCloudPCFormPresented: Bool = false
    public var isBrowserSheetPresented: Bool = false
    public var shouldAutoFocusPtyInput: Bool = false

    // Domain Data
    public private(set) var computers: [FabushiComputerNodeDto] = []
    public private(set) var pendingApprovals: [FabushiPendingComputerCommandDto] = []
    public private(set) var activeSessions: [FabushiRemoteSessionDto] = []
    public private(set) var assistConfig: FabushiRemoteAssistConfigDto = FabushiRemoteAssistConfigDto()
    public private(set) var tailnetStatus: FabushiTailnetNodeStatus = FabushiTailnetNodeStatus()
    public private(set) var forceDerpOnly: Bool = false
    public private(set) var lastEmergencyTakeover: FabushiHubEmergencyTakeoverResult? = nil
    public private(set) var overrideEpoch: UInt64 = 0
    public private(set) var latestFrameByNodeId: [String: String] = [:]
    public private(set) var remoteDirectoryEntriesByNodeId: [String: [String]] = [:]
    public private(set) var botProfilesByBotId: [UInt64: FabushiBotComputerProfileDto] = [:]
    public private(set) var lastOsc52ClipboardText: String? = nil
    public private(set) var privacyScreenEnabled: Bool = false
    public private(set) var activeDisplayIndex: UInt32 = 0

    // Multi-Session PTY Tabs
    public private(set) var ptyTabs: [FabushiPtySessionTab] = []
    public var selectedPtyTabId: String? = nil
    public var ptyInputDraft: String = ""

    // Status & Loading Flags
    public private(set) var isLoading: Bool = false
    public private(set) var isUpdatingAllowRemote: Bool = false
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var lastOperationMessage: String? = nil
    public private(set) var observedGeneration: UInt64

    @ObservationIgnored private let purgerId = "m5.remote.\(UUID().uuidString)"
    @ObservationIgnored nonisolated(unsafe) private var eventTask: Task<Void, Never>?

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        self.coordinator = coordinator
        self.router = router
        self.observedGeneration = coordinator.accountScopeGeneration
        installAccountResetHooks()
        startEventSubscription()
    }

    deinit {
        eventTask?.cancel()
    }

    public var activeWorkbenchNode: FabushiComputerNodeDto? {
        guard let activeWorkbenchNodeId else { return nil }
        return computers.first(where: { $0.nodeId == activeWorkbenchNodeId })
    }

    public var cloudComputers: [FabushiComputerNodeDto] {
        computers.filter(\.isCloudNode)
    }

    public var deviceComputers: [FabushiComputerNodeDto] {
        computers.filter { !$0.isCloudNode }
    }

    public var activeSessionCount: Int {
        activeSessions.count
    }

    public var activePtyTab: FabushiPtySessionTab? {
        if let selectedPtyTabId, let match = ptyTabs.first(where: { $0.id == selectedPtyTabId }) {
            return match
        }
        return ptyTabs.first
    }

    // MARK: - Lifecycle & Generation-Fenced Reset

    private func installAccountResetHooks() {
        coordinator.registerAccountSecretsPurger(id: purgerId) { [weak self] _ in
            self?.resetStateForAccountScopeChange(generation: self?.coordinator.accountScopeGeneration ?? 0)
        }
        coordinator.registerServerSwitchSecretsPurger(id: purgerId) { [weak self] in
            self?.resetStateForAccountScopeChange(generation: self?.coordinator.accountScopeGeneration ?? 0)
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
        resetStateForAccountScopeChange(generation: currentGen)
    }

    public func resetStateForAccountScopeChange(generation: UInt64) {
        observedGeneration = generation
        computers = []
        pendingApprovals = []
        activeSessions = []
        assistConfig = FabushiRemoteAssistConfigDto()
        tailnetStatus = FabushiTailnetNodeStatus()
        forceDerpOnly = false
        lastEmergencyTakeover = nil
        overrideEpoch = 0
        latestFrameByNodeId = [:]
        remoteDirectoryEntriesByNodeId = [:]
        botProfilesByBotId = [:]
        lastOsc52ClipboardText = nil
        privacyScreenEnabled = false
        activeDisplayIndex = 0
        ptyTabs = []
        selectedPtyTabId = nil
        ptyInputDraft = ""
        activeWorkbenchNodeId = nil
        selectedWorkbenchTab = .desktop
        isPermissionModalPresented = false
        isCloudPCFormPresented = false
        isBrowserSheetPresented = false
        shouldAutoFocusPtyInput = false
        isLoading = false
        isUpdatingAllowRemote = false
        lastErrorMessage = nil
        lastOperationMessage = nil
        router.activeRemoteControlStatus = nil
    }

    private func startEventSubscription() {
        let stream = coordinator.host.makeEventStream()
        eventTask = Task { @MainActor [weak self] in
            for await event in stream {
                guard !Task.isCancelled, let self else { break }
                self.reconcileAccountScopeIfNeeded()
                guard event.generation >= self.coordinator.accountScopeGeneration else { continue }
                if case .remoteEmergencyTakeover(let disconnected, let epoch, let elapsedUs) = event.kind {
                    self.overrideEpoch = max(self.overrideEpoch, epoch)
                    self.lastEmergencyTakeover = FabushiHubEmergencyTakeoverResult(
                        disconnectedSessions: disconnected,
                        overrideEpoch: epoch,
                        elapsedUs: elapsedUs
                    )
                    self.activeSessions = []
                    self.router.activeRemoteControlStatus = nil
                }
            }
        }
    }

    // MARK: - Deterministic Back-Stack Unwind (`D10-11`)

    /// Unwinds navigation strictly in order:
    /// `permission/capability modal -> auxiliary form/browser sheet -> workbench/device detail -> Hub Overview -> dismiss`.
    /// Returns `true` when an inner layer was unwound, or `false` when already at Hub Overview root.
    @discardableResult
    public func unwindBackNavigation() -> Bool {
        if isPermissionModalPresented {
            isPermissionModalPresented = false
            return true
        }
        if isCloudPCFormPresented {
            isCloudPCFormPresented = false
            return true
        }
        if isBrowserSheetPresented {
            isBrowserSheetPresented = false
            return true
        }
        if activeWorkbenchNodeId != nil {
            activeWorkbenchNodeId = nil
            shouldAutoFocusPtyInput = false
            selectedWorkbenchTab = .desktop
            return true
        }
        if selectedHubTab != .myDevices {
            selectedHubTab = .myDevices
            return true
        }
        return false
    }

    // MARK: - Device Card Entry Actions (`Enter Desktop` / `Shell` / `Settings`)

    public func openDesktop(node: FabushiComputerNodeDto) {
        reconcileAccountScopeIfNeeded()
        activeWorkbenchNodeId = node.nodeId
        selectedWorkbenchTab = .desktop
        shouldAutoFocusPtyInput = false
        if let frame = node.latestFrameDataUrl, !frame.isEmpty {
            latestFrameByNodeId[node.nodeId] = frame
        }
        ensureDefaultPtyTab(for: node)
    }

    /// `D10-11`: `Shell` action opens Interactive Workbench directly on the `RemoteTerminal` (PTY)
    /// tab, requests terminal input auto-focus, and triggers a `list_dir` directory refresh.
    public func openShell(node: FabushiComputerNodeDto) {
        reconcileAccountScopeIfNeeded()
        activeWorkbenchNodeId = node.nodeId
        selectedWorkbenchTab = .terminal
        shouldAutoFocusPtyInput = true
        ensureDefaultPtyTab(for: node)
        Task { @MainActor [weak self] in
            guard let self else { return }
            _ = await self.refreshRemoteDirectory(nodeId: node.nodeId, path: ".")
        }
    }

    public func openDeviceSettings(node: FabushiComputerNodeDto) {
        reconcileAccountScopeIfNeeded()
        activeWorkbenchNodeId = node.nodeId
        selectedWorkbenchTab = .settings
        shouldAutoFocusPtyInput = false
    }

    public func consumeAutoFocusPtyRequest() {
        shouldAutoFocusPtyInput = false
    }

    public func openNodeIfPresent(_ nodeId: String?) {
        guard let nodeId, !nodeId.isEmpty else { return }
        if let node = computers.first(where: { $0.nodeId == nodeId }) {
            openDesktop(node: node)
        } else {
            activeWorkbenchNodeId = nodeId
        }
    }

    // MARK: - Data Refresh RPCs

    public func refreshAll() async {
        reconcileAccountScopeIfNeeded()
        isLoading = true
        defer { isLoading = false }
        await refreshComputers()
        await refreshAssistInfo()
        await refreshSessions()
        await refreshTailnetStatus()
    }

    public func refreshComputers() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let response: FabushiComputerListResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("list"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            computers = response.computers
            pendingApprovals = response.pendingApprovals
            for node in response.computers {
                if let frame = node.latestFrameDataUrl, !frame.isEmpty {
                    latestFrameByNodeId[node.nodeId] = frame
                }
            }
            lastErrorMessage = nil
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
        }
    }

    public func refreshAssistInfo() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let config: FabushiRemoteAssistConfigDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("assistInfo"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            assistConfig = config
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
        }
    }

    public func refreshSessions() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let response: FabushiRemoteSessionsResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("sessionsList"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            activeSessions = response.sessions
            syncControlledOverlayPill()
        } catch {
            // Also consult local hub preemption status so local sessions stay accurate offline.
            if let localStatus: FabushiHubPreemptionStatus = try? await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("hubPreemptionStatus"),
                params: FabushiEmptyParams()
            ), coordinator.accountScopeGeneration == capturedGen {
                activeSessions = localStatus.sessions
                overrideEpoch = max(overrideEpoch, localStatus.overrideEpoch)
                syncControlledOverlayPill()
            }
        }
    }

    public func refreshTailnetStatus() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let status: FabushiTailnetNodeStatus = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.tailnet.method("status"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            tailnetStatus = status
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
        }
    }

    private func syncControlledOverlayPill() {
        if let first = activeSessions.first {
            let nodeName = computers.first(where: { $0.nodeId == first.targetNodeId })?.displayName
                ?? first.targetNodeId
            router.activeRemoteControlStatus = RemoteControlStatusState(
                id: first.sessionId,
                nodeName: nodeName,
                controllerLabel: first.controllerName
            )
        } else {
            router.activeRemoteControlStatus = nil
        }
    }

    // MARK: - D10-11 Optimistic `Allow Remote` Toggle with Strict Rollback

    @discardableResult
    public func toggleAllowRemote(_ newValue: Bool, locale: FabushiLocale = .en) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let previousConfig = assistConfig
        let capturedGen = coordinator.accountScopeGeneration

        // 1. Optimistic UI update
        assistConfig = FabushiRemoteAssistConfigDto(
            assistId: previousConfig.assistId,
            allowAssist: newValue,
            codeMode: previousConfig.codeMode,
            currentCode: previousConfig.currentCode,
            expiresAtMs: previousConfig.expiresAtMs
        )
        isUpdatingAllowRemote = true
        defer { isUpdatingAllowRemote = false }

        struct AllowParams: Encodable, Sendable {
            let allowAssist: Bool
            enum CodingKeys: String, CodingKey {
                case allowAssist = "allow_assist"
            }
        }

        do {
            let updated: FabushiRemoteAssistConfigDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("assistAllow"),
                params: AllowParams(allowAssist: newValue)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            assistConfig = updated
            lastErrorMessage = nil
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            // 2. Strict rollback on ANY failure + non-chat error toast
            assistConfig = previousConfig
            let prefix = FabushiPlatformStrings.text(.remoteAssistAllowFailedPrefix, locale: locale)
            let message = "\(prefix): \(error.localizedDescription)"
            lastErrorMessage = message
            router.toastController.showNonChatToast(message, isError: true)
            return false
        }
    }

    // MARK: - D10-11 Inline `Disconnect All (N)` & `<10ms` Emergency Takeover

    @discardableResult
    public func disconnectAllSessions(locale: FabushiLocale = .en) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        var remoteError: Error?

        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("sessionsDisconnectAll"),
                params: FabushiEmptyParams()
            )
        } catch {
            remoteError = error
        }

        // Always execute <10ms local emergency takeover kill-switch so active leases preempt immediately.
        let takeover = await emergencyTakeover(locale: locale, suppressSuccessToast: remoteError != nil)
        guard coordinator.accountScopeGeneration == capturedGen else { return false }

        if let remoteError {
            let message = remoteError.localizedDescription
            lastErrorMessage = message
            router.toastController.showNonChatToast(message, isError: true)
            return false
        }

        activeSessions = []
        router.activeRemoteControlStatus = nil
        if takeover != nil {
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.remoteDisconnectAllSuccess, locale: locale),
                isError: false
            )
        }
        return true
    }

    @discardableResult
    public func emergencyTakeover(
        locale: FabushiLocale = .en,
        suppressSuccessToast: Bool = false
    ) async -> FabushiHubEmergencyTakeoverResult? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let result: FabushiHubEmergencyTakeoverResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("hubEmergencyTakeover"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastEmergencyTakeover = result
            overrideEpoch = max(overrideEpoch, result.overrideEpoch)
            activeSessions = []
            router.activeRemoteControlStatus = nil
            lastErrorMessage = nil
            if !suppressSuccessToast {
                router.toastController.showNonChatToast(
                    FabushiPlatformStrings.text(.remoteEmergencyTakeoverSuccess, locale: locale),
                    isError: false
                )
            }
            return result
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    public func disconnectSession(sessionId: String, locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let sessionId: String
            enum CodingKeys: String, CodingKey { case sessionId = "session_id" }
        }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("sessionDisconnect"),
                params: Params(sessionId: sessionId)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            activeSessions.removeAll { $0.sessionId == sessionId }
            syncControlledOverlayPill()
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    // MARK: - Quick Assist & Remote Assist RPCs

    public func resetAssistCode(locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let updated: FabushiRemoteAssistConfigDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("assistResetCode"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            assistConfig = updated
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(locale, en: "Access code rotated", zhHans: "访问验证码已重置"),
                isError: false
            )
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func setCustomAssistCode(_ code: String, locale: FabushiLocale = .en) async {
        let trimmed = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable { let code: String }
        do {
            let updated: FabushiRemoteAssistConfigDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("assistSetCode"),
                params: Params(code: trimmed)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            assistConfig = updated
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(locale, en: "Custom access code saved", zhHans: "自定义验证码已保存"),
                isError: false
            )
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func copyAssistInvite(locale: FabushiLocale = .en) {
        let id = assistConfig.formattedAssistId
        let code = assistConfig.currentCode ?? ""
        let payload = code.isEmpty ? "Fabushi Assist ID: \(id)" : "Fabushi Assist ID: \(id) | Code: \(code)"
        FabushiSecureClipboard.copySensitive(payload)
        router.toastController.showNonChatToast(
            FabushiPlatformStrings.text(.copied, locale: locale),
            isError: false
        )
    }

    @discardableResult
    public func startRemoteAssist(
        assistId: String,
        accessCode: String,
        mode: FabushiRemoteControlMode = .desktop,
        locale: FabushiLocale = .en
    ) async -> FabushiAssistStartResponse? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        let cleanedId = assistId.filter { !$0.isWhitespace }
        let cleanedCode = accessCode.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanedId.isEmpty, !cleanedCode.isEmpty else { return nil }

        let request = FabushiAssistStartRequest(
            assistId: cleanedId,
            accessCode: cleanedCode,
            mode: mode
        )
        do {
            let response: FabushiAssistStartResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("assistStart"),
                params: request
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            activeWorkbenchNodeId = response.targetNodeId
            selectedWorkbenchTab = mode == .terminal ? .terminal : .desktop
            shouldAutoFocusPtyInput = mode == .terminal
            await refreshSessions()
            return response
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    // MARK: - Cloud PC CRUD, Policy, Connection Test, Wake-on-LAN & Approvals

    @discardableResult
    public func upsertCloudComputer(
        _ request: FabushiUpsertCloudComputerRequest,
        locale: FabushiLocale = .en
    ) async -> FabushiComputerNodeDto? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let node: FabushiComputerNodeDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("upsertCloud"),
                params: request
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if let idx = computers.firstIndex(where: { $0.nodeId == node.nodeId }) {
                computers[idx] = node
            } else {
                computers.append(node)
            }
            isCloudPCFormPresented = false
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(locale, en: "Cloud PC saved", zhHans: "云电脑已保存"),
                isError: false
            )
            return node
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    public func updatePolicy(
        nodeId: String,
        policy: FabushiUpdateComputerPolicyRequest,
        locale: FabushiLocale = .en
    ) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let nodeId: String
            let policy: FabushiUpdateComputerPolicyRequest
            enum CodingKeys: String, CodingKey {
                case nodeId = "node_id"
                case policy
            }
        }
        do {
            let updated: FabushiComputerNodeDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("updatePolicy"),
                params: Params(nodeId: nodeId, policy: policy)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            if let idx = computers.firstIndex(where: { $0.nodeId == updated.nodeId }) {
                computers[idx] = updated
            }
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.remotePolicySavedToast, locale: locale),
                isError: false
            )
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func deleteComputer(nodeId: String, locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let nodeId: String
            enum CodingKeys: String, CodingKey { case nodeId = "node_id" }
        }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("delete"),
                params: Params(nodeId: nodeId)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            computers.removeAll { $0.nodeId == nodeId }
            if activeWorkbenchNodeId == nodeId {
                activeWorkbenchNodeId = nil
            }
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(locale, en: "Computer removed", zhHans: "已移除该设备"),
                isError: false
            )
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    @discardableResult
    public func testConnection(nodeId: String, locale: FabushiLocale = .en) async -> FabushiRemoteComputerCommandResult? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let nodeId: String
            enum CodingKeys: String, CodingKey { case nodeId = "node_id" }
        }
        do {
            let result: FabushiRemoteComputerCommandResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("testConnection"),
                params: Params(nodeId: nodeId)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            let summary = result.ok
                ? FabushiPlatformStrings.text(locale, en: "Connection OK (\(result.durationMs)ms)", zhHans: "连接正常 (\(result.durationMs)ms)")
                : (result.stderr.isEmpty ? "Connection failed" : result.stderr)
            lastOperationMessage = summary
            router.toastController.showNonChatToast(summary, isError: !result.ok)
            return result
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    public func sendWakeOnLan(
        macAddress: String,
        broadcastIP: String? = nil,
        locale: FabushiLocale = .en
    ) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let macAddress: String
            let broadcastIp: String?
            enum CodingKeys: String, CodingKey {
                case macAddress = "mac_address"
                case broadcastIp = "broadcast_ip"
            }
        }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("sendWakeOnLan"),
                params: Params(macAddress: macAddress, broadcastIp: broadcastIP)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(locale, en: "Wake-on-LAN packet sent", zhHans: "已发送网络唤醒数据包"),
                isError: false
            )
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func approveCommand(commandId: String, approved: Bool, locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let commandId: String
            let approved: Bool
            enum CodingKeys: String, CodingKey {
                case commandId = "command_id"
                case approved
            }
        }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("approveCommand"),
                params: Params(commandId: commandId, approved: approved)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            pendingApprovals.removeAll { $0.commandId == commandId }
            router.toastController.showNonChatToast(
                approved
                    ? FabushiPlatformStrings.text(locale, en: "Command approved", zhHans: "已批准远程指令")
                    : FabushiPlatformStrings.text(locale, en: "Command denied", zhHans: "已拒绝远程指令"),
                isError: false
            )
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func loadBotProfile(botId: UInt64) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let botId: UInt64
            enum CodingKeys: String, CodingKey { case botId = "bot_id" }
        }
        do {
            let profile: FabushiBotComputerProfileDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("botProfileGet"),
                params: Params(botId: botId)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            botProfilesByBotId[botId] = profile
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
        }
    }

    // MARK: - Tailnet Mesh Controls

    public func startTailnet() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let status: FabushiTailnetNodeStatus = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.tailnet.method("start"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            tailnetStatus = status
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func stopTailnet() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let status: FabushiTailnetNodeStatus = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.tailnet.method("stop"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            tailnetStatus = status
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func setForceDerpOnly(_ force: Bool) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable { let force: Bool }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.tailnet.method("setForceDerpOnly"),
                params: Params(force: force)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            forceDerpOnly = force
            await refreshTailnetStatus()
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    // MARK: - Remote Workbench Execution (Desktop, PTY, Files, Hotkeys, Clipboard, Power)

    @discardableResult
    public func executeRemoteOp(
        nodeId: String,
        operation: FabushiRemoteComputerOp
    ) async -> FabushiRemoteComputerCommandResult? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        let request = FabushiRemoteComputerCommandRequest(
            nodeId: nodeId,
            callerLabel: "Fabushi iOS",
            operation: operation
        )
        do {
            let result: FabushiRemoteComputerCommandResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("executeRemoteOp"),
                params: request
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if let frame = result.frameDataUrl, !frame.isEmpty {
                latestFrameByNodeId[nodeId] = frame
            }
            if !result.entries.isEmpty {
                remoteDirectoryEntriesByNodeId[nodeId] = result.entries
            }
            lastErrorMessage = nil
            return result
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    @discardableResult
    public func refreshRemoteDirectory(nodeId: String, path: String = ".") async -> [String] {
        if let result = await executeRemoteOp(nodeId: nodeId, operation: .listDir(path: path)) {
            return result.entries
        }
        return remoteDirectoryEntriesByNodeId[nodeId] ?? []
    }

    public func captureDesktopFrame(nodeId: String) async {
        _ = await executeRemoteOp(nodeId: nodeId, operation: .captureScreen)
    }

    public func sendPointerAction(
        nodeId: String,
        action: String,
        x: Int32,
        y: Int32,
        button: String = "left"
    ) async {
        _ = await executeRemoteOp(
            nodeId: nodeId,
            operation: .mouseAction(action: action, x: x, y: y, button: button)
        )
    }

    public func remapAndSendHotkey(
        nodeId: String,
        key: String,
        modifiers: [String],
        hostOS: String = "macos"
    ) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct RemapParams: Encodable, Sendable {
            let modifiers: [String]
            let clientOs: String
            let hostOs: String
            let enabled: Bool
            enum CodingKeys: String, CodingKey {
                case modifiers
                case clientOs = "client_os"
                case hostOs = "host_os"
                case enabled
            }
        }
        var effectiveModifiers = modifiers
        if let remapped: FabushiRemapModifiersResponse = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.remote.method("remapModifiers"),
            params: RemapParams(modifiers: modifiers, clientOs: "ios", hostOs: hostOS, enabled: true)
        ), coordinator.accountScopeGeneration == capturedGen {
            effectiveModifiers = remapped.modifiers
        }
        _ = await executeRemoteOp(
            nodeId: nodeId,
            operation: .keyboardAction(action: "hotkey", key: key, modifiers: effectiveModifiers)
        )
    }

    public func switchDisplay(nodeId: String, displayIndex: UInt32) async {
        activeDisplayIndex = displayIndex
        _ = await executeRemoteOp(nodeId: nodeId, operation: .displaySwitch(displayIndex: displayIndex))
    }

    public func togglePrivacyScreen(nodeId: String, enabled: Bool) async {
        privacyScreenEnabled = enabled
        _ = await executeRemoteOp(
            nodeId: nodeId,
            operation: .privacyScreenToggle(enabled: enabled, message: nil)
        )
    }

    public func sendPowerAction(nodeId: String, kind: FabushiPowerActionKind) async {
        _ = await executeRemoteOp(nodeId: nodeId, operation: .powerAction(action: kind))
    }

    public func pullRemoteClipboard(nodeId: String, locale: FabushiLocale = .en) async {
        if let result = await executeRemoteOp(nodeId: nodeId, operation: .clipboardGet),
           !result.stdout.isEmpty
        {
            FabushiSecureClipboard.copySensitive(result.stdout)
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.copied, locale: locale),
                isError: false
            )
        }
    }

    public func pushLocalClipboard(nodeId: String, locale: FabushiLocale = .en) async {
        let text = UIPasteboard.general.string ?? ""
        guard !text.isEmpty else { return }
        _ = await executeRemoteOp(nodeId: nodeId, operation: .clipboardSet(text: text))
        router.toastController.showNonChatToast(
            FabushiPlatformStrings.text(locale, en: "Clipboard pushed to remote node", zhHans: "已发送剪贴板至远端设备"),
            isError: false
        )
    }

    // MARK: - Multi-Session PTY & OSC-52

    private func ensureDefaultPtyTab(for node: FabushiComputerNodeDto) {
        guard ptyTabs.isEmpty else {
            if selectedPtyTabId == nil {
                selectedPtyTabId = ptyTabs.first?.id
            }
            return
        }
        let tab = FabushiPtySessionTab(
            title: "PTY 1",
            sessionName: "ios-\(node.nodeId.prefix(8))-1"
        )
        ptyTabs = [tab]
        selectedPtyTabId = tab.id
    }

    public func createPtySession(for node: FabushiComputerNodeDto) {
        let index = ptyTabs.count + 1
        let tab = FabushiPtySessionTab(
            title: "PTY \(index)",
            sessionName: "ios-\(node.nodeId.prefix(8))-\(index)"
        )
        ptyTabs.append(tab)
        selectedPtyTabId = tab.id
        shouldAutoFocusPtyInput = true
    }

    public func closePtySession(id: String) {
        ptyTabs.removeAll { $0.id == id }
        if selectedPtyTabId == id {
            selectedPtyTabId = ptyTabs.first?.id
        }
    }

    public func sendPtyCommand(nodeId: String, rawCommand: String) async {
        let command = rawCommand.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !command.isEmpty else { return }
        ptyInputDraft = ""

        if ptyTabs.isEmpty {
            let tab = FabushiPtySessionTab(title: "PTY 1", sessionName: "ios-pty-1")
            ptyTabs = [tab]
            selectedPtyTabId = tab.id
        }
        let activeTabId = selectedPtyTabId ?? ptyTabs[0].id
        if let idx = ptyTabs.firstIndex(where: { $0.id == activeTabId }) {
            ptyTabs[idx].outputLines.append("$ \(command)")
        }

        let result = await executeRemoteOp(
            nodeId: nodeId,
            operation: .shellExec(command: command, cwd: activePtyTab?.cwd, timeoutMs: 15_000)
        )
        guard let result else { return }

        if !result.stdout.isEmpty {
            await parseOsc52Output(result.stdout)
        }
        if let idx = ptyTabs.firstIndex(where: { $0.id == activeTabId }) {
            if !result.stdout.isEmpty {
                ptyTabs[idx].outputLines.append(result.stdout)
            }
            if !result.stderr.isEmpty {
                ptyTabs[idx].outputLines.append(result.stderr)
            }
        }
    }

    @discardableResult
    public func parseOsc52Output(_ rawOutput: String) async -> [FabushiOsc52ClipboardPayload] {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        struct Params: Encodable, Sendable {
            let dataB64: String
            enum CodingKeys: String, CodingKey { case dataB64 = "data_b64" }
        }
        let b64 = Data(rawOutput.utf8).base64EncodedString()
        do {
            let payloads: [FabushiOsc52ClipboardPayload] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("ptyParseOsc52"),
                params: Params(dataB64: b64)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return [] }
            if let text = payloads.last?.text, !text.isEmpty {
                lastOsc52ClipboardText = text
                FabushiSecureClipboard.copySensitive(text)
            }
            return payloads
        } catch {
            return []
        }
    }
}
