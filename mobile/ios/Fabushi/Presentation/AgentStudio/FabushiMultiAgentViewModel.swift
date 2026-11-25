import Foundation
import Observation

/// `@MainActor @Observable` ViewModel managing all 16 `multi_agent.*` RPCs for a group chat room:
/// - `FloorControlQueue` (`UserInterruption` preempt + `D14-02` 120s group floor lease)
/// - 5-hop `LoopBudget`
/// - Revision-fenced `SharedGroupWorkspace` with optimistic CAS conflict detection
/// - `D14-01` Stage 1 Group Wake Gate & Stage 2 Stay-Silent `[PASS]` diagnostics
/// - Scoped Ed25519 Capability Token issuance & verification
@MainActor
@Observable
public final class FabushiMultiAgentViewModel {
    // MARK: - Room Identity & State

    public let chatId: Int64
    public private(set) var isLoading: Bool = false
    public private(set) var floorStatus: FabushiMultiAgentFloorStatusDTO
    public private(set) var lastPreemptedNonce: String?
    public private(set) var loopBudget: FabushiMultiAgentLoopBudgetStatusDTO
    public private(set) var workspace: FabushiMultiAgentWorkspaceSnapshotDTO
    public private(set) var workspaceConflict: FabushiMultiAgentWorkspaceConflictState?

    // Diagnostics & Telemetry
    public private(set) var lastWakeDecision: FabushiMultiAgentWakeGateDecisionDTO?
    public private(set) var lastOutboundVerdict: FabushiMultiAgentOutboundVerdictDTO?
    public private(set) var lastPassSentinelResult: Bool?
    public private(set) var lastIssuedToken: FabushiMultiAgentIssuedCapabilityTokenDTO?
    public private(set) var lastTokenVerification: FabushiMultiAgentVerifyCapabilityResultDTO?
    public private(set) var handoffHistory: [FabushiMultiAgentHandoffRecord] = []
    public private(set) var botActivitiesByBotId: [Int64: String] = [:]
    public private(set) var lastErrorMessage: String?

    public private(set) var lastObservedGeneration: UInt64 = 0
    public private(set) var lastObservedAccountId: String?

    // MARK: - Non-Observed Dependencies

    @ObservationIgnored public let coordinator: FabushiCoordinator
    @ObservationIgnored public let router: ShellNavigationRouter
    @ObservationIgnored public let cleanupHub: FabushiAgentStudioLogoutCleanupHub
    @ObservationIgnored private let taskBag = FabushiCancellableTaskBag()
    @ObservationIgnored private let cleanupHandlerId: String
    @ObservationIgnored private var isEventStreamActive = false

    public init(
        chatId: Int64,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub()
    ) {
        self.chatId = chatId
        self.coordinator = coordinator
        self.router = router
        self.cleanupHub = cleanupHub
        self.cleanupHandlerId = "multiAgent.vm.\(chatId).\(UUID().uuidString)"
        self.floorStatus = FabushiMultiAgentFloorStatusDTO(chatId: chatId)
        self.loopBudget = FabushiMultiAgentLoopBudgetStatusDTO(chatId: chatId)
        self.workspace = FabushiMultiAgentWorkspaceSnapshotDTO(chatId: chatId)
        self.lastObservedGeneration = coordinator.accountScopeGeneration
        self.lastObservedAccountId = coordinator.activeAccountId

        cleanupHub.install(on: coordinator)
        cleanupHub.registerResetHandler(id: cleanupHandlerId) { [weak self] reason in
            self?.resetOnLogout(reason: reason)
        }
    }

    deinit {
        taskBag.cancelAll()
    }

    // MARK: - Lifecycle, Event Stream & Logout Reset

    public func syncAccountScopeIfNeeded() {
        let currentGen = coordinator.accountScopeGeneration
        let currentAccount = coordinator.activeAccountId
        if currentGen != lastObservedGeneration || currentAccount != lastObservedAccountId {
            resetOnLogout(reason: .generationAdvance(currentGen))
            lastObservedGeneration = currentGen
            lastObservedAccountId = currentAccount
        }
    }

    public func resetOnLogout(reason: FabushiAgentStudioLogoutCleanupHub.ResetReason = .manual) {
        _ = reason
        taskBag.cancelAll()
        isEventStreamActive = false
        isLoading = false
        floorStatus = FabushiMultiAgentFloorStatusDTO(chatId: chatId)
        lastPreemptedNonce = nil
        loopBudget = FabushiMultiAgentLoopBudgetStatusDTO(chatId: chatId)
        workspace = FabushiMultiAgentWorkspaceSnapshotDTO(chatId: chatId)
        workspaceConflict = nil
        lastWakeDecision = nil
        lastOutboundVerdict = nil
        lastPassSentinelResult = nil
        lastIssuedToken = nil
        lastTokenVerification = nil
        handoffHistory = []
        botActivitiesByBotId = [:]
        lastErrorMessage = nil
        lastObservedGeneration = coordinator.accountScopeGeneration
        lastObservedAccountId = coordinator.activeAccountId
    }

    public func startEventSubscriptionIfNeeded() {
        guard !isEventStreamActive, coordinator.isStarted else { return }
        isEventStreamActive = true
        let stream = coordinator.host.makeEventStream()
        let task = Task { @MainActor [weak self] in
            for await event in stream {
                guard let self, !Task.isCancelled else { break }
                self.ingestDomainEvent(event)
            }
            self?.isEventStreamActive = false
        }
        taskBag.replace(key: "multiAgent.events", with: task)
    }

    public func ingestDomainEvent(_ event: FabushiDomainEvent) {
        guard event.generation >= coordinator.accountScopeGeneration else {
            return
        }
        switch event.kind {
        case .agentFloorChanged(let eventChatId, let speaker, let queuedCount, let loopHops):
            guard eventChatId == chatId else { return }
            let preservedGrant = (floorStatus.activeGrant?.agentId == speaker) ? floorStatus.activeGrant : nil
            floorStatus = FabushiMultiAgentFloorStatusDTO(
                chatId: chatId,
                granted: speaker != nil,
                currentSpeaker: speaker,
                queuedCount: queuedCount,
                activeGrant: preservedGrant
            )
            if loopHops >= 0 {
                let hopU32 = UInt32(clamping: loopHops)
                loopBudget = FabushiMultiAgentLoopBudgetStatusDTO(
                    chatId: chatId,
                    currentHop: hopU32,
                    maxHops: loopBudget.maxHops,
                    exhausted: hopU32 >= loopBudget.maxHops,
                    trace: loopBudget.trace
                )
            }

        case .agentWorkspaceUpdated(let eventChatId, let revision, _, _):
            guard eventChatId == chatId else { return }
            if revision > workspace.revision {
                let refreshTask = Task { @MainActor [weak self] in
                    await self?.loadWorkspaceSnapshot()
                }
                taskBag.replace(key: "multiAgent.workspaceRefresh", with: refreshTask)
            }

        case .agentHandoffEvent(let eventChatId, let fromAgent, let toAgent, let hop, let maxHops):
            guard eventChatId == chatId else { return }
            let rec = FabushiMultiAgentHandoffRecord(
                chatId: eventChatId,
                fromAgent: fromAgent,
                toAgent: toAgent,
                hop: hop,
                maxHops: maxHops,
                timestampMs: event.timestampMs
            )
            handoffHistory.insert(rec, at: 0)
            if handoffHistory.count > 32 {
                handoffHistory.removeLast(handoffHistory.count - 32)
            }

        case .botActivityChanged(let eventChatId, let botUserId, let state, let detail):
            guard eventChatId == chatId else { return }
            if let detail, !detail.isEmpty {
                botActivitiesByBotId[botUserId] = "\(state): \(detail)"
            } else {
                botActivitiesByBotId[botUserId] = state
            }

        default:
            break
        }
    }

    // MARK: - Refresh All Room State

    public func refreshRoomState() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        startEventSubscriptionIfNeeded()
        isLoading = true
        defer { isLoading = false }

        await loadFloorStatus()
        await loadLoopBudgetStatus()
        await loadWorkspaceSnapshot()
    }

    // MARK: - Floor Control Queue (`multi_agent.floor*`)

    public func loadFloorStatus() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        do {
            floorStatus = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("floorStatus"),
                params: Params(chat_id: chatId)
            )
        } catch {
            handleError(error)
        }
    }

    /// Requests speaking floor for `agentId`, explicitly passing `lease_ms = 120_000` (`D14-02`).
    @discardableResult
    public func requestFloor(
        agentId: String,
        priority: FabushiMultiAgentFloorPriority = .generalContribution,
        leaseMs: UInt64 = FabushiMultiAgentConstants.groupFloorLeaseMs
    ) async -> FabushiMultiAgentFloorStatusDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        let trimmed = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let agent_id: String
            let priority: String
            let lease_ms: UInt64
        }
        do {
            let res: FabushiMultiAgentFloorStatusDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("floorRequest"),
                params: Params(
                    chat_id: chatId,
                    agent_id: trimmed,
                    priority: priority.rawValue,
                    lease_ms: leaseMs
                )
            )
            floorStatus = res
            return res
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func preemptFloorForUser() async -> FabushiMultiAgentFloorPreemptDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct Params: Encodable, Sendable { let chat_id: Int64 }
        do {
            let res: FabushiMultiAgentFloorPreemptDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("floorPreemptUser"),
                params: Params(chat_id: chatId)
            )
            lastPreemptedNonce = res.preemptedNonce
            floorStatus = FabushiMultiAgentFloorStatusDTO(
                chatId: res.chatId,
                granted: res.currentSpeaker != nil,
                currentSpeaker: res.currentSpeaker,
                queuedCount: res.queuedCount,
                activeGrant: nil
            )
            return res
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func releaseFloor(agentId: String) async -> FabushiMultiAgentFloorReleaseDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        let trimmed = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let agent_id: String
        }
        do {
            let res: FabushiMultiAgentFloorReleaseDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("floorRelease"),
                params: Params(chat_id: chatId, agent_id: trimmed)
            )
            await loadFloorStatus()
            return res
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - 5-Hop Loop Budget (`multi_agent.loopBudget*`)

    public func loadLoopBudgetStatus() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        do {
            loopBudget = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("loopBudgetStatus"),
                params: Params(chat_id: chatId)
            )
        } catch {
            handleError(error)
        }
    }

    @discardableResult
    public func recordLoopHop(
        agentId: String,
        maxHops: UInt32 = FabushiMultiAgentConstants.defaultMaxLoopHops
    ) async -> FabushiMultiAgentLoopBudgetStatusDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let agent_id: String
            let max_hops: UInt32
        }
        do {
            let res: FabushiMultiAgentLoopBudgetStatusDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("loopBudgetRecordHop"),
                params: Params(chat_id: chatId, agent_id: agentId, max_hops: maxHops)
            )
            loopBudget = res
            return res
        } catch let hostError as FabushiHostError {
            if hostError.code == "loop_budget_exceeded" {
                lastErrorMessage = hostError.localizedDescription
                await loadLoopBudgetStatus()
                return nil
            }
            handleError(hostError)
            return nil
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func resetLoopBudget() async -> Bool {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return false }

        struct Params: Encodable, Sendable { let chat_id: Int64 }
        do {
            let res: FabushiMultiAgentLoopBudgetResetDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("loopBudgetReset"),
                params: Params(chat_id: chatId)
            )
            await loadLoopBudgetStatus()
            return res.reset
        } catch {
            handleError(error)
            return false
        }
    }

    // MARK: - Revision-Fenced `SharedGroupWorkspace` (`multi_agent.workspace*`)

    public func loadWorkspaceSnapshot() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        do {
            workspace = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("workspaceSnapshot"),
                params: Params(chat_id: chatId)
            )
        } catch {
            handleError(error)
        }
    }

    public func getWorkspaceEntry(key: String) async -> FabushiMultiAgentWorkspaceEntryDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let key: String
        }
        do {
            let entry: FabushiMultiAgentWorkspaceEntryDTO? = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("workspaceGet"),
                params: Params(chat_id: chatId, key: key)
            )
            return entry
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func setWorkspaceEntry(
        key: String,
        value: FabushiJSONValue,
        updatedBy: String? = nil,
        expectedRevision: UInt64? = nil
    ) async -> FabushiMultiAgentWorkspaceSetResponseDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        let trimmedKey = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedKey.isEmpty else { return nil }

        let author: String
        if let updatedBy, !updatedBy.isEmpty {
            author = updatedBy
        } else if case .authenticated(_, let username, _) = coordinator.authState {
            author = username
        } else {
            author = "user"
        }

        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let key: String
            let value: FabushiJSONValue
            let updated_by: String
            let expected_revision: UInt64?
        }

        do {
            let res: FabushiMultiAgentWorkspaceSetResponseDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("workspaceSet"),
                params: Params(
                    chat_id: chatId,
                    key: trimmedKey,
                    value: value,
                    updated_by: author,
                    expected_revision: expectedRevision
                )
            )
            workspace = res.snapshot
            workspaceConflict = nil
            return res
        } catch let hostError as FabushiHostError {
            if case .conflict(let message) = hostError {
                workspaceConflict = FabushiMultiAgentWorkspaceConflictState(
                    key: trimmedKey,
                    pendingValue: value,
                    expectedRevision: expectedRevision ?? workspace.revision,
                    message: message
                )
                lastErrorMessage = message
                return nil
            }
            handleError(hostError)
            return nil
        } catch {
            handleError(error)
            return nil
        }
    }

    public func dismissWorkspaceConflictAndReload() async {
        workspaceConflict = nil
        await loadWorkspaceSnapshot()
    }

    @discardableResult
    public func overwriteWorkspaceConflict() async -> Bool {
        guard let conflict = workspaceConflict else { return false }
        await loadWorkspaceSnapshot()
        let latestRev = workspace.revision
        let result = await setWorkspaceEntry(
            key: conflict.key,
            value: conflict.pendingValue,
            expectedRevision: latestRev
        )
        return result != nil
    }

    @discardableResult
    public func removeWorkspaceEntry(key: String) async -> FabushiMultiAgentWorkspaceEntryDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let key: String
        }
        do {
            let res: FabushiMultiAgentWorkspaceRemoveResponseDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("workspaceRemove"),
                params: Params(chat_id: chatId, key: key)
            )
            await loadWorkspaceSnapshot()
            return res.removed
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - Group Wake Gate & Stay-Silent `[PASS]` (`D14-01`)

    @discardableResult
    public func evaluateWakeGate(
        _ input: FabushiMultiAgentWakeGateInputDTO
    ) async -> FabushiMultiAgentWakeGateDecisionDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        do {
            let decision: FabushiMultiAgentWakeGateDecisionDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("evaluateWakeGate"),
                params: input
            )
            lastWakeDecision = decision
            return decision
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func evaluateOutboundReview(
        _ input: FabushiMultiAgentOutboundReviewInputDTO
    ) async -> FabushiMultiAgentOutboundVerdictDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        do {
            let verdict: FabushiMultiAgentOutboundVerdictDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("evaluateOutboundReview"),
                params: input
            )
            lastOutboundVerdict = verdict
            return verdict
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func checkIsPassSentinel(_ text: String) async -> Bool? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct Params: Encodable, Sendable { let text: String }
        do {
            let res: FabushiMultiAgentPassSentinelResultDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("isPassSentinel"),
                params: Params(text: text)
            )
            lastPassSentinelResult = res.isPass
            return res.isPass
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - Scoped Ed25519 Capability Tokens

    @discardableResult
    public func issueCapabilityToken(
        agentId: String,
        allowedActions: [String],
        ttlSecs: Int64 = 300
    ) async -> FabushiMultiAgentIssuedCapabilityTokenDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct Params: Encodable, Sendable {
            let agent_id: String
            let chat_id: Int64
            let allowed_actions: [String]
            let ttl_secs: Int64
        }
        do {
            let issued: FabushiMultiAgentIssuedCapabilityTokenDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("issueCapabilityToken"),
                params: Params(
                    agent_id: agentId,
                    chat_id: chatId,
                    allowed_actions: allowedActions,
                    ttl_secs: ttlSecs
                )
            )
            lastIssuedToken = issued
            return issued
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func verifyCapabilityToken(
        token: FabushiMultiAgentCapabilityTokenDTO,
        verifyingKeyHex: String,
        nowTs: Int64? = nil,
        agentId: String? = nil,
        action: String? = nil
    ) async -> FabushiMultiAgentVerifyCapabilityResultDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct Params: Encodable, Sendable {
            let token: FabushiMultiAgentCapabilityTokenDTO
            let verifying_key_hex: String
            let now_ts: Int64?
            let agent_id: String?
            let chat_id: Int64?
            let action: String?
        }
        do {
            let verified: FabushiMultiAgentVerifyCapabilityResultDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.multiAgent.method("verifyCapabilityToken"),
                params: Params(
                    token: token,
                    verifying_key_hex: verifyingKeyHex,
                    now_ts: nowTs,
                    agent_id: agentId,
                    chat_id: agentId != nil ? chatId : nil,
                    action: action
                )
            )
            lastTokenVerification = verified
            return verified
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - Helpers

    private func handleError(_ error: Error) {
        if let hostError = error as? FabushiHostError, hostError.isCancelled {
            return
        }
        if let coordError = error as? FabushiCoordinatorError,
           case .staleGeneration = coordError
        {
            syncAccountScopeIfNeeded()
            return
        }
        lastErrorMessage = error.localizedDescription
    }
}
