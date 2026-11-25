import Foundation
import Observation

@MainActor
@Observable
public final class FabushiMcpViewModel {
    @ObservationIgnored public let coordinator: FabushiCoordinator
    @ObservationIgnored public let router: ShellNavigationRouter
    @ObservationIgnored public let cleanupHub: FabushiAgentStudioLogoutCleanupHub
    @ObservationIgnored private let taskBag = FabushiCancellableTaskBag()
    @ObservationIgnored private let cleanupRegistrationId: String

    public private(set) var lastObservedGeneration: UInt64
    public private(set) var lastObservedAccountId: String?

    public var isLoading: Bool = false
    public var grants: [FabushiMcpOAuthGrantDTO] = []
    public var remoteServers: [FabushiMcpRemoteServerDTO] = []
    public var remoteTools: [FabushiJSONValue] = []
    public var localRegistryTools: [FabushiMcpLocalToolDTO] = []
    public var auditRecords: [FabushiMcpToolAuditRecordDTO] = []

    // Audit filters
    public var auditSearchQuery: String = ""
    public var selectedAuditScope: String?
    public var auditStatusFilter: FabushiMcpAuditStatusFilter = .all

    // QR-Poll login approval & kill-switch state
    public var pairTokenInput: String = ""
    public var lastApprovedPairToken: String?
    public var lastKillSwitchResult: FabushiJSONValue?
    public var isServerAuditPendingM6: Bool = false
    public var lastSandboxNotice: String?
    public var lastErrorMessage: String?

    public var filteredAuditRecords: [FabushiMcpToolAuditRecordDTO] {
        let query = auditSearchQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return auditRecords.filter { record in
            switch auditStatusFilter {
            case .all:
                break
            case .allowed:
                if !record.ok { return false }
            case .failed:
                if record.ok { return false }
            }
            if let scope = selectedAuditScope, !scope.isEmpty, record.inferredScope != scope {
                return false
            }
            if query.isEmpty { return true }
            return record.toolName.lowercased().contains(query)
                || record.inferredScope.lowercased().contains(query)
                || record.clientId.lowercased().contains(query)
                || record.argsSummary.lowercased().contains(query)
                || (record.errorCode?.lowercased().contains(query) ?? false)
        }
    }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub()
    ) {
        self.coordinator = coordinator
        self.router = router
        self.cleanupHub = cleanupHub
        self.lastObservedGeneration = coordinator.accountScopeGeneration
        self.lastObservedAccountId = coordinator.activeAccountId
        self.cleanupRegistrationId = "mcp.\(UUID().uuidString)"

        cleanupHub.registerResetHandler(id: cleanupRegistrationId) { [weak self] reason in
            self?.resetOnLogout(reason: reason)
        }
    }

    deinit {
        taskBag.cancelAll()
    }

    // MARK: - Account Scope Fencing & `resetOnLogout()`

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
        isLoading = false
        grants = []
        remoteServers = []
        remoteTools = []
        localRegistryTools = []
        auditRecords = []
        auditSearchQuery = ""
        selectedAuditScope = nil
        auditStatusFilter = .all
        pairTokenInput = ""
        lastApprovedPairToken = nil
        lastKillSwitchResult = nil
        isServerAuditPendingM6 = false
        lastSandboxNotice = nil
        lastErrorMessage = nil

        lastObservedGeneration = coordinator.accountScopeGeneration
        lastObservedAccountId = coordinator.activeAccountId
    }

    // MARK: - Refresh All

    public func loadAll() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoading = true
        defer { isLoading = false }

        await loadLocalRegistryTools()
        await loadGrants()
        await loadServers()
        await loadAuditLog()
    }

    // MARK: - OAuth 2.1 PKCE Grants (`mcp.listGrants`, `mcp.revokeGrant`)

    public func loadGrants() async {
        guard coordinator.isStarted else { return }
        guard case .authenticated = coordinator.authState else {
            grants = []
            return
        }
        do {
            let response: FabushiMcpOAuthGrantsResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("listGrants"),
                params: FabushiEmptyParams()
            )
            grants = response.grants
        } catch {
            handleError(error, silentOffline: true)
        }
    }

    @discardableResult
    public func revokeGrant(familyId: String) async -> Bool {
        guard coordinator.isStarted else { return false }
        let trimmed = familyId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        do {
            let response: FabushiMcpRevokeGrantResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("revokeGrant"),
                params: FabushiMcpRevokeGrantRequest(familyId: trimmed)
            )
            if response.revoked {
                grants.removeAll { $0.familyId == trimmed }
            }
            return response.revoked
        } catch {
            handleError(error)
            return false
        }
    }

    // MARK: - QR-Poll Login Approval (`auth.qrPairAccept`)

    @discardableResult
    public func approveQrPairLogin(rawPayload: String, locale: FabushiLocale = .en) async -> Bool {
        guard coordinator.isStarted else { return false }
        guard let token = FabushiMcpQrPairParser.extractPairToken(from: rawPayload) else {
            lastErrorMessage = "Invalid QR pair token."
            return false
        }

        do {
            let response: FabushiMcpQrPairAcceptResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("qrPairAccept"),
                params: FabushiMcpQrPairAcceptRequest(pairToken: token)
            )
            if response.accepted {
                lastApprovedPairToken = token
                pairTokenInput = ""
                router.toastController.showNonChatToast(
                    FabushiAgentStudioStrings.text("m4.mcp.qrApprovedToast", locale: locale),
                    isError: false
                )
            }
            return response.accepted
        } catch {
            handleError(error)
            return false
        }
    }

    // MARK: - Global Emergency Kill-Switch (`remote.sessionsDisconnectAll`)

    @discardableResult
    public func triggerEmergencyKillSwitch(locale: FabushiLocale = .en) async -> Bool {
        guard coordinator.isStarted else { return false }
        do {
            let result: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("sessionsDisconnectAll"),
                params: FabushiEmptyParams()
            )
            lastKillSwitchResult = result
            router.toastController.showNonChatToast(
                FabushiAgentStudioStrings.text("m4.mcp.killSwitchDoneToast", locale: locale),
                isError: false
            )
            await loadGrants()
            return true
        } catch {
            handleError(error)
            return false
        }
    }

    // MARK: - Remote MCP Servers & Tools (`mcp.listServers`, `registerServer`, `listTools`, `callTool`, `approvePermission`)

    public func loadServers() async {
        guard coordinator.isStarted else { return }
        guard case .authenticated = coordinator.authState else {
            remoteServers = []
            return
        }
        do {
            let rawList: [FabushiJSONValue] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("listServers"),
                params: FabushiEmptyParams()
            )
            remoteServers = rawList.enumerated().map { idx, val in
                FabushiMcpRemoteServerDTO(from: val, fallbackIndex: idx)
            }
        } catch {
            handleError(error, silentOffline: true)
        }
    }

    /// Registers a remote MCP server via `mcp.registerServer`.
    /// When `transport == "stdio"`, Rust host fails closed with `capabilityUnavailable` (`IOS-MAIN-INV-04`);
    /// this method preserves and surfaces that exact error cleanly without crashing or faking success.
    @discardableResult
    public func registerServer(
        name: String,
        transport: String,
        url: String? = nil,
        command: String? = nil,
        locale: FabushiLocale = .en
    ) async throws -> FabushiJSONValue {
        guard coordinator.isStarted else {
            throw FabushiHostError.notInitialized
        }
        do {
            let response: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("registerServer"),
                params: FabushiMcpRegisterServerRequest(
                    name: name,
                    transport: transport,
                    url: url,
                    command: command
                )
            )
            await loadServers()
            return response
        } catch let hostError as FabushiHostError {
            if case .capabilityUnavailable = hostError {
                lastSandboxNotice = FabushiAgentStudioStrings.text("m4.mcp.stdioBlockedNotice", locale: locale)
            }
            throw hostError
        }
    }

    public func loadRemoteTools() async {
        guard coordinator.isStarted else { return }
        guard case .authenticated = coordinator.authState else {
            remoteTools = []
            return
        }
        do {
            let list: [FabushiJSONValue] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("listTools"),
                params: FabushiEmptyParams()
            )
            remoteTools = list
        } catch {
            handleError(error, silentOffline: true)
        }
    }

    public func callRemoteTool(
        serverId: String? = nil,
        toolName: String,
        arguments: FabushiJSONValue
    ) async -> FabushiJSONValue? {
        guard coordinator.isStarted else { return nil }
        do {
            return try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("callTool"),
                params: FabushiMcpCallToolRequest(
                    serverId: serverId,
                    toolName: toolName,
                    arguments: arguments
                )
            )
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func approveRemotePermission(requestId: String, approved: Bool) async -> FabushiJSONValue? {
        guard coordinator.isStarted else { return nil }
        do {
            return try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("approvePermission"),
                params: FabushiMcpApprovePermissionRequest(requestId: requestId, approved: approved)
            )
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - Local Embedded Registry & Permission Dock (`mcp.localRegistryList`, `mcp.permissionDockResolve`)

    public func loadLocalRegistryTools() async {
        guard coordinator.isStarted else { return }
        do {
            let tools: [FabushiMcpLocalToolDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("localRegistryList"),
                params: FabushiEmptyParams()
            )
            localRegistryTools = tools
        } catch {
            handleError(error)
        }
    }

    @discardableResult
    public func resolveLocalPermissionDock(requestId: String, approved: Bool) async -> FabushiStudioToolDockResolveResponse? {
        guard coordinator.isStarted else { return nil }
        do {
            return try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("permissionDockResolve"),
                params: FabushiMcpApprovePermissionRequest(requestId: requestId, approved: approved)
            )
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - Redacted Tool Audit Log (Graceful `mcp.listAudit` Probe + Real Local Approval Ledger)

    private struct AuditListRequest: Encodable, Sendable {
        let limit: Int
    }

    private struct LocalApprovalsListRequest: Encodable, Sendable {
        let pendingOnly: Bool
        enum CodingKeys: String, CodingKey {
            case pendingOnly = "pending_only"
        }
    }

    /// Honestly probes server-side `mcp.listAudit` when authenticated; if the frozen host returns
    /// `.unknownCommand` (scheduled for M6), sets `isServerAuditPendingM6 = true` and loads real
    /// local SQLite approval ledger records redacted through `FabushiMcpAuditRedactor` — NEVER fabricating fake rows!
    public func loadAuditLog(limit: Int = 50) async {
        guard coordinator.isStarted else { return }
        var combined: [FabushiMcpToolAuditRecordDTO] = []

        if case .authenticated = coordinator.authState {
            do {
                let serverRecords: [FabushiMcpToolAuditRecordDTO] = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.mcp.method("listAudit"),
                    params: AuditListRequest(limit: limit)
                )
                isServerAuditPendingM6 = false
                combined.append(contentsOf: serverRecords.map { FabushiMcpAuditRedactor.redactRecord($0) })
            } catch let hostError as FabushiHostError {
                if case .unknownCommand = hostError {
                    isServerAuditPendingM6 = true
                } else {
                    handleError(hostError, silentOffline: true)
                }
            } catch {
                handleError(error, silentOffline: true)
            }
        } else {
            isServerAuditPendingM6 = true
        }

        // Supplement with real local SQLite approval ledger entries (zero fake rows).
        do {
            let localApprovals: [FabushiStudioApprovalRecordDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("approvalsList"),
                params: LocalApprovalsListRequest(pendingOnly: false)
            )
            let localMapped: [FabushiMcpToolAuditRecordDTO] = localApprovals.map { rec in
                let rawArgs = "{\"command\":\"\(escapeJson(rec.command))\",\"pattern\":\"\(escapeJson(rec.patternName))\",\"risk\":\"\(escapeJson(rec.riskLevel))\"}"
                let isOk = rec.status != .denied
                return FabushiMcpAuditRedactor.redactRecord(
                    FabushiMcpToolAuditRecordDTO(
                        auditId: "local_\(rec.id)",
                        userId: 0,
                        clientId: rec.decidedBy ?? "local_studio",
                        familyId: rec.sessionId,
                        sessionId: rec.sessionId,
                        toolName: rec.patternName.isEmpty ? "local_approval" : rec.patternName,
                        targetNodeId: nil,
                        argsSummary: rawArgs,
                        ok: isOk,
                        errorCode: isOk ? nil : "denied",
                        durationMs: 0,
                        createdAtMs: rec.createdAt,
                        source: .localApprovalLedger
                    )
                )
            }
            combined.append(contentsOf: localMapped)
        } catch {
            // Non-fatal
        }

        auditRecords = combined.sorted { $0.createdAtMs > $1.createdAtMs }
    }

    private func escapeJson(_ text: String) -> String {
        text
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n")
    }

    private func handleError(_ error: Error, silentOffline: Bool = false) {
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
            if silentOffline {
                if case .networkError = hostError { return }
                if case .unauthorized = hostError { return }
            }
        }
        lastErrorMessage = error.localizedDescription
    }
}
