import Foundation
import Observation

@MainActor
@Observable
public final class FabushiCodingViewModel {
    @ObservationIgnored public let coordinator: FabushiCoordinator
    @ObservationIgnored public let router: ShellNavigationRouter
    @ObservationIgnored public let cleanupHub: FabushiAgentStudioLogoutCleanupHub
    @ObservationIgnored private let taskBag = FabushiCancellableTaskBag()
    @ObservationIgnored private let cleanupRegistrationId: String

    public let chatId: Int64
    public private(set) var lastObservedGeneration: UInt64
    public private(set) var lastObservedAccountId: String?

    public var isLoading: Bool = false
    public var providersCatalog: [FabushiCodingProviderCatalogItemDTO] = []
    public var codingBots: [FabushiCodingBotSummaryDTO] = []
    public var remoteComputers: [FabushiCodingRemoteComputerDTO] = []
    public var activeThread: FabushiCodingThreadDTO?
    public var checkpoints: [FabushiCodingCheckpointDTO] = []
    public var pendingApprovals: [FabushiCodingLiveApprovalPrompt] = []
    public var latestDiffStat: FabushiCodingDiffStatDTO = FabushiCodingDiffStatDTO()
    public var streamingDiffPreview: String = ""
    public var selectedPermissionMode: FabushiCodingPermissionMode = .ask

    public var botFatherPromptDraft: String = ""
    public var lastBotFatherResponse: FabushiCodingBotFatherNlResponse?
    public var lastRemoteHostProbeSummary: String = ""
    public var lastCommandOutput: String = ""
    public var lastSandboxNotice: String?
    public var lastErrorMessage: String?

    public var currentOwnerUserId: UInt64? {
        if case .authenticated(let userId, _, _) = coordinator.authState {
            return userId
        }
        return nil
    }

    public init(
        chatId: Int64 = 0,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub()
    ) {
        self.chatId = chatId
        self.coordinator = coordinator
        self.router = router
        self.cleanupHub = cleanupHub
        self.lastObservedGeneration = coordinator.accountScopeGeneration
        self.lastObservedAccountId = coordinator.activeAccountId
        self.cleanupRegistrationId = "coding.\(chatId).\(UUID().uuidString)"

        cleanupHub.registerResetHandler(id: cleanupRegistrationId) { [weak self] reason in
            self?.resetOnLogout(reason: reason)
        }
        startEventSubscription()
    }

    deinit {
        taskBag.cancelAll()
    }

    // MARK: - `D10-02` Account & Generation Fencing + `resetOnLogout()`

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
        taskBag.cancel(key: "coding.refresh")
        isLoading = false
        codingBots = []
        remoteComputers = []
        activeThread = nil
        checkpoints = []
        pendingApprovals = []
        latestDiffStat = FabushiCodingDiffStatDTO()
        streamingDiffPreview = ""
        selectedPermissionMode = .ask
        botFatherPromptDraft = ""
        lastBotFatherResponse = nil
        lastRemoteHostProbeSummary = ""
        lastCommandOutput = ""
        lastSandboxNotice = nil
        lastErrorMessage = nil

        lastObservedGeneration = coordinator.accountScopeGeneration
        lastObservedAccountId = coordinator.activeAccountId
    }

    private func startEventSubscription() {
        let stream = coordinator.host.makeEventStream(bufferCapacity: 256)
        let eventTask = Task { @MainActor [weak self] in
            for await event in stream {
                guard let self, !Task.isCancelled else { break }
                // Strict generation fence: drop stale events from prior account/session generations.
                guard event.generation >= self.coordinator.accountScopeGeneration else { continue }
                self.handleHostEvent(event)
            }
        }
        taskBag.replace(key: "coding.events", with: eventTask)
    }

    private func handleHostEvent(_ event: FabushiDomainEvent) {
        switch event.kind {
        case .codingHostCommand(let payload):
            guard let env = FabushiCodingHostCommandEnvelope(payload: payload) else { return }
            // `D10-02` strict `owner_user_id` isolation on incoming coding host commands.
            if let cmdOwner = env.ownerUserId,
               let myOwner = currentOwnerUserId,
               cmdOwner != myOwner
            {
                return
            }
            lastRemoteHostProbeSummary = "Host command: \(env.cmd)"

        case .toolApprovalRequested(let approvalId, let toolName, let summary):
            if !pendingApprovals.contains(where: { $0.requestId == approvalId }) {
                pendingApprovals.insert(
                    FabushiCodingLiveApprovalPrompt(
                        requestId: approvalId,
                        turnId: activeThread?.threadId ?? "turn_live",
                        kind: "tool",
                        title: toolName,
                        detail: summary,
                        ruleKey: toolName
                    ),
                    at: 0
                )
            }

        default:
            break
        }
    }

    // MARK: - Load & Catalog

    public func loadAll(botUserId: Int64? = nil) async {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }
        isLoading = true
        defer { isLoading = false }

        await loadProvidersCatalog()
        await loadCodingBots()
        await probeRemoteCodingHosts()

        let resolvedBotId = botUserId ?? codingBots.first?.botUserId ?? 0
        if resolvedBotId != 0 && chatId != 0 {
            _ = await loadThread(botUserId: resolvedBotId, chatId: chatId)
        }
    }

    public func loadProvidersCatalog() async {
        guard coordinator.isStarted else { return }
        do {
            let response: FabushiCodingProvidersCatalogResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("listProvidersCatalog"),
                params: FabushiEmptyParams()
            )
            providersCatalog = response.providers
        } catch {
            handleError(error)
        }
    }

    public func parseProvider(_ rawName: String) async -> FabushiCodingProviderCatalogItemDTO? {
        guard coordinator.isStarted else { return nil }
        do {
            return try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("parseProvider"),
                params: FabushiCodingParseProviderRequest(provider: rawName)
            )
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - Coding Bots & BotFather NL (`coding.{listBots,createBot,patchBot}`, `fabushi.botCreateNl`)

    public func loadCodingBots() async {
        guard coordinator.isStarted else { return }
        guard case .authenticated = coordinator.authState else {
            codingBots = []
            return
        }
        do {
            let allBots: [FabushiCodingBotSummaryDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("listBots"),
                params: FabushiEmptyParams()
            )
            let ownerFilter = currentOwnerUserId
            codingBots = allBots.filter { bot in
                guard bot.codingAgent != nil else { return false }
                if let ownerFilter, bot.ownerUserId > 0 {
                    return UInt64(bot.ownerUserId) == ownerFilter
                }
                return true
            }
        } catch {
            // Non-fatal when offline; leave list clean without fabricating sample bots
            handleError(error, silentNetworkFailure: true)
        }
    }

    @discardableResult
    public func createCodingBot(_ request: FabushiCodingCreateBotRequest) async -> FabushiCodingCreateBotResponse? {
        guard coordinator.isStarted else { return nil }
        do {
            let created: FabushiCodingCreateBotResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("createBot"),
                params: request
            )
            codingBots.removeAll { $0.botUserId == created.bot.botUserId }
            codingBots.insert(created.bot, at: 0)
            return created
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func patchCodingBot(
        botId: UInt64,
        preferredModel: String? = nil,
        codingAgent: FabushiCodingAgentBotConfigDTO? = nil
    ) async -> FabushiCodingBotSummaryDTO? {
        guard coordinator.isStarted else { return nil }
        do {
            let updated: FabushiCodingBotSummaryDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("patchBot"),
                params: FabushiCodingPatchBotRequest(
                    botId: botId,
                    preferredModel: preferredModel,
                    codingAgent: codingAgent
                )
            )
            if let idx = codingBots.firstIndex(where: { $0.botUserId == updated.botUserId }) {
                codingBots[idx] = updated
            } else {
                codingBots.insert(updated, at: 0)
            }
            return updated
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func createBotViaBotFatherNL(
        prompt: String,
        avatarShape: String? = nil,
        avatarColor: String? = nil
    ) async -> FabushiCodingBotFatherNlResponse? {
        guard coordinator.isStarted else { return nil }
        let trimmed = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        do {
            let response: FabushiCodingBotFatherNlResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("botCreateNl"),
                params: FabushiCodingBotFatherNlRequest(
                    prompt: trimmed,
                    avatarShape: avatarShape,
                    avatarColor: avatarColor
                )
            )
            lastBotFatherResponse = response
            botFatherPromptDraft = ""
            await loadCodingBots()
            return response
        } catch {
            handleError(error)
            return nil
        }
    }

    // MARK: - Remote Coding Hosts (`computers.list`)

    @discardableResult
    public func probeRemoteCodingHosts(locale: FabushiLocale = .en) async -> [FabushiCodingRemoteComputerDTO] {
        guard coordinator.isStarted else { return [] }
        guard case .authenticated = coordinator.authState else {
            lastRemoteHostProbeSummary = FabushiAgentStudioStrings.text("m4.common.signInRequired", locale: locale)
            return []
        }

        do {
            let list: FabushiCodingComputerListResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.computers.method("list"),
                params: FabushiEmptyParams()
            )
            let ownerFilter = currentOwnerUserId
            let scoped = list.computers.filter { comp in
                if let ownerFilter, comp.ownerUserId > 0 {
                    return comp.ownerUserId == ownerFilter
                }
                return true
            }
            remoteComputers = scoped
            if scoped.isEmpty {
                lastRemoteHostProbeSummary = FabushiAgentStudioStrings.text("m4.coding.noRemoteHosts", locale: locale)
            } else {
                let names = scoped.map { "\($0.displayName) (\($0.ipAddress))" }.joined(separator: ", ")
                lastRemoteHostProbeSummary = "\(scoped.count) host(s): \(names)"
            }
            return scoped
        } catch {
            handleError(error, silentNetworkFailure: true)
            return []
        }
    }

    // MARK: - `D10-02` `owner_user_id`-Isolated Threads & Checkpoints (`coding.{upsertThread,getThread,saveCheckpoint,getCheckpoints}`)

    @discardableResult
    public func upsertThread(_ request: FabushiCodingThreadUpsertRequest) async -> FabushiCodingThreadUpsertResponse? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        do {
            let result: FabushiCodingThreadUpsertResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("upsertThread"),
                params: request
            )
            _ = await loadThread(
                botUserId: request.botUserId,
                chatId: request.chatId,
                topicId: request.topicId
            )
            return result
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func loadThread(
        botUserId: Int64,
        chatId targetChatId: Int64? = nil,
        topicId: Int64? = nil
    ) async -> FabushiCodingThreadDTO? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        let resolvedChatId = targetChatId ?? chatId
        do {
            let thread: FabushiCodingThreadDTO? = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("getThread"),
                params: FabushiCodingGetThreadRequest(
                    botUserId: botUserId,
                    chatId: resolvedChatId,
                    topicId: topicId
                )
            )
            activeThread = thread
            if let thread {
                selectedPermissionMode = thread.permissionMode
                _ = await loadCheckpoints(threadId: thread.threadId)
            } else {
                checkpoints = []
                latestDiffStat = FabushiCodingDiffStatDTO()
            }
            return thread
        } catch let hostError as FabushiHostError {
            if case .invalidResponse(let raw) = hostError,
               let data = raw.data(using: .utf8),
               let envelope = try? JSONDecoder().decode(FabushiRPCResponse<FabushiJSONValue>.self, from: data),
               envelope.ok,
               envelope.error == nil
            {
                activeThread = nil
                checkpoints = []
                latestDiffStat = FabushiCodingDiffStatDTO()
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
    public func saveCheckpoint(
        threadId: String,
        turnIdx: UInt64,
        beforeRef: String,
        afterRef: String,
        diffStat: FabushiCodingDiffStatDTO = FabushiCodingDiffStatDTO(),
        promptPreview: String = "",
        replyPreview: String = ""
    ) async -> FabushiCodingSaveCheckpointResponse? {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return nil }
        do {
            let saved: FabushiCodingSaveCheckpointResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("saveCheckpoint"),
                params: FabushiCodingSaveCheckpointRequest(
                    threadId: threadId,
                    turnIdx: turnIdx,
                    beforeRef: beforeRef,
                    afterRef: afterRef,
                    diffStat: diffStat,
                    promptPreview: promptPreview,
                    replyPreview: replyPreview
                )
            )
            latestDiffStat = diffStat
            _ = await loadCheckpoints(threadId: threadId)
            return saved
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func loadCheckpoints(threadId: String, limit: UInt64 = 20) async -> [FabushiCodingCheckpointDTO] {
        syncAccountScopeIfNeeded()
        guard coordinator.isStarted else { return [] }
        do {
            let list: [FabushiCodingCheckpointDTO] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method("getCheckpoints"),
                params: FabushiCodingGetCheckpointsRequest(threadId: threadId, limit: limit)
            )
            checkpoints = list
            if let latest = list.first {
                latestDiffStat = latest.diffStat
            }
            return list
        } catch {
            handleError(error)
            return []
        }
    }

    // MARK: - Remote Coding Slash Commands (`/diff`, `/rollback`, `/pr`, `/status`) & Approvals

    private struct CodingSlashCommandRequest: Encodable, Sendable {
        let command: String
        let sessionId: String?
        let chatId: Int64?
        let botId: UInt64?

        enum CodingKeys: String, CodingKey {
            case command
            case sessionId = "session_id"
            case chatId = "chat_id"
            case botId = "bot_id"
        }
    }

    @discardableResult
    public func dispatchRemoteCodingCommand(
        _ slashCommand: String,
        botId: UInt64? = nil,
        isChatStageSurface: Bool = false,
        isCompact: Bool = true
    ) async -> FabushiStudioRunCommandResponse? {
        guard coordinator.isStarted else { return nil }
        let trimmed = slashCommand.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        guard case .authenticated = coordinator.authState else {
            let msg = FabushiAgentStudioStrings.text("m4.common.signInRequired", locale: .en)
            lastCommandOutput = msg
            notify(msg, isError: true, isChatStageSurface: isChatStageSurface, isCompact: isCompact)
            return nil
        }

        do {
            let resp: FabushiStudioRunCommandResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("runCommand"),
                params: CodingSlashCommandRequest(
                    command: trimmed,
                    sessionId: activeThread?.threadId,
                    chatId: chatId != 0 ? chatId : nil,
                    botId: botId ?? codingBots.first.map { UInt64(max(0, $0.botUserId)) }
                )
            )
            let output = resp.replyText ?? resp.output
            lastCommandOutput = output
            if trimmed.hasPrefix("/diff") {
                streamingDiffPreview = output
            }
            if !output.isEmpty {
                notify(output, isError: false, isChatStageSurface: isChatStageSurface, isCompact: isCompact)
            }
            return resp
        } catch {
            handleError(error)
            return nil
        }
    }

    @discardableResult
    public func resolveCodingApproval(
        requestId: String,
        decision: FabushiCodingApprovalDecision,
        isChatStageSurface: Bool = false,
        isCompact: Bool = true
    ) async -> Bool {
        guard coordinator.isStarted else { return false }
        let approved = decision != .deny

        struct ResolveDockParams: Encodable, Sendable {
            let requestId: String
            let approved: Bool
            enum CodingKeys: String, CodingKey {
                case requestId = "request_id"
                case approved
            }
        }

        do {
            let resp: FabushiStudioToolDockResolveResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.mcp.method("permissionDockResolve"),
                params: ResolveDockParams(requestId: requestId, approved: approved)
            )
            pendingApprovals.removeAll { $0.requestId == requestId }
            notify(
                approved ? "Coding approval granted (\(decision.rawValue))" : "Coding action denied",
                isError: !approved,
                isChatStageSurface: isChatStageSurface,
                isCompact: isCompact
            )
            return resp.approved == approved
        } catch {
            handleError(error)
            return false
        }
    }

    // MARK: - Strict `IOS-MAIN-INV-04` Fail-Closed Local Subprocess Boundary

    /// Verifies that local CLI execution (`coding.probeLocal`, `startLocalTurn`, `localControl`)
    /// fails closed with `capabilityUnavailable` on iOS.
    @discardableResult
    public func probeLocalSubprocessFailClosed(action: String = "probeLocal", locale: FabushiLocale = .en) async -> Bool {
        guard coordinator.isStarted else { return false }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.coding.method(action),
                params: FabushiEmptyParams()
            )
            return false
        } catch let hostError as FabushiHostError {
            if case .capabilityUnavailable = hostError {
                lastSandboxNotice = FabushiAgentStudioStrings.text("m4.coding.remoteOnlyNotice", locale: locale)
                return true
            }
            handleError(hostError)
            return false
        } catch {
            handleError(error)
            return false
        }
    }

    // MARK: - Helpers

    private func notify(_ message: String, isError: Bool, isChatStageSurface: Bool, isCompact: Bool) {
        if isChatStageSurface && chatId != 0 {
            router.showChatToastIfVisible(
                message,
                chatId: chatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: isError
            )
        } else {
            router.toastController.showNonChatToast(message, isError: isError)
        }
    }

    private func handleError(_ error: Error, silentNetworkFailure: Bool = false) {
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
                return
            }
            if silentNetworkFailure {
                if case .networkError = hostError { return }
                if case .unauthorized = hostError { return }
            }
        }
        lastErrorMessage = error.localizedDescription
    }
}
