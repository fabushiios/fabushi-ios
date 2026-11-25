import Foundation
import Observation

/// Canonical `@MainActor @Observable` ViewModel backing the 7-tab Agent Studio workspace.
/// Dispatches strictly through `FabushiCoordinator.dispatchFenced(...)`, filters host push
/// events by `accountScopeGeneration`, applies `D11-05` secret redaction to all displayed
/// session trajectories, enforces `D10-10` 3-tier AI error classification, and wipes all
/// account-scoped state immediately on logout or account/server switch.
@MainActor
@Observable
public final class FabushiAgentStudioViewModel {
    // MARK: - Navigation & Loading State

    public var selectedTab: FabushiStudioTab = .overview
    public private(set) var isLoadingOverview: Bool = false
    public private(set) var isLoadingKanban: Bool = false
    public private(set) var isLoadingSkills: Bool = false
    public private(set) var isLoadingMemory: Bool = false
    public private(set) var isLoadingCron: Bool = false
    public private(set) var isLoadingSessions: Bool = false
    public private(set) var isLoadingApprovals: Bool = false

    // MARK: - Overview, Commands, Toolsets & Bots

    public private(set) var localStatus: FabushiStudioStatusResponse?
    public private(set) var remoteStatus: FabushiStudioStatusResponse?
    public private(set) var commands: [FabushiStudioCommandDescriptorDTO] = []
    public private(set) var toolsetSummary: FabushiStudioToolsetSummaryDTO?
    public private(set) var bots: [FabushiStudioBotSummaryDTO] = []
    public private(set) var sandboxNoticeMessage: String?

    // MARK: - Tasks FSM & Kanban

    public private(set) var tasks: [FabushiStudioTaskFsmDTO] = []
    public private(set) var kanbanBoards: [FabushiStudioKanbanBoardDTO] = []
    public var selectedKanbanBoardId: String?
    public private(set) var kanbanCards: [FabushiStudioKanbanCardDTO] = []

    // MARK: - Skills & Distilled `script_app` (`D11-04`)

    public var skillFilterQuery: String = ""
    public private(set) var skills: [FabushiStudioSkillDocumentDTO] = []
    public private(set) var lastPreflightResult: FabushiStudioScriptAppPreflightResult?
    public private(set) var lastReplayNotice: String?

    // MARK: - Memory, User, Soul, Honcho & Quarantine

    public private(set) var memoryState: FabushiStudioMemoryReadDTO?
    public private(set) var userMemoryState: FabushiStudioMemoryReadDTO?
    public private(set) var soulProfile: FabushiStudioSoulProfileDTO?
    public private(set) var honchoProfile: FabushiStudioHonchoProfileDTO?
    public private(set) var lastHonchoQueryResult: FabushiStudioHonchoQueryResultDTO?
    public private(set) var quarantineProposals: [FabushiStudioQuarantineProposalDTO] = []

    // MARK: - Cron

    public private(set) var cronJobs: [FabushiStudioCronJobDTO] = []
    public private(set) var cronPreview: FabushiStudioCronPreviewDTO?

    // MARK: - Sessions & Redacted Trajectories (`D11-05`)

    public private(set) var sessions: [FabushiStudioSessionDTO] = []
    public private(set) var inspectedSession: FabushiStudioSessionDTO?
    public private(set) var inspectedSessionRedactedMetadata: String = ""
    public private(set) var sessionSearchHits: [FabushiStudioSessionSearchHitDTO] = []
    public private(set) var lastSessionRedactionCount: Int = 0

    // MARK: - Approvals, Value-Bound Ledger & Browser Takeover (`D11-03`)

    public var approvalsPendingOnly: Bool = false
    public private(set) var approvals: [FabushiStudioApprovalRecordDTO] = []
    public private(set) var liveToolApprovalRequests: [FabushiStudioLiveToolApprovalRequest] = []
    public private(set) var valueBoundTokens: [FabushiStudioValueBoundTokenDTO] = []
    public private(set) var browserTakeoverRequests: [FabushiStudioBrowserTakeoverRequest] = []

    // MARK: - Error & Scope State

    public private(set) var lastClassifiedError: FabushiMagpieClassifiedError?
    public private(set) var lastErrorMessage: String?
    public private(set) var lastObservedGeneration: UInt64 = 0
    public private(set) var lastObservedAccountId: String?

    // MARK: - Non-Observed Dependencies

    @ObservationIgnored public let coordinator: FabushiCoordinator
    @ObservationIgnored public let router: ShellNavigationRouter
    @ObservationIgnored public let cleanupHub: FabushiAgentStudioLogoutCleanupHub
    @ObservationIgnored private let taskBag = FabushiCancellableTaskBag()
    @ObservationIgnored private let cleanupHandlerId: String
    @ObservationIgnored private let redactor = FabushiStudioSecretRedactor()
    @ObservationIgnored private var isEventStreamActive = false

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub()
    ) {
        self.coordinator = coordinator
        self.router = router
        self.cleanupHub = cleanupHub
        self.cleanupHandlerId = "studio.vm.\(UUID().uuidString)"
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

    // MARK: - Lifecycle, Generation Fencing & Logout Cleanup

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

        isLoadingOverview = false
        isLoadingKanban = false
        isLoadingSkills = false
        isLoadingMemory = false
        isLoadingCron = false
        isLoadingSessions = false
        isLoadingApprovals = false

        localStatus = nil
        remoteStatus = nil
        commands = []
        toolsetSummary = nil
        bots = []
        sandboxNoticeMessage = nil

        tasks = []
        kanbanBoards = []
        selectedKanbanBoardId = nil
        kanbanCards = []

        skillFilterQuery = ""
        skills = []
        lastPreflightResult = nil
        lastReplayNotice = nil

        memoryState = nil
        userMemoryState = nil
        soulProfile = nil
        honchoProfile = nil
        lastHonchoQueryResult = nil
        quarantineProposals = []

        cronJobs = []
        cronPreview = nil

        sessions = []
        inspectedSession = nil
        inspectedSessionRedactedMetadata = ""
        sessionSearchHits = []
        lastSessionRedactionCount = 0

        approvalsPendingOnly = false
        approvals = []
        liveToolApprovalRequests = []
        valueBoundTokens = []
        browserTakeoverRequests = []

        lastClassifiedError = nil
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
        taskBag.replace(key: "studio.events", with: task)
    }

    public func ingestDomainEvent(_ event: FabushiDomainEvent) {
        // Strict generation fence matching FabushiCoordinator.ingestDomainEvent.
        guard event.generation >= coordinator.accountScopeGeneration else {
            return
        }

        switch event.kind {
        case .taskStateUpdated(let taskId, let stateRaw):
            if let parsed = FabushiStudioTaskState.parse(stateRaw) {
                if let idx = tasks.firstIndex(where: { $0.taskId == taskId }) {
                    let existing = tasks[idx]
                    tasks[idx] = FabushiStudioTaskFsmDTO(
                        taskId: existing.taskId,
                        state: parsed,
                        stateHistory: existing.stateHistory,
                        abortReason: existing.abortReason
                    )
                } else {
                    tasks.insert(FabushiStudioTaskFsmDTO(taskId: taskId, state: parsed), at: 0)
                }
            }

        case .toolApprovalRequested(let approvalId, let toolName, let summary):
            let redactedSummary = redactor.redactText(summary).redactedText
            let item = FabushiStudioLiveToolApprovalRequest(
                approvalId: approvalId,
                toolName: toolName,
                summary: redactedSummary,
                receivedAtMs: event.timestampMs
            )
            if let idx = liveToolApprovalRequests.firstIndex(where: { $0.approvalId == approvalId }) {
                liveToolApprovalRequests[idx] = item
            } else {
                liveToolApprovalRequests.insert(item, at: 0)
            }

        case .serverError(let code, let message, _):
            let combined = code.isEmpty ? message : "\(code): \(message)"
            handleClassifiedError(rawMessage: combined, needsProviderCta: code == "needs_provider_selection")

        case .raw(let type, let json):
            ingestRawBrowserEventIfApplicable(type: type, rawJSON: json, timestampMs: event.timestampMs)

        default:
            break
        }
    }

    private func ingestRawBrowserEventIfApplicable(type: String, rawJSON: String, timestampMs: Int64) {
        let lowerType = type.lowercased()
        guard lowerType.contains("browser_takeover") || lowerType.contains("browser_control") else {
            return
        }
        guard let data = rawJSON.data(using: .utf8),
              let payload = try? JSONDecoder().decode(FabushiJSONValue.self, from: data),
              let sid = payload["session_id"]?.stringValue, !sid.isEmpty
        else {
            return
        }
        let reason = payload["reason"]?.stringValue ?? "User verification requested"
        let modeRaw = payload["mode"]?.stringValue ?? "waiting_for_takeover"
        let mode: FabushiStudioBrowserControlMode
        switch modeRaw {
        case "user_takeover":
            mode = .userTakeover(pausedAtStep: Int(payload["paused_at_step"]?.int64Value ?? 0))
        case "ai_controlling":
            mode = .aiControlling(
                activeStep: Int(payload["active_step"]?.int64Value ?? 0),
                statusText: payload["status_text"]?.stringValue ?? ""
            )
        case "idle":
            mode = .idle
        default:
            mode = .waitingForTakeover(reason: reason)
        }
        let req = FabushiStudioBrowserTakeoverRequest(
            sessionId: sid,
            reason: redactor.redactText(reason).redactedText,
            mode: mode,
            updatedAtMs: timestampMs
        )
        if let idx = browserTakeoverRequests.firstIndex(where: { $0.sessionId == sid }) {
            browserTakeoverRequests[idx] = req
        } else {
            browserTakeoverRequests.insert(req, at: 0)
        }
    }

    // MARK: - Overview, Status, Commands & 29 Toolsets

    public func refreshActiveTab() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        startEventSubscriptionIfNeeded()

        switch selectedTab {
        case .overview:
            await loadOverview()
        case .kanban:
            await loadTasksAndKanban()
        case .skills:
            await loadSkills(filter: skillFilterQuery.isEmpty ? nil : skillFilterQuery)
        case .memory:
            await loadMemoryAndSoul()
        case .cron:
            await loadCronJobs()
        case .approvals:
            await loadApprovals()
        case .sessions:
            await loadSessions()
        }
    }

    public func loadOverview() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoadingOverview = true
        defer { isLoadingOverview = false }

        do {
            struct StatusParams: Encodable, Sendable { let remote: Bool }
            let local: FabushiStudioStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("status"),
                params: StatusParams(remote: false)
            )
            localStatus = local
            if let summary = local.toolsets {
                toolsetSummary = summary
            }

            let cmdRes: FabushiStudioListCommandsResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("listCommands"),
                params: FabushiEmptyParams()
            )
            commands = cmdRes.commands

            let tsRes: FabushiStudioListToolsetsResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("listToolsets"),
                params: FabushiEmptyParams()
            )
            toolsetSummary = tsRes.summary

            if coordinator.authState != .unauthenticated {
                if let remote: FabushiStudioStatusResponse = try? await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.fabushi.method("status"),
                    params: StatusParams(remote: true)
                ), remote.isRemotePayload {
                    remoteStatus = remote
                    if remote.needsProviderSelection == true {
                        handleClassifiedError(
                            rawMessage: "needs_provider_selection: No configured provider on connected server",
                            needsProviderCta: true
                        )
                    }
                }

                if let fetchedBots: [FabushiStudioBotSummaryDTO] = try? await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.fabushi.method("botsList"),
                    params: FabushiEmptyParams()
                ) {
                    bots = fetchedBots
                }
            }
        } catch {
            handleRPCFailure(error)
        }
    }

    @discardableResult
    public func setToolsetEnabled(
        _ toolsetId: String,
        enabled: Bool,
        locale: FabushiLocale = .en
    ) async -> Bool {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return false }

        struct UpdateParams: Encodable, Sendable {
            let enable: [String]?
            let disable: [String]?
        }

        do {
            let updated: FabushiStudioToolsetSummaryDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("updateToolsets"),
                params: UpdateParams(
                    enable: enabled ? [toolsetId] : nil,
                    disable: enabled ? nil : [toolsetId]
                )
            )
            toolsetSummary = updated
            sandboxNoticeMessage = nil
            router.toastController.showNonChatToast(
                FabushiAgentStudioStrings.text("m4.studio.toolsets.updatedToast", locale: locale),
                isError: false
            )
            return true
        } catch let hostError as FabushiHostError {
            if case .capabilityUnavailable(let message) = hostError {
                sandboxNoticeMessage = message
                router.toastController.showNonChatToast(message, isError: false)
                return false
            }
            handleRPCFailure(hostError)
            return false
        } catch {
            handleRPCFailure(error)
            return false
        }
    }

    @discardableResult
    public func applyToolsetPreset(_ presetId: String, locale: FabushiLocale = .en) async -> Bool {
        await setToolsetEnabled(presetId, enabled: true, locale: locale)
    }

    // MARK: - Tasks FSM & Kanban Boards/Cards

    public func loadTasksAndKanban() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoadingKanban = true
        defer { isLoadingKanban = false }

        do {
            let fetchedTasks: [FabushiStudioTaskFsmDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("listTasks"),
                params: FabushiEmptyParams()
            )
            tasks = fetchedTasks

            let boards: [FabushiStudioKanbanBoardDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("kanbanBoards"),
                params: FabushiEmptyParams()
            )
            kanbanBoards = boards

            if let selected = selectedKanbanBoardId,
               boards.contains(where: { $0.id == selected })
            {
                await loadKanbanCards(boardId: selected)
            } else if let first = boards.first {
                selectedKanbanBoardId = first.id
                await loadKanbanCards(boardId: first.id)
            } else {
                selectedKanbanBoardId = nil
                kanbanCards = []
            }
        } catch {
            handleRPCFailure(error)
        }
    }

    @discardableResult
    public func createTask(taskId: String? = nil) async -> FabushiStudioTaskFsmDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct CreateParams: Encodable, Sendable { let task_id: String? }
        do {
            let trimmed = taskId?.trimmingCharacters(in: .whitespacesAndNewlines)
            let created: FabushiStudioTaskFsmDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("createTask"),
                params: CreateParams(task_id: (trimmed?.isEmpty ?? true) ? nil : trimmed)
            )
            upsertTask(created)
            return created
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func transitionTask(taskId: String, to state: FabushiStudioTaskState) async -> FabushiStudioTaskFsmDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct TransitionParams: Encodable, Sendable {
            let task_id: String
            let state: String
        }
        do {
            let updated: FabushiStudioTaskFsmDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("transitionTask"),
                params: TransitionParams(task_id: taskId, state: state.rpcTransitionValue)
            )
            upsertTask(updated)
            return updated
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func approveTask(taskId: String, approved: Bool, remote: Bool = false) async -> FabushiStudioTaskFsmDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct ApproveParams: Encodable, Sendable {
            let task_id: String
            let approved: Bool
            let remote: Bool
        }
        do {
            let updated: FabushiStudioTaskFsmDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("approveTask"),
                params: ApproveParams(task_id: taskId, approved: approved, remote: remote)
            )
            upsertTask(updated)
            return updated
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func createKanbanBoard(
        name: String,
        description: String = "",
        columns: [String]? = nil
    ) async -> FabushiStudioKanbanBoardDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        struct CreateBoardParams: Encodable, Sendable {
            let name: String
            let description: String
            let columns: [String]?
        }
        do {
            let board: FabushiStudioKanbanBoardDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("kanbanCreateBoard"),
                params: CreateBoardParams(name: trimmed, description: description, columns: columns)
            )
            kanbanBoards.append(board)
            selectedKanbanBoardId = board.id
            kanbanCards = []
            return board
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    public func loadKanbanCards(boardId: String, columnName: String? = nil) async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }

        struct ListCardsParams: Encodable, Sendable {
            let board_id: String
            let column_name: String?
        }
        do {
            let cards: [FabushiStudioKanbanCardDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("kanbanListCards"),
                params: ListCardsParams(board_id: boardId, column_name: columnName)
            )
            kanbanCards = cards
        } catch {
            handleRPCFailure(error)
        }
    }

    @discardableResult
    public func createKanbanCard(
        boardId: String,
        columnName: String,
        title: String,
        description: String = "",
        priority: String = "medium",
        assignee: String? = nil,
        labels: [String] = []
    ) async -> FabushiStudioKanbanCardDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        struct CreateCardParams: Encodable, Sendable {
            let board_id: String
            let column_name: String
            let title: String
            let description: String
            let priority: String
            let assignee: String?
            let labels: [String]
        }
        do {
            let card: FabushiStudioKanbanCardDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("kanbanCreateCard"),
                params: CreateCardParams(
                    board_id: boardId,
                    column_name: columnName,
                    title: trimmed,
                    description: description,
                    priority: priority,
                    assignee: assignee,
                    labels: labels
                )
            )
            kanbanCards.append(card)
            return card
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func moveKanbanCard(
        cardId: String,
        targetColumn: String,
        position: Int64? = nil
    ) async -> FabushiStudioKanbanCardDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct MoveCardParams: Encodable, Sendable {
            let card_id: String
            let target_column: String
            let position: Int64?
        }
        do {
            let updated: FabushiStudioKanbanCardDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("kanbanMoveCard"),
                params: MoveCardParams(card_id: cardId, target_column: targetColumn, position: position)
            )
            if let idx = kanbanCards.firstIndex(where: { $0.id == cardId }) {
                kanbanCards[idx] = updated
            }
            return updated
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func deleteKanbanCard(cardId: String) async -> Bool {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return false }

        struct DeleteCardParams: Encodable, Sendable { let card_id: String }
        do {
            let res: FabushiStudioDeleteCardResultDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("kanbanDeleteCard"),
                params: DeleteCardParams(card_id: cardId)
            )
            if res.deleted {
                kanbanCards.removeAll { $0.id == cardId }
            }
            return res.deleted
        } catch {
            handleRPCFailure(error)
            return false
        }
    }

    // MARK: - Skills & Distilled `script_app` (`D11-04`)

    public func loadSkills(filter: String? = nil) async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoadingSkills = true
        defer { isLoadingSkills = false }

        struct ListParams: Encodable, Sendable { let filter: String? }
        do {
            let trimmed = filter?.trimmingCharacters(in: .whitespacesAndNewlines)
            let fetched: [FabushiStudioSkillDocumentDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("skillsList"),
                params: ListParams(filter: (trimmed?.isEmpty ?? true) ? nil : trimmed)
            )
            skills = fetched
        } catch {
            handleRPCFailure(error)
        }
    }

    @discardableResult
    public func createSkill(
        name: String,
        description: String,
        body: String,
        platforms: [String] = ["ios", "macos"],
        toolsets: [String] = [],
        tags: [String] = [],
        version: String = "1.0.0",
        origin: FabushiStudioSkillOrigin = .user,
        trusted: Bool = true
    ) async -> FabushiStudioSkillDocumentDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct CreateParams: Encodable, Sendable {
            let name: String
            let description: String
            let body: String
            let platforms: [String]
            let toolsets: [String]
            let tags: [String]
            let version: String
            let origin: FabushiStudioSkillOrigin
            let trusted: Bool
        }
        do {
            let doc: FabushiStudioSkillDocumentDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("skillSave"),
                params: CreateParams(
                    name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                    description: description,
                    body: body,
                    platforms: platforms,
                    toolsets: toolsets,
                    tags: tags,
                    version: version,
                    origin: origin,
                    trusted: trusted
                )
            )
            upsertSkill(doc)
            return doc
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func saveSkillMarkdown(_ markdown: String) async -> FabushiStudioSkillDocumentDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct MarkdownParams: Encodable, Sendable { let markdown: String }
        do {
            let doc: FabushiStudioSkillDocumentDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("skillSave"),
                params: MarkdownParams(markdown: markdown)
            )
            upsertSkill(doc)
            return doc
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func patchSkill(name: String, oldText: String, newText: String) async -> FabushiStudioSkillDocumentDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct PatchParams: Encodable, Sendable {
            let name: String
            let old_text: String
            let new_text: String
        }
        do {
            let doc: FabushiStudioSkillDocumentDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("skillPatch"),
                params: PatchParams(name: name, old_text: oldText, new_text: newText)
            )
            upsertSkill(doc)
            return doc
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func deleteSkill(name: String) async -> Bool {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return false }

        struct DeleteParams: Encodable, Sendable { let name: String }
        do {
            let res: FabushiStudioDeleteSkillResultDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("skillDelete"),
                params: DeleteParams(name: name)
            )
            if res.deleted {
                skills.removeAll { $0.frontmatter.name == name }
            }
            return res.deleted
        } catch {
            handleRPCFailure(error)
            return false
        }
    }

    @discardableResult
    public func validateScriptAppForPublish(
        _ app: FabushiStudioAutomationScriptApp
    ) -> FabushiStudioScriptAppPreflightResult {
        let result = FabushiStudioScriptAppPreflightValidator.validate(app)
        lastPreflightResult = result
        return result
    }

    /// Dispatches deterministic replay via authenticated server `/run_script <slug>` when signed in,
    /// or surfaces an honest iOS capability notice when offline / browser toolset is host-gated.
    @discardableResult
    public func runDistilledScriptApp(_ skill: FabushiStudioSkillDocumentDTO, locale: FabushiLocale = .en) async -> Bool {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return false }

        guard coordinator.authState != .unauthenticated else {
            let notice = FabushiAgentStudioStrings.text("m4.studio.skills.replayNoticeIos", locale: locale)
            lastReplayNotice = notice
            router.toastController.showNonChatToast(notice, isError: false)
            return false
        }

        struct RunCommandParams: Encodable, Sendable { let command: String }
        do {
            let res: FabushiStudioRunCommandResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("runCommand"),
                params: RunCommandParams(command: "/run_script \(skill.frontmatter.name)")
            )
            let cleanedOutput = redactor.redactText(res.output.isEmpty ? (res.replyText ?? "") : res.output).redactedText
            lastReplayNotice = cleanedOutput.isEmpty
                ? FabushiAgentStudioStrings.text("m4.studio.skills.zeroLlmBadge", locale: locale)
                : cleanedOutput
            router.toastController.showNonChatToast(lastReplayNotice ?? "", isError: false)
            return res.handled
        } catch {
            let fallback = FabushiAgentStudioStrings.text("m4.studio.skills.replayNoticeIos", locale: locale)
            lastReplayNotice = fallback
            handleRPCFailure(error)
            return false
        }
    }

    // MARK: - Memory, Soul, Honcho & Quarantine

    public func loadMemoryAndSoul(honchoUserId: String? = nil) async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoadingMemory = true
        defer { isLoadingMemory = false }

        struct TargetParams: Encodable, Sendable { let target: String }
        struct HonchoParams: Encodable, Sendable { let user_id: String }
        let resolvedUser = honchoUserId ?? coordinator.activeAccountId ?? "default"

        do {
            memoryState = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("memoryGet"),
                params: TargetParams(target: FabushiStudioMemoryTarget.memory.rawValue)
            )
            userMemoryState = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("memoryGet"),
                params: TargetParams(target: FabushiStudioMemoryTarget.user.rawValue)
            )
            soulProfile = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("soulGet"),
                params: FabushiEmptyParams()
            )
            honchoProfile = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("honchoGet"),
                params: HonchoParams(user_id: resolvedUser)
            )
            quarantineProposals = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("memoryQuarantineList"),
                params: FabushiEmptyParams()
            )
        } catch {
            handleRPCFailure(error)
        }
    }

    @discardableResult
    public func saveMemory(target: FabushiStudioMemoryTarget, content: String) async -> FabushiStudioMemoryMutationDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct SaveParams: Encodable, Sendable {
            let target: String
            let content: String
        }
        do {
            let res: FabushiStudioMemoryMutationDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("memorySave"),
                params: SaveParams(target: target.rawValue, content: content)
            )
            applyMemoryMutation(res, target: target)
            return res
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func mutateMemory(
        target: FabushiStudioMemoryTarget,
        op: String,
        entry: String? = nil,
        matchSubstr: String? = nil
    ) async -> FabushiStudioMemoryMutationDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct MutateParams: Encodable, Sendable {
            let target: String
            let op: String
            let entry: String?
            let match_substr: String?
        }
        do {
            let res: FabushiStudioMemoryMutationDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("memoryMutate"),
                params: MutateParams(
                    target: target.rawValue,
                    op: op,
                    entry: entry,
                    match_substr: matchSubstr
                )
            )
            applyMemoryMutation(res, target: target)
            return res
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func promoteQuarantineProposal(_ proposal: FabushiStudioQuarantineProposalDTO) async -> Bool {
        let target: FabushiStudioMemoryTarget = proposal.target.lowercased() == "user" ? .user : .memory
        let result = await mutateMemory(target: target, op: "add", entry: proposal.content)
        return result != nil
    }

    @discardableResult
    public func saveSoul(markdown: String) async -> FabushiStudioSoulProfileDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        let trimmed = markdown.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        struct SoulSaveParams: Encodable, Sendable { let markdown: String }
        do {
            let updated: FabushiStudioSoulProfileDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("soulSave"),
                params: SoulSaveParams(markdown: trimmed)
            )
            soulProfile = updated
            return updated
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func queryHoncho(userId: String? = nil, question: String) async -> FabushiStudioHonchoQueryResultDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        let trimmed = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        struct QueryParams: Encodable, Sendable {
            let user_id: String
            let question: String
        }
        do {
            let res: FabushiStudioHonchoQueryResultDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("honchoPost"),
                params: QueryParams(
                    user_id: userId ?? coordinator.activeAccountId ?? "default",
                    question: trimmed
                )
            )
            lastHonchoQueryResult = res
            return res
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    // MARK: - Cron Schedules

    public func loadCronJobs(enabledOnly: Bool = false) async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoadingCron = true
        defer { isLoadingCron = false }

        struct ListParams: Encodable, Sendable { let enabled_only: Bool }
        do {
            cronJobs = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("cronList"),
                params: ListParams(enabled_only: enabledOnly)
            )
        } catch {
            handleRPCFailure(error)
        }
    }

    @discardableResult
    public func previewCron(schedule: String) async -> FabushiStudioCronPreviewDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct PreviewParams: Encodable, Sendable { let schedule: String }
        do {
            let res: FabushiStudioCronPreviewDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("cronPreview"),
                params: PreviewParams(schedule: schedule.trimmingCharacters(in: .whitespacesAndNewlines))
            )
            cronPreview = res
            return res
        } catch {
            cronPreview = nil
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func saveCronJob(
        name: String,
        schedule: String,
        prompt: String,
        timezone: String? = nil,
        model: String? = nil,
        skills: [String] = [],
        enabled: Bool = true
    ) async -> FabushiStudioCronJobDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct SaveCronParams: Encodable, Sendable {
            let name: String
            let schedule: String
            let timezone: String?
            let prompt: String
            let model: String?
            let skills: [String]
            let enabled: Bool
        }
        do {
            let job: FabushiStudioCronJobDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("cronSave"),
                params: SaveCronParams(
                    name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                    schedule: schedule.trimmingCharacters(in: .whitespacesAndNewlines),
                    timezone: timezone,
                    prompt: prompt,
                    model: model,
                    skills: skills,
                    enabled: enabled
                )
            )
            cronJobs.insert(job, at: 0)
            return job
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func deleteCronJob(jobId: String) async -> Bool {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return false }

        struct DeleteParams: Encodable, Sendable { let job_id: String }
        do {
            let res: FabushiStudioDeleteCronResultDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("cronDelete"),
                params: DeleteParams(job_id: jobId)
            )
            if res.deleted {
                cronJobs.removeAll { $0.id == jobId }
            }
            return res.deleted
        } catch {
            handleRPCFailure(error)
            return false
        }
    }

    // MARK: - Sessions & `D11-05` Redacted Trajectory Search

    public func loadSessions() async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoadingSessions = true
        defer { isLoadingSessions = false }

        do {
            sessions = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("sessionsList"),
                params: FabushiEmptyParams()
            )
        } catch {
            handleRPCFailure(error)
        }
    }

    @discardableResult
    public func createSession(
        title: String,
        model: String = "hermes-local",
        source: String = "ios_studio"
    ) async -> FabushiStudioSessionDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct CreateParams: Encodable, Sendable {
            let title: String
            let model: String
            let source: String
        }
        do {
            let created: FabushiStudioSessionDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("sessionCreate"),
                params: CreateParams(
                    title: title.trimmingCharacters(in: .whitespacesAndNewlines),
                    model: model,
                    source: source
                )
            )
            sessions.insert(created, at: 0)
            return created
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func inspectSession(sessionId: String) async -> FabushiStudioSessionDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct GetParams: Encodable, Sendable { let session_id: String }
        do {
            let session: FabushiStudioSessionDTO? = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("sessionGet"),
                params: GetParams(session_id: sessionId)
            )
            inspectedSession = session
            if let session {
                let metaRedaction = redactor.redactText(session.metadataJson)
                inspectedSessionRedactedMetadata = metaRedaction.redactedText
                lastSessionRedactionCount = metaRedaction.totalRedactions
            } else {
                inspectedSessionRedactedMetadata = ""
            }
            return session
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    public func searchSessions(query: String, limit: UInt64 = 20) async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            sessionSearchHits = []
            return
        }

        struct SearchParams: Encodable, Sendable {
            let query: String
            let limit: UInt64
        }
        do {
            let rawHits: [FabushiStudioSessionSearchHitDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("sessionSearch"),
                params: SearchParams(query: trimmed, limit: limit)
            )
            var totalRedactions = 0
            sessionSearchHits = rawHits.map { hit in
                let redactedSnippet = redactor.redactText(hit.snippet)
                let redactedContent = redactor.redactText(hit.content)
                totalRedactions += redactedSnippet.totalRedactions + redactedContent.totalRedactions
                return FabushiStudioSessionSearchHitDTO(
                    sessionId: hit.sessionId,
                    sessionTitle: hit.sessionTitle,
                    messageId: hit.messageId,
                    seq: hit.seq,
                    role: hit.role,
                    snippet: redactedSnippet.redactedText,
                    content: redactedContent.redactedText,
                    createdAt: hit.createdAt,
                    backend: hit.backend
                )
            }
            lastSessionRedactionCount = totalRedactions
        } catch {
            handleRPCFailure(error)
        }
    }

    // MARK: - Approvals, Value-Bound Ledger & Browser Takeover

    public func loadApprovals(pendingOnly: Bool? = nil) async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoadingApprovals = true
        defer { isLoadingApprovals = false }

        let filterPending = pendingOnly ?? approvalsPendingOnly
        struct ListParams: Encodable, Sendable {
            let session_id: String?
            let pending_only: Bool
        }
        do {
            approvals = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("approvalsList"),
                params: ListParams(session_id: nil, pending_only: filterPending)
            )
        } catch {
            handleRPCFailure(error)
        }
    }

    @discardableResult
    public func enqueueApproval(
        sessionId: String,
        command: String,
        riskLevel: String,
        patternName: String
    ) async -> FabushiStudioApprovalRecordDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct EnqueueParams: Encodable, Sendable {
            let session_id: String
            let command: String
            let risk_level: String
            let pattern_name: String
        }
        do {
            let rec: FabushiStudioApprovalRecordDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("approvalEnqueue"),
                params: EnqueueParams(
                    session_id: sessionId,
                    command: command,
                    risk_level: riskLevel,
                    pattern_name: patternName
                )
            )
            approvals.insert(rec, at: 0)
            return rec
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    /// Resolves a SQLite approval record using exact Rust wire values (`approved_once`,
    /// `approved_session`, `approved_permanent`, or `denied` — NEVER bare `"approved"`!).
    @discardableResult
    public func decideApproval(
        approvalId: String,
        decision: FabushiStudioApprovalDecisionWire,
        remote: Bool = false,
        reason: String? = nil
    ) async -> FabushiStudioApprovalRecordDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        do {
            if remote && coordinator.authState != .unauthenticated {
                struct RemoteDecideParams: Encodable, Sendable {
                    let remote: Bool
                    let approval_id: String
                    let decision: String
                    let reason: String?
                }
                let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.fabushi.method("approvalDecide"),
                    params: RemoteDecideParams(
                        remote: true,
                        approval_id: approvalId,
                        decision: decision.remoteDecisionValue,
                        reason: reason
                    )
                )
                await loadApprovals()
                return approvals.first { $0.id == approvalId }
            } else {
                struct LocalDecideParams: Encodable, Sendable {
                    let approval_id: String
                    let status: String
                    let decided_by: String?
                }
                let decidedBy: String?
                if case .authenticated(_, let username, _) = coordinator.authState {
                    decidedBy = username
                } else {
                    decidedBy = "local_user"
                }
                let updated: FabushiStudioApprovalRecordDTO = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.fabushi.method("approvalDecide"),
                    params: LocalDecideParams(
                        approval_id: approvalId,
                        status: decision.localStatusValue,
                        decided_by: decidedBy
                    )
                )
                if let idx = approvals.firstIndex(where: { $0.id == approvalId }) {
                    approvals[idx] = updated
                } else {
                    approvals.insert(updated, at: 0)
                }
                return updated
            }
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func resolveToolDock(requestId: String, approved: Bool) async -> Bool {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return false }

        struct DockParams: Encodable, Sendable {
            let request_id: String
            let approved: Bool
        }
        do {
            let res: FabushiStudioToolDockResolveResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("approvalDecide"),
                params: DockParams(request_id: requestId, approved: approved)
            )
            liveToolApprovalRequests.removeAll { $0.approvalId == requestId }
            return res.resolved || res.approved == approved
        } catch {
            handleRPCFailure(error)
            return false
        }
    }

    @discardableResult
    public func issueValueBoundApproval(
        action: String,
        target: String,
        workingDirectory: String? = nil,
        contentSha256: String? = nil,
        ttlMs: Int64? = 300_000
    ) async -> FabushiStudioValueBoundTokenDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct IssueParams: Encodable, Sendable {
            let action: String
            let target: String
            let working_directory: String?
            let content_sha256: String?
            let ttl_ms: Int64?
        }
        do {
            let token: FabushiStudioValueBoundTokenDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("approvalIssueValueBound"),
                params: IssueParams(
                    action: action,
                    target: target,
                    working_directory: workingDirectory,
                    content_sha256: contentSha256,
                    ttl_ms: ttlMs
                )
            )
            valueBoundTokens.insert(token, at: 0)
            return token
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    @discardableResult
    public func consumeValueBoundApproval(
        approvalId: String,
        action: String,
        target: String,
        workingDirectory: String? = nil,
        contentSha256: String? = nil,
        nowMs: Int64? = nil
    ) async -> FabushiStudioValueBoundTokenDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }

        struct ConsumeParams: Encodable, Sendable {
            let approval_id: String
            let action: String
            let target: String
            let working_directory: String?
            let content_sha256: String?
            let now_ms: Int64?
        }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("approvalConsumeValueBound"),
                params: ConsumeParams(
                    approval_id: approvalId,
                    action: action,
                    target: target,
                    working_directory: workingDirectory,
                    content_sha256: contentSha256,
                    now_ms: nowMs
                )
            )
            let nowTimestampMs = nowMs ?? Int64(Date().timeIntervalSince1970 * 1000)
            if let idx = valueBoundTokens.firstIndex(where: { $0.approvalId == approvalId }) {
                let existing = valueBoundTokens[idx]
                let updated = FabushiStudioValueBoundTokenDTO(
                    approvalId: existing.approvalId,
                    action: existing.action,
                    target: existing.target,
                    workingDirectory: existing.workingDirectory,
                    contentSha256: existing.contentSha256,
                    createdAtMs: existing.createdAtMs,
                    expiresAtMs: existing.expiresAtMs,
                    spent: true,
                    retired: existing.retired
                )
                valueBoundTokens[idx] = updated
                return updated
            }
            return FabushiStudioValueBoundTokenDTO(
                approvalId: approvalId,
                action: action,
                target: target,
                workingDirectory: workingDirectory,
                contentSha256: contentSha256,
                createdAtMs: nowTimestampMs,
                expiresAtMs: nowTimestampMs,
                spent: true,
                retired: false
            )
        } catch {
            handleRPCFailure(error)
            return nil
        }
    }

    public func handleBrowserTakeoverAction(
        sessionId: String,
        action: FabushiStudioBrowserTakeoverAction
    ) async {
        syncAccountScopeIfNeeded()
        let callback = action.callbackPayload(sessionId: sessionId)

        if coordinator.isStarted && coordinator.authState != .unauthenticated {
            struct CmdParams: Encodable, Sendable { let command: String }
            _ = try? await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("runCommand"),
                params: CmdParams(command: callback)
            ) as FabushiStudioRunCommandResponse
        }

        if let idx = browserTakeoverRequests.firstIndex(where: { $0.sessionId == sessionId }) {
            let existing = browserTakeoverRequests[idx]
            switch action {
            case .takeOver:
                browserTakeoverRequests[idx] = FabushiStudioBrowserTakeoverRequest(
                    sessionId: existing.sessionId,
                    reason: existing.reason,
                    mode: .userTakeover(pausedAtStep: 0)
                )
            case .resumeAi, .cancelAi:
                browserTakeoverRequests.remove(at: idx)
            }
        }
        router.toastController.showNonChatToast(callback, isError: false)
    }

    // MARK: - Error Classification (`D10-10`) & Helpers

    public func dismissClassifiedError() {
        lastClassifiedError = nil
        lastErrorMessage = nil
    }

    public func handleClassifiedError(
        rawMessage: String,
        needsProviderCta: Bool = false,
        locale: FabushiLocale = .en
    ) {
        let classified = FabushiMagpieErrorClassifier.classify(
            rawMessage: rawMessage,
            needsProviderCta: needsProviderCta
        )
        lastClassifiedError = classified
        lastErrorMessage = classified.detail(locale: locale)

        // D10-10 strict rule: ONLY Tier 1 (`unconfiguredProvider`) may raise the Shell
        // `FabushiProviderErrorCard` with the "Open AI Providers" CTA!
        if classified.showSettingsCTA {
            router.activeProviderErrorMessage = classified.detail(locale: locale)
        }
        router.toastController.showNonChatToast(
            classified.toastMessage(locale: locale),
            isError: true
        )
    }

    private func handleRPCFailure(_ error: Error) {
        if let hostError = error as? FabushiHostError, hostError.isCancelled {
            return
        }
        if let coordError = error as? FabushiCoordinatorError,
           case .staleGeneration = coordError
        {
            syncAccountScopeIfNeeded()
            return
        }
        let message = error.localizedDescription
        lastErrorMessage = message
        let lower = message.lowercased()
        if lower.contains("provider") || lower.contains("needs_provider_selection") || lower.contains("http 4") {
            handleClassifiedError(rawMessage: message)
        } else {
            router.toastController.showNonChatToast(message, isError: true)
        }
    }

    private func upsertTask(_ task: FabushiStudioTaskFsmDTO) {
        if let idx = tasks.firstIndex(where: { $0.taskId == task.taskId }) {
            tasks[idx] = task
        } else {
            tasks.insert(task, at: 0)
        }
    }

    private func upsertSkill(_ doc: FabushiStudioSkillDocumentDTO) {
        if let idx = skills.firstIndex(where: { $0.frontmatter.name == doc.frontmatter.name }) {
            skills[idx] = doc
        } else {
            skills.insert(doc, at: 0)
        }
    }

    private func applyMemoryMutation(_ res: FabushiStudioMemoryMutationDTO, target: FabushiStudioMemoryTarget) {
        let updated = FabushiStudioMemoryReadDTO(
            target: res.target,
            raw: res.raw,
            entries: res.entries,
            charCount: res.charCount,
            charLimit: res.charLimit,
            remainingChars: res.remainingChars,
            usagePercent: res.usagePercent
        )
        switch target {
        case .memory:
            memoryState = updated
        case .user:
            userMemoryState = updated
        }
    }
}
